use glam::DVec3;

use super::network::{Files, Network, PathKey};
use super::*;

fn omsi_root() -> Option<std::path::PathBuf> {
    let r = crate::install::find_omsi_root(&crate::install::Prefs::default());
    if r.is_none() {
        eprintln!("no OMSI 2 installation: skipped");
    }
    r
}

/// The map's grid is global (`omsi_map::configure_grid`): one map at a time, for every
/// test of the crate that reads or makes one.
pub(crate) fn one_map() -> crate::doc::OneMap {
    crate::doc::one_map()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("openmapper-tt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("TTData")).unwrap();
    d
}

/// Everything written as it was read, read again with openOMSI's reader.
fn write_all(tt: &Timetable, dir: &Path) {
    let d = dir.join("TTData");
    std::fs::write(d.join("Busstops.cfg"), encode(&busstops_text(&tt.stops))).unwrap();
    std::fs::write(d.join("StnLinks.cfg"), encode(&stnlinks_text(&tt.links, &tt.stops))).unwrap();
    for t in &tt.trips {
        std::fs::write(d.join(format!("{}.ttp", t.name)), encode(&trip_text(t))).unwrap();
    }
    for t in &tt.tracks {
        std::fs::write(d.join(format!("{}.ttr", track_name(t))), encode(&track_text(t))).unwrap();
    }
    for l in &tt.lines {
        std::fs::write(d.join(format!("{}.ttl", l.name)), encode(&line_text(l))).unwrap();
    }
}

fn same(a: &Timetable, b: &TimetableData, what: &str) {
    assert_eq!(a.stops, b.bus_stops, "{what}: stops");
    assert_eq!(a.links.len(), b.stn_links.len(), "{what}: links");
    for (x, y) in a.links.iter().zip(&b.stn_links) {
        assert_eq!((x.from_id, x.to_id, x.params, &x.entries), (y.from_id, y.to_id, y.params, &y.entries), "{what}: link");
        assert!((x.length - y.length).abs() < 1e-6);
    }
    assert_eq!(a.trips.len(), b.trips.len(), "{what}: trips");
    for (x, y) in a.trips.iter().zip(&b.trips) {
        assert_eq!((&x.name, &x.display_name, &x.terminus, &x.line, x.train_reverse), (&y.name, &y.display_name, &y.terminus, &y.line, y.train_reverse), "{what}: trip");
        assert_eq!((&x.stations, &x.stations_legacy, &x.profiles), (&y.stations, &y.stations_legacy, &y.profiles), "{what}: trip {}", x.name);
    }
    assert_eq!(a.tracks.len(), b.tracks.len(), "{what}: tracks");
    for (x, y) in a.tracks.iter().zip(&b.tracks) {
        assert_eq!(x.entries, y.entries, "{what}: track {}", track_name(x));
    }
    assert_eq!(a.lines.len(), b.lines.len(), "{what}: lines");
    for (x, y) in a.lines.iter().zip(&b.lines) {
        assert_eq!((&x.name, x.user_allowed, x.priority, &x.tours), (&y.name, y.user_allowed, y.priority, &y.tours), "{what}: line");
    }
}

#[test]
fn stock_timetables_round_trip() {
    let Some(root) = omsi_root() else { return };
    for m in ["Grundorf", "Berlin-Spandau"] {
        let src = root.join("maps").join(m);
        if !src.join("TTData").is_dir() {
            continue;
        }
        let tt = Timetable::load(&src, &[]);
        assert!(!tt.lines.is_empty(), "{m}: lines");
        let out = scratch(m);
        write_all(&tt, &out);
        let back = TimetableData::load(&out);
        assert!(back.errors.is_empty(), "{:?}", back.errors);
        same(&tt, &back, m);
        eprintln!("{m}: {} lines, {} trips, {} tracks, {} links, {} stops read back the same", tt.lines.len(), tt.trips.len(), tt.tracks.len(), tt.links.len(), tt.stops.len());
        let _ = std::fs::remove_dir_all(&out);
    }
}

#[test]
fn only_what_changed_is_written() {
    let mut tt = Timetable::default();
    let out = scratch("changed");
    let line = tt.add_line("7");
    let trip = tt.add_trip(&line, "7_A-B", vec![11, 12]);
    tt.lines[0].tours.push(omsi_timetable::Tour { number: "1".into(), ai_group: "Busses".into(), extra: ALL_DAYS.to_string(), trips: vec![omsi_timetable::TourTrip { trip: trip.clone(), profile: 0, departure: 6.0 * 60.0 }] });
    save_to(&mut tt, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!(back.lines.len(), 1);
    assert_eq!(back.lines[0].tours[0].trips[0].trip, "7_A-B");
    assert_eq!(back.trips[0].stations, vec![11, 12]);
    assert!(!tt.dirty);
    // a rename takes the old files away
    tt.rename_trip("7_A-B", "7_B-A").unwrap();
    tt.rename_line("7", "8").unwrap();
    save_to(&mut tt, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!(back.trips.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["7_B-A"]);
    assert_eq!(back.lines.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["8"]);
    assert_eq!(back.lines[0].tours[0].trips[0].trip, "7_B-A");
    assert_eq!(back.trips[0].line, "8");
    tt.delete_line("8");
    save_to(&mut tt, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert!(back.lines.is_empty() && back.trips.is_empty());
    let _ = std::fs::remove_dir_all(&out);
}

/// A tour without a depot (a new map's ailists.cfg has none) keeps its days.
#[test]
fn a_tour_without_depot_reads_back() {
    let mut tt = Timetable::default();
    let line = tt.add_line("3");
    tt.lines[0].tours.push(omsi_timetable::Tour { number: "1".into(), ai_group: String::new(), extra: "799".into(), trips: vec![omsi_timetable::TourTrip { trip: "3_A".into(), profile: 0, departure: 400.0 }] });
    let back = omsi_timetable::Line::parse(&omsi_cfg::CfgFile::from_str(format!("{line}.ttl"), &line_text(&tt.lines[0])));
    assert_eq!(back.tours, tt.lines[0].tours);
}

#[test]
fn text_that_1252_cannot_hold_is_utf16() {
    assert_eq!(encode("Bf. Zoo\n"), b"Bf. Zoo\r\n".to_vec());
    assert_eq!(encode("Straße"), vec![b'S', b't', b'r', b'a', 0xDF, b'e']);
    let b = encode("Вокзал");
    assert_eq!(&b[..2], &[0xFF, 0xFE]);
    assert_eq!(omsi_cfg::decode_text(&b), "Вокзал");
}

#[test]
fn lengths_as_omsi_writes_them() {
    assert_eq!(route::sig3(19.853), 19.9);
    assert_eq!(route::sig3(271.6), 272.0);
    assert_eq!(route::sig3(2.2834), 2.28);
    assert_eq!(route::sig3(0.0), 0.0);
}

/// The times at the stops follow the distance between the fixed ones.
#[test]
fn stop_times_spread_by_distance() {
    let mut p = omsi_timetable::TripProfile { name: "standard".into(), factor: 8.0, ..Default::default() };
    let t = stop_times(3, &[100.0, 300.0], &p);
    assert_eq!(t.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), vec![(0.0, 0.0), (2.0, 2.0), (8.0, 8.0)]);
    set_stop_time(&mut p, 4, 1, Some(5.0));
    let t = stop_times(4, &[100.0, 100.0, 100.0], &p);
    assert_eq!(t.iter().map(|x| x.1).collect::<Vec<_>>(), vec![0.0, 5.0, 6.5, 8.0]);
    assert!(t[1].2 && !t[2].2);
    assert_eq!(p.man_dep_time, vec![(1, 5.0)]);
    set_stop_time(&mut p, 4, 3, Some(9.0));
    assert_eq!(p.man_arr_time, vec![(3, 9.0)]);
    set_stop_time(&mut p, 4, 1, None);
    assert!(p.man_dep_time.is_empty());
}

/// Holidays.txt written and read back the same through openOMSI's reader; Grundorf's too.
#[test]
fn holidays_round_trip() {
    let c = omsi_map::Calendar {
        ranges: vec![omsi_map::HolidayRange { start: 20260720, end: 20260830, name: "Summer".into() }],
        holidays: vec![omsi_map::Holiday { date: 20261225, name: "Christmas".into() }, omsi_map::Holiday { date: 20270101, name: "New Year".into() }],
    };
    let back = omsi_map::Calendar::parse(&omsi_cfg::CfgFile::from_str("Holidays.txt", &holidays_text(&c)));
    assert_eq!(back, c);
    if let Some(root) = omsi_root() {
        let p = root.join("maps").join("Grundorf").join("Holidays.txt");
        if let Ok(g) = omsi_map::Calendar::load(&p) {
            let mut sorted = g.clone();
            sorted.ranges.sort_by_key(|r| r.start);
            sorted.holidays.sort_by_key(|h| h.date);
            let mut again = omsi_map::Calendar::parse(&omsi_cfg::CfgFile::from_str("Holidays.txt", &holidays_text(&g)));
            again.ranges.sort_by_key(|r| r.start);
            again.holidays.sort_by_key(|h| h.date);
            assert_eq!(again, sorted);
            assert!(g.holidays.len() > 50);
        }
    }
}

#[test]
fn clock_times() {
    assert_eq!(clock(247.0), "4:07");
    assert_eq!(clock(24.0 * 60.0 + 10.0), "24:10");
}

/// Grundorf's network as openMapper reads it names the paths of the stock tracks with
/// the same numbers OMSI wrote, and the route found through line 76's stops runs where the
/// stock one does.
#[test]
fn grundorf_routes_like_omsi() {
    let Some(root) = omsi_root() else { return };
    let cfg = root.join("maps").join("Grundorf").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let mut files = Files::new(vec![root.clone()]);
    let net = Network::build(&doc, &mut files);
    assert!(net.lanes.len() > 100, "{} lanes", net.lanes.len());
    let tt = &doc.timetable;
    for trip in ["76_BH-Kk", "76_Kk-BH"] {
        let track = tt.track(trip).unwrap();
        let mut hit = 0;
        for e in &track.entries {
            let v = &e.values;
            let key = PathKey { tile: doc.order[v[2] as usize], id: v[0] as i64, path: v[1] as usize };
            let Some(l) = net.by_key.get(&key).and_then(|l| l.first()) else { continue };
            let l = &net.lanes[*l];
            assert_eq!(l.internal as f64, v[3], "{trip}: internal number of {key:?}");
            assert!((route::sig3(l.file_length) - v[4]).abs() < 1e-9, "{trip}: length of {key:?}: {} vs {}", l.file_length, v[4]);
            hit += 1;
        }
        assert!(hit * 10 >= track.entries.len() * 9, "{trip}: {hit} of {} entries on the network", track.entries.len());
        // the route through the trip's stops
        let t = tt.trip(trip).unwrap();
        let pos: Vec<Option<DVec3>> = t.stations.iter().map(|id| stop_world(&doc, &mut files, *id)).collect();
        let r = route::find(&net, &pos, doc.global.left_hand_traffic, None);
        assert!(r.ok(), "{trip}: {:?}", r.problems);
        let ours: Vec<(f64, f64, f64)> = r.track(&net, &doc.order).iter().map(|e| (e.values[0], e.values[1], e.values[2])).collect();
        let stock: Vec<(f64, f64, f64)> = track.entries.iter().map(|e| (e.values[0], e.values[1], e.values[2])).collect();
        let common = ours.iter().filter(|k| stock.contains(k)).count();
        eprintln!("{trip}: {} lanes found, {} in the stock track of {}, {:.0} m", ours.len(), common, stock.len(), r.length());
        assert!(common * 10 >= ours.len() * 8, "{trip}: ours {ours:?}\nstock {stock:?}");
        // the station links' lengths as OMSI measured them
        let links = r.links(&net, &doc.order, &t.stations);
        for l in &links {
            if let Some(s) = tt.links.iter().find(|x| x.from_id == l.from_id && x.to_id == l.to_id) {
                eprintln!("  {} -> {}: {:.1} m (OMSI {:.1} m)", l.from_id, l.to_id, l.length, s.length);
                assert!((l.length - s.length).abs() < s.length * 0.15 + 15.0, "link {} -> {}: {} vs {}", l.from_id, l.to_id, l.length, s.length);
            }
        }
    }
}


/// A line made the way the Lines window makes one, on Grundorf: its stops routed, saved
/// next to the stock files and read back by openOMSI's reader, its track where line 76's runs.
#[test]
fn a_new_line_on_grundorf_saves_and_reads_back() {
    let Some(root) = omsi_root() else { return };
    let cfg = root.join("maps").join("Grundorf").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let mut files = Files::new(vec![root.clone()]);
    let net = Network::build(&doc, &mut files);
    let mut tt = doc.timetable.clone();
    let out = scratch("newline");
    // the stock files first, as a save of an installation map copies them
    write_all(&tt, &out);
    let stops = vec![108, 104, 129, 763];
    let line = tt.add_line("99");
    let trip = tt.add_trip(&line, "99_test", stops.clone());
    let pos: Vec<Option<DVec3>> = stops.iter().map(|id| stop_world(&doc, &mut files, *id)).collect();
    let r = route::find(&net, &pos, false, None);
    assert!(r.ok(), "{:?}", r.problems);
    tt.set_route(&trip, r.track(&net, &doc.order), r.links(&net, &doc.order, &stops));
    tt.lines.iter_mut().find(|l| l.name == line).unwrap().tours.push(omsi_timetable::Tour {
        number: "1".into(),
        ai_group: "Busses".into(),
        extra: ALL_DAYS.to_string(),
        trips: vec![omsi_timetable::TourTrip { trip: trip.clone(), profile: 0, departure: 8.0 * 60.0 }],
    });
    let ids: Vec<(TileKey, i64)> = net.stops.iter().map(|s| (s.0, s.1)).collect();
    tt.sync_stops(stop_entries(&doc, &ids));
    save_to(&mut tt, &out, &doc.order).unwrap();
    let back = TimetableData::load(&out);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    // the stock line is still there, the new one beside it
    assert!(back.lines.iter().any(|l| l.name == "76"));
    let l = back.lines.iter().find(|l| l.name == "99").unwrap();
    assert_eq!(l.tours[0].trips[0].trip, "99_test");
    assert_eq!(back.trip("99_test").unwrap().stations, stops);
    // openOMSI drives a trip by the track named like it: the stock line's first lanes
    let track = back.tracks.iter().find(|t| track_name(t) == "99_test").unwrap();
    let stock = back.tracks.iter().find(|t| track_name(t) == "76_BH-Kk").unwrap();
    let key = |e: &TrackEntry| (e.values[0], e.values[1], e.values[2], e.values[3]);
    let n = track.entries.len();
    assert!(n > 5);
    assert_eq!(track.entries.iter().map(key).collect::<Vec<_>>(), stock.entries[..n].iter().map(key).collect::<Vec<_>>());
    // and OMSI 2 by the station links
    for w in stops.windows(2) {
        assert!(back.stn_links.iter().any(|x| x.from_id == w[0] && x.to_id == w[1]), "link {w:?}");
    }
    assert!(back.bus_stops.iter().any(|s| s.object_id == 763 && s.name == "Elsterplatz"));
    let _ = std::fs::remove_dir_all(&out);
}

/// On a map made in openMapper: a straight two-lane road of two linked splines with a stop
/// on either side; the bus keeps right, so the way runs north past the east stops and south
/// past the west ones.
#[test]
fn a_new_map_routes_on_the_right() {
    let Some(root) = omsi_root() else { return };
    let road = "Splines\\Marcel\\str_2spur_8m_sedan1.sli";
    let mut files = Files::new(vec![root.clone()]);
    if files.get(road).is_none() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let mut doc = Document::new_map(&roots, "tt-test", "tt-test", 1);
    let k = (0, 0);
    let t = &mut doc.tiles.get_mut(&k).unwrap().tile;
    let spline = |id: i64, y: f64, prev: i64, next: i64| omsi_map::MapSpline { file: road.into(), id, prev_id: prev, next_id: next, pos: [150.0, y, 0.0], heading: 0.0, length: 50.0, ..Default::default() };
    t.splines.push(spline(1, 50.0, 0, 2));
    t.splines.push(spline(2, 100.0, 1, 0));
    let stop = |id: i64, x: f64, y: f64, name: &str| omsi_map::MapObject { file: "Sceneryobjects\\Generic\\bus_stop.sco".into(), id, pos: [x, y, 0.0], flag: 7, extra: vec![name.into(), "0".into(), "0".into(), "10".into()], ..Default::default() };
    t.objects.push(stop(10, 156.0, 60.0, "South East"));
    t.objects.push(stop(11, 156.0, 140.0, "North East"));
    t.objects.push(stop(12, 144.0, 140.0, "North West"));
    t.objects.push(stop(13, 144.0, 60.0, "South West"));
    let net = Network::build(&doc, &mut files);
    assert_eq!(net.stops.len(), 4);
    let pos = |ids: &[i64], files: &mut Files| ids.iter().map(|id| stop_world(&doc, files, *id)).collect::<Vec<_>>();
    let north = route::find(&net, &pos(&[10, 11], &mut files), false, None);
    assert!(north.ok(), "{:?}", north.problems);
    let lanes: Vec<(i64, usize)> = north.lanes.iter().map(|l| (net.lanes[*l].key.id, net.lanes[*l].key.path)).collect();
    assert_eq!(lanes, vec![(1, 2), (2, 2)]);
    assert!((north.length() - 80.0).abs() < 1.0, "{}", north.length());
    let south = route::find(&net, &pos(&[12, 13], &mut files), false, None);
    assert!(south.ok(), "{:?}", south.problems);
    let lanes: Vec<(i64, usize)> = south.lanes.iter().map(|l| (net.lanes[*l].key.id, net.lanes[*l].key.path)).collect();
    assert_eq!(lanes, vec![(2, 1), (1, 1)]);
    // a stop off any road
    let mut far = pos(&[10, 11], &mut files);
    far.push(Some(DVec3::new(10.0, 10.0, 0.0)));
    assert_eq!(route::find(&net, &far, false, None).problems, vec![route::Problem::NoRoad(2)]);
    // the track names tile index 0 and OMSI's numbers of the paths: four a spline
    let tr = north.track(&net, &doc.order);
    assert_eq!(tr[0].values, vec![1.0, 2.0, 0.0, 2.0, 50.0, 0.0]);
    assert_eq!(tr[1].values, vec![2.0, 2.0, 0.0, 6.0, 50.0, 0.0]);
    let links = north.links(&net, &doc.order, &[10, 11]);
    assert_eq!(links.len(), 1);
    assert_eq!((links[0].length, links[0].params[2], links[0].params[3]), (80.0, 10.0, 40.0));
}

/// Berlin-Spandau's bus trips routed again: most find their way, about as long as OMSI's
/// own station links say.
#[test]
fn spandau_bus_trips_route() {
    let Some(root) = omsi_root() else { return };
    let cfg = root.join("maps").join("Berlin-Spandau").join("global.cfg");
    if !cfg.is_file() {
        return;
    }
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let doc = Document::load(&roots, &cfg, |_| {}).unwrap();
    let mut files = Files::new(vec![root.clone()]);
    let net = Network::build(&doc, &mut files);
    let tt = &doc.timetable;
    let (mut n, mut found, mut close) = (0, 0, 0);
    for t in tt.trips.iter().filter(|t| t.stations.len() >= 2) {
        n += 1;
        let pos: Vec<Option<DVec3>> = t.stations.iter().map(|id| stop_world(&doc, &mut files, *id)).collect();
        let r = route::find(&net, &pos, doc.global.left_hand_traffic, None);
        if !r.ok() {
            eprintln!("{}: {:?}", t.name, r.problems);
            continue;
        }
        found += 1;
        let stock: Option<f64> = t.stations.windows(2).map(|w| tt.links.iter().find(|l| l.from_id == w[0] && l.to_id == w[1]).map(|l| l.length)).sum();
        if let Some(s) = stock {
            if (r.length() - s).abs() < s * 0.1 + 50.0 {
                close += 1;
            } else {
                eprintln!("{}: {:.0} m, OMSI {:.0} m", t.name, r.length(), s);
            }
        }
    }
    eprintln!("Spandau: {found} of {n} bus trips routed, {close} within 10 % of OMSI's length");
    assert!(found * 10 >= n * 7, "{found} of {n}");
}

/// Undo and redo take the timetable back and forth; quick edits of one kind are one step,
/// and the files of what undo took away go on the next save.
#[test]
fn timetable_edits_undo_and_redo() {
    let Some(root) = omsi_root() else { return };
    let _map = one_map();
    let roots = crate::install::Roots::new(root.clone());
    let mut doc = Document::new_map(&roots, "tt-undo", "tt-undo", 1);
    let mut h = crate::history::History::default();
    let snap = |doc: &Document| crate::history::Snapshot { timetable: Some(doc.timetable.clone()), ..Default::default() };
    let before = snap(&doc);
    doc.timetable.add_line("5");
    h.push_snapshot("tt", before);
    // a second edit straight after: the same step
    let before = snap(&doc);
    doc.timetable.add_trip("5", "5_A", vec![1, 2]);
    h.push_snapshot("tt", before);
    assert!(h.can_undo());
    h.undo(&mut doc);
    assert!(doc.timetable.lines.is_empty() && doc.timetable.trips.is_empty());
    assert!(!h.can_undo());
    let out = scratch("undo");
    // what undo took away is not written; a file of it there before is taken away
    std::fs::write(out.join("TTData").join("5.ttl"), b"x").unwrap();
    save_to(&mut doc.timetable, &out, &[]).unwrap();
    assert!(!out.join("TTData").join("5.ttl").exists());
    h.redo(&mut doc);
    assert_eq!(doc.timetable.lines.len(), 1);
    assert_eq!(doc.timetable.trips[0].stations, vec![1, 2]);
    save_to(&mut doc.timetable, &out, &[]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!((back.lines.len(), back.trips.len()), (1, 1));
    let _ = std::fs::remove_dir_all(&out);
}

/// A tile taken away moves the ones after it up the `[map]` list: the stops' tile indices
/// follow on the next save, also when nothing of the timetable was edited.
#[test]
fn removing_a_tile_renumbers_an_unchanged_timetable() {
    let out = scratch("renumber");
    let mut tt = Timetable::default();
    tt.stops.push(BusStopEntry { name: "East".into(), group: 2, object_id: 5, params: [0.0; 3] });
    tt.touch_stops();
    save_to(&mut tt, &out, &[(0, 0), (1, 0), (2, 0)]).unwrap();
    assert!(!tt.dirty);
    save_to(&mut tt, &out, &[(0, 0), (2, 0)]).unwrap();
    let back = TimetableData::load(&out);
    assert_eq!(back.bus_stops[0].group, 1, "the stop's tile is now the second of the list");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_line_copied_drives_copies_of_its_trips() {
    let mut tt = Timetable::default();
    let l = tt.add_line("76");
    let a = tt.add_trip(&l, "76_A-B", vec![1, 2]);
    let b = tt.add_trip(&l, "76_B-A", vec![2, 1]);
    tt.set_route(&a, vec![Default::default()], Vec::new());
    let line = tt.lines.iter_mut().find(|x| x.name == l).unwrap();
    line.tours.push(omsi_timetable::Tour { number: "1".into(), trips: vec![omsi_timetable::TourTrip { trip: a.clone(), ..Default::default() }, omsi_timetable::TourTrip { trip: b.clone(), ..Default::default() }], ..Default::default() });
    let copy = tt.duplicate_line(&l).unwrap();
    assert_ne!(copy, l);
    let c = tt.lines.iter().find(|x| x.name == copy).unwrap();
    let driven: Vec<&str> = c.tours[0].trips.iter().map(|x| x.trip.as_str()).collect();
    assert_eq!(driven, vec![format!("{copy}_A-B"), format!("{copy}_B-A")]);
    assert_eq!(tt.trip(&format!("{copy}_A-B")).unwrap().stations, vec![1, 2]);
    assert_eq!(tt.trip(&format!("{copy}_A-B")).unwrap().line, copy);
    // (the way found for the first copied along; the second is found again)
    assert!(tt.track(&format!("{copy}_A-B")).is_some());
    // the original as it was
    assert_eq!(tt.lines.iter().find(|x| x.name == l).unwrap().tours[0].trips[0].trip, a);
    assert_eq!(tt.trips.len(), 4);
}

#[test]
fn the_stock_maps_trips_fit_their_roads_as_read() {
    let _map = crate::doc::one_map();
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
        eprintln!("no OMSI 2 installation: skipped");
        return;
    };
    let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join(format!("openmapper-stale-{}", std::process::id())) };
    for name in ["Grundorf", "Berlin-Spandau"] {
        let cfg = omsi.join("maps").join(name).join("global.cfg");
        if !cfg.is_file() {
            continue;
        }
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut files = Files::new(vec![roots.content.clone(), omsi.clone()]);
        let net = Network::build(&doc, &mut files);
        let on_map: std::collections::HashSet<i64> = doc.tiles.values().flat_map(|t| t.tile.splines.iter().map(|s| s.id).chain(t.tile.objects.iter().map(|o| o.id))).collect();
        let every: std::collections::HashSet<usize> = (0..doc.order.len()).collect();
        let stale = super::route::stale_trips(&doc.timetable, &net, &doc.order, &on_map, &every);
        eprintln!("{name}: {} of {} trips stale: {:?}", stale.len(), doc.timetable.trips.len(), stale.iter().take(5).collect::<Vec<_>>());
        // (Spandau lists tiles it does not ship: trips over them cannot fit)
        if name == "Grundorf" {
            assert!(stale.is_empty(), "{stale:?}");
        }
    }
}


/// A tram's stops stand at its track beside a road: its way is found on the track, a bus's
/// on the road; a track never runs on into a lane; a trip found on the tracks stays there.
#[test]
fn a_tram_trip_runs_on_its_track() {
    let tile = (0, 0);
    let mut net = Network::default();
    let north = |x: f64, y0: f64, y1: f64| vec![DVec3::new(x, y0, 0.0), DVec3::new(x, y1, 0.0)];
    // a road north in two pieces, and 8 m east of it a track north in two pieces; where the
    // first piece of track ends a car lane begins as well
    net.add(PathKey { tile, id: 1, path: 0 }, north(0.0, 0.0, 100.0), 100.0, 0, 0, (true, false));
    net.add(PathKey { tile, id: 2, path: 0 }, north(0.0, 100.0, 200.0), 100.0, 1, 0, (true, false));
    net.add(PathKey { tile, id: 3, path: 0 }, north(8.0, 0.0, 100.0), 100.0, 2, 0, (true, true));
    net.add(PathKey { tile, id: 4, path: 0 }, north(8.0, 100.0, 200.0), 100.0, 3, 0, (true, true));
    net.add(PathKey { tile, id: 5, path: 0 }, north(8.0, 100.0, 120.0), 20.0, 4, 0, (true, false));
    net.link();
    let first_track = net.by_key[&PathKey { tile, id: 3, path: 0 }][0];
    let ids: Vec<i64> = net.lanes[first_track].next.iter().map(|&l| net.lanes[l].key.id).collect();
    assert_eq!(ids, vec![4], "a track runs on into the track only");
    let at = |pts: &[(f64, f64)]| pts.iter().map(|p| Some(DVec3::new(p.0, p.1, 0.0))).collect::<Vec<_>>();
    let on = |r: &route::Route| r.lanes.iter().map(|l| net.lanes[*l].key.id).collect::<Vec<_>>();
    // the tram's stops, 3 m east of the track (and 11 m of the road)
    let tram = at(&[(11.0, 50.0), (11.0, 180.0)]);
    let r = route::find(&net, &tram, false, None);
    assert!(r.ok(), "{:?}", r.problems);
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Rail), vec![3, 4]));
    // a bus's stops at the kerb
    let bus = at(&[(2.5, 50.0), (2.5, 180.0)]);
    let r = route::find(&net, &bus, false, None);
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Road), vec![1, 2]));
    // a stop too far from the road for a bus: the tracks still serve it
    let r = route::find(&net, &at(&[(11.0, 50.0), (20.0, 180.0)]), false, None);
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Rail), vec![3, 4]));
    let r = route::find(&net, &at(&[(11.0, 50.0), (20.0, 180.0)]), false, Some(route::Ways::Road));
    assert_eq!(r.problems, vec![route::Problem::NoRoad(1)]);
    // set to the road, the tram's stops are driven past on it
    let r = route::find(&net, &tram, false, Some(route::Ways::Road));
    assert_eq!((r.ways, on(&r)), (Some(route::Ways::Road), vec![1, 2]));
    // a trip whose way was found on the tracks is found there again
    let r = route::find(&net, &tram, false, None);
    let mut tt = Timetable::default();
    let line = tt.add_line("5");
    let trip = tt.add_trip(&line, "5_1", vec![10, 11]);
    assert_eq!(route::ways_of(&tt, &trip, &net, &[tile]), None);
    tt.set_route(&trip, r.track(&net, &[tile]), r.links(&net, &[tile], &[10, 11]));
    assert_eq!(route::ways_of(&tt, &trip, &net, &[tile]), Some(route::Ways::Rail));
}
