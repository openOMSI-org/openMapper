//! The roads' lanes as a timetable names them: every `[path]` of the map's splines and
//! objects by (tile, IDCode, path index), laid out in the way traffic drives it and linked
//! to the lanes it runs on into, as openOMSI links them (ends within 1.5 m, headings within
//! 40°).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use glam::{DVec2, DVec3, Vec3};
use omsi_geometry::SplineCurve;
use omsi_scenery::PathDef;

use crate::doc::{tile_origin, Document, TileKey};
use crate::place::Pose;

/// How near a lane's end must lie to the next one's start to be linked (m).
const LINK: f64 = 1.5;
/// How far the headings at a link may differ (degrees).
const LINK_TURN: f64 = 40.0;

/// How far apart two lanes of a spline may lie for a bus to change between them (m).
const LANE_CHANGE_WIDTH: f64 = 5.0;

/// What the timetable needs of a spline or object file.
#[derive(Debug, Default)]
pub struct FileInfo {
    pub paths: Vec<PathDef>,
    pub abs_height: bool,
    pub is_bus_stop: bool,
}

/// The files' paths, read once (from the content folder before the installation).
#[derive(Default)]
pub struct Files {
    roots: Vec<PathBuf>,
    cache: HashMap<String, Option<Arc<FileInfo>>>,
}

impl Files {
    pub fn new(roots: Vec<PathBuf>) -> Files {
        Files { roots, cache: HashMap::new() }
    }

    pub fn has_roots(&self) -> bool {
        !self.roots.is_empty()
    }

    /// The file is there (content folder or installation).
    pub fn exists(&self, rel: &str) -> bool {
        self.roots.iter().any(|r| omsi_cfg::vfs::exists(&omsi_cfg::resolve_path(r, rel)))
    }

    pub fn get(&mut self, rel: &str) -> Option<Arc<FileInfo>> {
        let key = rel.to_lowercase();
        if let Some(c) = self.cache.get(&key) {
            return c.clone();
        }
        let path = self.roots.iter().map(|r| omsi_cfg::resolve_path(r, rel)).find(|p| omsi_cfg::vfs::exists(p));
        let info = path.and_then(|p| {
            if key.ends_with(".sli") {
                omsi_scenery::Spline::load(&p).ok().map(|s| FileInfo { paths: s.paths, ..Default::default() })
            } else {
                omsi_scenery::SceneryObject::load(&p).ok().map(|o| FileInfo { abs_height: o.absolute_height(), is_bus_stop: o.is_bus_stop, paths: o.paths })
            }
        });
        let info = info.map(Arc::new);
        self.cache.insert(key, info.clone());
        info
    }
}

/// A path of the map: the tile, the spline's or object's IDCode and the `[path]` index in
/// its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathKey {
    pub tile: TileKey,
    pub id: i64,
    pub path: usize,
}

/// One way along a path.
#[derive(Debug, Clone)]
pub struct Lane {
    pub key: PathKey,
    /// Points the way traffic drives (world).
    pub pts: Vec<DVec3>,
    /// Distance along `pts` at each point.
    pub along: Vec<f64>,
    /// The length the timetable gives the path (the spline's, or the object path's).
    pub file_length: f64,
    /// OMSI's number of the path among all paths of its tile (a cache the `.ttr` keeps).
    pub internal: usize,
    pub next: Vec<usize>,
    /// The lanes beside it on the same spline going the same way (a lane change).
    pub side: Vec<usize>,
    /// Against the way the path lets traffic go: OMSI's buses drive a route so where it
    /// must (helper paths laid the wrong way), the route search only as a last resort.
    pub against: bool,
    /// A spline's path (an object's paths that start together branch off, they are no lanes
    /// side by side).
    pub spline: bool,
}

impl Lane {
    pub fn length(&self) -> f64 {
        *self.along.last().unwrap_or(&0.0)
    }

    fn heading_at_start(&self) -> DVec2 {
        (self.pts[1] - self.pts[0]).truncate().normalize_or_zero()
    }

