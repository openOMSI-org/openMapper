//! Bus lines from OpenStreetMap: each `route=bus` relation of the map's place (one way of a
//! line, its stops in order) becomes a trip of its line through the map's stops nearest to
//! the relation's, the way found along the roads; a line with a trip each way gets a tour
//! there and back all day, and the map a depot when it has none.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use omsi_timetable::{Tour, TourTrip};

use crate::doc::{tile_origin, Document, TileKey};
use crate::timetable::network::{Files, Network};
use crate::timetable::{route, ALL_DAYS};

/// One way of a line: its number, its name and its stops in order (latitude, longitude).
#[derive(Clone, Debug)]
pub struct Route {
    pub number: String,
    pub name: String,
    pub stops: Vec<(f64, f64)>,
}

/// The query for the bus routes within a box, with the nodes they stop at.
pub fn query(bbox: [f64; 4]) -> String {
    format!("[out:json][timeout:90];rel[\"route\"=\"bus\"]({:.6},{:.6},{:.6},{:.6})->.r;.r out body;node(r.r);out;", bbox[0], bbox[1], bbox[2], bbox[3])
}

/// The routes of an answer: the platforms in order (or the stop positions, for a route
/// that names no platforms).
pub fn parse(body: &str) -> Result<Vec<Route>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let els = v.get("elements").and_then(|e| e.as_array()).cloned().unwrap_or_default();
    let nodes: HashMap<i64, (f64, f64)> = els
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("node"))
        .filter_map(|e| Some((e.get("id")?.as_i64()?, (e.get("lat")?.as_f64()?, e.get("lon")?.as_f64()?))))
        .collect();
    let mut out = Vec::new();
    for r in els.iter().filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("relation")) {
        let tag = |k: &str| r.get("tags").and_then(|t| t.get(k)).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let members: Vec<(i64, String)> = r
            .get("members")
            .and_then(|m| m.as_array())
            .into_iter()
            .flatten()
            .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("node"))
            .filter_map(|m| Some((m.get("ref")?.as_i64()?, m.get("role")?.as_str()?.to_string())))
            .collect();
        let pick = |prefix: &str| -> Vec<(f64, f64)> { members.iter().filter(|m| m.1.starts_with(prefix)).filter_map(|m| nodes.get(&m.0).copied()).collect() };
        let platforms = pick("platform");
        let stops = if platforms.len() >= 2 { platforms } else { pick("stop") };
        let number = if tag("ref").is_empty() { tag("name") } else { tag("ref") };
        if stops.len() >= 2 && !number.is_empty() {
            out.push(Route { number, name: tag("name"), stops });
        }
    }
    Ok(out)
}

/// A name a file may have, from anything (OSM's names have slashes and arrows).
fn file_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '-' } else { c }).collect();
    let n = n.trim().trim_end_matches('.').to_string();
    if n.is_empty() { "1".into() } else { n }
}

/// What came of adding the lines.
#[derive(Debug, Default, PartialEq)]
pub struct Added {
    pub lines: usize,
    pub trips: usize,
    /// Trips the way along the roads was found for (the rest are named in the Lines window).
    pub routed: usize,
}

