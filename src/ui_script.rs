//! The Scripts window (Map menu): a small IDE for the map's code. On the left the project
//! (the map's scripts, which start when it is played; modules they share; data files), the Lua
//! scripts of the objects on the map, the OMSI 2 scripts (`.osc`) of the selected object and
//! the map's Rust plugin. In the middle the files in tabs; below them the problems, the
//! console, the debugger, the game's events to send by hand and the build. The scripts run in
//! the editor with the game's API, stop at breakpoints and are stepped through; the project
//! is built into one `.oop` to share.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use egui::{Color32, RichText, Vec2};

use crate::doc::{tile_origin, Sel};
use crate::editable;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::script::build::{self, Msg};
use crate::script::lex::Lang;
use crate::script::lint::{self, Problem};
use crate::script::project::{self, Kind, Project};
use crate::script::run::{self, Arg, Cmd, LineKind, Out, Session, Val};
use crate::scripting::{self, Behaviour};
use crate::ui::Action;
use crate::ui_code::{self, CodeState};

/// What a tab holds.
#[derive(Clone, PartialEq)]
enum TabKind {
    /// A file of the map's project (its path in it).
    Project(String),
    /// A Lua script of an object type (`[lua_script]`).
    Object,
    /// An OMSI 2 object script, checked against the object's variable lists.
    Osc { vars: Vec<String>, strs: Vec<String>, consts: Vec<PathBuf> },
    /// A file of the map's Rust plugin.
    Rust,
}

struct Tab {
    kind: TabKind,
    path: PathBuf,
    title: String,
    text: String,
    saved: String,
    writable: bool,
    problems: Vec<Problem>,
    stale: bool,
    code: CodeState,
}

impl Tab {
    fn dirty(&self) -> bool {
        self.text != self.saved
    }

    fn lang(&self) -> Lang {
        Lang::of(&self.path.to_string_lossy())
    }
}

/// A Lua script of objects on the map.
#[derive(Clone)]
struct ObjScript {
    /// The object type (its file, as the map names it).
    object: String,
    script: PathBuf,
    writable: bool,
    /// How many objects of the type the map has.
    count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bottom {
    Problems,
    Console,
    Debug,
    Events,
    Build,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Map,
    /// The object script (index into `objects`).
    Object(usize),
}

struct NewFile {
    kind: Kind,
    name: String,
    template: usize,
}

struct Line {
    kind: LineKind,
    text: String,
    at: Option<(String, usize)>,
}

struct Stopped {
    reason: String,
    file: String,
    line: usize,
    stack: Vec<run::Frame>,
}

pub struct ScriptsUi {
    pub open: bool,
    /// The map the project and lists are of.
    map_for: String,
    files: Vec<project::File>,
    files_stale: bool,
    tabs: Vec<Tab>,
    active: usize,
    objects: Option<Vec<ObjScript>>,
    /// The selected object's OMSI 2 scripts: its type, and its scripts' paths.
    osc_for: Option<String>,
    osc: Vec<(PathBuf, (Vec<String>, Vec<String>, Vec<PathBuf>))>,
    bottom: Bottom,
    show_api: bool,
    api_search: String,
    new_file: Option<NewFile>,
    rename: Option<(String, String)>,
    // the test run
    session: Option<Session>,
    run_root: PathBuf,
    target: Target,
    console: Vec<Line>,
    console_input: String,
    history: Vec<String>,
    history_at: usize,
    stopped: Option<Stopped>,
    frame: usize,
    expanded: HashMap<usize, Vec<(String, Val)>>,
    clock: f64,
    time: f64,
    running: bool,
    start_clock: f64,
    speed: f64,
    listening: (Vec<(String, usize)>, usize),
    state: (Vec<(String, String)>, Vec<(i64, Vec<(String, String)>)>),
    event: usize,
    event_args: Vec<String>,
    // the build
    building: Option<Receiver<Msg>>,
    build_log: Vec<String>,
    sign_key: String,
    // a light switched by a script
    light_open: bool,
    light: Behaviour,
    light_color: [u8; 3],
    light_range: f32,
    message: Option<(String, bool)>,
    /// A project file to open, and whether to run, once the map is there (checks without a
    /// person at the screen: OPENMAPPER_SCRIPTS).
    pending: Option<(String, bool, Option<usize>)>,
}

impl Default for ScriptsUi {
    fn default() -> Self {
        ScriptsUi {
            open: false,
            map_for: String::new(),
            files: Vec::new(),
            files_stale: true,
            tabs: Vec::new(),
            active: 0,
            objects: None,
            osc_for: None,
            osc: Vec::new(),
            bottom: Bottom::Problems,
            show_api: false,
            api_search: String::new(),
            new_file: None,
            rename: None,
            session: None,
            run_root: PathBuf::new(),
            target: Target::Map,
            console: Vec::new(),
            console_input: String::new(),
            history: Vec::new(),
            history_at: 0,
            stopped: None,
            frame: 0,
            expanded: HashMap::new(),
            clock: 8.0 * 3600.0,
            time: 0.0,
            running: false,
            start_clock: 8.0 * 3600.0,
            speed: 1.0,
            listening: (Vec::new(), 0),
            state: (Vec::new(), Vec::new()),
            event: 0,
            event_args: Vec::new(),
            building: None,
            build_log: Vec::new(),
            sign_key: String::new(),
            light_open: false,
            light: Behaviour::Hours { from: 18.0, to: 23.0 },
            light_color: [255, 220, 160],
            light_range: 12.0,
            message: None,
            pending: None,
        }
    }
}

fn read_text(path: &Path) -> String {
    std::fs::read(path).map(|b| omsi_cfg::decode_text(&b).replace("\r\n", "\n")).unwrap_or_default()
}

/// The `[lua_script]` files a `.sco` names, relative to it.
fn lua_scripts_of(sco: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = sco.lines().map(str::trim);
    while let Some(l) = lines.next() {
        if l.eq_ignore_ascii_case("[lua_script]") {
            if let Some(f) = lines.next() {
                let f = f.split('#').next().unwrap_or("").trim();
                if !f.is_empty() {
                    out.push(f.to_string());
                }
            }
        }
    }
    out
}

fn mono(t: impl Into<String>) -> RichText {
    RichText::new(t).monospace()
}

impl ScriptsUi {
    /// Open the window on the project (called from the menu and the command search).
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.files_stale = true;
    }

    /// Open the window on a project file, and run the scripts when asked.
    pub fn open_on(&mut self, rel: &str, run: bool, breakpoint: Option<usize>) {
        self.open = true;
        self.files_stale = true;
        self.pending = Some((rel.to_string(), run, breakpoint));
    }

