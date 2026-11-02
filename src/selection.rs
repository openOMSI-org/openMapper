//! What several selected things do together: move, turn and delete as one, found by a box
//! drawn on the screen. Plain functions on the document, so they can be tested without a
//! window.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3, Vec4};

use crate::doc::{tile_of, tile_origin, Document, Sel, TileKey};
use crate::history::History;
use crate::roads;

/// Where a selected thing stood when a move or turn began: an object's position, a
/// spline's start (world) and heading.
#[derive(Debug, Clone, Copy)]
pub struct Origin {
    pub sel: Sel,
    pub pos: DVec2,
    pub heading: f64,
}

/// The world position and heading of what `s` names (None: it is gone).
pub fn origin_of(doc: &Document, s: Sel) -> Option<Origin> {
    match s {
        Sel::Object(k, id) => {
            let o = doc.object(k, id)?;
            let org = tile_origin(k);
            Some(Origin { sel: s, pos: DVec2::new(org.x + o.pos[0], org.y + o.pos[1]), heading: o.rot[0] })
        }
        Sel::Spline(k, id) => {
            let sp = doc.spline(k, id)?;
            let org = tile_origin(k);
            Some(Origin { sel: s, pos: DVec2::new(org.x + sp.pos[0], org.y + sp.pos[1]), heading: sp.heading })
        }
        Sel::Attached(k, id) => {
            let a = doc.attachment(k, id)?;
            let first = attachment_first(doc, k, id)?;
            Some(Origin { sel: s, pos: first.pose.pos.truncate(), heading: a.rot[0] })
        }
    }
}

/// A `[splineAttachement]` record of a tile, by its IDCode.
pub fn attachment(doc: &Document, k: TileKey, id: i64) -> Option<&omsi_map::SplineAttachment> {
    doc.attachment(k, id)
}

/// The first copy a record places (None: its road is gone, or it stands past the end).
pub fn attachment_first(doc: &Document, k: TileKey, id: i64) -> Option<crate::place::RowObject> {
    let t = doc.tiles.get(&k)?;
    let a = t.tile.spline_attachments.iter().find(|a| a.id == id)?;
    crate::place::row_objects(a, &t.tile.splines, tile_origin(k)).into_iter().min_by_key(|o| o.index)
}

/// Every copy a record places (where it is drawn, for picking).
pub fn attachment_copies(doc: &Document, k: TileKey, id: i64) -> Vec<crate::place::RowObject> {
    let Some(t) = doc.tiles.get(&k) else { return Vec::new() };
    let Some(a) = t.tile.spline_attachments.iter().find(|a| a.id == id) else { return Vec::new() };
    crate::place::row_objects(a, &t.tile.splines, tile_origin(k))
}

/// The record as a map object, standing where its first copy does, so the properties panel can
/// treat it like any other object: `(tile, IDCode, the object, where it is on its road)`.
pub fn attachment_view(doc: &Document, k: TileKey, id: i64) -> Option<(TileKey, i64, omsi_map::MapObject, AttInfo)> {
    let a = attachment(doc, k, id)?;
    let first = attachment_first(doc, k, id)?;
    let org = tile_origin(k);
    let p = first.pose.pos;
    let o = omsi_map::MapObject {
        file: a.file.clone(),
        id: a.id,
        // (kept relative to the tile, like any object: the height above the ground there)
        pos: [p.x - org.x, p.y - org.y, p.z - doc.ground(p.truncate())],
        rot: [roads::norm_deg(a.rot[0] + road_heading(doc, k, a)), a.rot[1], a.rot[2]],
        flag: a.strings.len() as i32,
        extra: a.strings.clone(),
        var_parent: a.var_parent,
        rules: a.rules.clone(),
        ..Default::default()
    };
    // (where it lies on its road: distance along the chain, beside the centre line, height)
    let along = a.offset[2];
    Some((k, id, o, AttInfo { along, beside: a.offset[0], height: a.offset[1], interval: a.interval, range: a.range, tilt: a.tilt }))
}

/// The heading of a record's road where its first copy stands (for the object view's rotation).
pub fn attachment_world_rot(doc: &Document, k: TileKey, a: &omsi_map::SplineAttachment) -> f64 {
    roads::norm_deg(a.rot[0] + road_heading(doc, k, a))
}

/// The heading of the road a record lies on, where its first copy stands (0: the road is gone).
fn road_heading(doc: &Document, k: TileKey, a: &omsi_map::SplineAttachment) -> f64 {
    let Some(t) = doc.tiles.get(&k) else { return 0.0 };
    let Some(s) = t.tile.splines.get(a.spline_index.max(0) as usize).filter(|s| !s.deleted) else { return 0.0 };
    let curve = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
    let u = (a.offset[2] - s.map_chain_offset.unwrap_or(0.0)).clamp(0.0, s.length.max(0.0));
    curve.heading_at(u)
}

/// Where an attached object lies on its road: distance along the chain, beside the centre
/// line (right positive), and height above the road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttInfo {
    pub along: f64,
    pub beside: f64,
    pub height: f64,
    pub interval: f64,
    pub range: f64,
    pub tilt: bool,
}

impl AttInfo {
    /// How many copies the record places (1 when it is a single object).
    pub fn copies(&self) -> usize {
        if self.interval <= 1e-6 || self.range < self.interval {
            1
        } else {
            (self.range / self.interval + 1e-6).floor() as usize + 1
        }
    }
}

