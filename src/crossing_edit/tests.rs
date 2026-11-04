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
