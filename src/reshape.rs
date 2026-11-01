//! Reshaping a road after it is drawn: its end dragged somewhere else (a linked neighbour
//! following, tangent at the joint) or its middle pulled to bend it. The new shape is fitted
//! with the same arcs and biarcs roads are drawn with.

use glam::DVec2;
use omsi_geometry::SplineCurve;
use omsi_map::MapSpline;

use crate::doc::{tile_origin, Document, TileKey};
use crate::roads::{self, Seg};

/// A grip on a selected spline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Start,
    End,
    Mid,
}

/// One end of a stretch to fit: where it is and, when it must be kept, the heading the
/// road has there (the way it runs along the stretch).
#[derive(Debug, Clone, Copy)]
pub struct End {
    pub pos: DVec2,
    pub heading: Option<f64>,
}

fn angle_off(a: f64, b: f64) -> f64 {
    (roads::norm_deg(a - b + 180.0) - 180.0).abs()
}

/// The arcs from `a` to `b` keeping whichever headings are given.
pub fn fit(a: End, b: End) -> Option<Vec<Seg>> {
    match (a.heading, b.heading) {
        (Some(ha), Some(hb)) => {
            // (one arc when it happens to arrive right, else two)
            if let Some(s) = roads::arc_to(a.pos, ha, b.pos).filter(|s| angle_off(s.end_heading(), hb) < 0.5) {
                return Some(vec![s]);
            }
            roads::biarc(a.pos, ha, b.pos, hb)
        }
        (Some(ha), None) => roads::arc_to(a.pos, ha, b.pos).map(|s| vec![s]),
        (None, Some(hb)) => roads::arc_to(b.pos, roads::norm_deg(hb + 180.0), a.pos).map(|s| vec![roads::reverse(&s)]),
        (None, None) => roads::straight(a.pos, b.pos).map(|s| vec![s]),
    }
}

/// The one arc through three points (a straight when they lie on a line, in order).
pub fn arc_through(a: DVec2, m: DVec2, b: DVec2) -> Option<Seg> {
    let (u, v) = (m - a, b - a);
    let cross = u.perp_dot(v);
    if cross.abs() < 1e-6 * u.length() * v.length() {
        return (u.dot(v) > 0.0 && u.length() < v.length()).then(|| roads::straight(a, b)).flatten();
    }
    // the centre: equally far from all three
    let (uu, vv) = (u.length_squared(), v.length_squared());
    let c = a + DVec2::new(v.y * uu - u.y * vv, u.x * vv - v.x * uu) / (2.0 * cross);
    let r = a - c;
    let mut t = DVec2::new(-r.y, r.x);
    if t.dot(m - a) < 0.0 {
        t = -t;
    }
    roads::arc_to(a, roads::heading_of(t), b)
}

/// The shape from `a` to `b` through `m`: tangent to the headings given at the ends, and
/// smooth where it passes `m`.
pub fn bend(a: End, m: DVec2, b: End) -> Option<Vec<Seg>> {
    match (a.heading, b.heading) {
        (None, None) => arc_through(a.pos, m, b.pos).map(|s| vec![s]),
        (Some(_), _) => {
            let mut first = fit(a, End { pos: m, heading: None })?;
            let hm = first.last()?.end_heading();
            first.extend(fit(End { pos: m, heading: Some(hm) }, b)?);
            Some(first)
        }
        (None, Some(_)) => {
            let second = fit(End { pos: m, heading: None }, b)?;
            let hm = second.first()?.heading;
            let mut first = fit(a, End { pos: m, heading: Some(hm) })?;
            first.extend(second);
            Some(first)
        }
    }
}

/// A spline's new shape, in its own direction: its arcs, heights at its ends and the
/// gradients (%) to keep there (None: the even slope between the heights).
#[derive(Debug, Clone)]
pub struct Piece {
    pub key: TileKey,
    pub id: i64,
    pub segs: Vec<Seg>,
    pub z0: f64,
    pub z1: f64,
    pub g0: Option<f64>,
    pub g1: Option<f64>,
}

impl Piece {
    /// The same shape run the other way (a stretch fitted against the spline's direction).
    fn reversed(self) -> Piece {
        Piece {
            segs: self.segs.iter().rev().map(roads::reverse).collect(),
            z0: self.z1,
            z1: self.z0,
            g0: self.g1.map(|g| -g),
            g1: self.g0.map(|g| -g),
            ..self
        }
    }
}

/// What a reshape changes.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub pieces: Vec<Piece>,
}

/// Facts about a spline's two ends: positions with heights, headings the way it runs, and
/// gradients.
struct Ends {
    a: glam::DVec3,
    b: glam::DVec3,
    ha: f64,
    hb: f64,
    ga: f64,
    gb: f64,
}

fn ends_of(s: &MapSpline, k: TileKey) -> Ends {
    let c = SplineCurve::from_map(s, tile_origin(k));
    Ends { a: c.point_at(0.0), b: c.end_point(), ha: roads::norm_deg(s.heading), hb: roads::norm_deg(c.heading_at(s.length)), ga: c.slope_at(0.0) * 100.0, gb: c.slope_at(s.length) * 100.0 }
}

