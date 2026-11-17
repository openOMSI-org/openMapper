//! Complete to: a new piece of spline laid between two free ends, leaving the one the way it
//! points, arriving at the other the way that one goes on, at the heights of both and climbing
//! as they climb. The two pieces it joins are not changed (OMSI 2's editor bends the first one
//! round to meet the other): only their links take the new piece in, so a ring of kerbs is
//! closed and can be filled as a shape.
//!
//! The new piece is what OMSI's splines can be: straights and arcs, as few as fit (one arc, two
//! that meet tangentially, or arc, straight, arc), cut at 50 m like a drawn road. Its height is
//! one cubic over the whole length, each spline a `[spline_h]` piece of it, so it meets both
//! ends' heights and gradients exactly.

use glam::DVec3;
use omsi_map::MapSpline;

use crate::doc::{tile_of, tile_origin, Document, Sel, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::roads::{self, Seg};

/// What Complete to does, for its button.
pub fn help() -> &'static str {
    tr(
        "A new piece of the first selected spline's type between two free ends of the selected splines, meeting both in place, direction and height; the two are not changed. Again: the two ends left are joined, closing the ring.",
        "Новый кусок типа первого выбранного сплайна между двумя свободными концами выбранных сплайнов, совпадающий с обоими по месту, направлению и высоте; сами они не меняются. Ещё раз: соединяются два оставшихся конца, кольцо замыкается."
    )
}

/// A free end of a spline, as a spline going out of it would continue it.
#[derive(Debug, Clone, Copy)]
pub struct End {
    pub key: TileKey,
    pub id: i64,
    /// The spline's end (else its start).
    pub at_end: bool,
    pub pos: DVec3,
    /// The way out of the spline (degrees).
    pub heading: f64,
    /// Rise per metre going out.
    pub slope: f64,
    /// The cant (percent) a spline going out has there.
    pub cant: f64,
}

/// The end of spline `id` (its end, else its start), free or not.
pub fn end_of(doc: &Document, k: TileKey, id: i64, at_end: bool) -> Option<End> {
    let s = doc.spline(k, id)?;
    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
    Some(if at_end {
        End { key: k, id, at_end, pos: c.end_point(), heading: roads::norm_deg(c.heading_at(s.length)), slope: c.slope_at(s.length), cant: s.cant_end }
    } else {
        // (going out of its start runs against it: it falls where the spline climbs, and leans
        // the other way)
        End { key: k, id, at_end, pos: c.point_at(0.0), heading: roads::norm_deg(s.heading + 180.0), slope: -c.slope_at(0.0), cant: -s.cant_start }
    })
}

/// The free ends of `splines`, in their order.
pub fn free_ends(doc: &Document, splines: &[(TileKey, i64)]) -> Vec<End> {
    let mut out = Vec::new();
    for (k, id) in splines {
        let Some(s) = doc.spline(*k, *id) else { continue };
        for (free, at_end) in [(s.prev_id == 0, false), (s.next_id == 0, true)] {
            if free {
                out.extend(end_of(doc, *k, *id, at_end));
            }
        }
    }
    out
}

/// How far from an end's place and way the planned piece may arrive (m, degrees).
const MISS: f64 = 1e-3;
const MISS_DEG: f64 = 1e-3;

/// The plan of the piece from `a` to `b`: straights and arcs leaving `a` the way out of it and
/// arriving at `b` against the way out of it. None where none fits.
pub fn plan(a: &End, b: &End) -> Option<Vec<Seg>> {
    let p = a.pos.truncate();
    let q = b.pos.truncate();
    let chord = (q - p).length();
    if chord < 0.5 {
        return None;
    }
    let hq = roads::norm_deg(b.heading + 180.0);
    let arrives = |segs: &Vec<Seg>| {
        let Some(last) = segs.last() else { return false };
        let off = roads::norm_deg(last.end_heading() - hq);
        (last.end() - q).length() < MISS && off.min(360.0 - off) < MISS_DEG
    };
    let mut made: Vec<Vec<Seg>> = Vec::new();
    // (one arc, where the two ways allow it)
    if let Some(s) = roads::arc_to_any(p, a.heading, q) {
        made.push(vec![s]);
    }
    if let Some(v) = roads::biarc(p, a.heading, q, hq) {
        made.push(v);
    }
    for r in [roads::MIN_RADIUS, crate::join::MIN_PIECE_RADIUS] {
        if let Some(v) = crate::join::plan(p, a.heading, q, hq, crate::join::Side::Auto, r) {
            made.push(v);
        }
    }
    crate::join::best(made.into_iter().filter(arrives).collect(), chord)
}

