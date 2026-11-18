//! What the street model forbids, found on a built map (docs/street-model.md, layer G): a
//! road end left free where it is not a dead end, a link with a kink or a gap, an object
//! standing in a carriageway, a path running into a carriageway, a lamp, sign or stop not
//! turned to its road. The fixable ones are put right by `apply`: objects stepped out of the
//! carriageway to the kerb and turned square to their road, ends lying on one another linked.

use std::collections::HashMap;

use glam::DVec2;

use crate::assets::Assets;
use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::library::{Category, Library};
use crate::road_ends::{road_ends, EndState};
use crate::roads;

#[derive(Debug, Clone)]
pub enum StreetFix {
    /// Move the object to this place (world) and turn it to this heading.
    Place { tile: TileKey, id: i64, to: DVec2, heading: f64 },
    /// Turn the object.
    Turn { tile: TileKey, id: i64, heading: f64 },
    Link((TileKey, i64, bool), (TileKey, i64, bool)),
    /// Bring a linked spline's end (its start, or its end when turned round) to this height,
    /// keeping its other end where it is.
    SnapZ { tile: TileKey, id: i64, at_end: bool, z: f64 },
    /// Take the object away (a tree in a carriageway: moved to the kerb it would stand on the
    /// footway).
    Delete { tile: TileKey, id: i64 },
}

/// Whether an object's file names a tree or a bush (trees drawn as crossed cards are objects).
pub fn is_tree_file(file: &str) -> bool {
    let f = file.to_lowercase();
    ["tree", "baum", "derev", "nature", "busch", "bush", "kust", "\\les\\"].iter().any(|w| f.contains(w))
}

#[derive(Debug, Clone)]
pub struct StreetIssue {
    pub kind: &'static str,
    pub at: DVec2,
    pub ids: Vec<i64>,
    pub detail: String,
    pub fix: Option<StreetFix>,
}

/// A road's carriageway sampled every metre: point, heading, the kerbs left and right of the
/// way it is drawn (m from its line, left negative), the whole width's halves.
struct Road {
    id: i64,
    file: String,
    samples: Vec<(DVec2, f64)>,
    kerb: (f64, f64),
}

fn carriageway(model: &crate::assets::SplineModel, mirror: bool) -> Option<(f64, f64)> {
    let m = if mirror { -1.0 } else { 1.0 };
    let lanes: Vec<(f64, f64)> = model.def.paths.iter().filter(|q| q.kind == 0).map(|q| (q.start[0] as f64 * m, q.width as f64)).collect();
    if lanes.is_empty() {
        return None;
    }
    let lo = lanes.iter().map(|(x, w)| x - w / 2.0).fold(f64::MAX, f64::min);
    let hi = lanes.iter().map(|(x, w)| x + w / 2.0).fold(f64::MIN, f64::max);
    Some((lo, hi))
}

/// Whether the ground at `p` is paved: a ground layer of asphalt, concrete or paving laid there
/// (half strength or more) — a yard drive or car park a road may end on.
pub fn on_paving(doc: &Document, p: DVec2) -> bool {
    let k = tile_of(p);
    let Some(t) = doc.tiles.get(&k) else { return false };
    let org = tile_origin(k);
    let ts = crate::doc::tile_size();
    for (li, g) in doc.global.ground_textures.iter().enumerate().skip(1) {
        let name = g.texture.to_lowercase();
        if !["asf", "asphalt", "beton", "concrete", "paving", "plitk"].iter().any(|w| name.contains(w)) {
            continue;
        }
        let Some(Some(mask)) = t.masks.get(li) else { continue };
        let size = crate::doc::mask_size(g);
        if mask.len() != size * size {
            continue;
        }
        let (xi, yi) = (((p.x - org.x) / ts * size as f64) as usize, ((p.y - org.y) / ts * size as f64) as usize);
        if xi < size && yi < size && mask[(size - 1 - yi) * size + xi] >= 128 {
            return true;
        }
    }
    false
}