/// Drag a spline's end (`at_end`, else its start) to `p`. The other end keeps its place and
/// heading; a spline linked at the dragged end follows, tangent at the joint, its far end
/// kept. `ground` gives the ground's height (the end keeps its height above it), `locked`
/// says where a junction holds an end.
pub fn drag_end(doc: &Document, k: TileKey, id: i64, at_end: bool, p: DVec2, ground: &dyn Fn(DVec2) -> f64, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    let s = doc.spline(k, id).ok_or("gone")?;
    let e = ends_of(s, k);
    let old = if at_end { e.b } else { e.a };
    let link = if at_end { s.next_id } else { s.prev_id };
    if link == 0 && locked(old.truncate()) {
        return Err(crate::i18n::tr("This end belongs to a junction: select the junction and move it, the road follows", "Этот конец принадлежит перекрёстку: выберите перекрёсток и двигайте его, дорога последует"));
    }
    end_to(doc, k, id, at_end, p, None, ground(p) + (old.z - ground(old.truncate())), locked)
}

/// Drag a spline's end to `p` and have it arrive there heading `arrive` (the way along the road
/// towards that end): onto the end of a lane it is to join, tangent to it.
pub fn drag_end_aligned(doc: &Document, k: TileKey, id: i64, at_end: bool, p: DVec2, arrive: f64, ground: &dyn Fn(DVec2) -> f64, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    let s = doc.spline(k, id).ok_or("gone")?;
    let e = ends_of(s, k);
    let old = if at_end { e.b } else { e.a };
    carry_end(doc, k, id, at_end, p, arrive, ground(p) - ground(old.truncate()), locked)
}

/// Drag a spline's end to `p` and put it at the height `zp`: the place and height a face of an
/// object was met at under the cursor (a bridge's deck, a platform, a ramp), rather than the
/// ground's height plus the height the end kept above it.
pub fn drag_end_at(doc: &Document, k: TileKey, id: i64, at_end: bool, p: DVec2, zp: f64, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    end_to(doc, k, id, at_end, p, None, zp, locked)
}

/// Turn a spline's end (`at_end`, else its start) so the road runs heading `heading` there
/// (degrees, clockwise from north, the way the road itself runs). The end stays where it
/// is; the road bends to the new direction. A spline linked at that end follows, tangent
/// at the joint, its far end kept.
pub fn turn_end(doc: &Document, k: TileKey, id: i64, at_end: bool, heading: f64, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    let s = doc.spline(k, id).ok_or("gone")?;
    let e = ends_of(s, k);
    let (p, z, now, arrive) = if at_end { (e.b.truncate(), e.b.z, e.hb, roads::norm_deg(heading)) } else { (e.a.truncate(), e.a.z, e.ha, roads::norm_deg(heading + 180.0)) };
    if angle_off(now, heading) < 0.05 {
        return Ok(Plan::default());
    }
    end_to(doc, k, id, at_end, p, Some(arrive), z, locked)
}

/// Carry a road's end along with what it is joined to (a junction moved or turned): to
/// `p`, arriving there heading `arrive` (the way along the road towards that end), its height
/// changed by `dz`.
pub fn carry_end(doc: &Document, k: TileKey, id: i64, at_end: bool, p: DVec2, arrive: f64, dz: f64, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    let s = doc.spline(k, id).ok_or("gone")?;
    let e = ends_of(s, k);
    let old = if at_end { e.b } else { e.a };
    // (the height above the ground is not kept: the junction's plate sets it)
    end_to(doc, k, id, at_end, p, Some(arrive), old.z + dz, locked).or_else(|e| through_neighbour(doc, k, id, at_end, p, arrive, old.z + dz).ok_or(e))
}

/// A road too short to bend to where its end must go: it and the road before it are
/// shaped again as one, from that road's far end, the joint between them sliding along.
fn through_neighbour(doc: &Document, k: TileKey, id: i64, at_end: bool, p: DVec2, arrive: f64, zp: f64) -> Option<Plan> {
    let s = doc.spline(k, id)?;
    let link = if at_end { s.prev_id } else { s.next_id };
    let (nk, n) = doc.find_spline(link)?;
    let ne = ends_of(n, nk);
    let n_at_start = if n.prev_id == id { true } else if n.next_id == id { false } else { return None };
    // the stretch runs from the neighbour's far end, through the joint, to `p`
    let (g, hg, gg) = if n_at_start { (ne.b, roads::norm_deg(ne.hb + 180.0), -ne.gb) } else { (ne.a, ne.ha, ne.ga) };
    let segs = fit(End { pos: g.truncate(), heading: Some(hg) }, End { pos: p, heading: Some(arrive) })?;
    let total = roads::total_length(&segs);
    let at = total * n.length / (n.length + s.length).max(0.01);
    let (first, second) = cut(&segs, at);
    if first.is_empty() || second.is_empty() {
        return None;
    }
    let zj = g.z + (zp - g.z) * at / total.max(0.01);
    let gj = (zp - g.z) / total.max(0.01) * 100.0;
    let theirs = Piece { key: nk, id: n.id, segs: first, z0: g.z, z1: zj, g0: Some(gg), g1: Some(gj) };
    let mine = Piece { key: k, id, segs: second, z0: zj, z1: zp, g0: Some(gj), g1: Some(gj) };
    // (each the way it runs: the neighbour's own direction leads into the joint unless it
    // starts there; this one leaves the joint when its end is the one carried)
    Some(Plan { pieces: vec![if n_at_start { theirs.reversed() } else { theirs }, if at_end { mine } else { mine.reversed() }] })
}