/// The height and gradient (percent) at each joint of `pieces` (one more than there are
/// pieces): one cubic over the whole, from `a`'s height climbing as `a` does out of it, to
/// `b`'s height arriving as `b` climbs into it.
pub fn heights(pieces: &[Seg], a: &End, b: &End) -> Vec<(f64, f64)> {
    let total = roads::total_length(pieces).max(1e-6);
    let (z0, m0, z1, m1) = (a.pos.z, a.slope, b.pos.z, -b.slope);
    let at = |s: f64| {
        let t = (s / total).clamp(0.0, 1.0);
        let (t2, t3) = (t * t, t * t * t);
        let z = (2.0 * t3 - 3.0 * t2 + 1.0) * z0 + (t3 - 2.0 * t2 + t) * total * m0 + (3.0 * t2 - 2.0 * t3) * z1 + (t3 - t2) * total * m1;
        let dz = ((6.0 * t2 - 6.0 * t) * z0 + (3.0 * t2 - 4.0 * t + 1.0) * total * m0 + (6.0 * t - 6.0 * t2) * z1 + (3.0 * t2 - 2.0 * t) * total * m1) / total;
        (z, dz * 100.0)
    };
    let mut out = vec![at(0.0)];
    let mut s = 0.0;
    for p in pieces {
        s += p.length;
        out.push(at(s));
    }
    // (the ends exactly)
    out[0].0 = z0;
    out.last_mut().unwrap().0 = z1;
    out
}

/// The new piece from `a` to `b` put into the map, of the type of `a`'s spline, its pieces
/// linked to each other and to the two ends. Their keys, in order.
pub fn lay(doc: &mut Document, a: &End, b: &End) -> Result<Vec<(TileKey, i64)>, &'static str> {
    if (b.pos.truncate() - a.pos.truncate()).length() < 0.5 {
        return Err(tr("The two ends already meet", "Эти два конца уже сходятся"));
    }
    let template = doc.spline(a.key, a.id).cloned().ok_or(tr("Select the splines whose free ends are to be joined", "Выберите сплайны, свободные концы которых нужно соединить"))?;
    let segs = plan(a, b).ok_or(tr("No piece of spline can join these two ends", "Никакой кусок сплайна не соединит эти два конца"))?;
    let pieces = roads::split(&segs, 50.0);
    if pieces.iter().any(|p| !doc.tiles.contains_key(&tile_of(p.start))) {
        return Err(tr("The road leaves the map: add tiles there first", "Дорога выходит за карту: сначала добавьте тайлы"));
    }
    let profile = heights(&pieces, a, b);
    let total = roads::total_length(&pieces).max(1e-6);
    // (it goes on as the first one looks: run the other way, a spline is drawn mirrored)
    let mirror = template.mirror != !a.at_end;
    let mut tex = if a.at_end { template.tex_offset + template.length } else { 0.0 };
    let ids: Vec<i64> = pieces.iter().map(|_| doc.new_id()).collect();
    let mut made = Vec::new();
    let mut s = 0.0;
    for (i, p) in pieces.iter().enumerate() {
        let k = tile_of(p.start);
        let org = tile_origin(k);
        let (za, ga) = profile[i];
        let (zb, gb) = profile[i + 1];
        let cant = |at: f64| a.cant + (-b.cant - a.cant) * at / total;
        let sp = MapSpline {
            id: ids[i],
            prev_id: if i == 0 { a.id } else { ids[i - 1] },
            next_id: if i + 1 == ids.len() { b.id } else { ids[i + 1] },
            pos: [p.start.x - org.x, p.start.y - org.y, za],
            heading: p.heading,
            length: p.length,
            radius: p.radius,
            grad_start: ga,
            grad_end: gb,
            delta_h: Some(zb - za),
            is_h: true,
            cant_start: cant(s),
            cant_end: cant(s + p.length),
            skew_start: 0.0,
            skew_end: 0.0,
            tex_offset: tex,
            map_chain_offset: Some(tex),
            mirror,
            rules: Vec::new(),
            deleted: false,
            ..template.clone()
        };
        s += p.length;
        tex += p.length;
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.splines.push(sp);
        t.touch_content();
        made.push((k, ids[i]));
    }
    // the two ends take the new piece in (their shape stays as it was)
    for (e, mine) in [(a, ids[0]), (b, *ids.last().unwrap())] {
        if let Some(t) = doc.tiles.get_mut(&e.key) {
            if let Some(sp) = t.tile.splines.iter_mut().find(|sp| sp.id == e.id && !sp.deleted) {
                if e.at_end {
                    sp.next_id = mine;
                } else {
                    sp.prev_id = mine;
                }
            }
            t.touch_content();
        }
    }
    Ok(made)
}

