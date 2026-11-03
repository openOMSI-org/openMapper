//! Two editors' maps kept the same through a host over the loopback.

use std::time::{Duration, Instant};

use openmapper_collab::{Client, Event, Host, HostConfig, MapInfo, ID_BLOCK};

use super::codec;
use super::sync::{all_states, Note, Sync};
use crate::doc::Document;
use crate::history::History;
use crate::selection::tests::{add_object, flat_map};

struct Side {
    doc: Document,
    history: History,
    sync: Sync,
    client: Client,
    notes: Vec<Note>,
}

impl Side {
    /// One frame of the editor: what was changed here, what came, what goes out.
    fn frame(&mut self) {
        let ch = self.history.take_changed();
        if !ch.is_empty() {
            self.notes.extend(self.sync.edited(ch, &mut self.doc, &mut self.history));
        }
        for e in self.client.poll() {
            match e {
                Event::Msg(m) => self.notes.extend(self.sync.received(m, &mut self.doc, &mut self.history)),
                Event::Closed(why) => panic!("closed: {why}"),
            }
        }
        if let Some(m) = self.sync.next_edit(&self.doc) {
            self.client.send(m);
        }
        if let Some(m) = self.sync.keep_ids(&mut self.doc) {
            self.client.send(m);
        }
    }

    fn edit(&mut self, k: crate::doc::TileKey, f: impl FnOnce(&mut Document)) {
        self.history.begin("edit");
        self.history.touch_tile(&self.doc, k);
        f(&mut self.doc);
        self.history.commit();
    }
}

/// Ana hosts a two-tile map, Bruno joins it.
fn session() -> (Host, Side, Side) {
    let mut doc = flat_map(2);
    let cfg = HostConfig { bind: "127.0.0.1:0".parse().unwrap(), password: String::new(), open: true, work: 10, map: MapInfo { folder: "T".into(), name: "T".into() }, next_id: doc.global.next_id_code, max_peers: 8, ..Default::default() };
    let host = Host::start(cfg, all_states(&doc)).unwrap();
    let (client, w) = Client::connect(&host.addr().to_string(), "Ana", "", false).unwrap();
    let sync = Sync::new(&w.states, w.ids, true, &mut doc);
    let ana = Side { doc, history: History::default(), sync, client, notes: Vec::new() };
    let (client, w) = Client::connect(&host.addr().to_string(), "Bruno", "", true).unwrap();
    // (Bruno's map made from what the host sent, as a guest's is)
    let mut doc = flat_map(0);
    for s in &w.states {
        match (s.part, s.data.as_deref()) {
            (openmapper_collab::Part::Global, Some(b)) => assert!(codec::apply_global(&mut doc, b)),
            (openmapper_collab::Part::Tile(x, y), Some(b)) => assert!(codec::apply_tile(&mut doc, (x, y), Some(b))),
            _ => {}
        }
    }
    let sync = Sync::new(&w.states, w.ids, false, &mut doc);
    let bruno = Side { doc, history: History::default(), sync, client, notes: Vec::new() };
    (host, ana, bruno)
}

