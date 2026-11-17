//! Junctions made on the spot, as Transport Fever makes them: where a new road meets or
//! crosses another, both are cut back from the meeting point and a scenery object of its own
//! fills the gap. The object is ordinary OMSI content, written into openOMSI's content folder
//! (`Sceneryobjects\openMapper\<map>\junction_<id>.sco`): a flat asphalt plate (a DirectX
//! `.x` mesh) and a `[path]` for every way through it - each lane coming in joined to the
//! lanes going out of the other arms (straight on, right, left: the AI follows them as it
//! follows any crossing's paths, which openOMSI links to the roads' lanes where their ends
//! meet), and footpaths round its corners.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DVec2, DVec3};
use omsi_geometry::SplineCurve;
use omsi_map::MapObject;

use crate::assets::{Assets, SplineModel};
use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::roads::{self, Seg};

/// One road running into the junction: the spline whose end (or start) is at its edge.
#[derive(Debug, Clone, Copy)]
pub struct Arm {
    pub key: TileKey,
    pub id: i64,
    /// The spline's end lies at the junction (else its start).
    pub at_end: bool,
}

/// A lane end at the junction's edge (world).
#[derive(Debug, Clone, Copy)]
struct LaneEnd {
    arm: usize,
    pos: DVec3,
    /// The heading traffic moves with there (degrees).
    flow: f64,
    kind: i32,
    width: f64,
    /// Lateral place, right of the arm's way into the junction (m).
    side: f64,
    /// Traffic comes into the junction here (else it leaves; footways: both).
    incoming: bool,
}

/// Room a junction needs round its centre for these roads (m): the widest of them plus
/// a margin for the curves.
pub fn radius(half_widths: &[f64]) -> f64 {
    half_widths.iter().cloned().fold(3.0, f64::max) * 1.15 + 4.0
}

/// Room a junction needs when its roads meet at `angle` degrees (the sharper of the two
/// angles between them): at a sharp corner the kerbs meet far out from the centre, and the
/// arms must reach past that by enough for the kerb to be rounded off wider than the
/// footway beside it.
pub fn radius_at(half_widths: &[f64], angle: f64) -> f64 {
    let hw = half_widths.iter().cloned().fold(3.0, f64::max);
    let half = (angle.abs().rem_euclid(180.0).min(180.0 - angle.abs().rem_euclid(180.0)).max(20.0) * 0.5).to_radians();
    let base = radius(half_widths);
    base.max((hw + 2.5) / half.tan()).min(base * 3.0)
}

/// The arms' geometry: where each ends, which way it goes in, its half width and lanes.
fn arm_ends(doc: &Document, assets: &mut Assets, arms: &[Arm]) -> Option<(Vec<(DVec3, f64, f64, String)>, Vec<LaneEnd>)> {
    let mut ends = Vec::new();
    let mut lanes = Vec::new();
    for (ai, a) in arms.iter().enumerate() {
        let s = doc.spline(a.key, a.id)?;
        let model = assets.spline(&s.file)?;
        let c = SplineCurve::from_map(s, tile_origin(a.key));
        let at = if a.at_end { s.length } else { 0.0 };
        let into = if a.at_end { c.heading_at(at) } else { c.heading_at(at) + 180.0 };
        let p = c.point_at(at);
        ends.push((p, roads::norm_deg(into), model.half_width as f64, s.file.clone()));
        let mirror = if s.mirror { -1.0 } else { 1.0 };
        lanes.extend(lanes_at(&model, mirror, ai, a.at_end, |x, z| c.offset_point(at, x, z), c.heading_at(at)));
    }
    Some((ends, lanes))
}

/// The lanes of a road type where it meets a junction: `place(x, z)` is where a lane at
/// lateral place `x` (and height `z`) ends, `along` the heading of the road's own way there.
fn lanes_at(model: &SplineModel, mirror: f64, ai: usize, at_end: bool, place: impl Fn(f64, f64) -> DVec3, along: f64) -> Vec<LaneEnd> {
    let mut lanes = Vec::new();
    for path in &model.def.paths {
        let x = path.start[0] as f64 * mirror;
        let z = path.start[2] as f64;
        let pos = place(x, z);
        // along the spline: 0 forwards, 1 backwards, 2 both ways; (incoming, flow)
        let ways: Vec<(bool, f64)> = match path.direction {
            0 => vec![(at_end, along)],
            1 => vec![(!at_end, along + 180.0)],
            _ => vec![(true, if at_end { along } else { along + 180.0 }), (false, if at_end { along + 180.0 } else { along })],
        };
        // lateral place seen from the arm's way into the junction
        let side = if at_end { x } else { -x };
        for (incoming, flow) in ways {
            lanes.push(LaneEnd { arm: ai, pos, flow: roads::norm_deg(flow), kind: path.kind, width: path.width as f64, side, incoming });
        }
    }
    lanes
}

/// One `[path]` record (object frame = world less the junction's centre).
struct PathRec {
    seg: Seg,
    z0: f64,
    z1: f64,
    kind: i32,
    width: f64,
    direction: i32,
    blinker: i32,
    /// The light its traffic waits at (`[use_traffic_light]`), if any.
    light: Option<usize>,
}

/// What a junction has besides its roads: traffic lights (a plan of two phases, the main
/// road's and the side roads'), and zebra crossings on some of its arms. Kept in the
/// junction's own `.sco` as a comment line, so it is made again with them.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub lights: bool,
    /// Green for the main road and for the side roads (s).
    pub main_green: f64,
    pub side_green: f64,
    /// The arms with a zebra crossing, by their heading out of the junction (degrees).
    pub crossings: Vec<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Options { lights: false, main_green: 30.0, side_green: 20.0, crossings: Vec::new() }
    }
}

const OPTIONS_TAG: &str = "openMapper junction:";

impl Options {
    fn line(&self) -> String {
        let c: Vec<String> = self.crossings.iter().map(|h| format!("{h:.1}")).collect();
        // (helpers=omsi: its spline helpers are in OMSI's order, x height forward)
        format!("{OPTIONS_TAG} lights={} main={:.0} side={:.0} crossings={} helpers=omsi", self.lights as i32, self.main_green, self.side_green, c.join(","))
    }

    /// The options written in a junction's `.sco` text (defaults when none are).
    pub fn parse(sco: &str) -> Options {
        let mut o = Options::default();
        let Some(line) = sco.lines().find(|l| l.trim_start().starts_with(OPTIONS_TAG)) else { return o };
        for kv in line.trim_start()[OPTIONS_TAG.len()..].split_whitespace() {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match k {
                "lights" => o.lights = v == "1",
                "main" => o.main_green = v.parse().unwrap_or(o.main_green),
                "side" => o.side_green = v.parse().unwrap_or(o.side_green),
                "crossings" => o.crossings = v.split(',').filter_map(|x| x.parse().ok()).collect(),
                _ => {}
            }
        }
        o
    }

    /// Read them from a junction's `.sco` file.
    pub fn read(path: &Path) -> Options {
        std::fs::read(path).ok().map(|b| Options::parse(&omsi_cfg::decode_text(&b))).unwrap_or_default()
    }

    /// Whether the arm going out along `out` has a crossing.
    pub fn crossing_at(&self, out: f64) -> bool {
        self.crossings.iter().any(|h| (roads::norm_deg(h - out + 180.0) - 180.0).abs() < 20.0)
    }
}

/// The two arms most nearly in line (the main road through the junction).
fn main_road(outs: &[f64]) -> (usize, usize) {
    let mut best = (0, 1, -1.0);
    for i in 0..outs.len() {
        for j in i + 1..outs.len() {
            let d = (roads::norm_deg(outs[i] - outs[j] + 180.0) - 180.0).abs();
            if d > best.2 {
                best = (i, j, d);
            }
        }
    }
    (best.0, best.1)
}

/// The light program: light 0 the main road's, light 1 the side roads': red-yellow 2 s,
/// green, yellow 3 s, then red, with 2 s of all red between them.
fn light_program(o: &Options) -> String {
    let (gm, gs) = (o.main_green.max(5.0), o.side_green.max(5.0));
    let main_turn = 2.0 + gm + 3.0 + 2.0;
    let cycle = main_turn + 2.0 + gs + 3.0 + 2.0;
    let mut t = format!("[traffic_lights_group]\n{}\n\n", num(cycle));
    let _ = write!(t, "[traffic_light]\nMain road\n\n[phase]\n3\n2\n\n[phase]\n6\n{}\n\n[phase]\n9\n3\n\n[phase]\n0\n0\n\n", num(gm));
    let _ = write!(t, "[traffic_light]\nSide roads\n\n[phase]\n0\n{}\n\n[phase]\n3\n2\n\n[phase]\n6\n{}\n\n[phase]\n9\n3\n\n[phase]\n0\n0\n\n", num(main_turn), num(gs));
    t
}

/// The signal heads and their poles of a junction with lights: one at each arm traffic
/// comes in by, on its right (left where traffic keeps left), facing it.
pub const LIGHT_HEAD: &str = "Sceneryobjects\\Verkehrszeichen_MC\\Ampel_Kfz_1.sco";
pub const LIGHT_POLE: &str = "Sceneryobjects\\Verkehrszeichen_MC\\Mast_Ampel_klein_1.sco";