/// The point on a spline nearest `p`, as `(distance along it from its start, lateral offset to
/// the right)`. None: the spline has no length.
pub fn on_spline(s: &omsi_map::MapSpline, origin: DVec2, p: DVec2) -> Option<(f64, f64)> {
    let len = s.length.max(0.0);
    if len <= 1e-9 {
        return None;
    }
    let c = omsi_geometry::SplineCurve::from_map(s, origin);
    let n = (len.ceil() as usize).clamp(2, 2000);
    let mut best = (f64::INFINITY, 0.0f64);
    for i in 0..=n {
        let at = len * i as f64 / n as f64;
        let d = (c.point_at(at).truncate() - p).length();
        if d < best.0 {
            best = (d, at);
        }
    }
    // (a second pass between the two nearest samples, so a long piece is not out by a millimetre)
    let step = len / n as f64;
    let (mut lo, mut hi) = ((best.1 - step).max(0.0), (best.1 + step).min(len));
    for _ in 0..40 {
        let a = lo + (hi - lo) / 3.0;
        let b = hi - (hi - lo) / 3.0;
        let da = (c.point_at(a).truncate() - p).length();
        let db = (c.point_at(b).truncate() - p).length();
        if da <= db {
            hi = b;
        } else {
            lo = a;
        }
    }
    let at = (lo + hi) * 0.5;
    let q = c.point_at(at).truncate();
    let h = c.heading_at(at);
    let right = omsi_geometry::SplineCurve::dir(h);
    let right = DVec2::new(right.y, -right.x);
    Some((at, (p - q).dot(right)))
}

/// Put a record where a world point lies on its own road: its `along` (from the start of the
/// chain) and `beside` follow the point, its height is left alone. False: the road is gone.
pub fn move_attachment(doc: &mut Document, k: TileKey, id: i64, to: DVec2) -> bool {
    let Some(t) = doc.tiles.get(&k) else { return false };
    let Some(a) = t.tile.spline_attachments.iter().find(|a| a.id == id) else { return false };
    let Some(s) = t.tile.splines.get(a.spline_index.max(0) as usize).filter(|s| !s.deleted) else { return false };
    let Some((along, beside)) = on_spline(s, tile_origin(k), to) else { return false };
    let chain = s.map_chain_offset.unwrap_or(0.0);
    let (along, beside) = (chain + along, beside);
    let t = doc.tiles.get_mut(&k).unwrap();
    if let Some(a) = t.tile.spline_attachments.iter_mut().find(|a| a.id == id) {
        a.offset[0] = beside;
        a.offset[2] = along;
    }
    t.touch_content();
    true
}

/// The middle of a selection (objects' positions, splines' midpoints).
pub fn centre(doc: &Document, sels: &[Sel]) -> Option<DVec2> {
    let pts: Vec<DVec2> = sels
        .iter()
        .filter_map(|s| match *s {
            Sel::Object(..) | Sel::Attached(..) => origin_of(doc, *s).map(|o| o.pos),
            Sel::Spline(k, id) => doc.spline(k, id).map(|sp| omsi_geometry::SplineCurve::from_map(sp, tile_origin(k)).point_at(sp.length * 0.5).truncate()),
        })
        .collect();
    (!pts.is_empty()).then(|| pts.iter().sum::<DVec2>() / pts.len() as f64)
}

/// Turn a point round `c` clockwise by `deg` (headings turn clockwise too).
pub fn turn_point(p: DVec2, c: DVec2, deg: f64) -> DVec2 {
    let (s, co) = deg.to_radians().sin_cos();
    let d = p - c;
    c + DVec2::new(d.x * co + d.y * s, -d.x * s + d.y * co)
}

/// Keep the tiles a spline and its linked neighbours are in, before an edit changes them.
pub fn touch_spline(hist: &mut History, doc: &Document, k: TileKey, id: i64) {
    hist.touch_tile(doc, k);
    if let Some(s) = doc.spline(k, id) {
        for other in [s.prev_id, s.next_id] {
            if let Some((nk, _)) = doc.find_spline(other) {
                hist.touch_tile(doc, nk);
            }
        }
    }
}

/// Where the things of a move are in their tiles, read once: a move of thousands walks each
/// tile it touches a single time, instead of walking one for every piece it carries (which is
/// what a grab answered with seconds a frame on a map of a hundred thousand splines).
struct Places {
    splines: std::collections::HashMap<TileKey, std::collections::HashMap<i64, usize>>,
    objects: std::collections::HashMap<TileKey, std::collections::HashMap<i64, usize>>,
}

/// A tile with fewer things than this is walked: a place read for it would cost more than
/// the walk (a move of one thing is never slower for this).
const PLACES_SMALL: usize = 32;

impl Places {
    fn new() -> Self {
        Places { splines: std::collections::HashMap::new(), objects: std::collections::HashMap::new() }
    }

    /// Where the spline `id` is in tile `k` (None: it is not there).
    fn spline(&mut self, doc: &Document, k: TileKey, id: i64) -> Option<usize> {
        let t = doc.tiles.get(&k)?;
        if t.tile.splines.len() <= PLACES_SMALL {
            return t.tile.splines.iter().position(|s| s.id == id && !s.deleted);
        }
        let m = self.splines.entry(k).or_insert_with(|| {
            let mut m = std::collections::HashMap::with_capacity(t.tile.splines.len());
            for (i, s) in t.tile.splines.iter().enumerate() {
                if !s.deleted {
                    m.insert(s.id, i);
                }
            }
            m
        });
        m.get(&id).copied()
    }

    /// Where the object `id` is in tile `k` (None: it is not there).
    fn object(&mut self, doc: &Document, k: TileKey, id: i64) -> Option<usize> {
        let t = doc.tiles.get(&k)?;
        if t.tile.objects.len() <= PLACES_SMALL {
            return t.tile.objects.iter().position(|o| o.id == id);
        }
        let m = self.objects.entry(k).or_insert_with(|| {
            let mut m = std::collections::HashMap::with_capacity(t.tile.objects.len());
            for (i, o) in t.tile.objects.iter().enumerate() {
                m.insert(o.id, i);
            }
            m
        });
        m.get(&id).copied()
    }