/// Arcs cut in two `at` metres along them.
fn cut(segs: &[Seg], at: f64) -> (Vec<Seg>, Vec<Seg>) {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut acc = 0.0;
    for sg in segs {
        let (s0, s1) = (acc, acc + sg.length);
        if s1 <= at + 1e-9 {
            a.push(*sg);
        } else if s0 >= at - 1e-9 {
            b.push(*sg);
        } else {
            a.push(sg.piece(0.0, at - s0));
            b.push(sg.piece(at - s0, sg.length));
        }
        acc = s1;
    }
    (a, b)
}

/// The end moved to `p` at height `zp` (arriving with heading `arrive`, when given).
fn end_to(doc: &Document, k: TileKey, id: i64, at_end: bool, p: DVec2, arrive: Option<f64>, zp: f64, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    let s = doc.spline(k, id).ok_or("gone")?;
    let e = ends_of(s, k);
    // the stretch runs from the end that stays to the one dragged
    let (f, hf, gf, old) = if at_end { (e.a, e.ha, e.ga, e.b) } else { (e.b, roads::norm_deg(e.hb + 180.0), -e.gb, e.a) };
    let link = if at_end { s.next_id } else { s.prev_id };
    let too_sharp = crate::i18n::tr("That bend is too sharp", "Слишком крутой поворот");
    let segs = fit(End { pos: f.truncate(), heading: Some(hf) }, End { pos: p, heading: arrive }).ok_or(too_sharp)?;
    let hp = segs.last().unwrap().end_heading();
    let gp = (zp - f.z) / roads::total_length(&segs).max(0.1) * 100.0;
    let mine = Piece { key: k, id, segs, z0: f.z, z1: zp, g0: Some(gf), g1: Some(gp) };
    let mut plan = Plan { pieces: vec![if at_end { mine } else { mine.reversed() }] };
    if let Some((nk, n)) = doc.find_spline(link) {
        let ne = ends_of(n, nk);
        // (which of its ends is at the joint: the one linked back, else the nearer)
        let at_start = if n.prev_id == id { true } else if n.next_id == id { false } else { (ne.a - old).length() <= (ne.b - old).length() };
        let (g, hg, gg, far_link) = if at_start { (ne.b, ne.hb, ne.gb, n.next_id) } else { (ne.a, roads::norm_deg(ne.ha + 180.0), -ne.ga, n.prev_id) };
        let keep = far_link != 0 || locked(g.truncate());
        let segs = fit(End { pos: p, heading: Some(hp) }, End { pos: g.truncate(), heading: keep.then_some(hg) }).ok_or(too_sharp)?;
        let theirs = Piece { key: nk, id: n.id, segs, z0: zp, z1: g.z, g0: Some(gp), g1: keep.then_some(gg) };
        plan.pieces.push(if at_start { theirs } else { theirs.reversed() });
    }
    Ok(plan)
}

/// Pull a spline's middle to `m`: its ends stay; where they are joined to something their
/// headings stay too.
pub fn drag_mid(doc: &Document, k: TileKey, id: i64, m: DVec2, locked: &dyn Fn(DVec2) -> bool) -> Result<Plan, &'static str> {
    let s = doc.spline(k, id).ok_or("gone")?;
    let e = ends_of(s, k);
    let keep_a = s.prev_id != 0 || locked(e.a.truncate());
    let keep_b = s.next_id != 0 || locked(e.b.truncate());
    let segs = bend(End { pos: e.a.truncate(), heading: keep_a.then_some(e.ha) }, m, End { pos: e.b.truncate(), heading: keep_b.then_some(e.hb) })
        .ok_or(crate::i18n::tr("That bend is too sharp", "Слишком крутой поворот"))?;
    Ok(Plan { pieces: vec![Piece { key: k, id, segs, z0: e.a.z, z1: e.b.z, g0: keep_a.then_some(e.ga), g1: keep_b.then_some(e.gb) }] })
}

/// The splines a piece becomes (`template`: the spline it replaces), tile-local to `org`,
/// cut no longer than 50 m. IDCodes and links are the caller's.
fn piece_splines(template: &MapSpline, p: &Piece, org: DVec2) -> Vec<MapSpline> {
    let parts = roads::split(&p.segs, 50.0);
    let total = roads::total_length(&parts).max(0.01);
    let slope = (p.z1 - p.z0) / total * 100.0;
    let n = parts.len();
    let mut acc = 0.0;
    let mut out = Vec::new();
    for (i, sg) in parts.iter().enumerate() {
        let (f0, f1) = (acc / total, (acc + sg.length) / total);
        let za = p.z0 + (p.z1 - p.z0) * f0;
        let zb = p.z0 + (p.z1 - p.z0) * f1;
        let lerp = |a: f64, b: f64, f: f64| a + (b - a) * f;
        out.push(MapSpline {
            pos: [sg.start.x - org.x, sg.start.y - org.y, za],
            heading: roads::norm_deg(sg.heading),
            length: sg.length,
            radius: sg.radius,
            grad_start: if i == 0 { p.g0.unwrap_or(slope) } else { slope },
            grad_end: if i + 1 == n { p.g1.unwrap_or(slope) } else { slope },
            delta_h: Some(zb - za),
            is_h: true,
            cant_start: lerp(template.cant_start, template.cant_end, f0),
            cant_end: lerp(template.cant_start, template.cant_end, f1),
            skew_start: if i == 0 { template.skew_start } else { 0.0 },
            skew_end: if i + 1 == n { template.skew_end } else { 0.0 },
            tex_offset: template.tex_offset + acc,
            map_chain_offset: template.map_chain_offset.map(|o| o + acc),
            ..template.clone()
        });
        acc += sg.length;
    }
    out
}