fn signed_turn(from: f64, to: f64) -> f64 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// The ways through the junction.
fn paths(lanes: &[LaneEnd], centre: DVec3, light_of: &[Option<usize>]) -> Vec<PathRec> {
    let mut out = Vec::new();
    let c2 = centre.truncate();
    let arms = lanes.iter().map(|l| l.arm).max().map(|m| m + 1).unwrap_or(0);
    // traffic: every lane coming in to the lanes going out of the other arms, matched by
    // their place counted from the right (right turns from the right, left turns from the left)
    for i in 0..arms {
        let mut ins: Vec<&LaneEnd> = lanes.iter().filter(|l| l.arm == i && l.kind == 0 && l.incoming).collect();
        ins.sort_by(|a, b| b.side.total_cmp(&a.side));
        for j in (0..arms).filter(|j| *j != i) {
            let mut outs: Vec<&LaneEnd> = lanes.iter().filter(|l| l.arm == j && l.kind == 0 && !l.incoming).collect();
            // (seen leaving: its right is the arm's left coming in)
            outs.sort_by(|a, b| a.side.total_cmp(&b.side));
            if ins.is_empty() || outs.is_empty() {
                continue;
            }
            let turn = signed_turn(ins[0].flow, outs[0].flow);
            if turn.abs() > 150.0 {
                continue;
            }
            let blinker = if turn > 30.0 { 3 } else if turn < -30.0 { 2 } else { 0 };
            let pairs: Vec<(&LaneEnd, &LaneEnd)> = if blinker == 3 {
                vec![(ins[0], outs[0])]
            } else if blinker == 2 {
                vec![(ins[ins.len() - 1], outs[outs.len() - 1])]
            } else {
                (0..ins.len().max(outs.len())).map(|k| (ins[k.min(ins.len() - 1)], outs[k.min(outs.len() - 1)])).collect()
            };
            for (a, b) in pairs {
                let pa = a.pos.truncate() - c2;
                let pb = b.pos.truncate() - c2;
                let segs = roads::biarc(pa, a.flow, pb, b.flow).or_else(|| roads::straight(pa, pb).map(|s| vec![s]));
                let Some(segs) = segs else { continue };
                let total: f64 = segs.iter().map(|s| s.length).sum::<f64>().max(0.01);
                let mut acc = 0.0;
                for (n, s) in segs.into_iter().enumerate() {
                    let z0 = a.pos.z + (b.pos.z - a.pos.z) * acc / total - centre.z;
                    acc += s.length;
                    let z1 = a.pos.z + (b.pos.z - a.pos.z) * acc / total - centre.z;
                    // (traffic waits where it comes in: the light is on the way's first piece)
                    let light = if n == 0 { light_of.get(i).copied().flatten() } else { None };
                    out.push(PathRec { seg: s, z0, z1, kind: 0, width: a.width.max(2.5), direction: 0, blinker, light });
                }
            }
        }
    }
    // footways round the corners: each arm's outermost footway lane on a side to the
    // nearest one of the next arm round the junction
    let mut order: Vec<usize> = (0..arms).collect();
    let angle = |i: usize| -> f64 {
        let l = lanes.iter().find(|l| l.arm == i);
        l.map(|l| { let d = l.pos.truncate() - c2; d.y.atan2(d.x) }).unwrap_or(0.0)
    };
    order.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
    if arms >= 2 {
        for k in 0..arms {
            let (i, j) = (order[k], order[(k + 1) % arms]);
            if arms == 2 && k == 1 {
                break;
            }
            let walk_i: Vec<&LaneEnd> = lanes.iter().filter(|l| l.arm == i && l.kind == 1).collect();
            let walk_j: Vec<&LaneEnd> = lanes.iter().filter(|l| l.arm == j && l.kind == 1).collect();
            let mut best: Option<(f64, &LaneEnd, &LaneEnd)> = None;
            for a in &walk_i {
                for b in &walk_j {
                    let d = (a.pos - b.pos).length();
                    if best.map(|x| d < x.0).unwrap_or(true) {
                        best = Some((d, a, b));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let pa = a.pos.truncate() - c2;
                let pb = b.pos.truncate() - c2;
                if let Some(s) = roads::straight(pa, pb) {
                    out.push(PathRec { seg: s, z0: a.pos.z - centre.z, z1: b.pos.z - centre.z, kind: 1, width: a.width.max(1.5), direction: 2, blinker: 0, light: None });
                }
            }
        }
    }
    out
}

/// One side of a road's cross-section beyond its kerb line (kerb, footway...): the
/// points in order outward, as (distance past the kerb line, height, u, v scale).
#[derive(Clone)]
struct SideProfile {
    texture: Option<PathBuf>,
    points: Vec<(f64, f64, f64, f64)>,
}

/// An arm as the junction's mesh needs it, seen from the junction looking out along it.
#[derive(Clone)]
struct ArmGeo {
    end: DVec3,
    out: f64,
    /// Roadway edges (the kerb lines) to the right and left of the way out (m).
    right_w: f64,
    left_w: f64,
    #[allow(dead_code)]
    right: Vec<SideProfile>,
    left: Vec<SideProfile>,
    /// Texture phase where the arm meets the junction, and which way it runs into it.
    v0: f64,
    vdir: f64,
    /// The roadway's own texture and where its lane (no markings) lies: (texture, u of a
    /// lane's middle, the roadway's u range, metres per repeat along the road).
    road_tex: Option<(PathBuf, f64, f64, f64, f64, f64)>,
    /// How high the roadway's surface lies over the spline's own height (m): the profile a
    /// lane runs on, where roads are drawn above the line they are laid along.
    road_z: f64,
}

fn arm_geometry(doc: &Document, assets: &mut Assets, a: &Arm) -> Option<ArmGeo> {
    let s = doc.spline(a.key, a.id)?.clone();
    let model = assets.spline(&s.file)?;
    let c = SplineCurve::from_map(&s, tile_origin(a.key));
    let at = if a.at_end { s.length } else { 0.0 };
    let end = c.point_at(at);
    let out = roads::norm_deg(if a.at_end { c.heading_at(at) + 180.0 } else { c.heading_at(at) });
    Some(arm_geo_of(&model, s.mirror, a.at_end, end, out, at + s.tex_offset))
}

/// An arm's geometry from its road type: the spline's end at the junction (`at_end`: it is
/// the spline's end, else its start), `out` the heading out along the road, `v_at` how far
/// along its texture runs there.
fn arm_geo_of(model: &SplineModel, mirrored: bool, at_end: bool, end: DVec3, out: f64, v_at: f64) -> ArmGeo {
    let m = if mirrored { -1.0 } else { 1.0 };
    let profs: Vec<(usize, Vec<(f64, f64, f64, f64)>)> = model.def.profiles.iter().filter(|p| p.points.len() >= 2).map(|p| (p.texture, p.points.iter().map(|q| (q.x as f64 * m, q.z as f64, q.u as f64, q.v_scale as f64)).collect())).collect();
    // the roadway: where the spline's street lanes run (its `[path]`s say it: a half of a
    // divided road has its centre line on the median's edge, with grass on one side); else
    // the profiles that reach the centre line; else the whole width
    let lanes: Vec<(f64, f64)> = model.def.paths.iter().filter(|q| q.kind == 0).map(|q| (q.start[0] as f64 * m, q.width as f64)).collect();
    let all_lo = profs.iter().flat_map(|p| p.1.iter()).map(|q| q.0).fold(0.0, f64::min);
    let all_hi = profs.iter().flat_map(|p| p.1.iter()).map(|q| q.0).fold(0.0, f64::max);
    let touches = |pts: &Vec<(f64, f64, f64, f64)>| pts.iter().map(|q| q.0).fold(f64::MAX, f64::min) <= 0.05 && pts.iter().map(|q| q.0).fold(f64::MIN, f64::max) >= -0.05;
    let span = |pts: &Vec<(f64, f64, f64, f64)>| (pts.iter().map(|q| q.0).fold(f64::MAX, f64::min), pts.iter().map(|q| q.0).fold(f64::MIN, f64::max));
    let (xl, xr) = if !lanes.is_empty() {
        let lo = lanes.iter().map(|(x, w)| x - w / 2.0).fold(f64::MAX, f64::min);
        let hi = lanes.iter().map(|(x, w)| x + w / 2.0).fold(f64::MIN, f64::max);
        // (out to the edges of the profiles the lanes run on: the asphalt often reaches
        // further than the lanes, to a gutter or a parking strip, and a kerb is where it ends)
        let under: Vec<(f64, f64)> = profs.iter().map(|p| span(&p.1)).filter(|(a, b)| *b > lo + 0.05 && *a < hi - 0.05).collect();
        (under.iter().map(|s| s.0).fold(lo, f64::min), under.iter().map(|s| s.1).fold(hi, f64::max))
    } else {
        let road: Vec<&(usize, Vec<(f64, f64, f64, f64)>)> = profs.iter().filter(|p| touches(&p.1)).collect();
        if road.is_empty() {
            (all_lo, all_hi)
        } else {
            (road.iter().flat_map(|p| p.1.iter()).map(|q| q.0).fold(0.0, f64::min), road.iter().flat_map(|p| p.1.iter()).map(|q| q.0).fold(0.0, f64::max))
        }
    };
    let inside = |pts: &Vec<(f64, f64, f64, f64)>| pts.iter().all(|q| q.0 >= xl - 0.05 && q.0 <= xr + 0.05);
    // (the lane on the right: its texture is the asphalt the junction is laid with, taken
    // from the middle of a lane, where no markings are)
    let lane_mid = lanes.iter().map(|(x, _)| *x).find(|x| *x > xl && *x < xr).unwrap_or((xl + xr) * 0.5);
    let central = profs.iter().filter(|p| inside(&p.1)).find(|p| p.1.iter().any(|q| q.0 <= lane_mid) && p.1.iter().any(|q| q.0 >= lane_mid)).or_else(|| profs.iter().find(|p| inside(&p.1)));
    let side = |right: bool| -> Vec<SideProfile> {
        profs
            .iter()
            .filter(|(_, pts)| if right { pts.iter().all(|q| q.0 >= xr - 0.05) } else { pts.iter().all(|q| q.0 <= xl + 0.05) })
            .map(|(t, pts)| {
                let mut points: Vec<(f64, f64, f64, f64)> = pts.iter().map(|q| ((if right { q.0 - xr } else { xl - q.0 }).max(0.0), q.1, q.2, q.3)).collect();
                if points.first().map(|p| p.0).unwrap_or(0.0) > points.last().map(|p| p.0).unwrap_or(0.0) {
                    points.reverse();
                }
                SideProfile { texture: model.materials.get(*t).and_then(|m| m.texture.clone()), points }
            })
            .collect()
    };
    let (spline_right, spline_left) = (side(true), side(false));
    // (looking out along an arm whose end is at the junction, the spline's right is our left)
    let (right, left, right_w, left_w) = if at_end { (spline_left, spline_right, -xl, xr) } else { (spline_right, spline_left, xr, -xl) };
    let vs = central.map(|c| c.1[0].3).unwrap_or(0.2).abs().max(1e-3);
    let v0 = vs * v_at * m;
    let vdir = if at_end { m } else { -m };
    let road_tex = central.and_then(|(t, pts)| {
        let tex = model.materials.get(*t)?.texture.clone()?;
        // the u in the middle of a lane
        let x = lane_mid;
        let mut u = pts[0].2;
        for w in pts.windows(2) {
            if (w[0].0 - x) * (w[1].0 - x) <= 0.0 && (w[1].0 - w[0].0).abs() > 1e-6 {
                u = w[0].2 + (w[1].2 - w[0].2) * (x - w[0].0) / (w[1].0 - w[0].0);
            }
        }
        let range = (pts.iter().map(|q| q.2).fold(f64::MIN, f64::max) - pts.iter().map(|q| q.2).fold(f64::MAX, f64::min)).abs();
        let width = (pts.iter().map(|q| q.0).fold(f64::MIN, f64::max) - pts.iter().map(|q| q.0).fold(f64::MAX, f64::min)).abs().max(0.5);
        // (a lane's width: the street lanes' own, else half the roadway)
        let lane = model.def.paths.iter().filter(|p| p.kind == 0).map(|p| p.width as f64).fold(0.0, f64::max);
        let lane = if lane > 1.5 { lane / 0.9 } else { width * 0.5 };
        Some((tex, u, range, 1.0 / vs, width, lane))
    });
    let road_z = central
        .map(|(_, pts)| {
            let x = lane_mid;
            let mut z = pts[0].1;
            for w in pts.windows(2) {
                if (w[0].0 - x) * (w[1].0 - x) <= 0.0 && (w[1].0 - w[0].0).abs() > 1e-6 {
                    z = w[0].1 + (w[1].1 - w[0].1) * (x - w[0].0) / (w[1].0 - w[0].0);
                }
            }
            z
        })
        .unwrap_or(0.0);
    ArmGeo { end, out, right_w, left_w, right, left, v0, vdir, road_tex, road_z }
}

/// The junction's mesh: vertices (relative to the centre: position, normal, uv), triangles
/// with their material, and the materials' textures.
#[derive(Default)]
struct Mesh {
    verts: Vec<([f64; 3], [f64; 3], [f64; 2])>,
    tris: Vec<([usize; 3], usize)>,
    textures: Vec<Option<PathBuf>>,
    /// The roadway's own triangles (the fan round the middle, in the vertices' frame): where
    /// the paint goes, and at what height.
    roadway: Vec<[DVec3; 3]>,
}

impl Mesh {
    /// The roadway's height at `p` (the vertices' frame), None off it.
    fn roadway_z(&self, p: DVec2) -> Option<f64> {
        self.roadway.iter().find_map(|t| {
            let (a, b, c) = (t[0].truncate(), t[1].truncate(), t[2].truncate());
            let den = (b - a).perp_dot(c - a);
            if den.abs() < 1e-12 {
                return None;
            }
            let wb = (p - a).perp_dot(c - a) / den;
            let wc = (b - a).perp_dot(p - a) / den;
            let wa = 1.0 - wb - wc;
            (wa >= -1e-6 && wb >= -1e-6 && wc >= -1e-6).then(|| t[0].z * wa + t[1].z * wb + t[2].z * wc)
        })
    }

    /// A strip of paint `width` wide along `a` to `b` (the vertices' frame) laid on the roadway
    /// as a decal: cut into the pieces each roadway triangle holds, each piece on that
    /// triangle's own plane `MARK_LIFT` over it. It follows every fold of a plate whose arms
    /// lie at different heights, and what falls off the roadway is left out.
    fn paint_strip(&mut self, a: DVec2, b: DVec2, width: f64, mat: usize) {
        let len = (b - a).length();
        if len < 1e-3 {
            return;
        }
        let along = (b - a) / len;
        let across = DVec2::new(-along.y, along.x) * (width * 0.5);
        let rect = [a - across, b - across, b + across, a + across];
        let mut pieces = Vec::new();
        for t in &self.roadway {
            let (p0, p1, p2) = (t[0].truncate(), t[1].truncate(), t[2].truncate());
            let den = (p1 - p0).perp_dot(p2 - p0);
            if den.abs() < 1e-9 {
                continue;
            }
            // (the rectangle cut by each side of the triangle in turn, keeping the inside)
            let mut poly: Vec<DVec2> = rect.to_vec();
            for (e0, e1) in [(p0, p1), (p1, p2), (p2, p0)] {
                let inside = |p: DVec2| (e1 - e0).perp_dot(p - e0) * den.signum() >= 0.0;
                let mut next = Vec::with_capacity(poly.len() + 1);
                for k in 0..poly.len() {
                    let (c, d) = (poly[k], poly[(k + 1) % poly.len()]);
                    let (ci, di) = (inside(c), inside(d));
                    if ci {
                        next.push(c);
                    }
                    if ci != di {
                        let (fc, fd) = ((e1 - e0).perp_dot(c - e0), (e1 - e0).perp_dot(d - e0));
                        next.push(c + (d - c) * (fc / (fc - fd)));
                    }
                }
                poly = next;
                if poly.len() < 3 {
                    break;
                }
            }
            if poly.len() < 3 {
                continue;
            }
            let z = |p: DVec2| {
                let wb = (p - p0).perp_dot(p2 - p0) / den;
                let wc = (p1 - p0).perp_dot(p - p0) / den;
                t[0].z * (1.0 - wb - wc) + t[1].z * wb + t[2].z * wc + MARK_LIFT
            };
            pieces.push(poly.iter().map(|p| DVec3::new(p.x, p.y, z(*p))).collect::<Vec<_>>());
        }
        for poly in pieces {
            for k in 1..poly.len() - 1 {
                self.tri([poly[0], poly[k], poly[k + 1]], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], DVec3::Z, mat);
            }
        }
    }

    fn material(&mut self, tex: Option<PathBuf>) -> usize {
        match self.textures.iter().position(|t| *t == tex) {
            Some(i) => i,
            None => {
                self.textures.push(tex);
                self.textures.len() - 1
            }
        }
    }

    /// A triangle facing `n` (the vertices are turned round if they face the other way).
    fn tri(&mut self, p: [DVec3; 3], uv: [[f64; 2]; 3], n: DVec3, mat: usize) {
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        if face.length() < 1e-9 {
            return;
        }
        let order = if face.dot(n) >= 0.0 { [0, 1, 2] } else { [0, 2, 1] };
        let base = self.verts.len();
        let n = n.normalize_or_zero();
        for i in order {
            self.verts.push((p[i].to_array(), n.to_array(), uv[i]));
        }
        self.tris.push(([base, base + 1, base + 2], mat));
    }
}

/// Build the mesh: the roadway in the middle, and round each corner the kerb and footway
/// of the roads, swept along a smooth curve from one arm's kerb line to the next's.
/// `along`: the heading the roadway's texture runs (the widest road's), so its slabs and
/// patches lie as the road's do, not slanted across it.
fn junction_mesh(arms: &[ArmGeo], centre: DVec3, core_tex: Option<PathBuf>, period: f64, along: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let core = mesh.material(core_tex);
    let rel = |p: DVec3| p - centre;
    // the arms round the junction, counter-clockwise
    let mut order: Vec<usize> = (0..arms.len()).collect();
    // (by the way each runs out, and then by where it ends: two arms running out side by side,
    // a merge's main road and slip road, come round in the order they lie)
    let far = arms.iter().map(|a| (a.end - centre).truncate().length()).fold(0.0, f64::max) * 2.0 + 1.0;
    let ang = |i: usize| { let d = roads::dir(arms[i].out) * far + (arms[i].end - centre).truncate(); d.y.atan2(d.x) };
    order.sort_by(|a, b| ang(*a).total_cmp(&ang(*b)));
    let mut outline: Vec<DVec3> = Vec::new();
    for k in 0..order.len() {
        let a = &arms[order[k]];
        let b = &arms[order[(k + 1) % order.len()]];
        let r_pt = a.end.truncate() + roads::right(a.out) * a.right_w;
        let l_pt = a.end.truncate() + roads::right(a.out) * -a.left_w;
        outline.push(DVec3::new(r_pt.x, r_pt.y, a.end.z));
        outline.push(DVec3::new(l_pt.x, l_pt.y, a.end.z));
        // the kerb from this arm's left kerb line round to the next arm's right one
        let from = l_pt;
        let to = b.end.truncate() + roads::right(b.out) * b.right_w;
        let segs = kerb_round(from, a.out, to, b.out);
        let total: f64 = segs.iter().map(|s| s.length).sum();
        if total < 0.05 {
            continue;
        }
        // stations along the kerb: (point, heading, distance); every 0.75 m round a curve, a
        // few along a straight (its sides are straight whatever the stations)
        let mut st: Vec<(DVec2, f64, f64)> = Vec::new();
        let mut run = 0.0;
        for (si, sgm) in segs.iter().enumerate() {
            let step = if sgm.radius == 0.0 { 6.0 } else { 0.75 };
            let k = (sgm.length / step).ceil().max(1.0) as usize;
            for i in 0..=k {
                if i == 0 && si > 0 {
                    continue;
                }
                let d = sgm.length * i as f64 / k as f64;
                st.push((sgm.point(d), sgm.heading_at(d), run + d));
            }
            run += sgm.length;
        }
        let n = st.len() - 1;
        if n < 1 {
            continue;
        }
        for p in st.iter().skip(1).take(n.saturating_sub(1)) {
            let t = p.2 / total;
            outline.push(DVec3::new(p.0.x, p.0.y, a.end.z + (b.end.z - a.end.z) * t));
        }
        // the corner's kerb and footway: this arm's left side or the next arm's right one,
        // whichever reaches further out (the narrower one then runs into it, as a footway
        // meets a verge), swept along the curve (it lies to the right of the way round)
        let reach = |v: &[SideProfile]| v.iter().flat_map(|p| p.points.iter()).map(|q| q.0).fold(0.0, f64::max);
        let (sides, v0, vdir, back) = if reach(&b.right) > reach(&a.left) + 0.5 { (&b.right, b.v0, b.vdir, true) } else { (&a.left, a.v0, a.vdir, false) };
        // (the strip as it is at the arm it starts from and at the one it ends at: between two
        // roads of different width it grows from the one's footway to the other's)
        let (from_arm, to_arm) = (&a.left, &b.right);
        // (every corner grows from the one arm's footway to the other's, so each meets its road
        // at the road's own width: taken whole from the wider one, a narrower road's footway
        // would end in a step at its mouth)
        let blend = !from_arm.is_empty() && from_arm.len() == to_arm.len();
        let joint = arms.len() == 2 && blend;
        let strips: Vec<(&SideProfile, &Vec<(f64, f64, f64, f64)>, &Vec<(f64, f64, f64, f64)>)> = if blend {
            from_arm.iter().zip(to_arm.iter()).map(|(f, t)| (if back { t } else { f }, &f.points, &t.points)).collect()
        } else {
            sides.iter().map(|p| (p, &p.points, &p.points)).collect()
        };
        for (prof, p_from, p_to) in strips {
            let mat = mesh.material(prof.texture.clone());
            let n = p_from.len().max(p_to.len());
            // (the profile at a place along the kerb, the shorter one's last point repeated)
            let at = |t: f64| -> Vec<(f64, f64, f64, f64)> {
                (0..n)
                    .map(|i| {
                        let (f, g) = (p_from[i.min(p_from.len() - 1)], p_to[i.min(p_to.len() - 1)]);
                        (f.0 + (g.0 - f.0) * t, f.1 + (g.1 - f.1) * t, f.2 + (g.2 - f.2) * t, f.3 + (g.3 - f.3) * t)
                    })
                    .collect()
            };
            for q in st.windows(2) {
                let (p0, h0, s0) = q[0];
                let (p1, h1, s1) = q[1];
                // (between two roads the strip's cross-sections are turned from the one's
                // across to the other's, so they meet the roads' ends square, with no wedge
                // of ground between)
                let turn = |h: f64, s: f64| if joint { let from = roads::norm_deg(a.out + 180.0); from + signed_turn(from, b.out) * (s / total) } else { h };
                let (h0, h1) = (turn(h0, s0), turn(h1, s1));
                let (w0, w1) = (at(s0 / total), at(s1 / total));
                for i in 0..n.saturating_sub(1) {
                    let (d0, z0, u0, vs0) = w0[i];
                    let (d1, z1, u1, vs1) = w0[i + 1];
                    let (e0, y0, _, _) = w1[i];
                    let (e1, y1, _, _) = w1[i + 1];
                    // the face's normal in the cross-section: up for a flat part, towards the road
                    // for the face of a kerb
                    let (dd, dz) = (d1 - d0, z1 - z0);
                    let base = |p: DVec2, h: f64, s: f64, d: f64, z: f64| {
                        let xy = p + roads::right(h) * d;
                        let zz = a.end.z + (b.end.z - a.end.z) * (s / total) + z;
                        rel(DVec3::new(xy.x, xy.y, zz))
                    };
                    // (the texture runs on from the arm it is taken from)
                    let v = |s: f64, vs: f64| if back { v0 - vdir * vs * (total - s) } else { v0 + vdir * vs * s };
                    let pa = base(p0, h0, s0, d0, z0);
                    let pb = base(p0, h0, s0, d1, z1);
                    let pc = base(p1, h1, s1, e0, y0);
                    let pd = base(p1, h1, s1, e1, y1);
                    let r = roads::right((h0 + h1) * 0.5 + if (h1 - h0).abs() > 180.0 { 180.0 } else { 0.0 });
                    // (in the cross-section, the left of the way outward: up, or towards the road)
                    let nrm = DVec3::new(-r.x * dz, -r.y * dz, dd);
                    let nrm = if nrm.length() < 1e-9 { DVec3::Z } else { nrm };
                    let (ua, ub) = ([u0, v(s0, vs0)], [u1, v(s0, vs1)]);
                    let (uc, ud) = ([u0, v(s1, vs0)], [u1, v(s1, vs1)]);
                    mesh.tri([pa, pb, pc], [ua, ub, uc], nrm, mat);
                    mesh.tri([pb, pd, pc], [ub, ud, uc], nrm, mat);
                }
            }
        }
    }
    log::debug!("junction outline: {:?}", outline.iter().map(|p| { let q = *p - centre; (q.x.round() as i64, q.y.round() as i64) }).collect::<Vec<_>>());
    for (i, g) in order.iter().map(|i| &arms[*i]).enumerate() {
        log::debug!("arm {i}: end ({:.1}, {:.1}) out {:.0} right {:.1} left {:.1}", g.end.x - centre.x, g.end.y - centre.y, g.out, g.right_w, g.left_w);
    }
    // the roadway: a fan round the centre, its texture laid flat on the map
    // (at the height the roads' surface lies, not the line they are laid along)
    let road_z = arms.iter().map(|a| a.road_z).sum::<f64>() / arms.len().max(1) as f64;
    let mid = DVec3::new(0.0, 0.0, outline.iter().map(|p| p.z - centre.z).sum::<f64>() / outline.len().max(1) as f64 + road_z);
    let (fwd, side) = (roads::dir(along), roads::right(along));
    let uv = |p: DVec3| [(p.x * side.x + p.y * side.y) / period, -(p.x * fwd.x + p.y * fwd.y) / period];
    for i in 0..outline.len() {
        let p0 = rel(outline[i]) + DVec3::Z * road_z;
        let p1 = rel(outline[(i + 1) % outline.len()]) + DVec3::Z * road_z;
        mesh.tri([mid, p0, p1], [uv(mid), uv(p0), uv(p1)], DVec3::Z, core);
        mesh.roadway.push([mid, p0, p1]);
    }
    mesh
}

/// The kerb round a corner: from `a` (on a kerb line running out along `out_a`) to `b` (on
/// one running out along `out_b`). The two lines are carried on into the junction to where
/// they meet, and a circle touching both rounds the corner there, as large as the junction
/// has room for (a fillet); kerb lines in one line, or meeting outside, are joined straight.
fn kerb_round(a: DVec2, out_a: f64, b: DVec2, out_b: f64) -> Vec<Seg> {
    let (da, db) = (roads::dir(out_a), roads::dir(out_b));
    let straight = || roads::straight(a, b).map(|s| vec![s]).unwrap_or_default();
    // (the outside of a bend: the kerb lines part instead of meeting, and a smooth curve
    // carries the old edge round - a straight cut there left the ground showing)
    let smooth = || roads::biarc(a, roads::norm_deg(out_a + 180.0), b, out_b).unwrap_or_else(straight);
    // two arms leaving side by side (the nose of an island between them): each kerb runs back
    // along its own line to where the nearer arm ends, and a turn joins them there; one
    // curve from end to end crossed itself when one arm reached further out than the other
    if da.dot(db) > 0.7 {
        let m = (da + db).normalize_or_zero();
        let (pa, pb) = (a.dot(m), b.dot(m));
        let s0 = pa.min(pb);
        let a2 = a - da * ((pa - s0) / da.dot(m).max(0.2));
        let b2 = b - db * ((pb - s0) / db.dot(m).max(0.2));
        let mut segs = Vec::new();
        if (a2 - a).length() > 0.02 {
            segs.extend(roads::straight(a, a2));
        }
        match roads::biarc(a2, roads::norm_deg(out_a + 180.0), b2, out_b) {
            Some(t) => segs.extend(t),
            None => segs.extend(roads::straight(a2, b2)),
        }
        if (b - b2).length() > 0.02 {
            segs.extend(roads::straight(b2, b));
        }
        if !segs.is_empty() {
            return segs;
        }
    }
    let denom = da.perp_dot(db);
    if denom.abs() < 0.05 {
        // in one line (the straight side of a T): straight across; side by side: round
        return if da.dot(db) < 0.0 { straight() } else { smooth() };
    }
    // a + da * u = b + db * v
    let u = (b - a).perp_dot(db) / denom;
    let v = (b - a).perp_dot(da) / denom;
    if u > 0.01 || v > 0.01 {
        return smooth();
    }
    let x = a + da * u;
    let t = (-u).min(-v);
    let (ta, tb) = (x + da * t, x + db * t);
    let mut segs = Vec::new();
    if (ta - a).length() > 0.02 {
        segs.push(Seg { start: a, heading: roads::norm_deg(out_a + 180.0), length: (ta - a).length(), radius: 0.0 });
    }
    // the circle touching both lines at ta and tb (equally far from where they meet)
    let side = (tb - ta).dot(roads::right(out_a + 180.0));
    let chord = (tb - ta).length();
    let fwd = (tb - ta).dot(roads::dir(out_a + 180.0));
    if chord > 0.02 && side.abs() > 1e-6 {
        let r = (fwd * fwd + side * side) / (2.0 * side);
        let sweep = 2.0 * side.atan2(fwd);
        segs.push(Seg { start: ta, heading: roads::norm_deg(out_a + 180.0), length: (r * sweep).abs(), radius: r });
    }
    if (b - tb).length() > 0.02 {
        segs.push(Seg { start: tb, heading: roads::norm_deg(out_b), length: (b - tb).length(), radius: 0.0 });
    }
    if segs.is_empty() { straight() } else { segs }
}

/// A line painted along a road, as its texture shows it across the road.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PaintLine {
    /// Its middle across the spline (the spline's own x, unmirrored; m) and its width.
    x: f64,
    width: f64,
    /// Unbroken (else dashed).
    solid: bool,
    /// On the roadway (else on the strip just beside it, which the plate's kerb sweep
    /// carries round the corner as it is).
    inner: bool,
}

