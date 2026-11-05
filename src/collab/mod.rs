//! Editing one map together (File > Multiplayer): this editor hosts the map open now, or
//! joins one somebody else hosts, on the LAN or through a Cloudflare quick tunnel (openOMSI's
//! own, `omsi_net::tunnel`). The connection is `openmapper-collab`'s; this puts the map's
//! parts in and out of it (`sync`, `codec`) and keeps the session going each frame.
//!
//! The tiles, the global settings and the groups are shared. The timetable, the traffic and
//! the descriptions stay the host's in this version (a guest's change of them is put back),
//! and the host is the one who saves the map.

pub mod codec;
pub mod presence;
pub(crate) mod recovery;
pub mod sync;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use openmapper_collab::gateway::{self, Gateway};
use openmapper_collab::{Client, Event, Host, HostConfig, JoinError, MapInfo, Msg, Part, Peer, Presence, Refusal, Welcome, Why, DEFAULT_PORT};

use crate::doc::{next_rev, Document};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::install::Roots;
use sync::{Note, Sync};

/// The shortest password a host on the internet takes.
pub const MIN_PASSWORD: usize = 8;
/// People in one session, the host's own editor counted.
const MAX_PEERS: usize = 16;
/// Chat lines kept.
const CHAT: usize = 200;
/// The most of the others' work held while an edit of this editor's is open: messages, and
/// the bytes of their parts.
const MAX_HELD: usize = 4096;
const MAX_HELD_BYTES: u64 = 512 << 20;

pub struct Collab {
    pub open: bool,
    pub name: String,
    pub address: String,
    pub password: String,
    pub port: u16,
    /// Others on this network may join (the host listens on every address, not the loopback).
    pub lan: bool,
    /// Others anywhere may join through a Cloudflare quick tunnel.
    pub internet: bool,
    /// Hosting without a password was chosen and confirmed (never with `internet`; typing a
    /// password undoes it).
    pub no_password: bool,
    /// The window asks whether to host without a password.
    pub asking_no_password: bool,
    pub error: Option<String>,
    pub say: String,
    phase: Phase,
    /// A map joined, for the app to open (`take_joined`).
    joined: Option<Document>,
    /// Its groups, put in once it is open (opening reads them from its folder): the session
    /// waits until then.
    joined_groups: Option<Vec<crate::mapgroups::MapGroup>>,
}

impl Default for Collab {
    fn default() -> Collab {
        let name = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
        Collab { open: false, name: name.chars().take(32).collect(), address: String::new(), password: String::new(), port: DEFAULT_PORT, lan: true, internet: false, no_password: false, asking_no_password: false, error: None, say: String::new(), phase: Phase::Off, joined: None, joined_groups: None }
    }
}

enum Phase {
    Off,
    Joining(Receiver<Result<(Client, Welcome), String>>),
    Live(Box<Live>),
}

pub struct Live {
    client: Client,
    sync: Sync,
    pub you: Peer,
    pub peers: Vec<Peer>,
    pub map: MapInfo,
    /// Where each of the others works, and when it was heard.
    pub seen: HashMap<u32, (Presence, Instant)>,
    pub chat: Vec<(u32, String)>,
    /// What of this editor's presence was sent last.
    presence: presence::Sender,
    /// What came from the host while an edit was held open, to put in once it is closed,
    /// and the bytes of the parts it carries.
    held: std::collections::VecDeque<Msg>,
    held_bytes: u64,
    pub hosting: Option<Hosting>,
    /// The folder of the map the session is about (another map opened ends it).
    dir: PathBuf,
}

pub struct Hosting {
    _host: Host,
    _gateway: Option<Gateway>,
    /// The addresses on this network to give out, and what each is (read once).
    pub addrs: Vec<(String, &'static str)>,
    pub tunnel: Arc<Mutex<TunnelState>>,
}

pub enum TunnelState {
    Off,
    /// cloudflared starting (fetched first when it is not there).
    Starting,
    Up(omsi_net::tunnel::Tunnel),
    Failed,
}

impl TunnelState {
    pub fn url(&self) -> Option<String> {
        match self {
            TunnelState::Up(t) => t.url.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            _ => None,
        }
    }
}

impl Collab {
    pub fn live(&self) -> Option<&Live> {
        match &self.phase {
            Phase::Live(l) => Some(l),
            _ => None,
        }
    }

    pub fn joining(&self) -> bool {
        matches!(self.phase, Phase::Joining(_))
    }

