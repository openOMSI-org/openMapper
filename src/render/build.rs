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
    let mode = s.terrain_align.map(|v| v as u8).or(s.terrain_align_flag.then_some(1)).filter(|m| *m > 0);
    let curve = SplineCurve::from_map(s, org);
    let (all_rims, segments) = match mode {
        Some(m) => omsi_geometry::spline_hole_rims_with_stations(def, &curve, s.mirror, m),
        None => (Vec::new(), 0),
    };
    let rims: Vec<Vec<DVec3>> = all_rims.iter().filter(|r| !omsi_geometry::outline_crosses_itself(&r.iter().map(|v| v.truncate()).collect::<Vec<_>>())).cloned().collect();
    let outlines = match mode {
        // (an outline crossing itself cuts no hole, as in Omsi.exe and openOMSI)
        Some(m) => omsi_geometry::spline_hole_outlines(def, &curve, s.mirror, m).into_iter().filter(|r| !omsi_geometry::outline_crosses_itself(r)).collect(),
        None => Vec::new(),
    };
    // (the ground openOMSI fits under it instead of a hole, where it lies in a trough)
    let trough = mode.filter(|_| cuts && !rims.is_empty()).map(|m| {
        Arc::new(Trough {
            profile: omsi_geometry::build_spline_terrain_profile(def, &curve, s.mirror, m, &all_rims, DVec3::new(org.x, org.y, 0.0)),
            own_holes: !def.terrain_hole_profiles.is_empty(),
            segments,
        })
    });
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
        trough,
        materials: model.materials.clone(),
        instance: 0,
        in_chunk: false,
    }
}

/// What the ground under an aligned spline is fitted to (openOMSI's spline terrain profile),
/// whether its `.sli` gives its own `[terrainholeprofile]` (then it keeps its hole), and the
/// number of segments along its rims' sides.
pub struct Trough {
    pub profile: omsi_geometry::SplineTerrainProfile,
    pub own_holes: bool,
    pub segments: usize,
}

/// A spline that cuts a tile's ground.
pub struct CutSpline {
    /// Its mesh, relative to its tile's corner `org`.
    pub mesh: Arc<MeshData>,
    pub org: DVec2,
    /// It carries traffic.
    pub drivable: bool,
    /// Its `[spline_terrain_align]` outlines and rims (world), and what is fitted instead.
    pub outlines: Vec<Vec<DVec2>>,
    pub rims: Vec<Vec<DVec3>>,
    pub trough: Option<Arc<Trough>>,
}

/// A tile's ground cut, from what its and its neighbours' roads and surfaces are.
pub struct CutInput {
    pub key: TileKey,
    pub hash: u64,
    pub splines: Vec<CutSpline>,
    /// Surface objects and `[terrainhole]` cutters with where they stand, and which of its
    /// cutters lie buried (see [`hole_is_buried`]).
    pub objects: Vec<(Arc<ObjectModel>, Pose, Vec<bool>)>,
    pub terrain: Option<omsi_map::Terrain>,
    /// The ground of the tiles around, with their corners relative to this one's.
    pub neighbours: Vec<(omsi_map::Terrain, DVec2)>,
}

/// What a cut is: the ground's mask (rgba, `CUT` texels a side), the walls down from the
/// holes' rims, and the ground fitted to the aligned splines lying in troughs (tile-local).
pub struct CutDone {
    pub rgba: Vec<u8>,
    pub walls: Option<MeshData>,
    pub fitted: Option<MeshData>,
}

