//! The crossing path editor: the `[path]`s of a crossing object of the map, drawn from above
//! and edited - where they start and end, how wide, how steep, which way, which light they
//! wait at, which paths they block - then written back into a copy of the object.
//!
//! It does not build a crossing (the complex junction editor does): it changes one that is
//! already on the map, as OMSI 2's own `OmsiObjEditP` does, without touching the original
//! `.sco` in the installation.

use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};
use glam::{DVec2, DVec3};

use crate::assets::Assets;
use crate::crossing_edit::{self, outward_flow, Crossing, Helper, Joint, Path, Sheet, TURN_LEFT, TURN_NONE, TURN_RIGHT};
use crate::doc::Sel;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::library::Library;

const STREET: Color32 = Color32::from_rgb(232, 190, 96);
const FOOTWAY: Color32 = Color32::from_rgb(104, 184, 224);
const RAIL: Color32 = Color32::from_rgb(196, 148, 208);
const PICKED: Color32 = Color32::from_rgb(255, 148, 40);
const HANDLE: Color32 = Color32::from_rgb(238, 238, 238);
const SNAPPED: Color32 = Color32::from_rgb(96, 208, 128);
const DIM: Color32 = Color32::from_gray(150);
const BAD: Color32 = Color32::from_rgb(240, 110, 96);
const GOOD: Color32 = Color32::from_rgb(120, 200, 130);

/// How near a click must be to an end to take it (pixels).
const GRAB: f32 = 9.0;

/// Where a dragged end set out from. `spawned` is how many pieces were inserted after `root`
/// on the last rebuild, so the next move replaces them instead of piling more on.
#[derive(Clone)]
struct ReachAnchor {
    root: usize,
    start: DVec2,
    heading: f64,
    z: f64,
    /// The rise the path had at its start when it was taken.
    slope: f64,
    spawned: usize,
}

/// The crossing path editor's window and its state.
pub struct CrossingPaths {
    pub open: bool,
    /// The object being edited.
    target: Option<Sel>,
    /// The map it was opened on, and the `.sco` the object named then.
    map_dir: std::path::PathBuf,
    from_rel: String,
    /// The `.sco` the map names, and where it is.
    rel: String,
    abs: std::path::PathBuf,
    /// The bytes it was read from: the code page it is written back in.
    bytes: Vec<u8>,
    sheet: Option<Sheet>,
    /// The crossing as it was when opened (Revert).
    seen: Crossing,
    /// The path being edited, and the end being dragged of it.
    selected: Option<usize>,
    drag: Option<(usize, bool)>,
    /// The canvas: pixels per metre, and the point of the object at its centre.
    scale: f32,
    centre: DVec2,
    fit: bool,
    /// The object's own mesh, laid under the paths so the crossing itself is seen: the rel it
    /// was read for, its triangles (x, y, z in the object's frame, three to a triangle), and
    /// the extent of them.
    footprint: Option<(String, Vec<f32>, [f64; 4])>,
    /// `Draw`: a path is laid by dragging from one point to another, as a road is laid.
    draw: bool,
    /// Where such a path starts while the mouse is down, and the height it starts at (an end
    /// it was taken from has one).
    draw_from: Option<DVec2>,
    draw_from_z: f32,
    /// The end the drawn path started from, when it was taken onto one: the new path is laid
    /// in that path's way, at its rise.
    draw_join: Option<Joint>,
    /// A new path is being started from a nearby path's end: the next click on one (in the
    /// drawing or on the map) makes it. Its ends are `nearby`.
    from_nearby: bool,
    /// The picked path is being carried on to a nearby path's end: the next click on one makes
    /// the piece or pieces that join them, the picked path itself left as it is.
    complete_to: bool,
    /// The ends of the paths around the crossing - other objects' and the roads' own lanes -
    /// with the way each runs away from it, while `from_nearby` is on.
    nearby: Vec<Joint>,
    /// The end of a path taken with the mouse on the map (the object in 3D).
    map_drag: Option<(usize, bool)>,
    /// The height (world) the map drag keeps the ray on: the end's own height when it was
    /// taken, so the point stays under the cursor instead of sliding on the object's ground.
    map_drag_plane: Option<f64>,
    /// Where a dragged end set out from, so the piece can be rebuilt as several forward
    /// pieces instead of one loop, and the extras from the last rebuild can be replaced.
    drag_anchor: Option<ReachAnchor>,
    /// A drag of the start or end rotation, so the pieces it splits into can be rebuilt
    /// from the path as the drag began.
    rot_drag: Option<RotGesture>,
    /// The object's mesh, unthinned, so a new path can sit on the surface. The drawing uses
    /// [`CrossingPaths::footprint`], which is thinned.
    surface: std::sync::Arc<Vec<f32>>,
    surface_for: String,
    /// A path was picked on the map: the list is scrolled to it where it is out of sight.
    reveal: bool,
    /// Where the end being dragged has snapped to, in the object's frame, so the drawing can
    /// show it (None: not snapped).
    snap_mark: Option<DVec3>,
    /// The paths are shown on the map itself (the object in 3D) rather than drawn from above:
    /// the window keeps the properties, and the map keeps the crossing.
    view_3d: bool,
    /// The map's view is put on the crossing on the next frame (`Show on the map`).
    frame_on_map: bool,
    /// The file this window writes is already a copy the editor made (so it writes in place).
    copied: bool,
    /// How saving writes the file: in the object's own file (a `.bak` keeps the old one), or
    /// into a copy of the object of its own.
    save_mode: SaveMode,
    /// The window's close button was pressed with changes in it: the question is on screen
    /// (Save and close, Close without saving, Cancel).
    ask_close: bool,
    /// The undo and redo of the crossing's own edits, and the state the last record was taken
    /// of (the file is not in the map's history: it is not the map that changes).
    undo: Vec<Crossing>,
    redo: Vec<Crossing>,
    mark: Crossing,
    /// When the last record was taken (seconds of the interface's clock), so a drag of a
    /// number's arrows is one step, not thirty.
    last_push: f64,
    message: Option<(String, bool)>,
    /// The page shown: the paths, or the traffic light program (as OMSI's two editors).
    tab: Tab,
    /// The light and the phase picked in the light program, and the block edge being dragged.
    light: Option<usize>,
    phase: Option<usize>,
    edge: Option<(usize, usize)>,
    /// Pixels a second takes on the timeline.
    sec_px: f32,
    /// Whether the timeline follows the window's width (the whole cycle in view).
    sec_fit: bool,
    /// Crossing objects of the map, to pick from when nothing is selected.
    candidates: Vec<((i32, i32), i64, String)>,
    /// The room the drawing side was given, so a test can say it is not squeezed out.
    #[cfg(test)]
    last_canvas: Vec2,
    /// Where the drawing was laid out, so a test can press in it.
    #[cfg(test)]
    last_canvas_rect: Option<egui::Rect>,
    /// Where the picked path's numbers and the path list begin, so a test can say which comes
    /// first.
    #[cfg(test)]
    last_numbers_y: Option<f32>,
    #[cfg(test)]
    last_list_y: Option<f32>,
    /// Where the timeline was drawn, so a test can click a block in it.
    #[cfg(test)]
    last_timeline: Option<egui::Rect>,
    /// Where the window itself was drawn, so a test can say its body is not collapsed.
    #[cfg(test)]
    last_window: Option<egui::Rect>,
}

impl Default for CrossingPaths {
    fn default() -> Self {
        CrossingPaths {
            open: false,
            target: None,
            rel: String::new(),
            abs: Default::default(),
            bytes: Vec::new(),
            sheet: None,
            seen: Crossing::default(),
            map_dir: Default::default(),
            from_rel: String::new(),
            selected: None,
            drag: None,
            scale: 14.0,
            centre: DVec2::ZERO,
            fit: true,
            footprint: None,
            draw: false,
            draw_from: None,
            draw_from_z: 0.0,
            draw_join: None,
            from_nearby: false,
            complete_to: false,
            nearby: Vec::new(),
            map_drag: None,
            map_drag_plane: None,
            drag_anchor: None,
            rot_drag: None,
            surface: std::sync::Arc::new(Vec::new()),
            surface_for: String::new(),
            reveal: false,
            snap_mark: None,
            view_3d: false,
            frame_on_map: false,
            copied: false,
            save_mode: SaveMode::InPlace,
            ask_close: false,
            undo: Vec::new(),
            redo: Vec::new(),
            mark: Crossing::default(),
            last_push: 0.0,
            message: None,
            tab: Tab::Paths,
            light: None,
            phase: None,
            edge: None,
            sec_px: 14.0,
            sec_fit: true,
            candidates: Vec::new(),
            #[cfg(test)]
            last_canvas: Vec2::ZERO,
            #[cfg(test)]
            last_canvas_rect: None,
            #[cfg(test)]
            last_numbers_y: None,
            #[cfg(test)]
            last_list_y: None,
            #[cfg(test)]
            last_timeline: None,
            #[cfg(test)]
            last_window: None,
        }
    }
}

/// Whether an object of the map is one the crossing editor can open: one of openMapper's own
/// junctions, or an object of the catalogue that has `[path]`s of its own.
///
/// The catalogue files every signal head and every light program under Junctions as well (they
/// are junction pieces), and none of those has a path: offering them made a list of red lights.
fn is_crossing(lib: &Library, file: &str) -> bool {
    crate::editor::is_own_junction(file) || lib.find(file).is_some_and(|a| a.paths > 0)
}

/// Where the mark that a `.sco` is this editor's own copy sits.
fn sidecar(abs: &std::path::Path) -> std::path::PathBuf {
    abs.with_extension("crossing.json")
}

/// The most triangles the drawing of the object takes: a big model is thinned evenly, so the
/// crossing is seen under the paths without the drawing slowing down.
const FOOTPRINT_TRIS: usize = 12000;

/// The object's own mesh seen from above: what the crossing looks like under its paths.
fn footprint_of(assets: &mut Assets, rel: &str) -> Option<(String, Vec<f32>, [f64; 4])> {
    let model = assets.object(rel)?;
    let total: usize = model.parts.iter().map(|p| p.mesh.indices.len() / 3).sum();
    if total == 0 {
        return None;
    }
    let stride = (total.div_ceil(FOOTPRINT_TRIS)).max(1);
    let mut tris: Vec<f32> = Vec::with_capacity((total / stride + 1) * 9);
    let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    let mut n = 0usize;
    for part in &model.parts {
        let m = &part.mesh;
        for tri in m.indices.chunks_exact(3) {
            let take = n % stride == 0;
            n += 1;
            if !take {
                continue;
            }
            let pts: Option<Vec<_>> = tri.iter().map(|&i| m.positions.get(i as usize).copied()).collect();
            let Some(pts) = pts else { continue };
            for p in pts {
                tris.extend_from_slice(&[p.x, p.y, p.z]);
                lo = lo.min(DVec2::new(p.x as f64, p.y as f64));
                hi = hi.max(DVec2::new(p.x as f64, p.y as f64));
            }
        }
    }
    if tris.len() < 9 {
        return None;
    }
    Some((rel.to_string(), tris, [lo.x, lo.y, hi.x, hi.y]))
}

/// Every triangle of the object, for sitting a path on the deck. Capped so a huge model
/// cannot make a drag walk millions of them; the drawing's own mesh is thinned further.
fn surface_tris(assets: &mut Assets, rel: &str) -> Option<Vec<f32>> {
    let model = assets.object(rel)?;
    let total: usize = model.parts.iter().map(|p| p.mesh.indices.len() / 3).sum();
    if total == 0 {
        return None;
    }
    let stride = (total.div_ceil(60_000)).max(1);
    let mut tris = Vec::new();
    let mut n = 0usize;
    for part in &model.parts {
        let m = &part.mesh;
        for tri in m.indices.chunks_exact(3) {
            let take = n % stride == 0;
            n += 1;
            if !take {
                continue;
            }
            let pts: Option<Vec<_>> = tri.iter().map(|&i| m.positions.get(i as usize).copied()).collect();
            let Some(pts) = pts else { continue };
            for p in pts {
                tris.extend_from_slice(&[p.x, p.y, p.z]);
            }
        }
    }
    (!tris.is_empty()).then_some(tris)
}

/// Move one end of a path to `q` (the object's frame); `snapped` is the heading of the path
/// it was snapped to, when the crossing's own Ctrl snapping found one.
/// Snapping only moves an end: the heading is left to `set_start` / `set_end`, which work it
/// out from the chord. Taking the other path's heading as well turned a path round by 180
/// degrees whenever the two ran opposite ways.
fn drag_end(crossing: &mut Crossing, i: usize, at_end: bool, q: DVec2) {
    let Some(p) = crossing.paths.get_mut(i) else { return };
    if at_end {
        p.set_end(DVec3::new(q.x, q.y, p.end().z));
    } else {
        p.set_start(DVec3::new(q.x, q.y, p.start().z));
    }
}

/// The reach a snap works at, from the drawing's zoom: ends closer than this on the screen
/// are taken as the same point (14 pixels), so a snap is something seen, not a stray one.
fn snap_reach(scale: f32) -> f64 {
    (14.0 / scale.max(0.6) as f64).clamp(0.05, 8.0)
}

/// Where a dragged end of path `i` is taken: another path's end within `reach`, unless Ctrl is
/// held (the mouse's own point then).
/// The end another path has near `q`, with the way it runs away from it: what a dragged end
/// is put onto, so the two pieces meet not only at the point but in their way and their rise.
fn snap_joint(crossing: &Crossing, i: usize, q: DVec2, reach: f64, ctrl: bool) -> Option<Joint> {
    if ctrl {
        return None;
    }
    crossing.nearest_joint(DVec3::new(q.x, q.y, 0.0), reach, i)
}

/// Put path `i`'s start onto a joint: at its point, in its way, and at its rise - so the
/// piece joins the next one smoothly instead of merely touching it. An end is not turned
/// this way: see [`apply_reach`], which splits it into pieces instead.
fn join_start(crossing: &mut Crossing, i: usize, j: Joint) {
    let Some(p) = crossing.paths.get_mut(i) else { return };
    p.set_start(DVec3::new(j.pos.x, j.pos.y, j.pos.z));
    p.fit_start_tangent(j.heading);
    p.set_start_height_and_slope(j.pos.z, j.slope);
}

/// Put a path's end at height `z` by the rise its slopes carry: the `.sco` has no end height
/// of its own, so both slopes are set to the rise the end asks for.
fn set_end_height(p: &mut Path, z: f64) {
    let l = p.length();
    if l > 1e-3 {
        let rise = ((z - p.start().z) / l) as f32;
        p.v[6] = rise;
        p.v[7] = rise;
    }
}

