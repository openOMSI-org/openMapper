//! Spline meshes built on threads of their own: the frame sends what is to be built and
//! takes what is done, so opening a big map shows it at once and filling it in does not
//! hold the window up. A result that is no longer wanted (the spline changed again, the map
//! went) is dropped.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

use glam::{DVec2, DVec3, Vec3};
use omsi_geometry::SplineCurve;
use omsi_map::MapSpline;

use super::cull::Aabb;
use super::{build_spline, SplineEntry};
use crate::assets::{Alpha, ObjectModel, SplineModel};
use crate::place::Pose;
use omsi_geometry::MeshData;
use crate::doc::TileKey;

pub type Key = (TileKey, i64);

/// A spline's entry, all of it but what lives on the GPU.
pub fn make_entry(model: &SplineModel, s: &MapSpline, org: DVec2, corner: Vec3, hash: u64) -> SplineEntry {
    let mesh = build_spline(model, s, org);
    let def = &model.def;
    // (a blended layer and wires overhead cut no ground; see openOMSI's cut_terrain)
    let overlay = !model.materials.is_empty() && model.materials.iter().all(|m| m.alpha == Alpha::Blend);
    let lowest = def.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.z).fold(f32::MAX, f32::min);
    let drivable = def.paths.iter().any(|p| p.kind == 0 || p.kind == 1);
    // (only one hanging wholly overhead stays out, unless it is a road or has height
    // profiles: openOMSI's `cuts_terrain`)
    let cuts = !overlay && (lowest < 2.0 || !def.height_profiles.is_empty() || drivable);
    let mode = s.terrain_align.map(|v| v as u8).or(s.terrain_align_flag.then_some(1));
    let rims: Vec<Vec<DVec3>> = match mode {
        Some(m) => omsi_geometry::spline_hole_rims(def, &SplineCurve::from_map(s, org), s.mirror, m).into_iter().filter(|r| !omsi_geometry::outline_crosses_itself(&r.iter().map(|v| v.truncate()).collect::<Vec<_>>())).collect(),
        None => Vec::new(),
    };
    let outlines = match mode {
        // (an outline crossing itself cuts no hole, as in Omsi.exe and openOMSI)
        Some(m) => omsi_geometry::spline_hole_outlines(def, &SplineCurve::from_map(s, org), s.mirror, m).into_iter().filter(|r| !omsi_geometry::outline_crosses_itself(r)).collect(),
        None => Vec::new(),
    };
    let mut bbox = Aabb::EMPTY;
    for p in mesh.iter().flat_map(|m| m.positions.iter()) {
        bbox.add_sphere(*p + corner, 0.0);
    }
    let bounds = if bbox.is_empty() { [0.0; 4] } else { [bbox.lo.x - corner.x, bbox.lo.y - corner.y, bbox.hi.x - corner.x, bbox.hi.y - corner.y] };
    SplineEntry {
        hash,
        mesh: None,
        cpu: mesh.map(Arc::new),
        bounds,
        bbox,
        cuts,
        drivable,
        technical: def.only_editor || !def.profiles.iter().any(|p| p.points.len() >= 2),
        hidden: false,
        outlines,
        rims,
        materials: model.materials.clone(),
        instance: 0,
        in_chunk: false,
    }
}

/// A tile's ground cut, from what its and its neighbours' roads and surfaces are.
pub struct CutInput {
    pub key: TileKey,
    pub hash: u64,
    /// The splines that cut: mesh (relative to its tile's corner), that corner, carries
    /// traffic, outlines, rims.
    pub splines: Vec<(Arc<MeshData>, DVec2, bool, Vec<Vec<DVec2>>, Vec<Vec<DVec3>>)>,
    /// Surface objects and `[terrainhole]` cutters with where they stand, and which of its
    /// cutters lie buried (see [`hole_is_buried`]).
    pub objects: Vec<(Arc<ObjectModel>, Pose, Vec<bool>)>,
    pub terrain: Option<omsi_map::Terrain>,
}

/// What a cut is: the ground's mask (rgba, `CUT` texels a side) and the walls down from the
/// holes' rims (tile-local).
pub struct CutDone {
    pub rgba: Vec<u8>,
    pub walls: Option<MeshData>,
}