/// Whether a buffer stop (or a vehicle standing at the end) is within 4 m of a track end.
fn buffer_at(doc: &Document, p: DVec2) -> bool {
    let words = ["loko", "upor", "zagrad", "pzs", "buffer", "prellbock", "tupik"];
    doc.tiles.iter().any(|(k, t)| {
        let org = tile_origin(*k);
        t.tile.objects.iter().any(|o| (org + DVec2::new(o.pos[0], o.pos[1]) - p).length() < 4.0 && { let f = o.file.to_lowercase(); words.iter().any(|w| f.contains(w)) })
    })
}

/// Whether a surface object (a paved area, a plate) lies under `p`.
pub fn on_surface_object(doc: &Document, assets: &mut Assets, p: DVec2) -> bool {
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for o in &t.tile.objects {
            let c = org + DVec2::new(o.pos[0], o.pos[1]);
            if (c - p).length() > 150.0 {
                continue;
            }
            let Some(m) = assets.object(&o.file) else { continue };
            if !m.surface {
                continue;
            }
            // (into the object's own frame: x east, y north when it is not turned)
            let h = -o.rot[0].to_radians();
            let d = p - c;
            let local = DVec2::new(d.x * h.cos() - d.y * h.sin(), d.x * h.sin() + d.y * h.cos());
            if local.x >= m.lo.x as f64 - 0.3 && local.x <= m.hi.x as f64 + 0.3 && local.y >= m.lo.y as f64 - 0.3 && local.y <= m.hi.y as f64 + 0.3 {
                return true;
            }
        }
    }
    false
}

/// The heading of `h` nearest to `want` among want + k·90°.
fn square_to(h: f64, road: f64) -> f64 {
    let d = (h - road).rem_euclid(90.0);
    let k = if d > 45.0 { d - 90.0 } else { d };
    roads::norm_deg(h - k)
}

fn is_sign(file: &str) -> bool {
    let f = file.to_lowercase();
    ["sign", "znak", "schild", "zeichen", "verkehr", "ostanov", "haltestelle"].iter().any(|w| f.contains(w))
}

