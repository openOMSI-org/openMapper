//! The crossing editor's reading and writing of a `.sco`.
use super::*;

const SAMPLE: &str = "\
[friendlyname]
Kreuzung Test

[groups]
1
Test

[surface]

[absheight]

[mesh]
Kreuzung.o3d

[matl]
asphalt.bmp
0

[matl_alpha]
1

[script]
1
test.osc

[path]
-6.5
-12
0.1
90
0
12
0.02
0.02
0
3.5
0
3

[blockpath]
2
0

[use_traffic_light]
0

[path]
6.5
-12
0.1
90
0
12
0.02
0.02
0
3.5
0
2

[crossingproblem]

[splinehelper]
Test.sli
0
0.1
-12
270
0.1
0

[traffic_lights_group]
60

[traffic_light]
Main
[phase]
0
30
[phase]
6
25
[approachdist]
5

[new_attachment]

[attach]
x
1
2
3
";

fn sheet() -> Sheet {
    Sheet::parse(SAMPLE)
}

#[test]
fn reads_the_paths_and_the_lights() {
    let s = sheet();
    let c = &s.crossing;
    assert_eq!(c.paths.len(), 2);
    assert!((c.paths[0].start() - DVec3::new(-6.5, -12.0, 0.1)).length() < 1e-5);
    assert_eq!(c.paths[0].heading(), 90.0);
    assert_eq!(c.paths[0].length(), 12.0);
    assert_eq!(c.paths[0].turn(), TURN_RIGHT);
    assert_eq!(c.paths[0].blocks, vec![(2, 0)]);
    assert_eq!(c.paths[0].light, Some(0));
    assert!(!c.paths[0].crossing_problem);
    assert_eq!(c.paths[1].turn(), TURN_LEFT);
    assert!(c.paths[1].crossing_problem);
    assert_eq!(c.paths[1].light, None);
    assert_eq!(c.cycle, Some(60.0));
    assert_eq!(c.lights.len(), 1);
    assert_eq!(c.lights[0].name, "Main");
    assert_eq!(c.lights[0].phases.len(), 2);
    assert_eq!(c.lights[0].approach_dist, Some(5.0));
    assert_eq!(c.helpers.len(), 1);
    assert_eq!(c.helpers[0].spline, "Test.sli");
    assert!((c.helpers[0].local() - DVec3::new(0.0, -12.0, 0.1)).length() < 1e-5);
}

#[test]
fn the_model_and_the_script_survive_a_round_trip() {
    let text = sheet().text();
    for line in ["[friendlyname]\nKreuzung Test", "[mesh]\nKreuzung.o3d", "[matl]\nasphalt.bmp", "[matl_alpha]\n1", "[script]\n1\ntest.osc", "[new_attachment]", "[attach]\nx\n1\n2\n3"] {
        assert!(text.contains(line), "lost {line:?} in\n{text}");
    }
    // and it reads back as the same crossing
    let again = Sheet::parse(&text);
    assert_eq!(again.crossing, sheet().crossing);
    // and the game itself reads the paths back
    let sco = omsi_scenery::SceneryObject::parse(&omsi_cfg::CfgFile::from_str("t.sco", &text));
    assert_eq!(sco.paths.len(), 2);
    assert_eq!(sco.traffic_lights.len(), 1);
    assert_eq!(sco.traffic_lights_group, Some(60.0));
    assert_eq!(sco.path_blocks[0], vec![(2, 0)]);
    assert_eq!(sco.path_traffic_light[0], 0);
    assert_eq!(sco.path_traffic_light[1], -1);
    assert!(sco.path_crossing_problem[1]);
    assert_eq!(sco.spline_helpers.len(), 1);
}

#[test]
fn a_change_keeps_the_slopes_in_rise_per_metre() {
    let mut s = sheet();
    s.crossing.paths[0].v[6] = 0.04;
    s.crossing.paths[0].v[7] = 0.04;
    s.crossing.paths[0].v[5] = 20.0;
    let sco = omsi_scenery::SceneryObject::parse(&omsi_cfg::CfgFile::from_str("t.sco", &s.text()));
    let p = &sco.paths[0];
    assert_eq!(p.params[6], 0.04);
    assert_eq!(p.params[7], 0.04);
    // the curve the map draws rises 0.8 m over the 20 m, as the game does
    let c = crate::lanes::object_path_curve(p);
    assert!((c.point_at(20.0).z - (0.1 + 0.8)).abs() < 1e-4);
}