pub fn make_cut(c: &CutInput) -> CutDone {
    let key = c.key;
    let ts = crate::doc::tile_size();
    let corner = DVec3::new(key.0 as f64 * ts, key.1 as f64 * ts, 0.0);
    let mut surf = omsi_geometry::TileSurface::new(super::CUT);
    let mut any = false;
    let mut rims: Vec<Vec<DVec3>> = Vec::new();
    for (m, org, drivable, outlines, srims) in &c.splines {
        surf.rasterize_kind(m, &glam::Mat4::IDENTITY, DVec3::new(org.x, org.y, 0.0), key.0, key.1, *drivable);
        any |= !outlines.is_empty() || omsi_geometry::road_cut();
        for ring in outlines {
            surf.add_outline(ring, key.0, key.1);
        }
        rims.extend(srims.iter().map(|r| r.iter().map(|v| *v - corner).collect()));
    }
    for (model, pose, buried) in &c.objects {
        for (hm, _) in model.holes.iter().zip(buried).filter(|(_, b)| !**b) {
            surf.rasterize_hole(hm, &pose.rot, pose.pos, key.0, key.1);
            for rim in omsi_geometry::hole_mesh_rims(hm, &pose.rot, pose.pos) {
                let ring: Vec<DVec2> = rim.iter().map(|v| v.truncate()).collect();
                if !omsi_geometry::outline_crosses_itself(&ring) {
                    surf.add_outline(&ring, key.0, key.1);
                    rims.push(rim.iter().map(|v| *v - corner).collect());
                }
            }
            any = true;
        }
        if model.surface {
            for p in &model.parts {
                surf.rasterize_kind(&p.mesh, &pose.rot, pose.pos, key.0, key.1, true);
            }
            any |= omsi_geometry::road_cut();
        }
    }
    // (the walls stay open where a road goes on across a hole's rim, as in openOMSI: a
    // tunnel's cutter ending at road height under a hill put a curtain of grass across the
    // carriageway. What the wheels stand on tells where the roads are)
    if !rims.is_empty() {
        for (m, org, ..) in &c.splines {
            surf.add_spline_drive(m, None, DVec3::new(org.x, org.y, 0.0), key.0, key.1);
        }
        for (model, pose, _) in c.objects.iter().filter(|o| o.0.surface) {
            for p in &model.parts {
                surf.add_drive_mesh(&p.mesh, &pose.rot, pose.pos, key.0, key.1);
            }
        }
    }
    surf.finish();
    let walls = c.terrain.as_ref().map(|t| omsi_geometry::terrain_hole_walls_with_roads(&rims, t, &surf.drive)).filter(|m| !m.indices.is_empty());
    let terrain_at = |x: f32, y: f32| c.terrain.as_ref().map(|t| t.sample(x, y)).unwrap_or(0.0);
    let rgba = if any && surf.cuts_anything(&terrain_at, super::FLUSH) { surf.mask_image(&terrain_at, super::FLUSH) } else { vec![255u8; super::CUT * super::CUT * 4] };
    CutDone { rgba, walls }
}

/// How far (m) under the lowest ground of its footprint the top of a `[terrainhole]` cutter
/// must lie for it to cut nothing: far more than a tunnel's or an underpass's roof.
const BURIED_HOLE: f64 = 10.0;

/// Whether a `[terrainhole]` cutter lies wholly that far under the ground (openOMSI's
/// `hole_is_buried`): a junction a mapper left at height zero under a hill cut a pit thirty
/// metres deep (Berlin-Spandau). Where the ground is not known it is not buried.
pub fn hole_is_buried(hole: &MeshData, pose: &Pose, ground_at: impl Fn(f64, f64) -> Option<f64>) -> bool {
    if hole.positions.is_empty() {
        return false;
    }
    let (mut top, mut ground) = (f64::NEG_INFINITY, f64::INFINITY);
    for v in &hole.positions {
        let w = pose.rot.transform_point3(*v).as_dvec3() + pose.pos;
        let Some(g) = ground_at(w.x, w.y) else { return false };
        top = top.max(w.z);
        ground = ground.min(g);
    }
    top + BURIED_HOLE < ground
}

enum Done {
    Spline(Key, u64, u64, SplineEntry),
    Cut(TileKey, u64, u64, CutDone),
}

type Work = Box<dyn FnOnce() -> Done + Send>;

pub struct Builder {
    tx: mpsc::Sender<Work>,
    rx: mpsc::Receiver<Done>,
    /// Asked for and not back yet: the revision asked for.
    pending: HashMap<Key, u64>,
    cuts: HashMap<TileKey, u64>,
    /// Counts up when the map (or where it is drawn from) changes: older results go.
    pub generation: u64,
    /// Results back and still wanted, waiting to be taken.
    splines_done: Vec<(Key, SplineEntry)>,
    cuts_done: Vec<(TileKey, u64, CutDone)>,
}

