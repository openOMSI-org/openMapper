//! The AI's lanes as the map has them: every `[path]` of the roads (along the spline, beside
//! its centre line) and of the objects (junctions, stops), as lines with a direction and as
//! ends. openOMSI links a lane to another one where their ends meet, so an end with no other
//! end on it is where traffic stops dead: the Logic tool shows those, and AI paths are drawn
//! snapping to the ends.

use glam::{DVec2, DVec3, Vec3};
use omsi_geometry::SplineCurve;

use crate::assets::Assets;
use crate::doc::{tile_origin, Document, Sel, TileKey};
use crate::place::Pose;
use crate::roads;

/// How near two lane ends must lie to be linked (m).
pub const LINK: f64 = 0.6;

/// An object's `[path]` in its own frame. The two numbers after the length are the slope
/// at the start and at the end, as rise per metre (a spline stores that same slope in
/// percent, and its cant is already in the height of each lane). Without them the path
/// stays at its start height, so a junction on a hill draws a flat lane through a sloping plate.
pub fn object_path_curve(p: &omsi_scenery::PathDef) -> SplineCurve {
    let (g0, g1) = match (p.params.get(6), p.params.get(7)) {
        (Some(a), Some(b)) => (*a as f64 * 100.0, *b as f64 * 100.0),
        _ => (0.0, 0.0),
    };
    SplineCurve {
        start: DVec3::new(p.start[0] as f64, p.start[1] as f64, p.start[2] as f64),
        heading_deg: p.end[0] as f64,
        length: p.end[2] as f64,
        radius: p.end[1] as f64,
        grad_start: g0,
        grad_end: g1,
        delta_h: None,
        cant_start: 0.0,
        cant_end: 0.0,
        skew_start: 0.0,
        skew_end: 0.0,
        tex_offset: 0.0,
        seed: 0,
        half_cant_width: 0.0,
    }
}

/// One lane, as points along it in the way it is laid out (world).
#[derive(Debug, Clone)]
pub struct Lane {
    pub pts: Vec<DVec3>,
    /// 0 cars, 1 pedestrians, 2 rail.
    pub kind: i32,
    /// 0 traffic goes along `pts`, 1 against, 2 both ways.
    pub direction: i32,
    /// The spline it runs along (tile, IDCode, length), when it is a spline's.
    pub spline: Option<((i32, i32), i64, f64)>,
    /// The object it belongs to (tile, IDCode), when it is an object's.
    pub object: Option<((i32, i32), i64)>,
    /// Its place among its spline's or object's paths (what a path rule names).
    pub index: usize,
}

/// One end of a lane.
#[derive(Debug, Clone, Copy)]
pub struct LaneEnd {
    pub pos: DVec3,
    /// Away from the lane: the way a lane joined here goes on.
    pub heading: f64,
    pub kind: i32,
    /// Traffic may leave the lane here (and go on into one joined), or come into it.
    pub exit: bool,
    pub entry: bool,
    /// Another lane's end of the same kind lies on it.
    pub linked: bool,
}

impl Lane {
    /// Its two ends (before they are checked for links).
    pub(crate) fn ends(&self) -> [LaneEnd; 2] {
        let n = self.pts.len();
        let h0 = roads::heading_of((self.pts[1] - self.pts[0]).truncate());
        let h1 = roads::heading_of((self.pts[n - 1] - self.pts[n - 2]).truncate());
        let (fwd, back) = (self.direction != 1, self.direction != 0);
        [
            LaneEnd { pos: self.pts[0], heading: roads::norm_deg(h0 + 180.0), kind: self.kind, exit: back, entry: fwd, linked: false },
            LaneEnd { pos: self.pts[n - 1], heading: h1, kind: self.kind, exit: fwd, entry: back, linked: false },
        ]
    }

