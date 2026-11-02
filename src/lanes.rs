//! The AI's lanes as the map has them: every `[path]` of the roads (along the spline, beside
//! its centre line) and of the objects (junctions, stops), as lines with a direction and as
//! ends. openOMSI links a lane to another one where their ends meet, so an end with no other
//! end on it is where traffic stops dead: the Logic tool shows those, and AI paths are drawn
//! snapping to the ends.

use glam::{DVec2, DVec3, Vec3};
use omsi_geometry::SplineCurve;

use crate::assets::Assets;
use crate::doc::{tile_origin, Document, Sel};
use crate::place::Pose;
use crate::roads;

/// How near two lane ends must lie to be linked (m).
pub const LINK: f64 = 0.6;

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
    let mut lanes = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        if (org + DVec2::splat(crate::doc::tile_size() * 0.5) - centre).length() > radius + crate::doc::tile_size() {
            continue;
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
            if (start - centre).length() > s.length + radius {
                continue;
            }
            let Some(m) = assets.spline(&s.file) else { continue };
            if m.def.paths.is_empty() {
                continue;
            }
            let c = SplineCurve::from_map(s, org);
            let n = (s.length / 3.0).ceil().max(1.0) as usize;
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
            let Some(model) = assets.object(&o.file) else { continue };
            if model.paths.is_empty() {
                continue;
            }
            let world = doc.object_world(*k, o, model.abs_height);
            if (world.truncate() - centre).length() > radius + model.radius() as f64 {
                continue;
            }
            let pose = Pose::of_object(world, o.rot);
            for (index, p) in model.paths.iter().enumerate() {
                let c = SplineCurve {
                    start: DVec3::new(p.start[0] as f64, p.start[1] as f64, p.start[2] as f64),
                    heading_deg: p.end[0] as f64,
                    length: p.end[2] as f64,
                    radius: p.end[1] as f64,
                    grad_start: 0.0,
                    grad_end: 0.0,
                    delta_h: None,
                    cant_start: 0.0,
                    cant_end: 0.0,
                    skew_start: 0.0,
                    skew_end: 0.0,
                    tex_offset: 0.0,
                    seed: 0,
                    half_cant_width: 0.0,
                };
                if !(c.length > 0.05) {
                    continue;
                }
                let n = (c.length / 2.0).ceil().max(1.0) as usize;
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