impl Builder {
    pub fn new() -> Builder {
        let (tx, jobs) = mpsc::channel::<Work>();
        let (done, rx) = mpsc::channel();
        let jobs = Arc::new(Mutex::new(jobs));
        // (a few: the asset loader has threads of its own)
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(2).clamp(1, 4);
        for _ in 0..n {
            let jobs = jobs.clone();
            let done = done.clone();
            std::thread::spawn(move || loop {
                let job = { jobs.lock().unwrap_or_else(|e| e.into_inner()).recv() };
                let Ok(work) = job else { break };
                // (a panic on an odd spline or tile leaves it out, it does not end the thread)
                let Ok(d) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) else {
                    log::warn!("a spline mesh or ground cut could not be made");
                    continue;
                };
                if done.send(d).is_err() {
                    break;
                }
            });
        }
        Builder { tx, rx, pending: HashMap::new(), cuts: HashMap::new(), generation: 0, splines_done: Vec::new(), cuts_done: Vec::new() }
    }

    /// Ask for a spline to be built at this revision (once).
    pub fn request(&mut self, key: Key, hash: u64, model: Arc<SplineModel>, spline: &MapSpline, org: DVec2, corner: Vec3) {
        if self.pending.get(&key) == Some(&hash) {
            return;
        }
        self.pending.insert(key, hash);
        let generation = self.generation;
        let spline = spline.clone();
        let _ = self.tx.send(Box::new(move || Done::Spline(key, hash, generation, make_entry(&model, &spline, org, corner, hash))));
    }

    /// Ask for a tile's ground cut. One at a time for a tile: while its roads are still
    /// coming in, what it depends on changes from frame to frame, and the cut on its way is
    /// shown when done and asked for again after.
    pub fn request_cut(&mut self, input: CutInput) {
        if self.cuts.contains_key(&input.key) {
            return;
        }
        self.cuts.insert(input.key, input.hash);
        let generation = self.generation;
        let _ = self.tx.send(Box::new(move || Done::Cut(input.key, input.hash, generation, make_cut(&input))));
    }

    /// Whether a cut of this tile is on its way.
    pub fn cut_pending(&self, key: TileKey) -> bool {
        self.cuts.contains_key(&key)
    }

    /// Forget what is on its way (the map changed).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.cuts.clear();
        self.splines_done.clear();
        self.cuts_done.clear();
        self.generation += 1;
    }

    /// Splines and cuts still being made.
    pub fn busy(&self) -> usize {
        self.pending.len() + self.cuts.len()
    }

    fn poll(&mut self) {
        while let Ok(d) = self.rx.try_recv() {
            match d {
                Done::Spline(key, hash, generation, entry) => {
                    if generation == self.generation && self.pending.get(&key) == Some(&hash) {
                        self.pending.remove(&key);
                        self.splines_done.push((key, entry));
                    }
                }
                Done::Cut(key, hash, generation, cut) => {
                    if generation == self.generation && self.cuts.get(&key) == Some(&hash) {
                        self.cuts.remove(&key);
                        self.cuts_done.push((key, hash, cut));
                    }
                }
            }
        }
    }

    /// The splines that came back and are still wanted.
    pub fn take(&mut self) -> Vec<(Key, SplineEntry)> {
        self.poll();
        std::mem::take(&mut self.splines_done)
    }

    /// The cuts that came back and are still wanted (tile, revision).
    pub fn take_cuts(&mut self) -> Vec<(TileKey, u64, CutDone)> {
        self.poll();
        std::mem::take(&mut self.cuts_done)
    }

    /// Not wanted any more (built at once, or gone).
    pub fn forget(&mut self, key: &Key) {
        self.pending.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(z: f32) -> MeshData {
        MeshData { positions: vec![Vec3::new(-5.0, -5.0, z), Vec3::new(5.0, -5.0, z), Vec3::new(5.0, 5.0, z), Vec3::new(-5.0, 5.0, z)], ..Default::default() }
    }

    #[test]
    fn a_cutter_thirty_metres_down_is_buried_and_a_shallow_one_is_not() {
        let ground = |_: f64, _: f64| Some(30.0);
        let at = |z: f64| Pose { pos: DVec3::new(100.0, 100.0, z), rot: glam::Mat4::IDENTITY };
        assert!(hole_is_buried(&square(0.0), &at(0.0), ground));
        // (an underpass or a tunnel a few metres down still cuts, and one reaching the ground)
        assert!(!hole_is_buried(&square(0.0), &at(25.0), ground));
        let mut slope = square(0.0);
        slope.positions[0].z = 30.0;
        assert!(!hole_is_buried(&slope, &at(0.0), ground));
        assert!(!hole_is_buried(&square(0.0), &at(-50.0), |_, _| None));
    }
}