    /// The point on it nearest `p` (ground plane): (point, heading the lane is laid out in
    /// there, distance, how far along its spline when it is one's).
    pub fn nearest(&self, p: DVec2) -> Option<(DVec3, f64, f64, Option<f64>)> {
        let mut best: Option<(DVec3, f64, f64, Option<f64>)> = None;
        let steps = (self.pts.len() - 1).max(1) as f64;
        for (i, w) in self.pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let ab = (b - a).truncate();
            let len2 = ab.length_squared();
            if len2 < 1e-9 {
                continue;
            }
            let t = ((p - a.truncate()).dot(ab) / len2).clamp(0.0, 1.0);
            let q = a + (b - a) * t;
            let d = (q.truncate() - p).length();
            if best.map(|x| d < x.2).unwrap_or(true) {
                best = Some((q, roads::heading_of(ab), d, self.spline.map(|(_, _, len)| len * (i as f64 + t) / steps)));
            }
        }
        best
    }
}

/// How far from a crossing the neighbouring paths are gathered and drawn.
/// Zoomed in, still wide enough for the roads that meet a large junction; zoomed out, not
/// the whole map (a map of lanes overflowed the graphics buffer).
pub fn neighbour_reach(distance: f64) -> f64 {
    (distance * 3.0).clamp(300.0, 3500.0)
}

/// The samples of a lane that fall inside `radius` of `centre`, widened by one sample so the
/// drawn line reaches the circle, and how near the lane comes. None when it misses.
/// A segment that only cuts the circle counts too: a long spline is sampled coarsely, and
/// both samples can sit just outside while the road itself passes through.
pub fn visible_span(pts: &[DVec3], centre: DVec2, radius: f64) -> Option<(usize, usize, f64)> {
    if pts.is_empty() {
        return None;
    }
    let mut best = f64::MAX;
    let mut lo: Option<usize> = None;
    let mut hi = 0usize;
    let mut mark = |i: usize, d: f64| {
        if d < best {
            best = d;
        }
        if d <= radius {
            lo = Some(lo.unwrap_or(i).min(i));
            hi = hi.max(i);
        }
    };
    for (i, p) in pts.iter().enumerate() {
        mark(i, (p.truncate() - centre).length());
    }
    for (i, w) in pts.windows(2).enumerate() {
        let (a, b) = (w[0].truncate(), w[1].truncate());
        let ab = b - a;
        let len2 = ab.length_squared();
        if len2 < 1e-9 {
            continue;
        }
        let t = ((centre - a).dot(ab) / len2).clamp(0.0, 1.0);
        let d = (a + ab * t - centre).length();
        if d <= radius {
            mark(i, d);
            mark(i + 1, d);
        }
    }
    let lo = lo?;
    if pts.len() == 1 {
        return Some((0, 0, best));
    }
    Some((lo.saturating_sub(1), (hi + 1).min(pts.len() - 1), best))
}

/// The lanes within `radius` of `centre`, and their ends (linked ones marked).
/// The lanes of one road or object, each with its path number.
pub fn of_one(doc: &Document, assets: &mut Assets, sel: Sel) -> Vec<Lane> {
    let (at, reach) = match sel {
        Sel::Spline(k, id) => match doc.spline(k, id) {
            Some(s) => (tile_origin(k) + DVec2::new(s.pos[0], s.pos[1]), s.length + 1.0),
            None => return Vec::new(),
        },
        Sel::Object(k, id) => match doc.object(k, id) {
            Some(o) => (tile_origin(k) + DVec2::new(o.pos[0], o.pos[1]), 1.0),
            None => return Vec::new(),
        },
        Sel::Attached(k, id) => match crate::selection::attachment_first(doc, k, id) {
            Some(f) => (f.pose.pos.truncate(), 1.0),
            None => return Vec::new(),
        },
    };
    near(doc, assets, at, reach).0.into_iter().filter(|l| match sel {
        Sel::Spline(k, id) => l.spline.map(|s| (s.0, s.1)) == Some((k, id)),
        Sel::Object(k, id) => l.object == Some((k, id)),
        Sel::Attached(..) => false,
    }).collect()
}