/// Frames of both until everything is sent and nothing more comes for a moment.
fn settle(a: &mut Side, b: &mut Side) {
    let until = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0;
    while quiet < 20 {
        a.frame();
        b.frame();
        quiet = if a.sync.settled() && b.sync.settled() { quiet + 1 } else { 0 };
        assert!(Instant::now() < until, "never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The same tiles, byte for byte, and the same tile list.
fn same(a: &Document, b: &Document) {
    assert_eq!(a.tiles.keys().collect::<Vec<_>>(), b.tiles.keys().collect::<Vec<_>>());
    for (k, t) in &a.tiles {
        assert!(codec::tile_bytes(t) == codec::tile_bytes(&b.tiles[k]), "tile {k:?} differs");
    }
    assert_eq!(a.order, b.order);
}

#[test]
fn an_edit_reaches_the_other_editor() {
    let (_h, mut ana, mut bruno) = session();
    same(&ana.doc, &bruno.doc);
    ana.edit((1, 0), |d| d.tiles.get_mut(&(1, 0)).unwrap().terrain.heights[10] = 4.5);
    bruno.edit((0, 0), |d| add_object(d, (0, 0), 0, 50.0, 60.0));
    settle(&mut ana, &mut bruno);
    same(&ana.doc, &bruno.doc);
    assert_eq!(bruno.doc.tiles[&(1, 0)].terrain.heights[10], 4.5);
    assert_eq!(ana.doc.tiles[&(0, 0)].tile.objects.len(), 1);
    assert!(ana.notes.is_empty() && bruno.notes.is_empty(), "{:?} {:?}", ana.notes, bruno.notes);
}

#[test]
fn edits_of_one_tile_at_once_end_the_same_on_both() {
    let (_h, mut ana, mut bruno) = session();
    // both change tile 0, 0 before either hears of the other's
    ana.edit((0, 0), |d| d.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 1.0);
    bruno.edit((0, 0), |d| d.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 2.0);
    ana.frame();
    bruno.frame();
    settle(&mut ana, &mut bruno);
    same(&ana.doc, &bruno.doc);
    // one of them was told
    let told = ana.notes.iter().chain(&bruno.notes).filter(|n| matches!(n, Note::TurnedDown { .. })).count();
    assert_eq!(told, 1, "{:?} {:?}", ana.notes, bruno.notes);
}

#[test]
fn undo_does_not_put_back_what_somebody_else_did() {
    let (_h, mut ana, mut bruno) = session();
    ana.edit((0, 0), |d| d.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 1.0);
    settle(&mut ana, &mut bruno);
    bruno.edit((0, 0), |d| d.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 7.0);
    settle(&mut ana, &mut bruno);
    // Ana's step on tile 0, 0 is forgotten: her undo has nothing to put back
    assert!(!ana.history.can_undo());
    ana.history.undo(&mut ana.doc);
    settle(&mut ana, &mut bruno);
    assert_eq!(ana.doc.tiles[&(0, 0)].terrain.heights[0], 7.0);
    same(&ana.doc, &bruno.doc);
    // and Bruno's own undo still works, for both
    bruno.history.undo(&mut bruno.doc);
    settle(&mut ana, &mut bruno);
    assert_eq!(ana.doc.tiles[&(0, 0)].terrain.heights[0], 1.0);
    same(&ana.doc, &bruno.doc);
}

#[test]
fn a_new_tile_and_the_tile_list_come_together() {
    let (_h, mut ana, mut bruno) = session();
    bruno.history.begin("add tile");
    bruno.history.touch_tile(&bruno.doc, (5, 5));
    bruno.history.touch_global(&bruno.doc);
    bruno.doc.add_tile((5, 5));
    bruno.history.commit();
    settle(&mut ana, &mut bruno);
    assert!(ana.doc.tiles.contains_key(&(5, 5)));
    assert_eq!(ana.doc.order.last(), Some(&(5, 5)));
    same(&ana.doc, &bruno.doc);
    // and taken away again by Ana
    ana.history.begin("remove tile");
    ana.history.touch_tile(&ana.doc, (5, 5));
    ana.history.touch_global(&ana.doc);
    ana.doc.remove_tile((5, 5));
    ana.history.commit();
    settle(&mut ana, &mut bruno);
    assert!(!bruno.doc.tiles.contains_key(&(5, 5)));
    same(&ana.doc, &bruno.doc);
}

#[test]
fn each_editor_hands_out_idcodes_of_its_own() {
    let (_h, mut ana, mut bruno) = session();
    let a = ana.doc.new_id();
    let b = bruno.doc.new_id();
    assert_ne!(a, b);
    assert!((b - a).abs() >= ID_BLOCK - 1);
    // half of Bruno's block used: the next is asked for, and taken once his runs out
    bruno.doc.global.next_id_code = b + ID_BLOCK / 2;
    settle(&mut ana, &mut bruno);
    bruno.doc.global.next_id_code = b + ID_BLOCK;
    bruno.frame();
    let c = bruno.doc.new_id();
    // (the block after the two given out on joining)
    assert_eq!(c, a + 2 * ID_BLOCK);
}

#[test]
fn a_guest_cannot_change_the_hosts_timetable() {
    let (_h, mut ana, mut bruno) = session();
    let before = bruno.doc.timetable.clone();
    bruno.doc.timetable.dirty = true;
    bruno.history.push_snapshot("tt", crate::history::Snapshot { timetable: Some(before), ..Default::default() });
    settle(&mut ana, &mut bruno);
    assert!(bruno.notes.contains(&Note::HostOnly));
    // (undone: the step is there to redo, none to undo)
    assert!(bruno.history.can_redo() && !bruno.history.can_undo());
    // the host may
    let before = ana.doc.timetable.clone();
    ana.history.push_snapshot("tt", crate::history::Snapshot { timetable: Some(before), ..Default::default() });
    settle(&mut ana, &mut bruno);
    assert!(ana.notes.is_empty());
}

#[test]
fn groups_are_shared() {
    let (_h, mut ana, mut bruno) = session();
    let before = ana.doc.groups.clone();
    ana.doc.groups.push(crate::mapgroups::MapGroup { name: "Stop".into(), ids: vec![1, 2] });
    ana.history.push_snapshot("group", crate::history::Snapshot { groups: Some(before), ..Default::default() });
    settle(&mut ana, &mut bruno);
    assert_eq!(bruno.doc.groups, ana.doc.groups);
}

#[test]
fn a_guest_map_lands_in_a_folder_of_its_own() {
    let content = std::env::temp_dir().join(format!("openmapper-collab-{}", std::process::id()));
    std::fs::create_dir_all(content.join("maps").join("Grundorf_multiplayer")).unwrap();
    let roots = crate::install::Roots { omsi: content.clone(), content: content.clone() };
    let doc = flat_map(1);
    let mut states = all_states(&doc);
    states.iter_mut().for_each(|s| s.version = 1);
    let w = openmapper_collab::Welcome { you: openmapper_collab::Peer { id: 2, name: "B".into(), color: [0; 3] }, map: MapInfo { folder: "../Grundorf".into(), name: "G".into() }, ids: (1, 2), peers: vec![], states };
    let g = super::guest_doc(&roots, &w).unwrap();
    // (the folder name sent cannot lead out of the maps; the one there already is left alone)
    assert_eq!(g.dir, content.join("maps").join("Grundorf_multiplayer2"));
    assert_eq!(g.tiles.len(), 1);
    let _ = std::fs::remove_dir_all(&content);
}

#[test]
fn two_editors_through_the_session() {
    let content = std::env::temp_dir().join(format!("openmapper-collab-glue-{}", std::process::id()));
    let roots = crate::install::Roots { omsi: content.clone(), content: content.clone() };
    let mut ana_ed = crate::editor::Editor::default();
    let mut doc = flat_map(2);
    doc.dir = content.join("maps").join("Grundorf");
    add_object(&mut doc, (0, 0), 50, 120.0, 80.0);
    ana_ed.open(doc);
    // (opening reads the groups from the folder: given after)
    ana_ed.doc.as_mut().unwrap().groups.push(crate::mapgroups::MapGroup { name: "Stop".into(), ids: vec![100] });
    let mut ana = super::Collab { name: "Ana".into(), port: 0, password: "long enough".into(), ..Default::default() };
    ana.lan = false;
    ana.host(&mut ana_ed);
    assert!(ana.error.is_none(), "{:?}", ana.error);
    assert!(ana.live().unwrap().hosting.is_some() && !ana.is_guest());
    // (the port the system gave)
    let port = ana.live().unwrap().hosting.as_ref().map(|h| h._host.addr().port()).unwrap();

    let mut bruno_ed = crate::editor::Editor::default();
    let mut bruno = super::Collab { name: "Bruno".into(), address: format!("127.0.0.1:{port}"), password: "long enough".into(), ..Default::default() };
    bruno.join();
    let until = Instant::now() + Duration::from_secs(10);
    let doc = loop {
        bruno.tick(&mut bruno_ed, Some(&roots));
        if let Some(d) = bruno.take_joined() {
            break d;
        }
        assert!(bruno.error.is_none(), "{:?}", bruno.error);
        assert!(Instant::now() < until, "never joined");
        std::thread::sleep(Duration::from_millis(5));
    };
    // (what the app does: opens it, then hands the groups over)
    bruno_ed.open(doc);
    bruno.joined_open(&mut bruno_ed);
    assert!(bruno.is_guest());
    assert_eq!(bruno_ed.doc.as_ref().unwrap().groups.len(), 1);
    assert_eq!(bruno_ed.doc.as_ref().unwrap().dir, content.join("maps").join("Grundorf_multiplayer"));

    // Ana's cursor and selection, for Bruno to draw
    ana_ed.cursor = Some(glam::DVec3::new(120.0, 80.0, 0.0));
    ana_ed.cursor_in_view = true;
    ana_ed.selected = Some(crate::doc::Sel::Object((0, 0), 50));
    // Bruno places an object, Ana raises the ground
    {
        let d = bruno_ed.doc.as_mut().unwrap();
        bruno_ed.history.begin("place");
        bruno_ed.history.touch_tile(d, (1, 0));
        let id = d.new_id();
        add_object(d, (1, 0), id, 10.0, 10.0);
        bruno_ed.history.commit();
        let d = ana_ed.doc.as_mut().unwrap();
        ana_ed.history.begin("raise");
        ana_ed.history.touch_tile(d, (0, 0));
        d.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 3.0;
        ana_ed.history.commit();
    }
    let until = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0;
    while quiet < 20 {
        ana.tick(&mut ana_ed, Some(&roots));
        bruno.tick(&mut bruno_ed, Some(&roots));
        quiet = if ana.settled() && bruno.settled() { quiet + 1 } else { 0 };
        assert!(Instant::now() < until, "never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    same(ana_ed.doc.as_ref().unwrap(), bruno_ed.doc.as_ref().unwrap());
    let placed = ana_ed.doc.as_ref().unwrap().tiles[&(1, 0)].tile.objects[0].id;
    assert!(placed >= 100 + ID_BLOCK, "Bruno's object has an IDCode of his block: {placed}");
    let ana_id = ana.live().unwrap().you.id;
    let (seen, _) = &bruno.live().unwrap().seen[&ana_id];
    assert_eq!((seen.cursor, seen.selected.clone()), (Some([120.0, 80.0, 0.0]), vec![(0, 0, 50)]));
    // A remote update must not replace a tile in the middle of a brush/drag. Once the
    // gesture ends, the conflict converges and its local version is recoverable.
    bruno_ed.history.begin("brush still held");
    bruno_ed.history.touch_tile(bruno_ed.doc.as_ref().unwrap(), (0, 0));
    bruno_ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap().terrain.heights[1] = 11.0;
    ana_ed.history.begin("remote edit");
    ana_ed.history.touch_tile(ana_ed.doc.as_ref().unwrap(), (0, 0));
    ana_ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap().terrain.heights[2] = 22.0;
    ana_ed.history.commit();
    for _ in 0..30 {
        ana.tick(&mut ana_ed, Some(&roots));
        bruno.tick(&mut bruno_ed, Some(&roots));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(bruno_ed.history.is_open());
    assert_eq!(bruno_ed.doc.as_ref().unwrap().tiles[&(0, 0)].terrain.heights[1], 11.0);
    bruno_ed.history.commit();
    let until = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0;
    while quiet < 20 {
        ana.tick(&mut ana_ed, Some(&roots));
        bruno.tick(&mut bruno_ed, Some(&roots));
        quiet = if ana.settled() && bruno.settled() { quiet + 1 } else { 0 };
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    same(ana_ed.doc.as_ref().unwrap(), bruno_ed.doc.as_ref().unwrap());
    let conflicts = bruno_ed.doc.as_ref().unwrap().dir.join("openMapper/conflicts");
    assert!(std::fs::read_dir(conflicts).unwrap().next().is_some());
    // a guest does not save; leaving lifts the next IDCode past every one in the map
    assert_eq!(ana.live().unwrap().peers.len(), 2);
    bruno.leave(&mut bruno_ed);
    assert!(bruno.live().is_none());
    ana.leave(&mut ana_ed);
    assert!(ana_ed.doc.as_ref().unwrap().global.next_id_code > placed);
    let _ = std::fs::remove_dir_all(&content);
}

#[test]
fn hosting_without_a_password_only_when_chosen_and_never_on_the_internet() {
    let mut ed = crate::editor::Editor::default();
    let mut doc = flat_map(1);
    doc.dir = std::env::temp_dir().join("openmapper-collab-open").join("maps").join("M");
    ed.open(doc);
    let mut c = super::Collab { name: "Ana".into(), port: 0, lan: false, ..Default::default() };
    c.host(&mut ed);
    assert!(c.live().is_none() && c.error.is_some(), "an empty password is not the default");
    c.internet = true;
    c.no_password = true;
    c.host(&mut ed);
    assert!(c.live().is_none(), "never without a password on the internet");
    c.internet = false;
    c.host(&mut ed);
    assert!(c.live().is_some(), "{:?}", c.error);
}

#[test]
fn an_edit_held_open_keeps_reading_and_puts_the_others_work_in_once_it_is_closed() {
    let content = std::env::temp_dir().join(format!("openmapper-collab-held-{}", std::process::id()));
    let roots = crate::install::Roots { omsi: content.clone(), content: content.clone() };
    let mut ana_ed = crate::editor::Editor::default();
    let mut doc = flat_map(2);
    doc.dir = content.join("maps").join("Held");
    ana_ed.open(doc);
    let mut ana = super::Collab { name: "Ana".into(), port: 0, password: "long enough".into(), lan: false, ..Default::default() };
    ana.host(&mut ana_ed);
    let port = ana.live().unwrap().hosting.as_ref().map(|h| h._host.addr().port()).unwrap();
    let mut bruno_ed = crate::editor::Editor::default();
    let mut bruno = super::Collab { name: "Bruno".into(), address: format!("127.0.0.1:{port}"), password: "long enough".into(), ..Default::default() };
    bruno.join();
    let until = Instant::now() + Duration::from_secs(10);
    let doc = loop {
        bruno.tick(&mut bruno_ed, Some(&roots));
        if let Some(d) = bruno.take_joined() {
            break d;
        }
        assert!(Instant::now() < until, "never joined: {:?}", bruno.error);
        std::thread::sleep(Duration::from_millis(5));
    };
    bruno_ed.open(doc);
    bruno.joined_open(&mut bruno_ed);

    // Ana's brush stroke on tile 0, 0 is held open
    ana_ed.history.begin("brush");
    ana_ed.history.touch_tile(ana_ed.doc.as_ref().unwrap(), (0, 0));
    ana_ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 9.0;
    // meanwhile Bruno raises tile 1, 0, and far more comes than the host's queue for Ana holds
    {
        let d = bruno_ed.doc.as_mut().unwrap();
        bruno_ed.history.begin("raise");
        bruno_ed.history.touch_tile(d, (1, 0));
        d.tiles.get_mut(&(1, 0)).unwrap().terrain.heights[0] = 5.0;
        bruno_ed.history.commit();
    }
    for i in 0..600 {
        bruno.live().unwrap().client.send(openmapper_collab::Msg::Say(format!("line {i}")));
        if i % 50 == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let until = Instant::now() + Duration::from_secs(5);
    while ana.live().map(|l| l.chat.len() < 200).unwrap_or(false) || !bruno.settled() {
        ana.tick(&mut ana_ed, Some(&roots));
        bruno.tick(&mut bruno_ed, Some(&roots));
        assert!(Instant::now() < until, "chat {:?}", ana.live().map(|l| l.chat.len()));
        std::thread::sleep(Duration::from_millis(2));
    }
    for _ in 0..50 {
        ana.tick(&mut ana_ed, Some(&roots));
        std::thread::sleep(Duration::from_millis(2));
    }
    // (still in, Bruno's edit held: Ana's stroke is not ended under her)
    assert_eq!(ana.live().map(|l| l.peers.len()), Some(2), "Ana was let go");
    assert!(ana_ed.history.is_open());
    assert_eq!(ana_ed.doc.as_ref().unwrap().tiles[&(1, 0)].terrain.heights[0], 0.0);
    // the stroke ends: Bruno's edit comes in, Ana's goes out
    ana_ed.history.commit();
    let until = Instant::now() + Duration::from_secs(10);
    let mut quiet = 0;
    while quiet < 20 {
        ana.tick(&mut ana_ed, Some(&roots));
        bruno.tick(&mut bruno_ed, Some(&roots));
        quiet = if ana.settled() && bruno.settled() { quiet + 1 } else { 0 };
        assert!(Instant::now() < until, "never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    same(ana_ed.doc.as_ref().unwrap(), bruno_ed.doc.as_ref().unwrap());
    assert_eq!(ana_ed.doc.as_ref().unwrap().tiles[&(1, 0)].terrain.heights[0], 5.0);
    assert_eq!(bruno_ed.doc.as_ref().unwrap().tiles[&(0, 0)].terrain.heights[0], 9.0);
    let _ = std::fs::remove_dir_all(&content);
}


#[test]
fn protected_source_cannot_start_a_collaboration_host() {
    let _map = crate::doc::one_map();
    let mut ed = crate::editor::Editor::default();
    let mut doc = flat_map(1);
    doc._publisher_protected_source = true;
    ed.doc = Some(doc);
    let mut collab = super::Collab::default();
    collab.no_password = true;
    collab.lan = false;
    collab.port = 0;
    collab.host(&mut ed);
    assert!(collab.error.as_deref().is_some_and(|error| error.contains("protected")));
    assert!(matches!(collab.phase, super::Phase::Off), "protected map was made available to peers");
}