    fn tab_of(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.path == path)
    }

    fn open_tab(&mut self, kind: TabKind, path: PathBuf, writable: bool) {
        if let Some(i) = self.tab_of(&path) {
            self.active = i;
            return;
        }
        let text = read_text(&path);
        let title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.tabs.push(Tab { kind, path, title, saved: text.clone(), text, writable, problems: Vec::new(), stale: true, code: CodeState::default() });
        self.active = self.tabs.len() - 1;
    }

    fn check(&self, tab: &Tab) -> Vec<Problem> {
        let modules: Vec<String> = self.files.iter().filter(|f| f.kind == Kind::Module).map(|f| f.require_name()).collect();
        match (&tab.kind, tab.lang()) {
            (TabKind::Osc { vars, strs, consts }, _) => {
                let mut p: Vec<Problem> = scripting::check_osc(&tab.text, vars, strs, consts).into_iter().map(|d| Problem { line: d.line, message: d.message, error: true }).collect();
                // (the variables it uses that the object's list lacks: the game reads them as 0)
                let (nums, _) = scripting::used_vars(&tab.text);
                let missing: Vec<String> = nums.into_iter().filter(|n| !vars.iter().any(|v| v.eq_ignore_ascii_case(n))).collect();
                if !missing.is_empty() {
                    p.push(Problem { line: 0, message: format!("{}: {}", tr("Not in the object's variable list", "Нет в списке переменных объекта"), missing.join(", ")), error: false });
                }
                p
            }
            (TabKind::Object, Lang::Lua) => lint::check(&tab.text, &lint::Context { modules: &[], object: true }),
            (TabKind::Project(rel), Lang::Lua) if rel == "main.lua" || rel.starts_with("openmapper/") => Vec::new(),
            (_, Lang::Lua) => lint::check(&tab.text, &lint::Context { modules: &modules, object: false }),
            _ => Vec::new(),
        }
    }

    fn save_tab(&mut self, i: usize, project: &Project, friendly: &str) {
        let Some(t) = self.tabs.get_mut(i) else { return };
        if !t.writable || !t.dirty() {
            return;
        }
        let text = if matches!(t.kind, TabKind::Osc { .. }) { t.text.replace('\n', "\r\n") } else { t.text.clone() };
        let bytes = if matches!(t.kind, TabKind::Osc { .. }) { omsi_cfg::encode_text(&text) } else { text.into_bytes() };
        match crate::doc::safe::write_atomic(&t.path, &bytes) {
            Ok(()) => {
                t.saved = t.text.clone();
                crate::hot::own_change();
                if matches!(t.kind, TabKind::Project(_)) {
                    let _ = project.write_made(friendly);
                }
                self.message = Some((format!("{} {}", tr("Saved", "Сохранено"), t.title), false));
            }
            Err(e) => self.message = Some((e.to_string(), true)),
        }
    }

    fn save_all(&mut self, project: &Project, friendly: &str) {
        for i in 0..self.tabs.len() {
            self.save_tab(i, project, friendly);
        }
    }

    /// The scripts of objects on the map (read once, again on Refresh).
    fn find_object_scripts(&mut self, ed: &Editor, roots: &Roots) {
        let Some(doc) = ed.doc.as_ref() else { return };
        let mut counts: HashMap<String, usize> = HashMap::new();
        for t in doc.tiles.values() {
            for o in t.tile.objects.iter().chain(t.tile.attach_objects.iter()) {
                *counts.entry(o.file.to_lowercase()).or_default() += 1;
            }
        }
        let mut names: HashMap<String, String> = HashMap::new();
        for t in doc.tiles.values() {
            for o in t.tile.objects.iter().chain(t.tile.attach_objects.iter()) {
                names.entry(o.file.to_lowercase()).or_insert_with(|| o.file.clone());
            }
        }
        let mut out = Vec::new();
        for (key, rel) in names {
            let Some(sco) = editable::sco_path(roots, &rel) else { continue };
            let text = omsi_cfg::decode_text(&std::fs::read(&sco).unwrap_or_default());
            if !text.to_ascii_lowercase().contains("[lua_script]") {
                continue;
            }
            for s in lua_scripts_of(&text) {
                let script = omsi_cfg::resolve_path(sco.parent().unwrap_or(Path::new("")), &s);
                out.push(ObjScript { object: rel.clone(), writable: script.starts_with(&roots.content), script, count: counts[&key] });
            }
        }
        out.sort_by(|a, b| a.object.to_lowercase().cmp(&b.object.to_lowercase()));
        self.objects = Some(out);
    }

    /// A Lua script for the selected object's type: written beside its `.sco` and named in it.
    fn add_object_script(&mut self, ed: &Editor, roots: &Roots, rel: &str) -> anyhow::Result<PathBuf> {
        let _ = ed;
        let (dir, stem) = editable::folder_of(roots, rel);
        let name = format!("{}.lua", project::file_stem(&stem));
        let path = dir.join(&name);
        if let Some(mut src) = editable::load(roots, rel) {
            // (an openMapper object: its .sco is written from its source, so the line goes there)
            if !path.exists() {
                std::fs::write(&path, project::OBJECT_TEMPLATE)?;
            }
            if !lua_scripts_of(&src.extra).iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                src.extra.push_str(&format!("\n[lua_script]\n{name}\n"));
                editable::save(roots, rel, &src)?;
            }
        } else {
            let sco = editable::sco_path(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: no .sco"))?;
            anyhow::ensure!(sco.starts_with(&roots.content), "{}", tr("an object of the OMSI 2 installation is never changed: make it an object of your own first (its properties: Make editable)", "объект установки OMSI 2 не меняется: сначала сделайте его своим (его свойства: Сделать редактируемым)"));
            let dir = sco.parent().unwrap_or(Path::new("")).to_path_buf();
            let path = dir.join(&name);
            if !path.exists() {
                std::fs::write(&path, project::OBJECT_TEMPLATE)?;
            }
            let text = omsi_cfg::decode_text(&std::fs::read(&sco)?);
            if !lua_scripts_of(&text).iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                let add = format!("{}\r\n[lua_script]\r\n{name}\r\n", if text.ends_with('\n') { "" } else { "\r\n" });
                crate::doc::safe::write_atomic(&sco, &omsi_cfg::encode_text(&format!("{text}{add}")))?;
            }
            omsi_cfg::content_changed();
            crate::hot::own_change();
            return Ok(path);
        }
        omsi_cfg::content_changed();
        crate::hot::own_change();
        Ok(path)
    }

    // ------------------------------------------------------------------------------- the run

    fn breakpoints(&self) -> HashMap<String, BTreeSet<usize>> {
        let mut out = HashMap::new();
        for t in &self.tabs {
            if t.code.breakpoints.is_empty() {
                continue;
            }
            if let Ok(rel) = t.path.strip_prefix(&self.run_root) {
                out.insert(rel.to_string_lossy().replace('\\', "/"), t.code.breakpoints.clone());
            }
        }
        out
    }

    fn start_run(&mut self, ctx: &egui::Context, ed: &Editor, project: &Project, friendly: &str, step_in: bool) {
        let Some(doc) = ed.doc.as_ref() else { return };
        let _ = project.write_made(friendly);
        // the files as they stand in the tabs (unsaved changes included)
        let open: HashMap<PathBuf, String> = self.tabs.iter().map(|t| (t.path.clone(), t.text.clone())).collect();
        let lua_files = |root: &Path| -> Vec<(String, String)> {
            let mut out = Vec::new();
            let mut stack = vec![root.to_path_buf()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') || name == "dist" {
                        continue;
                    }
                    if p.is_dir() {
                        stack.push(p);
                    } else if name.ends_with(".lua") {
                        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                        let text = open.get(&p).cloned().unwrap_or_else(|| read_text(&p));
                        out.push((rel, text));
                    }
                }
            }
            out
        };
        let (root, entry, objects) = match self.target {
            Target::Map => (project.dir.clone(), "main.lua".to_string(), Vec::new()),
            Target::Object(i) => {
                let Some(o) = self.objects.as_ref().and_then(|v| v.get(i)) else { return };
                let root = o.script.parent().unwrap_or(Path::new("")).to_path_buf();
                let entry = o.script.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let mut objs = Vec::new();
                for (k, t) in &doc.tiles {
                    let org = tile_origin(*k);
                    for m in t.tile.objects.iter().filter(|m| m.file.eq_ignore_ascii_case(&o.object)) {
                        objs.push(run::Obj { id: m.id, file: m.file.clone(), pos: [org.x + m.pos[0], org.y + m.pos[1], m.pos[2]], heading: m.rot[0] });
                    }
                }
                (root, entry, objs)
            }
        };
        self.run_root = root.clone();
        let mut all = Vec::new();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for m in &t.tile.objects {
                all.push((m.id, m.file.clone(), [org.x + m.pos[0], org.y + m.pos[1], m.pos[2]]));
            }
        }
        // the ground: the tiles' heights copied for the run's thread
        let tiles: Vec<(glam::DVec2, omsi_map::Terrain)> = doc.tiles.iter().map(|(k, t)| (tile_origin(*k), t.terrain.clone())).collect();
        let size = omsi_map::tile_size() as f64;
        let ground = std::sync::Arc::new(move |x: f64, y: f64| -> Option<f64> {
            let (org, ter) = tiles.iter().find(|(o, _)| x >= o.x && y >= o.y && x < o.x + size && y < o.y + size)?;
            Some(omsi_geometry::terrain_height(ter, (x - org.x) as f32, (y - org.y) as f32) as f64)
        });
        self.console.clear();
        self.stopped = None;
        self.expanded.clear();
        self.state = Default::default();
        let setup = run::Setup {
            files: lua_files(&root),
            entry,
            map_folder: self.map_for.clone(),
            friendly: friendly.to_string(),
            map_path: doc.dir.join("global.cfg").to_string_lossy().to_string(),
            lht: doc.global.left_hand_traffic,
            ground,
            objects,
            all_objects: std::sync::Arc::new(all),
            clock: self.start_clock,
            speed: self.speed,
            breakpoints: HashMap::new(),
            step_in,
        };
        let mut setup = setup;
        setup.breakpoints = self.breakpoints();
        self.session = Some(Session::start(setup, Some(ctx.clone())));
        self.running = true;
        self.bottom = Bottom::Console;
    }

    fn poll_run(&mut self) {
        let Some(s) = &self.session else { return };
        let mut ended = false;
        for o in s.poll() {
            match o {
                Out::Line { kind, text, at } => {
                    self.console.push(Line { kind, text, at });
                    if self.console.len() > 5000 {
                        self.console.drain(..1000);
                    }
                }
                Out::Paused { reason, file, line, stack } => {
                    self.frame = 0;
                    self.expanded.clear();
                    let path = self.run_root.join(&file);
                    let kind = if self.target == Target::Map { TabKind::Project(file.clone()) } else { TabKind::Object };
                    self.open_tab(kind, path, true);
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        t.code.jump = Some(line);
                    }
                    self.stopped = Some(Stopped { reason, file, line, stack });
                    self.bottom = Bottom::Debug;
                }
                Out::Resumed => self.stopped = None,
                Out::Clock { clock, time, running } => {
                    self.clock = clock;
                    self.time = time;
                    self.running = running;
                }
                Out::Listening { events, timers } => self.listening = (events, timers),
                Out::Expanded { id, items } => {
                    self.expanded.insert(id, items);
                }
                Out::State { storage, objects } => self.state = (storage, objects),
                Out::Ended(err) => {
                    if let Some(e) = err {
                        self.console.push(Line { kind: LineKind::Error, text: e, at: None });
                    }
                    self.console.push(Line { kind: LineKind::Note, text: tr("The run ended", "Запуск окончен").into(), at: None });
                    ended = true;
                }
            }
        }
        if ended {
            self.session = None;
            self.stopped = None;
        }
    }

    fn send(&self, c: Cmd) {
        if let Some(s) = &self.session {
            s.send(c);
        }
    }

    /// A file named by a run (`scripts/x.lua`) opened at its line.
    fn goto(&mut self, file: &str, line: usize) {
        let path = self.run_root.join(file);
        let kind = if self.run_root.ends_with(project::plugin_name(&self.map_for)) { TabKind::Project(file.to_string()) } else { TabKind::Object };
        if path.is_file() {
            self.open_tab(kind, path, true);
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.code.jump = Some(line);
            }
        }
    }

    // ---------------------------------------------------------------------------- the window

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: Option<&Roots>, folder: &str, actions: &mut Vec<Action>) {
        if !self.open {
            // (a run goes on only while the window is open)
            if self.session.is_some() {
                self.session = None;
            }
            return;
        }
        let Some(roots) = roots else { return };
        let Some(doc) = ed.doc.as_ref() else { return };
        let friendly = doc.global.friendly_name.clone();
        let project = Project::of(&roots.content, folder);
        if self.map_for != folder {
            self.map_for = folder.to_string();
            self.tabs.clear();
            self.objects = None;
            self.session = None;
            self.files_stale = true;
        }
        if self.files_stale {
            if project.exists() || project.dir.join("main.lua").is_file() {
                let _ = project.create(&friendly);
            }
            self.files = project.files();
            self.files_stale = false;
        }
        let selected_object = match ed.selected {
            Some(Sel::Object(k, id)) if ed.selection().len() == 1 => crate::selection::any_object(doc, k, id).map(|o| o.file.clone()),
            _ => None,
        };
        if self.osc_for != selected_object {
            self.osc_for = selected_object.clone();
            self.osc.clear();
            if let Some(sco) = selected_object.as_deref().and_then(|r| editable::sco_path(roots, r)) {
                let listed = scripting::sco_scripts(&omsi_cfg::decode_text(&std::fs::read(&sco).unwrap_or_default()));
                let dir = sco.parent().unwrap_or(Path::new("")).to_path_buf();
                let declared = scripting::declared(&dir, &listed);
                for s in &listed.scripts {
                    self.osc.push((omsi_cfg::resolve_path(&dir, s), declared.clone()));
                }
            }
        }
        if let Some((rel, run, line)) = self.pending.take() {
            if !rel.is_empty() && project.path(&rel).is_file() {
                self.open_tab(TabKind::Project(rel.clone()), project.path(&rel), true);
                if let (Some(l), Some(t)) = (line, self.tabs.get_mut(self.active)) {
                    t.code.breakpoints.insert(l);
                }
            }
            if run {
                self.start_run(ctx, ed, &project, &friendly, false);
            }
        }
        self.poll_run();
        if let Some(rx) = &self.building {
            let mut done = false;
            for m in rx.try_iter() {
                match m {
                    Msg::Line(l) => self.build_log.push(l),
                    Msg::Done(Ok(p)) => {
                        self.build_log.push(format!("OK {}", p.display()));
                        // (the Rust plugin goes where the game loads it)
                        if p.starts_with(project.rust_dir()) {
                            let to = project.rust_dir().with_extension("oop");
                            match std::fs::copy(&p, &to) {
                                Ok(_) => self.build_log.push(format!("{} {}", tr("installed as", "установлено как"), to.display())),
                                Err(e) => self.build_log.push(e.to_string()),
                            }
                        }
                        done = true;
                    }
                    Msg::Done(Err(e)) => {
                        self.build_log.push(format!("{}: {e}", tr("failed", "не вышло")));
                        done = true;
                    }
                }
            }
            if done {
                self.building = None;
            }
        }
        for i in 0..self.tabs.len() {
            if self.tabs[i].stale {
                self.tabs[i].problems = self.check(&self.tabs[i]);
                self.tabs[i].stale = false;
            }
        }
        // the debugger's keys, while a code editor has the keyboard
        if crate::ui_code::has_focus(ctx) {
            let k = |key: egui::Key, m: egui::Modifiers| ctx.input_mut(|i| i.consume_key(m, key));
            if k(egui::Key::F5, egui::Modifiers::SHIFT) {
                self.session = None;
            } else if k(egui::Key::F5, egui::Modifiers::NONE) {
                if self.session.is_some() {
                    self.send(Cmd::Continue);
                } else {
                    self.save_all(&project, &friendly);
                    self.start_run(ctx, ed, &project, &friendly, false);
                }
            } else if k(egui::Key::F10, egui::Modifiers::NONE) {
                self.send(Cmd::StepOver);
            } else if k(egui::Key::F11, egui::Modifiers::SHIFT) {
                self.send(Cmd::StepOut);
            } else if k(egui::Key::F11, egui::Modifiers::NONE) {
                if self.session.is_none() {
                    self.start_run(ctx, ed, &project, &friendly, true);
                } else {
                    self.send(Cmd::StepIn);
                }
            }
        }

        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Code, tr("Scripts", "Скрипты"))).id(egui::Id::new("scripts ide")).open(&mut open).default_size(Vec2::new(1120.0, 720.0)).min_size(Vec2::new(640.0, 360.0)).resizable(true).show(ctx, |ui| {
            self.toolbar(ui, ed, &project, &friendly);
            ui.separator();
            egui::Panel::left("scripts tree").resizable(true).default_size(240.0).size_range(170.0..=420.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("scripts tree scroll").show(ui, |ui| self.tree(ui, ed, roots, &project, &friendly, selected_object.as_deref()));
            });
            if self.show_api {
                egui::Panel::right("scripts api").resizable(true).default_size(300.0).show(ui, |ui| self.api_panel(ui));
            }
            egui::Panel::bottom("scripts bottom").resizable(true).default_size(210.0).size_range(90.0..=520.0).show(ui, |ui| self.bottom_panel(ui));
            egui::CentralPanel::default().show(ui, |ui| self.editor_area(ui, &project, &friendly));
        });
        self.open = open;
        self.dialogs(ctx, &project, &friendly, actions);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ed: &Editor, project: &Project, friendly: &str) {
        ui.horizontal_wrapped(|ui| {
            let running = self.session.is_some();
            let paused = self.stopped.is_some();
            if !running {
                if ui.button(icons::labelled(Icon::Play, tr("Run", "Запустить"))).on_hover_text(tr("Run the scripts here, in the editor, with the game's functions (F5 in the code)", "Запустить скрипты здесь, в редакторе, с функциями игры (F5 в коде)")).clicked() {
                    self.save_all(project, friendly);
                    self.start_run(ui.ctx(), ed, project, friendly, false);
                }
                if ui.button(tr("Step in", "Войти")).on_hover_text(tr("Run and stop at the first line (F11)", "Запустить и остановиться на первой строке (F11)")).clicked() {
                    self.start_run(ui.ctx(), ed, project, friendly, true);
                }
            } else {
                if paused || !self.running {
                    if ui.button(icons::labelled(Icon::Play, tr("Continue", "Продолжить"))).on_hover_text("F5").clicked() {
                        self.send(Cmd::Continue);
                        self.running = true;
                    }
                } else if ui.button(tr("Pause", "Пауза")).on_hover_text(tr("Stop the clock; a script running stops at its next line", "Остановить часы; работающий скрипт остановится на следующей строке")).clicked() {
                    self.send(Cmd::Pause);
                    self.running = false;
                }
                ui.add_enabled_ui(paused, |ui| {
                    if ui.button(tr("Over", "Шаг")).on_hover_text(tr("Step over (F10)", "Шаг с обходом (F10)")).clicked() {
                        self.send(Cmd::StepOver);
                    }
                    if ui.button(tr("Into", "Внутрь")).on_hover_text(tr("Step into (F11)", "Шаг внутрь (F11)")).clicked() {
                        self.send(Cmd::StepIn);
                    }
                    if ui.button(tr("Out", "Наружу")).on_hover_text(tr("Step out (Shift+F11)", "Шаг наружу (Shift+F11)")).clicked() {
                        self.send(Cmd::StepOut);
                    }
                });
                if ui.button(tr("Stop", "Стоп")).on_hover_text("Shift+F5").clicked() {
                    self.session = None;
                    self.stopped = None;
                }
                if ui.button(tr("Restart", "Заново")).clicked() {
                    self.session = None;
                    self.save_all(project, friendly);
                    self.start_run(ui.ctx(), ed, project, friendly, false);
                }
            }
            ui.separator();
            // what runs
            let objects = self.objects.clone().unwrap_or_default();
            let label = match self.target {
                Target::Map => tr("The map's scripts", "Скрипты карты").to_string(),
                Target::Object(i) => objects.get(i).map(|o| o.script.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).unwrap_or_default(),
            };
            ui.add_enabled_ui(!running, |ui| {
                egui::ComboBox::from_id_salt("run target").selected_text(label).width(170.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.target, Target::Map, tr("The map's scripts", "Скрипты карты"));
                    for (i, o) in objects.iter().enumerate() {
                        let name = format!("{} ({} ×{})", o.script.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), o.object.rsplit('\\').next().unwrap_or(&o.object), o.count);
                        ui.selectable_value(&mut self.target, Target::Object(i), name);
                    }
                });
            });
            // the clock
            let shown = if running { self.clock } else { self.start_clock };
            let s = shown as i64;
            ui.label(mono(format!("{:02}:{:02}:{:02}", s / 3600 % 24, s / 60 % 60, s % 60))).on_hover_text(tr("The game's clock in the run", "Часы игры в запуске"));
            if running {
                let mut h = self.clock / 3600.0;
                if ui.add(egui::DragValue::new(&mut h).range(0.0..=23.99).speed(0.02).fixed_decimals(2)).on_hover_text(tr("Set the game's clock of the run (sends minute)", "Переставить часы игры в запуске (посылает minute)")).changed() {
                    self.send(Cmd::SetClock(h * 3600.0));
                }
            } else {
                let mut h = self.start_clock / 3600.0;
                if ui.add(egui::DragValue::new(&mut h).range(0.0..=23.99).speed(0.05).fixed_decimals(2).prefix(tr("starts ", "старт "))).on_hover_text(tr("The hour the run starts at", "Час, с которого начинается запуск")).changed() {
                    self.start_clock = h * 3600.0;
                }
            }
            egui::ComboBox::from_id_salt("run speed").selected_text(format!("×{}", self.speed)).width(64.0).show_ui(ui, |ui| {
                for s in [0.0, 1.0, 2.0, 10.0, 60.0, 600.0] {
                    if ui.selectable_value(&mut self.speed, s, format!("×{s}")).clicked() {
                        self.send(Cmd::Speed(s));
                    }
                }
            });
            ui.separator();
            let dirty = self.tabs.iter().any(|t| t.dirty());
            if ui.add_enabled(dirty, egui::Button::new(icons::labelled(Icon::Save, tr("Save all", "Сохранить всё")))).on_hover_text("Ctrl+S").clicked() {
                self.save_all(project, friendly);
            }
            if ui.add_enabled(self.building.is_none() && project.exists(), egui::Button::new(tr("Build .oop", "Собрать .oop"))).on_hover_text(tr("One file of the map's scripts to share: compiled and obfuscated, with the permissions they need (the Development Tools' compiler)", "Один файл со скриптами карты, чтобы делиться: скомпилирован и запутан, с нужными разрешениями (компилятор Development Tools)")).clicked() {
                self.save_all(project, friendly);
                self.build_log.clear();
                let key = (!self.sign_key.trim().is_empty()).then(|| PathBuf::from(self.sign_key.trim()));
                self.building = Some(build::spawn(project.dir.clone(), key, ui.ctx().clone()));
                self.bottom = Bottom::Build;
            }
            ui.toggle_value(&mut self.show_api, tr("API", "API")).on_hover_text(tr("The game's functions and events", "Функции и события игры"));
            if let Some((m, bad)) = &self.message {
                ui.label(RichText::new(m).small().color(if *bad { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() }));
            }
        });
    }

    fn tree(&mut self, ui: &mut egui::Ui, ed: &Editor, roots: &Roots, project: &Project, friendly: &str, selected: Option<&str>) {
        let weak = ui.visuals().weak_text_color();
        if !project.exists() {
            ui.label(RichText::new(tr("This map has no scripts yet. Its scripts run in openOMSI while it is played: they react to what happens (a stop, an hour, a trip), show messages, change the traffic and the weather.", "У этой карты пока нет скриптов. Её скрипты выполняются в openOMSI, пока на ней играют: реагируют на происходящее (остановка, час, рейс), показывают сообщения, меняют движение и погоду.")).small().color(weak));
            if ui.button(icons::labelled(Icon::Add, tr("Start the map's scripts", "Начать скрипты карты"))).clicked() {
                match project.create(friendly).and_then(|_| project.add(Kind::Script, "main", project::templates()[0].text, friendly)) {
                    Ok(f) => {
                        self.files_stale = true;
                        self.open_tab(TabKind::Project(f.rel.clone()), project.path(&f.rel), true);
                    }
                    Err(e) => self.message = Some((e.to_string(), true)),
                }
            }
            ui.separator();
        } else {
            self.project_tree(ui, project, friendly);
        }
        // the scripts of objects
        ui.add_space(6.0);
        let header = egui::CollapsingHeader::new(RichText::new(tr("Object scripts (Lua)", "Скрипты объектов (Lua)")).strong()).id_salt("tree objects").default_open(false).show(ui, |ui| {
            if self.objects.is_none() {
                self.find_object_scripts(ed, roots);
            }
            let list = self.objects.clone().unwrap_or_default();
            if list.is_empty() {
                ui.label(RichText::new(tr("No object on the map has a Lua script.", "Ни у одного объекта на карте нет Lua-скрипта.")).small().color(weak));
            }
            for (i, o) in list.iter().enumerate() {
                let name = o.script.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let r = ui.selectable_label(self.tabs.get(self.active).is_some_and(|t| t.path == o.script), format!("{name}  ·  ×{}", o.count)).on_hover_text(&o.object);
                if r.clicked() {
                    self.open_tab(TabKind::Object, o.script.clone(), o.writable);
                    if self.session.is_none() {
                        self.target = Target::Object(i);
                    }
                }
            }
            if ui.small_button(tr("Refresh", "Обновить")).clicked() {
                self.objects = None;
            }
        });
        header.header_response.on_hover_text(tr("Scripts a scenery object names in [lua_script]: one runs for every object of its type near the camera (openOMSI only)", "Скрипты, названные в [lua_script] сценарного объекта: один работает для всех объектов его типа рядом с камерой (только openOMSI)"));
        // the selected object
        egui::CollapsingHeader::new(RichText::new(tr("Selected object", "Выделенный объект")).strong()).id_salt("tree selected").default_open(true).show(ui, |ui| match selected {
            None => {
                ui.label(RichText::new(tr("Select one object on the map to see its scripts.", "Выделите на карте один объект, чтобы увидеть его скрипты.")).small().color(weak));
            }
            Some(rel) => {
                crate::ui::long_label(ui, rel.rsplit('\\').next().unwrap_or(rel), egui::TextStyle::Small, weak);
                for (path, declared) in self.osc.clone() {
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if ui.selectable_label(self.tabs.get(self.active).is_some_and(|t| t.path == path), format!("{name}  ·  OMSI 2")).clicked() {
                        let writable = path.starts_with(&roots.content);
                        self.open_tab(TabKind::Osc { vars: declared.0.clone(), strs: declared.1.clone(), consts: declared.2.clone() }, path.clone(), writable);
                    }
                }
                if self.osc.is_empty() {
                    ui.label(RichText::new(tr("No OMSI 2 script (.osc).", "Нет скрипта OMSI 2 (.osc).")).small().color(weak));
                }
                if ui.button(icons::labelled(Icon::Add, tr("Add a Lua script to its type", "Добавить Lua-скрипт его типу"))).on_hover_text(tr("A script for every object of this type: written beside its .sco and named in it ([lua_script]); OMSI 2 skips it", "Скрипт для всех объектов этого типа: пишется рядом с его .sco и называется в нём ([lua_script]); OMSI 2 его пропускает")).clicked() {
                    match self.add_object_script(ed, roots, rel) {
                        Ok(p) => {
                            self.objects = None;
                            self.open_tab(TabKind::Object, p, true);
                        }
                        Err(e) => self.message = Some((e.to_string(), true)),
                    }
                }
            }
        });
        // the Rust plugin
        egui::CollapsingHeader::new(RichText::new(tr("Rust plugin", "Плагин на Rust")).strong()).id_salt("tree rust").default_open(false).show(ui, |ui| {
            let dir = project.rust_dir();
            if !dir.join("Cargo.toml").is_file() {
                ui.label(RichText::new(tr("A plugin of the map written in Rust, compiled to WebAssembly into an .oop. Building it needs Rust (rustup) with the wasm32-unknown-unknown target.", "Плагин карты на Rust, скомпилированный в WebAssembly в .oop. Для сборки нужен Rust (rustup) с целью wasm32-unknown-unknown.")).small().color(weak));
                if ui.button(icons::labelled(Icon::Add, tr("Make the Rust plugin", "Создать плагин на Rust"))).clicked() {
                    match build::create_rust(&dir, &self.map_for, friendly) {
                        Ok(()) => self.open_tab(TabKind::Rust, dir.join("src").join("lib.rs"), true),
                        Err(e) => self.message = Some((e, true)),
                    }
                }
                return;
            }
            for rel in ["src/lib.rs", "Cargo.toml", "openomsi-plugin.toml"] {
                let p = dir.join(rel);
                if p.is_file() && ui.selectable_label(self.tabs.get(self.active).is_some_and(|t| t.path == p), mono(rel)).clicked() {
                    self.open_tab(TabKind::Rust, p, true);
                }
            }
            if ui.add_enabled(self.building.is_none(), egui::Button::new(tr("Build and install", "Собрать и установить"))).on_hover_text(tr("cargo builds it, the .oop goes into the content folder's plugins", "cargo собирает его, .oop кладётся в plugins папки контента")).clicked() {
                self.save_all(project, friendly);
                self.build_log.clear();
                let key = (!self.sign_key.trim().is_empty()).then(|| PathBuf::from(self.sign_key.trim()));
                self.building = Some(build::spawn(dir.clone(), key, ui.ctx().clone()));
                self.bottom = Bottom::Build;
            }
        });
        ui.add_space(6.0);
        if ui.small_button(icons::labelled(Icon::Add, tr("A light switched by a script…", "Свет, включаемый скриптом…"))).clicked() {
            self.light_open = true;
        }
    }

    fn project_tree(&mut self, ui: &mut egui::Ui, project: &Project, friendly: &str) {
        let weak = ui.visuals().weak_text_color();
        let files = self.files.clone();
        let section = |ui: &mut egui::Ui, this: &mut Self, title: &str, kind: Kind, help: &str| {
            let r = egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("tree", title)).default_open(true).show(ui, |ui| {
                let list: Vec<&project::File> = files.iter().filter(|f| f.kind == kind).collect();
                if list.is_empty() {
                    ui.label(RichText::new(tr("(none)", "(нет)")).small().color(weak));
                }
                for f in list {
                    let path = project.path(&f.rel);
                    let open = this.tabs.get(this.active).is_some_and(|t| t.path == path);
                    let problems = this.tabs.iter().find(|t| t.path == path).map(|t| t.problems.iter().filter(|p| p.error).count()).unwrap_or(0);
                    ui.horizontal(|ui| {
                        if kind == Kind::Script {
                            let mut on = f.on;
                            if ui.checkbox(&mut on, "").on_hover_text(tr("Starts with the map", "Запускается с картой")).changed() {
                                if let Err(e) = project.set_on(f.name(), on, friendly) {
                                    this.message = Some((e.to_string(), true));
                                }
                                this.files_stale = true;
                            }
                        }
                        let mut label = RichText::new(if kind == Kind::Module { f.require_name() } else { f.name().to_string() });
                        if !f.on {
                            label = label.color(weak).strikethrough();
                        }
                        if problems > 0 {
                            label = label.color(ui.visuals().error_fg_color);
                        }
                        let r = ui.selectable_label(open, label);
                        if r.clicked() {
                            let kind = TabKind::Project(f.rel.clone());
                            this.open_tab(kind, path.clone(), f.kind != Kind::Made);
                        }
                        r.context_menu(|ui| {
                            if f.kind != Kind::Made && f.kind != Kind::Manifest {
                                if ui.button(tr("Rename…", "Переименовать…")).clicked() {
                                    this.rename = Some((f.rel.clone(), f.name().trim_end_matches(".lua").to_string()));
                                    ui.close();
                                }
                                if kind == Kind::Script {
                                    if ui.button(tr("Earlier in the order", "Раньше по порядку")).clicked() {
                                        let _ = project.move_script(f.name(), -1, friendly);
                                        this.files_stale = true;
                                        ui.close();
                                    }
                                    if ui.button(tr("Later in the order", "Позже по порядку")).clicked() {
                                        let _ = project.move_script(f.name(), 1, friendly);
                                        this.files_stale = true;
                                        ui.close();
                                    }
                                }
                                if ui.button(tr("Delete (into openMapper's bin)", "Удалить (в корзину openMapper)")).clicked() {
                                    match project.delete(project.dir.parent().and_then(Path::parent).unwrap_or(Path::new("")), &f.rel, friendly) {
                                        Ok(()) => {
                                            this.tabs.retain(|t| t.path != path);
                                            this.active = this.active.min(this.tabs.len().saturating_sub(1));
                                        }
                                        Err(e) => this.message = Some((e.to_string(), true)),
                                    }
                                    this.files_stale = true;
                                    ui.close();
                                }
                            }
                            if ui.button(tr("Show in the file manager", "Показать в файловом менеджере")).clicked() {
                                crate::app::reveal(&path);
                                ui.close();
                            }
                        });
                    });
                }
                if kind != Kind::Data && ui.small_button(icons::labelled(Icon::Add, if kind == Kind::Script { tr("New script", "Новый скрипт") } else { tr("New module", "Новый модуль") })).clicked() {
                    this.new_file = Some(NewFile { kind, name: String::new(), template: if kind == Kind::Module { 1 } else { 0 } });
                }
            });
            r.header_response.on_hover_text(help);
        };
        section(ui, self, tr("Scripts", "Скрипты"), Kind::Script, tr("Each starts when the map is played, in this order (right-click to move one); one that fails leaves the others running. Use map.on, map.every, map.after: they act on this map only.", "Каждый запускается, когда играют на карте, в этом порядке (правый клик, чтобы переместить); упавший не мешает остальным. Используйте map.on, map.every, map.after: они действуют только на этой карте."));
        section(ui, self, tr("Modules", "Модули"), Kind::Module, tr("Code the scripts share: local util = require(\"modules.util\")", "Код, общий для скриптов: local util = require(\"modules.util\")"));
        section(ui, self, tr("Data", "Данные"), Kind::Data, tr("Files the scripts read (data/), pictures and sounds", "Файлы, которые читают скрипты (data/), картинки и звуки"));
        egui::CollapsingHeader::new(RichText::new(tr("Made by openMapper", "Сделано openMapper")).small()).id_salt("tree made").default_open(false).show(ui, |ui| {
            for f in files.iter().filter(|f| matches!(f.kind, Kind::Made | Kind::Manifest)) {
                let path = project.path(&f.rel);
                if ui.selectable_label(self.tabs.get(self.active).is_some_and(|t| t.path == path), mono(&f.rel)).clicked() {
                    self.open_tab(TabKind::Project(f.rel.clone()), path, f.kind == Kind::Manifest);
                }
            }
        });
    }

    fn editor_area(&mut self, ui: &mut egui::Ui, project: &Project, friendly: &str) {
        if self.tabs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(tr("Open a script on the left, or make a new one.\n\nF5 runs the scripts here in the editor, F9 sets a breakpoint, F10 / F11 step,\nCtrl+Space completes, Ctrl+F finds, Ctrl+G goes to a line, Ctrl+/ comments out.", "Откройте скрипт слева или создайте новый.\n\nF5 запускает скрипты здесь, в редакторе, F9 ставит точку останова, F10 / F11 шаг,\nCtrl+Space дополняет, Ctrl+F ищет, Ctrl+G переходит к строке, Ctrl+/ комментирует.")).color(ui.visuals().weak_text_color()));
            });
            return;
        }
        // the tabs
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (i, t) in self.tabs.iter().enumerate() {
                let title = format!("{}{}", t.title, if t.dirty() { " *" } else { "" });
                let r = ui.selectable_label(i == self.active, title).on_hover_text(t.path.display().to_string());
                if r.clicked() {
                    self.active = i;
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
                if ui.small_button("x").clicked() {
                    close = Some(i);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(i) = close {
            self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            if self.tabs.is_empty() {
                return;
            }
        }
        ui.separator();
        let i = self.active.min(self.tabs.len() - 1);
        let modules: Vec<String> = self.files.iter().filter(|f| f.kind == Kind::Module).map(|f| f.require_name()).collect();
        let exec = self.stopped.as_ref().filter(|s| self.run_root.join(&s.file) == self.tabs[i].path).map(|s| {
            // (the frame picked in the stack, where it is in this file)
            s.stack.get(self.frame).filter(|f| self.run_root.join(&f.file) == self.tabs[i].path).map(|f| f.line).unwrap_or(s.line)
        });
        let save;
        let help;
        {
            let t = &mut self.tabs[i];
            if !t.writable {
                ui.label(RichText::new(tr("Read only: a file of the installation or made by openMapper", "Только чтение: файл установки или сделанный openMapper")).small().color(ui.visuals().weak_text_color()));
            }
            let problems: Vec<(usize, bool)> = t.problems.iter().map(|p| (p.line, p.error)).collect();
            let lang = t.lang();
            let opts = ui_code::Opts { lang, writable: t.writable, problems: &problems, exec_line: exec, extra_words: &modules };
            let before = t.code.breakpoints.clone();
            let avail = ui.available_height() - 22.0;
            let out = ui.allocate_ui(Vec2::new(ui.available_width(), avail.max(80.0)), |ui| ui_code::code_editor(ui, egui::Id::new(("code", &t.path)), &mut t.text, &mut t.code, &opts)).inner;
            if out.changed {
                t.stale = true;
            }
            save = out.save;
            help = out.help;
            if t.code.breakpoints != before {
                let b = self.breakpoints();
                self.send(Cmd::Breakpoints(b));
            }
        }
        if save {
            self.save_tab(i, project, friendly);
        }
        // the status line: the cursor's line, the problems, help on the name under it
        let t = &self.tabs[i];
        ui.horizontal(|ui| {
            let errs = t.problems.iter().filter(|p| p.error).count();
            let warns = t.problems.len() - errs;
            ui.label(RichText::new(format!("{} {}", tr("Line", "Строка"), t.code.cursor_line)).small());
            if errs + warns > 0 {
                let r = ui.label(RichText::new(format!("{errs} {} · {warns} {}", tr("errors", "ошибок"), tr("warnings", "предупреждений"))).small().color(if errs > 0 { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color }));
                if r.clicked() {
                    self.bottom = Bottom::Problems;
                }
            }
            if let Some(h) = help {
                ui.label(RichText::new(h.lines().next().unwrap_or("")).small().monospace()).on_hover_text(h);
            }
        });
    }

    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let n: usize = self.tabs.iter().map(|t| t.problems.len()).sum();
            ui.selectable_value(&mut self.bottom, Bottom::Problems, format!("{} ({n})", tr("Problems", "Проблемы")));
            ui.selectable_value(&mut self.bottom, Bottom::Console, tr("Console", "Консоль"));
            ui.selectable_value(&mut self.bottom, Bottom::Debug, tr("Debugger", "Отладчик"));
            ui.selectable_value(&mut self.bottom, Bottom::Events, tr("Events", "События"));
            ui.selectable_value(&mut self.bottom, Bottom::Build, tr("Build", "Сборка"));
        });
        ui.separator();
        match self.bottom {
            Bottom::Problems => self.problems_view(ui),
            Bottom::Console => self.console_view(ui),
            Bottom::Debug => self.debug_view(ui),
            Bottom::Events => self.events_view(ui),
            Bottom::Build => self.build_view(ui),
        }
    }

    fn problems_view(&mut self, ui: &mut egui::Ui) {
        let mut go = None;
        egui::ScrollArea::vertical().id_salt("problems").auto_shrink([false, false]).show(ui, |ui| {
            let mut any = false;
            for (i, t) in self.tabs.iter().enumerate() {
                for p in &t.problems {
                    any = true;
                    let color = if p.error { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
                    let text = format!("{}  {}:{}  {}", if p.error { tr("error", "ошибка") } else { tr("warning", "предупреждение") }, t.title, p.line, p.message);
                    if ui.selectable_label(false, RichText::new(text).color(color)).clicked() {
                        go = Some((i, p.line));
                    }
                }
            }
            if !any {
                ui.label(RichText::new(tr("No problems in the open files.", "В открытых файлах проблем нет.")).color(ui.visuals().weak_text_color()));
            }
        });
        if let Some((i, l)) = go {
            self.active = i;
            self.tabs[i].code.jump = Some(l);
        }
    }

    fn console_view(&mut self, ui: &mut egui::Ui) {
        let mut go = None;
        let h = ui.available_height() - 28.0;
        egui::ScrollArea::vertical().id_salt("console").auto_shrink([false, false]).max_height(h.max(40.0)).stick_to_bottom(true).show(ui, |ui| {
            if self.console.is_empty() {
                ui.label(RichText::new(tr("What the scripts print, show on the screen and the errors they make appear here when they run (Run, F5).", "Здесь появляется то, что скрипты печатают, показывают на экране, и их ошибки, когда они запущены (Запустить, F5).")).color(ui.visuals().weak_text_color()));
            }
            for l in &self.console {
                let color = match l.kind {
                    LineKind::Print => ui.visuals().text_color(),
                    LineKind::Screen => Color32::from_rgb(90, 170, 230),
                    LineKind::Warn => ui.visuals().warn_fg_color,
                    LineKind::Error => ui.visuals().error_fg_color,
                    LineKind::Note | LineKind::Result => ui.visuals().weak_text_color(),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&l.text).monospace().color(color));
                    if let Some((f, n)) = &l.at {
                        if ui.small_button(format!("{f}:{n}")).clicked() {
                            go = Some((f.clone(), *n));
                        }
                    }
                });
            }
        });
        if let Some((f, n)) = go {
            self.goto(&f, n);
        }
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.console_input).font(egui::TextStyle::Monospace).hint_text(tr("Lua to run in the scripts (in the frame stopped at, if stopped); Up and Down: history", "Lua для выполнения в скриптах (в кадре остановки, если остановлено); стрелки вверх и вниз: история")).desired_width(ui.available_width() - 70.0));
            if r.has_focus() && !self.history.is_empty() {
                if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                    self.history_at = self.history_at.saturating_sub(1);
                    self.console_input = self.history[self.history_at].clone();
                }
                if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                    self.history_at = (self.history_at + 1).min(self.history.len());
                    self.console_input = self.history.get(self.history_at).cloned().unwrap_or_default();
                }
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button(tr("Run", "Выполнить")).clicked() || enter) && !self.console_input.trim().is_empty() {
                if self.session.is_none() {
                    self.console.push(Line { kind: LineKind::Note, text: tr("Start a run first (Run, F5)", "Сначала запустите (Запустить, F5)").into(), at: None });
                } else {
                    let frame = self.stopped.as_ref().map(|_| self.frame);
                    self.send(Cmd::Eval(self.console_input.clone(), frame));
                    self.history.push(std::mem::take(&mut self.console_input));
                    self.history_at = self.history.len();
                }
                r.request_focus();
            }
            if ui.small_button(tr("Clear", "Очистить")).clicked() {
                self.console.clear();
            }
        });
    }

    fn value_row(&mut self, ui: &mut egui::Ui, name: &str, v: &Val, depth: usize) {
        match v.table {
            Some(id) if depth < 6 => {
                let r = egui::CollapsingHeader::new(mono(format!("{name} = {}", v.text))).id_salt(("val", id, name, depth)).show(ui, |ui| {
                    match self.expanded.get(&id).cloned() {
                        Some(items) => {
                            for (k, v) in items.iter().take(300) {
                                self.value_row(ui, k, v, depth + 1);
                            }
                            if items.is_empty() {
                                ui.label(RichText::new(tr("(empty)", "(пусто)")).small());
                            }
                        }
                        None => {
                            ui.label(RichText::new("…").small());
                        }
                    }
                });
                if r.fully_open() && !self.expanded.contains_key(&id) {
                    self.send(Cmd::Expand(id));
                    self.expanded.insert(id, Vec::new());
                }
            }
            _ => {
                ui.label(mono(format!("{name} = {}", v.text))).on_hover_text(v.ty);
            }
        }
    }

    fn debug_view(&mut self, ui: &mut egui::Ui) {
        let Some(st) = self.stopped.as_ref() else {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(if self.session.is_some() { tr("Running. A breakpoint (a click left of a line number, or F9) stops the scripts there; Pause stops them at their next line.", "Работает. Точка останова (клик слева от номера строки или F9) остановит скрипты там; Пауза остановит на следующей строке.") } else { tr("Not running. Run (F5) the scripts; a breakpoint stops them, and here are the call stack, the variables of each function and the values to watch.", "Не запущено. Запустите (F5) скрипты; точка останова их остановит, а здесь будут стек вызовов, переменные каждой функции и значения для наблюдения.") }).color(ui.visuals().weak_text_color()));
            });
            if !self.state.0.is_empty() || !self.state.1.is_empty() {
                self.state_view(ui);
            }
            return;
        };
        let stack = st.stack.clone();
        let (reason, file, line) = (st.reason.clone(), st.file.clone(), st.line);
        ui.label(RichText::new(format!("{}: {file}:{line} ({reason})", tr("Stopped", "Остановлено"))).strong());
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("stack").auto_shrink([false, false]).show(&mut cols[0], |ui| {
                ui.label(RichText::new(tr("Call stack", "Стек вызовов")).small().strong());
                for (i, f) in stack.iter().enumerate() {
                    if ui.selectable_label(i == self.frame, mono(format!("{}  {}:{}", f.func, f.file, f.line))).clicked() {
                        self.frame = i;
                        let (f, l) = (f.file.clone(), f.line);
                        self.goto(&f, l);
                    }
                }
            });
            egui::ScrollArea::vertical().id_salt("vars").auto_shrink([false, false]).show(&mut cols[1], |ui| {
                if let Some(f) = stack.get(self.frame) {
                    ui.label(RichText::new(tr("Locals", "Локальные")).small().strong());
                    for (n, v) in &f.locals {
                        self.value_row(ui, n, v, 0);
                    }
                    if !f.upvalues.is_empty() {
                        ui.label(RichText::new(tr("Upvalues", "Внешние (upvalues)")).small().strong());
                        for (n, v) in &f.upvalues {
                            self.value_row(ui, n, v, 0);
                        }
                    }
                }
                ui.label(RichText::new(tr("The console evaluates in this frame: a name, an expression, an assignment.", "Консоль вычисляет в этом кадре: имя, выражение, присваивание.")).small().color(ui.visuals().weak_text_color()));
            });
        });
    }

    fn state_view(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr("Storage and objects", "Хранилище и объекты")).small().strong());
        egui::ScrollArea::vertical().id_salt("state").show(ui, |ui| {
            for (k, v) in &self.state.0 {
                ui.label(mono(format!("storage.{k} = {v}")));
            }
            for (id, vars) in &self.state.1 {
                ui.label(mono(format!("#{id}: {}", vars.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("  "))));
            }
        });
    }

    fn events_view(&mut self, ui: &mut egui::Ui) {
        let api = crate::script::api::api();
        ui.horizontal(|ui| {
            ui.label(tr("Send", "Послать"));
            let names: Vec<String> = api.events.iter().map(|e| e.name.clone()).collect();
            let mut pick = self.event.min(names.len().saturating_sub(1));
            egui::ComboBox::from_id_salt("event pick").selected_text(&names[pick]).width(180.0).height(400.0).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    let listening = self.listening.0.iter().find(|x| &x.0 == n).map(|x| format!("  ({})", x.1)).unwrap_or_default();
                    ui.selectable_value(&mut pick, i, format!("{n}{listening}"));
                }
            });
            if pick != self.event || self.event_args.len() != api.events[pick].args.len() {
                self.event = pick;
                self.event_args = run::default_args(&api.events[pick], self.clock);
            }
            let ev = &api.events[pick];
            for (a, v) in ev.args.iter().zip(self.event_args.iter_mut()) {
                ui.label(RichText::new(a).small());
                ui.add(egui::TextEdit::singleline(v).desired_width(90.0).font(egui::TextStyle::Monospace));
            }
            if ui.add_enabled(self.session.is_some() && self.stopped.is_none(), egui::Button::new(icons::labelled(Icon::Play, tr("Send", "Послать")))).on_hover_text(tr("As if the game sent it: every handler of it runs", "Как будто игра его послала: выполняются все его обработчики")).clicked() {
                let args = self.event_args.iter().map(|s| Arg::parse(s)).collect();
                self.send(Cmd::Fire(ev.name.clone(), args));
            }
        });
        ui.label(RichText::new(&api.events[self.event.min(api.events.len() - 1)].doc).small().color(ui.visuals().weak_text_color()));
        ui.separator();
        egui::ScrollArea::vertical().id_salt("listening").auto_shrink([false, false]).show(ui, |ui| {
            if self.session.is_none() {
                ui.label(RichText::new(tr("Run the scripts to send them the game's events by hand: a stop reached, a door, a new hour… The clock sends frame, minute and hour by itself.", "Запустите скрипты, чтобы посылать им события игры вручную: остановка, дверь, новый час… Часы сами посылают frame, minute и hour.")).color(ui.visuals().weak_text_color()));
                return;
            }
            ui.label(RichText::new(format!("{}: {}  ·  {}: {}", tr("Listening", "Слушают"), self.listening.0.iter().map(|(e, n)| format!("{e} ×{n}")).collect::<Vec<_>>().join(", "), tr("timers", "таймеры"), self.listening.1)).small());
        });
    }

    fn build_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(tr("Sign with the key", "Подписать ключом"));
            ui.add(egui::TextEdit::singleline(&mut self.sign_key).hint_text(tr("a secret key file of oopc keygen (optional)", "файл секретного ключа oopc keygen (необязательно)")).desired_width(320.0));
            if ui.small_button(tr("Browse…", "Обзор…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_file() {
                    self.sign_key = p.to_string_lossy().to_string();
                }
            }
            if self.building.is_some() {
                ui.spinner();
            }
        });
        egui::ScrollArea::vertical().id_salt("build log").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if self.build_log.is_empty() {
                ui.label(RichText::new(tr("Build .oop makes one file of the map's scripts for sharing (in the project's dist folder): compiled, obfuscated, with the permissions its code needs. The game runs the scripts' folder as it is; the .oop is for giving them away without their sources.", "«Собрать .oop» делает один файл со скриптами карты, чтобы делиться (в папке dist проекта): скомпилирован, запутан, с разрешениями, которые нужны его коду. Игра выполняет папку скриптов как есть; .oop нужен, чтобы отдать их без исходников.")).color(ui.visuals().weak_text_color()));
            }
            for l in &self.build_log {
                ui.label(mono(l));
            }
        });
    }

    fn api_panel(&mut self, ui: &mut egui::Ui) {
        let api = crate::script::api::api();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("openOMSI API {}", api.version)).strong());
        });
        ui.add(egui::TextEdit::singleline(&mut self.api_search).hint_text(tr("Search functions and events", "Поиск функций и событий")).desired_width(f32::INFINITY));
        let q = self.api_search.to_lowercase();
        let mut insert = None;
        egui::ScrollArea::vertical().id_salt("api list").auto_shrink([false, false]).show(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new(tr("Events", "События")).strong()).default_open(!q.is_empty()).show(ui, |ui| {
                for e in api.events.iter().filter(|e| q.is_empty() || e.name.contains(&q) || e.doc.to_lowercase().contains(&q)) {
                    let r = ui.selectable_label(false, mono(e.signature())).on_hover_text(format!("{}\n{} {}", e.doc, tr("since", "с версии"), e.since));
                    if r.double_clicked() {
                        insert = Some(format!("map.on(\"{}\", function({})\n\nend)\n", e.name, e.args.join(", ")));
                    }
                }
            });
            for g in std::iter::once("").chain(api.groups()) {
                let fs: Vec<&crate::script::api::Function> = api.functions.iter().filter(|f| if g.is_empty() { !f.name.contains('.') } else { f.name.starts_with(&format!("{g}.")) }).filter(|f| q.is_empty() || f.name.contains(&q) || f.doc.to_lowercase().contains(&q)).collect();
                if fs.is_empty() {
                    continue;
                }
                let title = if g.is_empty() { "omsi".to_string() } else { format!("omsi.{g}") };
                egui::CollapsingHeader::new(mono(title)).id_salt(("api", g)).default_open(!q.is_empty()).show(ui, |ui| {
                    for f in fs {
                        let r = ui.selectable_label(false, mono(f.signature())).on_hover_ui(|ui| {
                            ui.set_max_width(380.0);
                            ui.label(mono(format!("-> {}", f.returns)));
                            ui.label(&f.doc);
                            if let Some(p) = &f.permission {
                                ui.label(RichText::new(format!("{} {p} ({})", tr("permission", "разрешение"), tr("for an .oop", "для .oop"))).small());
                            }
                            ui.label(RichText::new(format!("{} {}", tr("since", "с версии"), f.since)).small());
                        });
                        if r.double_clicked() {
                            insert = Some(f.signature());
                        }
                    }
                });
            }
        });
        ui.label(RichText::new(tr("Double-click puts it into the script at the cursor.", "Двойной клик вставляет в скрипт у курсора.")).small().color(ui.visuals().weak_text_color()));
        if let (Some(text), Some(t)) = (insert, self.tabs.get_mut(self.active)) {
            if t.writable {
                let id = egui::Id::new(("code", &t.path)).with("text");
                let at = egui::text_edit::TextEditState::load(ui.ctx(), id).and_then(|s| s.cursor.char_range()).map(|r| r.primary.index.0).unwrap_or(t.text.chars().count());
                let b = t.text.char_indices().nth(at).map(|(b, _)| b).unwrap_or(t.text.len());
                t.text.insert_str(b, &text);
                t.stale = true;
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, project: &Project, friendly: &str, actions: &mut Vec<Action>) {
        if let Some(nf) = &mut self.new_file {
            let mut open = true;
            let mut make = false;
            let title = if nf.kind == Kind::Module { tr("New module", "Новый модуль") } else { tr("New script", "Новый скрипт") };
            egui::Window::new(title).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                let templates = project::templates();
                ui.horizontal(|ui| {
                    ui.label(tr("Name", "Имя"));
                    let r = ui.add(egui::TextEdit::singleline(&mut nf.name).hint_text(templates[nf.template].name));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        make = true;
                    }
                });
                ui.label(tr("Start from", "Взять за основу"));
                for (i, t) in templates.iter().enumerate() {
                    if (nf.kind == Kind::Module) == (i == 1) || (nf.kind == Kind::Script && i != 1) {
                        if ui.radio(nf.template == i, t.title).clicked() {
                            nf.template = i;
                        }
                    }
                }
                if ui.button(icons::labelled(Icon::Add, tr("Make", "Создать"))).clicked() {
                    make = true;
                }
            });
            if make {
                let t = &project::templates()[nf.template];
                let name = if nf.name.trim().is_empty() { t.name.to_string() } else { nf.name.clone() };
                match project.create(friendly).and_then(|_| project.add(nf.kind, &name, t.text, friendly)) {
                    Ok(f) => {
                        self.files_stale = true;
                        self.open_tab(TabKind::Project(f.rel.clone()), project.path(&f.rel), true);
                    }
                    Err(e) => self.message = Some((e.to_string(), true)),
                }
                open = false;
            }
            if !open {
                self.new_file = None;
            }
        }
        if let Some((rel, name)) = &mut self.rename {
            let mut open = true;
            let mut done = false;
            egui::Window::new(tr("Rename", "Переименовать")).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                let r = ui.text_edit_singleline(name);
                r.request_focus();
                if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button(tr("Rename", "Переименовать")).clicked() {
                    done = true;
                }
            });
            if done {
                let old = project.path(rel);
                match project.rename(rel, name, friendly) {
                    Ok(new_rel) => {
                        for t in self.tabs.iter_mut().filter(|t| t.path == old) {
                            t.path = project.path(&new_rel);
                            t.title = t.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            t.kind = TabKind::Project(new_rel.clone());
                        }
                        self.files_stale = true;
                    }
                    Err(e) => self.message = Some((e.to_string(), true)),
                }
                open = false;
            }
            if !open {
                self.rename = None;
            }
        }
        if self.light_open {
            let mut open = true;
            egui::Window::new(tr("A light switched by a script", "Свет, включаемый скриптом")).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.label(RichText::new(tr("A new object: a lamp that glows and lights the ground while its script says so. The script is yours to change after.", "Новый объект: лампа, которая светит и освещает землю, пока так говорит её скрипт. Потом скрипт можно менять.")).small());
                let kinds = [Behaviour::Hours { from: 18.0, to: 23.0 }, Behaviour::Blink { period: 2.0 }, Behaviour::Months { from: 12, to: 1 }];
                ui.horizontal_wrapped(|ui| {
                    for k in kinds {
                        let on = std::mem::discriminant(&k) == std::mem::discriminant(&self.light);
                        if ui.selectable_label(on, k.label()).clicked() && !on {
                            self.light = k;
                        }
                    }
                });
                match &mut self.light {
                    Behaviour::Hours { from, to } => {
                        ui.horizontal(|ui| {
                            ui.label(tr("From", "С"));
                            ui.add(egui::DragValue::new(from).range(0.0..=24.0).speed(0.1).suffix(tr(" h", " ч")));
                            ui.label(tr("to", "до"));
                            ui.add(egui::DragValue::new(to).range(0.0..=24.0).speed(0.1).suffix(tr(" h", " ч")));
                        });
                    }
                    Behaviour::Blink { period } => {
                        ui.horizontal(|ui| {
                            ui.label(tr("A round takes", "Один круг"));
                            ui.add(egui::DragValue::new(period).range(0.4..=60.0).speed(0.1).suffix(tr(" s", " с")));
                        });
                    }
                    Behaviour::Months { from, to } => {
                        ui.horizontal(|ui| {
                            ui.label(tr("From month", "С месяца"));
                            ui.add(egui::DragValue::new(from).range(1..=12));
                            ui.label(tr("to", "по"));
                            ui.add(egui::DragValue::new(to).range(1..=12));
                        });
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(tr("Colour", "Цвет"));
                    ui.color_edit_button_srgb(&mut self.light_color);
                    ui.label(tr("Reaches", "Светит на"));
                    ui.add(egui::DragValue::new(&mut self.light_range).range(1.0..=100.0).suffix(" m"));
                });
                if ui.button(icons::labelled(Icon::Add, tr("Make the light", "Создать свет"))).clicked() {
                    actions.push(Action::ScriptedLight(self.light, self.light_color, self.light_range));
                    self.light_open = false;
                }
            });
            if !open {
                self.light_open = false;
            }
        }
    }
}