    /// In a session somebody else hosts (the host saves the map, not this editor).
    pub fn is_guest(&self) -> bool {
        self.live().map(|l| l.hosting.is_none()).unwrap_or(false)
    }

    pub fn settled(&self) -> bool {
        self.live().map(|l| l.sync.settled()).unwrap_or(true)
    }

    /// Host the map open in the editor.
    pub fn host(&mut self, ed: &mut Editor) {
        self.error = None;
        let Some(doc) = ed.doc.as_mut() else { return };
        if doc._publisher_protected_source {
            self.error = Some("publisher-protected maps cannot be hosted as editable sessions".into());
            return;
        }
        if self.internet && self.password.chars().count() < MIN_PASSWORD {
            self.error = Some(format!("{} {MIN_PASSWORD}", tr("A host on the internet needs a password of at least", "Хосту в интернете нужен пароль не короче")));
            return;
        }
        // (no password only when chosen and confirmed, and never on the internet)
        let open = self.password.is_empty() && self.no_password && !self.internet;
        if self.password.is_empty() && !open {
            self.error = Some(tr("Type a password, or choose explicitly to host without one", "Введите пароль или явно выберите открыть без пароля").into());
            return;
        }
        ed.history.commit();
        let _ = ed.history.take_changed();
        let ip: std::net::IpAddr = if self.lan { std::net::Ipv4Addr::UNSPECIFIED.into() } else { std::net::Ipv4Addr::LOCALHOST.into() };
        let folder = doc.dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let map = MapInfo { folder, name: doc.global.friendly_name.clone() };
        let cfg = HostConfig { bind: (ip, self.port).into(), password: self.password.clone(), open, map, next_id: doc.global.next_id_code.max(1), max_peers: MAX_PEERS, ..Default::default() };
        let host = match Host::start(cfg, sync::all_states(doc)) {
            Ok(h) => h,
            Err(e) => {
                self.error = Some(format!("{} {}: {e}", tr("Could not listen on port", "Не удалось открыть порт"), self.port));
                return;
            }
        };
        let port = host.addr().port();
        let (client, w) = match Client::connect(&format!("127.0.0.1:{port}"), &self.name, &self.password, false) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(join_error(&e));
                return;
            }
        };
        let tunnel = Arc::new(Mutex::new(TunnelState::Off));
        let mut gw = None;
        if self.internet {
            match Gateway::start(host.addr()) {
                Ok(g) => {
                    *tunnel.lock().unwrap_or_else(|e| e.into_inner()) = TunnelState::Starting;
                    let (t, gw_port) = (tunnel.clone(), g.addr().port());
                    // (cloudflared may have to be fetched first: off the window's thread)
                    std::thread::spawn(move || {
                        let state = match omsi_net::tunnel::Tunnel::start(gw_port) {
                            Some(tun) => TunnelState::Up(tun),
                            None => TunnelState::Failed,
                        };
                        *t.lock().unwrap_or_else(|e| e.into_inner()) = state;
                    });
                    gw = Some(g);
                }
                Err(e) => self.error = Some(format!("{}: {e}", tr("The way in for the tunnel could not be opened", "Не удалось открыть вход для туннеля"))),
            }
        }
        let sync = Sync::new(&w.states, w.ids, true, doc);
        let addrs = if self.lan { omsi_net::addrs::joinable_addresses().iter().map(|a| (format!("{}:{port}", a.ip), a.label())).collect() } else { Vec::new() };
        let hosting = Hosting { _host: host, _gateway: gw, addrs, tunnel };
        self.phase = Phase::Live(Box::new(Live { client, sync, you: w.you, peers: w.peers, map: w.map, seen: HashMap::new(), chat: Vec::new(), presence: Default::default(), held: Default::default(), held_bytes: 0, hosting: Some(hosting), dir: doc.dir.clone() }));
        ed.toast(tr("Hosting this map: others can join it now", "Карта открыта для совместной правки"));
    }

    /// Join the host at the address typed (a LAN `host:port` or a tunnel's `https://…`).
    pub fn join(&mut self) {
        self.error = None;
        let (address, name, password) = (self.address.trim().to_string(), self.name.clone(), self.password.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = (|| {
                let target = match gateway::tunnel_url(&address) {
                    // (openOMSI's forwarding: a local port whose connections go through the tunnel)
                    Some(url) => omsi_net::ws::tcp_forward(&url).map_err(|e| e.to_string())?.to_string(),
                    None if address.contains(':') => address,
                    None => format!("{address}:{DEFAULT_PORT}"),
                };
                Client::connect(&target, &name, &password, true).map_err(|e| join_error(&e))
            })();
            let _ = tx.send(r);
        });
        self.phase = Phase::Joining(rx);
    }

    pub fn cancel_join(&mut self) {
        if self.joining() {
            self.phase = Phase::Off;
        }
    }

    /// End the session (hosting stops with it). The next IDCode handed out is past every one
    /// the map has: the others' blocks may lie above this editor's.
    pub fn leave(&mut self, ed: &mut Editor) {
        if let (Phase::Live(_), Some(doc)) = (&self.phase, ed.doc.as_mut()) {
            let top = doc.tiles.values().flat_map(|t| t.tile.objects.iter().map(|o| o.id).chain(t.tile.splines.iter().map(|s| s.id))).max().unwrap_or(0);
            doc.global.next_id_code = doc.global.next_id_code.max(top + 1);
            doc.global_dirty = true;
        }
        self.phase = Phase::Off;
        self.joined = None;
        self.joined_groups = None;
    }

    /// A map joined, for the app to open (then `joined_open`).
    pub fn take_joined(&mut self) -> Option<Document> {
        self.joined.take()
    }

    /// After the app opened the joined map (opening reads the groups from its folder).
    pub fn joined_open(&mut self, ed: &mut Editor) {
        if let (Some(groups), Some(doc)) = (self.joined_groups.take(), ed.doc.as_mut()) {
            doc.groups = groups;
            if let Phase::Live(l) = &mut self.phase {
                l.dir = doc.dir.clone();
            }
        }
    }

    pub fn say(&mut self) {
        let text = std::mem::take(&mut self.say);
        if let (Phase::Live(l), false) = (&self.phase, text.trim().is_empty()) {
            l.client.send(Msg::Say(text));
        }
    }

    /// Each frame: what came in, what was changed here, what goes out.
    pub fn tick(&mut self, ed: &mut Editor, roots: Option<&Roots>) {
        if let Phase::Joining(rx) = &self.phase {
            match rx.try_recv() {
                Ok(Ok((client, w))) => self.joined_now(client, w, roots),
                Ok(Err(e)) => {
                    self.error = Some(e);
                    self.phase = Phase::Off;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.phase = Phase::Off,
            }
            return;
        }
        let Phase::Live(l) = &mut self.phase else { return };
        // (the map is opened by the app after joining: until then there is nothing to keep)
        if self.joined_groups.is_some() {
            return;
        }
        let Some(doc) = ed.doc.as_mut() else {
            self.phase = Phase::Off;
            return;
        };
        if doc.dir != l.dir {
            // another map was opened
            self.phase = Phase::Off;
            ed.toast(tr("Multiplayer ended: another map was opened", "Совместная правка завершена: открыта другая карта"));
            return;
        }
        // (everything that came is read now, whatever the editor is doing: a reader left
        // waiting would fill the host's queue for this editor, and the host would let it go)
        let mut gone = None;
        for e in l.client.poll() {
            match e {
                Event::Closed(why) => gone = Some(why),
                Event::Msg(Msg::Peers(p)) => {
                    l.seen.retain(|id, _| p.iter().any(|q| q.id == *id));
                    l.peers = p;
                }
                Event::Msg(Msg::Seen { peer, presence }) => {
                    l.seen.insert(peer, (presence, Instant::now()));
                }
                Event::Msg(Msg::Said { peer, text }) => {
                    l.chat.push((peer, text));
                    if l.chat.len() > CHAT {
                        l.chat.remove(0);
                    }
                }
                Event::Msg(m) => {
                    l.held_bytes += msg_bytes(&m);
                    l.held.push_back(m);
                }
            }
        }
        let overflow = gone.is_none() && (l.held.len() > MAX_HELD || l.held_bytes > MAX_HELD_BYTES);
        if overflow {
            gone = Some("too much came in while an edit was held open".into());
        }
        let mut notes = Vec::new();
        let mut put = false;
        // A drag, a brush stroke or a road being drawn is one history step, open until it
        // ends: the others' edits wait for it (put in its middle they would end the step while
        // the tool still holds its starting state), and so does sending this editor's.
        let open = ed.history.is_open();
        if !open && gone.is_none() {
            let changed = ed.history.take_changed();
            if !changed.is_empty() {
                notes.extend(l.sync.edited(changed, doc, &mut ed.history));
            }
            while let Some(m) = l.held.pop_front() {
                l.held_bytes -= msg_bytes(&m);
                put |= matches!(m, Msg::Applied { seq: None, .. } | Msg::Rejected { .. });
                notes.extend(l.sync.received(m, doc, &mut ed.history));
                if notes.iter().any(|n| matches!(n, Note::RecoveryFailed(_))) {
                    break;
                }
            }
        }
        if put {
            doc.global_rev = next_rev();
            keep_selection(ed);
        }
        let Some(doc) = ed.doc.as_mut() else { return };
        if !open && gone.is_none() && !notes.iter().any(|n| matches!(n, Note::RecoveryFailed(_))) {
            if let Some(m) = l.sync.next_edit(doc) {
                l.client.send(m);
            }
        }
        if let Some(m) = l.sync.keep_ids(doc) {
            l.client.send(m);
        }
        if let Some(p) = l.presence.next(Instant::now(), presence::mine(ed)) {
            l.client.send_presence(p);
        }
        // (somebody silent this long has stopped sending: not drawn)
        l.seen.retain(|_, (_, heard)| heard.elapsed() < presence::SEEN_FOR);
        for n in notes {
            let name = |id: u32| l.peers.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| "?".into());
            match n {
                Note::TurnedDown { tiles, why } => {
                    let what = match why {
                        Why::Stale => tr("Somebody changed this first: your edit was put back", "Кто-то изменил это раньше: ваша правка отменена"),
                        Why::TooMuch => tr("The host takes no more: the edit is too big, or the session holds as much as it may. Your edit was put back", "Хост больше не принимает: правка слишком большая или сессия заполнена. Ваша правка отменена"),
                        Why::NotThere => tr("That lies too far from the map for this session: your edit was put back", "Это слишком далеко от карты для этой сессии: ваша правка отменена"),
                    };
                    ed.error(format!("{what} ({tiles} {})", tr("tiles", "тайлов")));
                }
                Note::Replaced { by, parts } => ed.toast(format!("{} {} ({parts})", name(by), tr("changed what you were editing: it is as they left it", "изменил(а) то, что вы правили: оставлено как у него/неё"))),
                Note::HostOnly => ed.error(tr("The timetable, the traffic and the descriptions are the host's in multiplayer: your change was put back", "Расписание, трафик и описания в совместной правке — у хоста: ваша правка отменена")),
                Note::Unreadable(p) => log::warn!("multiplayer: {p:?} could not be read, left as it was"),
                Note::Recovered(path) => ed.toast(format!("{}: {}", tr("Your conflicting edit was kept", "Ваша конфликтующая правка сохранена"), path.display())),
                Note::RecoveryFailed(error) => {
                    ed.error(format!("{}: {error}", tr("Could not keep your conflicting edit; multiplayer stopped", "Не удалось сохранить конфликтующую правку; совместная правка остановлена")));
                    gone = Some(error);
                }
            }
        }
        if overflow {
            ed.error(tr("Too much of the others' work came in while your edit was held open: multiplayer stopped. The map stays open; it can be saved as a copy.", "Пока ваша правка была открыта, пришло слишком много чужих правок: совместная правка остановлена. Карта остаётся открытой, её можно сохранить как копию."));
        }
        if let Some(why) = gone {
            log::info!("multiplayer: connection closed: {why}");
            let host = l.hosting.is_some();
            self.leave(ed);
            if !host && !overflow {
                ed.error(tr("The connection to the host was lost. The map stays open; it can be saved as a copy.", "Связь с хостом потеряна. Карта остаётся открытой, её можно сохранить как копию."));
            }
        }
    }

    fn joined_now(&mut self, client: Client, w: Welcome, roots: Option<&Roots>) {
        let Some(roots) = roots else {
            self.error = Some(tr("No OMSI 2 installation: the map has nowhere to go", "Нет установки OMSI 2: карте некуда сохраниться").into());
            self.phase = Phase::Off;
            return;
        };
        let mut doc = match guest_doc(roots, &w) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(e);
                self.phase = Phase::Off;
                return;
            }
        };
        let groups = w.states.iter().find(|s| s.part == Part::Groups).and_then(|s| s.data.as_deref()).and_then(|b| serde_json::from_slice(b).ok()).unwrap_or_default();
        let sync = Sync::new(&w.states, w.ids, false, &mut doc);
        let dir = doc.dir.clone();
        self.joined = Some(doc);
        self.joined_groups = Some(groups);
        self.phase = Phase::Live(Box::new(Live { client, sync, you: w.you, peers: w.peers, map: w.map, seen: HashMap::new(), chat: Vec::new(), presence: Default::default(), held: Default::default(), held_bytes: 0, hosting: None, dir }));
    }
}