/// The lines a road type has painted on it, and the paint's colour. None at all: a road with
/// no markings (a yard drive), whose plate gets none either.
#[derive(Debug, Clone, Default, PartialEq)]
struct RoadPaint {
    lines: Vec<PaintLine>,
    rgb: [u8; 3],
}

/// The paint on a road type, read from its own texture: across the roadway (and a little
/// beside it) every 2 cm, the texture's column there is looked down for pixels much brighter
/// than the asphalt. A stripe of such columns, 6 to 45 cm wide, bright all the way down
/// (a solid line) or in a few long runs (dashes), is a line; noise, grain, cobbles and wide
/// pale strips (concrete, gravel) are not.
fn scan_paint(model: &SplineModel) -> RoadPaint {
    let g = arm_geo_of(model, false, false, DVec3::ZERO, 0.0, 0.0);
    let (xl, xr, rz) = (-g.left_w, g.right_w, g.road_z);
    if !(xr - xl > 0.5) {
        return RoadPaint::default();
    }
    let (lo, hi) = (xl - 0.6, xr + 0.6);
    // the flat pieces of the profiles at the roadway's height: (x0, x1, u0, u1, material)
    let mut pieces: Vec<(f64, f64, f64, f64, usize)> = Vec::new();
    for p in &model.def.profiles {
        for w in p.points.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let (x0, x1, z0, z1) = (a.x as f64, b.x as f64, a.z as f64, b.z as f64);
            let flat = (x1 - x0).abs() > 1e-3 && ((z1 - z0) / (x1 - x0)).abs() < 0.15;
            if !flat || (z0 - rz).abs() > 0.06 || (z1 - rz).abs() > 0.06 || x0.max(x1) < lo || x0.min(x1) > hi {
                continue;
            }
            pieces.push((x0, x1, a.u as f64, b.u as f64, p.texture));
        }
    }
    // their pictures (and whether their alpha cuts them), by material
    let mut pics: std::collections::HashMap<usize, Option<(omsi_texture::Image, bool)>> = std::collections::HashMap::new();
    for pc in &pieces {
        pics.entry(pc.4).or_insert_with(|| {
            let m = model.materials.get(pc.4)?;
            let img = omsi_texture::decode_file(m.texture.as_ref()?).ok()?;
            if img.width == 0 || img.height == 0 || img.rgba.len() < (img.width * img.height * 4) as usize {
                return None;
            }
            let cut = img.has_alpha && m.alpha != crate::assets::Alpha::Opaque;
            Some((img, cut))
        });
    }
    const STEP: f64 = 0.02;
    const ROWS: usize = 128;
    let n = ((hi - lo) / STEP).ceil() as usize;
    let xs: Vec<f64> = (0..n).map(|i| lo + (i as f64 + 0.5) * STEP).collect();
    // down each column: the ground's brightness (the first piece that shows there) and the
    // brightest of the pieces that show there, with its colour
    let mut cols: Vec<Option<Vec<(f32, f32, [u8; 3])>>> = Vec::with_capacity(n);
    for &x in &xs {
        let mut col: Option<Vec<(f32, f32, [u8; 3])>> = None;
        for pc in pieces.iter().filter(|pc| (pc.0 - x) * (pc.1 - x) <= 0.0) {
            let Some(Some((img, cut))) = pics.get(&pc.4) else { continue };
            let (w, h) = (img.width as usize, img.height as usize);
            let u = pc.2 + (pc.3 - pc.2) * (x - pc.0) / (pc.1 - pc.0);
            let sx = ((u.rem_euclid(1.0) * w as f64) as usize).min(w - 1);
            let c = col.get_or_insert_with(|| vec![(-1.0, -1.0, [0, 0, 0]); ROWS]);
            for (r, s) in c.iter_mut().enumerate() {
                let sy = (((r as f64 + 0.5) / ROWS as f64 * h as f64) as usize).min(h - 1);
                let px = &img.rgba[(sy * w + sx) * 4..][..4];
                if *cut && px[3] < 128 {
                    continue;
                }
                let lum = (px[0] as f32 + px[1] as f32 + px[2] as f32) / 765.0;
                if s.0 < 0.0 {
                    s.0 = lum;
                }
                if lum > s.1 {
                    *s = (s.0, lum, [px[0], px[1], px[2]]);
                }
            }
        }
        cols.push(col);
    }
    // the asphalt: the middle brightness of the roadway's ground
    let mut ground: Vec<f32> = xs.iter().zip(&cols).filter(|(x, _)| **x >= xl && **x <= xr).flat_map(|(_, c)| c.iter().flatten().map(|s| s.0).filter(|v| *v >= 0.0)).collect();
    if ground.is_empty() {
        return RoadPaint::default();
    }
    ground.sort_by(f32::total_cmp);
    let base = ground[ground.len() / 2];
    let paint = |s: &(f32, f32, [u8; 3])| s.1 > base + 0.18 && s.1 > 0.5;
    // each column: how much of it is paint, and in how many runs down it (short gaps closed)
    let stats: Vec<(f64, usize)> = cols.iter().map(|c| c.as_ref().map(|c| paint_down(&c.iter().map(paint).collect::<Vec<_>>())).unwrap_or((0.0, usize::MAX))).collect();
    let mut ok: Vec<bool> = stats.iter().map(|(cover, runs)| is_paint(*cover, *runs)).collect();
    for i in 1..n.saturating_sub(1) {
        if !ok[i] && ok[i - 1] && ok[i + 1] {
            ok[i] = true;
        }
    }
    let mut lines = Vec::new();
    let mut colour = [0.0f64; 3];
    let mut colour_n = 0usize;
    let mut i = 0;
    while i < n {
        if !ok[i] {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < n && ok[j + 1] {
            j += 1;
        }
        let width = (j - i + 1) as f64 * STEP;
        let x = (xs[i] + xs[j]) * 0.5;
        let mut covers: Vec<f64> = stats[i..=j].iter().map(|s| s.0).collect();
        covers.sort_by(f64::total_cmp);
        let solid = covers[covers.len() / 2] >= 0.8;
        let inner = x >= xl + 0.02 && x <= xr - 0.02;
        // (beside the roadway only a narrow line along its edge: a pale strip further out is
        // a gutter or a verge)
        let edge = (x - xl).abs().min((x - xr).abs());
        if (0.06..=0.45).contains(&width) && (inner || (width <= 0.25 && edge <= 0.35)) {
            lines.push(PaintLine { x, width, solid, inner });
            // (the paint's colour from the line's middle, clear of its soft edges)
            let third = (j - i + 1) / 3;
            for c in cols[i + third..=j - third].iter().flatten() {
                for s in c.iter().filter(|s| paint(s)) {
                    for k in 0..3 {
                        colour[k] += s.2[k] as f64;
                    }
                    colour_n += 1;
                }
            }
        }
        i = j + 1;
    }
    let rgb = if colour_n > 0 { colour.map(|c| (((c / colour_n as f64) / 8.0).round() * 8.0).clamp(0.0, 255.0) as u8) } else { [232, 232, 224] };
    RoadPaint { lines, rgb }
}

/// Down one column of a texture, which pixels are paint: how much of it is (0..1), and in how
/// many runs (gaps of a pixel or two closed; it wraps round, as the texture repeats).
fn paint_down(b: &[bool]) -> (f64, usize) {
    let n = b.len();
    if n == 0 {
        return (0.0, 0);
    }
    let cover = b.iter().filter(|v| **v).count() as f64 / n as f64;
    let closed: Vec<bool> = (0..n).map(|r| b[r] || ((1..=2).any(|k| b[(r + n - k % n) % n]) && (1..=2).any(|k| b[(r + k) % n]))).collect();
    let runs = (0..n).filter(|r| closed[*r] && !closed[(r + n - 1) % n]).count();
    (cover, runs)
}

/// A column with that much paint in that many runs is a painted line: bright all the way
/// down (solid), or in a few long dashes; scattered bright grain is not.
fn is_paint(cover: f64, runs: usize) -> bool {
    cover >= 0.85 || (cover >= 0.2 && runs <= 3)
}

/// The paint of a road type, read from its texture once for as long as the type stays loaded.
fn road_paint(model: &std::sync::Arc<SplineModel>) -> std::sync::Arc<RoadPaint> {
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static SEEN: OnceLock<Mutex<Vec<(Weak<SplineModel>, Arc<RoadPaint>)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let known = |s: &Vec<(Weak<SplineModel>, Arc<RoadPaint>)>| s.iter().find(|(w, _)| w.strong_count() > 0 && std::ptr::eq(w.as_ptr(), Arc::as_ptr(model))).map(|(_, p)| p.clone());
    if let Some(p) = seen.lock().ok().and_then(|s| known(&s)) {
        return p;
    }
    let p = Arc::new(scan_paint(model));
    if let Ok(mut s) = seen.lock() {
        s.retain(|(w, _)| w.strong_count() > 0);
        s.push((Arc::downgrade(model), p.clone()));
    }
    p
}