/// The spline and its neighbours as far as they are linked: the chain it belongs to (a ring
/// counted once).
fn chain_of(doc: &Document, id: i64) -> Vec<i64> {
    let mut seen = vec![id];
    let mut todo = vec![id];
    while let Some(i) = todo.pop() {
        let Some((_, s)) = doc.find_spline(i) else { continue };
        for n in [s.prev_id, s.next_id] {
            if n != 0 && !seen.contains(&n) && seen.len() < 100_000 {
                seen.push(n);
                todo.push(n);
            }
        }
    }
    seen
}

/// The two ends to join among the free ends of `splines`: two of them, as they are; of more,
/// the two of different chains that the shortest piece joins (two of one chain, closing it,
/// only when there is nothing else). The first is of the spline selected first.
pub fn pick(doc: &Document, splines: &[(TileKey, i64)]) -> Option<(End, End)> {
    let ends = free_ends(doc, splines);
    if ends.len() < 2 {
        return None;
    }
    if ends.len() == 2 {
        return Some((ends[0], ends[1]));
    }
    let chains: Vec<Vec<i64>> = ends.iter().map(|e| chain_of(doc, e.id)).collect();
    let mut best: Option<(bool, f64, usize, usize)> = None;
    for i in 0..ends.len() {
        for j in i + 1..ends.len() {
            let Some(segs) = plan(&ends[i], &ends[j]) else { continue };
            let closes = chains[i].contains(&ends[j].id);
            let len = roads::total_length(&segs);
            if best.is_none_or(|(c, l, _, _)| (closes, len) < (c, l)) {
                best = Some((closes, len, i, j));
            }
        }
    }
    best.map(|(_, _, i, j)| (ends[i], ends[j]))
}