/// The map a host sent, as a document: in its own folder beside the guest's maps
/// (`<folder>_multiplayer`), never over a map of the same name the guest has.
fn guest_doc(roots: &Roots, w: &Welcome) -> Result<Document, String> {
    let global = w.states.iter().find(|s| s.part == Part::Global).and_then(|s| s.data.as_deref()).ok_or_else(|| tr("The host sent no map", "Хост не прислал карту").to_string())?;
    let base: String = w.map.folder.chars().filter(|c| c.is_alphanumeric() || " _-".contains(*c)).take(60).collect::<String>().trim().to_string();
    let base = if base.is_empty() { "Map".to_string() } else { base };
    let maps = roots.content.join("maps");
    let dir = (1..).map(|n| maps.join(if n == 1 { format!("{base}_multiplayer") } else { format!("{base}_multiplayer{n}") })).find(|d| !d.exists()).expect("a free folder");
    let mut doc = placeholder();
    doc.dir = dir.clone();
    doc.global.path = dir.join("global.cfg");
    if !codec::apply_global(&mut doc, global) {
        return Err(tr("The host's map could not be read", "Не удалось прочитать карту хоста").into());
    }
    omsi_map::configure_grid(&doc.global);
    for s in &w.states {
        if let (Part::Tile(x, y), Some(b)) = (s.part, s.data.as_deref()) {
            if !codec::apply_tile(&mut doc, (x, y), Some(b)) {
                log::warn!("multiplayer: tile {x}, {y} could not be read");
            }
        }
    }
    doc.global_dirty = true;
    Ok(doc)
}