/// An arm's painted lines as the plate sees them: (lateral place right of the way out, width,
/// on the roadway). None: its road has no paint.
struct ArmPaint {
    lines: Vec<(f64, f64, bool)>,
    rgb: [u8; 3],
}

fn arm_paint(p: &RoadPaint, vdir: f64) -> Option<ArmPaint> {
    // (`vdir` says both which end of the spline is at the junction and whether it is
    // mirrored: the spline's x is right of the way out times -vdir)
    (!p.lines.is_empty()).then(|| ArmPaint { lines: p.lines.iter().map(|l| (-vdir * l.x, l.width, l.inner)).collect(), rgb: p.rgb })
}

/// How high the paint lies over the asphalt (m): enough that OMSI's depth buffer keeps it on
/// top from afar, too little to see.
const MARK_LIFT: f64 = 0.01;
/// How far the roads' lines run on into the plate (m), and how wide a stop line is.
const LINE_RUN: f64 = 4.0;
const STOP_W: f64 = 0.4;

/// A strip of paint on the plate: along its middle from `a` to `b` (the arms' frame), `width`
/// across; on arm `arm`, a stop line or a line carried on.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
struct Mark {
    arm: usize,
    stop: bool,
    a: DVec2,
    b: DVec2,
    width: f64,
    rgb: [u8; 3],
}

/// Whether arm `b` carries arm `a`'s road straight on through the junction.
fn straight_on(a: &ArmGeo, b: &ArmGeo) -> bool {
    (roads::norm_deg(b.out - a.out) - 180.0).abs() < 30.0
}

/// The arms of the road that runs on through the junction and keeps its way (no stop line
/// on them, the side roads give way to it): the pair carried straight on through each
/// other, where there is one; of two such pairs, the wider road, none where both are as wide
/// (a crossroads of equals stops every way).
fn main_through(arms: &[ArmGeo]) -> Vec<bool> {
    let width = |g: &ArmGeo| g.left_w + g.right_w;
    let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
    for i in 0..arms.len() {
        for j in i + 1..arms.len() {
            if straight_on(&arms[i], &arms[j]) && !pairs.iter().any(|p| p.0 == i || p.1 == i || p.0 == j || p.1 == j) {
                pairs.push((i, j, width(&arms[i]).min(width(&arms[j]))));
            }
        }
    }
    pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut out = vec![false; arms.len()];
    match pairs.as_slice() {
        [] => {}
        [p] if arms.len() > 2 => {
            out[p.0] = true;
            out[p.1] = true;
        }
        [p, q, ..] if p.2 > q.2 + 0.5 => {
            out[p.0] = true;
            out[p.1] = true;
        }
        _ => {}
    }
    out
}

/// Whether `p` lies in arm `b`'s band: `span` across (right of its way out), from `from` to
/// `to` metres along its way out from its end.
fn in_band(b: &ArmGeo, p: DVec2, span: (f64, f64), from: f64, to: f64) -> bool {
    let q = p - b.end.truncate();
    let (along, side) = (q.dot(roads::dir(b.out)), q.dot(roads::right(b.out)));
    along >= from && along <= to && side >= span.0 && side <= span.1
}

/// How far from the junction each road ending there (cut at its middle, before it is cut
/// back) is to be cut back (m, along it): `r`, the room its corners ask for, or further where
/// that leaves its roadway on another road's: until it is clear of each other road's roadway
/// (and half a metre beside it) carried on across the junction, the road carried straight on
/// through aside. A road meeting a wide one whose roadway reaches far to one side of its line
/// (a one-way road drawn along its edge) stops at that road's kerb, not in its lanes, however
/// tight a corner style asks for; narrower roads keep the style's `r`. None (or a road not
/// loaded): `r`.
pub fn cut_back(doc: &Document, assets: &mut Assets, ends: &[Option<(TileKey, i64, bool)>], r: f64) -> Vec<f64> {
    let geo: Vec<Option<(ArmGeo, f64)>> = ends
        .iter()
        .map(|e| {
            let (key, id, at_end) = (*e)?;
            let hw = assets.spline(&doc.spline(key, id)?.file)?.half_width as f64;
            Some((arm_geometry(doc, assets, &Arm { key, id, at_end })?, hw))
        })
        .collect();
    let arms: Vec<(usize, &ArmGeo)> = geo.iter().enumerate().filter_map(|(i, g)| g.as_ref().map(|g| (i, &g.0))).collect();
    let widest = arms.iter().map(|(_, g)| g.left_w.max(g.right_w)).fold(0.0, f64::max);
    // (as far as a sharp corner may ask for)
    let most = (radius(&geo.iter().flatten().map(|g| g.1).collect::<Vec<_>>()) * 3.0).max(r);
    geo.iter()
        .enumerate()
        .map(|(i, g)| {
            let Some((a, _)) = g else { return r };
            let others: Vec<&ArmGeo> = arms.iter().filter(|(j, b)| *j != i && !straight_on(a, b)).map(|(_, b)| *b).collect();
            let (d, rt, e) = (roads::dir(a.out), roads::right(a.out), a.end.truncate());
            let n = ((a.left_w + a.right_w) / 0.5).ceil().max(1.0) as usize;
            let across: Vec<f64> = (0..=n).map(|k| -a.left_w + (a.left_w + a.right_w) * k as f64 / n as f64).collect();
            let blocked = |t: f64| across.iter().any(|s| others.iter().any(|b| in_band(b, e + d * t + rt * *s, (-b.left_w - 0.5, b.right_w + 0.5), -(widest + 1.0), f64::INFINITY)));
            let mut t = 0.0;
            while t < most && blocked(t) {
                t += 0.05;
            }
            r.max(t.min(most))
        })
        .collect()
}

/// How far into the plate an arm's roadway runs clear of the other arms' (m, at most `most`):
/// each other arm's `band` (its lateral span, right of its way out) carried on in to the
/// middle and past it by the widest road, except the arm that carries this road straight on
/// through.
fn room(arms: &[ArmGeo], i: usize, centre: DVec2, most: f64, band: &dyn Fn(usize) -> (f64, f64)) -> f64 {
    let a = &arms[i];
    let (d, r, e) = (roads::dir(a.out), roads::right(a.out), a.end.truncate());
    let widest = arms.iter().map(|g| g.left_w.max(g.right_w)).fold(0.0, f64::max);
    let n = ((a.left_w + a.right_w) / 0.5).ceil().max(1.0) as usize;
    let across: Vec<f64> = (0..=n).map(|k| -a.left_w + (a.left_w + a.right_w) * k as f64 / n as f64).collect();
    let others: Vec<(usize, (f64, f64))> = (0..arms.len()).filter(|j| *j != i && !straight_on(a, &arms[*j])).map(|j| (j, band(j))).collect();
    let blocked = |p: DVec2| {
        others.iter().any(|(j, span)| {
            let b = &arms[*j];
            let reach = (b.end.truncate() - centre).length() + widest + 1.0;
            in_band(b, p, *span, -reach, 0.0)
        })
    };
    let mut t = 0.0;
    while t < most {
        if across.iter().any(|s| blocked(e - d * t + r * *s)) {
            return t;
        }
        t += 0.05;
    }
    most
}

/// The paint on a plate, arm by arm (each with paint on its road, and not `skip`ped): its
/// lines on the roadway run on in from the arm's mouth up to `LINE_RUN` (less where the other
/// roads come close: none where they come within a metre and a half), and, with
/// `stop_lines`, a stop line lies across the lanes coming in where those lines end (at the
/// mouth when they do not run on; none where all its lanes leave, nor where it would lie on
/// the other roads' lanes). All of it kept on the roadway.
fn plate_marks(mesh: &Mesh, arms: &[ArmGeo], lanes: &[LaneEnd], paint: &[Option<ArmPaint>], skip: &[bool], stop_lines: bool, centre: DVec3) -> Vec<Mark> {
    let c2 = centre.truncate();
    let on = |p: DVec2| mesh.roadway_z(p - c2).is_some();
    // (each arm's roadway with a margin; and where its lanes run)
    let roadway = |j: usize| (-arms[j].left_w - 0.5, arms[j].right_w + 0.5);
    let traffic = |j: usize| {
        let v: Vec<(f64, f64)> = lanes.iter().filter(|l| l.arm == j && l.kind == 0).map(|l| (-l.side - l.width * 0.5, -l.side + l.width * 0.5)).collect();
        if v.is_empty() { (-arms[j].left_w, arms[j].right_w) } else { (v.iter().map(|s| s.0).fold(f64::MAX, f64::min), v.iter().map(|s| s.1).fold(f64::MIN, f64::max)) }
    };
    let through = main_through(arms);
    let mut out = Vec::new();
    for (i, g) in arms.iter().enumerate() {
        let Some(p) = paint.get(i).and_then(|p| p.as_ref()) else { continue };
        if skip.get(i).copied().unwrap_or(false) {
            continue;
        }
        let (d, r, e) = (roads::dir(g.out), roads::right(g.out), g.end.truncate());
        let at = |t: f64, s: f64| e - d * t + r * s;
        let clear = room(arms, i, c2, LINE_RUN + 3.0, &roadway);
        let run = if clear - 1.5 >= 1.0 { (clear - 1.5).min(LINE_RUN) } else { 0.0 };
        // the lines carried on in (as far as both their edges stay on the roadway)
        for &(off, w, inner) in &p.lines {
            if !inner || run <= 0.0 {
                continue;
            }
            let mut end = 0.0;
            let mut t: f64 = 0.25;
            loop {
                let t1 = t.min(run);
                if !(on(at(t1, off - w * 0.5)) && on(at(t1, off + w * 0.5))) {
                    break;
                }
                end = t1;
                if t1 >= run {
                    break;
                }
                t += 0.25;
            }
            if end >= 0.5 {
                out.push(Mark { arm: i, stop: false, a: at(0.0, off), b: at(end, off), width: w, rgb: p.rgb });
            }
        }
        let t0 = run.max(0.05);
        let t1 = t0 + STOP_W;
        if !stop_lines || through[i] || room(arms, i, c2, t1 + 0.1, &traffic) < t1 {
            continue;
        }
        // the stop line: across the lanes coming in, from the line between them and the lanes
        // going out (or the middle between them) to the edge line beside them (or the kerb)
        let span = |incoming: bool| -> Option<(f64, f64)> {
            let v: Vec<(f64, f64)> = lanes.iter().filter(|l| l.arm == i && l.kind == 0 && l.incoming == incoming).map(|l| (-l.side, l.width)).collect();
            if v.is_empty() {
                return None;
            }
            Some((v.iter().map(|(o, w)| o - w * 0.5).fold(f64::MAX, f64::min), v.iter().map(|(o, w)| o + w * 0.5).fold(f64::MIN, f64::max)))
        };
        let Some((in_lo, in_hi)) = span(true) else { continue };
        let low_edge = p.lines.iter().map(|l| l.0 - l.1 * 0.5).filter(|x| *x >= in_lo - 1.5 && *x <= in_lo + 0.5).reduce(f64::min).unwrap_or(-g.left_w + 0.1).max(-g.left_w);
        let high_edge = p.lines.iter().map(|l| l.0 + l.1 * 0.5).filter(|x| *x >= in_hi - 0.5 && *x <= in_hi + 1.5).reduce(f64::max).unwrap_or(g.right_w - 0.1).min(g.right_w);
        let near = |b: f64| p.lines.iter().filter(move |l| (l.0 - b).abs() <= 0.75);
        let (mut s0, mut s1) = match span(false) {
            None => (low_edge, high_edge),
            Some((out_lo, _)) if out_lo >= in_hi - 0.3 => {
                let b = (in_hi + out_lo) * 0.5;
                (low_edge, near(b).map(|l| l.0 - l.1 * 0.5).reduce(f64::min).unwrap_or(b))
            }
            Some((_, out_hi)) if out_hi <= in_lo + 0.3 => {
                let b = (out_hi + in_lo) * 0.5;
                (near(b).map(|l| l.0 + l.1 * 0.5).reduce(f64::max).unwrap_or(b), high_edge)
            }
            // (lanes both ways over the same width: no side to stop on)
            _ => continue,
        };
        let fits = |s: f64| on(at(t0, s)) && on(at(t1, s));
        while s0 < s1 && !fits(s0) {
            s0 += 0.05;
        }
        while s1 > s0 && !fits(s1) {
            s1 -= 0.05;
        }
        if s1 - s0 >= 1.0 {
            let tm = (t0 + t1) * 0.5;
            out.push(Mark { arm: i, stop: true, a: at(tm, s0), b: at(tm, s1), width: STOP_W, rgb: p.rgb });
        }
    }
    out
}

/// A plain picture of the paint's colour in `dir` (made once): the markings' texture.
fn marking_texture(dir: &Path, rgb: [u8; 3]) -> Option<PathBuf> {
    let p = dir.join(format!("om_marking_{:02x}{:02x}{:02x}.png", rgb[0], rgb[1], rgb[2]));
    if !std::fs::metadata(&p).map(|m| m.is_file() && m.len() > 0).unwrap_or(false) {
        image::RgbaImage::from_pixel(8, 8, image::Rgba([rgb[0], rgb[1], rgb[2], 255])).save(&p).ok()?;
    }
    Some(p)
}

/// The marks laid on the roadway as decals (see `Mesh::paint_strip`), with their paint's
/// picture.
fn add_marks(mesh: &mut Mesh, marks: &[Mark], tex_dir: &Path, centre: DVec3) {
    let c2 = centre.truncate();
    for m in marks {
        let Some(tex) = marking_texture(tex_dir, m.rgb) else { continue };
        let mat = mesh.material(Some(tex));
        mesh.paint_strip(m.a - c2, m.b - c2, m.width, mat);
    }
}

/// The mesh as a DirectX `.x` text file in OMSI's frame (x right, y up, z forward).
/// (Seen from the side a triangle faces, OMSI's front faces turn clockwise.) Every triangle
/// is there twice, the second one facing the other way: a game that culls back faces by a
/// rule of its own (the winding the version reads, the normals it trusts) shows the plate
/// from above either way, as OMSI's own crossings are modelled.
fn mesh_x(m: &Mesh, names: &[String]) -> String {
    // (one vertex for each place, normal and texture point however many triangles share it:
    // a city's plates are written several times smaller than with three of their own each)
    let q = |v: f64, s: f64| (v * s).round() as i64;
    let mut index: std::collections::HashMap<[i64; 8], usize> = std::collections::HashMap::new();
    let mut verts: Vec<([f64; 3], [f64; 3], [f64; 2])> = Vec::new();
    let tris: Vec<([usize; 3], usize)> = m
        .tris
        .iter()
        .map(|(f, mat)| {
            let f = f.map(|i| {
                let (p, n, t) = m.verts[i];
                let key = [q(p[0], 1000.0), q(p[1], 1000.0), q(p[2], 1000.0), q(n[0], 1000.0), q(n[1], 1000.0), q(n[2], 1000.0), q(t[0], 100000.0), q(t[1], 100000.0)];
                *index.entry(key).or_insert_with(|| {
                    verts.push((p, n, t));
                    verts.len() - 1
                })
            });
            (f, *mat)
        })
        .collect();
    let mut x = String::from("xof 0302txt 0032\n\nMesh {\n");
    let nv = verts.len();
    let nf = tris.len();
    let _ = writeln!(x, " {};", nv * 2);
    for pass in 0..2 {
        for (i, (p, _, _)) in verts.iter().enumerate() {
            let _ = writeln!(x, " {:.3};{:.3};{:.3};{}", p[0], p[2], p[1], if pass == 1 && i + 1 == nv { ";" } else { "," });
        }
    }
    let _ = writeln!(x, " {};", nf * 2);
    // (front: the order turned for the swapped axes; back: the order as it stands, on its own vertices)
    let faces = |x: &mut String, last: bool| {
        for (i, (f, _)) in tris.iter().enumerate() {
            let _ = writeln!(x, " 3;{},{},{};,", f[0], f[2], f[1]);
            let _ = writeln!(x, " 3;{},{},{};{}", nv + f[0], nv + f[1], nv + f[2], if last && i + 1 == nf { ";" } else { "," });
        }
    };
    faces(&mut x, true);
    let _ = writeln!(x, " MeshNormals {{\n  {};", nv * 2);
    for pass in 0..2 {
        let sign = if pass == 0 { 1.0 } else { -1.0 };
        for (i, (_, n, _)) in verts.iter().enumerate() {
            let _ = writeln!(x, "  {:.3};{:.3};{:.3};{}", n[0] * sign, n[2] * sign, n[1] * sign, if pass == 1 && i + 1 == nv { ";" } else { "," });
        }
    }
    let _ = writeln!(x, "  {};", nf * 2);
    faces(&mut x, true);
    x.push_str(" }\n");
    let _ = writeln!(x, " MeshTextureCoords {{\n  {};", nv * 2);
    for pass in 0..2 {
        for (i, (_, _, t)) in verts.iter().enumerate() {
            let _ = writeln!(x, "  {:.5};{:.5};{}", t[0], t[1], if pass == 1 && i + 1 == nv { ";" } else { "," });
        }
    }
    x.push_str(" }\n");
    let _ = writeln!(x, " MeshMaterialList {{\n  {};\n  {};", names.len().max(1), nf * 2);
    for (i, (_, mat)) in tris.iter().enumerate() {
        let _ = writeln!(x, "  {mat},\n  {mat}{}", if i + 1 == nf { ";" } else { "," });
    }
    for n in names {
        let _ = writeln!(x, "  Material {{\n   1.0;1.0;1.0;1.0;;\n   10.0;\n   0.0;0.0;0.0;;\n   0.0;0.0;0.0;;");
        if !n.is_empty() {
            let _ = writeln!(x, "   TextureFilename {{\n    \"{n}\";\n   }}");
        }
        x.push_str("  }\n");
    }
    x.push_str(" }\n}\n");
    x
}