pub fn near(doc: &Document, assets: &mut Assets, centre: DVec2, radius: f64) -> (Vec<Lane>, Vec<LaneEnd>) {
    near_skipping(doc, assets, centre, radius, None)
}

/// One end of a lane, without the line between the ends. `along` runs with the lane, in the
/// direction of its length, through this end (world axes, not normalised): its horizontal way
/// is the lane's heading there, and its rise over its length is the slope.
#[derive(Clone, Copy)]
pub struct LaneTip {
    pub pos: DVec3,
    pub along: DVec3,
    pub at_end: bool,
    pub direction: i32,
    pub kind: i32,
}

/// The ends within `radius` of `centre`, and nothing else. [`near_skipping`] lays out every
/// lane in full — hundreds of points a path, then a link check over all of them — which is
/// what the lines on the map need. A click on an end needs the two ends only, and doing the
/// whole of that every frame is what made the nearby-path modes stall.
pub fn tips_skipping(doc: &Document, assets: &mut Assets, centre: DVec2, radius: f64, skip: Option<(TileKey, i64)>) -> Vec<LaneTip> {
    let mut tips = Vec::new();
    let push = |tips: &mut Vec<LaneTip>, pos: DVec3, into: DVec3, at_end: bool, direction: i32, kind: i32| {
        if (pos.truncate() - centre).length() <= radius {
            tips.push(LaneTip { pos, along: into, at_end, direction, kind });
        }
    };
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        if (org + DVec2::splat(crate::doc::tile_size() * 0.5) - centre).length() > radius + crate::doc::tile_size() {
            continue;
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
            if (start - centre).length() > s.length + radius + 40.0 {
                continue;
            }
            let Some(m) = assets.spline(&s.file) else { continue };
            if m.def.paths.is_empty() || !(s.length > 0.05) {
                continue;
            }
            let c = SplineCurve::from_map(s, org);
            let mirror = if s.mirror { -1.0 } else { 1.0 };
            let d = 1.5_f64.min(s.length * 0.5).max(0.05);
            for p in &m.def.paths {
                let x = p.start[0] as f64 * mirror;
                let z = p.start[2] as f64;
                let a0 = c.offset_point(0.0, x, z);
                let a1 = c.offset_point(d, x, z);
                let b1 = c.offset_point(s.length - d, x, z);
                let b0 = c.offset_point(s.length, x, z);
                let direction = match (p.direction, s.mirror) {
                    (0, true) => 1,
                    (1, true) => 0,
                    (dir, _) => dir,
                };
                push(&mut tips, a0, a1 - a0, false, direction, p.kind);
                push(&mut tips, b0, b0 - b1, true, direction, p.kind);
            }
        }
        for o in &t.tile.objects {
            if skip == Some((*k, o.id)) {
                continue;
            }
            let Some(model) = assets.object(&o.file) else { continue };
            if model.paths.is_empty() {
                continue;
            }
            let world = doc.object_world(*k, o, model.abs_height);
            let mesh = model.radius() as f64;
            let paths = model.paths.iter().fold(0.0_f64, |m, p| {
                let start = (p.start[0] as f64).hypot(p.start[1] as f64);
                m.max(start + (p.end[2] as f64).abs())
            });
            if (world.truncate() - centre).length() > radius + mesh.max(paths) {
                continue;
            }
            let pose = Pose::of_object(world, o.rot);
            let to_world = |q: DVec3| pose.pos + pose.rot.transform_vector3(Vec3::new(q.x as f32, q.y as f32, q.z as f32)).as_dvec3();
            for p in &model.paths {
                let c = object_path_curve(p);
                if !(c.length > 0.05) {
                    continue;
                }
                let d = 1.5_f64.min(c.length * 0.5).max(0.05);
                let a0 = to_world(c.point_at(0.0));
                let a1 = to_world(c.point_at(d));
                let b1 = to_world(c.point_at(c.length - d));
                let b0 = to_world(c.point_at(c.length));
                push(&mut tips, a0, a1 - a0, false, p.direction, p.kind);
                push(&mut tips, b0, b0 - b1, true, p.direction, p.kind);
            }
        }
    }
    tips
}