/// What a plan looks like (world coordinates), to show while dragging.
pub fn preview(doc: &Document, plan: &Plan) -> Vec<MapSpline> {
    plan.pieces.iter().filter_map(|p| doc.spline(p.key, p.id).map(|s| piece_splines(s, p, DVec2::ZERO))).flatten().collect()
}

/// Put a plan into the document. A spline that became several keeps its IDCode on the
/// first; the rest get new ones and the chain is linked through them.
pub fn apply(doc: &mut Document, plan: &Plan) {
    for p in &plan.pieces {
        let org = tile_origin(p.key);
        let Some(old) = doc.spline(p.key, p.id).cloned() else { continue };
        let mut parts = piece_splines(&old, p, org);
        if parts.is_empty() {
            continue;
        }
        let ids: Vec<i64> = (0..parts.len()).map(|i| if i == 0 { old.id } else { doc.new_id() }).collect();
        for (i, s) in parts.iter_mut().enumerate() {
            s.id = ids[i];
            s.prev_id = if i == 0 { old.prev_id } else { ids[i - 1] };
            s.next_id = if i + 1 == ids.len() { old.next_id } else { ids[i + 1] };
        }
        let last = *ids.last().unwrap();
        let t = doc.tiles.get_mut(&p.key).unwrap();
        let idx = t.tile.splines.iter().position(|s| s.id == old.id && !s.deleted).unwrap();
        let mut parts = parts.into_iter();
        t.tile.splines[idx] = parts.next().unwrap();
        // (the others go at the end: attachment rows name splines by their place)
        t.tile.splines.extend(parts);
        t.touch_content();
        // the spline after it now follows the last piece
        if last != old.id && old.next_id != 0 {
            if let Some((nk, _)) = doc.find_spline(old.next_id) {
                let t = doc.tiles.get_mut(&nk).unwrap();
                for s in t.tile.splines.iter_mut().filter(|s| s.id == old.next_id) {
                    if s.prev_id == old.id {
                        s.prev_id = last;
                    } else if s.next_id == old.id {
                        s.next_id = last;
                    }
                }
                t.touch_content();
            }
        }
    }
}