/// The signed turn from `from` to `to`, in degrees, between -180 and 180.
fn ang_delta(from: f64, to: f64) -> f64 {
    let d = (to - from).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Height of the object's surface at `p` (its own frame): the highest triangle that covers
/// the point, which is the deck rather than the underside. None when the point is off the mesh.
fn surface_z(tris: &[f32], p: DVec2) -> Option<f64> {
    let mut best: Option<f64> = None;
    for t in tris.chunks_exact(9) {
        let a = DVec2::new(t[0] as f64, t[1] as f64);
        let b = DVec2::new(t[3] as f64, t[4] as f64);
        let c = DVec2::new(t[6] as f64, t[7] as f64);
        let (za, zb, zc) = (t[2] as f64, t[5] as f64, t[8] as f64);
        let v0 = c - a;
        let v1 = b - a;
        let v2 = p - a;
        let den = v0.x * v1.y - v1.x * v0.y;
        if den.abs() < 1e-12 {
            continue;
        }
        let u = (v2.x * v1.y - v1.x * v2.y) / den;
        let v = (v0.x * v2.y - v2.x * v0.y) / den;
        if u >= -0.02 && v >= -0.02 && u + v <= 1.02 {
            let z = za * (1.0 - u - v) + zb * v + zc * u;
            best = Some(best.map(|h| h.max(z)).unwrap_or(z));
        }
    }
    best
}

/// A drag of one end's rotation: `original` is the path it started from, and `spawned` is
/// how many pieces the last rebuild inserted after it.
struct RotGesture {
    index: usize,
    at_end: bool,
    original: Path,
    spawned: usize,
}

/// Rebuild `template` so it leaves on `h0` and arrives on `h1`, the two ends staying where
/// they are. One piece when a single arc does that; several when it would loop.
fn retarget(template: &Path, h0: f64, h1: f64) -> Vec<Path> {
    let a = template.start().truncate();
    let b = template.end().truncate();
    if (b - a).length() < 0.25 {
        return vec![template.clone()];
    }
    let h0 = crate::roads::norm_deg(h0);
    let h1 = crate::roads::norm_deg(h1);
    if let Some(segs) = crate::reshape::fit(crate::reshape::End { pos: a, heading: Some(h0) }, crate::reshape::End { pos: b, heading: Some(h1) }) {
        return lay_segments(template, &segs);
    }
    let segs = connect_segs(a, h0, b, h1);
    if segs.is_empty() {
        return Vec::new();
    }
    let mut z = template.start().z;
    let mut out = Vec::new();
    for seg in &segs {
        let q = lay_seg(template, seg, z);
        z = q.end().z;
        out.push(q);
    }
    if let Some(last) = out.last_mut() {
        last.set_end_height_and_slope(template.end().z, template.slope_end());
    }
    out
}

/// `segs` as paths copied from `template`. One keeps the slopes; several share the rise
/// and the last one ends at the template's own end height.
fn lay_segments(template: &Path, segs: &[crate::roads::Seg]) -> Vec<Path> {
    if segs.is_empty() {
        return Vec::new();
    }
    if segs.len() == 1 {
        let mut q = template.clone();
        let s = &segs[0];
        q.v[0] = s.start.x as f32;
        q.v[1] = s.start.y as f32;
        q.v[3] = s.heading as f32;
        q.v[4] = s.radius as f32;
        q.v[5] = s.length as f32;
        return vec![q];
    }
    let end_z = template.end().z;
    let s1 = template.slope_end();
    let s0 = template.slope_start();
    let total = segs.iter().map(|s| s.length).sum::<f64>().max(0.01);
    let mut z = template.start().z;
    let mut along = 0.0;
    let mut out = Vec::new();
    for (k, s) in segs.iter().enumerate() {
        let mut q = template.clone();
        q.v[0] = s.start.x as f32;
        q.v[1] = s.start.y as f32;
        q.v[2] = z as f32;
        q.v[3] = s.heading as f32;
        q.v[4] = s.radius as f32;
        q.v[5] = s.length as f32;
        if k + 1 == segs.len() {
            q.set_end_height_and_slope(end_z, s1);
        } else {
            let f0 = along / total;
            let f1 = (along + s.length) / total;
            q.v[6] = (s0 + (s1 - s0) * f0) as f32;
            q.v[7] = (s0 + (s1 - s0) * f1) as f32;
            z = q.end().z;
        }
        along += s.length;
        out.push(q);
    }
    out
}

/// Take `count` paths out after `from` (not including `from`), and the block rules that
/// named them.
fn drop_inserted(paths: &mut Vec<Path>, from: usize, count: usize) {
    if count == 0 || from > paths.len() {
        return;
    }
    let to = (from + count).min(paths.len());
    let n = (to - from) as i32;
    paths.drain(from..to);
    let from_i = from as i32;
    let to_i = from_i + n;
    for p in paths.iter_mut() {
        p.blocks.retain(|(a, _)| *a < from_i || *a >= to_i);
        for (a, _) in &mut p.blocks {
            if *a >= to_i {
                *a -= n;
            }
        }
    }
}

/// Insert `extra` just after `index`, and the block rules that name later paths follow.
fn insert_after(paths: &mut Vec<Path>, index: usize, mut extra: Vec<Path>) {
    if extra.is_empty() || index >= paths.len() {
        return;
    }
    let n = extra.len() as i32;
    let at = index as i32 + 1;
    for p in paths.iter_mut() {
        for (a, _) in &mut p.blocks {
            if *a >= at {
                *a += n;
            }
        }
    }
    for p in &mut extra {
        for (a, _) in &mut p.blocks {
            if *a >= at {
                *a += n;
            }
        }
    }
    for (k, p) in extra.into_iter().enumerate() {
        paths.insert(index + 1 + k, p);
    }
}

/// A join further than this from the path it joins is not a join: no crossing is tens of
/// kilometres across, so the joint must be in another frame (the map's, say). Refused rather
/// than built, since a join made from it is a path of millions of metres.
const FAR_JOIN: f64 = 10_000.0;

/// Past this deflection of the chord (degrees), one arc to the point turns back on itself
/// (`length / chord = θ / sin θ`) and the way is split into pieces instead.
const MAX_DEFLECT: f64 = 70.0;

/// One arc that leaves `start` on `heading`, turns `turn` degrees (its end is that much
/// further round), and is `chord` metres across. Its chord lies at `heading + turn / 2`.
fn arc_piece(start: DVec2, heading: f64, turn: f64, chord: f64) -> crate::roads::Seg {
    use crate::roads::Seg;
    let heading = crate::roads::norm_deg(heading);
    let chord = chord.max(0.01);
    if turn.abs() < 1e-4 {
        return Seg { start, heading, length: chord, radius: 0.0 };
    }
    let half = (turn.to_radians() * 0.5).abs();
    let r = chord / (2.0 * half.sin());
    Seg { start, heading, length: half * 2.0 * r, radius: r * turn.signum() }
}

/// Pieces from `start` along `heading` to `end`, each an arc that begins in exactly the way
/// the piece before it ended. One piece when the point lies ahead or off to the side;
/// several when one arc would loop back.
fn forward_segs(start: DVec2, heading: f64, end: DVec2) -> Vec<crate::roads::Seg> {
    use crate::roads;
    let mut segs = Vec::new();
    let mut pos = start;
    let mut h = roads::norm_deg(heading);
    for _ in 0..12 {
        let dist = (end - pos).length();
        if dist < 0.4 {
            return segs;
        }
        let deflect = ang_delta(h, roads::heading_of(end - pos));
        if deflect.abs() <= MAX_DEFLECT {
            // (an arc that leaves on `h` and passes through the point leaves its chord at
            // half its turn: the turn is twice the deflection)
            segs.push(arc_piece(pos, h, 2.0 * deflect, dist));
            return segs;
        }
        // A piece of the way round: part of the turn, so the way comes round a bit and the
        // point is nearer each time. Kept wide enough to be a bend (see `most_turn`).
        let chord = (dist * 0.6).clamp(1.0, 30.0);
        let turn = (2.0 * deflect.clamp(-35.0, 35.0)).clamp(-80.0, 80.0);
        let turn = turn.clamp(-most_turn(chord), most_turn(chord));
        let s = arc_piece(pos, h, turn, chord);
        h = s.end_heading();
        pos = s.end();
        segs.push(s);
    }
    if (end - pos).length() >= 0.4 {
        let dist = (end - pos).length();
        let deflect = ang_delta(h, roads::heading_of(end - pos)).clamp(-MAX_DEFLECT, MAX_DEFLECT);
        let turn = (2.0 * deflect).clamp(-most_turn(dist), most_turn(dist));
        segs.push(arc_piece(pos, h, turn, dist));
    }
    segs
}

/// A join shorter than this is closed with one straight piece: over a few centimetres there is
/// no room for a bend, and what a bend gives there is a knot.
const SHORT_JOIN: f64 = 2.0;

/// The tightest radius a join's piece may have (m). Below it the piece is not a bend but a
/// knot: a path of a few centimetres curled round on itself, which reads as the wrong way.
/// A tight junction turn is a metre or two; a centimetre is not a path at all.
const MIN_PIECE_RADIUS: f64 = 1.5;

/// The most a piece may turn and still be a bend of at least `MIN_PIECE_RADIUS` across `chord`.
fn most_turn(chord: f64) -> f64 {
    (2.0 * (chord / (2.0 * MIN_PIECE_RADIUS)).clamp(-1.0, 1.0).asin().to_degrees()).abs()
}

/// Two arcs that leave `start` on `h_a`, meet tangentially at a point in between, and arrive
/// at `end` on `h_b` - each arriving exactly in the way the next leaves, so the join has no
/// kink and both ways are kept. The meeting point is found by its own heading, taken in small
/// steps: a heading that puts the point in front of both ends is the one used, preferring the
/// widest radii.
///
/// A pair is only used when both of its arcs are real bends (`MIN_PIECE_RADIUS` at least) and
/// both are at least a metre long: a pair of centimetre knots closes the join in geometry but
/// is not a path anyone can drive.
fn s_join(start: DVec2, h_a: f64, end: DVec2, h_b: f64) -> Option<Vec<crate::roads::Seg>> {
    use crate::roads::{self, Seg};
    let (h_a, h_b) = (roads::norm_deg(h_a), roads::norm_deg(h_b));
    let v = end - start;
    let mut best: Option<(f64, Vec<Seg>)> = None;
    for step in -36..=36 {
        let hm = roads::norm_deg(h_a + step as f64 * 5.0);
        let (t1, t2) = (ang_delta(h_a, hm), ang_delta(hm, h_b));
        if t1.abs() > 170.0 || t2.abs() > 170.0 {
            continue;
        }
        let u1 = roads::dir(roads::norm_deg(h_a + t1 * 0.5));
        let u2 = roads::dir(roads::norm_deg(hm + t2 * 0.5));
        let det = u1.x * u2.y - u1.y * u2.x;
        if det.abs() < 1e-6 {
            continue;
        }
        let a = (v.x * u2.y - v.y * u2.x) / det;
        let b = (u1.x * v.y - u1.y * v.x) / det;
        if !(a > 1.0 && b > 1.0) {
            continue;
        }
        let r1 = if t1.abs() < 1e-4 { f64::MAX } else { a / (2.0 * (t1.to_radians() * 0.5).abs().sin()) };
        let r2 = if t2.abs() < 1e-4 { f64::MAX } else { b / (2.0 * (t2.to_radians() * 0.5).abs().sin()) };
        let narrow = r1.min(r2);
        // (a pair this tight is a knot, not a bend)
        if narrow < MIN_PIECE_RADIUS {
            continue;
        }
        let mid = start + u1 * a;
        let segs = vec![arc_piece(start, h_a, t1, a), arc_piece(mid, hm, t2, b)];
        if best.as_ref().is_none_or(|(r, _)| narrow > *r) {
            best = Some((narrow, segs));
        }
    }
    best.map(|(_, segs)| segs)
}

/// Pieces from `start` leaving on `h_a` and arriving at `end` on `h_b`. Every piece begins in
/// exactly the way the piece before it ended, so no join has a kink in it; the first leaves in
/// `h_a` and the last arrives in `h_b`.
///
/// One arc only when one arc can do both (an arc leaving on `h_a` arrives on `2 * phi - h_a`,
/// so meeting `h_b` is a coincidence of the geometry). Otherwise two arcs that meet tangentially
/// ([`s_join`]), and when the point is too far round for that, forward pieces that bring the way
/// round. The way the join leaves is never turned to make a single piece fit.
///
/// A join of a couple of metres or less is one straight piece: no bend of a usable radius fits
/// in it, and a bend squeezed in would be a knot of centimetres that reads as the wrong way.
fn connect_segs(start: DVec2, h_a: f64, end: DVec2, h_b: f64) -> Vec<crate::roads::Seg> {
    use crate::roads;
    let mut segs = Vec::new();
    let mut pos = start;
    let mut h = roads::norm_deg(h_a);
    let h_b = roads::norm_deg(h_b);
    for _ in 0..12 {
        let dist = (end - pos).length();
        if dist < 0.4 {
            return segs;
        }
        if dist < SHORT_JOIN {
            // (a straight to the point: `roads::straight` will not make one under half a
            // metre, and half a metre is exactly what a join this short is)
            if dist >= 0.05 {
                segs.push(crate::roads::Seg { start: pos, heading: roads::heading_of(end - pos), length: dist, radius: 0.0 });
            }
            return segs;
        }
        let phi = roads::heading_of(end - pos);
        // (one arc: its chord must lie at half its turn, and it must turn into `h_b`)
        let turn = ang_delta(h, h_b);
        if turn.abs() <= 140.0 && ang_delta(roads::norm_deg(h + turn * 0.5), phi).abs() <= 3.0 && (turn.abs() < 1e-4 || dist / (2.0 * (turn.to_radians() * 0.5).abs().sin()) >= MIN_PIECE_RADIUS) {
            segs.push(arc_piece(pos, h, turn, dist));
            return segs;
        }
        if let Some(two) = s_join(pos, h, end, h_b) {
            segs.extend(two);
            return segs;
        }
        // (too far round for either: go on forward, the way coming round as it goes)
        let chord = (dist * 0.6).clamp(1.0, 30.0);
        let turn = (2.0 * ang_delta(h, phi).clamp(-35.0, 35.0)).clamp(-80.0, 80.0);
        let turn = turn.clamp(-most_turn(chord), most_turn(chord));
        let s = arc_piece(pos, h, turn, chord);
        h = s.end_heading();
        pos = s.end();
        segs.push(s);
    }
    if (end - pos).length() >= 0.4 {
        let dist = (end - pos).length();
        let deflect = ang_delta(h, roads::heading_of(end - pos)).clamp(-MAX_DEFLECT, MAX_DEFLECT);
        let turn = (2.0 * deflect).clamp(-most_turn(dist), most_turn(dist));
        segs.push(arc_piece(pos, h, turn, dist));
    }
    segs
}

/// `template` laid along `seg`, starting at height `z0`. The seg is an arc (or a straight) of
/// a known way out, turn and length, so it is written as it is: no fitting, and no fallback
/// that would break the way the next piece is to begin.
fn lay_seg(template: &Path, s: &crate::roads::Seg, z0: f64) -> Path {
    let mut q = template.clone();
    q.v[0] = s.start.x as f32;
    q.v[1] = s.start.y as f32;
    q.v[2] = z0 as f32;
    q.v[3] = s.heading as f32;
    q.v[4] = s.radius as f32;
    q.v[5] = s.length as f32;
    let edges = (s.end() - s.start).length();
    // (a piece never becomes a loop: if what came out is far longer than the gap it spans,
    // a straight to the same point is written instead)
    if !q.length().is_finite() || q.length() > edges * 4.0 + 8.0 {
        q.v[3] = s.heading as f32;
        q.v[4] = 0.0;
        q.v[5] = edges.max(0.05) as f32;
    }
    q
}
/// What the last piece of a join ends at.
enum PieceEnd {
    /// A free end: seated on the object's surface where the mesh has one.
    Surface,
    /// Another path's end: its height and its rise, exactly.
    At { z: f64, slope: f64 },
}

/// `template`'s shape laid along `segs`, beginning at `a` with the rise `s_a` there. The
/// pieces between sit on the surface, so a join follows the deck rather than cutting through
/// it. `inherit`: the first piece keeps the template's own rules (a drag of an end, rather
/// than a new piece added beside it).
fn lay_join(template: &Path, a: DVec3, s_a: f64, segs: &[crate::roads::Seg], end: PieceEnd, inherit: bool, tris: &[f32]) -> Vec<Path> {
    let n = segs.len();
    let mut out: Vec<Path> = Vec::new();
    let mut z = a.z;
    for (k, seg) in segs.iter().enumerate() {
        let start_z = if k == 0 { a.z } else { z };
        let mut q = lay_seg(template, seg, start_z);
        if k == 0 && inherit {
            q.blocks = template.blocks.clone();
            q.crossing_problem = template.crossing_problem;
        } else {
            q.blocks.clear();
            q.crossing_problem = false;
        }
        q.light = template.light;
        let want = surface_z(tris, seg.end());
        if k + 1 == n {
            match end {
                // (a stub of a metre cannot climb to another path's height without an absurd
                // rise - 68% at the start of a 0.7 m piece. It keeps the rise the join leaves
                // at; the height is off by centimetres, which is nothing at that length.)
                PieceEnd::At { z, slope } if q.length() >= SHORT_JOIN => q.set_end_height_and_slope(z, slope),
                PieceEnd::At { .. } => {
                    q.v[6] = s_a as f32;
                    q.v[7] = s_a as f32;
                }
                PieceEnd::Surface => match want {
                    Some(z) if q.length() >= SHORT_JOIN || (z - start_z).abs() < 0.5 => set_end_height(&mut q, z),
                    Some(z) => {
                        // (short: the rise is what it can be, not what the deck asks for)
                        let l = q.length().max(0.05);
                        q.v[6] = ((z - start_z) / l) as f32;
                        q.v[7] = q.v[6];
                    }
                    None => {
                        q.v[6] = s_a as f32;
                        q.v[7] = s_a as f32;
                    }
                },
            }
        } else if k == 0 {
            // (the first piece leaves at the rise the path had, and still lands on the deck)
            let l = q.length();
            q.v[6] = s_a as f32;
            if l >= SHORT_JOIN {
                let z1 = want.unwrap_or(a.z + s_a * l);
                q.v[7] = (2.0 * (z1 - a.z) / l - s_a) as f32;
            } else {
                // (too short to bend the rise: it climbs at the one it came with)
                q.v[7] = s_a as f32;
            }
        } else if let Some(z1) = want {
            if q.length() >= SHORT_JOIN || (z1 - start_z).abs() < 0.5 {
                set_end_height(&mut q, z1);
            } else {
                let l = q.length().max(0.05);
                q.v[6] = ((z1 - start_z) / l) as f32;
                q.v[7] = q.v[6];
            }
        }
        z = q.end().z;
        out.push(q);
    }
    out
}

/// Replace `anchor.root` with the pieces that reach `aim`, deleting the ones the previous
/// move inserted. `joint` is the other path's end when the drag has snapped onto one: the
/// pieces then arrive in its way and at its height. The departure the path had is kept
/// either way - the end is split into pieces rather than the path turned round. The last
/// piece's index is returned.
fn apply_reach(crossing: &mut Crossing, anchor: &mut ReachAnchor, aim: DVec2, tris: &[f32], joint: Option<Joint>) -> Option<usize> {
    if anchor.root >= crossing.paths.len() {
        return None;
    }
    let template = crossing.paths[anchor.root].clone();
    drop_inserted(&mut crossing.paths, anchor.root + 1, anchor.spawned);
    anchor.spawned = 0;
    let legs = match joint {
        Some(j) => connect_segs(anchor.start, anchor.heading, aim, j.heading),
        None => forward_segs(anchor.start, anchor.heading, aim),
    };
    if legs.is_empty() {
        return None;
    }
    let end = match joint {
        Some(j) => PieceEnd::At { z: j.pos.z, slope: j.slope },
        None => PieceEnd::Surface,
    };
    let a = DVec3::new(anchor.start.x, anchor.start.y, anchor.z);
    let pieces = lay_join(&template, a, anchor.slope, &legs, end, true, tris);
    if pieces.is_empty() {
        return None;
    }
    let mut made = pieces.into_iter();
    if let Some(first) = made.next() {
        crossing.paths[anchor.root] = first;
    }
    for (k, q) in made.enumerate() {
        crossing.paths.insert(anchor.root + 1 + k, q);
        anchor.spawned += 1;
    }
    Some(anchor.root + legs.len() - 1)
}

/// A path carried on from the end of path `i`: it leaves in the way that one arrives, at
/// the same rise, on the same radius, a short piece (not a copy of a kilometre-long path,
/// and not straightened — a straight piece is only what the previous one already was).
/// None when `i` is not a path (a plain piece of ten metres is added instead).
fn continue_path(crossing: &mut Crossing, i: Option<usize>) -> Option<usize> {
    let q = match i.and_then(|i| crossing.paths.get(i)).cloned() {
        Some(p) => {
            let end = p.end();
            let c = p.curve();
            let heading = c.heading_at(c.length.max(0.0));
            let rise = p.slope_end();
            let mut q = p.clone();
            q.v[0] = end.x as f32;
            q.v[1] = end.y as f32;
            q.v[2] = end.z as f32;
            q.v[3] = heading as f32;
            // (the radius stays: zeroing it made the next piece a straight line that no
            // longer followed the one it came from)
            q.v[5] = (p.length().clamp(8.0, 15.0)) as f32;
            q.v[6] = rise as f32;
            q.v[7] = rise as f32;
            q.v[11] = TURN_NONE as f32;
            q.blocks.clear();
            q.crossing_problem = false;
            q
        }
        None => Path::default(),
    };
    crossing.paths.push(q);
    Some(crossing.paths.len() - 1)
}

/// Move one end to `q`, keeping the way the path runs at the other end and the rise it
/// leaves with, and keeping this end's height. The point then stays where the cursor put
/// it, and the piece still meets the path it was carried on from.
fn move_end(crossing: &mut Crossing, i: usize, at_end: bool, q: DVec2) {
    let Some(p) = crossing.paths.get(i) else { return };
    let (hold, slope, z) = if at_end {
        (p.heading(), p.slope_start(), p.end().z)
    } else {
        let c = p.curve();
        (c.heading_at(c.length.max(0.0)), p.slope_start(), p.start().z)
    };
    drag_end(crossing, i, at_end, q);
    let Some(p) = crossing.paths.get_mut(i) else { return };
    if at_end {
        p.fit_start_tangent(hold);
        // (a point behind the path would be a loop of kilometres: a straight piece to the
        // cursor instead, still at the same height)
        let chord = (p.end().truncate() - p.start().truncate()).length();
        if !p.length().is_finite() || p.length() > chord * 4.0 + 40.0 {
            p.v[4] = 0.0;
            p.set_end(DVec3::new(q.x, q.y, z));
        }
        let l = p.length();
        p.v[6] = slope as f32;
        if l > 1e-3 {
            p.v[7] = (2.0 * (z - p.start().z) / l - slope) as f32;
        }
    } else {
        p.fit_end_tangent(hold);
        let chord = (p.end().truncate() - p.start().truncate()).length();
        if !p.length().is_finite() || p.length() > chord * 4.0 + 40.0 {
            p.v[4] = 0.0;
            p.set_start(DVec3::new(q.x, q.y, z));
        }
        p.set_start_height_and_slope(z, slope);
    }
}

fn kind_color(kind: i32) -> Color32 {
    match kind {
        1 => FOOTWAY,
        2 => RAIL,
        _ => STREET,
    }
}

/// Which ways traffic may go on a path: along it (start to end) and/or against it. 0 is along,
/// 1 against, 2 both ways.
pub(crate) fn path_ways(direction: i32) -> (bool, bool) {
    match direction {
        1 => (false, true),
        2 => (true, true),
        _ => (true, false),
    }
}

fn kind_name(kind: i32) -> &'static str {
    match kind {
        1 => tr("Footway", "Тротуар"),
        2 => tr("Rail", "Рельсы"),
        _ => tr("Street", "Проезжая часть"),
    }
}

fn direction_name(d: i32) -> &'static str {
    match d {
        1 => tr("Against", "Против"),
        2 => tr("Both ways", "В обе стороны"),
        _ => tr("Along", "По направлению"),
    }
}

fn turn_name(t: i32) -> &'static str {
    match t {
        TURN_LEFT => tr("Left", "Налево"),
        TURN_RIGHT => tr("Right", "Направо"),
        _ => tr("None", "Нет"),
    }
}

/// What a `[phase]`'s state shows (the game's own groups).
fn aspect_name(state: i32) -> &'static str {
    match state {
        0..=2 => tr("Red", "Красный"),
        3..=5 => tr("Red and yellow", "Красный и жёлтый"),
        6 | 7 => tr("Green", "Зелёный"),
        8 => tr("Green and yellow", "Зелёный и жёлтый"),
        9..=11 => tr("Yellow", "Жёлтый"),
        _ => tr("Off", "Выключен"),
    }
}

/// The state's colour, as the crossing editor's timeline shows it.
fn aspect_color(state: i32) -> Color32 {
    match state {
        0..=2 => Color32::from_rgb(198, 56, 56),
        3..=5 => Color32::from_rgb(210, 146, 52),
        6 | 7 => Color32::from_rgb(62, 150, 74),
        8 => Color32::from_rgb(142, 168, 58),
        9..=11 => Color32::from_rgb(206, 192, 62),
        _ => Color32::from_rgb(70, 72, 70),
    }
}

/// The state as the game's own table writes it (`R` red, `Ro` red and yellow, `G` green,
/// `G+Y` green and yellow, `Y` yellow, `A` off, `B` blinking and `+A` with an arrow).
fn aspect_symbol(state: i32) -> &'static str {
    match state {
        0 => "R",
        1 => "R+B",
        2 => "R+P",
        3 => "Ro",
        4 => "Ro+B",
        5 => "Ro+P",
        6 => "G",
        7 => "G+B",
        8 => "G+Y",
        9 => "Y",
        10 => "Y+B",
        11 => "Y+P",
        _ => "A",
    }
}

/// Which page of the window is shown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Paths,
    Lights,
}

/// How Save writes the crossing: into the object's own `.sco` (keeping the old one as a
/// `.bak` beside it), or into a copy of the object the editor makes of its own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveMode {
    InPlace,
    Copy,
}

/// The timeline's room: what the row names take on the left, and the second marks on top.
const GUTTER: f32 = 118.0;
const HEAD: f32 = 18.0;
const ROW_H: f32 = 19.0;

fn unsaved_text() -> &'static str {
    tr("There are unsaved changes here: Save or Revert first", "Здесь есть несохранённые правки: сначала сохраните или отмените их")
}

impl CrossingPaths {
    fn unsaved(&self) -> bool {
        self.sheet.as_ref().is_some_and(|s| s.crossing != self.seen)
    }

    /// Whether the paths are shown - and worked on - on the map itself: the crossing object in
    /// 3D, with the paths laid over it.
    pub fn on_map(&self) -> bool {
        self.open && self.view_3d && self.tab == Tab::Paths && self.sheet.is_some()
    }

    /// The object the crossing belongs to.
    pub fn object(&self) -> Option<Sel> {
        self.target
    }

    /// The path picked here (the map draws it picked too, when the paths are shown there).
    pub fn picked(&self) -> Option<usize> {
        self.selected
    }

    /// A click on the map picks a path: false when nothing of the crossing is at that point.
    pub fn select_on_map(&mut self, hit: Option<usize>) -> bool {
        let Some(i) = hit else { return false };
        if !self.on_map() || self.sheet.as_ref().is_none_or(|s| i >= s.crossing.paths.len()) {
            return false;
        }
        self.selected = Some(i);
        self.reveal = true;
        true
    }

    /// The paths' sampled points in the object's own frame, a path at a time, for taking one
    /// with the mouse on the map.
    pub fn map_paths(&self) -> Vec<(usize, Vec<DVec3>)> {
        let Some(sheet) = self.sheet.as_ref() else { return Vec::new() };
        sheet.crossing.paths.iter().enumerate().map(|(i, p)| (i, p.points(0.5))).collect()
    }

    /// Path `i`'s end (`at_end`) as a joint: where it is, and the way this path runs away from
    /// it, so another path put there meets it smoothly rather than only at the point.
    pub fn joint_at(&self, i: usize, at_end: bool) -> Option<Joint> {
        let sheet = self.sheet.as_ref()?;
        let p = sheet.crossing.paths.get(i)?;
        let c = p.curve();
        let (pos, heading, slope) = if at_end {
            (p.end(), crate::roads::norm_deg(c.heading_at(c.length.max(0.0)) + 180.0), p.slope_end())
        } else {
            (p.start(), crate::roads::norm_deg(c.heading_at(0.0)), p.slope_start())
        };
        Some(Joint { pos, heading, slope, flow: outward_flow(p.direction(), at_end) })
    }

    // ---------------------------------------------------------------- undo

    /// Record the crossing as it was, before a change: what Ctrl+Z goes back to. Taking the
    /// record twice with nothing changed between adds nothing.
    pub fn push_undo(&mut self) {
        let Some(sheet) = self.sheet.as_ref() else { return };
        let before = sheet.crossing.clone();
        if self.undo.last() != Some(&before) {
            self.undo.push(before.clone());
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.mark = before;
    }

    /// A record of the crossing as it is now, when it has changed and no drag is under way:
    /// a burst of changes to one number is one step, not thirty.
    fn checkpoint(&mut self, now: f64) {
        if self.drag.is_some() || self.map_drag.is_some() || self.draw_from.is_some() || self.edge.is_some() {
            return;
        }
        let Some(sheet) = self.sheet.as_ref() else { return };
        let after = sheet.crossing.clone();
        if after == self.mark {
            return;
        }
        if now - self.last_push < 0.7 {
            // (the same burst: the state before it is already recorded, only the mark moves)
            self.mark = after;
            return;
        }
        let before = std::mem::replace(&mut self.mark, after);
        if self.undo.last() != Some(&before) {
            self.undo.push(before);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_push = now;
    }

    /// Ctrl+Z: the crossing goes back a step. False when there is nothing to go back to (the
    /// map's own undo takes the key then).
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        let Some(sheet) = self.sheet.as_ref() else { return false };
        self.redo.push(sheet.crossing.clone());
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.crossing = prev.clone();
        }
        self.mark = prev;
        self.last_push = 0.0;
        self.fix_picks();
        true
    }