/// The lanes within `radius` of `centre`, and their ends (linked ones marked), leaving out
/// one object: the crossing editor knows its own object already and used to sample all 787
/// of its paths only to throw them away afterwards. That object is skipped before anything
/// of it is read.
pub fn near_skipping(doc: &Document, assets: &mut Assets, centre: DVec2, radius: f64, skip: Option<(TileKey, i64)>) -> (Vec<Lane>, Vec<LaneEnd>) {
    let mut lanes = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        if (org + DVec2::splat(crate::doc::tile_size() * 0.5) - centre).length() > radius + crate::doc::tile_size() {
            continue;
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
            // (a lane sits a few metres off the centre line, so the line's own reach is
            // not quite enough)
            if (start - centre).length() > s.length + radius + 40.0 {
                continue;
            }
            let Some(m) = assets.spline(&s.file) else { continue };
            if m.def.paths.is_empty() {
                continue;
            }
            let c = SplineCurve::from_map(s, org);
            // (a step of 3 m, and a ceiling on it: a map with a broken length on a spline
            // used to ask for millions of samples, and the lanes of it filled memory)
            let n = ((s.length / 3.0).ceil().max(1.0) as usize).min(400);
            let mirror = if s.mirror { -1.0 } else { 1.0 };
            for (index, p) in m.def.paths.iter().enumerate() {
                let x = p.start[0] as f64 * mirror;
                let z = p.start[2] as f64;
                let pts: Vec<DVec3> = (0..=n).map(|i| c.offset_point(s.length * i as f64 / n as f64, x, z)).collect();
                // (a mirrored spline's lanes run the other way)
                let direction = match (p.direction, s.mirror) {
                    (0, true) => 1,
                    (1, true) => 0,
                    (d, _) => d,
                };
                lanes.push(Lane { pts, kind: p.kind, direction, spline: Some((*k, s.id, s.length)), object: None, index });
            }
        }
        for o in &t.tile.objects {
            if skip == Some((*k, o.id)) {
                continue;
            }
            let Some(model) = assets.object(&o.file) else { continue };
            if model.paths.is_empty() {
                continue;
            }
            let world = doc.object_world(*k, o, model.abs_height);
            // (a path runs out past the mesh: the mesh's radius alone hid junctions whose
            // roads leave the plate)
            let mesh = model.radius() as f64;
            let paths = model.paths.iter().fold(0.0_f64, |m, p| {
                let start = (p.start[0] as f64).hypot(p.start[1] as f64);
                m.max(start + (p.end[2] as f64).abs())
            });
            if (world.truncate() - centre).length() > radius + mesh.max(paths) {
                continue;
            }
            let pose = Pose::of_object(world, o.rot);
            // (an object of hundreds of paths is sampled coarsely: only its ends matter for
            // snapping, and 787 paths at 2 m would be most of the frame's work)
            let step = if model.paths.len() > 600 { 8.0 } else { 2.0 };
            for (index, p) in model.paths.iter().enumerate() {
                let c = object_path_curve(p);
                if !(c.length > 0.05) {
                    continue;
                }
                let n = ((c.length / step).ceil().max(1.0) as usize).min(400);
                let pts = (0..=n)
                    .map(|i| {
                        let q = c.point_at(c.length * i as f64 / n as f64);
                        pose.pos + pose.rot.transform_vector3(Vec3::new(q.x as f32, q.y as f32, q.z as f32)).as_dvec3()
                    })
                    .collect();
                lanes.push(Lane { pts, kind: p.kind, direction: p.direction, spline: None, object: Some((*k, o.id)), index });
            }
        }
    }
    let mut ends: Vec<LaneEnd> = lanes.iter().filter(|l| l.pts.len() >= 2).flat_map(|l| l.ends()).collect();
    // (linked: another lane of the kind ends there; found through a grid of LINK cells)
    let cell = |p: DVec3| ((p.x / LINK).floor() as i64, (p.y / LINK).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, e) in ends.iter().enumerate() {
        grid.entry(cell(e.pos)).or_default().push(i);
    }
    for i in 0..ends.len() {
        let (cx, cy) = cell(ends[i].pos);
        let mut hit = false;
        for dy in -1..=1 {
            for dx in -1..=1 {
                for &j in grid.get(&(cx + dx, cy + dy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let (a, b) = (&ends[i], &ends[j]);
                    if i != j && a.kind == b.kind && (a.pos.truncate() - b.pos.truncate()).length() < LINK && (a.pos.z - b.pos.z).abs() < 1.5 {
                        hit = true;
                    }
                }
            }
        }
        ends[i].linked = hit;
    }
    (lanes, ends)
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};

    use super::*;

    /// The sample ceiling: a lane of a spline or an object never asks for more points than the
    /// cap, so a broken length on one cannot fill memory (and the overlay buffer with it).
    #[test]
    fn a_lane_is_sampled_within_the_ceiling() {
        const CAP: usize = 400;
        // (what the two loops in `near` do: the step, and the ceiling)
        for length in [0.0f64, 1.0, 300.0, 1200.0, 5000.0, 100_000.0] {
            let n = ((length / 3.0).ceil().max(1.0) as usize).min(CAP);
            assert!(n <= CAP, "{length} m asked for {n} samples");
            assert!(n >= 1);
        }
        let n = ((5_000.0f64 / 2.0).ceil().max(1.0) as usize).min(CAP);
        assert!(n <= CAP);
    }

    /// A road object's path climbs with the slope written after its length (rise per metre),
    /// the same heights a spline's gradient gives, including a crest.
    #[test]
    fn an_object_path_follows_its_slope() {
        let path = |g0: f32, g1: f32| omsi_scenery::PathDef {
            start: [0.0, 0.0, 5.0],
            end: [0.0, 0.0, 20.0],
            params: vec![0.0, 0.0, 5.0, 0.0, 0.0, 20.0, g0, g1],
            ..Default::default()
        };
        let z = |g0, g1, s: f64| object_path_curve(&path(g0, g1)).point_at(s).z;
        assert!((z(0.04, 0.04, 0.0) - 5.0).abs() < 1e-6);
        assert!((z(0.04, 0.04, 10.0) - 5.4).abs() < 1e-6);
        assert!((z(0.04, 0.04, 20.0) - 5.8).abs() < 1e-6);
        assert!((z(0.06, -0.04, 10.0) - 5.35).abs() < 1e-6);
        assert!((z(0.06, -0.04, 20.0) - 5.2).abs() < 1e-6);
        assert!((z(0.0, 0.0, 20.0) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn neighbour_reach_stays_between_a_junction_and_the_map() {
        assert_eq!(neighbour_reach(10.0), 300.0);
        assert_eq!(neighbour_reach(100.0), 300.0);
        assert_eq!(neighbour_reach(1_000.0), 3000.0);
        assert_eq!(neighbour_reach(2_000.0), 3500.0);
    }

    #[test]
    fn a_lane_that_only_cuts_the_circle_is_still_shown() {
        let pts = vec![DVec3::new(-30.0, 4.0, 0.0), DVec3::new(30.0, 4.0, 0.0)];
        let (lo, hi, d) = visible_span(&pts, DVec2::ZERO, 10.0).unwrap();
        assert_eq!((lo, hi), (0, 1));
        assert!(d < 10.0);
        assert!(visible_span(&pts, DVec2::new(0.0, 80.0), 10.0).is_none());
    }
}
