use super::*;

fn omsi_root() -> Option<std::path::PathBuf> {
    let r = crate::install::find_omsi_root(&crate::install::Prefs::default());
    if r.is_none() {
        eprintln!("no OMSI 2 installation: skipped");
    }
    r
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("openmapper-traffic-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Every traffic file of the stock maps written again reads back the same through
/// openOMSI's readers.
#[test]
fn stock_traffic_round_trips() {
    let Some(root) = omsi_root() else { return };
    for m in ["Grundorf", "Berlin-Spandau"] {
        let src = root.join("maps").join(m);
        if !src.is_dir() {
            continue;
        }
        let mut t = Traffic::load(&src);
        assert!(!t.legacy, "{m}");
        assert_eq!(t.has_veh_groups, m != "Grundorf");
        assert!(!t.groups.is_empty() && t.depots().count() > 0, "{m}");
        let out = scratch(m);
        t.restored();
        save_to(&mut t, &out).unwrap();
        let back = Traffic::load(&out);
        let stock = omsi_map::AiLists::load(&omsi_cfg::resolve_path(&src, "ailists.cfg")).unwrap();
        let again = omsi_map::AiLists::load(&out.join("ailists.cfg")).unwrap();
        assert_eq!(again.groups, stock.groups, "{m}: ailists.cfg");
        assert_eq!((back.veh_groups.clone(), back.has_veh_groups), (t.veh_groups.clone(), t.has_veh_groups), "{m}");
        assert_eq!(back.density, t.density, "{m}");
        assert_eq!((&back.parked, &back.humans, &back.drivers), (&t.parked, &t.humans, &t.drivers), "{m}");
        // the time zone and the number plates too
        assert_eq!(back.has, t.has, "{m}");
        assert_eq!(back.timezone, t.timezone, "{m}: timezone.txt");
        assert_eq!(back.registrations, t.registrations, "{m}: registrations.txt");
        eprintln!("{m}: {} groups ({} depots), {} unscheduled, {} parked, {} people read back the same", t.groups.len(), t.depots().count(), t.veh_groups.len(), t.parked.len(), t.humans.len());
        let _ = std::fs::remove_dir_all(&out);
    }
}

/// The new map template's ailists.cfg is in the first format: its groups come out right and
/// are written in the second, which openOMSI reads the same.
#[test]
fn the_first_format_is_converted() {
    let Some(root) = omsi_root() else { return };
    let src = root.join("template").join("NewMap");
    if !src.join("ailists.cfg").is_file() {
        return;
    }
    let mut t = Traffic::load(&src);
    assert!(t.legacy);
    let busses = t.groups.iter().find(|g| g.name == "Busses").unwrap();
    assert_eq!(busses.vehicles.len(), 12);
    assert_eq!(busses.hof.as_deref(), Some("Spandau"));
    assert!(busses.vehicles[0].file.to_lowercase().ends_with(".bus"));
    let rest = t.groups.iter().find(|g| g.name == "NotInGroup").unwrap();
    assert_eq!(rest.vehicles.len(), 23 - 12);
    let out = scratch("legacy");
    t.touch(File::AiLists);
    save_to(&mut t, &out).unwrap();
    let again = omsi_map::AiLists::load(&out.join("ailists.cfg")).unwrap();
    assert_eq!(again.groups, t.groups);
    assert!(!t.legacy);
    let _ = std::fs::remove_dir_all(&out);
}

/// An OMSI 1 list (OMSI 1 Velbert's kind) is read as Omsi.exe reads it: a car group named
/// again starts over, and its depot with the old type group (a place in `[ailist]`, then
/// number and repaint pairs) is kept, and written again in the second format.
#[test]
fn an_omsi_1_list_keeps_its_depots() {
    let dir = scratch("omsi1");
    std::fs::write(
        dir.join("ailists.cfg"),
        "[ailist]\n0\n-1\n3\nvehicles\\Kombi\\kombi.ovh\nvehicles\\Coach\\coach.bus\nvehicles\\Van\\van.ovh\n\n\
[aigroup]\nStadtverkehr\n1\n0\n\n\
[aigroup]\nstadtverkehr\n1\n2\n\n\
[aigroup_depot]\nLinienbusse\nHof Waldbach\n\
[aigroup_depot_typgroup]\n1\n101\nrot\n102\nblau\n[end]\n",
    )
    .unwrap();
    let mut t = Traffic::load(&dir);
    assert!(t.legacy);
    let names: Vec<&str> = t.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["stadtverkehr", "Linienbusse", "NotInGroup"]);
    let depot = &t.groups[1];
    assert!(depot.is_depot);
    assert_eq!(depot.typgroups[0].file, "vehicles\\Coach\\coach.bus");
    assert_eq!(depot.typgroups[0].entries.iter().map(|e| (e.number.as_str(), e.paint.as_str())).collect::<Vec<_>>(), [("101", "rot"), ("102", "blau")]);
    let out = scratch("omsi1-out");
    t.touch(File::AiLists);
    save_to(&mut t, &out).unwrap();
    assert_eq!(omsi_map::AiLists::load(&out.join("ailists.cfg")).unwrap().groups, t.groups);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn unscheduled_traffic_and_renames() {
    let mut t = Traffic { groups: vec![car_group("Cars")], ..Default::default() };
    // (no unsched_vehgroups.txt: the first group drives about)
    assert!(t.unscheduled("Cars").0);
    t.set_unscheduled("Cars", true, 1, 0.5);
    assert_eq!(t.unscheduled("cars"), (true, 1, 0.5));
    t.rename_group(0, "Autos");
    assert_eq!(t.unscheduled("Autos"), (true, 1, 0.5));
    t.set_unscheduled("Autos", false, 1, 0.5);
    assert!(!t.unscheduled("Autos").0);
    let out = scratch("unsched");
    t.groups.push(depot("Depot", "Grundorf", "Vehicles\\MAN_NL_NG\\MAN_GN92_main.bus"));
    t.touch(File::AiLists);
    save_to(&mut t, &out).unwrap();
    let back = Traffic::load(&out);
    assert_eq!(back.groups, t.groups);
    assert_eq!(back.density, t.density);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn depot_lines() {
    let e = DepotEntry { number: "2530".into(), registration: "GRN-V 30".into(), ..Default::default() };
    assert_eq!(depot_line(&e), "2530\tGRN-V 30");
    let e = DepotEntry { number: "7".into(), from: Some(19890101), ..Default::default() };
    assert_eq!(DepotEntry::parse(&depot_line(&e)), e);
}

/// The installation's vehicles, parked cars and people are found, with their names.
#[test]
fn the_catalogue_finds_the_installations_vehicles() {
    let Some(root) = omsi_root() else { return };
    let c = catalogue::Catalogue::scan(&[std::env::temp_dir().join("openmapper-no-content"), root]);
    eprintln!("{} vehicles, {} parked, {} people", c.vehicles.len(), c.parked.len(), c.humans.len());
    assert!(c.vehicles.len() > 50 && c.parked.len() > 3 && c.humans.len() > 5);
    let golf = c.vehicles.iter().find(|v| v.rel.eq_ignore_ascii_case("Vehicles\\VW_Golf_2\\AI_VW_Golf_2.bus")).unwrap();
    assert!(golf.name.contains("Golf"), "{}", golf.name);
    assert_eq!(c.name_of("vehicles\\VW_Golf_2\\AI_VW_Golf_2.bus"), golf.name);
    assert!(c.hofs_of(traffic_bus()).iter().any(|h| h == "Grundorf"));
}

fn traffic_bus() -> &'static str {
    DEPOT_BUS
}

#[test]
fn stock_groups_are_found() {
    let Some(root) = omsi_root() else { return };
    let g = stock_groups(&root);
    let cars = g.iter().find(|g| g.name == "NormalCars").unwrap();
    assert!(cars.vehicles.len() > 5);
}