    fn heading_at_end(&self) -> DVec2 {
        let n = self.pts.len();
        (self.pts[n - 1] - self.pts[n - 2]).truncate().normalize_or_zero()
    }

    /// The point nearest `p` (ground plane): (distance along, distance off, `p` lies on the
    /// right of the way traffic drives).
    pub fn project(&self, p: DVec2) -> (f64, f64, bool) {
        let mut best = (0.0, f64::MAX, true);
        for (i, w) in self.pts.windows(2).enumerate() {
            let (a, b) = (w[0].truncate(), w[1].truncate());
            let ab = b - a;
            let len2 = ab.length_squared();
            if len2 < 1e-9 {
                continue;
            }
            let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
            let q = a + ab * t;
            let d = (p - q).length();
            if d < best.1 {
                let right = ab.perp_dot(p - a) < 0.0;
                best = (self.along[i] + (self.along[i + 1] - self.along[i]) * t, d, right);
            }
        }
        best
    }

    /// The point at distance `s` along.
    pub fn point_at(&self, s: f64) -> DVec3 {
        let i = self.along.partition_point(|a| *a < s).clamp(1, self.pts.len() - 1);
        let (a0, a1) = (self.along[i - 1], self.along[i]);
        let t = if a1 > a0 { ((s - a0) / (a1 - a0)).clamp(0.0, 1.0) } else { 0.0 };
        self.pts[i - 1] + (self.pts[i] - self.pts[i - 1]) * t
    }
}

/// The car lanes of the map.
#[derive(Default)]
pub struct Network {
    pub lanes: Vec<Lane>,
    pub by_key: HashMap<PathKey, Vec<usize>>,
    /// Bus stop objects: (tile, IDCode, world position).
    pub stops: Vec<(TileKey, i64, DVec3)>,
    /// Files still to be found (paths of them are missing).
    pub missing_files: usize,
}

fn lane_of(key: PathKey, mut pts: Vec<DVec3>, file_length: f64, internal: usize, (reversed, against): (bool, bool), spline: bool) -> Lane {
    if reversed {
        pts.reverse();
    }
    let mut along = Vec::with_capacity(pts.len());
    let mut acc = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            acc += (*p - pts[i - 1]).length();
        }
        along.push(acc);
    }
    Lane { key, pts, along, file_length, internal, next: Vec::new(), side: Vec::new(), against, spline }
}