    /// A tile changed shape (something was taken out of it or put in): read it again.
    fn forget(&mut self, k: TileKey) {
        self.splines.remove(&k);
        self.objects.remove(&k);
    }
}

/// Where every spline is and what it is linked to, read once: moving a chain of thousands
/// would else look each link up across the whole map, twice for every piece of it.
pub struct SplineWhere {
    map: std::collections::HashMap<i64, (TileKey, i64, i64)>,
}

impl SplineWhere {
    pub fn of(doc: &Document) -> Self {
        let mut map = std::collections::HashMap::new();
        for (k, t) in &doc.tiles {
            for s in &t.tile.splines {
                if !s.deleted {
                    map.insert(s.id, (*k, s.prev_id, s.next_id));
                }
            }
        }
        SplineWhere { map }
    }
}

/// `touch_spline` with the links read from `w` (the same tiles kept, a move of many paid for
/// once).
pub fn touch_spline_where(hist: &mut History, doc: &Document, w: &SplineWhere, k: TileKey, id: i64) {
    hist.touch_tile(doc, k);
    if let Some((_, prev, next)) = w.map.get(&id) {
        for other in [*prev, *next] {
            if let Some((nk, _, _)) = w.map.get(&other) {
                hist.touch_tile(doc, *nk);
            }
        }
    }
}

/// Put an object at a world position, into the file of the tile it is now over (None: no
/// tile there). Returns where it is now.
pub fn place_object_at(doc: &mut Document, k: TileKey, id: i64, to: DVec2) -> Option<Sel> {
    let nk = tile_of(to);
    if !doc.tiles.contains_key(&nk) {
        return None;
    }
    let org = tile_origin(nk);
    if nk == k {
        let o = doc.object_mut(k, id)?;
        o.pos[0] = to.x - org.x;
        o.pos[1] = to.y - org.y;
        doc.tiles.get_mut(&k)?.touch_content();
    } else {
        // (what hangs on it goes along: a hanging object's parent is looked for in its own
        // tile)
        let t = doc.tiles.get_mut(&k)?;
        let i = t.tile.objects.iter().position(|x| x.id == id)?;
        let mut o = t.tile.objects.remove(i);
        let mut kids = vec![id];
        let mut j = 0;
        while j < kids.len() {
            let p = kids[j];
            kids.extend(t.tile.attach_objects.iter().filter(|a| a.parent_id == Some(p)).map(|a| a.id));
            j += 1;
        }
        let (hanging, stay): (Vec<_>, Vec<_>) = std::mem::take(&mut t.tile.attach_objects).into_iter().partition(|a| kids[1..].contains(&a.id));
        t.tile.attach_objects = stay;
        t.touch_content();
        o.pos[0] = to.x - org.x;
        o.pos[1] = to.y - org.y;
        let t = doc.tiles.get_mut(&nk)?;
        t.tile.objects.push(o);
        t.tile.attach_objects.extend(hanging);
        t.touch_content();
    }
    Some(Sel::Object(nk, id))
}

/// Set a spline's start (world) and heading; it stays in its tile's file (spline
/// attachments name it by its place there).
pub fn place_spline_at(doc: &mut Document, k: TileKey, id: i64, to: DVec2, heading: f64) -> Option<Sel> {
    let org = tile_origin(k);
    let t = doc.tiles.get_mut(&k)?;
    let s = t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted)?;
    s.pos[0] = to.x - org.x;
    s.pos[1] = to.y - org.y;
    s.heading = roads::norm_deg(heading);
    t.touch_content();
    Some(Sel::Spline(k, id))
}

/// Move and turn a selection from where it stood (`from`): every position turned by `deg`
/// round `pivot`, then shifted by `delta`. History keeps every tile touched. Returns the
/// selection as it is now (objects may have gone over into another tile).
pub fn transform(doc: &mut Document, hist: &mut History, from: &[Origin], pivot: DVec2, deg: f64, delta: DVec2) -> Vec<Sel> {
    transform_known(doc, hist, from, pivot, deg, delta, None)
}