/// An empty document to fill.
fn placeholder() -> Document {
    Document {
        global: Default::default(),
        _publisher_mounts: std::sync::Arc::new(Default::default()),
        _publisher_protected_source: false,
        tiles: Default::default(),
        order: Vec::new(),
        dir: PathBuf::new(),
        source: None,
        global_dirty: false,
        fresh: true,
        global_rev: next_rev(),
        timetable: Default::default(),
        traffic: Default::default(),
        descriptions: Default::default(),
        groups: Vec::new(),
    }
}

/// What is selected and no longer there (somebody else's edit took it) is let go.
fn keep_selection(ed: &mut Editor) {
    let Some(doc) = ed.doc.as_ref() else { return };
    let there = |s: &crate::doc::Sel| match *s {
        crate::doc::Sel::Object(k, id) => doc.object(k, id).is_some(),
        crate::doc::Sel::Spline(k, id) => doc.spline(k, id).is_some(),
        crate::doc::Sel::Attached(k, id) => doc.attachment(k, id).is_some(),
    };
    ed.also.retain(|s| there(s));
    if ed.selected.as_ref().map(|s| !there(s)).unwrap_or(false) {
        ed.selected = None;
    }
    if ed.hover.as_ref().map(|s| !there(s)).unwrap_or(false) {
        ed.hover = None;
    }
}

pub fn join_error(e: &JoinError) -> String {
    match e {
        JoinError::Io(e) => format!("{}: {e}", tr("No connection", "Нет связи")),
        JoinError::NotAHost => tr("That address is not an openMapper host", "По этому адресу нет хоста openMapper").into(),
        JoinError::Refused(Refusal::Password) => tr("Wrong password", "Неверный пароль").into(),
        JoinError::Refused(Refusal::Full) => tr("The session is full", "Сессия заполнена").into(),
        JoinError::Refused(Refusal::Name) => tr("Type a name (up to 32 letters)", "Введите имя (до 32 букв)").into(),
        JoinError::Refused(Refusal::Protocol(v)) => format!("{} ({v})", tr("The host has another version of openMapper", "У хоста другая версия openMapper")),
    }
}

/// The bytes of the parts a message carries.
fn msg_bytes(m: &Msg) -> u64 {
    match m {
        Msg::Applied { states, .. } | Msg::Rejected { states, .. } => states.iter().map(|s| s.data.as_ref().map(|d| d.len() as u64).unwrap_or(0)).sum(),
        _ => 0,
    }
}