pub fn check(doc: &Document, lib: &Library, assets: &mut Assets, area: Option<(DVec2, f64)>) -> Vec<StreetIssue> {
    let mut out = Vec::new();
    assets.load_map_models(doc, 60_000);
    let near = |p: DVec2| area.map(|(c, r)| (p - c).length() <= r).unwrap_or(true);
    // the carriageways
    let mut roads_v: Vec<Road> = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
            let cat = lib.find(&sp.file).map(|a| a.category);
            let Some(model) = assets.spline(&sp.file) else { continue };
            if cat.map(|c| c != Category::Roads).unwrap_or(model.is_track()) {
                continue;
            }
            let Some(kerb) = carriageway(&model, sp.mirror) else { continue };
            let c = omsi_geometry::SplineCurve::from_map(sp, org);
            let n = (sp.length.ceil() as usize).max(1);
            let samples = (0..=n).map(|i| {
                let s = sp.length * i as f64 / n as f64;
                (c.point_at(s).truncate(), c.heading_at(s))
            });
            roads_v.push(Road { id: sp.id, file: sp.file.clone(), samples: samples.collect(), kerb });
        }
    }
    let cell = 4.0;
    let key = |p: DVec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (ri, r) in roads_v.iter().enumerate() {
        for (si, (p, _)) in r.samples.iter().enumerate() {
            grid.entry(key(*p)).or_default().push((ri, si));
        }
    }
    // where a point lies against the carriageways near it: (road, its heading there, the
    // across offset from its line (right positive), inside its carriageway)
    let place = |p: DVec2, reach: f64| -> Vec<(usize, f64, f64, bool)> {
        let (cx, cy) = key(p);
        let span = (reach / cell).ceil() as i64;
        let mut best: HashMap<usize, (f64, f64, f64)> = HashMap::new();
        for dx in -span..=span {
            for dy in -span..=span {
                for &(ri, si) in grid.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    let (q, h) = roads_v[ri].samples[si];
                    let along = roads::dir(h).dot(p - q);
                    if along.abs() > 0.6 {
                        continue;
                    }
                    let d = (p - q).length();
                    if best.get(&ri).map(|b| d < b.0).unwrap_or(true) {
                        best.insert(ri, (d, h, roads::right(h).dot(p - q)));
                    }
                }
            }
        }
        // (inside: more than 0.3 m within the kerb line, past what a kerb's own width blurs)
        best.into_iter().filter(|(_, b)| b.0 <= reach).map(|(ri, (_, h, x))| { let k = roads_v[ri].kerb; (ri, h, x, x > k.0 + 0.3 && x < k.1 - 0.3) }).collect()
    };

    // ends: free where not a dead end, lying on one another unjoined, paths into a carriageway
    let ends = road_ends(doc, lib, assets, false);
    // (at the map's edge a road is cut, not ended)
    let edge = |p: DVec2| [DVec2::X, -DVec2::X, DVec2::Y, -DVec2::Y].iter().any(|d| !doc.tiles.contains_key(&tile_of(p + *d * 3.0)));
    for (i, e) in ends.iter().enumerate() {
        let p = e.pos.truncate();
        if !near(p) {
            continue;
        }
        match e.state {
            EndState::Touching => {
                if let Some(o) = ends.iter().enumerate().find(|(j, o)| *j != i && o.state == EndState::Touching && (o.pos - e.pos).truncate().length() < 0.6 && (o.pos.z - e.pos.z).abs() < 0.5 && o.cat == e.cat) {
                    if i < o.0 {
                        out.push(StreetIssue { kind: "ends_unjoined", at: p, ids: vec![e.id, o.1.id], detail: "two ends lie on one another, not linked".into(), fix: Some(StreetFix::Link((e.tile, e.id, e.at_end), (o.1.tile, o.1.id, o.1.at_end))) });
                    }
                }
            }
            EndState::Free if !edge(p) => {
                if e.cat == Category::Paths {
                    if let Some((ri, _, x, _)) = place(p, 30.0).into_iter().find(|r| r.3) {
                        out.push(StreetIssue { kind: "path_into_road", at: p, ids: vec![e.id, roads_v[ri].id], detail: format!("a path ends inside the carriageway of {} ({x:.1} m off its line): paths end at the kerb", roads_v[ri].id), fix: None });
                    }
                } else if e.cat == Category::Tracks && !buffer_at(doc, p) {
                    out.push(StreetIssue { kind: "track_end_free", at: p, ids: vec![e.id], detail: "a track ends with nothing there: no buffer, no switch, not the map edge".into(), fix: None });
                } else if e.cat == Category::Roads && !on_paving(doc, p) && !on_surface_object(doc, assets, p) {
                    out.push(StreetIssue { kind: "road_end_free", at: p, ids: vec![e.id], detail: "a road ends in the open: nothing there, no paving (join it, end it at a plate, or pave where it ends)".into(), fix: None });
                }
            }
            _ => {}
        }
    }
    // links: a kink or a gap where one spline runs on into the next
    let mut by_id: HashMap<i64, (TileKey, &omsi_map::MapSpline)> = HashMap::new();
    for (k, t) in &doc.tiles {
        for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
            by_id.insert(sp.id, (*k, sp));
        }
    }
    for (k, t) in &doc.tiles {
        for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
            // the joints at this spline's end and, when the other one is turned round to meet it
            // start to start, at its start: (at this one's end, the other, at the other's end)
            let mut joints = Vec::new();
            if let Some((k2, nx)) = by_id.get(&sp.next_id).filter(|_| sp.next_id != 0) {
                // (two joined end to end name each other: one joint, looked at once - from both, each
                // fix would bring the other's end to its own old height, and the step stayed)
                if !(nx.next_id == sp.id && nx.prev_id != sp.id && nx.id < sp.id) {
                    joints.push((true, *k2, *nx, nx.prev_id != sp.id));
                }
            }
            if let Some((k2, nx)) = by_id.get(&sp.prev_id).filter(|_| sp.prev_id != 0) {
                // (one running on into this one is looked at from its own end; two joined start to
                // start, once, from the lower IDCode)
                if nx.prev_id == sp.id && nx.next_id != sp.id && sp.id < nx.id {
                    joints.push((false, *k2, *nx, false));
                }
            }
            if joints.is_empty() {
                continue;
            }
            // (only what is driven or walked along runs on smoothly: walls and fences turn corners)
            let carries = lib.find(&sp.file).map(|a| matches!(a.category, Category::Roads | Category::Tracks | Category::Paths)).unwrap_or_else(|| assets.spline(&sp.file).map(|m| m.joinable()).unwrap_or(false));
            if !carries {
                continue;
            }
            let a = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
            for (at_end, k2, nx, nx_at_end) in joints {
                let b = omsi_geometry::SplineCurve::from_map(nx, tile_origin(k2));
                // (each the way it is driven through the joint: out of this one, into the other)
                let (pa, ha) = if at_end { (a.end_point(), a.heading_at(sp.length)) } else { (a.point_at(0.0), roads::norm_deg(a.heading_at(0.0) + 180.0)) };
                let (pb, hb) = if nx_at_end { (b.end_point(), roads::norm_deg(b.heading_at(nx.length) + 180.0)) } else { (b.point_at(0.0), b.heading_at(0.0)) };
                if !near(pa.truncate()) {
                    continue;
                }
                // (structures off the ground — a footbridge's flights and landings — are drawn as they
                // are built, turning at their landings)
                if pa.z - doc.ground(pa.truncate()) > 1.5 && pb.z - doc.ground(pb.truncate()) > 1.5 {
                    continue;
                }
                let gap = (pa - pb).length();
                let kink = (roads::norm_deg(ha - hb + 180.0) - 180.0).abs();
                if gap > 0.05 || kink > 1.5 {
                    // (a step in height only: the other one is brought to this one's height there)
                    let flat_gap = (pa - pb).truncate().length();
                    let fix = (kink <= 1.5 && flat_gap < 0.02).then(|| StreetFix::SnapZ { tile: k2, id: nx.id, at_end: nx_at_end, z: pa.z });
                    out.push(StreetIssue { kind: "link_kink", at: pa.truncate(), ids: vec![sp.id, nx.id], detail: format!("joined with a {kink:.1}° kink and a {gap:.2} m gap"), fix });
                }
            }
        }
    }
    // objects: in a carriageway, or a lamp, sign or stop not square to its road
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for o in &t.tile.objects {
            let p = org + DVec2::new(o.pos[0], o.pos[1]);
            if !near(p) {
                continue;
            }
            let Some(model) = assets.object(&o.file) else { continue };
            // (what lies flat on the road is meant to be there: zebras, markings, crossing decks)
            let flat = model.hi.z - model.lo.z < 0.3;
            if model.surface || !model.paths.is_empty() || model.technical || model.editor_parts || flat {
                continue;
            }
            let asset = lib.find(&o.file);
            let roadside = asset.map(|a| a.lamp || a.is_bus_stop || a.light_head).unwrap_or(false) || is_sign(&o.file);
            let here = place(p, 25.0);
            if let Some((ri, h, x, _)) = here.iter().find(|r| r.3).copied() {
                let r = &roads_v[ri];
                // (out to the nearer kerb, 0.9 m past it)
                let to_right = (r.kerb.1 - x) < (x - r.kerb.0);
                let nx = if to_right { r.kerb.1 + 0.9 } else { r.kerb.0 - 0.9 };
                let to = p + roads::right(h) * (nx - x);
                let fix = if model.tree.is_some() || is_tree_file(&o.file) { StreetFix::Delete { tile: *k, id: o.id } } else { StreetFix::Place { tile: *k, id: o.id, to, heading: if roadside { square_to(o.rot[0], h) } else { o.rot[0] } } };
                out.push(StreetIssue { kind: "object_on_road", at: p, ids: vec![o.id, r.id], detail: format!("{} stands in the carriageway of {} ({})", o.file, r.id, r.file), fix: Some(fix) });
                continue;
            }
            if roadside {
                // (its road: the nearest within 8 m of the kerb)
                if let Some((_, h, _, _)) = here.iter().filter(|r| { let k = roads_v[r.0].kerb; r.2 > k.0 - 8.0 && r.2 < k.1 + 8.0 }).min_by(|a, b| a.2.abs().total_cmp(&b.2.abs())).copied() {
                    let want = square_to(o.rot[0], h);
                    let off = (roads::norm_deg(o.rot[0] - want + 180.0) - 180.0).abs();
                    if off > 5.0 {
                        out.push(StreetIssue { kind: "object_not_square", at: p, ids: vec![o.id], detail: format!("{} is turned {off:.0}° off its road", o.file), fix: Some(StreetFix::Turn { tile: *k, id: o.id, heading: want }) });
                    }
                }
            }
        }
    }
    out
}