/// The tiles a plan writes into.
pub fn tiles_of(doc: &Document, plan: &Plan) -> Vec<TileKey> {
    let mut v = Vec::new();
    for p in &plan.pieces {
        v.push(p.key);
        if let Some(s) = doc.spline(p.key, p.id) {
            for o in [s.prev_id, s.next_id] {
                if let Some((k, _)) = doc.find_spline(o) {
                    v.push(k);
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Set a spline's height (m) and gradient (%, the way it runs) at its end (`at_end`) or
/// start; the spline linked there is changed to match, so the joint stays smooth.
pub fn set_end(doc: &mut Document, k: TileKey, id: i64, at_end: bool, z: Option<f64>, g: Option<f64>) {
    let Some(s) = doc.spline(k, id).cloned() else { return };
    set_own_end(doc, k, id, at_end, z, g);
    let link = if at_end { s.next_id } else { s.prev_id };
    let Some((nk, n)) = doc.find_spline(link) else { return };
    let n_start = if n.prev_id == id { true } else if n.next_id == id { false } else { return };
    // (a neighbour running the same way climbs the same; one running back the opposite)
    let same = at_end == n_start;
    let nid = n.id;
    set_own_end(doc, nk, nid, !n_start, z, g.map(|g| if same { g } else { -g }));
}

/// The splines given, if they make one chain, in its order from one end, each with whether
/// it runs that way (None: they branch, leave gaps, or close a ring).
pub fn chain_order(doc: &Document, splines: &[(TileKey, i64)]) -> Option<Vec<(TileKey, i64, bool)>> {
    let set: std::collections::HashSet<i64> = splines.iter().map(|s| s.1).collect();
    let inside = |id: i64| id != 0 && set.contains(&id);
    // an end of the chain: a spline with at most one neighbour inside
    let (k0, first) = *splines.iter().find(|(k, id)| doc.spline(*k, *id).map(|s| !inside(s.prev_id) || !inside(s.next_id)).unwrap_or(false))?;
    let s0 = doc.spline(k0, first)?;
    // (it runs away from the chain's end when what follows it is inside)
    let mut out = vec![(k0, first, !inside(s0.prev_id))];
    let mut prev = first;
    let mut next = if inside(s0.next_id) && !inside(s0.prev_id) { s0.next_id } else if inside(s0.prev_id) && !inside(s0.next_id) { s0.prev_id } else { 0 };
    while next != 0 {
        if out.iter().any(|o| o.1 == next) {
            return None;
        }
        let (k, n) = doc.find_spline(next)?;
        let forward = if n.prev_id == prev { true } else if n.next_id == prev { false } else { return None };
        out.push((k, n.id, forward));
        let after = if forward { n.next_id } else { n.prev_id };
        prev = n.id;
        next = if inside(after) { after } else { 0 };
    }
    (out.len() == splines.len()).then_some(out)
}

/// Heights along a chain (`chain_order`) as one even slope from the height at its first
/// end to that at its last.
pub fn even_chain(doc: &mut Document, chain: &[(TileKey, i64, bool)]) {
    let ends: Vec<(f64, f64, f64)> = chain
        .iter()
        .filter_map(|(k, id, fwd)| {
            let s = doc.spline(*k, *id)?;
            let e = ends_of(s, *k);
            Some(if *fwd { (e.a.z, e.b.z, s.length) } else { (e.b.z, e.a.z, s.length) })
        })
        .collect();
    let (Some(first), Some(last)) = (ends.first(), ends.last()) else { return };
    let total: f64 = ends.iter().map(|e| e.2).sum::<f64>().max(0.1);
    let (z0, z1) = (first.0, last.1);
    let g = (z1 - z0) / total * 100.0;
    let mut at = 0.0;
    for ((k, id, fwd), e) in chain.iter().zip(&ends) {
        let (za, zb) = (z0 + (z1 - z0) * at / total, z0 + (z1 - z0) * (at + e.2) / total);
        at += e.2;
        let Some(t) = doc.tiles.get_mut(k) else { continue };
        let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == *id && !s.deleted) else { continue };
        // (its own way: one running back climbs the other way)
        let (zs, ze, gs) = if *fwd { (za, zb, g) } else { (zb, za, -g) };
        s.is_h = true;
        s.pos[2] = zs;
        s.delta_h = Some(ze - zs);
        s.grad_start = gs;
        s.grad_end = gs;
        t.touch_content();
    }
}

/// Set a spline's cant (%, positive: its right side lower) at its end or start; the spline
/// linked there takes the same banking at the joint.
pub fn set_cant(doc: &mut Document, k: TileKey, id: i64, at_end: bool, cant: f64) {
    let Some(s) = doc.spline(k, id).cloned() else { return };
    set_own_cant(doc, k, id, at_end, cant);
    let link = if at_end { s.next_id } else { s.prev_id };
    let Some((nk, n)) = doc.find_spline(link) else { return };
    let n_start = if n.prev_id == id { true } else if n.next_id == id { false } else { return };
    // (a neighbour running back has its right side on this one's left)
    let same = at_end == n_start;
    let nid = n.id;
    set_own_cant(doc, nk, nid, !n_start, if same { cant } else { -cant });
}

fn set_own_cant(doc: &mut Document, k: TileKey, id: i64, at_end: bool, cant: f64) {
    let Some(t) = doc.tiles.get_mut(&k) else { return };
    let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted) else { return };
    if at_end {
        s.cant_end = cant;
    } else {
        s.cant_start = cant;
    }
    t.touch_content();
}

fn set_own_end(doc: &mut Document, k: TileKey, id: i64, at_end: bool, z: Option<f64>, g: Option<f64>) {
    let Some(old) = doc.spline(k, id) else { return };
    let e = ends_of(old, k);
    let t = doc.tiles.get_mut(&k).unwrap();
    let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted) else { return };
    // (as a [spline_h] its height is fixed by the ends' heights and gradients: a plain
    // spline's parabola is the same cubic)
    s.is_h = true;
    let (mut z0, mut z1) = (e.a.z, e.b.z);
    if at_end {
        z1 = z.unwrap_or(z1);
        if let Some(g) = g {
            s.grad_end = g;
        }
    } else {
        z0 = z.unwrap_or(z0);
        if let Some(g) = g {
            s.grad_start = g;
        }
    }
    s.pos[2] = z0;
    s.delta_h = Some(z1 - z0);
    t.touch_content();
}

/// A spline turned round: it starts where it ended and runs back along the same curve
/// (heights, banking and the cut of its ends kept where they were; its links swapped).
/// `tex` is where its textures start along it now.
pub fn reversed(s: &MapSpline, org: DVec2, tex: f64) -> MapSpline {
    let c = omsi_geometry::SplineCurve::from_map(s, org);
    let end = c.end_point();
    let mut r = s.clone();
    r.pos = [end.x - org.x, end.y - org.y, end.z];
    r.heading = (c.heading_at(s.length) + 180.0).rem_euclid(360.0);
    r.radius = -s.radius;
    r.grad_start = -s.grad_end;
    r.grad_end = -s.grad_start;
    r.delta_h = s.delta_h.map(|d| -d);
    // (its right side is the old left: the banking changes sign; a skewed cut is the same
    // line seen the other way round)
    r.cant_start = -s.cant_end;
    r.cant_end = -s.cant_start;
    r.skew_start = s.skew_end;
    r.skew_end = s.skew_start;
    r.prev_id = s.next_id;
    r.next_id = s.prev_id;
    r.tex_offset = tex;
    r.map_chain_offset = s.map_chain_offset.map(|_| tex);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_spline, flat_map};

    fn curve(doc: &Document, id: i64) -> SplineCurve {
        let (k, s) = doc.find_spline(id).unwrap();
        SplineCurve::from_map(s, tile_origin(k))
    }

    #[test]
    fn a_chain_gets_one_even_slope() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 10.0, 10.0, 0.0, 50.0, 0, 2);
        // (the second runs back towards the first, the third on from it)
        add_spline(&mut doc, (0, 0), 2, 10.0, 110.0, 180.0, 50.0, 3, 1);
        add_spline(&mut doc, (0, 0), 3, 10.0, 110.0, 0.0, 100.0, 2, 0);
        set_own_end(&mut doc, (0, 0), 3, true, Some(20.0), None);
        let chain = chain_order(&doc, &[((0, 0), 2), ((0, 0), 3), ((0, 0), 1)]).unwrap();
        // (from either end)
        let order: Vec<(i64, bool)> = chain.iter().map(|c| (c.1, c.2)).collect();
        assert!(order == vec![(1, true), (2, false), (3, true)] || order == vec![(3, false), (2, true), (1, false)], "{order:?}");
        even_chain(&mut doc, &chain);
        // 20 m over 200 m: 10 %, 5 m up at the first joint, 10 m at the second
        let z = |id: i64, at: f64| curve(&doc, id).point_at(at).z;
        assert!((z(1, 50.0) - 5.0).abs() < 1e-6 && (z(2, 0.0) - 10.0).abs() < 1e-6 && (z(2, 50.0) - 5.0).abs() < 1e-6 && (z(3, 100.0) - 20.0).abs() < 1e-6);
        assert!((curve(&doc, 1).slope_at(25.0) - 0.1).abs() < 1e-6 && (curve(&doc, 2).slope_at(25.0) + 0.1).abs() < 1e-6);
        // a branch is no chain
        add_spline(&mut doc, (0, 0), 4, 10.0, 110.0, 90.0, 30.0, 2, 0);
        assert!(chain_order(&doc, &[((0, 0), 4), ((0, 0), 2), ((0, 0), 3)]).is_none());
    }

    #[test]
    fn a_cant_carries_over_the_joint() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 10.0, 10.0, 0.0, 50.0, 0, 2);
        // (the second runs back towards the first)
        add_spline(&mut doc, (0, 0), 2, 10.0, 110.0, 180.0, 50.0, 0, 1);
        set_cant(&mut doc, (0, 0), 1, true, 4.0);
        assert_eq!(doc.find_spline(1).unwrap().1.cant_end, 4.0);
        assert_eq!(doc.find_spline(2).unwrap().1.cant_end, -4.0);
    }

    #[test]
    fn an_arc_through_three_points_meets_them() {
        let (a, m, b) = (DVec2::new(0.0, 0.0), DVec2::new(20.0, 40.0), DVec2::new(0.0, 80.0));
        let s = arc_through(a, m, b).unwrap();
        assert!((s.end() - b).length() < 1e-6);
        let c = s.curve();
        let nearest = (0..=200).map(|i| (c.point_at(s.length * i as f64 / 200.0).truncate() - m).length()).fold(f64::MAX, f64::min);
        assert!(nearest < 0.5, "{nearest}");
        assert!(arc_through(a, DVec2::new(0.0, 40.0), b).unwrap().radius == 0.0);
    }

    #[test]
    fn a_bend_with_kept_headings_stays_tangent() {
        let a = End { pos: DVec2::new(0.0, 0.0), heading: Some(0.0) };
        let b = End { pos: DVec2::new(0.0, 100.0), heading: Some(0.0) };
        let segs = bend(a, DVec2::new(10.0, 50.0), b).unwrap();
        assert!((segs[0].heading - 0.0).abs() < 1e-9);
        assert!(angle_off(segs.last().unwrap().end_heading(), 0.0) < 2.0);
        assert!((segs.last().unwrap().end() - b.pos).length() < 1e-6);
        for w in segs.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-6);
            assert!(angle_off(w[0].end_heading(), w[1].heading) < 1e-6);
        }
    }

    /// A spline and the pieces it was cut into (new IDCodes, from 100 on in the tests), as
    /// one: its first and last curve and the last piece's IDCode.
    fn span(doc: &Document, id: i64) -> (SplineCurve, SplineCurve, i64) {
        let first = curve(doc, id);
        let mut cur = id;
        loop {
            let next = doc.find_spline(cur).unwrap().1.next_id;
            // (a piece follows on: linked back to this one)
            if next < 100 || doc.find_spline(next).unwrap().1.prev_id != cur {
                break;
            }
            cur = next;
        }
        (first, curve(doc, cur), cur)
    }

    #[test]
    fn a_free_end_goes_where_it_is_dragged() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 40.0, 0, 0);
        let plan = drag_end(&doc, (0, 0), 1, true, DVec2::new(110.0, 150.0), &|_| 0.0, &|_| false).unwrap();
        apply(&mut doc, &plan);
        let (a, b, _) = span(&doc, 1);
        assert!((b.end_point().truncate() - DVec2::new(110.0, 150.0)).length() < 1e-6);
        assert!((a.point_at(0.0).truncate() - DVec2::new(100.0, 100.0)).length() < 1e-6);
        assert!(a.heading_at(0.0).abs() < 1e-9, "the start keeps its heading");
    }

    #[test]
    fn a_dragged_start_moves_the_start() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 40.0, 0, 0);
        let plan = drag_end(&doc, (0, 0), 1, false, DVec2::new(95.0, 90.0), &|_| 0.0, &|_| false).unwrap();
        apply(&mut doc, &plan);
        let (a, b, _) = span(&doc, 1);
        assert!((a.point_at(0.0).truncate() - DVec2::new(95.0, 90.0)).length() < 1e-6);
        assert!((b.end_point().truncate() - DVec2::new(100.0, 140.0)).length() < 1e-6);
        assert!(angle_off(b.heading_at(b.length), 0.0) < 1e-6, "the end keeps its heading");
    }

    #[test]
    fn turning_an_end_keeps_it_put_and_the_next_road_follows() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 40.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 140.0, 0.0, 40.0, 1, 0);
        let plan = turn_end(&doc, (0, 0), 1, true, 20.0, &|_| false).unwrap();
        apply(&mut doc, &plan);
        let (a, b, _) = span(&doc, 1);
        assert!((a.point_at(0.0).truncate() - DVec2::new(100.0, 100.0)).length() < 1e-4, "the start stays");
        assert!(a.heading_at(0.0).abs() < 1.0, "the start keeps its direction");
        assert!((b.end_point().truncate() - DVec2::new(100.0, 140.0)).length() < 1e-3, "the end stays put");
        assert!(angle_off(b.heading_at(b.length), 20.0) < 1.0, "the end turns to the heading asked for");
        let n = curve(&doc, 2);
        assert!(angle_off(n.heading_at(0.0), 20.0) < 1.0, "the road joined there leaves along the same direction");
        assert!((n.end_point().truncate() - DVec2::new(100.0, 180.0)).length() < 1e-3, "that road's far end stays");
    }

    #[test]
    fn a_joint_dragged_keeps_both_splines_tangent_and_linked() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 50.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 100.0, 0.0, 50.0, 1, 3);
        add_spline(&mut doc, (0, 0), 3, 100.0, 150.0, 0.0, 50.0, 2, 0);
        let p = DVec2::new(108.0, 100.0);
        let plan = drag_end(&doc, (0, 0), 1, true, p, &|_| 0.0, &|_| false).unwrap();
        assert_eq!(plan.pieces.len(), 2);
        apply(&mut doc, &plan);
        let (_, c1, last1) = span(&doc, 1);
        assert!((c1.end_point().truncate() - p).length() < 1e-6);
        assert_eq!(doc.find_spline(last1).unwrap().1.next_id, 2);
        // spline 2 now starts there, along the same heading, and still reaches spline 3
        let (c2, cl, last2) = span(&doc, 2);
        assert_eq!(doc.spline((0, 0), 2).unwrap().prev_id, last1);
        assert!((c2.point_at(0.0).truncate() - p).length() < 1e-6);
        assert!(angle_off(c1.heading_at(c1.length), c2.heading_at(0.0)) < 1e-6);
        assert_ne!(last2, 2, "a biarc: two pieces at least");
        assert_eq!(doc.find_spline(last2).unwrap().1.next_id, 3);
        assert_eq!(doc.spline((0, 0), 3).unwrap().prev_id, last2);
        assert!((cl.end_point().truncate() - DVec2::new(100.0, 150.0)).length() < 1e-6);
        assert!(angle_off(cl.heading_at(cl.length), 0.0) < 2.0);
    }

    #[test]
    fn a_reversed_neighbour_follows_too() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 50.0, 0, 2);
        // spline 2 runs back south from (100, 150) to the joint at (100, 100): its end meets 1's
        add_spline(&mut doc, (0, 0), 2, 100.0, 150.0, 180.0, 50.0, 0, 1);
        let p = DVec2::new(104.0, 104.0);
        let plan = drag_end(&doc, (0, 0), 1, true, p, &|_| 0.0, &|_| false).unwrap();
        apply(&mut doc, &plan);
        let (_, c1, last1) = span(&doc, 1);
        let (c2a, c2, last2) = span(&doc, 2);
        assert!((c2.end_point().truncate() - p).length() < 1e-6);
        assert!((c2a.point_at(0.0).truncate() - DVec2::new(100.0, 150.0)).length() < 1e-6);
        assert!(angle_off(c1.heading_at(c1.length), c2.heading_at(c2.length) + 180.0) < 1e-6);
        assert_eq!(doc.find_spline(last2).unwrap().1.next_id, last1, "end to end, as before");
        assert_eq!(doc.find_spline(last1).unwrap().1.next_id, last2);
    }

    #[test]
    fn a_short_road_carried_far_bends_with_the_one_before_it() {
        let mut doc = flat_map(1);
        // 1 runs north 60 m into 2, 10 m long, whose end a junction holds
        add_spline(&mut doc, (0, 0), 1, 100.0, 30.0, 0.0, 60.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 90.0, 0.0, 10.0, 1, 0);
        // the junction goes 9 m east: 2 alone cannot bend that far and arrive heading north
        let p = DVec2::new(109.0, 100.0);
        let plan = carry_end(&doc, (0, 0), 2, true, p, 0.0, 0.0, &|_| false).unwrap();
        assert_eq!(plan.pieces.len(), 2);
        apply(&mut doc, &plan);
        let (_, c2, _) = span(&doc, 2);
        assert!((c2.end_point().truncate() - p).length() < 1e-6);
        assert!(angle_off(c2.heading_at(c2.length), 0.0) < 2.0);
        let (c1, _, _) = span(&doc, 1);
        assert!((c1.point_at(0.0).truncate() - DVec2::new(100.0, 30.0)).length() < 1e-6, "the far end stays");
        assert!(c1.heading_at(0.0).abs() < 1e-6);
    }

    #[test]
    fn a_junction_end_stays() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 40.0, 0, 0);
        assert!(drag_end(&doc, (0, 0), 1, true, DVec2::new(110.0, 150.0), &|_| 0.0, &|p| (p - DVec2::new(100.0, 140.0)).length() < 0.6).is_err());
    }

    #[test]
    fn heights_stay_above_the_ground_and_join() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 50.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 100.0, 0.0, 50.0, 1, 0);
        for s in &mut doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines {
            s.pos[2] = 1.0;
        }
        // the ground rises 2 m where the joint goes
        let ground = |p: DVec2| if p.x > 105.0 { 2.0 } else { 0.0 };
        let plan = drag_end(&doc, (0, 0), 1, true, DVec2::new(106.0, 100.0), &ground, &|_| false).unwrap();
        apply(&mut doc, &plan);
        let ((_, c1, _), (c2, _, _)) = (span(&doc, 1), span(&doc, 2));
        assert!((c1.end_point().z - 3.0).abs() < 1e-6, "{}", c1.end_point().z);
        assert!((c2.point_at(0.0).z - 3.0).abs() < 1e-6);
        assert!((c1.slope_at(c1.length) - c2.slope_at(0.0)).abs() < 1e-6, "smooth over the joint");
    }

    #[test]
    fn an_end_height_set_moves_the_neighbour_with_it() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 50.0, 0, 2);
        // spline 2 runs back from (100, 150) to the joint
        add_spline(&mut doc, (0, 0), 2, 100.0, 150.0, 180.0, 50.0, 0, 1);
        set_end(&mut doc, (0, 0), 1, true, Some(2.0), Some(4.0));
        let (c1, c2) = (curve(&doc, 1), curve(&doc, 2));
        assert!((c1.end_point().z - 2.0).abs() < 1e-9 && c1.point_at(0.0).z.abs() < 1e-9);
        assert!((c2.end_point().z - 2.0).abs() < 1e-9 && c2.point_at(0.0).z.abs() < 1e-9);
        assert!((c1.slope_at(c1.length) - 0.04).abs() < 1e-9);
        assert!((c2.slope_at(c2.length) + 0.04).abs() < 1e-9, "running back it falls");
        set_end(&mut doc, (0, 0), 1, false, Some(1.0), None);
        let c1 = curve(&doc, 1);
        assert!((c1.point_at(0.0).z - 1.0).abs() < 1e-9 && (c1.end_point().z - 2.0).abs() < 1e-9);
    }

    #[test]
    fn the_middle_pulled_bends_a_free_spline() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 60.0, 0, 0);
        let plan = drag_mid(&doc, (0, 0), 1, DVec2::new(110.0, 130.0), &|_| false).unwrap();
        apply(&mut doc, &plan);
        let (a, c, _) = span(&doc, 1);
        assert!((c.end_point().truncate() - DVec2::new(100.0, 160.0)).length() < 1e-6);
        // (out to the right first, then round to the end: through the point pulled to)
        assert!(a.heading_at(0.0) > 10.0 && a.heading_at(0.0) < 90.0);
        assert!((a.end_point().truncate() - DVec2::new(110.0, 130.0)).length() < 1.0, "{:?}", a.end_point());
    }

    #[test]
    fn a_reversed_spline_runs_back_along_the_same_curve() {
        let s = MapSpline { id: 7, prev_id: 3, next_id: 9, pos: [10.0, 20.0, 5.0], heading: 30.0, length: 80.0, radius: 120.0, grad_start: 2.0, grad_end: -1.0, delta_h: Some(1.5), is_h: true, cant_start: 3.0, cant_end: 1.0, skew_start: 10.0, ..Default::default() };
        let org = DVec2::new(300.0, 0.0);
        let r = reversed(&s, org, 0.0);
        let (a, b) = (SplineCurve::from_map(&s, org), SplineCurve::from_map(&r, org));
        for i in 0..=8 {
            let d = s.length * i as f64 / 8.0;
            assert!((a.point_at(d) - b.point_at(s.length - d)).length() < 1e-6, "at {d}");
            assert!((a.cant_at(d) + b.cant_at(s.length - d)).abs() < 1e-9);
        }
        assert_eq!((r.prev_id, r.next_id), (9, 3));
        let back = reversed(&r, org, s.tex_offset);
        assert!((back.heading - s.heading).abs() < 1e-9 && (back.pos[0] - s.pos[0]).abs() < 1e-9 && (back.pos[1] - s.pos[1]).abs() < 1e-9);
        assert_eq!((back.radius, back.grad_start, back.cant_end, back.skew_start), (s.radius, s.grad_start, s.cant_end, s.skew_start));
    }
}