impl Network {
    pub fn build(doc: &Document, files: &mut Files) -> Network {
        let mut net = Network::default();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            // OMSI numbers a tile's paths through its splines, then its objects
            let mut internal = 0;
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let Some(info) = files.get(&s.file) else {
                    net.missing_files += 1;
                    continue;
                };
                let c = SplineCurve::from_map(s, org);
                let n = (s.length / 3.0).ceil().clamp(1.0, 400.0) as usize;
                let mirror = if s.mirror { -1.0 } else { 1.0 };
                for (pi, p) in info.paths.iter().enumerate() {
                    if p.kind == 0 {
                        let x = p.start[0] as f64 * mirror;
                        let pts: Vec<DVec3> = (0..=n).map(|i| c.offset_point(s.length * i as f64 / n as f64, x, p.start[2] as f64)).collect();
                        // (a mirrored spline's lanes run the other way)
                        let dir = match (p.direction, s.mirror) {
                            (0, true) => 1,
                            (1, true) => 0,
                            (d, _) => d,
                        };
                        net.add(PathKey { tile: *k, id: s.id, path: pi }, pts, s.length, internal + pi, dir, true);
                    }
                }
                internal += info.paths.len();
            }
            // (a stop standing on a spline is an attachment, not an object of the tile)
            for a in t.tile.spline_attachments.iter().filter(|a| a.repeater.is_none()) {
                if !files.get(&a.file).map(|i| i.is_bus_stop).unwrap_or(false) {
                    continue;
                }
                if let Some(first) = crate::place::row_objects(a, &t.tile.splines, org).first() {
                    net.stops.push((*k, a.id, first.pose.pos));
                }
            }
            for o in &t.tile.objects {
                let Some(info) = files.get(&o.file) else {
                    net.missing_files += 1;
                    continue;
                };
                let world = doc.object_world(*k, o, info.abs_height);
                if info.is_bus_stop {
                    net.stops.push((*k, o.id, world));
                }
                let pose = Pose::of_object(world, o.rot);
                for (pi, p) in info.paths.iter().enumerate() {
                    if p.kind != 0 {
                        continue;
                    }
                    let c = crate::lanes::object_path_curve(p);
                    if !(c.length > 0.05) {
                        continue;
                    }
                    let n = (c.length / 2.0).ceil().clamp(1.0, 200.0) as usize;
                    let pts = (0..=n)
                        .map(|i| {
                            let q = c.point_at(c.length * i as f64 / n as f64);
                            pose.pos + pose.rot.transform_vector3(Vec3::new(q.x as f32, q.y as f32, q.z as f32)).as_dvec3()
                        })
                        .collect();
                    net.add(PathKey { tile: *k, id: o.id, path: pi }, pts, c.length, internal + pi, p.direction, false);
                }
                internal += info.paths.len();
            }
        }
        net.link();
        net
    }

    fn add(&mut self, key: PathKey, pts: Vec<DVec3>, file_length: f64, internal: usize, direction: i32, spline: bool) {
        if pts.len() < 2 {
            return;
        }
        // (the way traffic goes first; a one-way path's other way after it)
        let ways: &[(bool, bool)] = match direction {
            0 => &[(false, false), (true, true)],
            1 => &[(true, false), (false, true)],
            _ => &[(false, false), (true, false)],
        };
        for &(rev, against) in ways {
            self.by_key.entry(key).or_default().push(self.lanes.len());
            self.lanes.push(lane_of(key, pts.clone(), file_length, internal, (rev, against), spline));
        }
    }

    /// Each lane's `next`: the lanes that start where it ends, going on the same way.
    fn link(&mut self) {
        let cell = |p: DVec3| ((p.x / LINK).floor() as i64, (p.y / LINK).floor() as i64);
        let mut starts: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, l) in self.lanes.iter().enumerate() {
            starts.entry(cell(l.pts[0])).or_default().push(i);
        }
        let cos = LINK_TURN.to_radians().cos();
        let mut next = vec![Vec::new(); self.lanes.len()];
        for (i, a) in self.lanes.iter().enumerate() {
            let end = *a.pts.last().unwrap();
            let h = a.heading_at_end();
            let (cx, cy) = cell(end);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    for &j in starts.get(&(cx + dx, cy + dy)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let b = &self.lanes[j];
                        if j == i || b.key == a.key {
                            continue;
                        }
                        let d = b.pts[0] - end;
                        if d.truncate().length() <= LINK && d.z.abs() < 2.0 && h.dot(b.heading_at_start()) >= cos {
                            next[i].push(j);
                        }
                    }
                }
            }
        }
        for (l, n) in self.lanes.iter_mut().zip(next) {
            l.next = n;
        }
        // lane changes: the other paths of the same spline that start beside it, going the
        // same way
        let mut by_spline: HashMap<(TileKey, i64), Vec<usize>> = HashMap::new();
        for (i, l) in self.lanes.iter().enumerate().filter(|(_, l)| l.spline) {
            by_spline.entry((l.key.tile, l.key.id)).or_default().push(i);
        }
        let mut side = vec![Vec::new(); self.lanes.len()];
        for list in by_spline.values() {
            for &i in list {
                for &j in list {
                    let (a, b) = (&self.lanes[i], &self.lanes[j]);
                    if a.key.path != b.key.path && a.against == b.against && a.heading_at_start().dot(b.heading_at_start()) > 0.9 && (a.pts[0] - b.pts[0]).truncate().length() <= LANE_CHANGE_WIDTH {
                        side[i].push(j);
                    }
                }
            }
        }
        for (l, s) in self.lanes.iter_mut().zip(side) {
            l.side = s;
        }
    }
}