/// Put right what can be: gives back how many.
pub fn apply(doc: &mut Document, issues: &[StreetIssue]) -> usize {
    let mut n = 0;
    for i in issues {
        match &i.fix {
            Some(StreetFix::Place { tile, id, to, heading }) => {
                // (into the tile of its new place)
                let Some(mut o) = doc.object(*tile, *id).cloned() else { continue };
                let k2 = tile_of(*to);
                if !doc.tiles.contains_key(&k2) {
                    continue;
                }
                let org = tile_origin(k2);
                o.pos[0] = to.x - org.x;
                o.pos[1] = to.y - org.y;
                o.rot[0] = *heading;
                if k2 == *tile {
                    if let Some(m) = doc.object_mut(*tile, *id) {
                        *m = o;
                    }
                    doc.tiles.get_mut(tile).unwrap().touch_content();
                } else {
                    let t = doc.tiles.get_mut(tile).unwrap();
                    t.tile.objects.retain(|x| x.id != *id);
                    t.touch_content();
                    let t2 = doc.tiles.get_mut(&k2).unwrap();
                    t2.tile.objects.push(o);
                    t2.touch_content();
                }
                n += 1;
            }
            Some(StreetFix::Turn { tile, id, heading }) => {
                if let Some(m) = doc.object_mut(*tile, *id) {
                    m.rot[0] = *heading;
                    doc.tiles.get_mut(tile).unwrap().touch_content();
                    n += 1;
                }
            }
            Some(StreetFix::SnapZ { tile, id, at_end, z }) => {
                let org = tile_origin(*tile);
                let Some(sp) = doc.spline(*tile, *id).cloned() else { continue };
                let c = omsi_geometry::SplineCurve::from_map(&sp, org);
                let (z0, z1) = (c.point_at(0.0).z, c.end_point().z);
                let (z0, z1) = if *at_end { (z0, *z) } else { (*z, z1) };
                if let Some(m) = doc.tiles.get_mut(tile).and_then(|t| t.tile.splines.iter_mut().find(|s| s.id == *id)) {
                    m.pos[2] = z0;
                    m.delta_h = Some(z1 - z0);
                    m.is_h = true;
                }
                doc.tiles.get_mut(tile).unwrap().touch_content();
                n += 1;
            }
            Some(StreetFix::Delete { tile, id }) => {
                if let Some(t) = doc.tiles.get_mut(tile) {
                    let before = t.tile.objects.len();
                    t.tile.objects.retain(|o| o.id != *id);
                    if t.tile.objects.len() != before {
                        t.touch_content();
                        n += 1;
                    }
                }
            }
            Some(StreetFix::Link(a, b)) => {
                if crate::check::apply_link(doc, *a, *b) {
                    n += 1;
                }
            }
            None => {}
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two roads joined end to end with a step in height: one issue, and putting it right
    /// leaves their ends at one height (each was brought to the other's old one).
    #[test]
    fn a_step_between_roads_joined_end_to_end_is_put_right() {
        use crate::library::{Asset, Kind};
        let mut doc = crate::selection::tests::flat_map(1);
        // (north to y 150, and south down to it: both end there)
        crate::selection::tests::add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 50.0, 0, 2);
        crate::selection::tests::add_spline(&mut doc, (0, 0), 2, 100.0, 200.0, 180.0, 50.0, 0, 1);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[1].pos[2] = 1.0;
        let mut lib = Library::default();
        lib.assets.push(Asset { kind: Kind::Spline, rel: "Splines\\road.sli".into(), name: "road".into(), dir: String::new(), category: Category::Roads, haystack: String::new(), from_content: false, is_bus_stop: false, is_entry_point: false, is_car_park: false, abs_height: false, lamp: false, light_head: false, light_program: false, helpers: Vec::new(), paths: 0, half_width: 0.0, lanes: (0, 0, 0) });
        lib.reindex_for_tests();
        let dir = std::env::temp_dir().join("openmapper-street-check-none");
        let mut assets = Assets::new(dir.clone(), dir);
        let found: Vec<StreetIssue> = check(&doc, &lib, &mut assets, None).into_iter().filter(|i| i.kind == "link_kink").collect();
        assert_eq!(found.len(), 1, "one joint: {found:?}");
        apply(&mut doc, &found);
        let end = |id: i64| {
            let sp = doc.spline((0, 0), id).unwrap();
            omsi_geometry::SplineCurve::from_map(sp, tile_origin((0, 0))).end_point().z
        };
        assert!((end(1) - end(2)).abs() < 1e-6, "{} vs {}", end(1), end(2));
    }

    /// Two roads joined start to start (the one turned round to meet the other) are looked at
    /// too: a step between them was never found.
    #[test]
    fn a_step_between_roads_joined_start_to_start_is_found_and_put_right() {
        use crate::library::{Asset, Kind};
        let mut doc = crate::selection::tests::flat_map(1);
        // (north from y 150, and south from it: both start there)
        crate::selection::tests::add_spline(&mut doc, (0, 0), 1, 100.0, 150.0, 0.0, 50.0, 2, 0);
        crate::selection::tests::add_spline(&mut doc, (0, 0), 2, 100.0, 150.0, 180.0, 50.0, 1, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[1].pos[2] = 1.0;
        let mut lib = Library::default();
        lib.assets.push(Asset { kind: Kind::Spline, rel: "Splines\\road.sli".into(), name: "road".into(), dir: String::new(), category: Category::Roads, haystack: String::new(), from_content: false, is_bus_stop: false, is_entry_point: false, is_car_park: false, abs_height: false, lamp: false, light_head: false, light_program: false, helpers: Vec::new(), paths: 0, half_width: 0.0, lanes: (0, 0, 0) });
        lib.reindex_for_tests();
        let dir = std::env::temp_dir().join("openmapper-street-check-none");
        let mut assets = Assets::new(dir.clone(), dir);
        let found: Vec<StreetIssue> = check(&doc, &lib, &mut assets, None).into_iter().filter(|i| i.kind == "link_kink").collect();
        assert_eq!(found.len(), 1, "one joint: {found:?}");
        apply(&mut doc, &found);
        let start = |id: i64| doc.spline((0, 0), id).unwrap().pos[2];
        assert!((start(1) - start(2)).abs() < 1e-6, "{} vs {}", start(1), start(2));
    }

    #[test]
    fn a_heading_is_squared_to_the_nearest_quarter_of_its_road() {
        assert!((square_to(93.0, 0.0) - 90.0).abs() < 1e-9);
        assert!((square_to(10.0, 15.0) - 15.0).abs() < 1e-9);
        assert!((square_to(359.0, 44.0) - 314.0).abs() < 1e-9);
        assert!((square_to(200.0, 20.0) - 200.0).abs() < 1e-9);
    }
}