#[test]
fn path_2_keeps_its_two_extra_numbers() {
    let text = "[path_2]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3.5\n0\n0\n7\n8\n\n";
    let mut s = Sheet::parse(text);
    assert!(s.crossing.paths[0].second);
    assert_eq!(s.crossing.paths[0].extra, [7.0, 8.0]);
    s.crossing.paths[0].v[5] = 15.0;
    let out = s.text();
    assert!(out.contains("[path_2]"), "{out}");
    let sco = omsi_scenery::SceneryObject::parse(&omsi_cfg::CfgFile::from_str("t.sco", &out));
    assert_eq!(sco.paths[0].params[12], 7.0);
    assert_eq!(sco.paths[0].params[13], 8.0);
    assert_eq!(sco.paths[0].params[5], 15.0);
}

#[test]
fn switchdir_stays_with_its_own_path() {
    let text = "[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3.5\n0\n0\n[switchdir]\n1\n\n[path]\n10\n0\n0\n0\n0\n10\n0\n0\n0\n3.5\n0\n0\n[switchdir]\n0\n\n";
    let mut s = Sheet::parse(text);
    assert_eq!(s.crossing.paths[0].switch_dir, Some(1));
    assert_eq!(s.crossing.paths[1].switch_dir, Some(0));
    s.crossing.paths.swap(0, 1);
    let sco = omsi_scenery::SceneryObject::parse(&omsi_cfg::CfgFile::from_str("t.sco", &s.text()));
    assert_eq!(sco.path_switch_dir, vec![Some(0), Some(1)]);
}

#[test]
fn removing_a_path_renumbers_the_blocks_that_name_paths() {
    let mut c = sheet().crossing;
    c.paths[0].blocks = vec![(2, 0), (1, 5)];
    c.remove_path(1);
    // path 2 became 1, path 1 is gone
    assert_eq!(c.paths.len(), 1);
    assert_eq!(c.paths[0].blocks, vec![(1, 0)]);
}

#[test]
fn removing_a_light_moves_the_paths_that_wait_at_it() {
    let mut c = Crossing::default();
    c.lights = vec![
        TrafficLight { name: "A".into(), ..Default::default() },
        TrafficLight { name: "B".into(), ..Default::default() },
        TrafficLight { name: "C".into(), ..Default::default() },
    ];
    c.stops = vec![[2.0, 10.0, 0.0], [1.0, 10.0, 1.0]];
    let mut p = Path::default();
    p.light = Some(2);
    let mut q = Path::default();
    q.light = Some(1);
    c.paths = vec![p, q];
    c.remove_light(1);
    assert_eq!(c.lights.len(), 2);
    assert_eq!(c.paths[0].light, Some(1), "the third light is the second now");
    assert_eq!(c.paths[1].light, None, "its light is gone");
    assert_eq!(c.stops.len(), 1);
    assert_eq!(c.stops[0][0], 1.0);
}

#[test]
fn moving_a_path_renumbers_the_blocks() {
    let mut c = Crossing::default();
    c.paths = (0..3).map(|i| Path { v: [i as f32; 12], ..Default::default() }).collect();
    c.paths[0].blocks = vec![(2, 1)];
    c.paths[2].blocks = vec![(0, 2)];
    // 0 1 2 -> 1 2 0 : old 2 becomes 1, old 0 becomes 2
    c.move_path(0, 2);
    assert_eq!(c.paths[1].v[0], 2.0);
    assert_eq!(c.paths[2].v[0], 0.0);
    assert_eq!(c.paths[2].blocks, vec![(1, 1)], "the block followed the path it named");
    assert_eq!(c.paths[1].blocks, vec![(2, 2)]);
}