/// A plain square of a road's own surface, `period` metres a side (what the texture spans
/// along the road): `band_m` metres of a lane's middle (no markings), as wide as it is on
/// the road, mirrored to and fro across so it tiles without a seam, and the texture's full
/// length along. The junction's roadway, laid flat at the same scale, then has the roads'
/// grain and colour, not a stretched smear.
fn clean_asphalt(tex: &Path, u: f64, u_per_m: f64, period: f64, lane_m: f64, out: &Path) -> Option<()> {
    let buf = clean_asphalt_image(tex, u, u_per_m, period, lane_m)?;
    save_like(&buf, out)
}

/// Save a picture in the format its file name says (the road's own texture is a .bmp, .dds,
/// .tga or .jpg, and what is made from it is written as one too); a name of another kind
/// gets a PNG.
fn save_like(buf: &image::RgbaImage, out: &Path) -> Option<()> {
    let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "dds" => std::fs::write(out, crate::doc::dds::rgba(buf.width(), buf.height(), buf.as_raw())).ok(),
        "bmp" | "jpg" | "jpeg" => image::DynamicImage::ImageRgba8(buf.clone()).to_rgb8().save(out).ok(),
        "tga" | "png" => buf.save(out).ok(),
        _ => buf.save_with_format(out, image::ImageFormat::Png).ok(),
    }
}

/// The files that go with a texture (`name_wintersnow.dds`, `name.bmp.cfg`, `name.surf`…):
/// everything beside it that starts with its name and then an underscore or a dot.
pub fn companions(tex: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(own)) = (tex.parent(), tex.file_stem().map(|s| s.to_string_lossy().to_lowercase()), tex.file_name().map(|s| s.to_string_lossy().to_lowercase())) else { return Vec::new() };
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let f = p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
            p.is_file() && f != own && (f.starts_with(&format!("{stem}_")) || f.starts_with(&format!("{stem}.")))
        })
        .collect();
    out.sort();
    out
}

/// A texture copied into `to_dir` with everything that goes with it (its seasonal pictures,
/// its .cfg and .surf), under its own name.
pub fn copy_texture(tex: &Path, to_dir: &Path) {
    let Some(name) = tex.file_name() else { return };
    let to = to_dir.join(name);
    if !to.exists() {
        let _ = std::fs::copy(tex, &to);
    }
    for c in companions(tex) {
        let Some(n) = c.file_name() else { continue };
        let t = to_dir.join(n);
        if !t.exists() {
            let _ = std::fs::copy(&c, &t);
        }
    }
}

/// The plain asphalt made from `tex` as `<stem>_junction2.<same extension>` in `to_dir`, with
/// the same made from each of its companions (its winter pictures, in their formats) and the
/// others (.cfg, .surf) copied under the new name. Gives the main picture's path.
fn plain_like(tex: &Path, u: f64, u_per_m: f64, period: f64, lane_m: f64, to_dir: &Path) -> Option<PathBuf> {
    let stem = tex.file_stem()?.to_string_lossy().to_string();
    let ext = tex.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "png".into());
    // (2: made with every painted line, dashed ones too, kept out; a `_junction` picture made
    // before may hold a dashed line, and a name already there is not made again)
    let new_stem = format!("{stem}_junction2");
    let out = to_dir.join(format!("{new_stem}.{ext}"));
    // (one cut short, a full disk say, is made again)
    if out.exists() && omsi_texture::decode_file(&out).is_err() {
        let _ = std::fs::remove_file(&out);
    }
    if !out.exists() {
        clean_asphalt(tex, u, u_per_m, period, lane_m, &out)?;
    }
    for c in companions(tex) {
        let Some(name) = c.file_name().map(|f| f.to_string_lossy().to_string()) else { continue };
        let to = to_dir.join(format!("{new_stem}{}", &name[stem.len().min(name.len())..]));
        if to.exists() {
            continue;
        }
        let image_ext = c.extension().map(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "bmp" | "dds" | "tga" | "jpg" | "jpeg" | "png")).unwrap_or(false);
        // (a picture made the same way, else the file as it is)
        let done = image_ext && c.file_name().map(|f| !f.to_string_lossy().to_lowercase().ends_with(".cfg")).unwrap_or(false) && clean_asphalt(&c, u, u_per_m, period, lane_m, &to).is_some();
        if !done {
            let _ = std::fs::copy(&c, &to);
        }
    }
    Some(out)
}

