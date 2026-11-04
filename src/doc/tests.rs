//! A map saved and read again is the same map; an autosave brings its changes back; a save
//! keeps what it replaced.

use omsi_map::{EntryPoint, GroundTex, MapObject, MapRule, MapSpline, SplineAttachment};

use super::*;
use crate::install::Roots;

/// A content folder of its own for a test (the installation is not needed: nothing is read
/// from it).
pub(crate) fn scratch(name: &str) -> Roots {
    let d = std::env::temp_dir().join(format!("openmapper-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("maps")).unwrap();
    Roots { omsi: d.join("no-omsi"), content: d }
}

pub(crate) fn sample_map(roots: &Roots) -> Document {
    omsi_map::set_world_coordinates(false);
    omsi_map::set_tile_size(omsi_map::TILE_SIZE);
    let mut global = GlobalCfg::default();
    global.name = "Roundtrip".into();
    global.friendly_name = "Round trip: Тест".into();
    global.description = "two lines\nof text".into();
    global.next_id_code = 100;
    global.ground_textures = vec![
        GroundTex { texture: "Texture\\gras.bmp".into(), detail_texture: "Texture\\gras_det.bmp".into(), params: [0.0, 1.0, 60.0] },
        GroundTex { texture: "Texture\\sand.bmp".into(), detail_texture: "Texture\\noise_low.bmp".into(), params: [8.0, 16.0, 4.0] },
    ];
    let mut doc = Document {
        global,
        tiles: Default::default(),
        order: Vec::new(),
        dir: roots.content.join("maps").join("Roundtrip"),
        source: None,
        global_dirty: true,
        fresh: false,
        global_rev: next_rev(),
        timetable: Default::default(),
        traffic: Default::default(),
        descriptions: Default::default(),
        groups: Vec::new(),
    };
    for k in [(0, 0), (1, 0), (0, -1)] {
        doc.add_tile(k);
    }
    let t = doc.tiles.get_mut(&(0, 0)).unwrap();
    t.terrain.heights.iter_mut().enumerate().for_each(|(i, h)| *h = (i % 7) as f32 * 0.5);
    let n = mask_size(&doc.global.ground_textures[1]);
    let mut mask = vec![0u8; n * n];
    mask[..n * 4].iter_mut().for_each(|a| *a = 200);
    t.masks = vec![None, Some(mask)];
    t.water = Some([1.5, 1.5, 2.0, 2.25]);
    t.tile.objects.push(MapObject { file: "Sceneryobjects\\Generic\\entrypoint_bus.sco".into(), id: 7, pos: [120.0, 18.0, 0.0], rot: [90.0, 0.0, 0.0], flag: 1, extra: vec!["Start 1".into()], ..Default::default() });
    t.tile.objects.push(MapObject { file: "Sceneryobjects\\Bäume\\linde.sco".into(), id: 8, pos: [10.25, 299.5, 1.5], rot: [12.5, 0.0, 0.0], ..Default::default() });
    t.tile.splines.push(MapSpline { file: "Splines\\str_2spur.sli".into(), id: 9, next_id: 10, pos: [0.0, 150.0, 2.0], heading: 90.0, length: 300.0, ..Default::default() });
    // a record on that road: its strings, the variables it takes from another object and its rules
    t.tile.spline_attachments.push(SplineAttachment {
        file: "Sceneryobjects\\lamp.sco".into(),
        id: 11,
        spline_index: 0,
        offset: [-6.5, 0.25, 4.0],
        rot: [180.0, 0.0, 0.0],
        interval: 30.0,
        range: 400.0,
        tilt: true,
        strings: vec!["Hauptstr.".into(), "34N".into()],
        var_parent: Some(7),
        rules: vec![MapRule { path_index: 2, kind: "speedlimit".into(), value: 30.0, extra: 0.5, kill: true }],
        ..Default::default()
    });
    doc.tiles.get_mut(&(1, 0)).unwrap().tile.splines.push(MapSpline { file: "Splines\\str_2spur.sli".into(), id: 10, prev_id: 9, pos: [0.0, 150.0, 2.0], heading: 90.0, length: 80.0, radius: -60.0, grad_start: 2.0, grad_end: -1.0, ..Default::default() });
    doc.global.entry_points.push(EntryPoint { index: 0, object_id: 7, name: "Start 1".into(), ..Default::default() });
    crate::editor::sync_entry_points(&mut doc);
    doc
}

#[test]
fn a_map_saved_reads_back_the_same() {
    let _map = one_map();
    let roots = scratch("roundtrip");
    let mut doc = sample_map(&roots);
    save(&mut doc).unwrap();
    assert!(!doc.is_dirty());
    let back = Document::load(&roots, &doc.dir.join("global.cfg"), |_| {}).unwrap();
    assert_eq!(back.global.friendly_name, doc.global.friendly_name);
    assert_eq!(back.global.description.trim(), doc.global.description);
    assert_eq!(back.global.next_id_code, 100);
    assert_eq!(back.global.ground_textures.len(), 2);
    assert_eq!(back.tiles.keys().collect::<Vec<_>>(), doc.tiles.keys().collect::<Vec<_>>());
    assert_eq!(back.order, doc.order);
    for (k, t) in &doc.tiles {
        let b = &back.tiles[k];
        assert_eq!(b.water, t.water, "{k:?}");
        assert_eq!(b.tile.objects.len(), t.tile.objects.len(), "{k:?}");
        for (x, y) in b.tile.objects.iter().zip(&t.tile.objects) {
            assert_eq!((&x.file, x.id, x.pos, x.rot), (&y.file, y.id, y.pos, y.rot));
        }
        assert_eq!(b.tile.splines.len(), t.tile.splines.len());
        for (x, y) in b.tile.splines.iter().zip(&t.tile.splines) {
            assert_eq!((&x.file, x.id, x.prev_id, x.next_id, x.pos, x.heading, x.length, x.radius), (&y.file, y.id, y.prev_id, y.next_id, y.pos, y.heading, y.length, y.radius));
            assert_eq!((x.grad_start, x.grad_end), (y.grad_start, y.grad_end));
        }
        // a [splineAttachement] record is a whole object: its place, its strings, the object it
        // takes its variables from and its rules all come back
        assert_eq!(b.tile.spline_attachments, t.tile.spline_attachments, "{k:?}");
        assert_eq!(b.terrain.heights, t.terrain.heights, "{k:?}");
    }
    // the painting, through the A8 DDS
    let m = back.tiles[&(0, 0)].masks[1].as_ref().unwrap();
    assert_eq!(m, doc.tiles[&(0, 0)].masks[1].as_ref().unwrap());
    // the record comes back with the variables it takes from another object and its rules
    let a = back.attachment((0, 0), 11).expect("the record is in the map again");
    assert_eq!((a.var_parent, a.rules.len(), a.strings.len()), (Some(7), 1, 2), "{a:?}");
    assert_eq!(a.rules[0].kind, "speedlimit");
    assert_eq!(back.global.entry_points.len(), 1);
    assert_eq!(back.global.entry_points[0].object_id, 7);
    // (saved again unchanged: the same bytes)
    let first = std::fs::read(doc.dir.join("tile_0_0.map")).unwrap();
    let mut again = back;
    again.tiles.values_mut().for_each(|t| t.dirty = true);
    again.global_dirty = true;
    save(&mut again).unwrap();
    assert_eq!(std::fs::read(doc.dir.join("tile_0_0.map")).unwrap(), first);
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn a_save_keeps_what_it_replaced() {
    let _map = one_map();
    let roots = scratch("keep");
    let mut doc = sample_map(&roots);
    save(&mut doc).unwrap();
    // (the first save replaces nothing)
    assert!(safe::backups(&doc.dir).is_empty());
    let before = std::fs::read(doc.dir.join("tile_0_0.map")).unwrap();
    let t = doc.tiles.get_mut(&(0, 0)).unwrap();
    t.tile.objects.retain(|o| o.id != 8);
    t.touch_content();
    save(&mut doc).unwrap();
    let kept = safe::backups(&doc.dir);
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(kept[0].join("tile_0_0.map")).unwrap(), before);
    // only the changed tile and global.cfg were written and kept
    assert!(!kept[0].join("tile_1_0.map").exists());
    assert!(kept[0].join("global.cfg").is_file());
    // and going back brings the tree back
    safe::restore(&doc.dir, &kept[0]).unwrap();
    let back = Document::load(&roots, &doc.dir.join("global.cfg"), |_| {}).unwrap();
    assert!(back.object((0, 0), 8).is_some());
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn an_autosave_brings_the_changes_back() {
    let _map = one_map();
    let roots = scratch("recover");
    let mut doc = sample_map(&roots);
    save(&mut doc).unwrap();
    // a change, autosaved, then "a crash": the document is gone
    let t = doc.tiles.get_mut(&(1, 0)).unwrap();
    t.tile.objects.push(MapObject { file: "Sceneryobjects\\x.sco".into(), id: 55, pos: [1.0, 2.0, 0.0], ..Default::default() });
    t.touch_content();
    doc.add_tile((2, 0));
    let line = doc.timetable.add_line("99");
    doc.traffic.parked.push("Vehicles\\Car\\x.ovh".into());
    doc.traffic.restored();
    autosave::write(&doc, &roots.content).unwrap();
    drop(doc);
    let meta = autosave::pending(&roots.content).unwrap();
    let back = autosave::restore(&roots, &meta).unwrap();
    assert!(back.object((1, 0), 55).is_some());
    assert!(back.tiles.contains_key(&(2, 0)));
    assert!(back.timetable.lines.iter().any(|l| l.name == line) && back.timetable.dirty);
    assert!(back.traffic.parked.iter().any(|p| p.ends_with("x.ovh")) && back.traffic.dirty);
    assert!(back.is_dirty() && back.tiles[&(1, 0)].dirty && !back.tiles[&(0, 0)].dirty);
    assert_eq!(back.dir, roots.content.join("maps").join("Roundtrip"));
    // the map itself was not touched by the autosave
    let on_disk = Document::load(&roots, &back.dir.join("global.cfg"), |_| {}).unwrap();
    assert!(on_disk.object((1, 0), 55).is_none());
    assert!(on_disk.timetable.lines.is_empty() && on_disk.traffic.parked.is_empty());
    let _ = std::fs::remove_dir_all(&roots.content);
}

#[test]
fn odd_tiles_are_read_without_a_panic() {
    let _map = one_map();
    let roots = scratch("odd");
    let dir = roots.content.join("maps").join("Odd");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = "[name]\nOdd\n\n[map]\n0\n0\ntile_0_0.map\n\n[map]\n1\n0\ntile_1_0.map\n\n[map]\nx\ny\n\n[map]\n2\n0\ntile_2_0.map\n";
    std::fs::write(dir.join("global.cfg"), cfg).unwrap();
    // a tile cut off in the middle of a spline, a tile of junk, and one not there
    std::fs::write(dir.join("tile_0_0.map"), "[version]\n14\n\n[spline]\n0\nSplines\\a.sli\n12\n").unwrap();
    std::fs::write(dir.join("tile_1_0.map"), [0xFFu8, 0xFE, 0x00, 0xD8, 0x41, 0x00, 0xFF, 0xFF]).unwrap();
    std::fs::write(dir.join("tile_1_0.map.terrain"), [1u8, 2, 3]).unwrap();
    let doc = Document::load(&roots, &dir.join("global.cfg"), |_| {}).unwrap();
    assert!(doc.tiles.len() <= 2);
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// The maps of the installation, saved whole (every tile written again) into a content
/// folder of the test's own and read back: nothing moves, nothing is lost.
#[test]
fn installed_maps_survive_a_save() {
    let _map = one_map();
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
        eprintln!("no OMSI 2 installation: skipped");
        return;
    };
    let roots = scratch("installed");
    let roots = Roots { omsi: omsi.clone(), content: roots.content };
    let Ok(rd) = std::fs::read_dir(omsi.join("maps")) else { return };
    for e in rd.flatten() {
        let cfg = e.path().join("global.cfg");
        if !cfg.is_file() {
            continue;
        }
        let mut doc = Document::load(&roots, &cfg, |_| {}).unwrap();
        doc.tiles.values_mut().for_each(|t| t.dirty = true);
        save(&mut doc).unwrap();
        let back = Document::load(&roots, &doc.dir.join("global.cfg"), |_| {}).unwrap();
        let name = e.file_name();
        assert_eq!(back.tiles.len(), doc.tiles.len(), "{name:?}");
        assert_eq!(back.global.entry_points.len(), doc.global.entry_points.len(), "{name:?}");
        // (they name their tile by its place in the list)
        assert_eq!(back.order, doc.order, "{name:?}");
        for (x, y) in back.global.entry_points.iter().zip(&doc.global.entry_points) {
            assert_eq!((x.object_id, x.group), (y.object_id, y.group), "{name:?}");
        }
        assert_eq!(crate::doc::tile_size(), { omsi_map::configure_grid(&doc.global); crate::doc::tile_size() }, "{name:?}");
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6);
        for (k, t) in &doc.tiles {
            let b = &back.tiles[k];
            assert_eq!(b.tile.objects.len(), t.tile.objects.len(), "{name:?} {k:?}");
            assert_eq!(b.tile.attach_objects.len(), t.tile.attach_objects.len(), "{name:?} {k:?}");
            assert_eq!(b.tile.spline_attachments.len(), t.tile.spline_attachments.len(), "{name:?} {k:?}");
            for (x, y) in b.tile.objects.iter().zip(&t.tile.objects) {
                assert!(x.file == y.file && x.id == y.id && close(x.pos, y.pos) && close(x.rot, y.rot), "{name:?} {k:?} object {}", y.id);
                assert_eq!(x.extra, y.extra);
            }
            let live: Vec<_> = t.tile.splines.iter().filter(|s| !s.deleted).collect();
            assert_eq!(b.tile.splines.len(), live.len(), "{name:?} {k:?}");
            for (x, y) in b.tile.splines.iter().zip(live) {
                assert!(x.id == y.id && x.prev_id == y.prev_id && x.next_id == y.next_id && close(x.pos, y.pos), "{name:?} {k:?} spline {}: {:?} {:?}", y.id, (x.id, x.prev_id, x.next_id, x.pos), (y.id, y.prev_id, y.next_id, y.pos));
                assert!((x.length - y.length).abs() < 1e-6 && (x.radius - y.radius).abs() < 1e-6 && (x.heading - y.heading).abs() < 1e-6, "{name:?} {k:?} spline {}: {:?} {:?}", y.id, (x.length, x.radius, x.heading), (y.length, y.radius, y.heading));
            }
            assert_eq!(b.terrain.heights, t.terrain.heights, "{name:?} {k:?}");
            assert_eq!(b.water, t.water, "{name:?} {k:?}");
            assert_eq!(b.masks, t.masks, "{name:?} {k:?}");
        }
        eprintln!("{name:?}: {} tiles saved and read back", doc.tiles.len());
    }
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// A map whose tiles have names of their own keeps them: a tile not edited is not written,
/// so global.cfg must still name the file it is in.
#[test]
fn tiles_keep_their_own_file_names() {
    let _map = one_map();
    let roots = scratch("names");
    let dir = roots.content.join("maps").join("Names");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = "[name]\nNames\n\n[NextIDCode]\n50\n\n[map]\n0\n0\nkachel_a.map\n\n[map]\n1\n0\nkachel_b.map\n";
    std::fs::write(dir.join("global.cfg"), cfg).unwrap();
    let tile = "[version]\n14\n\n[terrain]\n\n[object]\n0\nSceneryobjects\\x.sco\n7\n10\n20\n0\n0\n0\n0\n0\n\n";
    std::fs::write(dir.join("kachel_a.map"), tile).unwrap();
    std::fs::write(dir.join("kachel_b.map"), tile.replace("\n7\n", "\n8\n")).unwrap();
    let mut doc = Document::load(&roots, &dir.join("global.cfg"), |_| {}).unwrap();
    assert_eq!(doc.tiles.len(), 2);
    let t = doc.tiles.get_mut(&(0, 0)).unwrap();
    t.tile.objects[0].pos[0] = 30.0;
    t.touch_content();
    save(&mut doc).unwrap();
    let back = Document::load(&roots, &dir.join("global.cfg"), |_| {}).unwrap();
    assert_eq!(back.tiles.len(), 2, "the tile not edited is still found");
    assert!(back.object((1, 0), 8).is_some());
    assert_eq!(back.object((0, 0), 7).map(|o| o.pos[0]), Some(30.0));
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// A road running on into a tile that is taken away ends there: its link to the spline
/// that went is let go (so a road can be joined to that end again).
#[test]
fn removing_a_tile_lets_go_of_links_into_it() {
    let _map = one_map();
    let roots = scratch("unlink");
    let mut doc = sample_map(&roots);
    let rev = doc.tiles[&(0, 0)].content_rev;
    doc.remove_tile((1, 0));
    let s = doc.spline((0, 0), 9).unwrap();
    assert_eq!(s.next_id, 0);
    assert_ne!(doc.tiles[&(0, 0)].content_rev, rev, "the view builds the tile again");
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// The descriptions in other languages come back from an autosave too.
#[test]
fn an_autosave_keeps_the_descriptions() {
    let _map = one_map();
    let roots = scratch("recover-dsc");
    let mut doc = sample_map(&roots);
    save(&mut doc).unwrap();
    doc.descriptions.set("ENG", "Round trip".into(), "In English".into());
    autosave::write(&doc, &roots.content).unwrap();
    drop(doc);
    let meta = autosave::pending(&roots.content).unwrap();
    let back = autosave::restore(&roots, &meta).unwrap();
    assert_eq!(back.descriptions.langs.get("ENG").map(|l| l.1.as_str()), Some("In English"));
    assert!(back.is_dirty() && back.descriptions.dirty);
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// A stop object moved over into another tile: the timetable's stop list names its new
/// tile on the next save, also when nothing of the timetable was edited.
#[test]
fn a_stop_moved_into_another_tile_is_found_there() {
    let _map = one_map();
    let roots = scratch("stop-tile");
    let mut doc = sample_map(&roots);
    // (the linde, id 8, is the stop; tile (0, 0) is the first of the list)
    doc.timetable.stops.push(omsi_timetable::BusStopEntry { name: "Linde".into(), group: 0, object_id: 8, params: [0.0; 3] });
    doc.timetable.touch_stops();
    save(&mut doc).unwrap();
    crate::selection::place_object_at(&mut doc, (0, 0), 8, tile_origin((1, 0)) + glam::DVec2::new(5.0, 5.0));
    save(&mut doc).unwrap();
    let back = omsi_timetable::TimetableData::load(&doc.dir);
    let at = doc.order.iter().position(|k| *k == (1, 0)).unwrap() as i32;
    assert_eq!(back.bus_stops[0].group, at);
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// The view of Berlin-Spandau as in 1980 carries fewer chrono patches than in 1995, and
/// the map itself is not changed by it.
#[test]
fn the_map_is_shown_as_on_a_day() {
    let _map = one_map();
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
    let cfg = omsi.join("maps").join("Berlin-Spandau").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let roots = Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-no-content") };
    let mut doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let dirty = doc.is_dirty();
    let early = doc.show_chrono(19800101);
    let patched = doc.tiles.values().filter(|t| !t.chrono.is_empty()).count();
    assert!(doc.show_chrono(19950101) > early + 5);
    assert!(doc.tiles.values().filter(|t| !t.chrono.is_empty()).count() > patched);
    assert_eq!(doc.is_dirty(), dirty);
}

#[test]
fn a_new_map_has_the_cars_its_traffic_names() {
    let _map = one_map();
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
        eprintln!("no OMSI 2 installation: skipped");
        return;
    };
    let roots = scratch("newcars");
    let roots = Roots { omsi, content: roots.content };
    let doc = Document::new_map(&roots, "Cars", "Cars", 1);
    for (name, _) in &doc.traffic.veh_groups {
        assert!(doc.traffic.groups.iter().any(|g| g.name.eq_ignore_ascii_case(name) && !g.vehicles.is_empty()), "{name}");
    }
    let _ = std::fs::remove_dir_all(&roots.content);
}

/// Where the objects of every row on tile (0, 0) stand.
fn row_spots(doc: &Document) -> Vec<DVec3> {
    let t = &doc.tiles[&(0, 0)].tile;
    let mut v: Vec<DVec3> = t.spline_attachments.iter().flat_map(|a| crate::place::row_objects(a, &t.splines, tile_origin((0, 0)))).map(|o| o.pose.pos).collect();
    v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    v
}

#[test]
fn a_cut_road_joined_again_is_the_road_it_was() {
    use crate::selection::tests::{add_spline, flat_map};
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), 1, 50.0, 50.0, 30.0, 120.0, 0, 0);
    {
        let s = &mut doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0];
        s.radius = 300.0;
        s.grad_start = 2.0;
        s.grad_end = 4.0;
    }
    let row = omsi_map::SplineAttachment { file: "x.sco".into(), id: 7, spline_index: 0, offset: [5.0, 0.0, 10.0], interval: 20.0, range: 100.0, ..Default::default() };
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(row);
    let before = doc.spline((0, 0), 1).unwrap().clone();
    let spots = row_spots(&doc);
    assert_eq!(spots.len(), 6);
    let b = doc.split_spline((0, 0), 1, 47.0).unwrap();
    // the row runs on over the cut
    assert!(row_spots(&doc).iter().zip(&spots).all(|(a, b)| (*a - *b).length() < 1e-6));
    // a row on the second piece too
    let mut row2 = doc.tiles[&(0, 0)].tile.spline_attachments[0].clone();
    row2.id = 8;
    row2.spline_index = 1;
    row2.offset = [-5.0, 0.0, 47.0 + 3.0];
    row2.range = 40.0;
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(row2);
    let spots = row_spots(&doc);
    doc.join_splines((0, 0), 1).unwrap();
    assert!(doc.find_spline(b).is_none());
    let after = doc.spline((0, 0), 1).unwrap();
    assert_eq!(after.next_id, 0);
    assert!((after.length - before.length).abs() < 1e-9 && (after.grad_end - before.grad_end).abs() < 1e-6, "{after:?}");
    assert!(row_spots(&doc).iter().zip(&spots).all(|(a, b)| (*a - *b).length() < 1e-6));
    assert_eq!(row_spots(&doc).len(), spots.len());
}

#[test]
fn pieces_of_other_bends_are_not_joined() {
    use crate::selection::tests::{add_spline, flat_map};
    let mut doc = flat_map(1);
    add_spline(&mut doc, (0, 0), 1, 50.0, 50.0, 0.0, 50.0, 0, 2);
    add_spline(&mut doc, (0, 0), 2, 50.0, 100.0, 0.0, 50.0, 1, 3);
    add_spline(&mut doc, (0, 0), 3, 50.0, 150.0, 0.0, 30.0, 2, 0);
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[1].radius = 200.0;
    assert!(doc.join_splines((0, 0), 1).is_err());
    // two straights in line: one, the third now following it
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[1].radius = 0.0;
    doc.join_splines((0, 0), 1).unwrap();
    assert_eq!(doc.spline((0, 0), 1).unwrap().length, 100.0);
    assert_eq!(doc.spline((0, 0), 3).unwrap().prev_id, 1);
    doc.join_splines((0, 0), 1).unwrap();
    assert_eq!(doc.spline((0, 0), 1).unwrap().length, 130.0);
    assert!(doc.join_splines((0, 0), 1).is_err());
}

/// A map whose NextIDCode is behind the roads it already has still gets a fresh IDCode:
/// the new spline is not the same one as a road that is already there.
#[test]
fn a_new_id_stays_above_ids_the_map_already_uses() {
    use crate::selection::tests::{add_object, add_spline, flat_map};
    let mut doc = flat_map(1);
    doc.global.next_id_code = 1;
    add_spline(&mut doc, (0, 0), 40, 10.0, 10.0, 0.0, 20.0, 0, 0);
    add_object(&mut doc, (0, 0), 41, 5.0, 5.0);
    assert_eq!(doc.new_id(), 42);
    assert_eq!(doc.new_id(), 43);
    assert!(doc.spline((0, 0), 40).is_some());
}