#[test]
fn snapping_finds_the_nearest_other_end() {
    let mut c = Crossing::default();
    let mut a = Path::default();
    a.v = [0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    let mut b = Path::default();
    b.v = [0.3, 10.2, 0.0, 180.0, 0.0, 10.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    c.paths = vec![a, b];
    let (i, at_end, at) = c.nearest_end(DVec3::new(0.1, 10.1, 0.0), crate::lanes::LINK, 0).unwrap();
    assert_eq!(i, 1);
    assert!(!at_end, "it is that path's start");
    assert!((at.truncate() - DVec2::new(0.3, 10.2)).length() < 1e-5);
    // its own ends do not count: the other path's end is found instead
    let (j, _, _) = c.nearest_end(DVec3::new(0.0, 10.0, 0.0), crate::lanes::LINK, 1).unwrap();
    assert_eq!(j, 0);
}

#[test]
fn dragging_the_end_turns_and_stretches_a_straight_path() {
    let mut p = Path::default();
    p.v = [0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    p.set_end(DVec3::new(10.0, 10.0, 0.0));
    assert!((p.heading() - 45.0).abs() < 1e-4);
    assert!((p.length() - (200.0f64).sqrt()).abs() < 1e-4);
}

#[test]
fn a_way_that_would_take_a_circle_is_a_straight_to_the_end() {
    // a path running north; asked to arrive running south, the arc that does it is nearly a
    // whole circle: its length grows without bound, and the path used to end up millions of
    // metres long (and every piece laid after it, at a height far from the map)
    let mut p = Path::default();
    p.v = [0.0, 0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    let end = p.end();
    p.fit_end_tangent(180.0);
    assert!(p.length().is_finite(), "a length: {}", p.length());
    assert!(p.length() <= 40.0 * 4.0 + 8.0, "no loop: {} m", p.length());
    assert!((p.end().truncate() - end.truncate()).length() < 1e-3, "the end stayed: {:?} vs {end:?}", p.end());
    // the same from the start:
    let mut p = Path::default();
    p.v = [0.0, 0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    let end = p.end();
    p.fit_start_tangent(180.0);
    assert!(p.length() <= 40.0 * 4.0 + 8.0, "no loop: {} m", p.length());
    // (the way the path leaves is the one thing a turn must not change: the arc keeps it, and
    // the end stops where that arc reaches rather than the path being turned round)
    assert!((p.heading() - 180.0).abs() < 1e-6, "it still leaves in 180: {}", p.heading());
    assert!(p.length() <= 40.0 * 8.0, "no loop: {} m", p.length());
    assert!(p.slope_start().is_finite() && p.slope_end().is_finite());

    // and a path whose length is already broken is only sampled so far
    // (a path of sixteen million metres used to ask for thirty-two million points)
    let mut p = Path::default();
    p.v[5] = 16_340_103.0;
    assert!(p.points(0.5).len() <= 4001, "{} points", p.points(0.5).len());
}

#[test]
fn a_path_takes_the_way_and_rise_of_the_one_it_joins() {
    // a straight piece running east
    let mut p = Path::default();
    p.v = [0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    let end = p.end();
    // asked to arrive running north: it becomes an arc, its start and end staying put
    p.fit_end_tangent(0.0);
    assert!((p.end().truncate() - end.truncate()).length() < 1e-3, "the end stayed: {:?} vs {end:?}", p.end());
    assert!((p.start().truncate() - DVec2::ZERO).length() < 1e-6, "the start stayed");
    let c = p.curve();
    assert!(c.heading_at(c.length).abs() < 1e-2, "it arrives running north: {}", c.heading_at(c.length));

    // the rise: the end's height and the way it climbs there, the start's height kept
    p.set_end_height_and_slope(2.0, 0.15);
    assert!((p.end().z - 2.0).abs() < 1e-2, "the end is at the height asked for: {}", p.end().z);
    assert!((p.slope_end() - 0.15).abs() < 1e-6, "and climbs there the way asked for: {}", p.slope_end());

    // and the start's own way: the end stays where it is
    let end = p.end();
    p.fit_start_tangent(0.0);
    assert!((p.end().truncate() - end.truncate()).length() < 1e-3, "the end stayed");
    let c = p.curve();
    assert!(c.heading_at(0.0).abs() < 1e-2, "it leaves running north: {}", c.heading_at(0.0));
}

#[test]
fn dragging_the_end_of_an_arc_keeps_its_radius() {
    let mut p = Path::default();
    p.v = [0.0, 0.0, 0.0, 0.0, 20.0, 0.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    // a quarter of the circle, 20 m radius, turning right from +y towards +x
    let q = p.curve().point_at(std::f64::consts::FRAC_PI_2 * 20.0);
    p.set_end(q);
    assert!((p.radius() - 20.0).abs() < 1e-3, "radius {}", p.radius());
    assert!((p.heading() - 0.0).abs() < 1e-3 || (p.heading() - 360.0).abs() < 1e-3, "heading {}", p.heading());
    assert!((p.length() - std::f64::consts::FRAC_PI_2 * 20.0).abs() < 1e-2, "length {}", p.length());
}

#[test]
fn crossing_a_full_diameter_widens_the_arc() {
    let mut p = Path::default();
    p.v = [0.0, 0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0];
    p.set_end(DVec3::new(0.0, 20.0, 0.0));
    assert!((p.radius() - 10.0).abs() < 1e-3, "radius {}", p.radius());
    assert!((p.length() - std::f64::consts::PI * 10.0).abs() < 1e-2);
}

#[test]
fn a_text_written_in_utf16_keeps_its_bytes() {
    let mut bytes = vec![0xFF, 0xFE];
    for u in "[friendlyname]\nПерекрёсток\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let text = omsi_cfg::decode_text(&bytes);
    assert!(text.contains("Перекрёсток"));
    assert_eq!(encode_like(&text, &bytes), bytes);
}

#[test]
fn a_text_written_in_utf8_with_a_mark_keeps_its_bytes() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("[friendlyname]\nПерекрёсток\n".as_bytes());
    let text = omsi_cfg::decode_text(&bytes);
    assert_eq!(encode_like(&text, &bytes), bytes);
}

#[test]
fn a_utf8_file_keeps_its_bytes() {
    let bytes = "[friendlyname]\nKreuzung\n".as_bytes().to_vec();
    assert_eq!(encode_like("[friendlyname]\nKreuzung\n", &bytes), bytes);
}

#[test]
fn an_object_without_paths_is_not_offered() {
    let text = "[friendlyname]\nHouse\n[mesh]\nhouse.o3d\n";
    let s = Sheet::parse(text);
    assert!(!s.has_paths());
    // and a sheet with nothing owned leaves the file as it was
    assert_eq!(s.text(), text);
}

#[test]
fn the_cycle_falls_back_to_the_longest_light() {
    let c = sheet().crossing;
    assert_eq!(c.cycle_len(), 60.0);
    let mut c2 = c.clone();
    c2.cycle = None;
    // Main has 30 + 25 s
    assert_eq!(c2.cycle_len(), 55.0);
}

/// Saving copies the whole object folder into openMapper's own - its model, its textures and
/// its script still beside the copy - writes the new paths there and reads them back, while
/// the installation's own `.sco` is left exactly as it was.
#[test]
fn a_saved_copy_keeps_the_object_together() {
    let dir = std::env::temp_dir().join(format!("om-crossing-{}-save", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let content = dir.join("content");
    let src = content.join("Sceneryobjects").join("Test").join("Kreuzung");
    std::fs::create_dir_all(&src).unwrap();
    let sco = src.join("Kreuzung.sco");
    std::fs::write(&sco, SAMPLE).unwrap();
    for f in ["Kreuzung.o3d", "asphalt.bmp", "test.osc"] {
        std::fs::write(src.join(f), b"x").unwrap();
    }
    let roots = crate::install::Roots { omsi: dir.join("omsi"), content: content.clone() };
    let rel = "Sceneryobjects\\Test\\Kreuzung\\Kreuzung.sco";

    let bytes = std::fs::read(&sco).unwrap();
    let mut s = sheet();
    s.crossing.paths[0].v[5] = 20.0;
    let new_rel = copy_into(&roots, rel, "TestMap", "Kreuzung").unwrap();
    assert!(new_rel.contains("openMapper"), "{new_rel}");
    let abs = omsi_cfg::resolve_path(&roots.content, &new_rel);
    write(&abs, &s.text(), &bytes).unwrap();
    verify(&abs, &s.crossing).unwrap();
    // the model, the texture and the script came along
    let at = abs.parent().unwrap();
    for f in ["Kreuzung.o3d", "asphalt.bmp", "test.osc"] {
        assert!(at.join(f).is_file(), "{f} did not come along");
    }
    // and the installation's own file is untouched
    assert_eq!(std::fs::read(&sco).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

fn scratch_roots(name: &str) -> crate::install::Roots {
    let dir = std::env::temp_dir().join(format!("om-crossing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("content")).unwrap();
    crate::install::Roots { omsi: dir.join("omsi"), content: dir.join("content") }
}

/// Writing into the object's own file keeps the file as it stood beside it as a `.bak`, and
/// nothing is written (nor the backup touched) when the text is the file's own.
#[test]
fn a_save_in_place_keeps_the_old_file_as_a_bak() {
    let dir = std::env::temp_dir().join(format!("om-crossing-{}-inplace", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let abs = dir.join("Kreuzung.sco");
    std::fs::write(&abs, SAMPLE).unwrap();
    // (the model the object names: the check that reads it back looks for it)
    std::fs::write(dir.join("Kreuzung.o3d"), b"x").unwrap();
    let bytes = std::fs::read(&abs).unwrap();
    let mut s = sheet();
    s.crossing.paths[0].v[5] = 20.0;
    let bak = bak_path(&abs);
    assert!(!bak.exists(), "no backup before a save");

    assert!(write_in_place(&abs, &s.text(), &bytes, &s.crossing).unwrap(), "the file was written");
    assert_eq!(std::fs::read(&bak).unwrap(), bytes, "the old file is the backup");
    assert_ne!(std::fs::read(&abs).unwrap(), bytes, "the file is the new text");

    // the same text again: nothing to write, and the backup is left as it was
    let now = std::fs::read(&abs).unwrap();
    let stamp = std::fs::metadata(&bak).unwrap().modified().unwrap();
    assert!(!write_in_place(&abs, &s.text(), &now, &s.crossing).unwrap(), "nothing changed");
    assert_eq!(std::fs::metadata(&bak).unwrap().modified().unwrap(), stamp, "the backup was not touched");

    // a second save with other text: the backup still holds the file from before the first
    s.crossing.paths[0].v[5] = 25.0;
    assert!(write_in_place(&abs, &s.text(), &now, &s.crossing).unwrap(), "the file was written again");
    assert_eq!(std::fs::read(&bak).unwrap(), bytes, "the backup is still the original file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Clearing the backups takes only those the editor noted as its own.
#[test]
fn clearing_takes_only_the_backups_the_editor_made() {
    let roots = scratch_roots("baks");
    let dir = roots.content.join("Sceneryobjects").join("Test");
    std::fs::create_dir_all(&dir).unwrap();
    let mine = dir.join("Kreuzung.sco.bak");
    let other = dir.join("SomethingElse.sco.bak");
    std::fs::write(&mine, b"x").unwrap();
    std::fs::write(&other, b"x").unwrap();
    baks::note(&roots, &mine);
    assert_eq!(baks::list(&roots).len(), 1, "only what was noted");
    assert_eq!(baks::clear(&roots), 1, "one backup went");
    assert!(!mine.exists(), "the editor's backup is gone");
    assert!(other.exists(), "somebody else's .bak is left alone");
    let _ = std::fs::remove_dir_all(&roots.content);
}

const SMALL: &str = "[mesh]\nplate.x\n\n[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\n\n";

/// One of openMapper's own junctions is a flat file of the folder the copy goes in: it does not
/// copy itself into itself, and brings only what it names.
#[test]
fn a_flat_junction_is_not_copied_into_itself() {
    let roots = scratch_roots("flat");
    let map = roots.content.join("Sceneryobjects").join("openMapper").join("TestMap");
    std::fs::create_dir_all(map.join("model")).unwrap();
    std::fs::create_dir_all(map.join("texture")).unwrap();
    std::fs::write(map.join("junction_1.sco"), SMALL).unwrap();
    std::fs::write(map.join("junction_2.sco"), SMALL.replace("plate.x", "other.x")).unwrap();
    std::fs::write(map.join("model").join("plate.x"), b"x").unwrap();
    std::fs::write(map.join("model").join("other.x"), b"x").unwrap();
    std::fs::write(map.join("texture").join("a.png"), b"x").unwrap();
    let rel = copy_into(&roots, "Sceneryobjects\\openMapper\\TestMap\\junction_1.sco", "TestMap", "junction_1").unwrap();
    let dir = omsi_cfg::resolve_path(&roots.content, &rel).parent().unwrap().to_path_buf();
    assert!(dir.join("model").join("plate.x").is_file());
    assert!(dir.join("texture").join("a.png").is_file());
    assert!(!dir.join("model").join("other.x").exists(), "another object's model");
    assert!(!dir.join("junction_2.sco").exists(), "another object");
    assert!(!dir.join("TestMap").exists() && !dir.join("junction_1").exists(), "nested");
    let _ = std::fs::remove_dir_all(roots.content.parent().unwrap());
}

/// A file named outside the object's folder would be lost by a copy: nothing is copied, and it says so.
#[test]
fn a_file_outside_the_folder_stops_the_copy() {
    let roots = scratch_roots("outside");
    let base = roots.content.join("Sceneryobjects").join("Test");
    std::fs::create_dir_all(base.join("X")).unwrap();
    std::fs::write(base.join("shared.osc"), b"x").unwrap();
    std::fs::write(base.join("X").join("x.sco"), format!("[script]\n1\n..\\shared.osc\n\n{SMALL}")).unwrap();
    assert!(copy_into(&roots, "Sceneryobjects\\Test\\X\\x.sco", "TestMap", "x").is_err());
    assert!(!roots.content.join("Sceneryobjects").join("openMapper").join("TestMap").join("x").exists());
    let _ = std::fs::remove_dir_all(roots.content.parent().unwrap());
}

/// What fails its check does not replace the file.
#[test]
fn a_write_that_fails_its_check_leaves_the_file() {
    let roots = scratch_roots("checked");
    let abs = roots.content.join("x.sco");
    std::fs::write(&abs, SMALL).unwrap();
    let s = Sheet::parse(SMALL);
    let mut wrong = s.crossing.clone();
    wrong.paths.clear();
    assert!(write_checked(&abs, "[path]\n", SMALL.as_bytes(), &wrong).is_err());
    assert_eq!(std::fs::read_to_string(&abs).unwrap(), SMALL);
    assert_eq!(std::fs::read_dir(&roots.content).unwrap().count(), 1, "nothing left beside it");
    let _ = std::fs::remove_dir_all(roots.content.parent().unwrap());
}

/// CR LF stays, a comment after a block's values stays, and a keyword with nothing before it to
/// belong to stays ahead of the paths.
#[test]
fn line_ends_comments_and_early_keywords_survive() {
    let text = "[crossingproblem]\n\n[path]\n0\n0\n0\n0\n0\n10\n0\n0\n0\n3\n0\n0\nthe main way\n\n[phase]\n0\n5\n".replace('\n', "\r\n");
    let out = Sheet::parse(&text).text();
    assert!(out.contains("\r\n") && !out.replace("\r\n", "").contains('\n'), "{out:?}");
    assert!(out.contains("the main way"), "{out:?}");
    let back = Sheet::parse(&out);
    assert_eq!(back.crossing.paths.len(), 1);
    assert!(!back.crossing.paths[0].crossing_problem, "it was not this path's");
    assert!(out.find("[crossingproblem]").unwrap() < out.find("[path]").unwrap());
    assert!(out.find("[phase]").unwrap() < out.find("[path]").unwrap());
}

/// A helper with an empty road type keeps its numbers in their places.
#[test]
fn a_blank_helper_spline_does_not_shift_its_numbers() {
    let s = Sheet::parse("[splinehelper]\n\n1\n2\n3\n4\n5\n6\n");
    assert_eq!(s.crossing.helpers[0].spline, "");
    assert_eq!(s.crossing.helpers[0].x, 1.0);
    assert_eq!(s.crossing.helpers[0].radius, 6.0);
}

/// A very small slope is written as it is, and a name its code page cannot hold is not lost.
#[test]
fn small_numbers_and_foreign_names_are_kept() {
    assert_eq!(num(0.00004), "0.00004");
    assert_eq!(num(-0.0), "0");
    let bytes = encode_like("Светофор", b"plain ascii");
    assert_eq!(omsi_cfg::decode_text(&bytes), "Светофор");
}

// ---------------------------------------------------------------- the timeline

/// A crossing with one light, its phases as `(state, seconds)`.
fn lit(cycle: Option<f32>, phases: &[(i32, f32)]) -> Crossing {
    let mut c = Crossing::default();
    c.cycle = cycle;
    c.lights.push(omsi_scenery::TrafficLight {
        name: "Main".into(),
        phases: phases.iter().map(|&(state, duration)| omsi_scenery::TrafficLightPhase { state, duration }).collect(),
        approach_dist: None,
    });
    c
}

#[test]
fn the_phases_lie_one_after_another_over_the_cycle() {
    let c = lit(Some(60.0), &[(0, 30.0), (3, 5.0), (6, 20.0), (9, 5.0)]);
    let s = c.segments(0);
    assert_eq!(s.len(), 4);
    assert_eq!(s.iter().map(|q| (q.start, q.end)).collect::<Vec<_>>(), vec![(0.0, 30.0), (30.0, 35.0), (35.0, 55.0), (55.0, 60.0)]);
    assert_eq!(s.iter().map(|q| q.phase).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(s.iter().map(|q| q.state).collect::<Vec<_>>(), vec![0, 3, 6, 9]);
    // nothing is held: each one ends with its own seconds
    assert!(s.iter().all(|q| q.own_end == q.end));
}

#[test]
fn the_last_phase_holds_to_the_cycles_end_when_the_phases_are_shorter() {
    let c = lit(Some(60.0), &[(0, 30.0), (6, 20.0)]);
    let s = c.segments(0);
    assert_eq!(s[s.len() - 1].start, 30.0);
    assert_eq!(s[s.len() - 1].own_end, 50.0);
    assert_eq!(s[s.len() - 1].end, 60.0, "the last one runs to the cycle's end");
}

#[test]
fn a_cycle_shorter_than_the_phases_cuts_them() {
    let c = lit(Some(30.0), &[(0, 20.0), (6, 30.0)]);
    let s = c.segments(0);
    assert_eq!(s.len(), 2);
    assert_eq!((s[1].start, s[1].end), (20.0, 30.0), "cut at the cycle's end");
    // a phase that would start past the cycle is not on it at all
    let c = lit(Some(30.0), &[(0, 40.0), (6, 10.0)]);
    assert_eq!(c.segments(0).len(), 1);
    assert_eq!(c.segments(0)[0].end, 30.0);
}

#[test]
fn a_phase_of_no_length_takes_no_room_on_the_timeline() {
    let c = lit(Some(30.0), &[(0, 10.0), (3, 0.0), (6, 10.0)]);
    let s = c.segments(0);
    assert_eq!(s.len(), 2);
    assert_eq!(s.iter().map(|q| q.phase).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!((s[1].start, s[1].end), (10.0, 30.0));
    // its phase is still in the file, and still picked in the list
    assert_eq!(c.lights[0].phases.len(), 3);
}

#[test]
fn a_second_of_the_cycle_finds_the_phase_showing_then() {
    let c = lit(Some(60.0), &[(0, 30.0), (3, 5.0), (6, 20.0), (9, 5.0)]);
    assert_eq!(c.phase_at(0, 0.0), Some(0));
    assert_eq!(c.phase_at(0, 29.99), Some(0));
    assert_eq!(c.phase_at(0, 30.0), Some(1), "the boundary belongs to the phase it starts");
    assert_eq!(c.phase_at(0, 34.9), Some(1));
    assert_eq!(c.phase_at(0, 35.0), Some(2));
    assert_eq!(c.phase_at(0, 59.9), Some(3));
    assert_eq!(c.phase_at(0, 60.0), None, "the cycle has ended");
    assert_eq!(c.phase_at(0, -1.0), None);
    // a light that is not there has nothing
    assert_eq!(c.phase_at(4, 1.0), None);
}

#[test]
fn dragging_an_edge_changes_that_phases_duration_and_the_rest_follows() {
    let mut c = lit(None, &[(0, 20.0), (6, 20.0)]);
    assert_eq!(c.cycle_len(), 40.0);
    assert!(c.resize_phase(0, 0, 30.0));
    assert_eq!(c.lights[0].phases[0].duration, 30.0);
    assert_eq!(c.lights[0].phases[1].duration, 20.0, "the other phase keeps its own");
    assert_eq!(c.cycle_len(), 50.0, "without a stated cycle, the cycle is the sum");
    assert_eq!(c.segments(0)[1].start, 30.0);
    // and back the other way, to a whole second
    assert!(c.resize_phase(0, 0, 12.4));
    assert_eq!(c.lights[0].phases[0].duration, 12.0);
}

#[test]
fn a_stated_cycle_keeps_the_drag_inside_it() {
    let mut c = lit(Some(60.0), &[(0, 30.0), (6, 20.0)]);
    assert!(c.resize_phase(0, 0, 200.0));
    assert_eq!(c.lights[0].phases[0].duration, 60.0, "as far as the cycle goes");
    // the last phase can be dragged shorter, and then holds to the cycle's end
    let mut c = lit(Some(60.0), &[(0, 30.0), (6, 20.0)]);
    assert!(c.resize_phase(0, 1, 40.0));
    assert_eq!(c.lights[0].phases[1].duration, 10.0, "40 s into the cycle, and it starts at 30");
    let s = c.segments(0);
    assert_eq!(s[s.len() - 1].end, 60.0);
    assert_eq!(s[s.len() - 1].own_end, 40.0, "the rest of the cycle is held");
}

#[test]
fn dragging_before_a_phases_own_start_leaves_it_at_nothing() {
    let mut c = lit(None, &[(0, 20.0), (6, 20.0)]);
    assert!(c.resize_phase(0, 1, 5.0));
    assert_eq!(c.lights[0].phases[1].duration, 0.0);
    assert_eq!(c.segments(0).len(), 1, "a phase of no length has no block");
    // a drag that changes nothing says so
    assert!(!c.resize_phase(0, 1, 5.0));
    assert!(!c.resize_phase(0, 9, 5.0), "no such phase");
    assert!(!c.resize_phase(7, 0, 5.0), "no such light");
}

#[test]
fn phases_and_lights_are_added_and_removed_at_the_picked_place() {
    let mut c = lit(Some(60.0), &[(0, 30.0), (6, 20.0)]);
    assert_eq!(c.insert_phase(0, 1), Some(1));
    assert_eq!(c.lights[0].phases.len(), 3);
    assert_eq!(c.lights[0].phases[1].state, 0);
    assert_eq!(c.lights[0].phases[1].duration, 10.0);
    assert!(c.remove_phase(0, 1));
    assert_eq!(c.lights[0].phases.len(), 2);
    assert!(!c.remove_phase(0, 5));
    let n = c.add_light();
    assert_eq!(n, 1);
    assert_eq!(c.lights[1].name, "Light 2");
    assert_eq!(c.lights[1].phases.len(), 2);
    assert_eq!(c.lights[1].phases[0].state, 0, "it starts on red");
    assert_eq!(c.lights[1].phases[1].state, 6, "and goes green");
}

#[test]
fn the_timeline_lists_every_light_one_after_another() {
    let mut c = lit(Some(20.0), &[(0, 10.0), (6, 10.0)]);
    c.lights.push(omsi_scenery::TrafficLight {
        name: "Side".into(),
        phases: vec![
            omsi_scenery::TrafficLightPhase { state: 0, duration: 10.0 },
            omsi_scenery::TrafficLightPhase { state: 6, duration: 10.0 },
        ],
        approach_dist: None,
    });
    let t = c.timeline();
    assert_eq!(t.len(), 4);
    assert_eq!(t.iter().map(|s| s.light).collect::<Vec<_>>(), vec![0, 0, 1, 1]);
    assert_eq!(t.iter().map(|s| s.phase).collect::<Vec<_>>(), vec![0, 1, 0, 1]);
    // each light's run starts again at nought
    assert_eq!((t[2].start, t[2].end), (0.0, 10.0));
}