/// Where in a road's texture the plain asphalt is taken from: (the middle u of the band, its
/// width in metres and in u, mirrored to and fro). Every column across the lane round `u` is
/// looked down for paint as the road's lines are read (`scan_paint`): bright for long runs
/// down it (a solid line, or dashes however far apart) or brighter on the whole than the
/// rest. None: the whole lane, repeated, so slabs keep their grid. Paint: the widest stretch
/// of the lane clear of it by 10 cm (round the lane's middle when it is there), up to half a
/// lane, mirrored, so no line, solid or dashed, comes into the plate.
fn clean_band(img: &omsi_texture::Image, u: f64, u_per_m: f64, lane_m: f64) -> (f64, f64, f64, bool) {
    let (w, h) = (img.width as usize, img.height as usize);
    let u_per_m = u_per_m.max(1e-6);
    let whole = lane_m.clamp(1.0, 5.0);
    let plain = (u, whole, (whole * u_per_m).max(0.002), false);
    let half_lane = (lane_m * 0.5).clamp(0.8, 2.0);
    let fallback = (u, half_lane, (half_lane * u_per_m).max(0.002), true);
    if w == 0 || h == 0 || img.rgba.len() < w * h * 4 {
        return plain;
    }
    // the columns across the lane (k counts on past the texture's edge: it repeats)
    let span = (whole.max(lane_m) * u_per_m).min(1.0);
    let k0 = ((u - span * 0.5) * w as f64).floor() as i64;
    let k1 = (((u + span * 0.5) * w as f64).ceil() as i64).min(k0 + w as i64 - 1);
    let rows = h.min(128);
    let lum = |k: i64, r: usize| {
        let sx = k.rem_euclid(w as i64) as usize;
        let sy = (((r as f64 + 0.5) / rows as f64 * h as f64) as usize).min(h - 1);
        let p = &img.rgba[(sy * w + sx) * 4..][..3];
        (p[0] as f32 + p[1] as f32 + p[2] as f32) / 765.0
    };
    let cols: Vec<Vec<f32>> = (k0..=k1).map(|k| (0..rows).map(|r| lum(k, r)).collect()).collect();
    let median = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let base = median(cols.iter().flatten().copied().collect());
    let means: Vec<f32> = cols.iter().map(|c| c.iter().sum::<f32>() / rows as f32).collect();
    let mean_base = median(means.clone());
    let painted: Vec<bool> = cols
        .iter()
        .zip(&means)
        .map(|(c, m)| {
            let (cover, runs) = paint_down(&c.iter().map(|v| *v > base + 0.18 && *v > 0.5).collect::<Vec<_>>());
            is_paint(cover, runs) || *m > mean_base + 0.1
        })
        .collect();
    if !painted.iter().any(|p| *p) {
        return plain;
    }
    // (kept clear of the lines' soft edges)
    let n = painted.len();
    let margin = (0.1 * u_per_m * w as f64).ceil().max(2.0) as usize;
    let clear: Vec<bool> = (0..n).map(|i| !painted[i.saturating_sub(margin)..=(i + margin).min(n - 1)].iter().any(|p| *p)).collect();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if clear[i] {
            let j = (i..n).take_while(|j| clear[*j]).last().unwrap_or(i);
            runs.push((i, j));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    // (in columns from k0: the lane's middle, and the least a band may be)
    let ku = u * w as f64 - k0 as f64;
    let least = (0.3 * u_per_m * w as f64).max(3.0);
    let len = |r: &(usize, usize)| (r.1 - r.0 + 1) as f64;
    let at_middle = runs.iter().find(|r| r.0 as f64 <= ku && ku < r.1 as f64 + 1.0 && len(r) >= least);
    let Some(run) = at_middle.or_else(|| runs.iter().max_by(|a, b| len(a).total_cmp(&len(b)))).copied() else { return fallback };
    if len(&run) < 2.0 {
        return fallback;
    }
    let band = (len(&run) - 0.02).min(half_lane * u_per_m * w as f64);
    let (lo, hi) = (run.0 as f64 + band * 0.5, run.1 as f64 + 1.0 - band * 0.5);
    let c = if at_middle.is_some() { ku.clamp(lo, hi.max(lo)) } else { (lo + hi) * 0.5 };
    let band_u = band / w as f64;
    ((k0 as f64 + c) / w as f64, band_u / u_per_m, band_u, true)
}

fn clean_asphalt_image(tex: &Path, u: f64, u_per_m: f64, period: f64, lane_m: f64) -> Option<image::RgbaImage> {
    let img = omsi_texture::decode_file(tex).ok()?;
    let (w, h) = (img.width as usize, img.height as usize);
    let (centre_u, band_m, band_u, mirror) = clean_band(&img, u, u_per_m, lane_m);
    let size = 512usize;
    let mut buf = image::RgbaImage::new(size as u32, size as u32);
    // (each band side by side taken from another stretch of the road, so its patches and
    // slabs do not stand in rows of the same one)
    let bands = (period / band_m).round().max(1.0) as usize;
    let band_m = period / bands as f64;
    for y in 0..size {
        for x in 0..size {
            let m = x as f64 / size as f64 * period;
            let j = ((m / band_m) as usize).min(bands - 1);
            let shift = if j == 0 { 0.0 } else { (j as f64 * 0.618_034).fract() };
            let sy = (((y as f64 / size as f64 + shift).fract() * h as f64) as usize).min(h - 1);
            let t = if mirror {
                let f = (m / band_m).rem_euclid(2.0);
                if f < 1.0 { f } else { 2.0 - f }
            } else {
                (m / band_m).rem_euclid(1.0)
            };
            let uu = (centre_u - band_u * 0.5 + band_u * t).rem_euclid(1.0);
            let sx = ((uu * w as f64) as usize).min(w - 1);
            let p = &img.rgba[(sy * w + sx) * 4..][..4];
            buf.put_pixel(x as u32, y as u32, image::Rgba([p[0], p[1], p[2], 255]));
        }
    }
    flatten_columns(&mut buf);
    Some(buf)
}

/// Each column brought to the picture's mean brightness: the wheel tracks and the worn
/// stripes a lane's middle carries along the road (which, spread over a whole plate, read as
/// ribs) go, the asphalt's grain and patches stay.
fn flatten_columns(buf: &mut image::RgbaImage) {
    let (w, h) = (buf.width() as usize, buf.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let lum = |p: &image::Rgba<u8>| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0;
    let cols: Vec<f64> = (0..w).map(|x| (0..h).map(|y| lum(buf.get_pixel(x as u32, y as u32))).sum::<f64>() / h as f64).collect();
    // (the columns' means smoothed a little across, so a single dark crack keeps its line)
    let smooth: Vec<f64> = (0..w).map(|x| {
        let r = 1usize;
        let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
        cols[a..=b].iter().sum::<f64>() / (b - a + 1) as f64
    }).collect();
    let mean = cols.iter().sum::<f64>() / w as f64;
    for x in 0..w {
        let k = if smooth[x] > 1.0 { mean / smooth[x] } else { 1.0 };
        for y in 0..h {
            let p = buf.get_pixel_mut(x as u32, y as u32);
            for c in 0..3 {
                p[c] = (p[c] as f64 * k).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// How finely a plate's own shape is kept: arms that differ by less make the same file. Far under
/// what shows where a road meets its plate, and well over a float's noise.
const PLATE_GRID_M: f64 = 0.01;
const PLATE_GRID_DEG: f64 = 0.25;

fn snapped(v: f64, grid: f64) -> f64 {
    (v / grid).round() * grid
}

/// The junction as its object file holds it. Roads meeting the same way must make the same file,
/// wherever they lie on the map, whichever way round, and whichever was drawn first:
/// `order` is the arms counter-clockwise from the one every such junction picks, `turn` the
/// object's heading on the map (0 when an option names arms by their heading on the map), and the
/// arms and lanes are in the object's frame, kept to the plate's grid, with the texture along the
/// kerbs starting afresh at each arm rather than running on from that road's own length.
struct PlateFrame {
    order: Vec<usize>,
    turn: f64,
    arms: Vec<ArmGeo>,
    lanes: Vec<LaneEnd>,
}

fn plate_frame(geo: &[ArmGeo], files: &[String], lanes: &[LaneEnd], centre: DVec3, keep_heading: bool) -> PlateFrame {
    let ccw = |g: &ArmGeo| roads::norm_deg(-g.out).rem_euclid(360.0);
    let mut round: Vec<usize> = (0..geo.len()).collect();
    round.sort_by(|a, b| ccw(&geo[*a]).total_cmp(&ccw(&geo[*b])));
    // (each arm as it looks from the junction, and the turn on to the next round)
    let looks: Vec<String> = (0..round.len())
        .map(|k| {
            let (i, next) = (round[k], round[(k + 1) % round.len()]);
            let g = &geo[i];
            let to_next = (ccw(&geo[next]) - ccw(g)).rem_euclid(360.0);
            let d = g.end - centre;
            format!("{}|{}|{:.2}|{:.2}|{:.2}|{:.2}|{:.2}|{:.2}", files[i].to_lowercase(), g.vdir, g.right_w, g.left_w, snapped(d.truncate().length(), PLATE_GRID_M), snapped(d.z, PLATE_GRID_M), snapped(to_next, PLATE_GRID_DEG), g.road_z)
        })
        .collect();
    let from_arm = |k: usize| -> Vec<&String> { (0..looks.len()).map(|n| &looks[(k + n) % looks.len()]).collect() };
    let first = (0..looks.len()).min_by_key(|k| from_arm(*k)).unwrap_or(0);
    let order: Vec<usize> = (0..round.len()).map(|n| round[(first + n) % round.len()]).collect();
    let turn = if keep_heading { 0.0 } else { order.first().map(|i| geo[*i].out).unwrap_or(0.0) };
    let frame = |p: DVec3| -> DVec3 {
        let q = crate::selection::turn_point((p - centre).truncate(), DVec2::ZERO, -turn);
        DVec3::new(snapped(q.x, PLATE_GRID_M), snapped(q.y, PLATE_GRID_M), snapped(p.z - centre.z, PLATE_GRID_M))
    };
    let heading = |h: f64| roads::norm_deg(snapped(roads::norm_deg(h - turn), PLATE_GRID_DEG));
    let arms = order
        .iter()
        .map(|i| {
            let g = &geo[*i];
            ArmGeo { end: frame(g.end), out: heading(g.out), v0: 0.0, ..g.clone() }
        })
        .collect();
    let lanes = order
        .iter()
        .enumerate()
        .flat_map(|(n, i)| lanes.iter().filter(move |l| l.arm == *i).map(move |l| (n, l)))
        .map(|(n, l)| LaneEnd { arm: n, pos: frame(l.pos), flow: heading(l.flow), ..*l })
        .collect();
    PlateFrame { order, turn, arms, lanes }
}

/// How far from `centre` the plate reaches: out to the corners of the arms' ends, with the
/// kerb and footway beyond the roadway (the ground under it is brought to its height out to
/// there, not only to the roads' own width).
pub fn reach(doc: &Document, assets: &mut Assets, centre: DVec3, arms: &[Arm]) -> f64 {
    arms.iter()
        .filter_map(|a| arm_geometry(doc, assets, a))
        .map(|g| {
            let beyond = g.right.iter().chain(g.left.iter()).flat_map(|p| p.points.iter()).map(|q| q.0).fold(0.0, f64::max);
            let wide = g.right_w.max(g.left_w) + beyond;
            ((g.end.truncate() - centre.truncate()).length().powi(2) + wide * wide).sqrt()
        })
        .fold(0.0, f64::max)
}

/// The same with lights and crossings as `opts` says.
pub fn make_with(doc: &mut Document, assets: &mut Assets, content: &Path, asphalt: Option<&Path>, centre: DVec3, arms: &[Arm], opts: &Options) -> anyhow::Result<i64> {
    let (_, lanes) = arm_ends(doc, assets, arms).ok_or_else(|| anyhow::anyhow!("a road of the junction is not loaded"))?;
    let geo: Vec<ArmGeo> = arms.iter().map(|a| arm_geometry(doc, assets, a)).collect::<Option<_>>().ok_or_else(|| anyhow::anyhow!("a road of the junction is not loaded"))?;
    // (from here on, what goes in the file: in the object's frame, its centre the origin)
    let files: Vec<String> = arms.iter().map(|a| doc.spline(a.key, a.id).map(|s| s.file.clone()).unwrap_or_default()).collect();
    let plate = plate_frame(&geo, &files, &lanes, centre, !opts.crossings.is_empty());
    let plate_lights: Vec<Option<usize>> = if opts.lights && arms.len() >= 3 {
        let outs: Vec<f64> = plate.arms.iter().map(|g| g.out).collect();
        let (a, b) = main_road(&outs);
        (0..arms.len()).map(|i| Some(if i == a || i == b { 0 } else { 1 })).collect()
    } else {
        vec![None; arms.len()]
    };
    // (the signal heads stand on the map, arm by arm as the roads came)
    let mut light_of = vec![None; arms.len()];
    for (n, i) in plate.order.iter().enumerate() {
        light_of[*i] = plate_lights[n];
    }
    let mut recs = paths(&plate.lanes, DVec3::ZERO, &plate_lights);
    // zebra crossings: a footway across each arm chosen, a step inside its end
    let mut zebras: Vec<(DVec2, f64, f64, f64)> = Vec::new();
    for g in plate.arms.iter().filter(|g| opts.crossing_at(g.out)) {
        let r = roads::right(g.out);
        let mid = g.end.truncate() - roads::dir(g.out) * 2.0;
        let (a, b) = (mid - r * (g.left_w + 0.5), mid + r * (g.right_w + 0.5));
        if let Some(seg) = roads::straight(a, b) {
            let z = g.end.z;
            recs.push(PathRec { seg, z0: z, z1: z, kind: 1, width: 3.0, direction: 2, blinker: 0, light: None });
            zebras.push((mid, g.out, g.left_w, g.right_w));
        }
    }
    let id = doc.new_id();
    let folder: String = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into()).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let sco_root = std::fs::read_dir(content)?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("sceneryobjects")).unwrap_or(false)).unwrap_or_else(|| content.join("Sceneryobjects"));
    let dir = sco_root.join("openMapper").join(&folder);
    let tex_dir = dir.join("texture");
    std::fs::create_dir_all(dir.join("model"))?;
    std::fs::create_dir_all(&tex_dir)?;
    // the roadway's texture: the asphalt of the roads that meet (a plate in the colour of its
    // roads, cut so that no patch comes along in rows), else the installation's own junction
    // asphalt
    let widest = plate.arms.iter().filter(|g| g.road_tex.is_some()).max_by(|a, b| (a.left_w + a.right_w).total_cmp(&(b.left_w + b.right_w)));
    let road_tex = widest.and_then(|g| g.road_tex.clone());
    let (core_tex, period) = match road_tex {
        Some((tex, u, range, period, width, lane)) => {
            // (most of a lane: slabs and patches keep their size; the markings at its edges
            // stay out. Named, formatted and accompanied as the road's own texture is)
            let out = plain_like(&tex, u, range / width, period.clamp(2.0, 40.0), lane, &tex_dir);
            (out.or_else(|| asphalt.map(Path::to_path_buf)), period.clamp(2.0, 40.0))
        }
        None => (asphalt.map(Path::to_path_buf), 8.0),
    };
    let along = widest.map(|g| g.out).unwrap_or(0.0);
    let mut mesh = junction_mesh(&plate.arms, DVec3::ZERO, core_tex, period, along);
    // the roads' markings carried on to it: their lines, and stop lines (not across an arm
    // with a zebra crossing, which is where its traffic stops)
    let paint: Vec<Option<ArmPaint>> = plate
        .order
        .iter()
        .zip(&plate.arms)
        .map(|(i, g)| assets.spline(&files[*i]).and_then(|m| arm_paint(&road_paint(&m), g.vdir)))
        .collect();
    let zebra_arm: Vec<bool> = plate.arms.iter().map(|g| opts.crossing_at(g.out)).collect();
    let marks = plate_marks(&mesh, &plate.arms, &plate.lanes, &paint, &zebra_arm, true, DVec3::ZERO);
    add_marks(&mut mesh, &marks, &tex_dir, DVec3::ZERO);
    if !zebras.is_empty() {
        let white = tex_dir.join("om_zebra_white.png");
        if !white.exists() {
            let _ = image::RgbaImage::from_pixel(8, 8, image::Rgba([235, 235, 230, 255])).save(&white);
        }
        let mat = mesh.material(Some(white));
        for (mid, out, lw, rw) in &zebras {
            let (d, r) = (roads::dir(*out), roads::right(*out));
            // stripes 0.5 m wide, a metre apart, 3 m along the road (laid on the roadway's
            // surface, which lies over the roads' lines by their profile)
            let mut x = -lw + 0.25;
            while x < rw - 0.25 {
                let c = *mid + r * x;
                mesh.paint_strip(c - d * 1.5, c + d * 1.5, 0.5, mat);
                x += 1.0;
            }
        }
    }
    // every texture beside the object
    let mut names = Vec::new();
    for t in &mesh.textures {
        let Some(p) = t else {
            names.push(String::new());
            continue;
        };
        let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        if p.parent() != Some(tex_dir.as_path()) {
            copy_texture(p, &tex_dir);
        }
        names.push(name);
    }
    // (the plate is written from what it is made of, in its own frame: junctions that come out
    // the same share one object file, one model and one set of textures, so the game loads and
    // draws it once. Named by what is in it)
    let x_text = mesh_x(&mesh, &names);
    let mut body = String::new();
    if plate_lights.iter().any(|l| l.is_some()) {
        body.push_str(&light_program(opts));
    }
    // where roads join it (the editors snap to these)
    for (g, i) in plate.arms.iter().zip(&plate.order) {
        let file = &files[*i];
        let p = g.end;
        // (OMSI's order: x, height, forward)
        let _ = write!(body, "[splinehelper]\n{file}\n{}\n{}\n{}\n{}\n0.1\n0\n\n", num(p.x), num(p.z), num(p.y), num(roads::norm_deg(g.out + 180.0)));
    }
    for p in &recs {
        let s = &p.seg;
        let grad = if s.length > 0.01 { (p.z1 - p.z0) / s.length } else { 0.0 };
        let _ = write!(body, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n", num(s.start.x), num(s.start.y), num(p.z0), num(s.heading), num(s.radius), num(s.length), num(grad), num(grad), p.kind, num(p.width), p.direction, p.blinker);
        if let Some(l) = p.light {
            let _ = write!(body, "[use_traffic_light]\n{l}\n\n");
        }
    }
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&x_text, &body, opts.line()).hash(&mut h);
        h.finish()
    };
    let name = format!("junction_{hash:016x}");
    let model_path = dir.join("model").join(format!("{name}.x"));
    let sco_path = dir.join(format!("{name}.sco"));
    if !(model_path.is_file() && sco_path.is_file()) {
        std::fs::write(&model_path, &x_text)?;
        let mut sco = String::new();
        let _ = write!(sco, "{}\n\n[friendlyname]\nJunction {}\n\n[groups]\n1\nopenMapper\n\n[surface]\n\n[absheight]\n\n[mesh]\n{name}.x\n\n", opts.line(), &format!("{hash:016x}")[..6]);
        sco.push_str(&body);
        std::fs::write(&sco_path, omsi_cfg::encode_text(&sco.replace('\n', "\r\n")))?;
    }
    crate::hot::own_change();
    let k = tile_of(centre.truncate());
    let k = if doc.tiles.contains_key(&k) { k } else { arms[0].key };
    let org = tile_origin(k);
    let rel = format!("Sceneryobjects\\openMapper\\{folder}\\{name}.sco");
    let t = doc.tiles.get_mut(&k).ok_or_else(|| anyhow::anyhow!("no tile"))?;
    t.tile.objects.push(MapObject { file: rel, id, pos: [centre.x - org.x, centre.y - org.y, centre.z], rot: [roads::norm_deg(plate.turn), 0.0, 0.0], ..Default::default() });
    t.touch_content();
    // the signal heads, on poles, bound to the junction's lights by [varparent]
    let left_hand = doc.global.left_hand_traffic;
    for (i, g) in geo.iter().enumerate() {
        let Some(light) = light_of[i] else { continue };
        if !lanes.iter().any(|l| l.arm == i && l.kind == 0 && l.incoming) {
            continue;
        }
        // (traffic coming in keeps to the left of the way out where it keeps right)
        let side = if left_hand { roads::right(g.out) * (g.right_w + 1.2) } else { -roads::right(g.out) * (g.left_w + 1.2) };
        let p = g.end.truncate() + side - roads::dir(g.out) * 1.0;
        let hk = tile_of(p);
        if !doc.tiles.contains_key(&hk) {
            continue;
        }
        let org = tile_origin(hk);
        for (file, z, extra) in [(LIGHT_POLE, 0.0, Vec::new()), (LIGHT_HEAD, 2.55, vec![light.to_string()])] {
            let oid = doc.new_id();
            let t = doc.tiles.get_mut(&hk).unwrap();
            let flag = extra.len() as i32;
            t.tile.objects.push(MapObject { file: file.to_string(), id: oid, pos: [p.x - org.x, p.y - org.y, z], rot: [g.out, 0.0, 0.0], flag, extra, var_parent: Some(id), ..Default::default() });
            t.touch_content();
        }
    }
    Ok(id)
}

/// The folder a map's junction objects are written in (`Sceneryobjects\openMapper\<folder>`
/// in the content folder), made with its `model` and `texture` folders.
pub fn object_dir(content: &Path, folder: &str) -> std::io::Result<PathBuf> {
    let sco_root = std::fs::read_dir(content)?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("sceneryobjects")).unwrap_or(false)).unwrap_or_else(|| content.join("Sceneryobjects"));
    let dir = sco_root.join("openMapper").join(folder);
    std::fs::create_dir_all(dir.join("model"))?;
    std::fs::create_dir_all(dir.join("texture"))?;
    Ok(dir)
}

/// The name of a map's folder as its junction objects' folder is named.
pub fn folder_name(doc: &Document) -> String {
    doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into()).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// A road coming into a drawn junction: its road type, where it ends (in the drawing's frame)
/// and the heading out along it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateArm {
    pub file: String,
    pub pos: DVec2,
    pub out: f64,
    /// The road's spline runs into the junction here (its end is at it); else it runs away
    /// from it (its start is). The road type's left and right and the lanes' ways are the
    /// spline's own.
    pub at_end: bool,
}

/// The plate of a junction drawn in the complex junction editor: the roadway, the kerbs round
/// each corner and the footways, as the Roads tool makes them, and the lanes through it.
pub struct Plate {
    mesh: Mesh,
    /// Its `[path]` records (drawing's frame).
    pub paths: String,
    /// The same lanes as lines (drawing's frame): the way along it, the path's kind and its
    /// direction (0 along the line, 1 against, 2 both).
    pub lanes: Vec<(Seg, i32, i32)>,
    /// The paint on it (drawing's frame).
    #[allow(dead_code)]
    marks: Vec<Mark>,
}

/// The plate between `arms` (the roads end at their points, which the plate meets). `tex_dir`
/// is where the roadway's own asphalt is written (None: nothing is written, the roadway has no
/// texture: for a drawing on screen). None when fewer than two arms, or a road type is not loaded.
pub fn layout_plate(assets: &mut Assets, arms: &[PlateArm], tex_dir: Option<&Path>, asphalt: Option<&Path>) -> Option<Plate> {
    if arms.len() < 2 {
        return None;
    }
    let (mut geo, mut lanes, mut models) = (Vec::new(), Vec::new(), Vec::new());
    for (i, a) in arms.iter().enumerate() {
        let model = assets.spline(&a.file)?;
        let end = DVec3::new(a.pos.x, a.pos.y, 0.0);
        // (the spline's own way there: into the junction, or away from it)
        let along = if a.at_end { roads::norm_deg(a.out + 180.0) } else { a.out };
        let r = roads::right(along);
        geo.push(arm_geo_of(&model, false, a.at_end, end, a.out, 0.0));
        lanes.extend(lanes_at(&model, 1.0, i, a.at_end, |x, z| end + DVec3::new(r.x * x, r.y * x, z), along));
        models.push(model);
    }
    // (made round the junction's own middle, then moved to the drawing's frame)
    let mid = arms.iter().map(|a| a.pos).sum::<DVec2>() / arms.len() as f64;
    let centre = DVec3::new(mid.x, mid.y, 0.0);
    let recs = paths(&lanes, centre, &vec![None; arms.len()]);
    let widest = geo.iter().filter(|g| g.road_tex.is_some()).max_by(|a, b| (a.left_w + a.right_w).total_cmp(&(b.left_w + b.right_w)));
    let (core_tex, period) = match (widest.and_then(|g| g.road_tex.clone()), tex_dir) {
        (Some((tex, u, range, period, width, lane)), Some(dir)) => {
            let out = plain_like(&tex, u, range / width, period.clamp(2.0, 40.0), lane, dir);
            (out.or_else(|| asphalt.map(Path::to_path_buf)), period.clamp(2.0, 40.0))
        }
        (_, Some(_)) => (asphalt.map(Path::to_path_buf), 8.0),
        _ => (None, 8.0),
    };
    let along = widest.map(|g| g.out).unwrap_or(0.0);
    let mut mesh = junction_mesh(&geo, centre, core_tex, period.max(1.0), along);
    // the roads' markings carried on to it (where its textures are written); stop lines where
    // three roads or more meet, not on a joint between two
    let marks = match tex_dir {
        Some(dir) => {
            let paint: Vec<Option<ArmPaint>> = models.iter().zip(&geo).map(|(m, g)| arm_paint(&road_paint(m), g.vdir)).collect();
            let marks = plate_marks(&mesh, &geo, &lanes, &paint, &[], arms.len() >= 3, centre);
            add_marks(&mut mesh, &marks, dir, centre);
            marks
        }
        None => Vec::new(),
    };
    for v in &mut mesh.verts {
        v.0[0] += mid.x;
        v.0[1] += mid.y;
    }
    let mut paths = String::new();
    for p in &recs {
        let s = &p.seg;
        let start = s.start + mid;
        let grad = if s.length > 0.01 { (p.z1 - p.z0) / s.length } else { 0.0 };
        let _ = write!(paths, "[path]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n", num(start.x), num(start.y), num(p.z0), num(s.heading), num(s.radius), num(s.length), num(grad), num(grad), p.kind, num(p.width), p.direction, p.blinker);
    }
    let lanes = recs.iter().map(|p| (Seg { start: p.seg.start + mid, ..p.seg }, p.kind, p.direction)).collect();
    Some(Plate { mesh, paths, lanes, marks })
}