/// `transform`, with what a spline is linked to already read (a grab reads it once, when it
/// takes hold, and every frame of it then moves the chain without reading the map again).
pub fn transform_known(doc: &mut Document, hist: &mut History, from: &[Origin], pivot: DVec2, deg: f64, delta: DVec2, link: Option<&SplineWhere>) -> Vec<Sel> {
    let mut out = Vec::new();
    // (a chain of thousands would else have every link of it looked up across the whole map;
    // a spline keeps the tile it is in, so one reading of them serves the whole move)
    let own = (link.is_none() && from.len() > 16).then(|| SplineWhere::of(doc));
    let where_ = link.or(own.as_ref());
    // (and every piece of it looked up in its own tile: the tile is read once)
    let mut places = (from.len() > 16).then(Places::new);
    for o in from {
        let to = turn_point(o.pos, pivot, deg) + delta;
        let h = o.heading + deg;
        match o.sel {
            Sel::Object(k, id) => {
                // (the object may have gone over already: it is looked for where it is now)
                let here = places.as_mut().and_then(|p| p.object(doc, k, id));
                let Some(cur) = (if here.is_some() { Some(k) } else { find_object(doc, k, id) }) else { continue };
                hist.touch_tile(doc, cur);
                let nk = tile_of(to);
                if doc.tiles.contains_key(&nk) {
                    hist.touch_tile(doc, nk);
                }
                // (still over the tile it was in: it is set where it stands, the tile not walked
                // for it - a move of thousands would pay for that walk every frame)
                let at = match (nk == cur, here) {
                    (true, Some(i)) => Some(i),
                    (true, None) => places.as_mut().and_then(|p| p.object(doc, cur, id)),
                    _ => None,
                };
                if let Some(i) = at {
                    if let Some(t) = doc.tiles.get_mut(&cur) {
                        let org = tile_origin(cur);
                        if let Some(ob) = t.tile.objects.get_mut(i) {
                            ob.rot[0] = roads::norm_deg(h);
                            ob.pos[0] = to.x - org.x;
                            ob.pos[1] = to.y - org.y;
                        }
                        t.touch_content();
                    }
                    out.push(Sel::Object(cur, id));
                    continue;
                }
                if let Some(ob) = doc.object_mut(cur, id) {
                    ob.rot[0] = roads::norm_deg(h);
                }
                out.push(place_object_at(doc, cur, id, to).unwrap_or(Sel::Object(cur, id)));
                // (it is in another tile now: the two read wrong)
                if let Some(p) = places.as_mut() {
                    p.forget(cur);
                    p.forget(nk);
                }
            }
            Sel::Spline(k, id) => {
                match where_ {
                    Some(w) => touch_spline_where(hist, doc, w, k, id),
                    None => touch_spline(hist, doc, k, id),
                }
                // (a spline stays in its tile's file: set where it is, the tile not walked)
                let at = places.as_mut().and_then(|p| p.spline(doc, k, id));
                let placed = at.and_then(|i| {
                    let org = tile_origin(k);
                    let t = doc.tiles.get_mut(&k)?;
                    let s = t.tile.splines.get_mut(i)?;
                    s.pos[0] = to.x - org.x;
                    s.pos[1] = to.y - org.y;
                    s.heading = roads::norm_deg(h);
                    t.touch_content();
                    Some(Sel::Spline(k, id))
                });
                if let Some(s) = placed {
                    out.push(s);
                } else if let Some(s) = place_spline_at(doc, k, id, to, h) {
                    out.push(s);
                }
            }
            Sel::Attached(k, id) => {
                // (it stays on its road: only its place along it and beside it change)
                hist.touch_tile(doc, k);
                if let Some(a) = doc.attachment_mut(k, id) {
                    a.rot[0] = roads::norm_deg(h);
                }
                if move_attachment(doc, k, id, to) {
                    out.push(Sel::Attached(k, id));
                }
            }
        }
    }
    hist.touch_global(doc);
    crate::editor::sync_entry_points(doc);
    out
}

/// An object by IDCode, in tile `k` or, having moved, in any other.
fn find_object(doc: &Document, k: TileKey, id: i64) -> Option<TileKey> {
    if doc.object(k, id).is_some() {
        return Some(k);
    }
    doc.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == id)).map(|(k, _)| *k)
}

/// After a move: the links from the selection's splines to splines outside it no longer
/// meet, so both sides let go of each other.
pub fn unlink_outside(doc: &mut Document, hist: &mut History, sels: &[Sel]) {
    let ids: BTreeSet<i64> = sels.iter().filter_map(|s| if let Sel::Spline(_, id) = s { Some(*id) } else { None }).collect();
    let mut cuts: Vec<(i64, i64)> = Vec::new();
    for s in sels {
        let Sel::Spline(k, id) = *s else { continue };
        let Some(sp) = doc.spline(k, id) else { continue };
        for other in [sp.prev_id, sp.next_id] {
            if other != 0 && !ids.contains(&other) {
                cuts.push((id, other));
            }
        }
    }
    for (a, b) in cuts {
        for (x, y) in [(a, b), (b, a)] {
            let Some((k, _)) = doc.find_spline(x) else { continue };
            hist.touch_tile(doc, k);
            let t = doc.tiles.get_mut(&k).unwrap();
            for s in t.tile.splines.iter_mut().filter(|s| s.id == x) {
                if s.prev_id == y {
                    s.prev_id = 0;
                }
                if s.next_id == y {
                    s.next_id = 0;
                }
            }
            t.touch_content();
        }
    }
}

/// Delete everything selected.
pub fn delete(doc: &mut Document, hist: &mut History, sels: &[Sel]) {
    hist.touch_global(doc);
    for s in sels {
        match *s {
            Sel::Object(k, id) => {
                hist.touch_tile(doc, k);
                if doc.object(k, id).is_some() {
                    doc.delete_object(k, id);
                } else {
                    delete_attached(doc, k, id);
                }
            }
            Sel::Spline(k, id) => {
                touch_spline(hist, doc, k, id);
                doc.delete_spline(k, id);
            }
            Sel::Attached(k, id) => {
                hist.touch_tile(doc, k);
                doc.delete_attachment(k, id);
            }
        }
    }
}

/// A road end held by a junction (or any object with `[splinehelper]` points): where it
/// is and the heading out of the junction there.
#[derive(Debug, Clone, Copy)]
pub struct Arm {
    pub pos: DVec3,
    pub out: f64,
}

/// The world points of an object's spline helpers (`helpers`: x, y, z, heading out in its
/// frame), with their headings out.
pub fn helper_points(doc: &Document, k: TileKey, id: i64, abs: bool, helpers: &[[f32; 4]]) -> Vec<(DVec3, f64)> {
    let Some(o) = doc.object(k, id) else { return Vec::new() };
    let world = doc.object_world(k, o, abs);
    let m = crate::render::object_matrix(DVec3::ZERO, o.rot);
    helpers.iter().map(|h| (world + m.transform_point3(glam::Vec3::new(h[0], h[1], h[2])).as_dvec3(), roads::norm_deg(h[3] as f64 + o.rot[0]))).collect()
}