impl Editor {
    /// Complete to: a new piece between two free ends of the selected splines, of the first
    /// one's type; the two are not changed. One undo step; the selection stays, so the same
    /// key again joins the two ends left (closing a ring).
    pub fn complete_to(&mut self) {
        let splines: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let Some(doc) = self.doc.as_mut() else { return };
        let Some((a, b)) = pick(doc, &splines) else {
            self.toast(tr("Select the splines whose free ends are to be joined", "Выберите сплайны, свободные концы которых нужно соединить"));
            return;
        };
        self.history.begin(tr("Complete to", "Достроить до"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        match lay(doc, &a, &b) {
            Ok(_) => self.history.commit(),
            Err(e) => {
                self.history.cancel();
                self.error(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_spline, flat_map};

    /// A piece of spline exactly as it is in the map.
    fn curve(doc: &Document, id: i64) -> (MapSpline, omsi_geometry::SplineCurve) {
        let (k, s) = doc.find_spline(id).unwrap();
        (s.clone(), omsi_geometry::SplineCurve::from_map(s, tile_origin(k)))
    }

    /// The new pieces, from `a`'s end along the links to `b`: each meets the next in place,
    /// way and height, and the first and last meet the two ends.
    fn check_joined(doc: &Document, made: &[(TileKey, i64)], a: &End, b: &End) {
        assert!(!made.is_empty());
        let mut at = a.pos;
        let mut way = a.heading;
        let mut slope = a.slope;
        for (_, id) in made {
            let (s, c) = curve(doc, *id);
            let start = c.point_at(0.0);
            assert!((start - at).length() < 1e-3, "piece {id} starts at {start:?}, not at {at:?}");
            let off = roads::norm_deg(s.heading - way);
            assert!(off.min(360.0 - off) < 1e-3, "piece {id} leaves at {} not {way}", s.heading);
            assert!((c.slope_at(0.0) - slope).abs() < 1e-6, "piece {id} climbs {} not {slope}", c.slope_at(0.0));
            at = c.end_point();
            way = roads::norm_deg(c.heading_at(s.length));
            slope = c.slope_at(s.length);
        }
        assert!((at - b.pos).length() < 1e-3, "it ends at {at:?}, not at {:?}", b.pos);
        let into = roads::norm_deg(b.heading + 180.0);
        let off = roads::norm_deg(way - into);
        assert!(off.min(360.0 - off) < 1e-3, "it arrives at {way}, not {into}");
        assert!((slope + b.slope).abs() < 1e-6, "it arrives climbing {slope}, not {}", -b.slope);
    }

    /// Two half circles of kerb facing each other, as the issue's island: the first Complete
    /// to joins one pair of ends with a straight, the second closes the ring with the other.
    #[test]
    fn two_half_circles_are_closed_into_a_ring() {
        let mut doc = flat_map(1);
        // (west: from (100, 120) north round to (100, 100)... as a half circle turning left,
        // and east the same turned round)
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 270.0, std::f64::consts::PI * 10.0, 0, 0);
        add_spline(&mut doc, (0, 0), 2, 200.0, 120.0, 90.0, std::f64::consts::PI * 10.0, 0, 0);
        for s in doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.iter_mut() {
            s.radius = 10.0;
            s.file = "Splines\\kerb.sli".into();
            s.pos[2] = if s.id == 1 { 2.0 } else { 5.0 };
            s.grad_start = 1.0;
            s.grad_end = 3.0;
        }
        let before: Vec<MapSpline> = doc.tiles[&(0, 0)].tile.splines.clone();
        let sel = [((0, 0), 1), ((0, 0), 2)];
        let (a, b) = pick(&doc, &sel).expect("two ends to join");
        assert_eq!(a.id, 1, "from the spline selected first");
        let made = lay(&mut doc, &a, &b).unwrap();
        check_joined(&doc, &made, &a, &b);
        // (a straight 100 m across, cut in two)
        let (s, _) = curve(&doc, made[0].1);
        assert_eq!(s.radius, 0.0);
        assert_eq!(s.file, "Splines\\kerb.sli");
        // the second time: the two ends left, closing the ring
        let (a2, b2) = pick(&doc, &sel).expect("the other two ends");
        assert!(a2.id == 1 && b2.id == 2 && a2.at_end != a.at_end);
        let made2 = lay(&mut doc, &a2, &b2).unwrap();
        check_joined(&doc, &made2, &a2, &b2);
        assert!(pick(&doc, &sel).is_none(), "a closed ring has no free end");
        // (the two half circles kept their shape; only their links changed)
        for old in &before {
            let (now, _) = curve(&doc, old.id);
            assert_eq!(MapSpline { prev_id: 0, next_id: 0, ..now.clone() }, MapSpline { prev_id: 0, next_id: 0, ..old.clone() });
            assert!(now.prev_id != 0 && now.next_id != 0);
        }
    }

    /// Ends that point past each other at an angle, at different heights and gradients: still
    /// met exactly, in place, way and climb.
    #[test]
    fn crooked_ends_are_met_exactly() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 50.0, 50.0, 30.0, 20.0, 0, 0);
        add_spline(&mut doc, (0, 0), 2, 260.0, 300.0, 200.0, 25.0, 0, 0);
        {
            let t = doc.tiles.get_mut(&(0, 0)).unwrap();
            t.tile.splines[0].radius = -40.0;
            t.tile.splines[0].grad_end = 4.0;
            t.tile.splines[1].pos[2] = 7.0;
            t.tile.splines[1].radius = 60.0;
            t.tile.splines[1].grad_start = -2.0;
            t.tile.splines[1].delta_h = Some(-1.0);
            t.tile.splines[1].is_h = true;
        }
        // (from the first one's end to the second's end, both pointing at the gap)
        let a = end_of(&doc, (0, 0), 1, true).unwrap();
        let b = end_of(&doc, (0, 0), 2, true).unwrap();
        let made = lay(&mut doc, &a, &b).unwrap();
        check_joined(&doc, &made, &a, &b);
        // (and the links: the new first piece's start is the first spline's end, its last
        // piece's end the second spline's end, which runs back into it)
        assert_eq!(doc.spline((0, 0), 1).unwrap().next_id, made[0].1);
        assert_eq!(doc.spline((0, 0), 2).unwrap().next_id, made.last().unwrap().1);
        assert_eq!(curve(&doc, made[0].1).0.prev_id, 1);
        assert_eq!(curve(&doc, made.last().unwrap().1).0.next_id, 2);
    }

    /// From a spline's start the piece runs the other way from it: it is drawn mirrored, so it
    /// looks the same as the spline it goes on from.
    #[test]
    fn going_on_from_a_start_mirrors_the_piece() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 20.0, 0, 0);
        add_spline(&mut doc, (0, 0), 2, 130.0, 40.0, 0.0, 20.0, 0, 0);
        let a = end_of(&doc, (0, 0), 1, false).unwrap();
        let b = end_of(&doc, (0, 0), 2, true).unwrap();
        let made = lay(&mut doc, &a, &b).unwrap();
        check_joined(&doc, &made, &a, &b);
        assert!(curve(&doc, made[0].1).0.mirror);
        assert_eq!(doc.spline((0, 0), 1).unwrap().prev_id, made[0].1);
    }

    #[test]
    fn ends_that_meet_are_not_joined() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 20.0, 0, 0);
        add_spline(&mut doc, (0, 0), 2, 100.0, 120.0, 0.0, 20.0, 0, 0);
        let a = end_of(&doc, (0, 0), 1, true).unwrap();
        let b = end_of(&doc, (0, 0), 2, false).unwrap();
        assert!(lay(&mut doc, &a, &b).is_err());
    }
}