impl Plate {
    /// The plate's triangles seen from above for drawing them with their pictures: each with
    /// its texture (None: the roadway, which the drawing paints flat), and the uv at its corners.
    pub fn flat_textured(&self) -> Vec<([DVec2; 3], Option<PathBuf>, [[f32; 2]; 3])> {
        self.mesh
            .tris
            .iter()
            .map(|(t, mat)| {
                let v = |i: usize| &self.mesh.verts[t[i]];
                let tex = self.mesh.textures.get(*mat).cloned().flatten();
                ([0, 1, 2].map(|i| DVec2::new(v(i).0[0], v(i).0[1])), tex, [0, 1, 2].map(|i| [v(i).2[0] as f32, v(i).2[1] as f32]))
            })
            .collect()
    }

    /// The plate as a mesh of an editable object: its materials added to `materials`, its
    /// textures (name to file) to `textures`.
    pub fn model_mesh(&self, materials: &mut Vec<openmapper_mesh::Material>, textures: &mut std::collections::HashMap<String, PathBuf>) -> openmapper_mesh::Mesh {
        let slot: Vec<u32> = self
            .mesh
            .textures
            .iter()
            .map(|t| {
                let name = t.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string());
                if let (Some(n), Some(p)) = (&name, t) {
                    textures.entry(n.clone()).or_insert_with(|| p.clone());
                }
                let mat = openmapper_mesh::Material { name: name.clone().unwrap_or_else(|| "Plate".into()), color: [1.0, 1.0, 1.0, 1.0], texture: name, ..Default::default() };
                let i = materials.iter().position(|x| x.texture == mat.texture && x.color == mat.color && x.alpha == mat.alpha).unwrap_or_else(|| {
                    materials.push(mat);
                    materials.len() - 1
                });
                i as u32
            })
            .collect();
        let mut out = openmapper_mesh::Mesh::default();
        for (t, mat) in &self.mesh.tris {
            let ids: Vec<u32> = t.iter().map(|&i| out.add_vertex(glam::Vec3::new(self.mesh.verts[i].0[0] as f32, self.mesh.verts[i].0[1] as f32, self.mesh.verts[i].0[2] as f32))).collect();
            let uvs: Vec<glam::Vec2> = t.iter().map(|&i| glam::Vec2::new(self.mesh.verts[i].2[0] as f32, self.mesh.verts[i].2[1] as f32)).collect();
            out.add_face(ids, uvs, slot.get(*mat).copied().unwrap_or(0));
        }
        out.weld(1e-5);
        out
    }

    /// Written as a scenery object of its own (`Sceneryobjects\openMapper\<folder>\junction_<hash>.sco`)
    /// for a drawing put on the map: its relative path.
    pub fn write_object(&self, content: &Path, folder: &str) -> anyhow::Result<String> {
        let dir = object_dir(content, folder)?;
        let tex_dir = dir.join("texture");
        let mut names = Vec::new();
        for t in &self.mesh.textures {
            let Some(p) = t else {
                names.push(String::new());
                continue;
            };
            if p.parent() != Some(tex_dir.as_path()) {
                copy_texture(p, &tex_dir);
            }
            names.push(p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
        }
        let x_text = mesh_x(&self.mesh, &names);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&x_text, &self.paths).hash(&mut h);
            h.finish()
        };
        let name = format!("junction_{hash:016x}");
        let (model_path, sco_path) = (dir.join("model").join(format!("{name}.x")), dir.join(format!("{name}.sco")));
        if !(model_path.is_file() && sco_path.is_file()) {
            std::fs::write(&model_path, &x_text)?;
            let sco = format!("{}\n\n[friendlyname]\nJunction {}\n\n[groups]\n1\nopenMapper\n\n[surface]\n\n[absheight]\n\n[mesh]\n{name}.x\n\n{}", Options::default().line(), &format!("{hash:016x}")[..6], self.paths);
            std::fs::write(&sco_path, omsi_cfg::encode_text(&sco.replace('\n', "\r\n")))?;
        }
        crate::hot::own_change();
        Ok(format!("Sceneryobjects\\openMapper\\{folder}\\{name}.sco"))
    }
}

/// Delete the junction files (`junction_<id>.sco` and its `.x`) in `dir` whose object file
/// (`Sceneryobjects\\openMapper\\<folder>\\junction_<id>.sco`, lower case) is not in `used`,
/// leaving those numbered from `keep_from` on (made in this session: undo may need them).
pub fn remove_unused(dir: &Path, folder: &str, used: &std::collections::HashSet<String>, keep_from: i64) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if !(lower.starts_with("junction_") && lower.ends_with(".sco")) {
            continue;
        }
        let rel = format!("sceneryobjects\\openmapper\\{}\\{lower}", folder.to_lowercase());
        let num: Option<i64> = lower["junction_".len()..lower.len() - 4].parse().ok();
        // (one named by its contents is shared; a recent one may be wanted by undo)
        let recent = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() < 86_400).unwrap_or(true);
        if used.contains(&rel) || num.map(|n| n >= keep_from).unwrap_or(recent) {
            continue;
        }
        let stem = &name[..name.len() - 4];
        let _ = std::fs::remove_file(e.path());
        let _ = std::fs::remove_file(dir.join("model").join(format!("{stem}.x")));
        n += 1;
    }
    if n > 0 {
        crate::hot::own_change();
    }
    n
}

/// The objects that belong to junction `id` (its signal heads and poles).
pub fn children(doc: &Document, id: i64) -> Vec<(TileKey, i64)> {
    doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.var_parent == Some(id)).map(move |o| (*k, o.id))).collect()
}

/// Where two polylines cross: (point, distance along a, distance along b).
pub fn crossings(a: &[(DVec2, f64)], b: &[(DVec2, f64)]) -> Vec<(DVec2, f64, f64)> {
    let mut out = Vec::new();
    for wa in a.windows(2) {
        for wb in b.windows(2) {
            let (p, r) = (wa[0].0, wa[1].0 - wa[0].0);
            let (q, s) = (wb[0].0, wb[1].0 - wb[0].0);
            let rxs = r.perp_dot(s);
            if rxs.abs() < 1e-9 {
                continue;
            }
            let t = (q - p).perp_dot(s) / rxs;
            let u = (q - p).perp_dot(r) / rxs;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                let pt = p + r * t;
                // (a crossing right on a joint of the lines is found on both sides of it)
                if !out.iter().any(|o: &(DVec2, f64, f64)| (o.0 - pt).length() < 0.01) {
                    out.push((pt, wa[0].1 + (wa[1].1 - wa[0].1) * t, wb[0].1 + (wb[1].1 - wb[0].1) * u));
                }
            }
        }
    }
    out
}

/// A spline's centre line every 2 m: (point, distance along it).
pub fn polyline(c: &SplineCurve) -> Vec<(DVec2, f64)> {
    let n = (c.length / 2.0).ceil().max(1.0) as usize;
    (0..=n).map(|i| {
        let s = c.length * i as f64 / n as f64;
        (c.point_at(s).truncate(), s)
    }).collect()
}