/// The free spline end (not linked on) nearest `p` within 0.6 m: (tile, spline, at its end).
pub fn free_end_at(doc: &Document, p: DVec2) -> Option<(TileKey, i64, bool)> {
    let mut best: Option<(f64, (TileKey, i64, bool))> = None;
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for s in t.tile.splines.iter().filter(|s| !s.deleted && (s.prev_id == 0 || s.next_id == 0)) {
            let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
            if (start - p).length() > s.length + 1.0 {
                continue;
            }
            let c = omsi_geometry::SplineCurve::from_map(s, org);
            for (free, q, at_end) in [(s.prev_id == 0, c.point_at(0.0), false), (s.next_id == 0, c.end_point(), true)] {
                let d = (q.truncate() - p).length();
                if free && d < 0.6 && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, (*k, s.id, at_end)));
                }
            }
        }
    }
    best.map(|b| b.1)
}

/// The road ends an object holds: those lying at its helper points.
pub fn arms_of(doc: &Document, points: &[(DVec3, f64)]) -> Vec<Arm> {
    let mut out: Vec<(i64, bool, Arm, f64)> = Vec::new();
    for (p, h) in points {
        let Some((k, id, at_end)) = free_end_at(doc, p.truncate()) else { continue };
        let Some(s) = doc.spline(k, id) else { continue };
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
        // (a helper is written for each way along a road: the one pointing the way the road
        // leaves the junction is kept)
        let away = if at_end { c.heading_at(s.length) + 180.0 } else { c.heading_at(0.0) };
        let miss = (roads::norm_deg(away - h + 180.0) - 180.0).abs();
        let arm = Arm { pos: if at_end { c.end_point() } else { c.point_at(0.0) }, out: *h };
        match out.iter_mut().find(|o| o.0 == id && o.1 == at_end) {
            Some(o) if miss < o.3 => {
                o.2 = arm;
                o.3 = miss;
            }
            Some(_) => {}
            None => out.push((id, at_end, arm, miss)),
        }
    }
    out.into_iter().map(|o| o.2).collect()
}

/// The plans that carry arms along when what holds them is turned by `deg` round `pivot`,
/// shifted by `delta` and raised by `dz`; arms that cannot follow give their reason.
pub fn carry_plans(doc: &Document, arms: &[Arm], pivot: DVec2, deg: f64, delta: DVec2, dz: f64, locked: &dyn Fn(DVec2) -> bool) -> Vec<Result<crate::reshape::Plan, &'static str>> {
    arms.iter()
        .filter_map(|a| {
            let (k, id, at_end) = free_end_at(doc, a.pos.truncate())?;
            let to = turn_point(a.pos.truncate(), pivot, deg) + delta;
            if (to - a.pos.truncate()).length() < 1e-6 && deg.abs() < 1e-9 && dz.abs() < 1e-9 {
                return None;
            }
            // (along the road towards the junction: against the way out of it)
            Some(crate::reshape::carry_end(doc, k, id, at_end, to, roads::norm_deg(a.out + deg + 180.0), dz, locked))
        })
        .collect()
}

/// An object of a tile, or one hanging on another (`[attachObj]`).
pub fn any_object(doc: &Document, k: TileKey, id: i64) -> Option<&omsi_map::MapObject> {
    doc.object(k, id).or_else(|| doc.tiles.get(&k)?.tile.attach_objects.iter().find(|o| o.id == id))
}

pub fn any_object_mut(doc: &mut Document, k: TileKey, id: i64) -> Option<&mut omsi_map::MapObject> {
    let t = doc.tiles.get_mut(&k)?;
    if t.tile.objects.iter().any(|o| o.id == id) {
        return t.tile.objects.iter_mut().find(|o| o.id == id);
    }
    t.tile.attach_objects.iter_mut().find(|o| o.id == id)
}

/// An object anywhere on the map by its IDCode, with its tile.
pub fn object_by_id(doc: &Document, id: i64) -> Option<(TileKey, &omsi_map::MapObject)> {
    doc.tiles.iter().find_map(|(k, t)| t.tile.objects.iter().chain(&t.tile.attach_objects).find(|o| o.id == id).map(|o| (*k, o)))
}

/// The file, the object it takes its state from, and its labels: an object, one hanging on
/// another, or a spline attachment.
pub fn state_holder(doc: &Document, k: TileKey, id: i64) -> Option<(String, Option<i64>, Vec<String>)> {
    if let Some(o) = any_object(doc, k, id) {
        return Some((o.file.clone(), o.var_parent, o.extra.clone()));
    }
    let a = doc.attachment(k, id)?;
    Some((a.file.clone(), a.var_parent, a.strings.clone()))
}

/// The object an object takes its state from (`[varparent]`). A spline attachment is an
/// object too: a signal head laid along a road takes its light the same way.
pub fn var_parent_of(doc: &Document, id: i64) -> Option<i64> {
    if let Some((_, o)) = object_by_id(doc, id) {
        return o.var_parent;
    }
    doc.tiles.values().find_map(|t| t.tile.spline_attachments.iter().find(|a| a.id == id).and_then(|a| a.var_parent))
}

/// The file of an object or a spline attachment, by its IDCode.
pub fn file_by_id(doc: &Document, id: i64) -> Option<String> {
    object_by_id(doc, id).map(|(_, o)| o.file.clone()).or_else(|| doc.tiles.values().find_map(|t| t.tile.spline_attachments.iter().find(|a| a.id == id).map(|a| a.file.clone())))
}

/// What is selected for the properties: an object, or a spline attachment standing in for one.
pub fn as_sel(doc: &Document, k: TileKey, id: i64) -> Sel {
    if doc.attachment(k, id).is_some() { Sel::Attached(k, id) } else { Sel::Object(k, id) }
}