pub fn make_cut(c: &CutInput) -> CutDone {
    let key = c.key;
    let ts = crate::doc::tile_size();
    let corner = DVec3::new(key.0 as f64 * ts, key.1 as f64 * ts, 0.0);
    let neighbours: Vec<(&omsi_map::Terrain, DVec2)> = c.neighbours.iter().map(|(t, o)| (t, *o)).collect();
    let mut surf = omsi_geometry::TileSurface::new(super::CUT);
    let mut any = false;
    let mut rims: Vec<Vec<DVec3>> = Vec::new();
    // (the aligned splines the ground is fitted to instead of cut, as openOMSI does since
    // 0.3.42: one in a trough; one in a cutting - the hill more than a metre over its rim -
    // or whose `.sli` gives its own hole keeps Omsi.exe's hole and soil walls)
    let mut troughs: Vec<(&omsi_geometry::SplineTerrainProfile, DVec3)> = Vec::new();
    let mut fitted: Vec<&CutSpline> = Vec::new();
    for sp in &c.splines {
        let origin = DVec3::new(sp.org.x, sp.org.y, 0.0);
        surf.rasterize_kind(&sp.mesh, &glam::Mat4::IDENTITY, origin, key.0, key.1, sp.drivable);
        any |= omsi_geometry::road_cut();
        if let (Some(t), Some(terrain)) = (&sp.trough, &c.terrain) {
            if !t.own_holes && !omsi_geometry::profile_in_cutting(&t.profile, origin - corner, terrain, &neighbours) {
                troughs.push((&t.profile, origin - corner));
                fitted.push(sp);
                continue;
            }
        }
        any |= !sp.outlines.is_empty();
        for ring in &sp.outlines {
            surf.add_outline(ring, key.0, key.1);
        }
        rims.extend(sp.rims.iter().map(|r| r.iter().map(|v| *v - corner).collect()));
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
    if !rims.is_empty() || !fitted.is_empty() {
        for sp in &c.splines {
            surf.add_spline_drive(&sp.mesh, None, DVec3::new(sp.org.x, sp.org.y, 0.0), key.0, key.1);
        }
        for (model, pose, _) in c.objects.iter().filter(|o| o.0.surface) {
            for p in &model.parts {
                surf.add_drive_mesh(&p.mesh, &pose.rot, pose.pos, key.0, key.1);
            }
        }
    }
    surf.finish();
    let fitted_ground = match &c.terrain {
        Some(terrain) if !troughs.is_empty() => {
            // (the road surfaces bound the fitted ground; the troughs' own backing does not)
            let roads: Vec<(&MeshData, DVec3)> = c.splines.iter().filter(|s| s.drivable || s.trough.is_none()).map(|s| (&*s.mesh, DVec3::new(s.org.x, s.org.y, 0.0) - corner)).collect();
            match omsi_geometry::try_build_fitted_terrain_mesh_with_roads(terrain, &troughs, &neighbours, &roads) {
                Ok(m) => Some(m),
                Err(e) => {
                    // (the saved ground, with the holes of the splines it comes up over)
                    log::warn!("tile {key:?}: cannot fit the ground to its aligned splines: {e}");
                    let raster = &surf;
                    let covered: Vec<&Vec<DVec3>> = fitted
                        .iter()
                        .flat_map(|sp| {
                            let segments = sp.trough.as_ref().map(|t| t.segments).unwrap_or(0);
                            sp.rims.iter().filter(move |r| ground_over_spline(raster, terrain, r, segments, corner))
                        })
                        .collect();
                    for rim in covered {
                        surf.add_outline(&rim.iter().map(|v| v.truncate()).collect::<Vec<_>>(), key.0, key.1);
                        rims.push(rim.iter().map(|v| *v - corner).collect());
                        any = true;
                    }
                    None
                }
            }
        }
        _ => None,
    };
    let walls = c.terrain.as_ref().map(|t| omsi_geometry::terrain_hole_walls_with_roads(&rims, t, &surf.drive)).filter(|m| !m.indices.is_empty());
    let terrain_at = |x: f32, y: f32| c.terrain.as_ref().map(|t| t.sample(x, y)).unwrap_or(0.0);
    let rgba = if any && surf.cuts_anything(&terrain_at, super::FLUSH) { surf.mask_image(&terrain_at, super::FLUSH) } else { vec![255u8; super::CUT * super::CUT * 4] };
    CutDone { rgba, walls, fitted: fitted_ground }
}

/// How far (m) the ground may stand over an aligned spline before its hole is cut after all
/// where the fit failed (openOMSI's `GROUND_OVER_SPLINE`).
const GROUND_OVER_SPLINE: f32 = 0.05;

/// Whether the saved ground comes up over an aligned spline anywhere across its rim (a ditch,
/// a trough), asked at a quarter, half and three quarters of the way across at every station
/// of the rim on this tile; true where no point of it can be asked (openOMSI's
/// `ground_over_spline`).
fn ground_over_spline(surf: &omsi_geometry::TileSurface, terrain: &omsi_map::Terrain, rim: &[DVec3], segments: usize, corner: DVec3) -> bool {
    if segments == 0 || rim.len() < 2 * segments + 2 {
        return true;
    }
    // (the closed ring runs right, far cap, left, near cap)
    let inner = segments - 1;
    let cap = rim.len() / 2 - inner;
    let near_left = rim.len() - cap;
    let pairs = std::iter::once((rim[rim.len() - 1], rim[near_left])).chain((0..=inner).map(|i| (rim[i], rim[near_left - 1 - i])));
    let side = crate::doc::tile_size();
    let mut asked = false;
    for p in pairs.flat_map(|(a, b)| [a.lerp(b, 0.25), a.lerp(b, 0.5), a.lerp(b, 0.75)]) {
        let l = p - corner;
        if !(0.0..=side).contains(&l.x) || !(0.0..=side).contains(&l.y) {
            continue;
        }
        let Some((x, y, surface)) = surf.sample_at_texel_center(l.x as f32, l.y as f32) else { continue };
        asked = true;
        if terrain.sample(x, y) > surface + GROUND_OVER_SPLINE {
            return true;
        }
    }
    !asked
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

    /// A cut of tile (0, 0) with one aligned road 8 m wide running north across it at height
    /// 0, its ground everywhere at `ground`.
    fn aligned_road_cut(ground: f32) -> CutDone {
        let model = SplineModel {
            def: omsi_scenery::Spline {
                profiles: vec![omsi_scenery::SplineProfile {
                    texture: 0,
                    points: vec![omsi_scenery::SplineProfilePoint { x: -4.0, z: 0.0, ..Default::default() }, omsi_scenery::SplineProfilePoint { x: 4.0, z: 0.0, ..Default::default() }],
                }],
                ..Default::default()
            },
            materials: Vec::new(),
            half_width: 4.0,
        };
        let s = MapSpline { file: "Splines\\road.sli".into(), id: 1, pos: [150.0, 100.0, 0.0], length: 100.0, terrain_align_flag: true, ..Default::default() };
        let e = make_entry(&model, &s, DVec2::ZERO, Vec3::ZERO, 0);
        assert!(e.trough.is_some() && !e.rims.is_empty());
        let mut terrain = omsi_map::Terrain::flat();
        terrain.heights.iter_mut().for_each(|h| *h = ground);
        let spline = CutSpline { mesh: e.cpu.clone().unwrap(), org: DVec2::ZERO, drivable: e.drivable, outlines: e.outlines.clone(), rims: e.rims.clone(), trough: e.trough.clone() };
        make_cut(&CutInput { key: (0, 0), hash: 0, splines: vec![spline], objects: Vec::new(), terrain: Some(terrain), neighbours: Vec::new() })
    }

    /// An aligned road lying on the ground gets the ground fitted under it and cuts no hole,
    /// as in openOMSI; one in a cutting (the hill 5 m over it) keeps Omsi.exe's hole and walls.
    #[test]
    fn an_aligned_road_in_a_trough_has_ground_fitted_and_one_in_a_cutting_a_hole() {
        let _map = crate::doc::one_map();
        let trough = aligned_road_cut(0.0);
        assert!(trough.fitted.is_some_and(|m| !m.indices.is_empty()));
        assert!(trough.rgba.iter().all(|&b| b == 255) && trough.walls.is_none());
        let cutting = aligned_road_cut(5.0);
        assert!(cutting.fitted.is_none());
        assert!(cutting.rgba.chunks_exact(4).any(|p| p[3] == 0) && cutting.walls.is_some());
    }
}