/// The routes (their stops in world coordinates, also those beyond the map) put into the
/// map's timetable.
pub fn add(doc: &mut Document, roots: Vec<std::path::PathBuf>, routes: &[(String, String, Vec<DVec2>)]) -> Added {
    let mut added = Added::default();
    // the map's stop objects, all in the timetable's list
    let stops: Vec<(TileKey, i64, DVec2)> = doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.flag == 7 && o.extra.len() >= 4).map(move |o| (*k, o.id, tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1])))).collect();
    let ids: Vec<(TileKey, i64)> = stops.iter().map(|s| (s.0, s.1)).collect();
    let entries = crate::timetable::stop_entries(doc, &ids);
    doc.timetable.sync_stops(entries);
    let mut files = Files::new(roots);
    let net = Network::build(doc, &mut files);
    let left = doc.global.left_hand_traffic;
    // (a line's trips, there and back, for its tour)
    let mut of_line: Vec<(String, Vec<(String, f32)>)> = Vec::new();
    for (number, name, pts) in routes {
        // each stop of the route at the map's stop nearest to it, in order, none twice in a
        // row; where the route leaves the map (a stop of it with none near) it is cut, and the
        // longest run within the map is the trip
        let mut runs: Vec<Vec<i64>> = vec![Vec::new()];
        for p in pts {
            let near = stops.iter().map(|s| (s.1, (s.2 - *p).length())).filter(|s| s.1 < 30.0).min_by(|a, b| a.1.total_cmp(&b.1));
            match near {
                Some((id, _)) => {
                    let run = runs.last_mut().unwrap();
                    if run.last() != Some(&id) {
                        run.push(id);
                    }
                }
                None if !runs.last().unwrap().is_empty() => runs.push(Vec::new()),
                None => {}
            }
        }
        let ids = runs.into_iter().max_by_key(|r| r.len()).unwrap_or_default();
        if ids.len() < 2 {
            continue;
        }
        // (a route's variants often stop alike: one trip for them)
        let line_name = file_name(number);
        if doc.timetable.trips.iter().any(|t| t.line.eq_ignore_ascii_case(&line_name) && t.stations == ids) {
            continue;
        }
        let line = match doc.timetable.lines.iter().find(|l| l.name.eq_ignore_ascii_case(&line_name)) {
            Some(l) => l.name.clone(),
            None => {
                added.lines += 1;
                doc.timetable.add_line(&line_name)
            }
        };
        let terminus = doc.timetable.stop_name(*ids.last().unwrap()).unwrap_or("").to_string();
        let trip_name = file_name(&format!("{line}_{}", if terminus.is_empty() { name.as_str() } else { terminus.as_str() }));
        let trip = doc.timetable.add_trip(&line, &trip_name, ids.clone());
        added.trips += 1;
        let pos: Vec<Option<DVec3>> = ids.iter().map(|id| crate::timetable::stop_world(doc, &mut files, *id)).collect();
        let r = route::find(&net, &pos, left, None);
        let mut minutes = (pts.len() as f32 * 1.5).max(5.0);
        if r.ok() {
            // (about 18 km/h with the stops)
            minutes = ((r.length() / 1000.0 / 18.0 * 60.0) + 1.0).ceil().max(2.0) as f32;
            let (track, links) = (r.track(&net, &doc.order), r.links(&net, &doc.order, &ids));
            let tt = &mut doc.timetable;
            tt.set_route(&trip, track, links);
            if let Some(p) = tt.trip_mut(&trip).and_then(|x| x.profiles.first_mut()) {
                p.factor = minutes;
            }
            tt.reroute.remove(&trip.to_lowercase());
            added.routed += 1;
        }
        match of_line.iter_mut().find(|l| l.0 == line) {
            Some(l) => l.1.push((trip, minutes)),
            None => of_line.push((line, vec![(trip, minutes)])),
        }
    }
    // a tour for each new line: its trips one after the other from 06:00 to 22:00
    if added.trips > 0 && doc.traffic.depots().next().is_none() {
        crate::ui_timetable::add_depot(doc);
    }
    let depot = doc.traffic.depots().next().map(|d| d.name.clone()).unwrap_or_default();
    for (line, trips) in of_line {
        let tt = &mut doc.timetable;
        let Some(l) = tt.lines.iter_mut().find(|l| l.name == line) else { continue };
        if !l.tours.is_empty() {
            continue;
        }
        let mut list = Vec::new();
        let mut dep = 6.0 * 60.0;
        let mut k = 0;
        while dep <= 22.0 * 60.0 {
            let (t, m) = &trips[k % trips.len()];
            list.push(TourTrip { trip: t.clone(), profile: 0, departure: dep });
            dep += m + 4.0;
            k += 1;
        }
        l.tours.push(Tour { number: "1".into(), ai_group: depot.clone(), extra: ALL_DAYS.to_string(), trips: list });
        tt.touch_line(&line);
    }
    // (the map's own depot files learn the new lines' destinations, stops and routes)
    crate::ui_hof::follow_lines(doc);
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_is_read_with_its_platforms_in_order() {
        let body = r#"{"elements":[
            {"type":"relation","id":1,"members":[{"type":"node","ref":10,"role":"stop"},{"type":"node","ref":11,"role":"platform"},{"type":"way","ref":5,"role":""},{"type":"node","ref":12,"role":"platform_exit_only"}],"tags":{"route":"bus","ref":"130","name":"Bus 130: A => B"}},
            {"type":"node","id":10,"lat":1.0,"lon":1.0},{"type":"node","id":11,"lat":2.0,"lon":2.0},{"type":"node","id":12,"lat":3.0,"lon":3.0}]}"#;
        let r = parse(body).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].number, "130");
        assert_eq!(r[0].stops, vec![(2.0, 2.0), (3.0, 3.0)]);
        assert_eq!(file_name("Bus 130: A => B"), "Bus 130- A => B".replace(['>'], "-"));
    }
}