/// The objects that take their state from object `id` (`[varparent]`).
pub fn var_children(doc: &Document, id: i64) -> Vec<(TileKey, &omsi_map::MapObject)> {
    doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().chain(&t.tile.attach_objects).filter(|o| o.var_parent == Some(id)).map(move |o| (*k, o))).collect()
}

/// Take away an object hanging on another, and whatever hangs on it in turn.
fn delete_attached(doc: &mut Document, k: TileKey, id: i64) {
    let Some(t) = doc.tiles.get_mut(&k) else { return };
    let mut gone = vec![id];
    let mut i = 0;
    while i < gone.len() {
        let p = gone[i];
        gone.extend(t.tile.attach_objects.iter().filter(|o| o.parent_id == Some(p)).map(|o| o.id));
        i += 1;
    }
    t.tile.attach_objects.retain(|o| !gone.contains(&o.id));
    t.touch_content();
}

/// Where a world point appears in a view of `w` × `h` points (None: behind the eye).
pub fn project(cam: &crate::camera::Camera, p: DVec3, w: f32, h: f32) -> Option<(f32, f32)> {
    let vp = cam.proj(w / h) * cam.view();
    let q = vp * Vec4::from(((p - cam.origin()).as_vec3(), 1.0));
    if q.w <= 1e-6 {
        return None;
    }
    let (x, y) = (q.x / q.w, q.y / q.w);
    Some(((x + 1.0) * 0.5 * w, (1.0 - y) * 0.5 * h))
}