    /// Ctrl+Y: a step taken back is taken again.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        if let Some(sheet) = self.sheet.as_mut() {
            self.undo.push(sheet.crossing.clone());
            sheet.crossing = next.clone();
        }
        self.mark = next;
        self.last_push = 0.0;
        self.fix_picks();
        true
    }

    /// `N`: a path carried on from the picked one, as the "+" button adds it. False when
    /// there is no crossing open to add it to.
    pub fn continue_path(&mut self) -> bool {
        let count = self.sheet.as_ref().map(|s| s.crossing.paths.len()).unwrap_or(0);
        if count == 0 && self.selected.is_none() {
            return false;
        }
        self.push_undo();
        let at = {
            let Some(sheet) = self.sheet.as_mut() else { return false };
            continue_path(&mut sheet.crossing, self.selected)
        };
        // (the new end sits on the deck; both slopes become that one grade)
        let z = at.and_then(|i| {
            let end = self.sheet.as_ref()?.crossing.paths.get(i)?.end().truncate();
            surface_z(&self.surface, end)
        });
        if let (Some(i), Some(z)) = (at, z) {
            if let Some(p) = self.sheet.as_mut().and_then(|s| s.crossing.paths.get_mut(i)) {
                set_end_height(p, z);
            }
        }
        self.selected = at;
        self.reveal = true;
        true
    }

    /// Whether the next click on a nearby path's end is to start a new path there, or to carry
    /// the picked one on to it.
    pub fn wants_nearby(&self) -> bool {
        self.open && (self.from_nearby || self.complete_to) && self.sheet.is_some()
    }

    /// Whether the picked path is the one being carried on to a nearby end.
    pub fn completing(&self) -> bool {
        self.complete_to && self.open && self.sheet.is_some()
    }

    /// The click on a nearby end, doing whichever of the two modes is on.
    pub fn take_nearby(&mut self, j: Joint) -> Option<usize> {
        if self.complete_to {
            self.complete_to_joint(j)
        } else {
            self.start_from_joint(j)
        }
    }

    /// `Complete to`: carry the picked path on to a nearby path's end. The picked path is not
    /// touched - its end, its way there and its rise stay as they are - and the pieces that
    /// join it to the other end are added after it.
    ///
    /// One piece only when one arc can leave in the picked path's way and arrive in the
    /// other's; otherwise the join is several forward pieces, rather than turning the picked
    /// path's way out of its end to make a single piece fit.
    pub fn complete_to_joint(&mut self, j: Joint) -> Option<usize> {
        if !self.completing() {
            return None;
        }
        let Some(i) = self.selected else { return None };
        let surface = std::sync::Arc::clone(&self.surface);
        let Some(sheet) = self.sheet.as_ref() else { return None };
        let p = sheet.crossing.paths.get(i)?.clone();
        let c = p.curve();
        let a = p.end();
        // (the way the picked path runs on past its end, and the rise it has there)
        let h_a = crate::roads::norm_deg(c.heading_at(c.length.max(0.0)));
        let s_a = p.slope_end();
        let b = j.pos;
        let h_b = j.heading;
        let s_b = j.slope;
        let delta = b.truncate() - a.truncate();
        let chord = delta.length();
        if chord < 0.25 || chord > FAR_JOIN {
            return None;
        }
        let phi = crate::roads::heading_of(delta);
        let same = |x: f64, y: f64| ((x - y).rem_euclid(360.0)).min((y - x).rem_euclid(360.0)) < 1.0;
        let mut template = p.clone();
        template.blocks.clear();
        template.crossing_problem = false;
        // (several pieces as soon as one arc cannot meet both ways: the picked path's way out
        // of its end is not turned to make a single piece fit)
        let segs = if same(h_a, phi) && same(h_a, h_b) {
            vec![crate::roads::Seg { start: a.truncate(), heading: h_a, length: chord, radius: 0.0 }]
        } else {
            connect_segs(a.truncate(), h_a, b.truncate(), h_b)
        };
        let made = lay_join(&template, a, s_a, &segs, PieceEnd::At { z: b.z, slope: s_b }, false, &surface);
        if made.is_empty() {
            return None;
        }
        self.push_undo();
        let Some(sheet) = self.sheet.as_mut() else { return None };
        let mut last = None;
        for q in made {
            sheet.crossing.paths.push(q);
            last = Some(sheet.crossing.paths.len() - 1);
        }
        self.selected = last;
        self.reveal = true;
        self.complete_to = false;
        self.from_nearby = false;
        self.nearby.clear();
        last
    }

    /// The nearby ends a new path can be started from, in the object's own frame.
    pub fn neighbours(&self) -> &[Joint] {
        &self.nearby
    }

    /// Turn the picking of a nearby end on and off (`Start from a nearby path`).
    pub fn set_from_nearby(&mut self, on: bool) {
        self.from_nearby = on;
        if on {
            self.complete_to = false;
        }
        if !on {
            self.nearby.clear();
        } else {
            // (the drawing is brought to take them in: they lie further out than the crossing)
            self.fit = true;
        }
    }

    /// Turn `Complete to a nearby path` on and off.
    pub fn set_complete_to(&mut self, on: bool) {
        self.complete_to = on;
        if on {
            self.from_nearby = false;
            self.fit = true;
        }
        if !on {
            self.nearby.clear();
        }
    }

    /// The nearby ends a new path can be started from, gathered from the lanes round the
    /// crossing (other objects', and the roads' own).
    /// The ends of the paths near the crossing - other objects', and the roads' own lanes -
    /// with the way each runs away from it, all in the object's own frame (which is what the
    /// drawing and a new path are written in).
    fn gather_nearby(&self, ed: &mut Editor, assets: Option<&mut Assets>) -> Vec<Joint> {
        let (Some(Sel::Object(k, id)), Some(doc), Some(assets)) = (self.target, ed.doc.as_ref(), assets) else { return Vec::new() };
        let Some(o) = doc.object(k, id) else { return Vec::new() };
        let (file, rot) = (o.file.clone(), o.rot);
        let abs = assets.object(&file).map(|m| m.abs_height).unwrap_or(true);
        let world = doc.object_world(k, o, abs);
        let m = crate::render::object_matrix(DVec3::ZERO, rot);
        let inv = m.inverse();
        // (the same reach the 3D view draws: a shorter one left green circles on the map
        // that no click could take, because they were never in this list)
        let reach = ed.neighbour_reach();
        let (lanes, _) = crate::lanes::near(doc, assets, world.truncate(), reach);
        let mut out = Vec::new();
        // (the crossing's own ends as well, but not the picked path's: a path is carried on or
        // started from another, never from itself)
        if let Some(sheet) = self.sheet.as_ref() {
            for (i, q) in sheet.crossing.paths.iter().enumerate() {
                if Some(i) == self.selected {
                    continue;
                }
                if let Some(j) = self.joint_at(i, true) {
                    out.push(j);
                }
                if let Some(j) = self.joint_at(i, false) {
                    out.push(j);
                }
            }
        }
        for l in lanes.iter().filter(|l| l.object != Some((k, id))) {
            let n = l.pts.len();
            if n < 2 {
                continue;
            }
            // (both ends, and at each the way is *away* from the joint along the lane: at the
            // start that is against the lane's own travel, at the end it is with it)
            for (pos, (a, b), at_end) in [(l.pts[0], (l.pts[1], l.pts[0]), false), (l.pts[n - 1], (l.pts[n - 2], l.pts[n - 1]), true)] {
                // (an end a kilometre down a long spline is not a circle on this crossing)
                if (pos.truncate() - world.truncate()).length() > reach {
                    continue;
                }
                let d = b - a;
                let len = d.truncate().length().max(1e-6);
                let local = inv.transform_point3((pos - world).as_vec3()).as_dvec3();
                // (the object's turn is taken out of the lane's heading, the path being
                // written in the object's frame)
                let heading = crate::roads::norm_deg(crate::roads::heading_of(d.truncate()) - rot[0]);
                out.push(Joint { pos: local, heading, slope: d.z / len, flow: crate::crossing_edit::outward_flow(l.direction, at_end) });
            }
        }
        log::debug!("crossing nearby: {} ends within {reach} m", out.len());
        out
    }

    /// Start a new path from a nearby path's end: it leaves in that path's way, at its rise,
    /// ten metres long (its numbers are there to be changed after). Its traffic direction is
    /// the join's (`Joint::flow`), so it follows the way the other path is driven rather than
    /// always being one way.
    pub fn start_from_joint(&mut self, j: Joint) -> Option<usize> {
        if !self.wants_nearby() {
            return None;
        }
        // (a joint in another frame - the map's rather than the object's - is not one to start
        // from: no crossing is ten kilometres across)
        if !j.pos.is_finite() || j.pos.truncate().length() > FAR_JOIN {
            log::warn!("start from a joint {:.0} m from the object: left alone", j.pos.truncate().length());
            return None;
        }
        self.push_undo();
        let mut p = Path::default();
        p.v[0] = j.pos.x as f32;
        p.v[1] = j.pos.y as f32;
        p.v[2] = j.pos.z as f32;
        p.v[3] = j.heading as f32;
        p.v[4] = 0.0;
        p.v[5] = 10.0;
        p.v[6] = j.slope as f32;
        p.v[7] = j.slope as f32;
        p.v[10] = j.flow as f32;
        let at = {
            let Some(sheet) = self.sheet.as_mut() else { return None };
            sheet.crossing.paths.push(p);
            sheet.crossing.paths.len() - 1
        };
        let z = self.sheet.as_ref().and_then(|s| s.crossing.paths.get(at)).and_then(|p| surface_z(&self.surface, p.end().truncate()));
        if let Some(z) = z {
            if let Some(p) = self.sheet.as_mut().and_then(|s| s.crossing.paths.get_mut(at)) {
                set_end_height(p, z);
            }
        }
        self.selected = Some(at);
        self.reveal = true;
        self.from_nearby = false;
        self.nearby.clear();
        Some(at)
    }

    /// The picks that a step back may have taken the ground from (a path or light number
    /// past the end).
    fn fix_picks(&mut self) {
        let Some(sheet) = self.sheet.as_ref() else { return };
        let (paths, lights) = (sheet.crossing.paths.len(), sheet.crossing.lights.len());
        if self.selected.is_some_and(|i| i >= paths) {
            self.selected = paths.checked_sub(1);
        }
        if self.light.is_some_and(|i| i >= lights) {
            self.light = lights.checked_sub(1);
            self.phase = None;
        }
        if let (Some(l), Some(p)) = (self.light, self.phase) {
            if sheet.crossing.lights.get(l).is_none_or(|x| p >= x.phases.len()) {
                self.phase = None;
            }
        }
        self.drag = None;
        self.map_drag = None;
        self.map_drag_plane = None;
        self.drag_anchor = None;
        self.draw_from = None;
        self.edge = None;
    }

    /// How far the paths reach, in metres: the width of the view that frames them all.
    pub fn span(&self) -> f64 {
        let ends = self.map_ends();
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for (_, _, p) in &ends {
            lo = lo.min(p.truncate());
            hi = hi.max(p.truncate());
        }
        if let Some((_, _, b)) = &self.footprint {
            if b[2] > b[0] && b[3] > b[1] {
                lo = lo.min(DVec2::new(b[0], b[1]));
                hi = hi.max(DVec2::new(b[2], b[3]));
            }
        }
        if lo.x > hi.x {
            return 30.0;
        }
        (hi - lo).length().clamp(6.0, 300.0)
    }

    /// The ends of the paths, three numbers each in the object's own frame (x, y, z and
    /// whether it is the far end), for taking one with the mouse on the map.
    pub fn map_ends(&self) -> Vec<(usize, bool, DVec3)> {
        let Some(sheet) = self.sheet.as_ref() else { return Vec::new() };
        sheet
            .crossing
            .paths
            .iter()
            .enumerate()
            .flat_map(|(i, p)| [(i, true, p.end()), (i, false, p.start())])
            .collect()
    }

    /// Take the end of a path the map's click found, to drag it there: false when there is
    /// nothing of that path to take.
    pub fn take_map_end(&mut self, hit: Option<(usize, bool)>) -> bool {
        let Some((i, at_end)) = hit else { return false };
        if !self.on_map() || self.sheet.as_ref().is_none_or(|s| i >= s.crossing.paths.len()) {
            return false;
        }
        self.selected = Some(i);
        self.map_drag = Some((i, at_end));
        // (a new grab is judged on this end's own height, not the one a previous drag locked)
        self.map_drag_plane = None;
        self.drag_anchor = at_end.then(|| {
            let p = self.sheet.as_ref().and_then(|s| s.crossing.paths.get(i))?;
            Some(ReachAnchor { root: i, start: p.start().truncate(), heading: p.heading(), z: p.start().z, slope: p.slope_start(), spawned: 0 })
        }).flatten();
        true
    }

    /// The world height a map drag keeps the cursor's ray on. The first call locks it.
    pub fn map_drag_plane(&mut self, z: f64) -> f64 {
        *self.map_drag_plane.get_or_insert(z)
    }

    /// The end the map is dragging, and that it is let go.
    pub fn map_dragging(&self) -> Option<(usize, bool)> {
        self.map_drag
    }

    pub fn let_go(&mut self) {
        self.map_drag = None;
        self.map_drag_plane = None;
        self.drag_anchor = None;
    }

    /// Drag the end the map took to this point of the object's frame, as the drawing in the
    /// window does. `snap` is how near another end must be to be taken (None: no snapping,
    /// which is what holding Ctrl gives).
    pub fn drag_map_end(&mut self, at: DVec3, joint: Option<Joint>) -> bool {
        let Some((i, at_end)) = self.map_drag else { return false };
        if self.sheet.as_ref().is_none_or(|s| i >= s.crossing.paths.len()) {
            self.map_drag = None;
            self.map_drag_plane = None;
            self.drag_anchor = None;
            return false;
        }
        if at_end {
            // (the end is rebuilt as pieces from where the path set out: the way it leaves,
            // and the point it starts at, are not turned to make one piece reach the joint)
            if self.drag_anchor.is_none() {
                let p = self.sheet.as_ref().and_then(|s| s.crossing.paths.get(i)).map(|p| (p.start(), p.heading(), p.slope_start()));
                if let Some((start, heading, slope)) = p {
                    self.drag_anchor = Some(ReachAnchor { root: i, start: start.truncate(), heading, z: start.z, slope, spawned: 0 });
                }
            }
            let aim = joint.map(|j| j.pos.truncate()).unwrap_or(at.truncate());
            return self.reach_aim(aim, joint);
        }
        let Some(sheet) = self.sheet.as_mut() else { return false };
        match joint {
            Some(j) => join_start(&mut sheet.crossing, i, j),
            None => move_end(&mut sheet.crossing, i, at_end, at.truncate()),
        }
        true
    }

    /// Rebuild the dragged end as forward pieces (or pieces that arrive in `joint`'s way),
    /// on the surface.
    fn reach_aim(&mut self, aim: DVec2, joint: Option<Joint>) -> bool {
        let Some(mut anchor) = self.drag_anchor.clone() else { return false };
        let tris = std::sync::Arc::clone(&self.surface);
        let Some(last) = self.sheet.as_mut().and_then(|s| apply_reach(&mut s.crossing, &mut anchor, aim, &tris, joint)) else {
            return false;
        };
        self.drag_anchor = Some(anchor);
        self.selected = Some(last);
        self.map_drag = Some((last, true));
        true
    }

    /// Open on the selected object, or on a crossing picked from the map's list.
    pub fn request(&mut self, ed: &mut Editor, roots: &Roots, lib: &Library) {
        // (what is being edited is not dropped for another object)
        if self.unsaved() && !matches!((ed.selected, self.target), (Some(a), Some(b)) if a == b) {
            self.open = true;
            self.message = Some((unsaved_text().into(), true));
            return;
        }
        self.sheet = None;
        if let Some(Sel::Object(k, id)) = ed.selected {
            if self.open_on(ed, roots, k, id) {
                self.open = true;
                return;
            }
        }
        self.candidates.clear();
        if let Some(doc) = ed.doc.as_ref() {
            for (k, t) in &doc.tiles {
                for o in &t.tile.objects {
                    if self.candidates.len() >= 300 {
                        break;
                    }
                    // (only an object with `[path]`s is a crossing: the catalogue puts every
                    // signal head and every light program in Junctions, and none of those has
                    // a path of its own - the list was mostly red lights)
                    if is_crossing(lib, &o.file) {
                        self.candidates.push((*k, o.id, o.file.clone()));
                    }
                }
            }
        }
        self.open = true;
        log::info!("crossing editor: {} crossings on the map to pick from", self.candidates.len());
        if self.message.is_none() {
            self.message = Some((tr("Select a crossing object, or pick one of these", "Выберите объект перекрёстка или возьмите один из этих").into(), false));
        }
    }

    /// Which page the window shows when it opens (checks of the light program's page).
    pub fn open_page(&mut self, lights: bool) {
        self.tab = if lights { Tab::Lights } else { Tab::Paths };
    }

    /// Show the paths on the map itself, framed on the crossing (a check run of the 3D view).
    pub fn show_on_map(&mut self) {
        self.tab = Tab::Paths;
        self.view_3d = true;
        self.frame_on_map = true;
    }

    /// Whether a crossing is open (a check run says so before it asks for the map's list).
    pub fn has_sheet(&self) -> bool {
        self.sheet.is_some()
    }

    /// Close the window and drop what is in it: the file stays as it was (nothing was written
    /// until Save). The next opening reads it again.
    fn discard(&mut self) {
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.crossing = self.seen.clone();
        }
        self.ask_close = false;
        self.open = false;
        self.sheet = None;
        self.message = None;
        self.drag = None;
        self.map_drag = None;
        self.map_drag_plane = None;
        self.draw_from = None;
        self.edge = None;
        self.selected = None;
        self.footprint = None;
    }

    /// Open the first crossing of the map's own list (a check run, with no one to pick one).
    pub fn open_first_candidate(&mut self, ed: &mut Editor, roots: &Roots) -> bool {
        let Some((k, id)) = self.candidates.first().map(|(k, id, _)| (*k, *id)) else { return false };
        self.open_on(ed, roots, k, id)
    }

    /// Read the object's `.sco` for editing. False (and a message) when it has no paths.
    fn open_on(&mut self, ed: &Editor, roots: &Roots, k: (i32, i32), id: i64) -> bool {
        let Some(doc) = ed.doc.as_ref() else { return false };
        let Some(o) = doc.object(k, id) else { return false };
        let rel = o.file.clone();
        if let Some(why) = crossing_edit::why_not(roots, &rel) {
            log::info!("crossing editor: {rel} cannot be edited: {why}");
            self.message = Some((format!("{}: {}", &rel, why), true));
            self.sheet = None;
            return false;
        }
        match crossing_edit::open(roots, &rel) {
            Ok((abs, bytes, sheet)) => {
                self.copied = sidecar(&abs).is_file();
                self.target = Some(Sel::Object(k, id));
                self.map_dir = doc.dir.clone();
                self.from_rel = rel.clone();
                self.rel = rel;
                self.abs = abs;
                self.bytes = bytes;
                self.seen = sheet.crossing.clone();
                self.selected = (!sheet.crossing.paths.is_empty()).then_some(0);
                self.mark = sheet.crossing.clone();
                self.undo.clear();
                self.redo.clear();
                self.last_push = 0.0;
                self.save_mode = SaveMode::InPlace;
                self.sheet = Some(sheet);
                self.fit = true;
                self.drag = None;
                self.map_drag = None;
        self.map_drag_plane = None;
                self.draw_from = None;
                self.footprint = None;
                self.message = None;
                true
            }
            Err(e) => {
                log::info!("crossing editor: {rel} cannot be read: {e}");
                self.message = Some((e.to_string(), true));
                self.sheet = None;
                false
            }
        }
    }

    /// The window.
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, roots: Option<&Roots>, assets: Option<&mut Assets>, actions: &mut Vec<crate::ui::Action>) {
        let _ = lib;
        if !self.open {
            if ed.crossing_draft.is_some() {
                ed.crossing_draft = None;
            }
            if ed.crossing_selected.is_some() {
                ed.crossing_selected = None;
            }
            return;
        }
        // (a change to the crossing is a step of its own undo: taken here, before the window
        // draws, so what a drag turned into is one record)
        self.checkpoint(ctx.input(|i| i.time));
        let mut open = self.open;
        if self.sheet.is_some() {
            // the map shows the paths as they are here, not as the file still has them
            if let (Some(Sel::Object(k, id)), Some(sheet)) = (self.target, self.sheet.as_ref()) {
                ed.crossing_draft = Some((k, id, sheet.crossing.paths.clone()));
            }
            ed.crossing_selected = self.picked();
        } else {
            ed.crossing_draft = None;
            ed.crossing_selected = None;
        }
        let screen = ctx.content_rect();
        // (the paths laid on the map itself: the window keeps the properties at the screen's
        // right edge, so the crossing - the object in 3D - is what is seen and worked on)
        let beside = self.view_3d && self.tab == Tab::Paths;
        let size = if beside { Vec2::new(400.0, (screen.height() - 90.0).max(420.0)) } else { Vec2::new(1000.0, 660.0) };
        // (in the middle of the screen, as the crossing editor's own window is)
        let pos = if beside { Pos2::new(screen.right() - size.x - 10.0, screen.top() + 34.0) } else { screen.center() - size * 0.5 };
        let mut save = false;
        let mut revert = false;
        let mut assets = assets;
        // (a new path started from a nearby path: its ends are gathered here, so the drawing
        // and the map both know what can be clicked)
        if self.wants_nearby() && self.sheet.is_some() {
            self.nearby = self.gather_nearby(ed, assets.as_deref_mut());
        } else if !self.nearby.is_empty() {
            self.nearby.clear();
        }
        // (each page named as the crossing editor's own window names it)
        let title = match self.tab {
            Tab::Paths => icons::labelled(Icon::Route, tr("Crossing editor", "Редактор перекрёстка")),
            Tab::Lights => icons::labelled(Icon::Traffic, tr("Traffic lights", "Светофоры")),
        };
        let mut window = egui::Window::new(title)
            .id(egui::Id::new("crossing paths window"))
            .open(&mut open)
            .default_pos(pos)
            .default_size(size)
            .resizable(!beside);
        window = if beside { window.current_pos(pos).fixed_size(size) } else { window.min_size(Vec2::new(880.0, 540.0)) };
        let shown = window
            .show(ctx, |ui| {
                // (a scroll area takes the layout of the ui it is made in, so each half gets
                // its own top-down layout: put straight into the row below, the left side
                // would lay its labels out sideways and push the view off the window)
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 3.0);
                let body = if beside {
                    Vec2::new(ui.available_width().max(320.0), ui.available_height().max(400.0))
                } else {
                    Vec2::new(ui.available_width().max(800.0), ui.available_height().max(440.0))
                };
                ui.set_min_size(body);
                let Some(mut sheet) = self.sheet.take() else {
                    self.no_target(ui, ed, roots, lib);
                    return;
                };
                self.head(ui, &sheet, &mut save, &mut revert, roots);                ui.separator();
                let body = if beside {
                    Vec2::new(ui.available_width().max(320.0), ui.available_height().max(400.0))
                } else {
                    Vec2::new(ui.available_width().max(800.0), ui.available_height().max(400.0))
                };
                match self.tab {
                    Tab::Paths if beside => {
                        // (the panel's whole height scrolls: the numbers of the picked path -
                        // its radius and length among them - are always within reach)
                        egui::ScrollArea::vertical().id_salt("crossing 3d side").auto_shrink([false, false]).show(ui, |ui| {
                            ui.set_width(320.0);
                            ui.horizontal(|ui| {
                                if ui.button(tr("Back to 2D", "К 2D")).on_hover_text(tr("Draw the crossing from above, in the window, and lay paths there", "Рисовать перекрёсток сверху, в окне, и проводить пути там")).clicked() {
                                    self.view_3d = false;
                                }
                                if ui.button(tr("Frame", "Вписать")).on_hover_text(tr("Put the crossing in the middle of the map's view", "Поставить перекрёсток в середину вида карты")).clicked() {
                                    self.frame_on_map = true;
                                }
                            });
                            ui.label(RichText::new(tr("Click a path on the map to pick it. Drag an end there to move it: it snaps to another path's end - the crossing's own or a nearby object's - and Ctrl keeps it free. A snapped end takes that path's way and rise, so the two meet smoothly", "Щёлкните путь на карте, чтобы выбрать его. Тяните конец — он притягивается к концу другого пути (своего или соседнего объекта), с Ctrl — свободно. Притянутый конец принимает направление и уклон того пути, так что они сходятся плавно")).small().color(DIM));
                            ui.label(RichText::new(tr("Nearby paths are drawn paler: another object's, and the roads' own lanes. An end can be joined to theirs, but they are not changed from here", "Ближние пути нарисованы бледнее: чужих объектов и полосы самих дорог. К их концу можно присоединиться, но отсюда они не меняются")).small().color(DIM));
                            ui.separator();
                            self.paths_side(ui, ed, &mut sheet);
                        });
                    }
                    Tab::Paths => self.paths_body(ui, ed, &mut sheet, body, assets.as_deref_mut()),
                    Tab::Lights => self.lights_body(ui, ed, &mut sheet, body),
                }
                self.sheet = Some(sheet);
            });
        #[cfg(test)]
        if let Some(s) = &shown {
            self.last_window = Some(s.response.rect);
        }
        #[cfg(not(test))]
        let _ = &shown;
        // (the crossing put in the middle of the map's view: the object itself, in 3D)
        if self.frame_on_map {
            self.frame_on_map = false;
            actions.push(crate::ui::Action::CrossingFrame);
        }
        // (closed with edits in it: they are not dropped without a word - the question is
        // put below, and the window stays open until it is answered)
        if !open && self.unsaved() {
            open = true;
            self.ask_close = true;
        }
        self.open = open;
        if !self.open {
            self.discard();
            ed.crossing_draft = None;
            ed.crossing_selected = None;
        } else if self.ask_close {
            // (asked: Save and close / Close without saving / Cancel)
            let mut answer: Option<bool> = None;
            let screen = ctx.content_rect();
            egui::Window::new(tr("Close the crossing editor", "Закрыть редактор перекрёстка"))
                .id(egui::Id::new("crossing close question"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_max_width((screen.width() * 0.5).clamp(280.0, 460.0));
                    ui.label(RichText::new(tr("There are changes here that are not saved yet. Save them before closing?", "Здесь есть несохранённые правки. Сохранить их перед закрытием?")).strong());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(icons::labelled(Icon::Save, tr("Save and close", "Сохранить и закрыть"))).clicked() {
                            answer = Some(true);
                        }
                        if ui.button(tr("Close without saving", "Закрыть без сохранения")).clicked() {
                            answer = Some(false);
                        }
                        if ui.button(tr("Cancel", "Отмена")).clicked() {
                            self.ask_close = false;
                        }
                    });
                });
            match answer {
                Some(true) => {
                    // (saving may fail - a read-only file, no installation chosen: then the
                    // window stays open, with the reason on it)
                    if let (Some(roots), Some(assets)) = (roots, assets.as_deref_mut()) {
                        self.save(ed, roots, assets);
                    }
                    self.ask_close = false;
                    if !self.unsaved() {
                        self.open = false;
                        self.discard();
                        ed.crossing_draft = None;
                        ed.crossing_selected = None;
                    }
                }
                // (the file is as it was: the changes are dropped with the window)
                Some(false) => {
                    self.discard();
                    ed.crossing_draft = None;
                    ed.crossing_selected = None;
                }
                None => {}
            }
        }
        if revert {
            if let Some(sheet) = self.sheet.as_mut() {
                sheet.crossing = self.seen.clone();
            }
            self.drag = None;
            self.edge = None;
            self.message = Some((tr("Back to how it was opened", "Обратно к состоянию при открытии").into(), false));
        }
        if save {
            if let (Some(roots), Some(assets)) = (roots, assets.as_deref_mut()) {
                self.save(ed, roots, assets);
            }
        }
    }

    /// Above both pages: what is open, the two pages, and saving - as the crossing editor's
    /// own title bar carries them.
    fn head(&mut self, ui: &mut egui::Ui, sheet: &Sheet, save: &mut bool, revert: &mut bool, roots: Option<&Roots>) {
        let name = std::path::Path::new(&self.rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| self.rel.clone());
        let copied = self.copied;
        // (beside the map the head is laid in rows: a single row of it would widen the panel
        // over the crossing it is there to leave seen)
        let side = self.view_3d && self.tab == Tab::Paths;
        let paths = format!("{} ({})", tr("Paths", "Пути"), sheet.crossing.paths.len());
        let lights = format!("{} ({})", tr("Traffic lights", "Светофоры"), sheet.crossing.lights.len());
        let in_place = self.save_mode == SaveMode::InPlace;
        let save_label = if in_place {
            tr("Save", "Сохранить")
        } else if copied {
            tr("Save", "Сохранить")
        } else {
            tr("Save as a copy", "Сохранить копию")
        };
        let save_hint = if in_place {
            tr("Write the paths into the object's own file; the file as it stood is kept as a .bak beside it", "Записать пути в собственный файл объекта; прежний файл сохраняется рядом как .bak")
        } else if copied {
            tr("Write the paths into openMapper's copy of the object", "Записать пути в копию объекта openMapper")
        } else {
            tr("The object's folder is copied into openMapper's, the paths written there, and the map's object pointed at it: the original file is not touched", "Папка объекта копируется в openMapper, пути пишутся туда, объект карты начинает ссылаться на неё: исходный файл не трогается")
        };
        let revert_hint = tr("Back to how the object was when it was opened (the file is not touched)", "Обратно к тому, каким объект был при открытии (файл не трогается)");
        // (how it saves, and the backups it has made: the crossing editor's own "Save as")
        let menu = |ui: &mut egui::Ui, me: &mut CrossingPaths| {
            ui.menu_button("…", |ui| {
                let in_place = me.save_mode == SaveMode::InPlace;
                if ui.selectable_label(in_place, tr("In the object's own file (a .bak keeps the old one)", "В собственном файле объекта (старый — в .bak)")).on_hover_text(tr("The map's object already names this file: the paths are written into it, and the file as it stood is kept beside it as its .bak", "Объект карты уже ссылается на этот файл: пути пишутся в него, прежний сохраняется рядом как .bak")).clicked() {
                    me.save_mode = SaveMode::InPlace;
                    ui.close();
                }
                if ui.selectable_label(!in_place, tr("As a new file", "Новым файлом")).on_hover_text(tr("The whole object is copied into openMapper's content folder under a name of its own, and the map's object points at the copy", "Объект целиком копируется в папку содержимого openMapper под собственным именем, объект карты ссылается на копию")).clicked() {
                    me.save_mode = SaveMode::Copy;
                    ui.close();
                }
                ui.separator();
                let own = crossing_edit::bak_of(&me.abs);
                if ui.add_enabled(own.is_some(), egui::Button::new(tr("Remove this file's .bak", "Удалить .bak этого файла"))).clicked() {
                    if crossing_edit::remove_bak(&me.abs) {
                        me.message = Some((tr("Its .bak is gone", "Его .bak удалён").into(), false));
                    }
                    ui.close();
                }
                let count = roots.map(crossing_edit::baks::list).map(|l| l.len()).unwrap_or(0);
                if ui.add_enabled(count > 0, egui::Button::new(format!("{} ({count})", tr("Remove the .bak files this editor made", "Удалить .bak, сделанные этим редактором")))).clicked() {
                    if let Some(r) = roots {
                        let n = crossing_edit::baks::clear(r);
                        me.message = Some((format!("{n} .bak"), false));
                    }
                    ui.close();
                }
            });
        };
        if side {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    menu(ui, self);
                });
            });
            ui.horizontal(|ui| {
                if ui.selectable_label(self.tab == Tab::Paths, paths).clicked() {
                    self.tab = Tab::Paths;
                }
                if ui.selectable_label(self.tab == Tab::Lights, lights).clicked() {
                    self.tab = Tab::Lights;
                }
            });
            ui.horizontal(|ui| {
                *save = ui.button(icons::labelled(Icon::Save, save_label)).on_hover_text(save_hint).clicked();
                *revert = ui.button(icons::labelled(Icon::Undo, tr("Revert", "Отменить"))).on_hover_text(revert_hint).clicked();
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong());
                let note = if in_place {
                    tr("in its own file (a .bak keeps the old one)", "в своём файле (старый — в .bak)")
                } else if copied {
                    tr("openMapper's copy", "Копия openMapper")
                } else {
                    tr("as a new file (a copy is made on saving)", "новым файлом (копия делается при сохранении)")
                };
                ui.label(RichText::new(note).small().color(DIM));
                ui.separator();
                if ui.selectable_label(self.tab == Tab::Paths, paths).clicked() {
                    self.tab = Tab::Paths;
                }
                if ui.selectable_label(self.tab == Tab::Lights, lights).clicked() {
                    self.tab = Tab::Lights;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    menu(ui, self);
                    *save = ui.button(icons::labelled(Icon::Save, save_label)).on_hover_text(save_hint).clicked();
                    *revert = ui.button(icons::labelled(Icon::Undo, tr("Revert", "Отменить"))).on_hover_text(revert_hint).clicked();
                });
            });
        }
        if let Some((msg, bad)) = &self.message {
            ui.label(RichText::new(msg).small().color(if *bad { BAD } else { GOOD }));
        }
    }

    /// The paths page: the list and its numbers on the left, the crossing drawn on the right.
    fn paths_body(&mut self, ui: &mut egui::Ui, ed: &mut Editor, sheet: &mut Sheet, body: Vec2, mut assets: Option<&mut Assets>) {
        // (the object's own mesh, read once for the drawing: the crossing seen under its paths)
        if self.footprint.as_ref().map(|f| f.0 != self.rel).unwrap_or(true) {
            self.footprint = assets.as_deref_mut().and_then(|a| footprint_of(a, &self.rel));
        }
        if self.surface_for != self.rel {
            let rel = self.rel.clone();
            self.surface = std::sync::Arc::new(assets.and_then(|a| surface_tris(a, &rel)).unwrap_or_default());
            self.surface_for = rel;
        }
        let (left, rest) = (320.0f32, (body.x - 334.0).max(400.0));
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(left, body.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("crossing side").auto_shrink([false, false]).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(left - 18.0);
                        self.paths_side(ui, ed, sheet);
                    });
                });
            });
            ui.separator();
            ui.allocate_ui_with_layout(Vec2::new(rest, body.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                self.view_bar(ui);
                self.canvas_ui(ui, sheet);
            });
        });
    }

    /// The light page: the program on the left, its timeline on the right - the crossing
    /// editor's traffic light window.
    fn lights_body(&mut self, ui: &mut egui::Ui, _ed: &mut Editor, sheet: &mut Sheet, body: Vec2) {
        let (left, rest) = (352.0f32, (body.x - 366.0).max(400.0));
        let side = left - 20.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(left, body.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_max_width(side);
                // (a scroll area so a long request-loop or light list is never out of reach)
                egui::ScrollArea::vertical().id_salt("crossing lights side").auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(side);
                    self.lights_side(ui, sheet, side);
                });
            });
            ui.separator();
            ui.allocate_ui_with_layout(Vec2::new(rest, body.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                self.timeline_ui(ui, sheet);
            });
        });
    }

    fn no_target(&mut self, ui: &mut egui::Ui, ed: &mut Editor, roots: Option<&Roots>, lib: &Library) {
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Pick a crossing object of the map", "Возьмите объект перекрёстка на карте")).strong());
        ui.label(RichText::new(tr("The paths, the blocking and the traffic light program of an existing crossing are edited here. A save writes the object's own file, the file as it stood kept as a .bak beside it; a copy of the object can be made instead.", "Здесь правят пути, блокировки и программу светофора готового перекрёстка. Сохранение пишет собственный файл объекта, прежний остаётся рядом как .bak; вместо этого можно сделать копию объекта.")).small().color(DIM));
        if let Some((msg, bad)) = &self.message {
            ui.colored_label(if *bad { BAD } else { DIM }, msg);
        }
        ui.add_space(6.0);
        let mut pick = None;
        egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(80.0)).show(ui, |ui| {
            for (k, id, rel) in &self.candidates {
                // (its own name where the catalogue has one: the file's is a machine's)
                let name = lib.find(rel).map(|a| a.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| std::path::Path::new(&rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| rel.clone()));
                if ui.selectable_label(false, name).on_hover_text(format!("{rel}\n{}, {}", k.0, k.1)).clicked() {
                    pick = Some((*k, *id));
                }
            }
        });
        if let (Some((k, id)), Some(roots)) = (pick, roots) {
            if self.open_on(ed, roots, k, id) {
                self.message = None;
            }
        }
    }

    /// The paths page's left side: the list, and the numbers of the one picked.
    fn paths_side(&mut self, ui: &mut egui::Ui, ed: &mut Editor, sheet: &mut Sheet) {
        let Some(Sel::Object(..)) = self.target else { return };
        let mut changed = false;
        // (shown on the map, the panel is a narrow strip: the picked path's numbers come
        // first, since they are what is edited there, and the list follows)
        let numbers_first = self.view_3d && self.tab == Tab::Paths;
        // ---- the paths
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ({})", tr("Paths", "Пути"), sheet.crossing.paths.len())).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::labelled(Icon::Add, "")).on_hover_text(tr("Add a path carried on from the picked one (N)", "Добавить путь, продолжающий выбранный (N)")).clicked() {
                    self.push_undo();
                    // (from the picked path's end, in the way it runs there - not a copy
                    // put beside it: that is what the Duplicate button is for)
                    if let Some(at) = continue_path(&mut sheet.crossing, self.selected) {
                        self.selected = Some(at);
                    }
                    self.reveal = true;
                    changed = true;
                }
                if ui.small_button(icons::labelled(Icon::Copy, "")).on_hover_text(tr("Duplicate", "Дублировать")).clicked() {
                    if let Some(q) = self.selected.and_then(|i| sheet.crossing.paths.get(i)).cloned() {
                        sheet.crossing.paths.push(q);
                        self.selected = Some(sheet.crossing.paths.len() - 1);
                        changed = true;
                    }
                }
            });
        });
        // (a new path started at the end of a nearby path - another object's, a road's own
        // lane or another path of this crossing - which is then clicked in the drawing or on
        // the map; or the picked path carried on to one of them)
        let from_nearby = self.from_nearby;
        let complete_to = self.complete_to;
        let start_hint = tr("Start a new path at the end of a path nearby. Click one of the green circles", "Начать новый путь у конца соседнего пути. Щёлкните зелёный кружок");
        let complete_hint = tr("Carry the picked path on to a nearby path's end: the picked path keeps its end, way and rise, and one piece or two join them. Click one of the green circles", "Продолжить выбранный путь до конца соседнего: у выбранного остаются конец, направление и уклон, а соединяют их один или два отрезка. Щёлкните зелёный кружок");
        if ui.selectable_label(from_nearby, RichText::new(tr("Start from a nearby path", "Начать от соседнего пути")).small().color(if from_nearby { SNAPPED } else { DIM })).on_hover_text(start_hint).clicked() {
            let on = !self.from_nearby;
            self.set_from_nearby(on);
        }
        if ui.selectable_label(complete_to, RichText::new(tr("Complete to a nearby path", "Довести до соседнего пути")).small().color(if complete_to { SNAPPED } else { DIM })).on_hover_text(complete_hint).clicked() {
            let on = !self.complete_to;
            self.set_complete_to(on);
        }
        let mut remove = None;
        let mut move_to = None;
        if numbers_first {
            changed |= self.path_numbers(ui, sheet);
        }
        if self.from_nearby {
            ui.label(RichText::new(tr("Click a green circle: a new path starts at that nearby path's end, in its way", "Щёлкните зелёный кружок: новый путь начнётся у конца того соседнего пути, в его направлении")).small().color(SNAPPED));
        } else if self.complete_to {
            ui.label(RichText::new(tr("Click a green circle: one piece or two are added from the picked path's end to that one, the picked path left as it is", "Щёлкните зелёный кружок: от конца выбранного пути добавятся один или два отрезка до него, выбранный остаётся как есть")).small().color(SNAPPED));
        }
        ui.separator();
        #[cfg(test)]
        {
            self.last_list_y = Some(ui.cursor().top());
        }
        egui::ScrollArea::vertical().id_salt("crossing paths").max_height(if numbers_first { 110.0 } else { 150.0 }).auto_shrink([false, true]).show(ui, |ui| {
            for i in 0..sheet.crossing.paths.len() {
                let p = &sheet.crossing.paths[i];
                let mut text = format!("#{i}  {}  {:.1} m", kind_name(p.kind()), p.length());
                if p.turn() != TURN_NONE {
                    text.push_str(&format!("  {}", turn_name(p.turn())));
                }
                if let Some(l) = p.light {
                    text.push_str(&format!("  L{l}"));
                }
                if !p.blocks.is_empty() {
                    text.push_str(&format!("  ⛔{}", p.blocks.len()));
                }
                let on = self.selected == Some(i);
                let row = ui.horizontal(|ui| {
                    if ui.selectable_label(on, RichText::new(text).color(kind_color(p.kind()))).clicked() {
                        self.selected = Some(i);
                    }
                    if i > 0 && ui.small_button("↑").on_hover_text(tr("Move up", "Выше")).clicked() {
                        move_to = Some((i, i - 1));
                    }
                    if i + 1 < sheet.crossing.paths.len() && ui.small_button("↓").on_hover_text(tr("Move down", "Ниже")).clicked() {
                        move_to = Some((i, i + 1));
                    }
                    if ui.small_button(icons::labelled(Icon::Delete, "")).on_hover_text(tr("Remove this path", "Удалить этот путь")).clicked() {
                        remove = Some(i);
                    }
                });
                // (picked on the map: the list is brought to it, once)
                if on && self.reveal {
                    row.response.scroll_to_me(Some(egui::Align::Center));
                    self.reveal = false;
                }
            }
        });
        if let Some((from, to)) = move_to {
            sheet.crossing.move_path(from, to);
            self.selected = Some(to);
            changed = true;
        }
        if let Some(i) = remove {
            sheet.crossing.remove_path(i);
            self.selected = if sheet.crossing.paths.is_empty() { None } else { Some(i.min(sheet.crossing.paths.len() - 1)) };
            changed = true;
        }

        // ---- the numbers of the picked path
        // (in the 3D panel they were drawn above, before the list: what is edited there is
        // the picked path's numbers, and they must not be below the fold)
        if !numbers_first {
            changed |= self.path_numbers(ui, sheet);
        }

        // ---- the spline helpers
        ui.separator();
        let mut helper_gone = None;
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Spline helpers", "Точки привязки сплайнов"), sheet.crossing.helpers.len())).strong()).id_salt("crossing helpers").default_open(false).show(ui, |ui| {
            ui.label(RichText::new(tr("Where a road of the map snaps onto the crossing. x, height, forward in the object's frame", "Где дорога карты пристыковывается к перекрёстку. x, высота, вперёд в системе объекта")).small().color(DIM));
            for (n, h) in sheet.crossing.helpers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{n}")).small().color(DIM));
                    changed |= ui.add(egui::TextEdit::singleline(&mut h.spline).desired_width(120.0)).on_hover_text(tr("The road type it takes (a `.sli`)", "Тип дороги, который он принимает (`.sli`)")).changed();
                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                        helper_gone = Some(n);
                    }
                });
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut h.x).speed(0.05).prefix("x ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.fwd).speed(0.05).prefix("f ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.up).speed(0.05).prefix("h ")).changed();
                });
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut h.heading).speed(0.5).suffix("°")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.length).speed(0.05).prefix("len ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.radius).speed(0.1).prefix("r ")).changed();
                });
            }
            if let Some(n) = helper_gone {
                sheet.crossing.helpers.remove(n);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button(icons::labelled(Icon::Add, tr("A helper", "Точка"))).clicked() {
                    let spline = ed.road_type.clone().unwrap_or_default();
                    sheet.crossing.helpers.push(Helper { spline, x: 0.0, up: 0.0, fwd: 0.0, heading: 0.0, length: 0.1, radius: 0.0 });
                    changed = true;
                }
                if ui.button(tr("At the picked end", "У выбранного конца")).on_hover_text(tr("A helper where the picked path ends, facing out of the crossing", "Точка там, где кончается выбранный путь, наружу от перекрёстка")).clicked() {
                    if let Some(p) = self.selected.and_then(|i| sheet.crossing.paths.get(i)) {
                        let at = p.end();
                        let c = p.curve();
                        let heading = c.heading_at(c.length) as f32;
                        let spline = ed.road_type.clone().unwrap_or_default();
                        sheet.crossing.helpers.push(Helper { spline, x: at.x as f32, up: at.z as f32, fwd: at.y as f32, heading, length: 0.1, radius: 0.0 });
                        changed = true;
                    }
                }
            });
        });

        if changed {
            // (nothing to do: the view and the map both draw the sheet as it is each frame;
            // the file is only written on Save)
            let _ = changed;
        }
    }

    /// Turn one end of path `index` to `heading`. The other end's angle and both points stay.
    /// A turn one arc cannot make is several pieces, rebuilt from the path as the drag began.
    fn apply_rotation(&mut self, crossing: &mut Crossing, index: usize, at_end: bool, heading: f64) {
        if index >= crossing.paths.len() {
            return;
        }
        let fresh = self.rot_drag.as_ref().map(|g| g.index != index || g.at_end != at_end).unwrap_or(true);
        if fresh {
            self.rot_drag = Some(RotGesture { index, at_end, original: crossing.paths[index].clone(), spawned: 0 });
        }
        let Some(g) = self.rot_drag.as_ref() else { return };
        let (index, spawned, original) = (g.index, g.spawned, g.original.clone());
        if index >= crossing.paths.len() {
            return;
        }
        drop_inserted(&mut crossing.paths, index + 1, spawned);
        crossing.paths[index] = original;
        let base = &crossing.paths[index];
        let h0 = if at_end { base.heading() } else { heading };
        let h1 = if at_end {
            heading
        } else {
            let c = base.curve();
            c.heading_at(c.length.max(0.0))
        };
        let pieces = retarget(base, h0, h1);
        if pieces.is_empty() {
            if let Some(g) = self.rot_drag.as_mut() {
                g.spawned = 0;
            }
            return;
        }
        let extra = pieces.len() - 1;
        crossing.paths[index] = pieces[0].clone();
        insert_after(&mut crossing.paths, index, pieces.into_iter().skip(1).collect());
        if let Some(g) = self.rot_drag.as_mut() {
            g.spawned = extra;
        }
    }

    /// The picked path's numbers: where it starts, how it runs (heading, radius, length), its
    /// slopes and width, the way traffic goes, its turn signal and light, what it blocks.
    /// True when one of them was changed.
    fn path_numbers(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet) -> bool {
        let Some(i) = self.selected.filter(|i| *i < sheet.crossing.paths.len()) else { return false };
        if self.rot_drag.as_ref().is_some_and(|g| g.index != i) {
            self.rot_drag = None;
        }
        let mut changed = false;
        let mut rot_edit = false;
        ui.separator();
        #[cfg(test)]
        {
            self.last_numbers_y = Some(ui.cursor().top());
        }
        let crossing = &mut sheet.crossing;
        let p = &mut crossing.paths[i];
        egui::Grid::new("crossing numbers").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
            ui.label(RichText::new(tr("Start point", "Начало")).small().color(DIM));
            ui.horizontal(|ui| {
                changed |= ui.add(egui::DragValue::new(&mut p.v[0]).speed(0.05).prefix("x ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.v[1]).speed(0.05).prefix("y ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.v[2]).speed(0.05).prefix("z ")).changed();
            });
            ui.end_row();
            // (the end, as the file gives it: x and y move the end itself, z is the slopes'
            // doing - typing one sets both slopes to the rise it asks for)
            ui.label(RichText::new(tr("End point", "Конец")).small().color(DIM));
            let mut end = p.end();
            let (mut ex, mut ey, mut ez) = (end.x, end.y, end.z);
            ui.horizontal(|ui| {
                let cx = ui.add(egui::DragValue::new(&mut ex).speed(0.05).prefix("x ")).changed();
                let cy = ui.add(egui::DragValue::new(&mut ey).speed(0.05).prefix("y ")).changed();
                let cz = ui.add(egui::DragValue::new(&mut ez).speed(0.05).prefix("z ")).changed();
                if cx || cy {
                    end.x = ex;
                    end.y = ey;
                    p.set_end(end);
                    changed = true;
                }
                if cz {
                    set_end_height(p, ez);
                    changed = true;
                }
            });
            ui.end_row();
            ui.label(RichText::new(tr("Radius", "Радиус")).small().color(DIM));
            let mut r = p.radius();
            if ui.add(egui::DragValue::new(&mut r).speed(0.1).suffix(" m")).on_hover_text(tr("0 is a straight line, above 0 turns right, below 0 left", "0 — прямая, больше 0 — направо, меньше 0 — налево")).changed() {
                p.v[4] = r as f32;
                changed = true;
            }
            ui.end_row();
            ui.label(RichText::new(tr("Length", "Длина")).small().color(DIM));
            let mut l = p.length();
            if ui.add(egui::DragValue::new(&mut l).speed(0.1).range(0.0..=2000.0).suffix(" m")).changed() {
                p.v[5] = l as f32;
                changed = true;
            }
            ui.end_row();
            ui.label(RichText::new(tr("Slope", "Уклон")).small().color(DIM));
            ui.horizontal(|ui| {
                let mut a = p.slope_start() * 100.0;
                let mut b = p.slope_end() * 100.0;
                if ui.add(egui::DragValue::new(&mut a).speed(0.05).suffix(" %")).changed() {
                    p.v[6] = (a / 100.0) as f32;
                    changed = true;
                }
                if ui.add(egui::DragValue::new(&mut b).speed(0.05).suffix(" %")).changed() {
                    p.v[7] = (b / 100.0) as f32;
                    changed = true;
                }
                if ui.button(tr("Even grade", "Одинаковый уклон")).on_hover_text(tr("Both ends take the one rise between their heights", "Оба конца с одним уклоном между их высотами")).clicked() {
                    let z = p.end().z;
                    set_end_height(p, z);
                    changed = true;
                }
            });
            ui.end_row();
            ui.label(RichText::new(tr("Kind", "Тип")).small().color(DIM));
            let mut k = p.v[8] as i32;
            let before = k;
            egui::ComboBox::from_id_salt("crossing kind").selected_text(kind_name(k)).width(150.0).show_ui(ui, |ui| {
                for n in [0, 1, 2] {
                    ui.selectable_value(&mut k, n, kind_name(n));
                }
            });
            p.v[8] = k as f32;
            changed |= k != before;
            ui.end_row();
            ui.label(RichText::new(tr("Width", "Ширина")).small().color(DIM));
            changed |= ui.add(egui::DragValue::new(&mut p.v[9]).speed(0.05).range(0.0..=30.0).suffix(" m")).changed();
            ui.end_row();
            ui.label(RichText::new(tr("Direction", "Направление движения")).small().color(DIM));
            let mut d = p.v[10] as i32;
            let before = d;
            egui::ComboBox::from_id_salt("crossing direction").selected_text(direction_name(d)).width(150.0).show_ui(ui, |ui| {
                for n in [0, 1, 2] {
                    ui.selectable_value(&mut d, n, direction_name(n));
                }
            });
            p.v[10] = d as f32;
            changed |= d != before;
            ui.end_row();
            ui.label(RichText::new(tr("Turn signal", "Поворот")).small().color(DIM));
            let mut t = p.v[11] as i32;
            let before = t;
            egui::ComboBox::from_id_salt("crossing turn").selected_text(turn_name(t)).width(150.0).show_ui(ui, |ui| {
                for n in [TURN_NONE, TURN_LEFT, TURN_RIGHT] {
                    ui.selectable_value(&mut t, n, turn_name(n));
                }
            });
            p.v[11] = t as f32;
            changed |= t != before;
            ui.end_row();
            ui.label(RichText::new(tr("Light", "Светофор")).small().color(DIM));
            let names: Vec<(i32, String)> = std::iter::once((-1, tr("None", "Нет").to_string())).chain(crossing.lights.iter().enumerate().map(|(n, l)| (n as i32, format!("#{n} {}", if l.name.trim().is_empty() { "-" } else { l.name.trim() })))).collect();
            let mut cur = p.light.unwrap_or(-1);
            let before = cur;
            egui::ComboBox::from_id_salt("crossing light").selected_text(names.iter().find(|(n, _)| *n == cur).map(|(_, s)| s.clone()).unwrap_or_else(|| tr("None", "Нет").into())).width(150.0).show_ui(ui, |ui| {
                for (n, s) in &names {
                    ui.selectable_value(&mut cur, *n, s);
                }
            });
            if cur != before {
                p.light = (cur >= 0).then_some(cur);
                changed = true;
            }
            ui.end_row();
        });
        // (start and end rotation, as a road's two ends have: each turns on its own, and the
        // point stays. Read after the grid so a number edited above is already on the path)
        {
            let spawned = self.rot_drag.as_ref().filter(|g| g.index == i).map(|g| g.spawned).unwrap_or(0);
            let last = (i + spawned).min(sheet.crossing.paths.len() - 1);
            let mut h0 = crate::roads::norm_deg(sheet.crossing.paths[i].heading());
            let end_c = sheet.crossing.paths[last].curve();
            let mut h1 = crate::roads::norm_deg(end_c.heading_at(end_c.length.max(0.0)));
            let leave = tr("How this path leaves, degrees. The point stays; a turn one arc cannot make is split into pieces", "Как путь уходит, градусы. Точка остаётся; поворот, который одной дуге не сделать, делится на части");
            let arrive = tr("How this path arrives, degrees. The point stays; a turn one arc cannot make is split into pieces", "Как путь приходит, градусы. Точка остаётся; поворот, который одной дуге не сделать, делится на части");
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Rotate", "Поворот")).small().color(DIM));
                let r0 = ui.add(egui::DragValue::new(&mut h0).speed(0.4).max_decimals(1).suffix("°").prefix(format!("{} ", tr("Start", "Начало")))).on_hover_text(leave);
                let r1 = ui.add(egui::DragValue::new(&mut h1).speed(0.4).max_decimals(1).suffix("°").prefix(format!("{} ", tr("End", "Конец")))).on_hover_text(arrive);
                if r0.changed() {
                    self.apply_rotation(&mut sheet.crossing, i, false, h0);
                    changed = true;
                    rot_edit = true;
                } else if r1.changed() {
                    self.apply_rotation(&mut sheet.crossing, i, true, h1);
                    changed = true;
                    rot_edit = true;
                }
                if r0.drag_stopped() || r1.drag_stopped() {
                    if let Some(g) = self.rot_drag.take() {
                        if g.at_end {
                            self.selected = Some((g.index + g.spawned).min(sheet.crossing.paths.len().saturating_sub(1)));
                        }
                    }
                }
            });
        }
        let blocked = ui.checkbox(&mut sheet.crossing.paths[i].crossing_problem, tr("Keeps the junction clear", "Держит перекрёсток свободным")).on_hover_text(tr("A vehicle waiting on this path holds the crossing empty for the others (`[crossingproblem]`)", "Машина на этом пути держит перекрёсток свободным для других (`[crossingproblem]`)")).changed();
        changed |= blocked;
        if blocked {
            rot_edit = false;
        }
        // blocking: which other paths wait while this one is taken
        let others: Vec<(usize, String)> = (0..sheet.crossing.paths.len()).filter(|j| *j != i).map(|j| (j, format!("#{j} {}", kind_name(sheet.crossing.paths[j].kind())))).collect();
        if !others.is_empty() {
            let n = sheet.crossing.paths[i].blocks.len();
            egui::CollapsingHeader::new(RichText::new(format!("{} ({n})", tr("Blocks", "Блокирует"))).small()).id_salt("crossing blocks").default_open(n > 0).show(ui, |ui| {
                for (j, label) in &others {
                    let mut on = sheet.crossing.paths[i].blocks.iter().any(|(a, _)| *a == *j as i32);
                    if ui.checkbox(&mut on, label).changed() {
                        let p = &mut sheet.crossing.paths[i];
                        p.blocks.retain(|(a, _)| *a != *j as i32);
                        if on {
                            p.blocks.push((*j as i32, 0));
                        }
                        changed = true;
                        rot_edit = false;
                    }
                }
            });
        }
        if changed && !rot_edit {
            self.rot_drag = None;
        }
        changed
    }

    /// Above the drawing: the zoom, laying a path, and the map's own view of the object.
    fn view_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("−").on_hover_text(tr("Zoom out", "Отдалить")).clicked() {
                self.scale = (self.scale / 1.35).clamp(0.6, 400.0);
            }
            if ui.button("+").on_hover_text(tr("Zoom in", "Приблизить")).clicked() {
                self.scale = (self.scale * 1.35).clamp(0.6, 400.0);
            }
            if ui.button(tr("Fit", "Вписать")).on_hover_text(tr("The whole crossing in view", "Весь перекрёсток в окне")).clicked() {
                self.fit = true;
            }
            ui.separator();
            // (a path laid with the mouse, from where it is pressed to where it is let go -
            // as a road is laid with the spline tool)
            if ui.selectable_label(self.draw, icons::labelled(Icon::Add, tr("Draw path", "Нарисовать путь"))).on_hover_text(tr("Drag from a path's start to its end to add one, as a road is drawn", "Проведите от начала пути к его концу, чтобы добавить путь, как рисуют дорогу")).clicked() {
                self.draw = !self.draw;
            }
            ui.separator();
            let show_3d = self.view_3d;
            if ui.selectable_label(show_3d, tr("3D", "3D")).on_hover_text(tr("Show the path on the map itself: the crossing object in 3D, with its paths", "Показать пути на самой карте: объект перекрёстка в 3D со своими путями")).clicked() {
                // (the map is looked at as it is: pressing 3D only changes where the paths are
                // drawn, it does not take the view somewhere else. `Show on the map` frames it.)
                self.view_3d = true;
                self.draw = false;
            }
            if ui.selectable_label(!show_3d, tr("2D", "2D")).on_hover_text(tr("Draw the crossing from above, in the window", "Рисовать перекрёсток сверху, в окне")).clicked() {
                self.view_3d = false;
                self.frame_on_map = false;
            }
            ui.label(RichText::new(tr("Drag an end to move it. Snaps to another path's end; hold Ctrl to move it freely. A snapped end takes the other path's way and rise too, so the two meet smoothly. Wheel zooms, right or middle drag pans", "Тяните конец, чтобы сдвинуть его. Притягивается к концу другого пути; с Ctrl — свободно. Притянутый конец принимает и направление, и уклон другого пути, так что они сходятся плавно. Колесо — масштаб, правая/средняя кнопка — сдвиг")).small().color(DIM));
        });
    }

    /// The light page's left side: which light, which of its phases, what that phase shows,
    /// the detector and the request loops - as the crossing editor's traffic light window.
    fn lights_side(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet, width: f32) {
        if self.light.is_some_and(|l| l >= sheet.crossing.lights.len()) {
            self.light = None;
            self.phase = None;
        }
        if self.light.is_none() && !sheet.crossing.lights.is_empty() {
            self.light = Some(0);
        }
        let count = sheet.crossing.lights.len();

        // ---- which light, its name and the cycle it runs in (the window's own top box)
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(width - 12.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Traffic light", "Светофор")).strong());
                let shown = self.light.map(|i| i + 1).unwrap_or(0);
                ui.label(RichText::new(format!("{shown} / {count}")).monospace().small().color(DIM));
                if ui.add_enabled(self.light.is_some_and(|i| i > 0), egui::Button::new("<").small()).clicked() {
                    self.light = self.light.map(|i| i - 1);
                    self.phase = Some(0);
                }
                if ui.add_enabled(self.light.is_some_and(|i| i + 1 < count), egui::Button::new(">").small()).clicked() {
                    self.light = self.light.map(|i| i + 1);
                    self.phase = Some(0);
                }
                if ui.small_button(icons::labelled(Icon::Add, "")).on_hover_text(tr("A light", "Светофор")).clicked() {
                    self.light = Some(sheet.crossing.add_light());
                    self.phase = Some(0);
                }
                if ui.add_enabled(self.light.is_some(), egui::Button::new(icons::labelled(Icon::Delete, "")).small()).on_hover_text(tr("Remove this light", "Удалить этот светофор")).clicked() {
                    if let Some(i) = self.light {
                        sheet.crossing.remove_light(i);
                        self.light = if sheet.crossing.lights.is_empty() { None } else { Some(i.min(sheet.crossing.lights.len() - 1)) };
                        self.phase = Some(0);
                    }
                }
            });
            if let Some(i) = self.light {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Name", "Имя")).small().color(DIM));
                    ui.add(egui::TextEdit::singleline(&mut sheet.crossing.lights[i].name).desired_width(ui.available_width()));
                });
                ui.horizontal(|ui| {
                    let mut on = sheet.crossing.has_cycle();
                    if ui.checkbox(&mut on, tr("Cycle", "Цикл")).on_hover_text(tr("`[traffic_lights_group]`: the seconds of one run. Unset, the longest light's phases add up to it", "`[traffic_lights_group]`: секунды одного круга. Если не задан — сумма фаз самой длинной программы")).changed() {
                        sheet.crossing.cycle = if on { Some(sheet.crossing.phases_sum().max(1.0)) } else { None };
                    }
                    let len = sheet.crossing.cycle_len();
                    if let Some(c) = sheet.crossing.cycle.as_mut() {
                        ui.add(egui::DragValue::new(c).speed(0.5).range(1.0..=600.0).suffix(" s"));
                    } else {
                        ui.label(RichText::new(format!("{len:.0} s")).monospace().small().color(DIM));
                    }
                });
            } else if count == 0 {
                ui.label(RichText::new(tr("A crossing with lights needs the signal heads on the map: the game only runs a program that heads are linked to", "Перекрёстку со светофором нужны головки на карте: игра запускает программу, только если к ней привязаны головки")).small().color(DIM));
            }
        });

        // ---- the phase and what it shows (the window's "Traffic Light Phase" box)
        ui.add_space(3.0);
        if let Some(i) = self.light {
            let n = sheet.crossing.lights[i].phases.len();
            if self.phase.is_some_and(|p| p >= n) {
                self.phase = if n > 0 { Some(n - 1) } else { None };
            }
            if self.phase.is_none() && n > 0 {
                self.phase = Some(0);
            }
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(width - 12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Phase", "Фаза")).strong());
                    ui.label(RichText::new(format!("{} / {n}", self.phase.map(|p| p + 1).unwrap_or(0))).monospace().small().color(DIM));
                    if ui.add_enabled(self.phase.is_some_and(|p| p > 0), egui::Button::new("<").small()).clicked() {
                        self.phase = self.phase.map(|p| p - 1);
                    }
                    if ui.add_enabled(self.phase.is_some_and(|p| p + 1 < n), egui::Button::new(">").small()).clicked() {
                        self.phase = self.phase.map(|p| p + 1);
                    }
                    if ui.small_button(icons::labelled(Icon::Add, "")).on_hover_text(tr("A phase", "Фаза")).clicked() {
                        let at = self.phase.map(|p| p + 1).unwrap_or(n);
                        self.phase = sheet.crossing.insert_phase(i, at);
                    }
                    if ui.add_enabled(self.phase.is_some(), egui::Button::new(icons::labelled(Icon::Delete, "")).small()).on_hover_text(tr("Delete", "Удалить")).clicked() {
                        if let Some(p) = self.phase {
                            sheet.crossing.remove_phase(i, p);
                            let n = sheet.crossing.lights[i].phases.len();
                            self.phase = if n == 0 { None } else { Some(p.min(n - 1)) };
                        }
                    }
                });
                if let Some(p) = self.phase {
                    let mut state = sheet.crossing.lights[i].phases[p].state;
                    // (the states laid out three to a row, as the crossing editor's own list)
                    egui::Grid::new("crossing aspects").num_columns(3).spacing([8.0, 0.0]).max_col_width(width / 3.0 - 12.0).show(ui, |ui| {
                        for code in 0..=12 {
                            let label = RichText::new(format!("{code} {}", aspect_symbol(code))).small().color(aspect_color(code));
                            ui.radio_value(&mut state, code, label).on_hover_text(format!("{} ({})", aspect_name(code), aspect_symbol(code)));
                            if code % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                    if state != sheet.crossing.lights[i].phases[p].state {
                        sheet.crossing.lights[i].phases[p].state = state;
                    }
                    let mut det = sheet.crossing.lights[i].approach_dist;
                    let mut on = det.is_some();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("Duration", "Длительность")).small().color(DIM));
                        ui.add(egui::DragValue::new(&mut sheet.crossing.lights[i].phases[p].duration).speed(0.2).range(0.0..=900.0).suffix(" s"));
                        ui.separator();
                        if ui.checkbox(&mut on, tr("Detector", "Мелдер")).on_hover_text(tr("`[approachdist]`: how far before the stop line a vehicle registers a request", "`[approachdist]`: за сколько метров до стоп-линии машина запрашивает")).changed() {
                            det = if on { Some(det.unwrap_or(10.0).max(0.0)) } else { None };
                        }
                        if let Some(d) = det.as_mut() {
                            ui.add(egui::DragValue::new(d).speed(0.2).range(0.0..=200.0).suffix(" m"));
                        }
                    });
                    sheet.crossing.lights[i].approach_dist = det;
                    let sum = sheet.crossing.phases_sum();
                    let len = sheet.crossing.cycle_len();
                    if (sum - len).abs() > 0.5 {
                        ui.label(RichText::new(tr("The phases do not add up to the cycle: the last one holds to its end", "Фазы не складываются в цикл: последняя длится до его конца")).small().color(DIM));
                    }
                }
            });
        }

        // ---- the lights of the crossing, as the window's own list
        ui.add_space(3.0);
        egui::ScrollArea::vertical().id_salt("crossing light list").max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
            for (k, l) in sheet.crossing.lights.iter().enumerate() {
                let name = if l.name.trim().is_empty() { tr("(no name)", "(без имени)") } else { l.name.trim() };
                let color = l.phases.first().map(|p| aspect_color(p.state)).unwrap_or(DIM);
                if ui.selectable_label(self.light == Some(k), RichText::new(format!("{k}: {name}")).color(color)).clicked() {
                    self.light = Some(k);
                    self.phase = Some(0);
                }
            }
        });

        // ---- the request loops, as the numbers they are
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Request loops", "Петли запроса"), sheet.crossing.stops.len() + sheet.crossing.jumps.len())).small()).id_salt("crossing stops").default_open(false).show(ui, |ui| {
            let mut stop_gone = None;
            for (n, s) in sheet.crossing.stops.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("stop", "стоп")).small().color(DIM));
                    ui.add(egui::DragValue::new(&mut s[0]).speed(0.2).prefix("L"));
                    ui.add(egui::DragValue::new(&mut s[1]).speed(0.2).suffix(" s"));
                    ui.add(egui::DragValue::new(&mut s[2]).range(0.0..=1.0).speed(0.1).prefix("req "));
                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                        stop_gone = Some(n);
                    }
                });
            }
            if let Some(n) = stop_gone {
                sheet.crossing.stops.remove(n);
            }
            let mut jump_gone = None;
            for (n, j) in sheet.crossing.jumps.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("jump", "переход")).small().color(DIM));
                    ui.add(egui::DragValue::new(&mut j[0]).speed(0.2).prefix("L"));
                    ui.add(egui::DragValue::new(&mut j[1]).speed(0.2).suffix(" s"));
                    ui.add(egui::DragValue::new(&mut j[2]).range(0.0..=1.0).speed(0.1).prefix("req "));
                    ui.add(egui::DragValue::new(&mut j[3]).speed(0.2).prefix("→ ").suffix(" s"));
                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                        jump_gone = Some(n);
                    }
                });
            }
            if let Some(n) = jump_gone {
                sheet.crossing.jumps.remove(n);
            }
        });
    }

    /// The light program's timeline: a row per light, a column per second, each phase a block
    /// of its colour, the phase's number written in it - the traffic light window's grid.
    fn timeline_ui(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet) {
        let cycle = sheet.crossing.cycle_len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Timeline", "Временная диаграмма")).strong());
            ui.label(RichText::new(format!("{cycle:.0} s")).monospace().small().color(DIM));
            if ui.small_button("−").on_hover_text(tr("Zoom out", "Отдалить")).clicked() {
                self.sec_px = (self.sec_px / 1.25).clamp(3.0, 80.0);
                self.sec_fit = false;
            }
            if ui.small_button("+").on_hover_text(tr("Zoom in", "Приблизить")).clicked() {
                self.sec_px = (self.sec_px * 1.25).clamp(3.0, 80.0);
                self.sec_fit = false;
            }
            if ui.small_button(tr("Fit", "Вписать")).on_hover_text(tr("The whole cycle across the width (as the traffic light window has it)", "Весь цикл по ширине (как в окне светофора)")).clicked() {
                self.sec_fit = true;
            }
        });
        // (a second per pixel width the whole cycle fits in, until the zoom is used: the
        // window shows the program end to end, as OMSI's own does)
        if self.sec_fit && cycle > 0.0 {
            let room = (ui.available_width() - GUTTER - 24.0).max(120.0);
            self.sec_px = (room / cycle).clamp(2.0, 40.0);
        }
        ui.label(RichText::new(tr("Click a block to edit that phase; drag its right edge to change how long it lasts", "Нажмите на блок, чтобы править эту фазу; потяните его правый край, чтобы изменить её длительность")).small().color(DIM));

        let rows = sheet.crossing.lights.len();
        let size = Vec2::new((GUTTER + cycle * self.sec_px + 18.0).max(ui.available_width()), HEAD + rows.max(1) as f32 * ROW_H + 2.0);
        egui::ScrollArea::both().id_salt("crossing timeline").auto_shrink([false, false]).show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
            #[cfg(test)]
            {
                self.last_timeline = Some(rect);
            }
            let p = ui.painter_at(rect);
            let x = |t: f32| rect.left() + GUTTER + t * self.sec_px;
            let top = rect.top() + HEAD;
            p.rect_filled(rect, egui::CornerRadius::ZERO, Color32::from_rgb(26, 28, 26));
            p.rect_filled(egui::Rect::from_min_max(rect.min, Pos2::new(rect.right(), top)), egui::CornerRadius::ZERO, Color32::from_rgb(38, 41, 38));

            // the seconds: a line each, a number every five
            let tick = if self.sec_px >= 9.0 { 1.0 } else if self.sec_px >= 4.5 { 2.0 } else { 5.0 };
            let mut t = 0.0f32;
            while t <= cycle + 0.01 {
                let major = (t % 5.0).abs() < 1e-3;
                let xa = x(t);
                p.line_segment([Pos2::new(xa, rect.top()), Pos2::new(xa, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(if major { 42 } else { 14 })));
                if major && self.sec_px >= 7.0 || (t % 10.0).abs() < 1e-3 {
                    p.text(Pos2::new(xa + 2.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, format!("{t:.0}"), egui::FontId::monospace(9.0), DIM);
                }
                t += tick;
            }

            // the lights, one row each
            let segs = sheet.crossing.timeline();
            for i in 0..rows {
                let y0 = top + i as f32 * ROW_H;
                let row = egui::Rect::from_min_max(Pos2::new(rect.left(), y0), Pos2::new(rect.right(), y0 + ROW_H));
                if i % 2 == 1 {
                    p.rect_filled(row, egui::CornerRadius::ZERO, Color32::from_white_alpha(6));
                }
                if self.light == Some(i) {
                    p.rect_filled(egui::Rect::from_min_max(row.min, Pos2::new(rect.left() + GUTTER, row.bottom())), egui::CornerRadius::ZERO, Color32::from_rgb(46, 54, 48));
                }
                p.line_segment([Pos2::new(rect.left(), row.bottom()), Pos2::new(rect.right(), row.bottom())], Stroke::new(1.0, Color32::from_white_alpha(18)));
                let l = &sheet.crossing.lights[i];
                let name = if l.name.trim().is_empty() { "-" } else { l.name.trim() };
                p.text(Pos2::new(rect.left() + 5.0, y0 + ROW_H / 2.0), egui::Align2::LEFT_CENTER, format!("{i}  {name}"), egui::FontId::monospace(9.0), if self.light == Some(i) { HANDLE } else { DIM });
                p.line_segment([Pos2::new(rect.left() + GUTTER, y0), Pos2::new(rect.left() + GUTTER, y0 + ROW_H)], Stroke::new(1.0, Color32::from_white_alpha(28)));
                for s in segs.iter().filter(|s| s.light == i) {
                    let block = egui::Rect::from_min_max(
                        Pos2::new(x(s.start), y0 + 1.0),
                        Pos2::new(x(s.end).max(x(s.start) + 1.0), y0 + ROW_H - 1.0),
                    );
                    let fill = aspect_color(s.state);
                    p.rect_filled(block, egui::CornerRadius::ZERO, fill);
                    if s.own_end < s.end - 0.001 {
                        // (the last phase holds to the cycle's end: that tail is paler)
                        p.rect_filled(egui::Rect::from_min_max(Pos2::new(x(s.own_end), block.top()), block.max), egui::CornerRadius::ZERO, fill.gamma_multiply(0.55));
                    }
                    let on = self.light == Some(i) && self.phase == Some(s.phase);
                    if on {
                        p.rect_stroke(block, egui::CornerRadius::ZERO, Stroke::new(1.4, HANDLE), egui::StrokeKind::Inside);
                    }
                    if block.width() >= 13.0 {
                        p.text(block.center(), egui::Align2::CENTER_CENTER, s.phase.to_string(), egui::FontId::monospace(9.0), Color32::from_black_alpha(190));
                    }
                }
            }
            p.rect_stroke(rect, egui::CornerRadius::ZERO, Stroke::new(1.0, Color32::from_white_alpha(30)), egui::StrokeKind::Inside);

            // ---- the mouse
            let time_at = |pos: Pos2| ((pos.x - rect.left() - GUTTER) / self.sec_px).clamp(0.0, cycle);
            let row_at = |pos: Pos2| -> Option<usize> {
                if pos.y < top {
                    return None;
                }
                let i = ((pos.y - top) / ROW_H).floor() as usize;
                (i < rows).then_some(i)
            };
            let edge_near = |pos: Pos2| -> Option<(usize, usize)> {
                let i = row_at(pos)?;
                let near = segs.iter().find(|s| s.light == i && (x(s.end) - pos.x).abs() <= 4.0)?;
                Some((i, near.phase))
            };
            if resp.hovered() && resp.hover_pos().and_then(edge_near).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if resp.drag_started_by(egui::PointerButton::Primary) {
                self.edge = None;
                // (where the press went down: by the time a drag starts the pointer has moved)
                let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
                if let Some(pos) = origin {
                    if let Some(i) = row_at(pos) {
                        if pos.x < rect.left() + GUTTER {
                            self.light = Some(i);
                            self.phase = Some(0);
                        } else if let Some((l, ph)) = edge_near(pos) {
                            // (on a block's edge: that phase is the one being dragged)
                            self.light = Some(l);
                            self.phase = Some(ph);
                            self.edge = Some((l, ph));
                        } else if let Some(ph) = sheet.crossing.phase_at(i, time_at(pos)) {
                            self.light = Some(i);
                            self.phase = Some(ph);
                        }
                    }
                }
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                if let (Some((l, ph)), Some(pos)) = (self.edge, resp.interact_pointer_pos()) {
                    sheet.crossing.resize_phase(l, ph, time_at(pos));
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                self.edge = None;
            }
            if resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    if let Some(i) = row_at(pos) {
                        self.light = Some(i);
                        if pos.x < rect.left() + GUTTER {
                            self.phase = Some(0);
                        } else if let Some(ph) = sheet.crossing.phase_at(i, time_at(pos)) {
                            self.phase = Some(ph);
                        }
                    }
                }
            }
        });
    }

    fn save(&mut self, ed: &mut Editor, roots: &Roots, assets: &mut Assets) {
        let Some(Sel::Object(k, id)) = self.target else { return };
        let Some(sheet) = self.sheet.as_ref() else { return };
        let crossing = sheet.crossing.clone();
        // (the object may not be what this was opened on any more: another map, or an undo that
        // put the original back, which then gets the copy again)
        let now = ed.doc.as_ref().filter(|d| d.dir == self.map_dir).and_then(|d| d.object(k, id)).map(|o| o.file.clone());
        let on_ours = now.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&self.rel));
        let on_original = now.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&self.from_rel));
        if !on_ours && !on_original {
            self.message = Some((tr("The map has changed since this was opened: open the object again", "Карта изменилась с момента открытия: откройте объект заново").into(), true));
            return;
        }
        if crossing.helpers.iter().any(|h| h.spline.trim().is_empty()) {
            self.message = Some((tr("A spline helper has no road type: pick one or remove it", "У точки привязки нет типа дороги: выберите его или удалите точку").into(), true));
            return;
        }
        let text = sheet.text();
        // (in the object's own file: the map's object keeps naming the file it names, and the
        // file as it stood is kept as its `.bak`)
        if self.save_mode == SaveMode::InPlace {
            let abs = self.abs.clone();
            match crossing_edit::write_in_place(&abs, &text, &self.bytes, &crossing) {
                Ok(false) => {
                    self.seen = crossing;
                    self.message = Some((tr("Saved", "Сохранено").into(), false));
                    return;
                }
                Ok(true) => {
                    crossing_edit::baks::note(roots, &crossing_edit::bak_path(&abs));
                    self.copied = true;
                    if let Ok(b) = std::fs::read(&abs) {
                        self.bytes = b;
                    }
                    self.seen = crossing;
                    crate::hot::own_change();
                    let key = self.rel.to_lowercase();
                    assets.objects.remove(&key);
                    let _ = assets.object(&self.rel);
                    self.message = Some((tr("Saved", "Сохранено").into(), false));
                    return;
                }
                Err(e) => {
                    self.message = Some((format!("{e} {}", tr("Save the crossing as a new file instead", "Сохраните перекрёсток как новый файл")), true));
                    return;
                }
            }
        }
        let fresh = !self.copied;
        let (rel, abs) = if self.copied {
            (self.rel.clone(), self.abs.clone())
        } else {
            let map = ed.doc.as_ref().map(crate::junction::folder_name).unwrap_or_else(|| "map".into());
            let stem = std::path::Path::new(&self.rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "crossing".into());
            match crossing_edit::copy_into(roots, &self.rel, &map, &stem) {
                Ok(r) => {
                    let a = omsi_cfg::resolve_path(&roots.content, &r);
                    (r, a)
                }
                Err(e) => {
                    self.message = Some((e.to_string(), true));
                    return;
                }
            }
        };
        if let Err(e) = crossing_edit::write_checked(&abs, &text, &self.bytes, &crossing) {
            if fresh {
                crossing_edit::discard_copy(roots, &rel);
            }
            self.message = Some((e.to_string(), true));
            return;
        }
        // (the mark that this file is the editor's own: the next save writes it in place)
        let _ = std::fs::write(sidecar(&abs), format!("{{\"from\": {:?}}}", self.from_rel));
        self.copied = true;
        self.rel = rel.clone();
        self.abs = abs.clone();
        if let Ok(b) = std::fs::read(&abs) {
            self.bytes = b;
        }
        self.seen = crossing;
        crate::hot::own_change();
        assets.objects.remove(&rel.to_lowercase());
        let _ = assets.object(&rel);
        if fresh || !on_ours {
            ed.set_object_file(k, id, &rel);
        }
        self.message = Some((tr("Saved", "Сохранено").into(), false));
    }

    // ------------------------------------------------------------ the view

    fn fit_to(&mut self, size: Vec2, sheet: &Sheet) {
        let mut lo = DVec2::splat(f64::MAX);
        let mut hi = DVec2::splat(f64::MIN);
        for p in &sheet.crossing.paths {
            for q in p.points(1.0) {
                lo = lo.min(q.truncate());
                hi = hi.max(q.truncate());
            }
        }
        for h in &sheet.crossing.helpers {
            lo = lo.min(h.local().truncate());
            hi = hi.max(h.local().truncate());
        }
        // (the object itself: a crossing whose roads run far out is framed whole)
        if let Some((_, _, b)) = &self.footprint {
            if b[2] > b[0] && b[3] > b[1] {
                lo = lo.min(DVec2::new(b[0], b[1]));
                hi = hi.max(DVec2::new(b[2], b[3]));
            }
        }
        // (and the nearby ends a new path can be started from, when that mode is on)
        if self.wants_nearby() {
            for j in &self.nearby {
                lo = lo.min(j.pos.truncate());
                hi = hi.max(j.pos.truncate());
            }
        }
        if lo.x > hi.x {
            lo = DVec2::splat(-15.0);
            hi = DVec2::splat(15.0);
        }
        let centre = (lo + hi) / 2.0;
        let span = (hi - lo).max(DVec2::splat(12.0)) * 1.25;
        let s = (size.x as f64 / span.x).min(size.y as f64 / span.y);
        self.centre = centre;
        self.scale = (s as f32).clamp(0.6, 400.0);
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet) {
        let size = Vec2::new(ui.available_width().max(320.0), ui.available_height().max(320.0));
        #[cfg(test)]
        {
            self.last_canvas = size;
        }
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if self.fit {
            self.fit = false;
            self.fit_to(rect.size(), sheet);
        }
        #[cfg(test)]
        {
            self.last_canvas_rect = Some(rect);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let pinch = ui.input(|i| i.zoom_delta());
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            let factor = (1.0 + scroll * 0.0025).max(0.2) * pinch;
            if factor != 1.0 {
                if let Some(h) = resp.hover_pos() {
                    let (scale, centre) = (self.scale, self.centre);
                    let before = DVec2::new(centre.x + (h.x - rect.center().x) as f64 / scale as f64, centre.y - (h.y - rect.center().y) as f64 / scale as f64);
                    self.scale = (self.scale * factor).clamp(0.6, 400.0);
                    let after = DVec2::new(centre.x + (h.x - rect.center().x) as f64 / self.scale as f64, centre.y - (h.y - rect.center().y) as f64 / self.scale as f64);
                    self.centre += before - after;
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            self.centre -= DVec2::new(d.x as f64, -d.y as f64) / self.scale as f64;
        }
        let (scale, centre) = (self.scale, self.centre);
        let to_screen = move |p: DVec2| Pos2::new(rect.center().x + ((p.x - centre.x) * scale as f64) as f32, rect.center().y - ((p.y - centre.y) * scale as f64) as f32);
        let from_screen = move |q: Pos2| DVec2::new(centre.x + (q.x - rect.center().x) as f64 / scale as f64, centre.y - (q.y - rect.center().y) as f64 / scale as f64);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, Color32::from_rgb(30, 34, 31));
        // the object itself, so the crossing is seen under the paths
        {
            if let Some((_, tris, _)) = &self.footprint {
                let mut mesh = egui::Mesh::default();
                for t in tris.chunks_exact(9) {
                    let base = mesh.vertices.len() as u32;
                    for v in 0..3 {
                        let o = v * 3;
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: to_screen(DVec2::new(t[o] as f64, t[o + 1] as f64)),
                            uv: egui::epaint::WHITE_UV,
                            color: Color32::from_rgba_unmultiplied(190, 196, 190, 20),
                        });
                    }
                    mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
                }
                if !mesh.vertices.is_empty() {
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        }
        // the grid: metres close, tens away
        {
            let step = if scale >= 14.0 { 1.0 } else if scale >= 4.0 { 5.0 } else if scale >= 1.2 { 10.0 } else { 50.0 };
            let (lo, hi) = (from_screen(rect.left_bottom()), from_screen(rect.right_top()));
            let line = |a: DVec2, b: DVec2, zero: bool, major: bool| painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(if zero { 1.5 } else { 1.0 }, Color32::from_white_alpha(if zero { 70 } else if major { 26 } else { 12 })));
            let mut x = (lo.x / step).floor() * step;
            while x <= hi.x {
                line(DVec2::new(x, lo.y), DVec2::new(x, hi.y), x == 0.0, (x / (step * 5.0)).fract().abs() < 1e-6);
                x += step;
            }
            let mut y = (lo.y / step).floor() * step;
            while y <= hi.y {
                line(DVec2::new(lo.x, y), DVec2::new(hi.x, y), y == 0.0, (y / (step * 5.0)).fract().abs() < 1e-6);
                y += step;
            }
        }
        // the object's own frame: x to the right, y (forward) up the page
        painter.line_segment([to_screen(DVec2::new(-3.0, 0.0)), to_screen(DVec2::new(3.0, 0.0))], Stroke::new(1.4, Color32::from_rgb(150, 90, 90)));
        painter.line_segment([to_screen(DVec2::new(0.0, -3.0)), to_screen(DVec2::new(0.0, 3.0))], Stroke::new(1.4, Color32::from_rgb(90, 150, 90)));

        // the paths
        for (i, p) in sheet.crossing.paths.iter().enumerate() {
            let on = self.selected == Some(i);
            let base = if on { PICKED } else { kind_color(p.kind()) };
            let pts = p.points(0.5);
            // its edges, so the width is seen
            let half = (p.width() / 2.0).max(0.0);
            let mut left = Vec::with_capacity(pts.len());
            let mut right = Vec::with_capacity(pts.len());
            for (n, q) in pts.iter().enumerate() {
                let t = if n + 1 < pts.len() { pts[n + 1] - *q } else if n > 0 { *q - pts[n - 1] } else { DVec3::Y };
                let t = t.truncate();
                let len = t.length();
                let tangent = if len > 1e-9 { t / len } else { DVec2::Y };
                let normal = DVec2::new(tangent.y, -tangent.x);
                left.push(q.truncate() + normal * half);
                right.push(q.truncate() - normal * half);
            }
            let edge = Stroke::new(0.8, base.gamma_multiply(0.5));
            for w in left.windows(2) {
                painter.line_segment([to_screen(w[0]), to_screen(w[1])], edge);
            }
            for w in right.windows(2) {
                painter.line_segment([to_screen(w[0]), to_screen(w[1])], edge);
            }
            for w in pts.windows(2) {
                painter.line_segment([to_screen(w[0].truncate()), to_screen(w[1].truncate())], Stroke::new(if on { 1.7 } else { 1.1 }, base));
            }
            // the way it goes
            let dir_arrow = |at: DVec2, along: DVec2, color: Color32| {
                let len = along.length();
                if len < 1e-6 {
                    return;
                }
                let d = along / len;
                let n = DVec2::new(-d.y, d.x);
                let tip = at + d * (7.0 / scale as f64);
                painter.add(egui::Shape::convex_polygon(vec![to_screen(tip), to_screen(at + n * (2.8 / scale as f64)), to_screen(at - n * (2.8 / scale as f64))], color, Stroke::NONE));
            };
            // (the way it goes, all along the path: one arrow at the middle is lost on a
            // long path, and a path of a few metres would show none at all)
            {
                let spacing = (46.0 / scale as f64).clamp(1.2, 25.0);
                let (along, against) = path_ways(p.direction());
                let mut acc = spacing * 0.5;
                let mut shown = 0;
                for w in pts.windows(2) {
                    let seg = (w[1] - w[0]).truncate().length();
                    if seg < 1e-9 {
                        continue;
                    }
                    acc += seg;
                    if acc < spacing {
                        continue;
                    }
                    acc = 0.0;
                    let mid = (w[0].truncate() + w[1].truncate()) * 0.5;
                    let d = (w[1] - w[0]).truncate() / seg;
                    if along {
                        dir_arrow(mid, d, base);
                        shown += 1;
                    }
                    if against {
                        dir_arrow(mid, -d, base);
                        shown += 1;
                    }
                }
                if shown == 0 {
                    // (a path shorter than the spacing still says which way it goes)
                    if let (Some(a), Some(b)) = (pts.first(), pts.last()) {
                        let mid = (a.truncate() + b.truncate()) * 0.5;
                        let d = b.truncate() - a.truncate();
                        if along {
                            dir_arrow(mid, d, base);
                        }
                        if against {
                            dir_arrow(mid, -d, base);
                        }
                    }
                }
            }
            // its ends
            for at in [p.start().truncate(), p.end().truncate()] {
                let s = to_screen(at);
                painter.circle_filled(s, if on { 3.2 } else { 2.4 }, if on { HANDLE } else { base });
                painter.circle_stroke(s, 4.8, Stroke::new(0.8, Color32::from_white_alpha(if on { 110 } else { 34 })));
            }
        }
        // the spline helpers: where roads snap on
        for h in &sheet.crossing.helpers {
            let at = to_screen(h.local().truncate());
            painter.circle_stroke(at, 4.4, Stroke::new(1.3, SNAPPED));
            let d = crate::roads::dir(h.heading as f64);
            painter.line_segment([at, to_screen(h.local().truncate() + d * 4.0)], Stroke::new(1.2, SNAPPED));
        }
        // the end being dragged is locked onto another end: shown, so a snap is not a surprise
        if let Some(at) = self.snap_mark {
            painter.circle_stroke(to_screen(at.truncate()), 9.0, Stroke::new(1.6, SNAPPED));
        }
        // a new path is started from one of these: the ends of the paths round the crossing,
        // each with the way it runs away from it
        if self.wants_nearby() {
            for j in &self.nearby {
                let s = to_screen(j.pos.truncate());
                if !rect.contains(s) {
                    continue;
                }
                painter.circle_stroke(s, 5.4, Stroke::new(1.4, SNAPPED));
                let d = crate::roads::dir(j.heading);
                painter.line_segment([s, to_screen(j.pos.truncate() + d * 3.0)], Stroke::new(1.2, SNAPPED));
            }
        }

        // ---- the mouse
        let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let at = |pos: Pos2| -> DVec2 { from_screen(pos) };
        let handle = |sheet: &Sheet, pos: Pos2| -> Option<(usize, bool)> {
            let mut best: Option<(f32, usize, bool)> = None;
            for (i, p) in sheet.crossing.paths.iter().enumerate() {
                for (at_end, q) in [(true, p.end()), (false, p.start())] {
                    let d = to_screen(q.truncate()).distance(pos);
                    if d <= GRAB && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, i, at_end));
                    }
                }
            }
            best.map(|b| (b.1, b.2))
        };
        let path_at = |sheet: &Sheet, pos: Pos2| -> Option<usize> {
            let mut best: Option<(f32, usize)> = None;
            for (i, p) in sheet.crossing.paths.iter().enumerate() {
                for q in p.points(0.5) {
                    let d = to_screen(q.truncate()).distance(pos);
                    if d <= GRAB && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, i));
                    }
                }
            }
            best.map(|b| b.1)
        };
        // (a new path started at a nearby path's end - or the picked one carried on to it: the
        // press itself takes the one nearest the pointer, before anything of the crossing's
        // own paths is looked at - a green circle on top of an end would otherwise be a drag
        // of that end)
        if self.wants_nearby() && resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
            if let Some(o) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                let pick = self
                    .nearby
                    .iter()
                    .copied()
                    .map(|j| (to_screen(j.pos.truncate()).distance(o), j))
                    .filter(|(d, _)| *d <= GRAB + 6.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, j)) = pick {
                    self.take_nearby(j);
                    return;
                }
            }
        }
        // (a new path is laid from where the mouse went down, as a road is laid with the
        // spline tool - taken on the press itself, so the drag starts where it began. On an
        // existing end it starts there, so a path can be carried on out of another)
        if self.draw && resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
            if let Some(o) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                let mut from = at(o);
                let mut z = sheet.crossing.paths.get(self.selected.unwrap_or(0)).map(|p| p.start().z).unwrap_or(0.0);
                self.draw_join = None;
                if let Some(j) = snap_joint(&sheet.crossing, usize::MAX, from, snap_reach(self.scale), ctrl) {
                    from = j.pos.truncate();
                    z = j.pos.z;
                    self.snap_mark = Some(j.pos);
                    self.draw_join = Some(j);
                }
                self.draw_from = Some(from);
                self.draw_from_z = z as f32;
                self.push_undo();
            }
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                // (drawing: the press laid a new path's start, so no path of the crossing is
                // taken and dragged)
                self.drag = if self.draw_from.is_some() { None } else { handle(sheet, pos) };
                if self.drag.is_some() {
                    self.snap_mark = None;
                    self.push_undo();
                }
                if self.drag.is_none() && !self.draw {
                    self.selected = path_at(sheet, pos).or(self.selected);
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let (Some((i, at_end)), Some(pos)) = (self.drag, resp.interact_pointer_pos()) {
                if i < sheet.crossing.paths.len() {
                    let q = at(pos);
                    // (snapping is what the editor does: Ctrl keeps the end where the mouse is.
                    // A snapped end takes the other path's way and rise too, not just its point)
                    let joint = snap_joint(&sheet.crossing, i, q, snap_reach(self.scale), ctrl);
                    self.snap_mark = joint.map(|j| j.pos);
                    match joint {
                        Some(j) if at_end => {
                            // (a snapped end: pieces from where the path set out, arriving in
                            // the other path's way - not the path turned round onto the point)
                            if self.drag_anchor.as_ref().is_none_or(|a| a.root != i) {
                                if let Some(p) = sheet.crossing.paths.get(i) {
                                    self.drag_anchor = Some(ReachAnchor { root: i, start: p.start().truncate(), heading: p.heading(), z: p.start().z, slope: p.slope_start(), spawned: 0 });
                                }
                            }
                            if let Some(mut anchor) = self.drag_anchor.clone() {
                                let tris = std::sync::Arc::clone(&self.surface);
                                if let Some(last) = apply_reach(&mut sheet.crossing, &mut anchor, j.pos.truncate(), &tris, Some(j)) {
                                    self.drag_anchor = Some(anchor);
                                    self.selected = Some(last);
                                }
                            }
                        }
                        Some(j) => join_start(&mut sheet.crossing, i, j),
                        None if at_end => {
                            if self.drag_anchor.as_ref().is_none_or(|a| a.root != i) {
                                if let Some(p) = sheet.crossing.paths.get(i) {
                                    self.drag_anchor = Some(ReachAnchor { root: i, start: p.start().truncate(), heading: p.heading(), z: p.start().z, slope: p.slope_start(), spawned: 0 });
                                }
                            }
                            if let Some(mut anchor) = self.drag_anchor.clone() {
                                let tris = std::sync::Arc::clone(&self.surface);
                                if let Some(last) = apply_reach(&mut sheet.crossing, &mut anchor, q, &tris, None) {
                                    self.drag_anchor = Some(anchor);
                                    self.selected = Some(last);
                                }
                            }
                        }
                        None => move_end(&mut sheet.crossing, i, at_end, q),
                    }
                }
            }
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag = None;
            self.drag_anchor = None;
            self.snap_mark = None;
        }
        // (let go: the path laid is the crossing's, and it is the one picked)
        if self.draw_from.is_some() && ui.input(|i| i.pointer.any_released()) {
            let release = ui.input(|i| i.pointer.latest_pos()).or(resp.interact_pointer_pos());
            let from_join = self.draw_join.take();
            if let Some(from) = self.draw_from.take() {
                if let Some(pos) = release {
                    let mut q = at(pos);
                    let end_joint = snap_joint(&sheet.crossing, usize::MAX, q, snap_reach(self.scale), ctrl);
                    if let Some(j) = end_joint {
                        q = j.pos.truncate();
                    }
                    if (q - from).length() > 0.25 {
                        let z = self.draw_from_z;
                        let mut p = Path::default();
                        p.set_start(DVec3::new(from.x, from.y, z as f64));
                        p.set_end(DVec3::new(q.x, q.y, z as f64));
                        // (laid out of another path's end: it leaves in that path's way, and
                        // at its rise; laid onto one: it arrives in its way as well)
                        if let Some(j) = from_join {
                            let legs = match end_joint {
                                Some(e) => connect_segs(from, j.heading, q, e.heading),
                                None => forward_segs(from, j.heading, q),
                            };
                            let end = match end_joint {
                                Some(e) => PieceEnd::At { z: e.pos.z, slope: e.slope },
                                None => PieceEnd::Surface,
                            };
                            let a = DVec3::new(from.x, from.y, j.pos.z);
                            // (driven the way the path it was laid out of is driven)
                            for mut piece in lay_join(&p, a, j.slope, &legs, end, false, &self.surface) {
                                piece.v[10] = j.flow as f32;
                                sheet.crossing.paths.push(piece);
                            }
                            self.selected = Some(sheet.crossing.paths.len() - 1);
                            self.reveal = true;
                            return;
                        }
                        if let Some(j) = end_joint {
                            p.fit_end_tangent(j.heading);
                            p.set_end_height_and_slope(j.pos.z, j.slope);
                        } else {
                            // (a path drawn on the crossing sits on the deck, one grade)
                            if let Some(z) = surface_z(&self.surface, p.start().truncate()) {
                                p.v[2] = z as f32;
                            }
                            if let Some(z) = surface_z(&self.surface, p.end().truncate()) {
                                set_end_height(&mut p, z);
                            } else {
                                let z = p.end().z;
                                set_end_height(&mut p, z);
                            }
                        }
                        sheet.crossing.paths.push(p);
                        self.selected = Some(sheet.crossing.paths.len() - 1);
                        self.reveal = true;
                    }
                }
            }
        }
        if resp.clicked() && self.drag.is_none() && !self.draw {
            if let Some(pos) = resp.interact_pointer_pos() {
                if self.wants_nearby() {
                    // (a nearby path's end picked: a new path starts there, or the picked one
                    // is carried on to it)
                    let hit = self
                        .nearby
                        .iter()
                        .copied()
                        .filter(|j| to_screen(j.pos.truncate()).distance(pos) <= GRAB + 6.0)
                        .min_by(|a, b| to_screen(a.pos.truncate()).distance(pos).total_cmp(&to_screen(b.pos.truncate()).distance(pos)));
                    if let Some(j) = hit {
                        self.take_nearby(j);
                    }
                } else {
                    self.selected = path_at(sheet, pos);
                }
            }
        }
        if resp.hovered() {
            if let Some(pos) = resp.hover_pos() {
                if handle(sheet, pos).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                } else if self.draw || self.wants_nearby() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            }
        }
        // the path being laid: from where it was pressed to where the mouse is
        if let Some(from) = self.draw_from {
            if let Some(pos) = resp.hover_pos().or(resp.interact_pointer_pos()) {
                let to = at(pos);
                let half = (Path::default().width() / 2.0) as f32 * scale;
                let d = to - from;
                let n = if d.length() > 1e-9 { DVec2::new(d.y, -d.x).normalize() * half as f64 } else { DVec2::new(0.0, half as f64) };
                let quad = vec![to_screen(from + n), to_screen(to + n), to_screen(to - n), to_screen(from - n)];
                painter.add(egui::Shape::convex_polygon(quad, PICKED.gamma_multiply(0.25), Stroke::new(1.2, PICKED)));
                painter.circle_filled(to_screen(from), 3.5, PICKED);
                painter.circle_filled(to_screen(to), 3.5, PICKED);
            }
        }
        // the picked path's ends, again, over everything: these are what is dragged
        if let Some(i) = self.selected.filter(|i| *i < sheet.crossing.paths.len()) {
            let p = &sheet.crossing.paths[i];
            for at in [p.start().truncate(), p.end().truncate()] {
                painter.circle_stroke(to_screen(at), 9.0, Stroke::new(1.6, PICKED));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crossing with a few paths, so the left side has a list to lay out.
    const SCO: &str = "\
[friendlyname]
Kreuzung

[mesh]
Kreuzung.o3d

[path]
-6.5
-12
0.1
90
0
12
0
0
0
3.5
0
3

[path]
6.5
-12
0.1
90
0
12
0
0
0
3.5
0
2

[path]
0
0
0
0
0
10
0
0
0
3.5
2
0

[use_traffic_light]
0

[traffic_lights_group]
60

[traffic_light]
Main
[phase]
0
30
[phase]
6
25
";

    /// The window gives the properties the left third and the drawing the rest: a scroll area
    /// takes the layout it is made in, so laid straight into the row it would put the whole
    /// path list sideways in one line and push the drawing out of the window (which is what
    /// the window looked like: one row of paths and nothing else).
    #[test]
    fn the_window_puts_the_drawing_beside_the_properties() {
        let mut w = opened(Tab::Paths);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let canvas = w.last_canvas;
        assert!(canvas.x > 400.0, "the drawing got {canvas:?}: the properties are laid out beside it");
        assert!(canvas.y > 300.0, "the drawing got {canvas:?}");
        // and the map is shown the paths as the window has them
        let draft = ed.crossing_draft.as_ref().expect("the map shows the draft");
        assert_eq!(draft.2.len(), 3);
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(icons::fonts());
        ctx
    }

    /// With nothing open the window still shows the picker, whole: a window collapsed to its
    /// title bar would be a blank grey strip.
    #[test]
    fn the_window_asks_for_a_crossing_when_none_is_open() {
        let mut w = CrossingPaths { open: true, ..Default::default() };
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let rect = w.last_window.expect("the window is drawn");
        assert!(rect.height() > 200.0, "the window's body is collapsed: {rect:?}");
        assert!(rect.width() > 400.0, "the window is {rect:?}");
    }

    fn opened(tab: Tab) -> CrossingPaths {
        let mut w = CrossingPaths { open: true, ..Default::default() };
        w.target = Some(Sel::Object((0, 0), 1));
        w.rel = "Sceneryobjects\\Test\\Kreuzung.sco".into();
        w.selected = Some(0);
        w.sheet = Some(crate::crossing_edit::Sheet::parse(SCO));
        // (as `open_on` leaves it: the state the first step goes back to, and nothing unsaved)
        w.seen = w.sheet.as_ref().unwrap().crossing.clone();
        w.mark = w.seen.clone();
        w.tab = tab;
        w
    }

    fn run(ctx: &egui::Context, w: &mut CrossingPaths, ed: &mut Editor, events: Vec<egui::Event>, time: f64) {
        let lib = Library::default();
        let mut actions = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            w.window(&ctx, ed, &lib, None, None, &mut actions);
        });
    }

    fn press(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() }
    }

    /// The light page gives the timeline the room beside the program, and draws the phases
    /// the file has: red for 30 s, then green to the end of the cycle.
    #[test]
    fn the_lights_page_lays_the_timeline_beside_the_program() {
        let mut w = opened(Tab::Lights);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let rect = w.last_timeline.expect("the timeline is drawn");
        assert!(rect.width() > 300.0, "the timeline got {rect:?}");
        assert!(rect.left() > 330.0, "the program keeps the left side: {rect:?}");
        assert!(rect.height() >= HEAD + ROW_H, "a row per light: {rect:?}");
        // the crossing is picked when the page is opened, and its first phase with it
        let sheet = w.sheet.as_ref().unwrap();
        assert_eq!(sheet.crossing.lights.len(), 1);
        assert_eq!(w.light, Some(0));
        assert_eq!(w.phase, Some(0));
        // the cycle's 60 s are on the timeline
        assert!(rect.width() >= GUTTER + 60.0 * w.sec_px - 1.0, "{rect:?}");
    }

    /// A longer cycle still shows end to end: the timeline takes the window's width on its own,
    /// as the traffic light window's does.
    #[test]
    fn a_longer_cycle_still_fits_the_width() {
        let mut w = opened(Tab::Lights);
        let mut ed = Editor::default();
        let ctx = context();
        {
            let c = &mut w.sheet.as_mut().unwrap().crossing;
            c.cycle = Some(240.0);
            for p in &mut c.lights[0].phases {
                p.duration *= 4.0;
            }
        }
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let rect = w.last_timeline.expect("the timeline is drawn");
        let cycle = w.sheet.as_ref().unwrap().crossing.cycle_len();
        assert!(cycle >= 230.0, "the cycle grew to {cycle} s");
        assert!(
            GUTTER + cycle * w.sec_px <= rect.width() + 1.0,
            "{cycle} s at {} px/s does not fit {rect:?}",
            w.sec_px
        );
    }

    /// Beside the map the window is a narrow panel at the screen's right edge, so the crossing
    /// itself - the object in 3D - stays in view.
    #[test]
    fn the_panel_beside_the_map_leaves_the_crossing_seen() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let rect = w.last_window.expect("the window is laid out");
        let screen = ctx.content_rect();
        assert!(rect.width() <= 460.0, "the panel is narrow, the map keeps the rest: {}", rect.width());
        assert!((rect.right() - screen.right()).abs() < 24.0, "the panel is at the screen's right edge: {rect:?} in {screen:?}");
        assert!(w.on_map(), "the paths are on the map");
        assert!(ed.crossing_draft.is_some(), "the map is shown the paths");
    }

    /// The end can be typed as well as the start: x and y move it, and a height sets the
    /// slopes the file carries it with.
    #[test]
    fn the_end_can_be_typed() {
        let mut p = Path::default();
        p.set_start(DVec3::new(1.0, 2.0, 0.5));
        p.set_end(DVec3::new(11.0, 2.0, 0.5));
        // x and y move the end (and turn the path, as dragging it does)
        let mut q = p.clone();
        q.set_end(DVec3::new(6.0, 9.0, q.end().z));
        assert!((q.end().x - 6.0).abs() < 1e-3 && (q.end().y - 9.0).abs() < 1e-3, "{:?}", q.end());
        // a height is the slopes' doing
        set_end_height(&mut p, 2.5);
        assert!((p.end().z - 2.5).abs() < 1e-3, "the end sits at the height typed: {:?}", p.end());
        assert!((p.slope_start() - p.slope_end()).abs() < 1e-6, "both slopes carry it");
    }

    /// Shown on the map, the panel puts the picked path's numbers above the list: they are
    /// what is edited there, and a list of ten paths would otherwise push them out of sight.
    #[test]
    fn the_3d_panel_puts_the_numbers_above_the_list() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let numbers = w.last_numbers_y.expect("the numbers of the picked path are drawn");
        let list = w.last_list_y.expect("the list is drawn");
        assert!(numbers < list, "the numbers come first in 3D: {numbers} vs {list}");

        // (and the 2D window keeps the list first, as the crossing editor's own page has it)
        let mut w = opened(Tab::Paths);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let numbers = w.last_numbers_y.expect("the numbers are drawn");
        let list = w.last_list_y.expect("the list is drawn");
        assert!(list < numbers, "the list comes first in 2D: {list} vs {numbers}");
    }

    /// Only an object with `[path]`s is offered as a crossing: the catalogue puts every signal
    /// head and light program in Junctions too, and a list of those is no list of crossings.
    #[test]
    fn only_objects_with_paths_are_offered() {
        use crate::library::{Asset, Category, Kind};
        let mk = |rel: &str, category: Category, paths: usize| Asset {
            kind: Kind::Object,
            rel: rel.to_string(),
            name: rel.to_string(),
            dir: String::new(),
            category,
            haystack: String::new(),
            from_content: false,
            is_bus_stop: false,
            is_entry_point: false,
            is_car_park: false,
            abs_height: false,
            lamp: false,
            light_head: false,
            light_program: false,
            helpers: Vec::new(),
            paths,
            half_width: 0.0,
            lanes: (0, 0, 0),
        };
        let mut lib = Library::default();
        lib.assets.push(mk("Traffic_Light\\Head.sco", Category::Junctions, 0));
        lib.assets.push(mk("Crossings\\Kreuzung.sco", Category::Junctions, 9));
        lib.assets.push(mk("Sceneryobjects\\openMapper\\M\\junction_1.sco", Category::Objects, 0));
        lib.reindex_for_tests();
        assert!(!is_crossing(&lib, "Traffic_Light\\Head.sco"), "a signal head has no path to edit");
        assert!(is_crossing(&lib, "Crossings\\Kreuzung.sco"), "a crossing with paths is offered");
        assert!(is_crossing(&lib, "Sceneryobjects\\openMapper\\M\\junction_1.sco"), "openMapper's own junction");
        assert!(!is_crossing(&lib, "not\\catalogue\\thing.sco"), "an unread object is not offered");
    }

    /// Closing with changes in it asks: the window stays until the question is answered, and
    /// "close without saving" leaves nothing behind.
    #[test]
    fn closing_with_changes_asks_first() {
        let mut w = opened(Tab::Paths);
        let mut ed = Editor::default();
        let ctx = context();
        w.sheet.as_mut().unwrap().crossing.paths[0].v[9] = 5.0;
        assert!(w.unsaved());
        let lib = Library::default();
        let mut actions = Vec::new();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), time: Some(0.0), ..Default::default() };
        let _ = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            w.window(&ctx, &mut ed, &lib, None, None, &mut actions);
        });
        // (the close button, then the question)
        w.ask_close = true;
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), time: Some(0.1), ..Default::default() };
        let _ = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            w.window(&ctx, &mut ed, &lib, None, None, &mut actions);
        });
        assert!(w.ask_close, "the question is on screen");
        assert!(w.open, "the window stays until it is answered");
        // (answered "without saving": gone, and the file's crossing as it was)
        w.discard();
        assert!(!w.open);
        assert!(w.sheet.is_none(), "nothing is left to be shown");
        assert!(!w.ask_close, "the question is put away with the window");
    }

    /// "+" carries a path on from the picked one: it starts at that path's end, in the way it
    /// runs there, and is not a copy put beside it.
    #[test]
    fn a_new_path_carries_on_from_the_picked_one() {
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        let picked = crossing.paths[2].clone();
        let end = picked.end();
        let at = continue_path(&mut crossing, Some(2)).expect("a path was added");
        assert_eq!(at, crossing.paths.len() - 1);
        let added = &crossing.paths[at];
        assert!((added.start().truncate() - end.truncate()).length() < 1e-3, "it starts where the picked one ends: {:?} vs {:?}", added.start(), end);
        assert!((added.start().z - end.z).abs() < 1e-3, "at the same height");
        // (the picked path is not touched, and the new one is not a copy of it)
        assert_eq!(crossing.paths[2], picked);
        assert!((added.start().truncate() - picked.start().truncate()).length() > 1.0, "it is not the copy beside it");
        // (straight on, in the way the picked one ends)
        assert!((added.radius() - picked.radius()).abs() < 1e-4, "it keeps the radius, {}", added.radius());
        assert!((added.slope_start() - picked.slope_end()).abs() < 1e-4, "it leaves at the rise the previous one arrives with");
        assert!(added.length() <= 15.0, "a short piece, not a copy of the whole length: {}", added.length());
        let was = picked.curve().heading_at(picked.curve().length);
        let now = added.heading();
        assert!(((was - now).rem_euclid(360.0)).min((now - was).rem_euclid(360.0)) < 1e-2, "the way it runs: {was} vs {now}");
        // (and it is one step back for Ctrl+Z)
        let mut w = opened(Tab::Paths);
        w.selected = Some(1);
        assert!(w.continue_path());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), 4);
        assert!(w.undo());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), 3, "one step back");
    }

    /// A curved path carried on stays curved, in the way and at the rise it arrived, and only
    /// a short piece — not a straight line of the whole previous length.
    #[test]
    fn a_carried_path_keeps_the_curve_and_the_rise() {
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        let p = &mut crossing.paths[2];
        p.v[3] = 20.0;
        p.v[4] = -40.0;
        p.v[5] = 80.0;
        p.v[6] = 0.01;
        p.v[7] = 0.04;
        let picked = crossing.paths[2].clone();
        let end_h = picked.curve().heading_at(picked.length());
        let at = continue_path(&mut crossing, Some(2)).unwrap();
        let q = &crossing.paths[at];
        assert!((q.radius() - (-40.0)).abs() < 1e-3, "radius {}", q.radius());
        assert!((q.length() - 15.0).abs() < 1e-3, "short, {}", q.length());
        assert!((q.slope_start() - 0.04).abs() < 1e-4 && (q.slope_end() - 0.04).abs() < 1e-4);
        let d = (q.heading() - end_h).rem_euclid(360.0).min((end_h - q.heading()).rem_euclid(360.0));
        assert!(d < 0.05, "heading {d}");
        assert!((q.start() - picked.end()).length() < 1e-2);
    }

    /// A join that one arc cannot make is pieces that keep the way out and the way in: the
    /// departure is never turned to make a single piece fit, and each piece begins in the way
    /// the piece before it ended, so no join has a kink.
    #[test]
    fn a_join_keeps_both_ways() {
        let a = DVec2::new(0.0, 10.0);
        let b = DVec2::new(6.0, 16.0);
        // (in line with the way out and the way in: one piece)
        let straight = connect_segs(DVec2::ZERO, 0.0, DVec2::new(0.0, 12.0), 0.0);
        assert_eq!(straight.len(), 1);
        // (off to the side, arriving southward: pieces, the first still leaving north)
        let segs = connect_segs(a, 0.0, b, 180.0);
        assert!(segs.len() >= 2, "one arc cannot arrive south from a northward start: {}", segs.len());
        assert!(ang_delta(segs[0].heading, 0.0).abs() < 0.5, "it leaves the way it did: {}", segs[0].heading);
        assert!((segs[0].start - a).length() < 1e-9);
        let last = segs.last().unwrap();
        assert!((last.end() - b).length() < 1e-6, "it ends at the point: {:?}", last.end());
        assert!(ang_delta(last.end_heading(), 180.0).abs() < 0.5, "arriving in the other's way: {}", last.end_heading());
        assert!(last.length > 0.0);
        at_each_join(&segs, a, b);
    }

    /// A piece begins in the way the one before it ended, the run starts where asked, and the
    /// last piece ends at the point: what leaves each join looking like one line.
    fn at_each_join(segs: &[crate::roads::Seg], start: DVec2, end: DVec2) {
        assert!((segs[0].start - start).length() < 1e-6, "it does not start where asked: {:?}", segs[0].start);
        for w in segs.windows(2) {
            let (before, after) = (&w[0], &w[1]);
            assert!((before.end() - after.start).length() < 1e-6, "a gap at a join: {:?} vs {:?}", before.end(), after.start);
            let d = ang_delta(before.end_heading(), after.heading).abs();
            assert!(d < 0.5, "a kink at a join: {} then {} ({d}°)", before.end_heading(), after.heading);
        }
        assert!((segs.last().unwrap().end() - end).length() < 1e-6, "it does not end at the point: {:?}", segs.last().unwrap().end());
    }

    /// A point off to the side or behind is reached by short forward pieces, not one loop.
    #[test]
    fn a_point_behind_is_several_forward_pieces() {
        let start = DVec2::new(0.0, 0.0);
        let end = DVec2::new(-8.0, -2.0);
        let segs = forward_segs(start, 0.0, end);
        assert!(segs.len() >= 2, "one arc would loop: {} pieces", segs.len());
        assert!(segs.len() <= 6, "too many pieces: {}", segs.len());
        at_each_join(&segs, start, end);
        assert!(ang_delta(segs[0].heading, 0.0).abs() < 0.5, "it leaves the way it did: {}", segs[0].heading);
        for s in &segs {
            let step = s.end() - s.start;
            assert!(step.length() < 40.0, "a piece is a long loop: {}", step.length());
            assert!(crate::roads::dir(s.heading).dot(step) > 0.0, "a piece went backwards: {step:?}");
        }
    }

    #[test]
    fn a_path_says_which_ways_it_goes() {
        assert_eq!(path_ways(0), (true, false), "along it");
        assert_eq!(path_ways(1), (false, true), "against it");
        assert_eq!(path_ways(2), (true, true), "both ways");
        // (what the file's unknown numbers are taken as: the way a new path runs)
        assert_eq!(path_ways(7), (true, false));
    }

    /// A new path laid outward from a join is driven the way the path it joins is driven: into
    /// the new one when the other's traffic arrives there, against it when the other's leaves.
    #[test]
    fn a_new_path_follows_the_way_of_the_one_it_joins() {
        use crate::crossing_edit::Joint;
        // (another path ending here, its traffic running along it into the join)
        assert_eq!(outward_flow(0, true), 0, "traffic arrives: the new path is driven along it");
        // (its traffic running against it, so it leaves from this end)
        assert_eq!(outward_flow(1, true), 1, "traffic leaves: the new path is driven towards the join");
        // (at the other path's start it is the other way round)
        assert_eq!(outward_flow(0, false), 1);
        assert_eq!(outward_flow(1, false), 0);
        // (both ways stays both ways)
        assert_eq!(outward_flow(2, true), 2);
        assert_eq!(outward_flow(2, false), 2);

        let mut w = opened(Tab::Paths);
        w.set_from_nearby(true);
        let j = Joint { pos: DVec3::new(5.0, 5.0, 1.0), heading: 90.0, slope: 0.05, flow: 1 };
        let at = w.start_from_joint(j).expect("started");
        let p = &w.sheet.as_ref().unwrap().crossing.paths[at];
        assert_eq!(p.direction(), 1, "the new path takes the join's direction");
        assert!(ang_delta(p.heading(), 90.0).abs() < 0.5, "and its way out: {}", p.heading());
    }

    /// A join of half a metre is one straight piece: no bend of a usable radius fits, and a
    /// bend squeezed in is a knot that reads as the wrong way.
    #[test]
    fn a_tiny_join_is_one_straight_piece_not_a_knot() {
        let a = DVec2::new(-48.0346, 19.2132);
        let b = DVec2::new(-47.6323, 18.9858);
        assert!((b - a).length() < 0.5);
        let segs = connect_segs(a, 143.5, b, 337.6);
        assert_eq!(segs.len(), 1, "one piece: {}", segs.len());
        let s = &segs[0];
        assert_eq!(s.radius, 0.0, "not a knot: radius {}", s.radius);
        assert!((s.end() - b).length() < 1e-6, "it ends on the point: {:?}", s.end());
        // (what the tiny knot used to be: a bend of 166 degrees in 0.46 m, radius 0.23 m)
        assert!(s.length < 1.0, "{} m", s.length);

        // and a join wide enough for a real bend is still two arcs that meet tangentially
        let wide = connect_segs(DVec2::new(0.0, 10.0), 0.0, DVec2::new(6.0, 16.0), 180.0);
        at_each_join(&wide, DVec2::new(0.0, 10.0), DVec2::new(6.0, 16.0));
        for s in &wide {
            assert!(s.radius == 0.0 || s.radius.abs() >= MIN_PIECE_RADIUS, "a real bend: radius {}", s.radius);
        }
    }

    /// A nearby end, in the object's own frame, is refused when it is not in that frame at all:
    /// a joint left in the map's coordinates is ten million metres away, and a path started
    /// from it lands out there (a radius of millions, a length past the file's limit).
    #[test]
    fn a_joint_from_another_frame_is_not_taken() {
        let mut w = opened(Tab::Paths);
        w.set_from_nearby(true);
        let before = w.sheet.as_ref().unwrap().crossing.paths.len();
        let far = Joint { pos: DVec3::new(-11_748_849.0, -23_623_152.5, 66.8), heading: 90.0, slope: 0.05, flow: 0 };
        assert!(w.start_from_joint(far).is_none(), "a joint ten million metres out is not a join");
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before, "nothing was added");
        assert!(w.wants_nearby(), "and the mode stays on, ready for a real one");

        // (a nearby end in the object's frame is still taken)
        let near = Joint { pos: DVec3::new(5.0, 5.0, 1.0), heading: 90.0, slope: 0.05, flow: 0 };
        assert!(w.start_from_joint(near).is_some());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before + 1);
        // (and a join across the map is refused too, rather than making a path of millions)
        w.set_complete_to(true);
        let p = w.sheet.as_ref().unwrap().crossing.paths[2].end();
        let out = Joint { pos: p + DVec3::new(-2_000_000.0, 0.0, 0.0), heading: 0.0, slope: 0.0, flow: 0 };
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(w.complete_to_joint(out).is_none());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n, "nothing was added");
    }

    /// The deck's height is the top of the mesh at that point.
    #[test]
    fn a_path_takes_the_top_of_the_object() {
        let tris = [
            0.0, 0.0, 1.0, 10.0, 0.0, 1.0, 0.0, 10.0, 1.0, // deck
            0.0, 0.0, 0.2, 10.0, 0.0, 0.2, 0.0, 10.0, 0.2, // underside
        ];
        let z = surface_z(&tris, DVec2::new(2.0, 2.0)).unwrap();
        assert!((z - 1.0).abs() < 1e-6, "{z}");
        assert!(surface_z(&tris, DVec2::new(30.0, 30.0)).is_none());
    }

    /// Even grade sets both slopes to the rise between the two heights.
    #[test]
    fn even_grade_uses_one_slope() {
        let mut p = Path::default();
        p.v[2] = 1.0;
        p.v[5] = 20.0;
        p.v[6] = 0.2;
        p.v[7] = -0.1;
        set_end_height(&mut p, 1.0 + 0.5);
        assert!((p.slope_start() - 0.025).abs() < 1e-4, "{}", p.slope_start());
        assert!((p.slope_end() - p.slope_start()).abs() < 1e-6);
        assert!((p.end().z - 1.5).abs() < 1e-3, "{}", p.end().z);
    }

    /// The start and the end each keep their own angle: turning the arrival does not swing
    /// the departure round to match it, and the end point stays.
    #[test]
    fn the_ends_rotate_apart() {
        let p = Path::default();
        let end = p.end().truncate();
        let parts = retarget(&p, 0.0, 25.0);
        assert!(!parts.is_empty());
        let leave = ang_delta(parts[0].heading(), 0.0).abs();
        assert!(leave < 1.0, "it still leaves north: {}", parts[0].heading());
        let last = parts.last().unwrap();
        let c = last.curve();
        let arrive = c.heading_at(c.length.max(0.0));
        assert!(ang_delta(arrive, 25.0).abs() < 2.0, "it arrives at its own angle: {arrive}");
        assert!(ang_delta(parts[0].heading(), arrive).abs() > 5.0, "the two angles are not one");
        assert!((last.end().truncate() - end).length() < 0.25, "the end stayed: {:?}", last.end());

        let same = retarget(&p, 0.0, 0.0);
        assert_eq!(same.len(), 1, "ahead, one piece");
        assert!(ang_delta(same[0].heading(), 0.0).abs() < 0.5);
    }

    /// Dragging an end keeps the way the path leaves, and the end stays where it was put.
    #[test]
    fn dragging_an_end_keeps_the_way_it_leaves() {
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        let h = crossing.paths[0].heading();
        let z = crossing.paths[0].end().z;
        move_end(&mut crossing, 0, true, DVec2::new(10.0, 0.0));
        let p = &crossing.paths[0];
        assert!((p.end().truncate() - DVec2::new(10.0, 0.0)).length() < 0.05, "end {:?}", p.end());
        let d = (p.heading() - h).rem_euclid(360.0).min((h - p.heading()).rem_euclid(360.0));
        assert!(d < 0.05, "heading moved by {d}");
        assert!((p.end().z - z).abs() < 0.05, "height {}", p.end().z);
    }

    /// The map is held while the crossing editor is open: nothing on it is hovered or picked,
    /// so the pointer cannot even highlight an object or a spline under it.
    #[test]
    fn the_map_is_held_while_the_crossing_is_edited() {
        use crate::selection::tests::{add_object, flat_map};
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        let mut assets = Assets::new(std::env::temp_dir(), std::env::temp_dir());
        let lib = Library::default();
        // (the object is under the ray: hovered and picked with the map free)
        ed.hover_ray(&mut assets, &lib, DVec3::new(100.0, 100.0, 40.0), -DVec3::Z);
        let free = ed.hover;
        ed.crossing_hold = true;
        ed.hover_ray(&mut assets, &lib, DVec3::new(100.0, 100.0, 40.0), -DVec3::Z);
        assert!(ed.hover.is_none(), "nothing is hovered while the map is held ({free:?} with it free)");
        assert!(ed.hover_handle.is_none());
        // (let go, and the map answers again)
        ed.crossing_hold = false;
        ed.hover_ray(&mut assets, &lib, DVec3::new(100.0, 100.0, 40.0), -DVec3::Z);
        assert_eq!(ed.hover, free, "the map answers once the editor is closed");
    }

    /// A new path can be started at the end of a nearby path (another object's, or a road's
    /// lane): it leaves in that path's way, at its rise.
    #[test]
    fn a_path_is_started_from_a_nearby_end() {
        let mut w = opened(Tab::Paths);
        let before = w.sheet.as_ref().unwrap().crossing.paths.len();
        let j = Joint { pos: DVec3::new(5.0, 5.0, 1.0), heading: 90.0, slope: 0.05, flow: 0 };
        // (not while the mode is off)
        assert!(!w.wants_nearby());
        assert!(w.start_from_joint(j).is_none(), "nothing is done while the mode is off");
        w.set_from_nearby(true);
        assert!(w.wants_nearby());
        let at = w.start_from_joint(j).expect("a path was started");
        let p = &w.sheet.as_ref().unwrap().crossing.paths[at];
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before + 1);
        assert!((p.start() - j.pos).length() < 1e-3, "it starts at the nearby end: {:?}", p.start());
        assert!((p.heading() - j.heading).abs() < 1e-3, "in that path's way: {}", p.heading());
        assert!((p.slope_start() - j.slope).abs() < 1e-5, "at its rise: {}", p.slope_start());
        assert!((p.slope_end() - j.slope).abs() < 1e-5, "and it keeps climbing so");
        assert_eq!(w.picked(), Some(at), "the new path is the one picked");
        assert!(!w.wants_nearby(), "the mode is done with once one is made");
        // (one step back for Ctrl+Z)
        assert!(w.undo());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before);
    }

    /// `Complete to`: the picked path is left exactly as it was, and one piece or two join its
    /// end to a nearby end - arriving in that path's way and at its rise.
    #[test]
    fn completing_to_a_nearby_end_keeps_the_picked_path_as_it_was() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        w.selected = Some(2);
        let before = w.sheet.as_ref().unwrap().crossing.paths.clone();
        let p = before[2].clone();
        let c = p.curve();
        let a = p.end();
        let way = c.heading_at(c.length.max(0.0));
        let rise = p.slope_end();
        // (off to the side, in another way: a corner, so two pieces)
        let j = Joint { pos: a + DVec3::new(6.0, 6.0, 0.4), heading: 180.0, slope: 0.03, flow: 0 };
        w.set_complete_to(true);
        assert!(w.completing());
        let last = w.complete_to_joint(j).expect("joined");
        let paths = &w.sheet.as_ref().unwrap().crossing.paths;
        assert!(paths.len() > before.len(), "a piece at least: {}", paths.len());
        assert_eq!(paths[2], before[2], "the picked path is not touched");
        let first = &paths[before.len()];
        assert!((first.start() - a).length() < 1e-2, "it starts where the picked one ends: {:?} vs {a:?}", first.start());
        assert!(((first.heading() - way).rem_euclid(360.0)).min((way - first.heading()).rem_euclid(360.0)) < 0.5, "in the picked path's way: {} vs {way}", first.heading());
        assert!((first.slope_start() - rise).abs() < 1e-5, "and at its rise: {} vs {rise}", first.slope_start());
        let end = &paths[last];
        assert!((end.end().truncate() - j.pos.truncate()).length() < 0.05, "it ends at the other end: {:?} vs {:?}", end.end(), j.pos);
        let ec = end.curve();
        let got = ec.heading_at(ec.length.max(0.0));
        assert!(((got - j.heading).rem_euclid(360.0)).min((j.heading - got).rem_euclid(360.0)) < 0.5, "arriving in its way: {got} vs {}", j.heading);
        assert!((end.end().z - j.pos.z).abs() < 0.02, "at its height: {}", end.end().z);
        assert!((end.slope_end() - j.slope).abs() < 1e-4, "and at its rise: {}", end.slope_end());
        assert!(!w.completing(), "the mode is done with once one is made");
        // (one step back for Ctrl+Z)
        assert!(w.undo());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before.len());
    }

    /// When the ways agree and the other end lies along the picked path's way, a single piece
    /// does it.
    #[test]
    fn a_completion_in_line_is_one_piece() {
        let mut w = opened(Tab::Paths);
        w.selected = Some(2);
        let before = w.sheet.as_ref().unwrap().crossing.paths.len();
        let p = w.sheet.as_ref().unwrap().crossing.paths[2].clone();
        let a = p.end();
        let c = p.curve();
        let way = c.heading_at(c.length.max(0.0));
        // (in the way the path runs there: heading 0 is north, as `roads::dir` has it)
        let d = crate::roads::dir(way);
        let target = a + DVec3::new(d.x, d.y, 0.0) * 8.0;
        w.set_complete_to(true);
        let last = w.complete_to_joint(Joint { pos: target, heading: way, slope: 0.0, flow: 0 }).expect("joined");
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before + 1, "one straight piece");
        assert_eq!(last, before);
    }

    /// A path is laid with the mouse, from where it is pressed to where it is let go, as a
    /// road is drawn with the spline tool.
    #[test]
    fn a_path_is_drawn_with_the_mouse() {
        let mut w = opened(Tab::Paths);
        w.draw = true;
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let before = w.sheet.as_ref().unwrap().crossing.paths.len();
        let rect = w.last_canvas_rect.expect("the drawing is laid out");
        let from = Pos2::new(rect.center().x - 40.0, rect.center().y);
        let to = Pos2::new(rect.center().x + 40.0, rect.center().y);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(from), press(from, true)], 0.2);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(to)], 0.3);
        run(&ctx, &mut w, &mut ed, vec![press(to, false)], 0.4);
        let sheet = w.sheet.as_ref().unwrap();
        assert_eq!(sheet.crossing.paths.len(), before + 1, "a path was added");
        let p = sheet.crossing.paths.last().unwrap();
        let (a, b) = (p.start().truncate(), p.end().truncate());
        assert!((b.x - a.x).abs() > 1.0, "the path runs from the press to the release: {a:?}..{b:?}");
        assert_eq!(w.selected, Some(before), "the new path is the one picked");
    }

    /// Drawing on an existing end starts there: a path can be carried on out of another,
    /// and the path whose end was pressed is not dragged instead.
    #[test]
    fn a_path_is_drawn_out_of_an_existing_end() {
        let mut w = opened(Tab::Paths);
        w.draw = true;
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let sheet = w.sheet.as_ref().unwrap();
        let before = sheet.crossing.paths.len();
        let end = sheet.crossing.paths[2].end();
        let untouched = sheet.crossing.paths[2].clone();
        let rect = w.last_canvas_rect.expect("the drawing is laid out");
        let (scale, centre) = (w.scale, w.centre);
        let to_screen = |p: DVec2| Pos2::new(rect.center().x + ((p.x - centre.x) * scale as f64) as f32, rect.center().y - ((p.y - centre.y) * scale as f64) as f32);
        let from = to_screen(end.truncate());
        let to = Pos2::new(from.x + 60.0, from.y);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(from), press(from, true)], 0.2);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(to)], 0.3);
        run(&ctx, &mut w, &mut ed, vec![press(to, false)], 0.4);
        let sheet = w.sheet.as_ref().unwrap();
        let added = &sheet.crossing.paths[before..];
        assert!(!added.is_empty(), "a path was added");
        assert_eq!(sheet.crossing.paths[2], untouched, "the path pressed on was not dragged");
        let first = &added[0];
        assert!((first.start().truncate() - end.truncate()).length() < 1e-3, "the new path starts at the end: {:?} vs {:?}", first.start(), end);
        assert!((first.start().z - end.z).abs() < 1e-3, "at the end's own height: {} vs {}", first.start().z, end.z);
        // (one path or several, they run on from each other without a kink)
        for pair in added.windows(2) {
            let (was, now) = (&pair[0], &pair[1]);
            assert!((was.end().truncate() - now.start().truncate()).length() < 0.05, "a gap between pieces: {:?} vs {:?}", was.end(), now.start());
            let c = was.curve();
            let arrive = c.heading_at(c.length.max(0.0));
            assert!(ang_delta(arrive, now.heading()).abs() < 0.5, "a kink between pieces: {arrive} then {}", now.heading());
        }
        let want = end.truncate() + DVec2::new(60.0 / w.scale as f64, 0.0);
        let last = added.last().unwrap();
        assert!((last.end().truncate() - want).length() < 0.3, "it ends where the mouse was let go: {:?} vs {want:?}", last.end());
    }

    /// The paths can be worked on the map itself: an end of one is taken there and dragged,
    /// and the crossing keeps what the drag made of it.
    #[test]
    fn an_end_is_dragged_on_the_map() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        assert!(w.on_map(), "the paths are shown on the map");
        assert!(w.take_map_end(Some((1, true))), "the green phase's path is taken");
        assert_eq!(w.map_dragging(), Some((1, true)));
        // (the map puts the cursor at a point of the object's own frame; None: no snapping)
        assert!(w.drag_map_end(DVec3::new(4.0, 6.0, 0.0), None));
        let sheet = w.sheet.as_ref().unwrap();
        let i = w.selected.unwrap();
        let p = &sheet.crossing.paths[i];
        assert!((p.end().x - 4.0).abs() < 0.05, "the end moved: {:?}", p.end());
        w.let_go();
        assert_eq!(w.map_dragging(), None);
        assert!(!w.drag_map_end(DVec3::new(1.0, 1.0, 0.0), None), "nothing is dragged once let go");
    }

    /// An end dragged near another path's end is taken onto it, and takes that path's way and
    /// rise as well - so two pieces meet in more than the point. When one arc cannot do both,
    /// the join is several pieces and the way the path leaves is left as it was.
    #[test]
    fn a_dragged_end_joins_the_way_of_the_other() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        // (the path from the middle of the crossing, carried onto the eastbound path's end)
        let start = w.sheet.as_ref().unwrap().crossing.paths[2].start();
        let leave = w.sheet.as_ref().unwrap().crossing.paths[2].heading();
        assert!(w.take_map_end(Some((2, true))));
        let target = w.sheet.as_ref().unwrap().crossing.paths[0].end();
        let joint = w.joint_at(0, true).expect("the other end is a joint");
        // (near it, not on it: what the mouse gives a few pixels away)
        let near = target + DVec3::new(0.12, -0.08, 0.0);
        assert!(w.drag_map_end(near, Some(joint)));
        let i = w.selected.unwrap();
        let paths = &w.sheet.as_ref().unwrap().crossing.paths;
        assert!(i > 2, "the join is split into pieces, the last of them at {i}");
        // (the way the path left its start is not turned to make the join fit)
        let first = &paths[2];
        assert!((first.start() - start).length() < 1e-3, "the start stays: {:?} vs {:?}", first.start(), start);
        assert!(ang_delta(first.heading(), leave).abs() < 0.5, "and its way out: {} vs {leave}", first.heading());
        // (each piece begins where the one before it ended)
        for k in 3..=i {
            let (prev, now) = (&paths[k - 1], &paths[k]);
            assert!((now.start().truncate() - prev.end().truncate()).length() < 0.05, "piece {k} is not joined to the one before");
        }
        let p = &paths[i];
        assert!((p.end().truncate() - target.truncate()).length() < 1e-3, "the end landed on it: {:?} vs {:?}", p.end(), target);
        // (and it arrives in the other path's way: the tangents meet, no kink)
        let c = p.curve();
        let their = crate::roads::norm_deg(joint.heading);
        let mine = crate::roads::norm_deg(c.heading_at(c.length.max(0.0)));
        assert!(((mine - their).rem_euclid(360.0)).min((their - mine).rem_euclid(360.0)) < 0.5, "the ways meet: {mine} vs {their}");
        assert!((p.end().z - joint.pos.z).abs() < 0.02, "at its height: {}", p.end().z);
        assert!((p.slope_end() - joint.slope).abs() < 1e-4, "and at its rise: {}", p.slope_end());

        // without a joint the end stays where the cursor put it
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        assert!(w.take_map_end(Some((2, true))));
        let far = DVec3::new(12.0, 4.0, 0.0);
        assert!(w.drag_map_end(far, None));
        let i = w.selected.unwrap();
        let p = &w.sheet.as_ref().unwrap().crossing.paths[i];
        assert!((p.end().truncate() - far.truncate()).length() < 0.2, "free drag: {:?}", p.end());
    }

    /// Ctrl being held keeps a drag free in the drawing: the snap is not taken, so the end
    /// stays where the mouse let go.
    #[test]
    fn ctrl_keeps_the_end_where_the_mouse_is() {
        let w = opened(Tab::Paths);
        let crossing = &w.sheet.as_ref().unwrap().crossing;
        let target = crossing.paths[0].end().truncate();
        let near = target + DVec2::new(0.15, 0.0);
        // (a reach well round it: the point is the only thing that decides)
        assert!(snap_joint(crossing, 1, near, 1.0, false).is_some(), "the end is taken onto the other");
        assert!(snap_joint(crossing, 1, near, 1.0, true).is_none(), "Ctrl leaves it where the mouse is");
        // a reach that does not get there takes nothing, Ctrl or not
        assert!(snap_joint(crossing, 1, DVec2::new(target.x, target.y + 20.0), 1.0, false).is_none());
    }

    /// The crossing answers Ctrl+Z and Ctrl+Y on its own steps, and a burst of changes to one
    /// number is a single step.
    #[test]
    fn the_crossing_steps_back_and_forward() {
        let mut w = opened(Tab::Paths);
        let was = w.sheet.as_ref().unwrap().crossing.paths[2].length();
        w.sheet.as_mut().unwrap().crossing.paths[2].v[5] = 25.0;
        // (a change no drag is under way for: recorded as a step)
        w.checkpoint(10.0);
        assert_eq!(w.undo.len(), 1, "one step was kept");
        assert!(w.undo());
        assert!((w.sheet.as_ref().unwrap().crossing.paths[2].length() - was).abs() < 1e-3, "the length is back");
        assert!(w.redo());
        assert!((w.sheet.as_ref().unwrap().crossing.paths[2].length() - 25.0).abs() < 1e-3, "the change is taken again");
        assert!(!w.redo(), "nothing more to take again");

        // a burst: two records within the same half second are one step
        let mut w = opened(Tab::Paths);
        w.sheet.as_mut().unwrap().crossing.paths[2].v[5] = 20.0;
        w.checkpoint(10.0);
        w.sheet.as_mut().unwrap().crossing.paths[2].v[5] = 22.0;
        w.checkpoint(10.2);
        assert_eq!(w.undo.len(), 1, "the burst is one step");
        assert!(w.undo());
        assert!((w.sheet.as_ref().unwrap().crossing.paths[2].length() - 10.0).abs() < 1e-3, "one step goes back to before the burst");
    }

    /// A path is picked on the map itself, and the map is told which one while the window is
    /// open (it draws it picked).
    #[test]
    fn a_path_is_picked_on_the_map() {
        let mut w = opened(Tab::Paths);
        assert!(!w.select_on_map(Some(1)), "the 2D page picks nothing on the map");
        w.view_3d = true;
        assert!(w.select_on_map(Some(1)));
        assert_eq!(w.picked(), Some(1));
        assert!(!w.select_on_map(Some(9)), "a path that is not there is not picked");
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        assert_eq!(ed.crossing_selected, Some(1), "the map is told which path is picked");
    }

    /// Saving writes the object's own file unless the copy is asked for.
    #[test]
    fn the_ordinary_save_is_into_the_objects_own_file() {
        let mut w = opened(Tab::Paths);
        assert!(w.save_mode == SaveMode::InPlace, "the object's own file is where a save goes");
        w.save_mode = SaveMode::Copy;
        assert!(!w.unsaved(), "nothing changed yet");
    }

    /// Clicking a block picks its light and its phase, as the numbers on the left edit them.
    #[test]
    fn clicking_a_block_picks_that_phase() {
        let mut w = opened(Tab::Lights);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let rect = w.last_timeline.expect("the timeline is drawn");
        // (the second phase runs from 30 s to 55 s)
        let at = Pos2::new(rect.left() + GUTTER + 40.0 * w.sec_px, rect.top() + HEAD + ROW_H * 0.5);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(at), press(at, true)], 0.2);
        run(&ctx, &mut w, &mut ed, vec![press(at, false)], 0.3);
        assert_eq!(w.light, Some(0));
        assert_eq!(w.phase, Some(1), "the block at 40 s is the green phase");
        // and the phase of the first block is picked by clicking further left
        let at = Pos2::new(rect.left() + GUTTER + 10.0 * w.sec_px, rect.top() + HEAD + ROW_H * 0.5);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(at), press(at, true)], 0.4);
        run(&ctx, &mut w, &mut ed, vec![press(at, false)], 0.5);
        assert_eq!(w.phase, Some(0), "the block at 10 s is the red phase");
    }

    /// Dragging the right edge of a block changes that phase's duration, and the ones after it
    /// move along; the file is what it says afterwards.
    #[test]
    fn dragging_a_blocks_edge_changes_its_duration() {
        let mut w = opened(Tab::Lights);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        let rect = w.last_timeline.expect("the timeline is drawn");
        let y = rect.top() + HEAD + ROW_H * 0.5;
        let from = Pos2::new(rect.left() + GUTTER + 30.0 * w.sec_px, y);
        let to = Pos2::new(rect.left() + GUTTER + 20.0 * w.sec_px, y);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(from), press(from, true)], 0.2);
        run(&ctx, &mut w, &mut ed, vec![egui::Event::PointerMoved(to)], 0.3);
        run(&ctx, &mut w, &mut ed, vec![press(to, false)], 0.4);
        let sheet = w.sheet.as_ref().unwrap();
        assert_eq!(sheet.crossing.lights[0].phases[0].duration, 20.0);
        assert_eq!(sheet.crossing.lights[0].phases[1].duration, 25.0, "the green phase keeps its own");
        assert_eq!(w.edge, None, "the drag has ended");
        // the second phase now starts at 20 s, and still holds to the cycle's end
        let segs = sheet.crossing.segments(0);
        assert_eq!(segs[1].start, 20.0);
        assert_eq!(segs[1].end, 60.0);
    }
}