/// The asphalt texture junction plates are drawn with: the installation's own.
pub fn asphalt_texture(omsi: &Path) -> Option<PathBuf> {
    let dir = std::fs::read_dir(omsi).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("texture")).unwrap_or(false))?;
    for name in ["str_asphdrk.bmp", "str_asphalt.bmp", "asphalt.bmp"] {
        if let Some(p) = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false)) {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn options_are_read_back_from_the_line_written() {
        let o = Options { lights: true, main_green: 42.0, side_green: 17.0, crossings: vec![90.0, 270.5] };
        let text = format!("{}\r\n\r\n[friendlyname]\r\nJunction 1\r\n", o.line());
        let back = Options::parse(&text);
        assert_eq!(back.lights, true);
        assert_eq!((back.main_green, back.side_green), (42.0, 17.0));
        assert!(back.crossing_at(91.0) && back.crossing_at(270.0) && !back.crossing_at(0.0));
        assert_eq!(Options::parse("[friendlyname]\nx"), Options::default());
    }

    #[test]
    fn the_main_road_is_the_pair_most_in_line_and_its_light_comes_first() {
        assert_eq!(main_road(&[0.5, 90.0, 270.0]), (1, 2));
        let p = light_program(&Options { lights: true, main_green: 30.0, side_green: 20.0, crossings: vec![] });
        // a cycle of both greens, their yellows, red-yellows and all-red times
        assert!(p.starts_with("[traffic_lights_group]\n64\n"), "{p}");
        assert_eq!(p.matches("[traffic_light]").count(), 2);
    }

    #[test]
    fn only_junction_files_nothing_uses_from_before_are_removed() {
        let dir = std::env::temp_dir().join(format!("om-gc-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("model")).unwrap();
        for n in [5, 6, 50] {
            std::fs::write(dir.join(format!("junction_{n}.sco")), "x").unwrap();
            std::fs::write(dir.join("model").join(format!("junction_{n}.x")), "x").unwrap();
        }
        std::fs::write(dir.join("other.sco"), "x").unwrap();
        let used: std::collections::HashSet<String> = ["sceneryobjects\\openmapper\\m\\junction_6.sco".to_string()].into();
        // 5 unused and old: removed; 6 used; 50 made this session (from 40 on)
        assert_eq!(remove_unused(&dir, "m", &used, 40), 1);
        assert!(!dir.join("junction_5.sco").exists() && !dir.join("model/junction_5.x").exists());
        assert!(dir.join("junction_6.sco").exists() && dir.join("junction_50.sco").exists() && dir.join("other.sco").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn two_roads_crossing_meet_once() {
        let a: Vec<(DVec2, f64)> = (0..=10).map(|i| (DVec2::new(i as f64 * 10.0 - 50.0, 0.0), i as f64 * 10.0)).collect();
        let b: Vec<(DVec2, f64)> = (0..=10).map(|i| (DVec2::new(5.0, i as f64 * 10.0 - 50.0), i as f64 * 10.0)).collect();
        let x = crossings(&a, &b);
        assert_eq!(x.len(), 1);
        assert!((x[0].0 - DVec2::new(5.0, 0.0)).length() < 1e-9);
        assert!((x[0].1 - 55.0).abs() < 1e-9 && (x[0].2 - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_kerb_rounds_a_corner_smoothly() {
        // an arm running out east with its left kerb 4 m north of the centre line, ending
        // 12 m out; the next arm running out north with its right kerb 4 m east
        let a = DVec2::new(12.0, 4.0);
        let b = DVec2::new(4.0, 12.0);
        let segs = kerb_round(a, 90.0, b, 0.0);
        assert!(!segs.is_empty(), "{segs:?}");
        let end = segs.last().unwrap();
        assert!((end.end() - b).length() < 1e-6, "ends at {:?}", end.end());
        for w in segs.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-6);
            assert!((roads::norm_deg(w[0].end_heading() - w[1].heading + 180.0) - 180.0).abs() < 1e-6);
        }
        // it rounds the corner through its inside
        let mid = segs[segs.len() / 2].point(segs[segs.len() / 2].length * 0.5);
        assert!(mid.x > 4.0 && mid.y > 4.0 && mid.x < 12.0 && mid.y < 12.0, "{mid:?}");
    }

    fn test_road() -> SplineModel {
        let lane = |x: f32, direction: i32| omsi_scenery::PathDef { kind: 0, start: [x, 0.0, 0.0], width: 3.0, direction, ..Default::default() };
        SplineModel {
            def: omsi_scenery::Spline {
                profiles: vec![omsi_scenery::SplineProfile {
                    texture: 0,
                    points: vec![omsi_scenery::SplineProfilePoint { x: -4.0, z: 0.0, ..Default::default() }, omsi_scenery::SplineProfilePoint { x: 4.0, z: 0.0, ..Default::default() }],
                }],
                paths: vec![lane(2.0, 0), lane(-2.0, 1)],
                ..Default::default()
            },
            materials: Vec::new(),
            half_width: 4.0,
        }
    }

    /// Three roads out of `centre`, turned by `turn`, made into a junction: its object.
    fn t_junction(doc: &mut Document, assets: &mut Assets, content: &Path, centre: DVec2, turn: f64, ids: [i64; 3], lengths: [f64; 3]) -> MapObject {
        let mut arms = Vec::new();
        for ((rel, id), length) in [0.0, 90.0, 270.0].into_iter().zip(ids).zip(lengths) {
            let out = roads::norm_deg(turn + rel);
            let p = centre + roads::dir(out) * 10.0;
            crate::selection::tests::add_spline(doc, (0, 0), id, p.x, p.y, out, length, 0, 0);
            arms.push(Arm { key: (0, 0), id, at_end: false });
        }
        let id = make_with(doc, assets, content, None, DVec3::new(centre.x, centre.y, 0.0), &arms, &Options::default()).unwrap();
        doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.id == id).unwrap().clone()
    }

    /// The same roads meeting at the same angles make one object, wherever the junction lies, whichever
    /// way it faces, whichever road came first and however long the roads run on.
    #[test]
    fn junctions_of_the_same_shape_share_one_object() {
        let roots = crate::doc::tests::scratch("junction-shared");
        let content = roots.content.clone();
        std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("shared");
        let mut assets = Assets::new(roots.omsi.clone(), content.clone());
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(test_road())));
        let a = t_junction(&mut doc, &mut assets, &content, DVec2::new(100.0, 100.0), 0.0, [1, 2, 3], [20.0, 20.0, 20.0]);
        let b = t_junction(&mut doc, &mut assets, &content, DVec2::new(200.0, 200.0), 37.0, [6, 5, 4], [13.5, 41.2, 27.3]);
        assert_eq!(a.file, b.file);
        assert!((roads::norm_deg(b.rot[0] - a.rot[0]) - 37.0).abs() < 1e-6, "{:?} {:?}", a.rot, b.rot);
        let dir = omsi_cfg::resolve_path(&content, &a.file);
        let dir = dir.parent().unwrap();
        let scos = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "sco")).count();
        assert_eq!(scos, 1);
        // each junction's lanes meet its roads' lanes on the map, however it is turned
        assets.object_wait(&a.file, 5000).expect("the object reads back");
        for centre in [DVec2::new(100.0, 100.0), DVec2::new(200.0, 200.0)] {
            let (_, ends) = crate::lanes::near(&doc, &mut assets, centre, 14.0);
            let inner: Vec<_> = ends.iter().filter(|e| (e.pos.truncate() - centre).length() < 12.5).collect();
            assert!(inner.len() >= 12, "{}", inner.len());
            assert!(inner.iter().all(|e| e.linked), "{centre:?} {inner:?}");
        }
        let _ = std::fs::remove_dir_all(&content);
    }

    /// A road 8 m wide (lanes 3 m each way at ±2 m, or both forward when `one_way`) whose
    /// texture runs once across it: grey asphalt, with (when `painted`) a dashed white line
    /// down its middle and a solid one 32 cm in from each edge. The texture is written in `dir`.
    fn painted_road(dir: &Path, painted: bool, one_way: bool) -> SplineModel {
        let (w, h) = (256u32, 64u32);
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([92, 92, 90, 255]));
        if painted {
            for x in 0..w {
                let u = (x as f64 + 0.5) / w as f64;
                for y in 0..h {
                    let edge = (0.03..0.05).contains(&u) || (0.95..0.97).contains(&u);
                    let dash = (0.49..0.51).contains(&u) && y < h / 2;
                    if edge || dash {
                        img.put_pixel(x, y, image::Rgba([228, 228, 220, 255]));
                    }
                }
            }
        }
        let tex = dir.join(if painted { "painted.png" } else { "plain.png" });
        img.save(&tex).unwrap();
        let mut m = test_road();
        m.def.profiles[0].points[0].u = 0.0;
        m.def.profiles[0].points[1].u = 1.0;
        m.materials = vec![crate::assets::Material { texture: Some(tex), color: [1.0; 4], alpha: crate::assets::Alpha::Opaque, glow: None }];
        if one_way {
            for p in &mut m.def.paths {
                p.direction = 0;
            }
        }
        m
    }

    fn test_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("om-junction-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_roads_lines_are_read_from_its_texture() {
        let dir = test_dir("scan");
        let p = scan_paint(&painted_road(&dir, true, false));
        assert_eq!(p.lines.len(), 3, "{p:?}");
        let (l, c, r) = (p.lines[0], p.lines[1], p.lines[2]);
        // (the texture's u runs 0..1 over x -4..4: the lines' middles at -3.68, 0 and 3.68)
        assert!((l.x + 3.68).abs() < 0.05 && (c.x).abs() < 0.05 && (r.x - 3.68).abs() < 0.05, "{p:?}");
        assert!(l.solid && !c.solid && r.solid, "{p:?}");
        assert!(p.lines.iter().all(|l| l.inner && (0.1..0.25).contains(&l.width)), "{p:?}");
        assert!(p.rgb.iter().all(|c| *c >= 200), "{p:?}");
        // a road with no paint has no lines
        assert!(scan_paint(&painted_road(&dir, false, false)).lines.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The plate where roads of the type meet, coming in along `outs` (each road's spline starting at
    /// the plate, or ending there with `at_end`).
    fn plate_of(model: SplineModel, dir: &Path, outs: &[(f64, bool)]) -> Plate {
        plate_at(model, dir, outs, 12.0)
    }

    /// The same with the roads' ends `reach` metres out of the middle.
    fn plate_at(model: SplineModel, dir: &Path, outs: &[(f64, bool)], reach: f64) -> Plate {
        let roots = crate::doc::tests::scratch(&format!("plate-{}", dir.file_name().unwrap().to_string_lossy()));
        let mut assets = Assets::new(roots.omsi.clone(), roots.content.clone());
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(model)));
        let arms: Vec<PlateArm> = outs.iter().map(|(out, at_end)| PlateArm { file: "Splines\\road.sli".into(), pos: DVec2::new(50.0, 50.0) + roads::dir(*out) * reach, out: *out, at_end: *at_end }).collect();
        let plate = layout_plate(&mut assets, &arms, Some(dir), None).unwrap();
        let _ = std::fs::remove_dir_all(&roots.content);
        plate
    }

    /// Where a mark lies on arm `out` (its mouth 12 m out of (50, 50)): how far in from the
    /// mouth, and its span across (right of the way out).
    fn mark_place(m: &Mark, out: f64) -> (f64, f64, f64) {
        mark_place_at(m, out, 12.0)
    }

    fn mark_place_at(m: &Mark, out: f64, reach: f64) -> (f64, f64, f64) {
        let mouth = DVec2::new(50.0, 50.0) + roads::dir(out) * reach;
        let (d, r) = (roads::dir(out), roads::right(out));
        let (a, b) = (m.a - mouth, m.b - mouth);
        (-(a + b).dot(d) * 0.5, a.dot(r).min(b.dot(r)), a.dot(r).max(b.dot(r)))
    }

    #[test]
    fn three_and_four_arm_plates_carry_stop_lines_on_their_incoming_arms() {
        for outs in [vec![0.0, 90.0, 270.0], vec![0.0, 90.0, 180.0, 270.0]] {
            let dir = test_dir(&format!("stop{}", outs.len()));
            let arms: Vec<(f64, bool)> = outs.iter().map(|o| (*o, false)).collect();
            let plate = plate_of(painted_road(&dir, true, false), &dir, &arms);
            for (i, out) in outs.iter().enumerate() {
                let stops: Vec<&Mark> = plate.marks.iter().filter(|m| m.arm == i && m.stop).collect();
                // (a T: the road running on through, east-west, keeps its way and has none)
                if outs.len() == 3 && i > 0 {
                    assert!(stops.is_empty(), "arm {i}: {:?}", plate.marks);
                    continue;
                }
                assert_eq!(stops.len(), 1, "arm {i}: {:?}", plate.marks);
                // across the lane coming in (left of the way out: -3.5..-0.5), from the edge line
                // to the middle line, a few metres in from the mouth, short of the crossing road
                let (t, s0, s1) = mark_place(stops[0], *out);
                assert!((s0 + 3.76).abs() < 0.1 && (s1 + 0.08).abs() < 0.1, "arm {i}: {s0} {s1}");
                assert!(t > 2.0 && t < 7.5, "arm {i}: {t}");
                // the road's three lines run on in up to it
                let lines: Vec<&Mark> = plate.marks.iter().filter(|m| m.arm == i && !m.stop).collect();
                assert_eq!(lines.len(), 3, "arm {i}: {lines:?}");
                for l in &lines {
                    let (tl, _, _) = mark_place(l, *out);
                    assert!(tl > 1.0 && tl * 2.0 <= t - STOP_W * 0.5 + 0.05, "arm {i}: {tl} {t}");
                }
            }
            // drawn as quads with the paint's own picture, over the roadway
            let mat = plate.mesh.textures.iter().position(|t| t.as_ref().is_some_and(|p| p.file_name().unwrap().to_string_lossy().starts_with("om_marking_"))).expect("a marking texture");
            assert!(plate.mesh.textures[mat].as_ref().unwrap().is_file());
            let tris: Vec<_> = plate.mesh.tris.iter().filter(|(_, m)| *m == mat).collect();
            assert!(tris.len() >= outs.len() * 4 * 2, "{}", tris.len());
            for (f, _) in tris {
                for v in f {
                    let p = plate.mesh.verts[*v].0;
                    assert!((p[2] - MARK_LIFT).abs() < 1e-6, "{p:?}");
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A lane's texture: grey grain (deterministic), with a thin dashed line (3 px of 256,
    /// half the length) near its right edge at px 109..=111 when `dashed`.
    fn lane_texture(dashed: bool) -> omsi_texture::Image {
        let (w, h) = (256usize, 128usize);
        let mut seed = 12345u32;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let g = if dashed && (109..=111).contains(&x) && y < h / 2 { 225 } else { 84 + ((seed >> 24) % 16) as u8 };
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        omsi_texture::Image { width: w as u32, height: h as u32, rgba, has_alpha: false }
    }

    /// The plate's asphalt is cut from a lane of the road's texture; a dashed line in it, however
    /// thin (one the old test, looking at seven columns at each edge of the lane, stepped
    /// over), is left out, so the plate shows no stripes of dashes.
    #[test]
    fn plain_asphalt_leaves_out_a_thin_dashed_line() {
        let dir = test_dir("asphalt");
        // (the texture runs once over 8 m; the lane's middle at u 0.25, a lane 3 m of path)
        let (u, u_per_m, lane_m) = (0.25, 0.125, 3.0 / 0.9);
        let img = lane_texture(true);
        let (c, band_m, band_u, mirror) = clean_band(&img, u, u_per_m, lane_m);
        assert!(mirror && band_m >= 0.8, "{c} {band_m} {band_u}");
        let (a, b) = ((c - band_u * 0.5) * 256.0, (c + band_u * 0.5) * 256.0);
        assert!(b < 109.0 - 2.0 || a > 112.0 + 2.0, "the band {a}..{b} takes in the line");
        // and the picture made from it has no stripe brighter or darker than the grain
        let tex = dir.join("lane.png");
        image::RgbaImage::from_raw(256, 128, img.rgba.clone()).unwrap().save(&tex).unwrap();
        let out = clean_asphalt_image(&tex, u, u_per_m, 8.0, lane_m).unwrap();
        let lum = |p: &image::Rgba<u8>| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 765.0;
        let mut all: Vec<f32> = out.pixels().map(lum).collect();
        all.sort_by(f32::total_cmp);
        let base = all[all.len() / 2];
        assert!(out.pixels().all(|p| (lum(p) - base).abs() < 0.1), "{base} {:?} {:?}", all.first(), all.last());
        // with no paint in the lane, the whole lane is taken, repeated (slabs keep their grid)
        let (c, band_m, _, mirror) = clean_band(&lane_texture(false), u, u_per_m, lane_m);
        assert!(!mirror && (c - u).abs() < 1e-9 && (band_m - lane_m).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Where the arms come in at different heights the roadway folds along its fan; the paint
    /// lies on it all the same, every piece just over the triangle it is on.
    #[test]
    fn paint_lies_on_a_plate_whose_arms_are_at_different_heights() {
        let dir = test_dir("fold");
        let model = painted_road(&dir, true, false);
        let paint = scan_paint(&model);
        let (mut geo, mut lanes) = (Vec::new(), Vec::new());
        for (i, (out, h)) in [0.0, 90.0, 180.0, 270.0].into_iter().zip([0.0, 0.3, -0.2, 0.1]).enumerate() {
            let end = DVec3::new(0.0, 0.0, h) + (roads::dir(out) * 7.0).extend(0.0);
            let r = roads::right(out);
            geo.push(arm_geo_of(&model, false, false, end, out, 0.0));
            lanes.extend(lanes_at(&model, 1.0, i, false, |x, z| end + DVec3::new(r.x * x, r.y * x, z), out));
        }
        let mut mesh = junction_mesh(&geo, DVec3::ZERO, None, 8.0, 0.0);
        let arm_paints: Vec<Option<ArmPaint>> = geo.iter().map(|g| arm_paint(&paint, g.vdir)).collect();
        let marks = plate_marks(&mesh, &geo, &lanes, &arm_paints, &[], true, DVec3::ZERO);
        assert_eq!(marks.iter().filter(|m| m.stop).count(), 4, "{marks:?}");
        let before = mesh.tris.len();
        add_marks(&mut mesh, &marks, &dir, DVec3::ZERO);
        assert!(mesh.tris.len() > before + 8);
        for (f, _) in &mesh.tris[before..] {
            let c = f.iter().map(|v| DVec3::from(mesh.verts[*v].0)).sum::<DVec3>() / 3.0;
            let z = mesh.roadway_z(c.truncate()).expect("paint on the roadway");
            assert!((c.z - z - MARK_LIFT).abs() < 1e-6, "{c:?} over {z}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Roads cut back only just clear of each other (tight corners): no room for the lines to
    /// run on, but each arm still stops at its mouth, short of the crossing road's lanes.
    #[test]
    fn a_tight_plate_has_its_stop_lines_at_the_mouths() {
        let dir = test_dir("tight");
        let outs = [0.0, 90.0, 180.0, 270.0];
        let arms: Vec<(f64, bool)> = outs.iter().map(|o| (*o, false)).collect();
        let plate = plate_at(painted_road(&dir, true, false), &dir, &arms, 4.3);
        assert!(plate.marks.iter().all(|m| m.stop), "{:?}", plate.marks);
        for (i, out) in outs.iter().enumerate() {
            let stop = plate.marks.iter().find(|m| m.arm == i).unwrap_or_else(|| panic!("arm {i}: {:?}", plate.marks));
            let (t, s0, s1) = mark_place_at(stop, *out, 4.3);
            assert!(t > 0.0 && t + STOP_W * 0.5 <= 4.3 - 3.5, "arm {i}: {t}");
            assert!(s0 < -3.0 && s1 > -0.5 && s1 < 0.0, "arm {i}: {s0} {s1}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_one_way_road_leaving_has_no_stop_line_and_unpainted_roads_have_no_marks() {
        let dir = test_dir("oneway");
        // a one-way road running in from the east and on out to the north and the south...
        let plate = plate_of(painted_road(&dir, true, true), &dir, &[(90.0, true), (0.0, false), (180.0, false), (270.0, true)]);
        let stops: Vec<usize> = plate.marks.iter().filter(|m| m.stop).map(|m| m.arm).collect();
        assert_eq!(stops, vec![0, 3], "{:?}", plate.marks);
        // ...stops right across its whole roadway, edge line to edge line
        let (_, s0, s1) = mark_place(plate.marks.iter().find(|m| m.stop).unwrap(), 90.0);
        assert!((s0 + 3.76).abs() < 0.1 && (s1 - 3.76).abs() < 0.1, "{s0} {s1}");
        // a yard drive's plate: nothing painted, no paint's picture
        let plain = plate_of(painted_road(&dir, false, false), &dir, &[(0.0, false), (90.0, false), (270.0, false)]);
        assert!(plain.marks.is_empty());
        assert!(!plain.mesh.textures.iter().flatten().any(|p| p.to_string_lossy().contains("om_marking_")));
        // two roads joined: lines, no stop line
        let joint = plate_of(painted_road(&dir, true, false), &dir, &[(0.0, false), (150.0, false)]);
        assert!(!joint.marks.is_empty() && joint.marks.iter().all(|m| !m.stop), "{:?}", joint.marks);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An automatic junction's object carries the paint, and the plate made without it is
    /// another object (named by what is in it).
    #[test]
    fn a_junction_object_carries_its_roads_markings() {
        let roots = crate::doc::tests::scratch("junction-paint");
        let content = roots.content.clone();
        std::fs::create_dir_all(content.join("Sceneryobjects")).unwrap();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.dir = content.join("maps").join("paint");
        let mut assets = Assets::new(roots.omsi.clone(), content.clone());
        let tex = test_dir("object");
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(painted_road(&tex, true, false))));
        let a = t_junction(&mut doc, &mut assets, &content, DVec2::new(100.0, 100.0), 0.0, [1, 2, 3], [20.0, 20.0, 20.0]);
        assets.splines.insert("splines\\road.sli".into(), crate::assets::Slot::Ready(std::sync::Arc::new(painted_road(&tex, false, false))));
        let b = t_junction(&mut doc, &mut assets, &content, DVec2::new(200.0, 200.0), 0.0, [4, 5, 6], [20.0, 20.0, 20.0]);
        assert_ne!(a.file, b.file);
        let sco = omsi_cfg::resolve_path(&content, &a.file);
        let x = std::fs::read_to_string(sco.parent().unwrap().join("model").join(format!("{}.x", sco.file_stem().unwrap().to_string_lossy()))).unwrap();
        let name = x.lines().find(|l| l.contains("om_marking_")).expect("the paint's texture in the model").trim().trim_matches(|c| c == '"' || c == ';').to_string();
        assert!(sco.parent().unwrap().join("texture").join(&name).is_file(), "{name}");
        let _ = std::fs::remove_dir_all(&content);
        let _ = std::fs::remove_dir_all(&tex);
    }

    #[test]
    fn a_junction_mesh_reads_back_through_openomsi() {
        let arm = |end: DVec3, out: f64| ArmGeo {
            end,
            out,
            right_w: 3.5,
            left_w: 3.5,
            right: vec![SideProfile { texture: None, points: vec![(0.0, 0.0, 0.0, 0.2), (0.0, 0.15, 0.1, 0.2), (2.0, 0.15, 0.5, 0.2)] }],
            left: vec![SideProfile { texture: None, points: vec![(0.0, 0.0, 0.0, 0.2), (0.0, 0.15, 0.1, 0.2), (2.0, 0.15, 0.5, 0.2)] }],
            v0: 0.0,
            vdir: 1.0,
            road_tex: None,
            road_z: 0.0,
        };
        let arms = vec![arm(DVec3::new(10.0, 0.0, 0.0), 90.0), arm(DVec3::new(-10.0, 0.0, 0.0), 270.0), arm(DVec3::new(0.0, 10.0, 0.0), 0.0)];
        let mesh = junction_mesh(&arms, DVec3::ZERO, None, 8.0, 0.0);
        assert!(mesh.tris.len() > 20);
        // every roadway triangle faces up
        for (f, mat) in &mesh.tris {
            if *mat == 0 {
                let p: Vec<DVec3> = f.iter().map(|i| DVec3::from(mesh.verts[*i].0)).collect();
                assert!((p[1] - p[0]).cross(p[2] - p[0]).z >= 0.0);
            }
        }
        let x = mesh_x(&mesh, &[String::new()]);
        let dir = std::env::temp_dir().join(format!("om-junction-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("j.x"), x).unwrap();
        let m = omsi_o3d::load_mesh(&dir.join("j.x")).unwrap();
        assert_eq!(m.triangles.len(), mesh.tris.len() * 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