/// The objects and splines inside a screen box (view points, `a` and `b` its corners). A
/// spline is inside when its start, middle and end are; `skip` leaves out what the view
/// does not show.
pub fn in_box(doc: &Document, cam: &crate::camera::Camera, a: (f32, f32), b: (f32, f32), w: f32, h: f32, mut skip: impl FnMut(&str) -> bool, splines: bool) -> Vec<Sel> {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    let inside = |p: DVec3| project(cam, p, w, h).map(|(x, y)| x >= x0 && x <= x1 && y >= y0 && y <= y1).unwrap_or(false);
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        for o in &t.tile.objects {
            if skip(&o.file) {
                continue;
            }
            if inside(doc.object_world(*k, o, false)) {
                out.push(Sel::Object(*k, o.id));
            }
        }
        // (each [splineAttachement] record is a thing of its own: its first copy's place)
        for a in &t.tile.spline_attachments {
            if skip(&a.file) {
                continue;
            }
            if attachment_first(doc, *k, a.id).map(|f| inside(f.pose.pos)).unwrap_or(false) {
                out.push(Sel::Attached(*k, a.id));
            }
        }
        let org = tile_origin(*k);
        for s in t.tile.splines.iter().filter(|s| splines && !s.deleted) {
            let c = omsi_geometry::SplineCurve::from_map(s, org);
            if [0.0, 0.5, 1.0].iter().all(|f| inside(c.point_at(s.length * f))) {
                out.push(Sel::Spline(*k, s.id));
            }
        }
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::doc::TileDoc;
    use omsi_map::{MapObject, MapSpline};

    /// Spline ends with nothing linked there within `r` of `c` (tests).
    pub fn free_ends_near(doc: &Document, c: glam::DVec2, r: f64) -> usize {
        let mut n = 0;
        for (k, t) in &doc.tiles {
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let cv = omsi_geometry::SplineCurve::from_map(s, crate::doc::tile_origin(*k));
                if s.prev_id == 0 && (cv.point_at(0.0).truncate() - c).length() < r {
                    n += 1;
                }
                if s.next_id == 0 && (cv.end_point().truncate() - c).length() < r {
                    n += 1;
                }
            }
        }
        n
    }

    /// A map of `n` × 1 flat tiles from tile 0, 0 east, for tests.
    pub fn flat_map(n: i32) -> Document {
        crate::doc::hold_map_for_test();
        let mut doc = Document {
            global: omsi_map::GlobalCfg { next_id_code: 100, ..Default::default() },
            _publisher_mounts: std::sync::Arc::new(Default::default()),
            _publisher_protected_source: false,
            tiles: Default::default(),
            order: Vec::new(),
            dir: std::env::temp_dir().join("openmapper-test"),
            source: None,
            global_dirty: false,
            fresh: true,
            global_rev: 0,
            timetable: Default::default(),
            traffic: Default::default(),
            descriptions: Default::default(),
            groups: Vec::new(),
        };
        for x in 0..n {
            doc.tiles.insert((x, 0), TileDoc::new_flat(0.0));
            doc.order.push((x, 0));
        }
        doc
    }

    pub fn add_object(doc: &mut Document, k: TileKey, id: i64, x: f64, y: f64) {
        doc.tiles.get_mut(&k).unwrap().tile.objects.push(MapObject { file: "Sceneryobjects\\x.sco".into(), id, pos: [x, y, 0.0], ..Default::default() });
    }

    pub fn add_spline(doc: &mut Document, k: TileKey, id: i64, x: f64, y: f64, heading: f64, length: f64, prev: i64, next: i64) {
        doc.tiles.get_mut(&k).unwrap().tile.splines.push(MapSpline { file: "Splines\\road.sli".into(), id, prev_id: prev, next_id: next, pos: [x, y, 0.0], heading, length, ..Default::default() });
    }

    /// A `[splineAttachement]` record stands where its first copy does; a move re-projects it
    /// onto its own road, so it stays on the road and its height above it is kept.
    #[test]
    fn moving_an_attached_object_keeps_it_on_its_road() {
        let mut doc = flat_map(1);
        // (north from (100, 100): right of it is east)
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 200.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(omsi_map::SplineAttachment {
            file: "Sceneryobjects\\lamp.sco".into(),
            id: 5,
            spline_index: 0,
            offset: [-2.0, 1.0, 10.0],
            interval: 20.0,
            range: 40.0,
            ..Default::default()
        });
        let s = Sel::Attached((0, 0), 5);
        let from = origin_of(&doc, s).unwrap();
        // (2 m left of the road, 10 m along it)
        assert!((from.pos - DVec2::new(98.0, 110.0)).length() < 1e-6, "{:?}", from.pos);
        let mut hist = History::default();
        hist.begin("t");
        let now = transform(&mut doc, &mut hist, &[from], DVec2::ZERO, 0.0, DVec2::new(107.0, 130.0) - from.pos);
        hist.commit();
        assert_eq!(now, vec![s]);
        let a = doc.attachment((0, 0), 5).unwrap();
        assert!((a.offset[2] - 30.0).abs() < 1e-6, "{:?}", a.offset);
        assert!((a.offset[0] - 7.0).abs() < 1e-6, "{:?}", a.offset);
        assert!((a.offset[1] - 1.0).abs() < 1e-6, "the height above the road is kept: {:?}", a.offset);
        // every copy of the row is on the road still, 7 m to its right, 1 m above it
        let t = &doc.tiles[&(0, 0)].tile;
        let copies = crate::place::row_objects(&t.spline_attachments[0], &t.splines, tile_origin((0, 0)));
        assert_eq!(copies.len(), 3);
        for c in &copies {
            assert!((c.pose.pos.x - 107.0).abs() < 1e-6 && (c.pose.pos.z - 1.0).abs() < 1e-6, "{:?}", c.pose.pos);
        }
        // (the whole move is one step)
        hist.undo(&mut doc);
        assert!((doc.attachment((0, 0), 5).unwrap().offset[0] + 2.0).abs() < 1e-6);
    }

    /// A move of many pieces keeps the tiles of the links they have outside it, as a move of
    /// one does: the links are read once for the whole move (the same tiles, a move of a
    /// chain of thousands paid for once).
    #[test]
    fn a_link_outside_the_move_is_found_wherever_it_is() {
        let mut one = flat_map(2);
        let mut many = flat_map(2);
        for doc in [&mut one, &mut many] {
            add_spline(doc, (0, 0), 1, 20.0, 20.0, 0.0, 100.0, 0, 2);
            add_spline(doc, (1, 0), 2, 20.0, 20.0, 0.0, 100.0, 1, 0);
        }
        let mut h1 = History::default();
        h1.begin("t");
        touch_spline(&mut h1, &one, (0, 0), 1);
        h1.commit();
        let mut h2 = History::default();
        h2.begin("t");
        let w = SplineWhere::of(&many);
        touch_spline_where(&mut h2, &many, &w, (0, 0), 1);
        h2.commit();
        let a = h1.take_changed().tiles;
        let b = h2.take_changed().tiles;
        assert!(a.contains(&(1, 0)), "the link's tile is kept: {a:?}");
        assert_eq!(a, b);
    }

    #[test]
    fn a_group_turns_round_its_centre_and_moves_as_one() {
        let mut doc = flat_map(2);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        add_object(&mut doc, (0, 0), 2, 120.0, 100.0);
        let mut hist = History::default();
        let sels = [Sel::Object((0, 0), 1), Sel::Object((0, 0), 2)];
        let from: Vec<Origin> = sels.iter().filter_map(|s| origin_of(&doc, *s)).collect();
        let c = centre(&doc, &sels).unwrap();
        assert!((c - DVec2::new(110.0, 100.0)).length() < 1e-9);
        hist.begin("t");
        let now = transform(&mut doc, &mut hist, &from, c, 90.0, DVec2::new(250.0, 0.0));
        hist.commit();
        // turned a quarter clockwise round (110, 100): (100, 100) to (110, 110); then 250 east,
        // over into the next tile
        assert_eq!(now[0], Sel::Object((1, 0), 1));
        let o = doc.object((1, 0), 1).unwrap();
        assert!((o.pos[0] - 60.0).abs() < 1e-9 && (o.pos[1] - 110.0).abs() < 1e-9, "{:?}", o.pos);
        assert!((o.rot[0] - 90.0).abs() < 1e-9);
        let o2 = doc.object((1, 0), 2).unwrap();
        assert!((o2.pos[1] - 90.0).abs() < 1e-9, "{:?}", o2.pos);
        // one step undoes it all
        hist.undo(&mut doc);
        assert!(doc.object((0, 0), 1).is_some() && doc.object((0, 0), 2).is_some());
        assert!(doc.tiles[&(1, 0)].tile.objects.is_empty());
    }

    #[test]
    fn moving_part_of_a_chain_lets_go_of_the_rest() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 10.0, 10.0, 0.0, 50.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 10.0, 60.0, 0.0, 50.0, 1, 3);
        add_spline(&mut doc, (0, 0), 3, 10.0, 110.0, 0.0, 50.0, 2, 0);
        let mut hist = History::default();
        hist.begin("t");
        let sels = [Sel::Spline((0, 0), 2), Sel::Spline((0, 0), 3)];
        let from: Vec<Origin> = sels.iter().filter_map(|s| origin_of(&doc, *s)).collect();
        transform(&mut doc, &mut hist, &from, DVec2::ZERO, 0.0, DVec2::new(20.0, 0.0));
        unlink_outside(&mut doc, &mut hist, &sels);
        hist.commit();
        let s = |id| doc.spline((0, 0), id).unwrap().clone();
        assert_eq!((s(1).next_id, s(2).prev_id), (0, 0));
        // inside the selection the link stays
        assert_eq!((s(2).next_id, s(3).prev_id), (3, 2));
        assert!((s(3).pos[0] - 30.0).abs() < 1e-9);
    }

    #[test]
    fn delete_takes_the_whole_selection_in_one_step() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 10.0, 10.0);
        add_spline(&mut doc, (0, 0), 2, 10.0, 10.0, 0.0, 50.0, 0, 3);
        add_spline(&mut doc, (0, 0), 3, 10.0, 60.0, 0.0, 50.0, 2, 0);
        let mut hist = History::default();
        hist.begin("t");
        delete(&mut doc, &mut hist, &[Sel::Object((0, 0), 1), Sel::Spline((0, 0), 2)]);
        hist.commit();
        assert!(doc.object((0, 0), 1).is_none() && doc.spline((0, 0), 2).is_none());
        assert_eq!(doc.spline((0, 0), 3).unwrap().prev_id, 0);
        hist.undo(&mut doc);
        assert!(doc.object((0, 0), 1).is_some() && doc.spline((0, 0), 2).is_some());
        assert_eq!(doc.spline((0, 0), 3).unwrap().prev_id, 2);
    }

    #[test]
    fn deleting_a_hanging_object_takes_what_hangs_on_it() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 10.0, 10.0);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.attach_objects.push(MapObject { file: "sign.sco".into(), id: 6, parent_id: Some(1), ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "plate.sco".into(), id: 7, parent_id: Some(6), ..Default::default() });
        assert!(any_object(&doc, (0, 0), 6).is_some());
        let mut hist = History::default();
        hist.begin("t");
        delete(&mut doc, &mut hist, &[Sel::Object((0, 0), 6)]);
        hist.commit();
        assert!(doc.object((0, 0), 1).is_some());
        assert!(doc.tiles[&(0, 0)].tile.attach_objects.is_empty());
    }

    #[test]
    fn a_point_projects_where_its_ray_comes_from() {
        let mut cam = crate::camera::Camera::default();
        cam.snap();
        let (w, h) = (800.0, 600.0);
        let (o, d) = cam.ray(300.0, 200.0, w, h);
        let p = o + d * 120.0;
        let (x, y) = project(&cam, p, w, h).unwrap();
        assert!((x - 300.0).abs() < 0.5 && (y - 200.0).abs() < 0.5, "{x} {y}");
    }

    #[test]
    fn a_box_selects_what_lies_inside_it() {
        let mut doc = flat_map(1);
        let mut cam = crate::camera::Camera::default();
        cam.snap();
        let (w, h) = (800.0, 600.0);
        // (the camera looks at 150, 150)
        add_object(&mut doc, (0, 0), 1, 150.0, 150.0);
        add_object(&mut doc, (0, 0), 2, 290.0, 10.0);
        add_spline(&mut doc, (0, 0), 3, 148.0, 148.0, 0.0, 4.0, 0, 0);
        let got = in_box(&doc, &cam, (350.0, 250.0), (450.0, 350.0), w, h, |_| false, true);
        assert!(got.contains(&Sel::Object((0, 0), 1)));
        assert!(got.contains(&Sel::Spline((0, 0), 3)));
        assert!(!got.contains(&Sel::Object((0, 0), 2)));
    }

    /// An object moved over into the next tile takes what hangs on it along: the game and
    /// the view look for a hanging object's parent in its own tile.
    #[test]
    fn what_hangs_on_an_object_goes_with_it_into_the_next_tile() {
        let mut doc = flat_map(2);
        add_object(&mut doc, (0, 0), 1, 290.0, 10.0);
        add_object(&mut doc, (0, 0), 2, 20.0, 10.0);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.attach_objects.push(MapObject { file: "sign.sco".into(), id: 6, parent_id: Some(1), ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "plate.sco".into(), id: 7, parent_id: Some(6), ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "other.sco".into(), id: 8, parent_id: Some(2), ..Default::default() });
        assert_eq!(place_object_at(&mut doc, (0, 0), 1, DVec2::new(310.0, 10.0)), Some(Sel::Object((1, 0), 1)));
        let ids = |k: TileKey| doc.tiles[&k].tile.attach_objects.iter().map(|o| o.id).collect::<Vec<_>>();
        assert_eq!(ids((1, 0)), vec![6, 7]);
        assert_eq!(ids((0, 0)), vec![8]);
    }

    /// An object deleted takes what hangs on it and what hangs on that.
    #[test]
    fn deleting_an_object_takes_everything_hanging_on_it() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 10.0, 10.0);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.attach_objects.push(MapObject { file: "sign.sco".into(), id: 6, parent_id: Some(1), ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "plate.sco".into(), id: 7, parent_id: Some(6), ..Default::default() });
        let mut hist = History::default();
        hist.begin("t");
        delete(&mut doc, &mut hist, &[Sel::Object((0, 0), 1)]);
        hist.commit();
        assert!(doc.tiles[&(0, 0)].tile.attach_objects.is_empty());
    }

    /// A road deleted takes its rows of objects and what hangs on them (lamps on masts).
    #[test]
    fn deleting_a_road_takes_what_hangs_on_its_rows() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 2, 50.0, 50.0, 0.0, 20.0, 0, 0);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.spline_attachments.push(omsi_map::SplineAttachment { file: "mast.sco".into(), id: 5, spline_index: 0, interval: 10.0, range: 20.0, ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "lamp.sco".into(), id: 6, parent_id: Some(5), ..Default::default() });
        t.tile.attach_objects.push(MapObject { file: "bulb.sco".into(), id: 7, parent_id: Some(6), ..Default::default() });
        doc.delete_spline((0, 0), 2);
        let t = &doc.tiles[&(0, 0)].tile;
        assert!(t.spline_attachments.is_empty() && t.attach_objects.is_empty());
    }
}
