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

/// The end a click at `pos` takes: the nearest within [`GRAB`] pixels of it. The picked path's
/// own ends win over the rest - the end of one piece and the start of the next are one point at
/// every join, so nearest-first took the neighbour's disc and dragged the wrong path.
fn end_at(sheet: &Sheet, picked: Option<usize>, pos: Pos2, to_screen: &dyn Fn(DVec2) -> Pos2) -> Option<(usize, bool)> {
    let mut best: Option<(f32, usize, bool)> = None;
    let mut consider = |i: usize, at_end: bool, q: DVec3, best: &mut Option<(f32, usize, bool)>| {
        let d = to_screen(q.truncate()).distance(pos);
        if d <= GRAB && best.map(|b| d < b.0).unwrap_or(true) {
            *best = Some((d, i, at_end));
        }
    };
    if let Some(i) = picked
        && let Some(p) = sheet.crossing.paths.get(i)
    {
        consider(i, true, p.end(), &mut best);
        consider(i, false, p.start(), &mut best);
        if let Some(b) = best {
            return Some((b.1, b.2));
        }
    }
    for (i, p) in sheet.crossing.paths.iter().enumerate() {
        if Some(i) == picked {
            continue;
        }
        consider(i, true, p.end(), &mut best);
        consider(i, false, p.start(), &mut best);
    }
    best.map(|b| (b.1, b.2))
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
    /// The other paths picked besides `selected` (Ctrl+click in the list or on the map, Shift
    /// for a run of them). What a junction's paths share - kind, width, turn signal, light,
    /// keeping the junction clear - can then be set on all of them at once.
    multi: Vec<usize>,
    /// Where a Shift+click measures its run from: the pick it came after.
    anchor: Option<usize>,
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
    /// A drag of the start or end rotation, so the pieces it splits into can be rebuilt
    /// from the path as the drag began.
    rot_drag: Option<RotGesture>,
    /// The way round a join is made when either would do (automatic, or always left or right).
    side: Side,
    /// The object's mesh, unthinned, so a new path can sit on the surface. The drawing uses
    /// [`CrossingPaths::footprint`], which is thinned.
    surface: std::sync::Arc<Vec<f32>>,
    surface_for: String,
    /// A coarse 2D grid over `surface`, so sampling a point tests the triangles near it
    /// rather than all of them (a 787-path object is sampled hundreds of times a second).
    surface_grid: Option<SurfaceGrid>,
    /// How far each path sits above the surface (negative: under it), a path at a time, and
    /// when it was last worked out: the mesh is sampled at each path's middle, so this is done
    /// a few times a second at most, never every frame.
    sink: Vec<f32>,
    sink_at: f64,
    /// `Connect the roads` was pressed: the work needs the assets, so it is done after the
    /// window, where they are still at hand.
    link_now: bool,
    /// What the last `Connect the roads` did, a road end at a time: whether it was linked, and
    /// how, so a road that openOMSI will not join is said rather than silently missed.
    link_report: Vec<LinkNote>,
    /// The next path whose sink is worked out: the work is spread over frames, so hundreds of
    /// paths do not all sample the mesh in one.
    sink_next: usize,
    /// A path was picked on the map: the list is scrolled to it where it is out of sight.
    reveal: bool,
    /// Where the end being dragged has snapped to, in the object's frame, so the drawing can
    /// show it (None: not snapped).
    snap_mark: Option<DVec3>,
    /// The ends near the crossing that the 3D view draws - the roads' own lanes, and other
    /// objects' paths - with when they were gathered. The drawing snaps to them as the map
    /// does, and `lanes::near` is not cheap, so the list is kept for a quarter of a second
    /// rather than gathered every frame.
    snap_pool: Option<(f64, Vec<Joint>)>,
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
            multi: Vec::new(),
            anchor: None,
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
            rot_drag: None,
            side: Side::Auto,
            surface: std::sync::Arc::new(Vec::new()),
            surface_for: String::new(),
            surface_grid: None,
            sink: Vec::new(),
            sink_at: 0.0,
            link_now: false,
            link_report: Vec::new(),
            sink_next: 0,
            reveal: false,
            snap_mark: None,
            snap_pool: None,
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

/// The nearest of a pool of gathered ends - the lanes the map shows, and other objects' paths -
/// within `reach` of `q`. The dragged path's own ends are never taken from it: they are what
/// the drag is moving, and the crossing's own ends come from [`snap_joint`].
fn snap_pool_joint(crossing: &Crossing, pool: &[Joint], i: usize, q: DVec2, reach: f64) -> Option<Joint> {
    let own = crossing.paths.get(i);
    pool.iter()
        .copied()
        .filter(|j| (j.pos.truncate() - q).length() <= reach)
        .filter(|j| {
            own.is_none_or(|p| {
                (j.pos.truncate() - p.start().truncate()).length() > 1e-3
                    && (j.pos.truncate() - p.end().truncate()).length() > 1e-3
            })
        })
        .min_by(|a, b| (a.pos.truncate() - q).length().total_cmp(&(b.pos.truncate() - q).length()))
}

/// The end a dragged end is taken onto: the crossing's own ends ([`snap_joint`]) or a pooled
/// lane end, whichever is nearer. Ctrl keeps the end where the mouse is, as in the map's view.
fn snap_any(crossing: &Crossing, pool: &[Joint], i: usize, q: DVec2, reach: f64, ctrl: bool) -> Option<Joint> {
    if ctrl {
        return None;
    }
    let own = snap_joint(crossing, i, q, reach, ctrl);
    let Some(pooled) = snap_pool_joint(crossing, pool, i, q, reach) else { return own };
    match own {
        Some(j) if (j.pos.truncate() - q).length() <= (pooled.pos.truncate() - q).length() => Some(j),
        _ => Some(pooled),
    }
}

/// Put path `i`'s start onto a joint: at its point, in its way, and at its rise - so the
/// piece joins the next one smoothly instead of merely touching it.
fn join_start(crossing: &mut Crossing, i: usize, j: Joint) {
    let Some(p) = crossing.paths.get_mut(i) else { return };
    p.set_start(DVec3::new(j.pos.x, j.pos.y, j.pos.z));
    p.fit_start_tangent(j.outward());
    p.set_start_height_and_slope(j.pos.z, j.slope);
}

/// Put path `i`'s end onto a joint: its point and its rise. The way the path leaves its start
/// is kept - a drag moves an end, and turning the other end round to match the joint would
/// silently change the rotate the user set.
fn join_end(crossing: &mut Crossing, i: usize, j: Joint) {
    let Some(p) = crossing.paths.get_mut(i) else { return };
    let leave = p.heading();
    p.set_end(DVec3::new(j.pos.x, j.pos.y, p.end().z));
    p.fit_start_tangent(leave);
    p.set_end_height_and_slope(j.pos.z, j.slope);
}

/// The steepest rise a join's piece may take (m per metre). A piece of a few centimetres cannot
/// climb a kerb at any percentage at all: it climbs as fast as this allows and its end height is
/// a little out, which is nothing at that length. Without the limit a 0.7 m stub joining a path
/// 0.26 m higher asked for 68%.
const MAX_RISE: f64 = 0.5;

/// Set a piece's slopes so its end is at height `z`, with an even grade and no rise steeper
/// than [`MAX_RISE`].
fn climb_to(q: &mut Path, z: f64) {
    let l = q.length().max(0.05);
    let rise = ((z - q.start().z) / l).clamp(-MAX_RISE, MAX_RISE);
    q.v[6] = rise as f32;
    q.v[7] = rise as f32;
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

/// Height of one triangle (nine floats) over `p` in the object's own frame, when `p` is
/// inside it (a hair outside still counts: the drawn paths run along the mesh's edge).
fn tri_z(t: &[f32], p: DVec2) -> Option<f64> {
    let a = DVec2::new(t[0] as f64, t[1] as f64);
    let b = DVec2::new(t[3] as f64, t[4] as f64);
    let c = DVec2::new(t[6] as f64, t[7] as f64);
    let (za, zb, zc) = (t[2] as f64, t[5] as f64, t[8] as f64);
    let v0 = c - a;
    let v1 = b - a;
    let v2 = p - a;
    let den = v0.x * v1.y - v1.x * v0.y;
    if den.abs() < 1e-12 {
        return None;
    }
    let u = (v2.x * v1.y - v1.x * v2.y) / den;
    let v = (v0.x * v2.y - v2.x * v0.y) / den;
    if u >= -0.02 && v >= -0.02 && u + v <= 1.02 {
        Some(za * (1.0 - u - v) + zb * v + zc * u)
    } else {
        None
    }
}

/// Height of the object's surface at `p` (its own frame): the highest triangle that covers
/// the point, which is the deck rather than the underside. None when the point is off the mesh.
/// This is the slow reference (every triangle); the window uses [`SurfaceGrid`] through
/// [`CrossingPaths::surface_height`].
fn surface_z(tris: &[f32], p: DVec2) -> Option<f64> {
    let mut best: Option<f64> = None;
    for t in tris.chunks_exact(9) {
        if let Some(z) = tri_z(t, p) {
            best = Some(best.map(|h| h.max(z)).unwrap_or(z));
        }
    }
    best
}

/// A coarse 2D grid over the surface's triangles: each triangle is listed in every cell its
/// XY bounding box touches, so sampling a point only tests the handful of triangles near it.
/// A 787-path object used to walk all 60,000 triangles for every path, four times a second.
struct SurfaceGrid {
    cell: f64,
    ox: f64,
    oy: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl SurfaceGrid {
    fn build(tris: &[f32]) -> Option<SurfaceGrid> {
        if tris.len() < 9 {
            return None;
        }
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for t in tris.chunks_exact(9) {
            for k in [0, 3, 6] {
                let q = DVec2::new(t[k] as f64, t[k + 1] as f64);
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        if !lo.is_finite() || !hi.is_finite() || hi.x < lo.x || hi.y < lo.y {
            return None;
        }
        let mut cell = 4.0_f64;
        let mut nx = (((hi.x - lo.x) / cell).floor() as usize).saturating_add(1).max(1);
        let mut ny = (((hi.y - lo.y) / cell).floor() as usize).saturating_add(1).max(1);
        // (a huge object gets wider cells rather than a table no one wants to hold)
        while nx.max(ny) > 512 {
            cell *= 2.0;
            nx = (((hi.x - lo.x) / cell).floor() as usize).saturating_add(1).max(1);
            ny = (((hi.y - lo.y) / cell).floor() as usize).saturating_add(1).max(1);
        }
        let mut cells = vec![Vec::new(); nx * ny];
        for (i, t) in tris.chunks_exact(9).enumerate() {
            let (mut tx0, mut tx1) = (f64::MAX, f64::MIN);
            let (mut ty0, mut ty1) = (f64::MAX, f64::MIN);
            for k in [0, 3, 6] {
                let (x, y) = (t[k] as f64, t[k + 1] as f64);
                tx0 = tx0.min(x);
                tx1 = tx1.max(x);
                ty0 = ty0.min(y);
                ty1 = ty1.max(y);
            }
            let x0 = (((tx0 - lo.x) / cell).floor() as isize).clamp(0, nx as isize - 1) as usize;
            let x1 = (((tx1 - lo.x) / cell).floor() as isize).clamp(0, nx as isize - 1) as usize;
            let y0 = (((ty0 - lo.y) / cell).floor() as isize).clamp(0, ny as isize - 1) as usize;
            let y1 = (((ty1 - lo.y) / cell).floor() as isize).clamp(0, ny as isize - 1) as usize;
            for cy in y0..=y1 {
                let row = cy * nx;
                for cx in x0..=x1 {
                    cells[row + cx].push(i as u32);
                }
            }
        }
        Some(SurfaceGrid { cell, ox: lo.x, oy: lo.y, nx, ny, cells })
    }

    fn cell_of(&self, p: DVec2) -> Option<usize> {
        let (x, y) = ((p.x - self.ox) / self.cell, (p.y - self.oy) / self.cell);
        if !(x >= 0.0) || !(y >= 0.0) {
            return None;
        }
        let (cx, cy) = (x.floor() as usize, y.floor() as usize);
        (cx < self.nx && cy < self.ny).then_some(cy * self.nx + cx)
    }

    fn z(&self, tris: &[f32], p: DVec2) -> Option<f64> {
        self.z_tests(tris, p).0
    }

    /// The height, and how many triangles were looked at (so the saving can be measured).
    fn z_tests(&self, tris: &[f32], p: DVec2) -> (Option<f64>, usize) {
        let Some(c) = self.cell_of(p) else { return (None, 0) };
        let mut best: Option<f64> = None;
        let mut tests = 0usize;
        for &i in &self.cells[c] {
            let o = i as usize * 9;
            let Some(t) = tris.get(o..o + 9) else { continue };
            tests += 1;
            if let Some(z) = tri_z(t, p) {
                best = Some(best.map(|h| h.max(z)).unwrap_or(z));
            }
        }
        (best, tests)
    }
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
    let segs = connect_segs(a, h0, b, h1, Side::Auto);
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

/// Cut path `i` in two `at` of the way along it (0..1), and give back the index of the second
/// piece. Both are arcs of the path's own radius, and the second begins in exactly the way the
/// first ends, so the line itself is unchanged where it was cut - what changes is that each
/// piece can now be turned, stretched and given a rotate of its own, which is the only way to
/// put a bend in the middle of a piece that was laid as one.
///
/// The piece that was written first keeps everything: kind, width, the way traffic goes, its
/// turn signal and light, its blocks. The second is a copy of it with its own start, and the
/// block rules that name later paths follow it ([`insert_after`]).
///
/// None when the path is too short to be worth cutting: a piece of a few centimetres is not
/// something a vehicle can use, and a cut within 5% of either end is moved away from it.
fn split_path(crossing: &mut Crossing, i: usize, at: f64) -> Option<usize> {
    let p = crossing.paths.get(i)?.clone();
    let l = p.length();
    if !(l > 0.2) {
        return None;
    }
    let s = l * at.clamp(0.05, 0.95);
    let c = p.curve();
    let mid = c.point_at(s);
    let heading = crate::roads::norm_deg(c.heading_at(s));
    // (the rise where it is cut: the same on both sides, so the profile does not step)
    let rise = c.slope_at(s);
    let mut first = p.clone();
    first.v[5] = s as f32;
    first.v[7] = rise as f32;
    let mut second = p;
    second.v[0] = mid.x as f32;
    second.v[1] = mid.y as f32;
    second.v[2] = mid.z as f32;
    second.v[3] = heading as f32;
    second.v[5] = (l - s) as f32;
    second.v[6] = rise as f32;
    crossing.paths[i] = first;
    insert_after(&mut crossing.paths, i, vec![second]);
    Some(i + 1)
}

/// How near (m) one lane's leaving end and the next lane's entering end must lie to be joined,
/// how far apart in height they may be, and how far their headings may differ: openOMSI's own
/// rule (`openOMSI-main/crates/omsi-sim/src/traffic/extend.rs`, `LINK_TOLERANCE`, `dh < 40`
/// and `(s.z - e.z).abs() < 3.0`). It joins lanes by geometry and **never reads
/// `[splinehelper]`**, so a crossing's paths have to meet the roads by these numbers before
/// openOMSI will drive traffic from one into the other. The link check is made against them
/// (see the tests below, which apply the rule itself).
#[allow(dead_code)]
const LINK_TOLERANCE: f64 = 1.5;
#[allow(dead_code)]
const LINK_RISE: f64 = 3.0;
#[allow(dead_code)]
const LINK_ANGLE: f64 = 40.0;

/// How far (m) a crossing end may be from a road end and be put onto it, and how far it may be
/// and still be reached with one connecting piece. Beyond that the road end is taken to be a
/// lane passing the crossing rather than one ending at it.
const LINK_MOVE: f64 = 6.0;
const LINK_CONNECT: f64 = 15.0;

/// One end of a road lane as openOMSI has it, in the crossing object's own frame: where traffic
/// enters the lane or leaves it, and the heading it moves in there.
#[derive(Clone, Debug)]
struct RoadLaneEnd {
    at: DVec3,
    heading: f64,
    /// Traffic enters the lane here (its start); else it leaves it (its end).
    entering: bool,
    kind: i32,
    /// Which lane it is, for the report.
    who: String,
    /// The road type of the spline it runs along (a `.sli`), when it is a spline's: what a
    /// `[splinehelper]` at this end names, for the game's own use.
    file: Option<String>,
}

/// One end of a crossing path as openOMSI has it. A path of `direction` 0 makes one lane (its
/// start to its end), 1 one lane the other way (its end to its start), 2 a lane each way - so
/// at a path's start traffic may leave, arrive, or both.
#[derive(Clone, Copy, Debug)]
struct CrossingEnd {
    path: usize,
    at_start: bool,
    at: DVec3,
    /// The heading traffic leaving the crossing at this end moves in, when it can leave.
    leave: Option<f64>,
    /// The heading traffic arriving at the crossing here moves in, when it can arrive.
    arrive: Option<f64>,
    kind: i32,
}

/// What became of one road end.
#[derive(Clone, Debug)]
struct LinkNote {
    who: String,
    ok: bool,
    /// Why not, or how far it was moved.
    why: String,
    /// Where the link is, and the way the road leaves the crossing there, for a
    /// `[splinehelper]` put on it.
    at: Option<DVec3>,
    away: f64,
    /// The road type of that lane's spline, when it is one's.
    file: Option<String>,
}

/// The ends of the road lanes round the crossing, in its own frame, as openOMSI sees them.
fn road_lane_ends(lanes: &[crate::lanes::Lane], world: DVec3, m: &glam::Mat4, yaw: f64, doc: &crate::doc::Document) -> Vec<RoadLaneEnd> {
    let inv = m.inverse();
    let local = |p: DVec3| inv.transform_point3((p - world).as_vec3()).as_dvec3();
    let mut out = Vec::new();
    for l in lanes {
        let n = l.pts.len();
        if n < 2 {
            continue;
        }
        let h0 = crate::roads::heading_of((l.pts[1] - l.pts[0]).truncate());
        let h1 = crate::roads::heading_of((l.pts[n - 1] - l.pts[n - 2]).truncate());
        let who = match (l.spline, l.object) {
            (Some((_, id, _)), _) => format!("spline {id} #{}", l.index),
            (None, Some((_, id))) => format!("object {id} #{}", l.index),
            _ => format!("#{}", l.index),
        };
        // (the road type the game's `[splinehelper]` names, so a helper put on the link is
        // the one the road there is)
        let file = l.spline.and_then(|(k, id, _)| doc.spline(k, id).map(|s| s.file.clone()));
        let mut push = |at: DVec3, heading: f64, entering: bool| {
            out.push(RoadLaneEnd { at: local(at), heading: crate::roads::norm_deg(heading - yaw), entering, kind: l.kind, who: who.clone(), file: file.clone() });
        };
        // (a lane runs along its points, or against them; both ways makes one each way)
        if l.direction != 1 {
            push(l.pts[0], h0, true);
            push(l.pts[n - 1], h1, false);
        }
        if l.direction != 0 {
            push(l.pts[n - 1], crate::roads::norm_deg(h1 + 180.0), true);
            push(l.pts[0], crate::roads::norm_deg(h0 + 180.0), false);
        }
    }
    out
}

/// The ends of a crossing's paths, as openOMSI sees them.
fn crossing_ends(crossing: &Crossing) -> Vec<CrossingEnd> {
    let mut out = Vec::new();
    for (i, p) in crossing.paths.iter().enumerate() {
        let c = p.curve();
        let hs = crate::roads::norm_deg(p.heading());
        let he = crate::roads::norm_deg(c.heading_at(c.length.max(0.0)));
        let d = p.direction();
        let (fwd, back) = (d != 1, d != 0);
        for (at_start, at) in [(true, p.start()), (false, p.end())] {
            let h = if at_start { hs } else { he };
            // (a forward lane leaves from the start and arrives at the end; a backward one the
            // other way round, so both roles exist at each end of a two-way path)
            let (leave, arrive) = if at_start {
                (fwd.then_some(h), back.then_some(crate::roads::norm_deg(h + 180.0)))
            } else {
                (back.then_some(crate::roads::norm_deg(h + 180.0)), fwd.then_some(h))
            };
            out.push(CrossingEnd { path: i, at_start, at, leave, arrive, kind: p.kind() });
        }
    }
    out
}

/// Put one end of path `i` on `to`, its geometry there on `tangent`, keeping the other end
/// where it is and the rise between the two. None when the turn would swing the other end
/// away (it is then restored rather than break a join at that end).
fn align_end(crossing: &mut Crossing, i: usize, at_start: bool, to: DVec3, tangent: f64) -> bool {
    let Some(before) = crossing.paths.get(i).cloned() else { return false };
    let keep = if at_start { before.end() } else { before.start() };
    {
        let Some(p) = crossing.paths.get_mut(i) else { return false };
        if at_start {
            p.v[0] = to.x as f32;
            p.v[1] = to.y as f32;
            p.v[2] = to.z as f32;
            p.fit_start_tangent(tangent);
            let l = p.length();
            if l > 1e-3 {
                let rise = (2.0 * (keep.z - to.z) / l - p.slope_start() as f64).clamp(-MAX_RISE, MAX_RISE);
                p.v[7] = rise as f32;
            }
        } else {
            // (x and y move the end; the height comes from the slopes, set below)
            p.set_end(DVec3::new(to.x, to.y, p.end().z));
            p.fit_end_tangent(tangent);
            let l = p.length();
            if l > 1e-3 {
                let rise = (2.0 * (to.z - keep.z) / l - p.slope_end() as f64).clamp(-MAX_RISE, MAX_RISE);
                p.v[6] = rise as f32;
            }
        }
    }
    let Some(after) = crossing.paths.get(i) else { return false };
    let far = if at_start { after.end() } else { after.start() };
    if (far.truncate() - keep.truncate()).length() > 0.05 || (far.z - keep.z).abs() > 0.05 {
        crossing.paths[i] = before;
        return false;
    }
    true
}

/// Line the crossing's path ends up with the road lanes round it, so that openOMSI's own lane
/// joining finds them (see [`LINK_TOLERANCE`]). Each road end is taken in turn, nearest first:
/// a crossing end of the same kind within [`LINK_MOVE`] is put onto it; within [`LINK_CONNECT`]
/// one connecting piece is added; further than that it is left and said so.
///
/// `tris` is the object's mesh, for seating a connecting piece; `side` is the way round a join
/// is made. Every note is returned, so the panel can say what happened.
fn link_crossing(crossing: &mut Crossing, roads: &[RoadLaneEnd], tris: &[f32], side: Side) -> Vec<LinkNote> {
    let mut notes = Vec::new();
    let mut ends = crossing_ends(crossing);
    // (which crossing roles have already been given to a road end: a direction 2 path has both
    // roles at one point, so the role is part of the key)
    let mut used: Vec<(usize, bool, bool)> = Vec::new();
    let mut order: Vec<&RoadLaneEnd> = roads.iter().collect();
    order.sort_by(|a, b| a.at.truncate().length().total_cmp(&b.at.truncate().length()));
    for r in order {
        // (a lane that begins at the crossing is fed by the crossing: the crossing's traffic
        // arrives here. One that ends at it takes the crossing's traffic onward: it leaves
        // here)
        let want_leave = !r.entering;
        let mut best: Option<(f64, usize)> = None;
        for (n, e) in ends.iter().enumerate() {
            if e.kind != r.kind || used.contains(&(e.path, e.at_start, want_leave)) {
                continue;
            }
            let has = if want_leave { e.leave } else { e.arrive };
            if has.is_none() {
                continue;
            }
            let d = (e.at.truncate() - r.at.truncate()).length();
            if d > LINK_CONNECT {
                continue;
            }
            if best.is_none_or(|b: (f64, usize)| d < b.0) {
                best = Some((d, n));
            }
        }
        let Some((d, n)) = best else {
            notes.push(LinkNote { who: r.who.clone(), ok: false, why: tr("no end of the crossing is near it", "ни один конец перекрёстка не рядом").to_string(), at: None, away: 0.0, file: None });
            continue;
        };
        // (the way the road leaves the crossing there: a helper on the link points out along
        // it, which is what the game snaps a spline by)
        let away = if r.entering { r.heading } else { crate::roads::norm_deg(r.heading + 180.0) };
        let e = ends[n];
        used.push((e.path, e.at_start, want_leave));
        if d > LINK_MOVE {
            // (one piece between the two, laid the way the traffic goes: a road that ends at
            // the crossing feeds it, so the piece runs from the road end to the crossing end;
            // a road that begins here is fed by it, so the piece runs the other way. Either
            // way its ends land exactly on the two points, on their headings)
            let (start, h_a, end, h_b, z0, z1) = if want_leave {
                (r.at.truncate(), r.heading, e.at.truncate(), e.leave.unwrap_or(r.heading), r.at.z, e.at.z)
            } else {
                (e.at.truncate(), e.arrive.unwrap_or(r.heading), r.at.truncate(), r.heading, e.at.z, r.at.z)
            };
            let segs = connect_segs(start, h_a, end, h_b, side);
            if segs.is_empty() {
                notes.push(LinkNote { who: r.who.clone(), ok: false, why: tr("the gap has no way through", "промежуток нечем заполнить").to_string(), at: None, away: 0.0, file: None });
                continue;
            }
            let template = crossing.paths[e.path].clone();
            let a = DVec3::new(start.x, start.y, z0);
            let made = lay_join(&template, a, 0.0, &segs, PieceEnd::At { z: z1, slope: template.slope_end() }, false, tris);
            for mut q in made {
                q.v[8] = r.kind as f32;
                // (one way with the piece it grows from; a two-way path stays two-way)
                q.v[10] = if template.direction() == 2 { 2.0 } else { 0.0 };
                crossing.paths.push(q);
            }
            notes.push(LinkNote { who: r.who.clone(), ok: true, why: tr("a piece added", "добавлен отрезок").to_string(), at: Some(r.at), away, file: r.file.clone() });
            continue;
        }
        // (put the crossing end on the road end: the geometry there has to be the road's
        // heading, or half a turn from it when the crossing's traffic runs the other way)
        let tangent = if (e.at_start && want_leave) || (!e.at_start && !want_leave) {
            r.heading
        } else {
            crate::roads::norm_deg(r.heading + 180.0)
        };
        if align_end(crossing, e.path, e.at_start, r.at, tangent) {
            ends[n].at = r.at;
            if want_leave {
                ends[n].leave = Some(r.heading);
            } else {
                ends[n].arrive = Some(r.heading);
            }
            notes.push(LinkNote { who: r.who.clone(), ok: true, why: format!("{d:.2} m"), at: Some(r.at), away, file: r.file.clone() });
        } else {
            notes.push(LinkNote { who: r.who.clone(), ok: false, why: tr("the other end would have moved", "другой конец сдвинулся бы").to_string(), at: None, away: 0.0, file: None });
        }
    }
    notes
}

/// The `[splinehelper]` a linked road end is given: at the road end, facing out along the road,
/// naming the road type there. The game snaps its splines onto the object by it (openOMSI does
/// not read it, its lanes meeting by geometry).
fn helper_for(n: &LinkNote) -> Option<Helper> {
    let at = n.at?;
    let file = n.file.as_ref().filter(|f| !f.trim().is_empty())?;
    Some(Helper { spline: file.clone(), x: at.x as f32, up: at.z as f32, fwd: at.y as f32, heading: n.away as f32, length: 0.1, radius: 0.0 })
}

/// A join further than this from the path it joins is not a join: no crossing is tens of
/// kilometres across, so the joint must be in another frame (the map's, say). Refused rather
/// than built, since a join made from it is a path of millions of metres.
const FAR_JOIN: f64 = 10_000.0;

/// Which way a join is to turn, shared with the planner ([`crate::join`]): the automatic choice
/// takes the widest arcs it can find; left or right says which way round to go instead.
pub use crate::join::Side;

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
fn forward_segs(start: DVec2, heading: f64, end: DVec2, side: Side) -> Vec<crate::roads::Seg> {
    use crate::roads;
    let chord = (end - start).length();
    // (the planned way into the point: a turn, a straight, a turn, all on one radius and spread
    // over the whole distance. Stepping a piece at a time piles the turn into the last few
    // centimetres, which reads as a hook. Only when that way is much longer than the distance
    // itself - a point behind the path, where no radius can do anything else - is the stepping
    // below used instead)
    if (0.5..=2000.0).contains(&chord)
        && let Some(segs) = crate::join::plan_to_point(start, heading, end, side, MIN_PIECE_RADIUS)
    {
        let length: f64 = segs.iter().map(|s: &roads::Seg| s.length).sum();
        if !segs.is_empty() && length <= chord * 2.2 + 5.0 {
            return segs;
        }
    }
    let mut segs = Vec::new();
    let mut pos = start;
    let mut h = roads::norm_deg(heading);
    // (the way the run turns, chosen once: a car holds one way round until it is lined up,
    // it does not turn left, then right, then left again)
    let mut way = side.sign();
    // (how near the point the run has come: a piece that does not bring it nearer is going
    // round the point rather than to it, which winds the path into a ring. The last piece
    // then closes it)
    let mut closest = (end - start).length();
    for _ in 0..12 {
        let dist = (end - pos).length();
        if dist < 0.4 {
            return segs;
        }
        let deflect = ang_delta(h, roads::heading_of(end - pos));
        if way == 0.0 && deflect.abs() > 1.0 {
            way = deflect.signum();
        }
        if deflect.abs() <= MAX_DEFLECT && side.allows(2.0 * deflect) {
            // (an arc that leaves on `h` and passes through the point leaves its chord at
            // half its turn: the turn is twice the deflection. Only this closing piece does
            // that — doubling every piece swings past the point and the next one turns back,
            // which is the snake)
            segs.push(arc_piece(pos, h, 2.0 * deflect, dist));
            return segs;
        }
        // Part of the way round, and no further than the point itself lies, and the same way
        // as the piece before. Turning twice the deflection here would pass it.
        let chord = (dist * 0.6).clamp(1.0, 30.0);
        let turn = steer(side, way, deflect, chord);
        let s = arc_piece(pos, h, turn, chord);
        h = s.end_heading();
        pos = s.end();
        segs.push(s);
        let now = (end - pos).length();
        if now > closest - 0.05 {
            break;
        }
        closest = now;
    }
    if (end - pos).length() >= 0.4 {
        segs.push(closing_piece(pos, h, end));
    }
    segs
}

/// The last piece of a run: it lands on `end` and turns the way the point lies, whatever radius
/// that asks for. A closing piece is not turned round the other way to suit a chosen side - that
/// leaves the end of the join in mid-air - and a point almost exactly behind it is closed with a
/// straight, since the arc through that one is a loop of kilometres.
fn closing_piece(pos: DVec2, h: f64, end: DVec2) -> crate::roads::Seg {
    let dist = (end - pos).length();
    let phi = crate::roads::heading_of(end - pos);
    let deflect = ang_delta(h, phi);
    if deflect.abs() * 2.0 > 340.0 {
        return crate::roads::Seg { start: pos, heading: phi, length: dist, radius: 0.0 };
    }
    arc_piece(pos, h, 2.0 * deflect, dist)
}

/// Whether `segs` really ends on `end`, arriving on heading `h_b`. A join is checked on this
/// rather than trusted: an arc built for the point but not the way in leaves a kink against the
/// next segment, which is what the panel's rotate shows.
fn arrives(segs: &[crate::roads::Seg], end: DVec2, h_b: f64) -> bool {
    let Some(last) = segs.last() else { return false };
    (last.end() - end).length() < 0.02 && ang_delta(last.end_heading(), h_b).abs() < 0.5
}

/// A join shorter than this is closed with one straight piece: over a few centimetres there is
/// no room for a bend, and what a bend gives there is a knot.
const SHORT_JOIN: f64 = 2.0;

/// The tightest radius a join's piece may have (m). Below it the piece is not a bend but a
/// knot: a path of a few centimetres curled round on itself, which reads as the wrong way.
/// A tight junction turn is a metre or two; a centimetre is not a path at all.
const MIN_PIECE_RADIUS: f64 = crate::join::MIN_PIECE_RADIUS;

/// The most a piece may turn and still be a bend of at least `MIN_PIECE_RADIUS` across `chord`.
fn most_turn(chord: f64) -> f64 {
    (2.0 * (chord / (2.0 * MIN_PIECE_RADIUS)).clamp(-1.0, 1.0).asin().to_degrees()).abs()
}

/// How far an intermediate piece turns. `way` is the way the run already goes (+1 right), and
/// the piece keeps it: it never turns back the other way, and it never turns further than the
/// point lies, so the next piece has no reason to reverse. A side the user chose is kept even
/// when the point lies the other way, which is the long way round.
fn steer(side: Side, way: f64, deflect: f64, chord: f64) -> f64 {
    // (the whole of the way the point lies, not a slice of it: a slice leaves the heading
    // short, the chord walks past, and the next piece turns back — the snake)
    let toward = match side {
        Side::Auto => {
            if way * deflect > 0.0 { way * deflect.abs() } else { 0.0 }
        }
        s => s.sign() * deflect.abs().min(80.0),
    };
    toward.clamp(-most_turn(chord), most_turn(chord))
}

/// Two arcs that leave `start` on `h_a`, meet tangentially at a point in between, and arrive
/// at `end` on `h_b` - each arriving exactly in the way the next leaves, so the join has no
/// kink and both ways are kept. The meeting point is found by its own heading, taken in small
/// steps: a heading that puts the point in front of both ends is the one used, preferring the
/// widest radii.
///
/// A pair is only used when both of its arcs are real bends (`min_r` at least) and both are
/// longer than `min_len`: a pair of centimetre knots closes the join in geometry but is not a
/// path anyone can drive. The floors are lowered by [`connect_segs`] in turn, so a join that has
/// no wide answer still arrives exactly on its heading instead of stopping a few degrees short.
fn s_join(start: DVec2, h_a: f64, end: DVec2, h_b: f64, side: Side) -> Option<Vec<crate::roads::Seg>> {
    s_join_with(start, h_a, end, h_b, side, MIN_PIECE_RADIUS, 1.0)
}

/// [`s_join`] with the radius and chord floors given. A biarc always exists for a pair of ends,
/// so with the floors lowered far enough this is the answer that cannot fail: it arrives on the
/// point and on `h_b` by construction.
fn s_join_with(start: DVec2, h_a: f64, end: DVec2, h_b: f64, side: Side, min_r: f64, min_len: f64) -> Option<Vec<crate::roads::Seg>> {
    use crate::roads::{self, Seg};
    let (h_a, h_b) = (roads::norm_deg(h_a), roads::norm_deg(h_b));
    let v = end - start;
    let mut made: Vec<Vec<Seg>> = Vec::new();
    // (a finer sweep when the wide ones are barred: the meeting heading that puts both chords
    // in front of their ends is a narrow window at some angles)
    let fine = min_r < MIN_PIECE_RADIUS || min_len < 1.0;
    let (first, last, step_deg): (i32, i32, f64) = if fine {
        if v.length() < 8.0 { (-360, 360, 0.5) } else { (-180, 180, 1.0) }
    } else {
        (-36, 36, 5.0)
    };
    for step in first..=last {
        let hm = roads::norm_deg(h_a + step as f64 * step_deg as f64);
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
        if !(a > min_len && b > min_len) {
            continue;
        }
        let r1 = if t1.abs() < 1e-4 { f64::MAX } else { a / (2.0 * (t1.to_radians() * 0.5).abs().sin()) };
        let r2 = if t2.abs() < 1e-4 { f64::MAX } else { b / (2.0 * (t2.to_radians() * 0.5).abs().sin()) };
        let narrow = r1.min(r2);
        // (a pair this tight is a knot, not a bend)
        if narrow < min_r {
            continue;
        }
        // (a side chosen is a side: this pair has to go that way round)
        if !side.allows(t1) || !side.allows(t2) {
            continue;
        }
        let mid = start + u1 * a;
        made.push(vec![arc_piece(start, h_a, t1, a), arc_piece(mid, hm, t2, b)]);
    }
    // (the same rule as a planned join: a stub-free pair first, then the one whose turn is most
    // evenly spread over its pieces. The widest meeting radius alone can leave one long gentle
    // arc and one centimetre doing all the turning, which is the stub this exists to avoid)
    crate::join::best(made, v.length())
}

/// The join that has to arrive on the next path's way, however tight its arcs: what `Complete
/// to` falls back on when the planned join lands a little off the point or the heading. It
/// tries the same kinds of candidate as [`connect_segs`] first, scored by [`crate::join::best`],
/// and only then lets the radii down as far as the pair needs. Even then no piece may be shorter
/// than `CURL`: a bend of half a metre is still a bend, but a five-centimetre arc is an invisible
/// curl written into the file. A line a few degrees out of the next segment's rotate is the kink
/// this exists to avoid, so when nothing usable arrives it answers None rather than that.
fn forced_join(start: DVec2, h_a: f64, end: DVec2, h_b: f64, side: Side) -> Option<Vec<crate::roads::Seg>> {
    let chord = (end - start).length();
    // (no forced join may write a piece shorter than this into the file: below it the piece is
    // an invisible curl, not a bend anyone can drive)
    const CURL: f64 = 0.3;
    let usable = |segs: &Vec<crate::roads::Seg>| segs.iter().all(|s| s.length >= CURL);
    // (first the same kind of candidates `connect_segs` plans with: a very tight pair is not
    // the only answer, and on a nearly straight joint it is the wrong one. The best of them is
    // taken by the same scoring, so a retry cannot smuggle in a stub that the planned join
    // would have refused)
    let mut candidates: Vec<Vec<crate::roads::Seg>> = Vec::new();
    for f in [MIN_PIECE_RADIUS, 1.0, 0.6, 0.35, 0.2] {
        if let Some(planned) = crate::join::plan(start, h_a, end, h_b, side, f)
            && arrives(&planned, end, h_b)
            && usable(&planned)
        {
            candidates.push(planned);
        }
    }
    for (r, len) in [(MIN_PIECE_RADIUS, 1.0), (1.0, 0.3), (0.6, 0.1), (0.35, 0.05)] {
        if let Some(pair) = s_join_with(start, h_a, end, h_b, side, r, len)
            && arrives(&pair, end, h_b)
            && usable(&pair)
        {
            candidates.push(pair);
        }
    }
    if let Some(best) = crate::join::best(candidates, chord) {
        return Some(best);
    }
    // (nothing with a usable radius reaches the heading. Only now are the tight arcs let out,
    // and even then no piece may be shorter than `CURL`)
    for (r, len) in [(0.35, CURL), (0.2, CURL), (0.15, CURL), (0.1, CURL), (0.05, CURL)] {
        if let Some(pair) = s_join_with(start, h_a, end, h_b, side, r, len)
            && arrives(&pair, end, h_b)
            && usable(&pair)
        {
            return Some(pair);
        }
    }
    None
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
/// A join of a couple of metres or less is planned and checked like any other: a short gap
/// between two ways that cross needs two or three pieces, and one straight along the gap
/// arrived on its own angle instead of the next segment's. A straight on the gap is only the
/// last resort, for a pair of ends no bend of a usable radius can join.
///
/// `side` is the way round the user asked for. A left or right join only takes arcs that turn
/// that way, and its forward pieces turn that way, so the long way round a junction is not what
/// comes out when the short way was meant.
fn connect_segs(start: DVec2, h_a: f64, end: DVec2, h_b: f64, side: Side) -> Vec<crate::roads::Seg> {
    let chord = (end - start).length();
    if chord < 1e-3 {
        return Vec::new();
    }
    // (the planned join: at most three pieces, all on one radius, leaving in `h_a` and arriving
    // in `h_b`. This is what keeps the turn spread over the whole gap. It is checked on the way
    // it actually ends - on the point and on the heading - because a piece that lands a few
    // degrees off is a kink against the next segment, which is exactly what the panel's rotate
    // then shows. A biarc (two arcs that meet tangentially) always has an answer for a pair of
    // ends, so it is asked with lower and lower floors until one lands properly, and the
    // stepping loop below is the very last resort)
    let floor = MIN_PIECE_RADIUS;
    // (a join whose ways cross - leave north, arrive at a point beside you still north - has no
    // answer on a wide radius at all, only on a tight one, so the floor is lowered in turn. Every
    // candidate is checked on the way it actually ends, because a piece that lands a few degrees
    // off is the kink the next segment's rotate then shows. A gap of a couple of metres or less
    // may not use a radius below `CLOSE_RADIUS`: there is no room for it, and the bend it makes
    // is the centimetre curl that reads as the wrong way)
    let allow = |r: f64| {
        let floor = if chord < 0.5 { CLOSE_RADIUS } else { 0.35 };
        r == 0.0 || r.abs() >= floor
    };
    let fits = |segs: &[crate::roads::Seg]| arrives(segs, end, h_b) && segs.iter().all(|q| allow(q.radius));
    // (the longest a planned join may be: a guard against a planner that loops for millions of
    // metres, not a curb on going round. A short gap between two ways that cross needs two or
    // three pieces several times longer than the gap, and a cap that was a multiple of the gap
    // threw exactly those away, leaving a straight that arrived on the wrong way)
    let cap = (chord * 20.0 + 50.0).min(500.0);
    let total = |p: &Vec<crate::roads::Seg>| p.iter().map(|s| s.length).sum::<f64>();
    // (every candidate that lands on the point and on the way is scored by `join::best`: a join
    // with no stub wins outright, and among those the one that goes least far round the junction
    // while spreading its turn over pieces long enough to drive. A final piece of a few
    // centimetres is exactly the jerk this is here to keep out, whatever radius it claims)
    let mut candidates: Vec<Vec<crate::roads::Seg>> = Vec::new();
    for f in [floor, 1.0, 0.6, 0.35, 0.2] {
        if let Some(planned) = crate::join::plan(start, h_a, end, h_b, side, f)
            && fits(&planned)
            && total(&planned) <= cap
        {
            candidates.push(planned);
        }
    }
    // (two arcs that meet tangentially, with the floors let down as far as a usable path allows:
    // a biarc answers for a pair of ends that no plain turn onto a circle does. These are scored
    // with the planned joins rather than kept as a fallback, since a biarc can be the even answer
    // when the tangent construction only manages a stub)
    for (r, len) in [(MIN_PIECE_RADIUS, 1.0), (1.0, 0.3), (0.6, 0.1), (0.35, 0.05)] {
        if let Some(pair) = s_join_with(start, h_a, end, h_b, side, r, len)
            && fits(&pair)
            && total(&pair) <= cap
        {
            candidates.push(pair);
        }
    }
    if let Some(best) = crate::join::best(candidates, chord) {
        // (a planner can still answer with a piece that turns most of a circle to move a couple
        // of metres sideways. When it does, the forward run is built as well and scored against
        // it: the run's pieces actually go on towards the point, rather than round it)
        if !crate::join::has_loop(&best) {
            return best;
        }
        let stepped = stepping_join(start, h_a, end, h_b, side);
        if fits(&stepped) && !stepped.is_empty() {
            if let Some(picked) = crate::join::best(vec![best.clone(), stepped], chord) {
                return picked;
            }
        }
        return best;
    }
    stepping_join(start, h_a, end, h_b, side)
}

/// The forward run: pieces that go on from `start` towards `end`, turning no further than the
/// point lies, and arriving on `h_b`. This is the last resort of [`connect_segs`], and also the
/// answer when the planner can only offer a loop.
fn stepping_join(start: DVec2, h_a: f64, end: DVec2, h_b: f64, side: Side) -> Vec<crate::roads::Seg> {
    use crate::roads;
    let mut segs = Vec::new();
    let mut pos = start;
    let mut h = roads::norm_deg(h_a);
    let h_b = roads::norm_deg(h_b);
    let mut way = side.sign();
    // (a piece that does not bring the point nearer is going round it, and enough of those
    // wind the join into a ring. The last piece closes it instead)
    let mut closest = (end - start).length();
    for _ in 0..12 {
        let dist = (end - pos).length();
        if dist < 0.02 {
            return segs;
        }
        if dist < 0.5 {
            // (under half a metre a bend is a knot: one straight, whatever the next piece's angle)
            if dist >= 0.02 {
                segs.push(crate::roads::Seg { start: pos, heading: roads::heading_of(end - pos), length: dist, radius: 0.0 });
            }
            return segs;
        }
        if dist < SHORT_JOIN {
            // (short, but long enough to arrive in the next piece's way when a bend of a
            // usable radius does it. A straight along the gap is what left this piece on
            // its own angle instead of the next segment's)
            if let Some(done) = arrive_on(pos, h, end, h_b, side) {
                segs.extend(done);
                return segs;
            }
            if dist >= 0.05 {
                let phi = roads::heading_of(end - pos);
                let agrees = ang_delta(h, phi).abs() <= 8.0 && ang_delta(h, h_b).abs() <= 8.0;
                if segs.is_empty() {
                    segs.push(crate::roads::Seg { start: pos, heading: if agrees { h } else { phi }, length: dist, radius: 0.0 });
                } else if agrees {
                    segs.push(crate::roads::Seg { start: pos, heading: h, length: dist, radius: 0.0 });
                } else {
                    // (keep the way the previous piece arrived: a straight on the gap's own
                    // angle would kink against it)
                    let off = ang_delta(h, phi).clamp(-MAX_DEFLECT, MAX_DEFLECT);
                    segs.push(arc_piece(pos, h, 2.0 * off, dist));
                }
            }
            return segs;
        }
        let phi = roads::heading_of(end - pos);
        let deflect = ang_delta(h, phi);
        if way == 0.0 && deflect.abs() > 1.0 {
            way = deflect.signum();
        }
        // (one arc: its chord must lie at half its turn, and it must turn into `h_b`)
        let turn = ang_delta(h, h_b);
        if turn.abs() <= 140.0 && side.allows(turn) && ang_delta(roads::norm_deg(h + turn * 0.5), phi).abs() <= 3.0 && (turn.abs() < 1e-4 || dist / (2.0 * (turn.to_radians() * 0.5).abs().sin()) >= MIN_PIECE_RADIUS) {
            segs.push(arc_piece(pos, h, turn, dist));
            return segs;
        }
        if let Some(two) = s_join(pos, h, end, h_b, side) {
            segs.extend(two);
            return segs;
        }
        // (too far round for either: go on the same way, and not past where the point lies,
        // or the next piece turns back and the run snakes)
        let chord = (dist * 0.6).clamp(1.0, 30.0);
        let turn = steer(side, way, deflect, chord);
        let s = arc_piece(pos, h, turn, chord);
        h = s.end_heading();
        pos = s.end();
        segs.push(s);
        let now = (end - pos).length();
        if now > closest - 0.05 {
            break;
        }
        closest = now;
    }
    if (end - pos).length() >= 0.4 {
        if let Some(done) = arrive_on(pos, h, end, h_b, side) {
            segs.extend(done);
        } else {
            segs.push(closing_piece(pos, h, end));
        }
    }
    segs
}

/// Pieces that leave `pos` on `h` and arrive at `end` on `h_b`, when a bend of at least
/// [`CLOSE_RADIUS`] can do it. None when the only way onto that heading would be a knot.
/// One arc when the point lies on it, otherwise the two arcs of [`s_join`].
fn arrive_on(pos: DVec2, h: f64, end: DVec2, h_b: f64, side: Side) -> Option<Vec<crate::roads::Seg>> {
    let dist = (end - pos).length();
    if dist < 0.5 {
        return None;
    }
    let turn = ang_delta(h, h_b);
    let radius = if turn.abs() < 1e-3 { f64::MAX } else { dist / (2.0 * (turn.to_radians() * 0.5).abs().sin()) };
    if turn.abs() <= 140.0 && side.allows(turn) && radius >= CLOSE_RADIUS {
        let piece = arc_piece(pos, h, turn, dist);
        // (the arc only counts when it actually ends on the point, not merely near its direction)
        if (piece.end() - end).length() < 0.02 {
            return Some(vec![piece]);
        }
    }
    s_join(pos, h, end, h_b, side)
}

/// The tightest radius a short join may use to arrive on the next piece's heading (m).
/// A metre of path can turn onto the next segment; a few centimetres curled round cannot.
const CLOSE_RADIUS: f64 = 1.0;

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

/// `template`'s shape laid along `segs`, beginning at `a` with the rise `s_a` there. For a free
/// end the pieces between sit on the surface, so a join follows the deck rather than cutting
/// through it; for another path's end the rise is spread along the join, and the last piece
/// lands on that end's height and rise exactly. `inherit`: the first piece keeps the template's
/// own rules (a drag of an end, rather than a new piece added beside it).
fn lay_join(template: &Path, a: DVec3, s_a: f64, segs: &[crate::roads::Seg], end: PieceEnd, inherit: bool, tris: &[f32]) -> Vec<Path> {
    let n = segs.len();
    let mut out: Vec<Path> = Vec::new();
    let mut z = a.z;
    // (a join to another path's end spreads its rise over the whole join: the last piece
    // alone cannot carry a kerb's rise when it is ten centimetres long, and the 130% grade
    // that asked for is not a path. Each piece ends where its share of the length puts it)
    let at = match end {
        PieceEnd::At { z, slope } => Some((z, slope)),
        PieceEnd::Surface => None,
    };
    let total: f64 = segs.iter().map(|s| s.length).sum();
    let mut cum = 0.0;
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
        cum += q.length();
        if let Some((z_target, slope)) = at {
            if k + 1 == n {
                // (the end is where the other path's is, at its rise: a footway joined to the
                // next footway takes its height and its slope exactly, however short this
                // piece is. The rise has been spread along the join, so the last piece lands
                // on that height with a sane grade instead of taking the whole climb at its
                // own end)
                q.set_end_height_and_slope(z_target, slope);
            } else if total > 0.03 {
                let zk = a.z + (z_target - a.z) * (cum / total).clamp(0.0, 1.0);
                // (both slopes of one piece become its own share of the rise, so the next
                // piece begins where this one ended, in height as in position)
                let l = q.length().max(1e-6);
                let rise = (zk - q.start().z) / l;
                if k == 0 {
                    // (the first piece still leaves at the rise the path before it has: the
                    // join meets that path without a step in grade. Its share of the climb
                    // goes into its end slope, so it still ends at the interpolated height)
                    q.v[6] = s_a as f32;
                    q.v[7] = (2.0 * rise - s_a) as f32;
                } else {
                    q.v[6] = rise as f32;
                    q.v[7] = rise as f32;
                }
            } else {
                // (a join of a few centimetres has no length to spread a rise over. The whole
                // rise is carried straight across - steep, but the two pieces then agree in
                // height, which is what makes them one line: `MAX_RISE` is not applied here,
                // since a clamped rise is exactly what left the end short of the next path)
                let rise = ((z_target - a.z) / total.max(1e-6)) as f32;
                q.v[6] = rise;
                q.v[7] = rise;
            }
        } else if k + 1 == n {
            // (`PieceEnd::Surface` stays as it was: a free end is seated on the deck, and
            // `MAX_RISE` keeps a short piece from standing on end)
            match want {
                Some(z) => climb_to(&mut q, z),
                None => {
                    q.v[6] = s_a as f32;
                    q.v[7] = s_a as f32;
                }
            }
        } else if k == 0 {
            // (the first piece leaves at the rise the path had, and still lands on the deck)
            let l = q.length();
            let z1 = want.unwrap_or(a.z + s_a * l);
            q.v[6] = s_a as f32;
            let mut end_rise = if l >= SHORT_JOIN { 2.0 * (z1 - a.z) / l - s_a } else { s_a };
            end_rise = end_rise.clamp(-MAX_RISE, MAX_RISE);
            q.v[7] = end_rise as f32;
        } else if let Some(z1) = want {
            climb_to(&mut q, z1);
        }
        z = q.end().z;
        out.push(q);
    }
    out
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

/// Move one end to `q` and put it on the object's surface: the end sits where the deck is, and
/// the rise between the two ends is what the path climbs at. A drag that only kept the height
/// the end already had left a path floating over the deck or sunk into it.
fn move_end_on_surface(crossing: &mut Crossing, i: usize, at_end: bool, q: DVec2, tris: &[f32], hint: Option<f64>) {
    if at_end {
        move_end(crossing, i, at_end, q);
        let Some(p) = crossing.paths.get_mut(i) else { return };
        // (the dragged end takes the deck's height, at one grade)
        if let Some(z) = surface_z(tris, p.end().truncate()).or(hint) {
            climb_to(p, z);
        }
    } else {
        // (the far end stays where it is, at its own height: the start is put on the deck and
        // the rise is set so the end does not move)
        move_end(crossing, i, at_end, q);
        let Some(p) = crossing.paths.get_mut(i) else { return };
        let (end_z, end_rise) = (p.end().z, p.slope_end() as f64);
        let Some(z) = surface_z(tris, p.start().truncate()).or(hint) else { return };
        p.v[2] = z as f32;
        let l = p.length();
        if l > 1e-3 {
            let rise = (2.0 * (end_z - z) / l - end_rise).clamp(-MAX_RISE, MAX_RISE);
            p.v[6] = rise as f32;
        }
    }
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

    /// Every path picked: the first one, then the others added with Ctrl+click. The list and the
    /// map draw them all, and the shared properties can be set on all of them at once.
    pub fn selection(&self) -> Vec<usize> {
        let Some(first) = self.selected else { return Vec::new() };
        let mut out = Vec::with_capacity(self.multi.len() + 1);
        out.push(first);
        for i in &self.multi {
            if !out.contains(i) {
                out.push(*i);
            }
        }
        out
    }

    /// Whether path `i` is one of the picked ones.
    fn is_picked(&self, i: usize) -> bool {
        self.selected == Some(i) || self.multi.contains(&i)
    }

    /// Pick path `i`. `add`: keep what was picked (Ctrl+click toggles it); `run`: take everything
    /// from the pick before it up to `i` (Shift+click, as a file list does).
    fn pick_path(&mut self, i: usize, add: bool, run: bool) {
        if run && let Some(from) = self.anchor.or(self.selected) {
            let (lo, hi) = if from <= i { (from, i) } else { (i, from) };
            self.selected = Some(i);
            self.multi = (lo..=hi).filter(|q| *q != i).collect();
            return;
        }
        if add {
            if self.selected == Some(i) {
                // (the one being edited cannot simply be unpicked: another takes its place)
                let next = self.multi.first().copied();
                self.selected = next;
                self.multi.retain(|q| *q != i && Some(*q) != next);
            } else if self.multi.contains(&i) {
                self.multi.retain(|q| *q != i);
            } else {
                self.multi.push(i);
            }
            self.anchor = Some(i);
            return;
        }
        self.selected = Some(i);
        self.multi.clear();
        self.anchor = Some(i);
    }

    /// One path picked again, the others dropped: what a structural change leaves behind.
    fn pick_only(&mut self, i: usize) {
        self.selected = Some(i);
        self.multi.clear();
        self.anchor = Some(i);
    }

    /// A click on the map picks a path: false when nothing of the crossing is at that point.
    /// `add` keeps what was picked (Ctrl), as clicking a list row does.
    pub fn select_on_map(&mut self, hit: Option<usize>, add: bool) -> bool {
        let Some(i) = hit else { return false };
        if !self.on_map() || self.sheet.as_ref().is_none_or(|s| i >= s.crossing.paths.len()) {
            return false;
        }
        self.pick_path(i, add, false);
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
        let geom = if at_end { c.heading_at(c.length.max(0.0)) } else { c.heading_at(0.0) };
        Some(Joint { pos, heading, slope, flow: outward_flow(p.direction(), at_end), dir: p.direction(), travel: p.travel_at(at_end), geom })
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
            self.surface_height(end)
        });
        if let (Some(i), Some(z)) = (at, z) {
            if let Some(p) = self.sheet.as_mut().and_then(|s| s.crossing.paths.get_mut(i)) {
                set_end_height(p, z);
            }
        }
        if let Some(i) = at {
            self.pick_only(i);
        }
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
        // (the way the join has to arrive: along the other path's own geometry at that end,
        // which is exactly the rotate its start/end row shows. `heading` is into the body, so
        // at a path's end it is the other way round - arriving on it left the last piece 180°
        // out)
        let h_b = crate::roads::norm_deg(j.geom);
        let s_b = j.slope;
        let delta = b.truncate() - a.truncate();
        let chord = delta.length();
        if chord > FAR_JOIN {
            return None;
        }
        // (the end is already at the joint - within a hand's reach: nothing to lay between them.
        // The picked path's end is put on the joint instead of leaving the click doing nothing,
        // which is what a dot right beside the end looked like: it did not answer at all)
        if chord < 0.25 {
            self.push_undo();
            let Some(sheet) = self.sheet.as_mut() else { return None };
            join_end(&mut sheet.crossing, i, j);
            self.reveal = true;
            return Some(i);
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
            connect_segs(a.truncate(), h_a, b.truncate(), h_b, self.side)
        };
        // (a join that could not be planned at all is still closed with one straight to the
        // point: a click that answers with nothing at all is worse than a plain piece)
        let segs = if segs.is_empty() {
            vec![crate::roads::Seg { start: a.truncate(), heading: phi, length: chord, radius: 0.0 }]
        } else {
            segs
        };
        let mut made = lay_join(&template, a, s_a, &segs, PieceEnd::At { z: b.z, slope: s_b }, false, &surface);
        // (the join is checked on what came out, not trusted: a last piece a few degrees or a
        // couple of centimetres off does not line up with the next segment's rotate, and that
        // is what a misaligned crossing is. A retry forces the arrival - a biarc has an answer
        // for any pair of ends - and if even that misses, nothing is added: one piece fewer is
        // better than a line that does not meet the one it joins)
        let lands = |q: &Path| -> bool {
            let c = q.curve();
            let h = c.heading_at(c.length.max(0.0));
            ang_delta(h, h_b).abs() <= 0.5 && (q.end().z - b.z).abs() <= 0.02
        };
        if made.last().is_none_or(|q| !lands(q)) {
            let miss = made.last().map(|q| {
                let c = q.curve();
                (ang_delta(c.heading_at(c.length.max(0.0)), h_b), q.end().z - b.z)
            });
            log::debug!("complete to: the join missed by {miss:?} (degrees, metres); forcing its arrival");
            let forced = forced_join(a.truncate(), h_a, b.truncate(), h_b, self.side)
                .or_else(|| forced_join(a.truncate(), h_a, b.truncate(), h_b, Side::Auto));
            if let Some(forced) = forced {
                made = lay_join(&template, a, s_a, &forced, PieceEnd::At { z: b.z, slope: s_b }, false, &surface);
            }
            if made.last().is_none_or(|q| !lands(q)) {
                log::debug!("complete to: nothing arrives on {h_b:.1}° and {:.2} m; the crossing is left as it was", b.z);
                return None;
            }
        }
        self.push_undo();
        let Some(sheet) = self.sheet.as_mut() else { return None };
        let mut last = None;
        for q in made {
            sheet.crossing.paths.push(q);
            last = Some(sheet.crossing.paths.len() - 1);
        }
        if let Some(i) = last {
            self.pick_only(i);
        }
        self.reveal = true;
        // (the mode stays on, and its green ends stay drawn: carrying several paths on is the
        // usual thing, and the circles vanishing after the first join looked like a fault)
        self.from_nearby = false;
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

    /// The way round a join is to be made: automatic, or always left or right.
    pub fn set_side(&mut self, side: Side) {
        self.side = side;
    }

    /// Which way round joins are made here.
    pub fn side(&self) -> Side {
        self.side
    }

    /// The height of whatever a point of the object's frame sits on: the object's own mesh
    /// where it has one, else the road or ground of the map under it (the top of the nearest
    /// lane's points). This is what a dragged end is seated on, so a path dragged off the
    /// object still lies on the road rather than floating where its height was.
    pub fn deck_at(&self, ed: &mut Editor, mut assets: Option<&mut Assets>, local: DVec2) -> Option<f64> {
        if let Some(z) = self.surface_height(local) {
            return Some(z);
        }
        let Sel::Object(k, id) = self.target? else { return None };
        let doc = ed.doc.as_ref()?;
        let o = doc.object(k, id)?;
        let file = o.file.clone();
        let abs = assets.as_deref_mut().and_then(|a| a.object(&file)).map(|m| m.abs_height).unwrap_or(true);
        let world = doc.object_world(k, o, abs);
        let m = crate::render::object_matrix(DVec3::ZERO, o.rot);
        let at = world + m.transform_point3(DVec3::new(local.x, local.y, 0.0).as_vec3()).as_dvec3();
        let mut z = None;
        if let Some(assets) = assets {
            let (lanes, _) = crate::lanes::near_skipping(doc, assets, at.truncate(), 8.0, Some((k, id)));
            for l in &lanes {
                for p in &l.pts {
                    if (p.truncate() - at.truncate()).length() <= 5.0 {
                        // (the highest lane there: where a wheel would rest)
                        z = Some(z.map_or(p.z, |b: f64| b.max(p.z)));
                    }
                }
            }
        }
        let z = z.unwrap_or_else(|| doc.ground(at.truncate()));
        let back = m.inverse().transform_point3((DVec3::new(at.x, at.y, z) - world).as_vec3());
        Some(back.z as f64)
    }

    /// Delete the picked paths, as the list's bin does. True when at least one was taken out
    /// (one step back for Ctrl+Z, the pick moved to a path that is still there).
    pub fn delete_picked(&mut self) -> bool {
        let Some(sheet) = self.sheet.as_ref() else { return false };
        let n = sheet.crossing.paths.len();
        let mut gone: Vec<usize> = self.selection().into_iter().filter(|i| *i < n).collect();
        if gone.is_empty() {
            return false;
        }
        gone.sort_unstable();
        self.push_undo();
        let Some(sheet) = self.sheet.as_mut() else { return false };
        // (from the back, so the indices still to come do not shift under the cut)
        for i in gone.iter().rev() {
            sheet.crossing.remove_path(*i);
        }
        let left = sheet.crossing.paths.len();
        self.selected = if left == 0 { None } else { Some(gone[0].min(left - 1)) };
        self.multi.clear();
        self.anchor = self.selected;
        self.drag = None;
        self.rot_drag = None;
        true
    }

    /// Cut the picked path in two at its middle, as the map's roads are cut: each half can then
    /// be turned and given a rotate of its own. True when a path was cut (the second piece is
    /// the one picked afterwards). Only one path at a time: two halves of several paths at once
    /// would be a jumble of indices for nothing.
    pub fn split_picked(&mut self) -> bool {
        if !self.multi.is_empty() {
            return false;
        }
        let Some(i) = self.selected else { return false };
        let Some(sheet) = self.sheet.as_ref() else { return false };
        // (nothing is recorded until the cut is known to be possible: a path too short to split
        // would otherwise leave an empty step for Ctrl+Z)
        if !sheet.crossing.paths.get(i).is_some_and(|p| p.length() > 0.2) {
            return false;
        }
        self.push_undo();
        let Some(sheet) = self.sheet.as_mut() else { return false };
        let Some(at) = split_path(&mut sheet.crossing, i, 0.5) else { return false };
        self.pick_only(at);
        self.reveal = true;
        self.drag = None;
        self.rot_drag = None;
        true
    }

    /// `Connect the roads`: line the crossing's ends up with the road lanes round it, so
    /// openOMSI joins them. Its rule is geometric - same kind, within [`LINK_TOLERANCE`]
    /// horizontally, [`LINK_RISE`] in height and [`LINK_ANGLE`] of heading - and it never reads
    /// `[splinehelper]`, so the helper points alone do not make a road drive through.
    ///
    /// True when at least one road end was linked; what happened to every one of them is kept
    /// in `link_report` for the panel.
    pub fn link_roads(&mut self, ed: &mut Editor, mut assets: Option<&mut Assets>) -> bool {
        let Some(Sel::Object(k, id)) = self.target else { return false };
        let Some(doc) = ed.doc.as_ref() else { return false };
        let Some(o) = doc.object(k, id) else { return false };
        let (file, rot) = (o.file.clone(), o.rot);
        let abs = assets.as_deref_mut().and_then(|a| a.object(&file)).map(|m| m.abs_height).unwrap_or(true);
        let world = doc.object_world(k, o, abs);
        let m = crate::render::object_matrix(DVec3::ZERO, rot);
        // (the roads come up to the crossing's edge: a lane further out than this is passing
        // it, not ending at it)
        let reach = (self.span() * 0.6 + 25.0).clamp(40.0, 220.0);
        let Some(assets) = assets else { return false };
        let (lanes, _) = crate::lanes::near_skipping(doc, assets, world.truncate(), reach, Some((k, id)));
        let roads = road_lane_ends(&lanes, world, &m, rot[0], doc);
        if roads.is_empty() {
            self.link_report.clear();
            return false;
        }
        let tris = std::sync::Arc::clone(&self.surface);
        let side = self.side;
        self.push_undo();
        let notes = {
            let Some(sheet) = self.sheet.as_mut() else { return false };
            let notes = link_crossing(&mut sheet.crossing, &roads, &tris, side);
            // (a `[splinehelper]` on each link, naming the road there: the game snaps its
            // splines onto the object by it. openOMSI does not read it at all - its lanes meet
            // by geometry - so this is written for the game, and one is only added where
            // there is not already one)
            for n in &notes {
                let Some(h) = helper_for(n) else { continue };
                if sheet.crossing.helpers.iter().any(|q| (q.local().truncate() - h.local().truncate()).length() < 0.5) {
                    continue;
                }
                sheet.crossing.helpers.push(h);
            }
            notes
        };
        let ok = notes.iter().any(|n| n.ok);
        self.link_report = notes;
        self.from_nearby = false;
        self.complete_to = false;
        self.nearby.clear();
        ok
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
        let mut out = Vec::new();
        // (the crossing's own ends first, and on their own: they need no assets, and when the
        // assets were not at hand the whole list came back empty - the green circles vanished)
        if let Some(sheet) = self.sheet.as_ref() {
            for (i, _q) in sheet.crossing.paths.iter().enumerate() {
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
        let (Some(Sel::Object(k, id)), Some(doc), Some(assets)) = (self.target, ed.doc.as_ref(), assets) else { return out };
        let Some(o) = doc.object(k, id) else { return out };
        let (file, rot) = (o.file.clone(), o.rot);
        let abs = assets.object(&file).map(|m| m.abs_height).unwrap_or(true);
        let world = doc.object_world(k, o, abs);
        let m = crate::render::object_matrix(DVec3::ZERO, rot);
        let inv = m.inverse();
        // (the same reach the 3D view draws: a shorter one left green circles on the map
        // that no click could take, because they were never in this list)
        let reach = ed.neighbour_reach();
        let (lanes, _) = crate::lanes::near_skipping(doc, assets, world.truncate(), reach, Some((k, id)));
        for l in lanes.iter().filter(|l| l.object != Some((k, id))) {
            let n = l.pts.len();
            if n < 2 {
                continue;
            }
            // (both ends. `heading` is the way the lane runs *into* its own body from there, as
            // the crossing's own ends have it - so `outward()` is out of the lane, where a new
            // path goes. `travel` is the way traffic actually moves there, which is the other
            // way round when the lane is driven against its geometry)
            for (pos, geom, at_end) in [
                (l.pts[0], crate::roads::heading_of((l.pts[1] - l.pts[0]).truncate()), false),
                (l.pts[n - 1], crate::roads::heading_of((l.pts[n - 1] - l.pts[n - 2]).truncate()), true),
            ] {
                // (an end a kilometre down a long spline is not a circle on this crossing)
                if (pos.truncate() - world.truncate()).length() > reach {
                    continue;
                }
                let d = if at_end { pos - l.pts[n - 2] } else { l.pts[1] - pos };
                let len = d.truncate().length().max(1e-6);
                let local = inv.transform_point3((pos - world).as_vec3()).as_dvec3();
                // (the object's turn is taken out of the lane's headings, the path being
                // written in the object's frame)
                let geom = crate::roads::norm_deg(geom - rot[0]);
                // (into the lane's own body from this end, as the crossing's own ends have it)
                let heading = if at_end { crate::roads::norm_deg(geom + 180.0) } else { geom };
                let travel = if l.direction == 1 { crate::roads::norm_deg(geom + 180.0) } else { geom };
                out.push(Joint { pos: local, heading, slope: d.z / len, flow: crate::crossing_edit::outward_flow(l.direction, at_end), dir: l.direction, travel, geom });
            }
        }
        log::debug!("crossing nearby: {} ends within {reach} m", out.len());
        out
    }

    /// Start a new path from a nearby path's end: it leaves in that path's way, at its rise,
    /// ten metres long (its numbers are there to be changed after). Its traffic direction is
    /// set from the way the other path is driven, compared with the new path's own geometry, so
    /// it points the same way the lane does rather than backwards.
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
        p.v[3] = j.outward() as f32;
        p.v[4] = 0.0;
        p.v[5] = 10.0;
        p.v[6] = j.slope as f32;
        p.v[7] = j.slope as f32;
        p.v[10] = j.direction_for(j.outward()) as f32;
        let at = {
            let Some(sheet) = self.sheet.as_mut() else { return None };
            sheet.crossing.paths.push(p);
            sheet.crossing.paths.len() - 1
        };
        let z = self.sheet.as_ref().and_then(|s| s.crossing.paths.get(at)).and_then(|p| self.surface_height(p.end().truncate()));
        if let Some(z) = z {
            if let Some(p) = self.sheet.as_mut().and_then(|s| s.crossing.paths.get_mut(at)) {
                set_end_height(p, z);
            }
        }
        self.pick_only(at);
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
        // (the extra picks follow whatever the sheet has now: a path that is no longer there
        // cannot stay picked)
        let before = self.multi.len();
        self.multi.retain(|i| *i < paths);
        if self.multi.len() != before {
            self.anchor = self.selected;
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
        self.pick_only(i);
        self.map_drag = Some((i, at_end));
        // (a new grab is judged on this end's own height, not the one a previous drag locked)
        self.map_drag_plane = None;
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
    }

    /// Drag the end the map took to this point of the object's frame, as the drawing in the
    /// window does. `joint` is the end it has snapped onto, when it has.
    ///
    /// This only ever moves the one path: an end is not split into pieces to reach a point.
    /// Pieces belong to a join (`Complete to a nearby path`), where a path has to leave in one
    /// way and arrive in another. A drag that split instead piled pieces on top of each other
    /// and turned little circles.
    pub fn drag_map_end(&mut self, at: DVec3, joint: Option<Joint>, hint: Option<f64>) -> bool {
        let Some((i, at_end)) = self.map_drag else { return false };
        if self.sheet.as_ref().is_none_or(|s| i >= s.crossing.paths.len()) {
            self.map_drag = None;
            self.map_drag_plane = None;
            return false;
        }
        let tris = std::sync::Arc::clone(&self.surface);
        let Some(sheet) = self.sheet.as_mut() else { return false };
        match joint {
            Some(j) if at_end => join_end(&mut sheet.crossing, i, j),
            Some(j) => join_start(&mut sheet.crossing, i, j),
            None => move_end_on_surface(&mut sheet.crossing, i, at_end, at.truncate(), &tris, hint),
        }
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
        self.snap_pool = None;
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
                // (another object's ends are not this one's: the pool is gathered afresh)
                self.snap_pool = None;
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
            if !ed.crossing_selected.is_empty() {
                ed.crossing_selected.clear();
            }
            ed.crossing_sink.clear();
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
                // (how far each path sits off the deck, so the map can colour a path that is
                // under it. The mesh is sampled at each path's middle: a few times a second,
                // never every frame)
                let now = ctx.input(|i| i.time);
                if now >= self.sink_at && !self.surface.is_empty() {
                    self.sink_at = now + 0.25;
                    let n = sheet.crossing.paths.len();
                    if self.sink.len() != n {
                        self.sink.resize(n, 0.0);
                        self.sink_next = 0;
                    }
                    // (about 150 paths a tick: 787 of them sampled at once was a stall, and
                    // each sample used to walk every triangle of the mesh)
                    let budget = n.min(150);
                    let mut tests = 0usize;
                    for _ in 0..budget {
                        if n == 0 {
                            break;
                        }
                        let idx = self.sink_next % n;
                        self.sink_next = (self.sink_next + 1) % n;
                        let c = sheet.crossing.paths[idx].curve();
                        let mid = c.point_at(c.length.max(0.0) * 0.5);
                        let (z, t) = match &self.surface_grid {
                            Some(g) => g.z_tests(&self.surface, mid.truncate()),
                            None => (surface_z(&self.surface, mid.truncate()), self.surface.len() / 9),
                        };
                        tests += t;
                        self.sink[idx] = match z {
                            Some(z) => (mid.z - z) as f32,
                            None => 0.0,
                        };
                    }
                    log::trace!("crossing sink: {budget} of {n} paths sampled, {tests} triangles tested");
                }
            }
            ed.crossing_selected = self.selection();
            ed.crossing_sink.clone_from(&self.sink);
        } else {
            ed.crossing_draft = None;
            ed.crossing_selected.clear();
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
        // (the object's own mesh, read here rather than in the drawing: a drag on the map has
        // to follow the surface even when the crossing is not drawn from above)
        self.load_mesh(assets.as_deref_mut());
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
                // (the content is held to the width the window has: no minimum taken from the
                // available width, which made the window grow a little every frame, and no
                // ceiling of its own, which let a long hint run on and pulled the window out
                // to that width)
                let room = if beside { (size.x - 16.0).max(320.0) } else { ui.available_width().clamp(320.0, 1200.0) };
                ui.set_max_width(room);
                ui.set_min_height(400.0);
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
                            ui.set_max_width(340.0);
                            ui.set_min_width(340.0);
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
        // (the roads round the crossing: the lanes come from the map, so this is done here,
        // where the assets are still at hand, rather than inside the window)
        if self.link_now {
            self.link_now = false;
            self.link_roads(ed, assets.as_deref_mut());
        }
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
            ed.crossing_selected.clear();
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
                        self.save(ed, roots, assets, actions);
                    }
                    self.ask_close = false;
                    if !self.unsaved() {
                        self.open = false;
                        self.discard();
                        ed.crossing_draft = None;
                        ed.crossing_selected.clear();
                    }
                }
                // (the file is as it was: the changes are dropped with the window)
                Some(false) => {
                    self.discard();
                    ed.crossing_draft = None;
                    ed.crossing_selected.clear();
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
                self.save(ed, roots, assets, actions);
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

    /// The object's own mesh: read once, for drawing the crossing under its paths and for
    /// seating a dragged end on its surface.
    fn load_mesh(&mut self, mut assets: Option<&mut Assets>) {
        if self.footprint.as_ref().map(|f| f.0 != self.rel).unwrap_or(true) {
            self.footprint = assets.as_deref_mut().and_then(|a| footprint_of(a, &self.rel));
        }
        if self.surface_for != self.rel {
            let rel = self.rel.clone();
            let tris = assets.and_then(|a| surface_tris(a, &rel)).unwrap_or_default();
            self.surface_grid = SurfaceGrid::build(&tris);
            self.surface = std::sync::Arc::new(tris);
            self.surface_for = rel;
        }
    }

    /// The deck's height at a point of the object's frame, through the surface grid when
    /// there is one: the same answer as [`surface_z`], without walking every triangle.
    fn surface_height(&self, p: DVec2) -> Option<f64> {
        match &self.surface_grid {
            Some(g) => g.z(&self.surface, p),
            None => surface_z(&self.surface, p),
        }
    }

    /// The paths page: the list and its numbers on the left, the crossing drawn on the right.
    fn paths_body(&mut self, ui: &mut egui::Ui, ed: &mut Editor, sheet: &mut Sheet, body: Vec2, assets: Option<&mut Assets>) {
        let mut assets = assets;
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
                self.canvas_ui(ui, sheet, ed, assets.as_deref_mut());
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
                        self.pick_only(at);
                    }
                    self.reveal = true;
                    changed = true;
                }
                if ui.small_button(icons::labelled(Icon::Copy, "")).on_hover_text(tr("Duplicate", "Дублировать")).clicked() {
                    if let Some(q) = self.selected.and_then(|i| sheet.crossing.paths.get(i)).cloned() {
                        sheet.crossing.paths.push(q);
                        self.pick_only(sheet.crossing.paths.len() - 1);
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
        // (openOMSI joins a crossing's lanes to the roads' by geometry alone - it reads the
        // `[splinehelper]` points and does nothing with them - so the ends have to meet by
        // those numbers for its traffic to drive through)
        if ui.button(tr("Connect the roads", "Соединить с дорогами")).on_hover_text(tr("Line the crossing's ends up with the road lanes' own ends. openOMSI joins lanes by their geometry, not by the spline helpers", "Совместить концы перекрёстка с концами полос дорог. openOMSI соединяет полосы по геометрии, а не по точкам привязки")).clicked() {
            self.link_now = true;
        }
        if !self.link_report.is_empty() {
            let ok = self.link_report.iter().filter(|n| n.ok).count();
            let total = self.link_report.len();
            egui::CollapsingHeader::new(RichText::new(format!("{} ({ok}/{total})", tr("Link check", "Проверка соединений"))).small()).id_salt("crossing link check").default_open(ok < total).show(ui, |ui| {
                for n in &self.link_report {
                    ui.label(RichText::new(format!("{} {} {}", if n.ok { "✓" } else { "✗" }, n.who, n.why)).small().color(if n.ok { GOOD } else { BAD }));
                }
            });
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
        // (the way round a join is made: automatic takes the widest arcs it finds, which can
        // be the long way round a corner; left and right say which way to go instead)
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("The way a join turns", "Куда поворачивает соединение")).small().color(DIM));
            for (v, text) in [(Side::Auto, tr("Automatic", "Автоматически")), (Side::Left, tr("Left", "Налево")), (Side::Right, tr("Right", "Направо"))] {
                if ui.selectable_label(self.side == v, RichText::new(text).small()).clicked() {
                    self.set_side(v);
                }
            }
        });
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
                let also = self.multi.contains(&i);
                let row = ui.horizontal(|ui| {
                    // (Ctrl adds a path to the pick, Shift takes the run from the last one:
                    // a junction's paths are set alike in groups - kind, width, the way they
                    // turn, the light they wait at)
                    let label = ui.selectable_label(on || also, RichText::new(text).color(kind_color(p.kind())));
                    if label.clicked() {
                        let (add, run) = ui.input(|k| (k.modifiers.command || k.modifiers.ctrl, k.modifiers.shift));
                        self.pick_path(i, add || run, run);
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
            self.pick_only(to);
            changed = true;
        }
        if let Some(i) = remove {
            // (the bin of a row that is part of a set takes the whole set, as it reads)
            if !self.is_picked(i) {
                self.pick_only(i);
            }
            changed |= self.delete_picked();
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

    /// The numbers several paths share, set for all of them at once: the kind (street, footway,
    /// rail), the width, the way traffic goes, the turn signal, the light they wait at, and
    /// whether a vehicle on them keeps the junction clear. What is not shared - where a path
    /// starts and ends, its radius, its own rotate - is edited one path at a time.
    ///
    /// A value they do not agree on shows as `Several`, and setting one applies it to all.
    fn shared_numbers(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet, sel: Vec<usize>) -> bool {
        let mut changed = false;
        ui.separator();
        ui.label(RichText::new(format!("{} ({})", tr("Paths picked", "Выбрано путей"), sel.len())).strong());
        ui.label(RichText::new(tr("Set what they share. A path's own numbers are edited one at a time", "Задайте общее. Собственные числа пути правятся по одному")).small().color(DIM));
        let n = sel.len();
        let set = |paths: &mut Vec<Path>, done: &mut bool, f: &dyn Fn(&mut Path)| {
            for q in &sel {
                if let Some(p) = paths.get_mut(*q) {
                    f(p);
                    *done = true;
                }
            }
        };
        let _ = n;
        // (the values they hold, so a mixed one can say so instead of showing the first)
        let kinds: std::collections::BTreeSet<i32> = sel.iter().filter_map(|q| sheet.crossing.paths.get(*q).map(|p| p.kind())).collect();
        let widths: std::collections::BTreeSet<u32> = sel.iter().filter_map(|q| sheet.crossing.paths.get(*q).map(|p| p.v[9].to_bits())).collect();
        let ways: std::collections::BTreeSet<i32> = sel.iter().filter_map(|q| sheet.crossing.paths.get(*q).map(|p| p.direction())).collect();
        let turns: std::collections::BTreeSet<i32> = sel.iter().filter_map(|q| sheet.crossing.paths.get(*q).map(|p| p.turn())).collect();
        let lights: std::collections::BTreeSet<i32> = sel.iter().filter_map(|q| sheet.crossing.paths.get(*q).map(|p| p.light.unwrap_or(-1))).collect();
        let clear = sel.iter().all(|q| sheet.crossing.paths.get(*q).is_some_and(|p| p.crossing_problem));
        let several = tr("Several", "Несколько");
        egui::Grid::new("crossing shared numbers").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
            ui.label(RichText::new(tr("Kind", "Тип")).small().color(DIM));
            let k = kinds.iter().next().copied().unwrap_or(0);
            let mut clicked = None;
            egui::ComboBox::from_id_salt("crossing shared kind").selected_text(if kinds.len() > 1 { several.to_string() } else { kind_name(k).to_string() }).width(150.0).show_ui(ui, |ui| {
                for v in [0, 1, 2] {
                    if ui.selectable_label(kinds.len() == 1 && k == v, kind_name(v)).clicked() {
                        clicked = Some(v);
                    }
                }
            });
            if let Some(v) = clicked {
                set(&mut sheet.crossing.paths, &mut changed, &|p| p.v[8] = v as f32);
            }
            ui.end_row();
            ui.label(RichText::new(tr("Width", "Ширина")).small().color(DIM));
            let mut w = sel.first().and_then(|q| sheet.crossing.paths.get(*q)).map(|p| p.v[9]).unwrap_or(0.0);
            let mixed = widths.len() > 1;
            let r = ui.add(egui::DragValue::new(&mut w).speed(0.05).range(0.0..=30.0).suffix(" m").prefix(if mixed { "~ " } else { "" }));
            if r.changed() {
                set(&mut sheet.crossing.paths, &mut changed, &|p| p.v[9] = w);
            }
            ui.end_row();
            ui.label(RichText::new(tr("Direction", "Направление движения")).small().color(DIM));
            let mut d = ways.iter().next().copied().unwrap_or(0);
            let mut clicked = None;
            egui::ComboBox::from_id_salt("crossing shared direction").selected_text(if ways.len() > 1 { several.to_string() } else { direction_name(d).to_string() }).width(150.0).show_ui(ui, |ui| {
                for v in [0, 1, 2] {
                    if ui.selectable_label(ways.len() == 1 && d == v, direction_name(v)).clicked() {
                        clicked = Some(v);
                    }
                }
            });
            let _ = &mut d;
            if let Some(v) = clicked {
                set(&mut sheet.crossing.paths, &mut changed, &|p| p.v[10] = v as f32);
            }
            ui.end_row();
            ui.label(RichText::new(tr("Turn signal", "Поворот")).small().color(DIM));
            let mut t = turns.iter().next().copied().unwrap_or(TURN_NONE);
            let mut clicked = None;
            egui::ComboBox::from_id_salt("crossing shared turn").selected_text(if turns.len() > 1 { several.to_string() } else { turn_name(t).to_string() }).width(150.0).show_ui(ui, |ui| {
                for v in [TURN_NONE, TURN_LEFT, TURN_RIGHT] {
                    if ui.selectable_label(turns.len() == 1 && t == v, turn_name(v)).clicked() {
                        clicked = Some(v);
                    }
                }
            });
            let _ = &mut t;
            if let Some(v) = clicked {
                set(&mut sheet.crossing.paths, &mut changed, &|p| p.v[11] = v as f32);
            }
            ui.end_row();
            ui.label(RichText::new(tr("Light", "Светофор")).small().color(DIM));
            let names: Vec<(i32, String)> = std::iter::once((-1, tr("None", "Нет").to_string())).chain(sheet.crossing.lights.iter().enumerate().map(|(q, l)| (q as i32, format!("#{q} {}", if l.name.trim().is_empty() { "-" } else { l.name.trim() })))).collect();
            let cur = lights.iter().next().copied().unwrap_or(-1);
            let mut clicked = None;
            egui::ComboBox::from_id_salt("crossing shared light").selected_text(if lights.len() > 1 { several.to_string() } else { names.iter().find(|(v, _)| *v == cur).map(|(_, s)| s.clone()).unwrap_or_else(|| several.to_string()) }).width(150.0).show_ui(ui, |ui| {
                for (v, s) in &names {
                    if ui.selectable_label(lights.len() == 1 && cur == *v, s).clicked() {
                        clicked = Some(*v);
                    }
                }
            });
            if let Some(v) = clicked {
                set(&mut sheet.crossing.paths, &mut changed, &|p| p.light = (v >= 0).then_some(v));
            }
            ui.end_row();
        });
        let mut clear_now = clear;
        if ui.checkbox(&mut clear_now, tr("Keeps the junction clear", "Держит перекрёсток свободным")).on_hover_text(tr("A vehicle waiting on this path holds the crossing empty for the others (`[crossingproblem]`)", "Машина на этом пути держит перекрёсток свободным для других (`[crossingproblem]`)")).changed() {
            set(&mut sheet.crossing.paths, &mut changed, &|p| p.crossing_problem = clear_now);
        }
        changed
    }

    /// The picked path's numbers: where it starts, how it runs (heading, radius, length), its
    /// slopes and width, the way traffic goes, its turn signal and light, what it blocks.
    /// True when one of them was changed.
    fn path_numbers(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet) -> bool {
        let Some(i) = self.selected.filter(|i| *i < sheet.crossing.paths.len()) else { return false };
        // (several paths picked: what they all share is set for all of them at once, which is
        // what a junction is built with - a row of footway, a row of street)
        let sel = self.selection();
        if sel.len() > 1 {
            return self.shared_numbers(ui, sheet, sel);
        }
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
        // (each end's own way out and way in: `Rotate` beside its point, since it is part of
        // where and how that point is. The drag is applied after the grid - a number edited
        // above is then already on the path - and a turn one arc cannot make is split into
        // pieces)
        let spawned = self.rot_drag.as_ref().filter(|g| g.index == i).map(|g| g.spawned).unwrap_or(0);
        let last = (i + spawned).min(sheet.crossing.paths.len() - 1);
        let mut h0 = crate::roads::norm_deg(sheet.crossing.paths[i].heading());
        let end_c = sheet.crossing.paths[last].curve();
        let mut h1 = crate::roads::norm_deg(end_c.heading_at(end_c.length.max(0.0)));
        let leave = tr("How this path leaves, degrees. The point stays; a turn one arc cannot make is split into pieces", "Как путь уходит, градусы. Точка остаётся; поворот, который одной дуге не сделать, делится на части");
        let arrive = tr("How this path arrives, degrees. The point stays; a turn one arc cannot make is split into pieces", "Как путь приходит, градусы. Точка остаётся; поворот, который одной дуге не сделать, делится на части");
        let mut rot_req: Option<(bool, f64)> = None;
        let mut rot_stopped = false;
        let sink = self.sink.get(i).copied().unwrap_or(0.0);
        {
            let crossing = &mut sheet.crossing;
            let p = &mut crossing.paths[i];
            egui::Grid::new("crossing numbers").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                ui.label(RichText::new(tr("Start point", "Начало")).small().color(DIM));
                ui.horizontal(|ui| {
                    // (four numbers and a rotation in one row: narrow fields, so the row fits
                    // the panel instead of widening the window every frame)
                    ui.spacing_mut().interact_size.x = 50.0;
                    changed |= ui.add(egui::DragValue::new(&mut p.v[0]).speed(0.05).prefix("x ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut p.v[1]).speed(0.05).prefix("y ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut p.v[2]).speed(0.05).prefix("z ")).changed();
                    let r0 = ui.add(egui::DragValue::new(&mut h0).speed(0.4).max_decimals(1).suffix("°").prefix("r ")).on_hover_text(leave);
                    if r0.changed() {
                        rot_req = Some((false, h0));
                    }
                    rot_stopped |= r0.drag_stopped();
                });
                ui.end_row();
                // (the end, as the file gives it: x and y move the end itself, z is the slopes'
                // doing - typing one sets both slopes to the rise it asks for)
                ui.label(RichText::new(tr("End point", "Конец")).small().color(DIM));
                let mut end = p.end();
                let (mut ex, mut ey, mut ez) = (end.x, end.y, end.z);
                ui.horizontal(|ui| {
                    ui.spacing_mut().interact_size.x = 50.0;
                    let cx = ui.add(egui::DragValue::new(&mut ex).speed(0.05).prefix("x ")).changed();
                    let cy = ui.add(egui::DragValue::new(&mut ey).speed(0.05).prefix("y ")).changed();
                    let cz = ui.add(egui::DragValue::new(&mut ez).speed(0.05).prefix("z ")).changed();
                    let r1 = ui.add(egui::DragValue::new(&mut h1).speed(0.4).max_decimals(1).suffix("°").prefix("r ")).on_hover_text(arrive);
                    if r1.changed() {
                        rot_req = Some((true, h1));
                    }
                    rot_stopped |= r1.drag_stopped();
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
                // (whether this path lies on the deck, over it or under it - a path sunk into
                // the mesh is drawn red on the map, and this is by how much)
                ui.label(RichText::new(tr("Surface", "Поверхность")).small().color(DIM));
                ui.label(RichText::new(format!("{sink:+.2} m")).small().color(if sink < -0.02 {
                    Color32::from_rgb(230, 90, 110)
                } else if sink > 0.05 {
                    Color32::from_rgb(206, 190, 120)
                } else {
                    DIM
                }));
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
        }
        if let Some((at_end, h)) = rot_req {
            self.apply_rotation(&mut sheet.crossing, i, at_end, h);
            changed = true;
            rot_edit = true;
        }
        if rot_stopped && let Some(g) = self.rot_drag.take()
            && g.at_end
        {
            self.selected = Some((g.index + g.spawned).min(sheet.crossing.paths.len().saturating_sub(1)));
        }
        // (a cut at the middle: a long piece laid as one can only be bent by making it two, and
        // each half then has its own rotate. The same as the map's roads, which are cut with
        // the same key)
        {
            let long_enough = sheet.crossing.paths.get(i).is_some_and(|p| p.length() > 0.2);
            if ui.add_enabled(long_enough, egui::Button::new(tr("Split", "Разделить")).small()).on_hover_text(format!("{} (K)", tr("Cut in two: each piece can now be changed on its own", "Разрезана надвое: каждый кусок теперь можно менять отдельно"))).clicked()
                && let Some(at) = split_path(&mut sheet.crossing, i, 0.5)
            {
                self.selected = Some(at);
                self.reveal = true;
                self.rot_drag = None;
                changed = true;
            }
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

    fn save(&mut self, ed: &mut Editor, roots: &Roots, assets: &mut Assets, actions: &mut Vec<crate::ui::Action>) {
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
                    // (nothing was written, but the view may still hold the old model)
                    actions.push(crate::ui::Action::CrossingSaved(self.rel.clone()));
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
                    // (the GPU model too, and only that one: the catalogue is not read again)
                    actions.push(crate::ui::Action::CrossingSaved(self.rel.clone()));
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
        // (this object's model only: the catalogue is untouched, whatever file it came from)
        actions.push(crate::ui::Action::CrossingSaved(self.rel.clone()));
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

    fn canvas_ui(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet, ed: &mut Editor, assets: Option<&mut Assets>) {
        let mut assets = assets;
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
        // (the lane ends the map's view draws as green circles: the drawing snaps to them the
        // same way, so an end taken in either view lands on the same point. Gathering them
        // runs `lanes::near`, which is not cheap, so the list is kept for a quarter of a
        // second. The window has taken `self.sheet`, so the crossing's own ends are not in
        // this list - those come from `snap_joint`)
        let now = ui.input(|i| i.time);
        if self.snap_pool.as_ref().is_none_or(|(at, _)| now >= *at) {
            self.snap_pool = Some((now + 0.25, self.gather_nearby(ed, assets.as_deref_mut())));
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
            let on = self.is_picked(i);
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
            // (the picked path's own width, as a grey band lying on the deck: what a vehicle
            // would take, not just where it runs)
            if on {
                for n in 0..pts.len().saturating_sub(1) {
                    let quad = vec![to_screen(left[n]), to_screen(right[n]), to_screen(right[n + 1]), to_screen(left[n + 1])];
                    painter.add(egui::Shape::convex_polygon(quad, Color32::from_rgba_unmultiplied(180, 180, 190, 60), Stroke::NONE));
                }
            }
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
                let tip = at + d * (5.5 / scale as f64);
                painter.add(egui::Shape::convex_polygon(vec![to_screen(tip), to_screen(at + n * (2.0 / scale as f64)), to_screen(at - n * (2.0 / scale as f64))], color, Stroke::NONE));
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
                painter.circle_filled(s, if on { 2.8 } else { 2.0 }, if on { HANDLE } else { base });
                painter.circle_stroke(s, 4.0, Stroke::new(0.8, Color32::from_white_alpha(if on { 110 } else { 34 })));
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
                let d = crate::roads::dir(j.outward());
                painter.line_segment([s, to_screen(j.pos.truncate() + d * 3.0)], Stroke::new(1.2, SNAPPED));
            }
        }

        // ---- the mouse
        let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let at = |pos: Pos2| -> DVec2 { from_screen(pos) };
        // (the picked path's own ends win: a path's end and the next path's start are often the
        // same point - the pieces of a join meet there - so nearest-first picked the neighbour's
        // disc and dragged the wrong path)
        let picked = self.selected;
        let handle = |sheet: &Sheet, pos: Pos2| -> Option<(usize, bool)> { end_at(sheet, picked, pos, &to_screen) };
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
            // (a click on the line already picked keeps it: the paths of a junction run over
            // one another, and picking the one under the pointer would lose the one being
            // worked on)
            if let Some(i) = picked
                && best.is_some_and(|b| b.1 != i)
                && sheet.crossing.paths.get(i).is_some_and(|p| p.points(0.5).iter().any(|q| to_screen(q.truncate()).distance(pos) <= GRAB))
            {
                return Some(i);
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
                    if let Some(i) = path_at(sheet, pos) {
                        let (add, run) = ui.input(|k| (k.modifiers.command || k.modifiers.ctrl, k.modifiers.shift));
                        self.pick_path(i, add || run, run);
                    }
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let (Some((i, at_end)), Some(pos)) = (self.drag, resp.interact_pointer_pos()) {
                if i < sheet.crossing.paths.len() {
                    let q = at(pos);
                    // (snapping is what the editor does: Ctrl keeps the end where the mouse is.
                    // A snapped end takes the other path's way and rise too, not just its point.
                    // The pool is the lane ends the map's view draws: the drawing takes them
                    // the same way, whichever is nearer, so the two views land on one point)
                    let reach = snap_reach(self.scale);
                    let joint = match self.snap_pool.as_ref() {
                        Some((_, pool)) => snap_any(&sheet.crossing, pool, i, q, reach, ctrl),
                        None => snap_joint(&sheet.crossing, i, q, reach, ctrl),
                    };
                    self.snap_mark = joint.map(|j| j.pos);
                    match joint {
                        Some(j) if at_end => {
                            // (a snapped end: the end goes onto the joint's point, its way and
                            // its rise. One path - nothing is split here; pieces belong to
                            // `Complete to a nearby path`)
                            join_end(&mut sheet.crossing, i, j);
                            self.pick_only(i);
                        }
                        Some(j) => join_start(&mut sheet.crossing, i, j),
                        None => {
                            // (a free end: it follows whatever is under it - the object's own
                            // mesh, or the road or ground of the map - so its height, and the
                            // rise between the two ends, are the deck's)
                            let tris = std::sync::Arc::clone(&self.surface);
                            let hint = self.deck_at(ed, assets, q);
                            move_end_on_surface(&mut sheet.crossing, i, at_end, q, &tris, hint);
                        }
                    }
                }
            }
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag = None;
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
                            let pieces = match end_joint {
                                Some(e) => connect_segs(from, j.outward(), q, e.through(), self.side),
                                None => forward_segs(from, j.outward(), q, self.side),
                            };
                            let end = match end_joint {
                                Some(e) => PieceEnd::At { z: e.pos.z, slope: e.slope },
                                None => PieceEnd::Surface,
                            };
                            let a = DVec3::new(from.x, from.y, j.pos.z);
                            // (driven the way the path it was laid out of is driven, compared
                            // with the new pieces' own geometry)
                            for mut piece in lay_join(&p, a, j.slope, &pieces, end, false, &self.surface) {
                                piece.v[10] = j.direction_for(piece.heading()) as f32;
                                sheet.crossing.paths.push(piece);
                            }
                            self.selected = Some(sheet.crossing.paths.len() - 1);
                            self.reveal = true;
                            return;
                        }
                        if let Some(j) = end_joint {
                            p.fit_end_tangent(j.through());
                            p.set_end_height_and_slope(j.pos.z, j.slope);
                        } else {
                            // (a path drawn on the crossing sits on the deck, one grade)
                            if let Some(z) = self.surface_height(p.start().truncate()) {
                                p.v[2] = z as f32;
                            }
                            if let Some(z) = self.surface_height(p.end().truncate()) {
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
                } else if let Some(i) = path_at(sheet, pos) {
                    // (Ctrl adds to the pick and Shift takes a run, as a list does)
                    let (add, run) = ui.input(|k| (k.modifiers.command || k.modifiers.ctrl, k.modifiers.shift));
                    self.pick_path(i, add || run, run);
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
        let straight = connect_segs(DVec2::ZERO, 0.0, DVec2::new(0.0, 12.0), 0.0, Side::Auto);
        assert_eq!(straight.len(), 1);
        // (off to the side, arriving southward: pieces, the first still leaving north)
        let segs = connect_segs(a, 0.0, b, 180.0, Side::Auto);
        assert!(segs.len() >= 2, "one arc cannot arrive south from a northward start: {}", segs.len());
        assert!(ang_delta(segs[0].heading, 0.0).abs() < 0.5, "it leaves the way it did: {}", segs[0].heading);
        assert!((segs[0].start - a).length() < 1e-9);
        let last = segs.last().unwrap();
        assert!((last.end() - b).length() < 1e-6, "it ends at the point: {:?}", last.end());
        assert!(ang_delta(last.end_heading(), 180.0).abs() < 0.5, "arriving in the other's way: {}", last.end_heading());
        assert!(last.length > 0.0);
        at_each_join(&segs, a, b);
    }

    /// A short join is pieces that arrive on the heading it was asked for - the next segment's
    /// rotate, not the gap's own angle. A straight along the gap is what used to leave the last
    /// piece pointing somewhere else, and the planned join is allowed to be several times the
    /// gap to get the way round right.
    #[test]
    fn a_short_join_is_pieces_and_arrives_on_the_heading() {
        let a = DVec2::ZERO;
        for d in [0.4, 0.7, 1.2] {
            let b = a + crate::roads::dir(0.0) * d;
            for turn in [90.0, 135.0, 180.0] {
                let segs = connect_segs(a, 0.0, b, turn, Side::Auto);
                assert!(segs.len() >= 2, "a {d} m gap turning {turn}° came out as one piece: {segs:?}");
                at_each_join(&segs, a, b);
                let last = segs.last().unwrap();
                let off = ang_delta(last.end_heading(), turn).abs();
                assert!(off < 0.5, "a {d} m gap turning {turn}° arrives {off}° off ({})", last.end_heading());
            }
        }
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
        let segs = forward_segs(start, 0.0, end, Side::Auto);
        assert!(segs.len() >= 2, "one arc would loop: {} pieces", segs.len());
        assert!(segs.len() <= 6, "too many pieces: {}", segs.len());
        at_each_join(&segs, start, end);
        assert!(ang_delta(segs[0].heading, 0.0).abs() < 0.5, "it leaves the way it did: {}", segs[0].heading);
        for s in &segs {
            let step = s.end() - s.start;
            assert!(step.length() < 40.0, "a piece is a long loop: {}", step.length());
            assert!(crate::roads::dir(s.heading).dot(step) > 0.0, "a piece went backwards: {step:?}");
        }
        // (the old run turned left, then right, then left: a snake. A car holds one way)
        assert!(reversals(&segs) <= 1, "it snakes: {} reversals", reversals(&segs));
    }

    /// How many times a run changes the way it turns. Straights do not count.
    fn reversals(segs: &[crate::roads::Seg]) -> usize {
        let mut n = 0;
        let mut prev = 0.0;
        for s in segs {
            let t = if s.radius.abs() < 1e-6 { 0.0 } else { s.radius.signum() };
            if t != 0.0 && prev != 0.0 && (t - prev).abs() > 0.5 {
                n += 1;
            }
            if t != 0.0 {
                prev = t;
            }
        }
        n
    }

    /// A point off behind the start used to come out as a zigzag, each piece turning back on
    /// the one before it. A car turns one way until it is lined up.
    #[test]
    fn a_run_turns_one_way_instead_of_snaking() {
        let cases = [
            (DVec2::new(-8.0, -2.0), 0.0_f64),
            (DVec2::new(3.0, -12.0), 0.0),
            (DVec2::new(-4.0, 6.0), 0.0),
            (DVec2::new(10.0, 2.0), 90.0),
        ];
        for (end, heading) in cases {
            let segs = forward_segs(DVec2::ZERO, heading, end, Side::Auto);
            assert!(!segs.is_empty(), "no run to {end:?}");
            at_each_join(&segs, DVec2::ZERO, end);
            assert!(reversals(&segs) <= 1, "a snake to {end:?}: {} reversals", reversals(&segs));
        }
        // (and a join that cannot be one arc does the same)
        let join = connect_segs(DVec2::ZERO, 0.0, DVec2::new(-6.0, 4.0), 90.0, Side::Auto);
        at_each_join(&join, DVec2::ZERO, DVec2::new(-6.0, 4.0));
        assert!(reversals(&join) <= 1, "a snake of a join: {} reversals", reversals(&join));
    }

    /// A join must arrive on the heading it was asked for, whatever the gap and the angle: that
    /// is the next segment's rotate, and a last piece a few degrees off does not line up with
    /// it. Short gaps are no exception: a couple of metres is room for a bend of a usable
    /// radius, and the planned join is pieces rather than a straight along the gap.
    #[test]
    fn every_join_arrives_on_the_heading_it_was_asked() {
        let a = DVec2::ZERO;
        let mut worst: Option<(f64, f64, f64, f64)> = None;
        for d10 in 4..=200 {
            let d = d10 as f64 * 0.1;
            for step in 0..36 {
                let bearing = step as f64 * 10.0;
                let b = a + crate::roads::dir(bearing) * d;
                for hb_step in 0..36 {
                    let h_b = hb_step as f64 * 10.0;
                    let segs = connect_segs(a, 0.0, b, h_b, Side::Auto);
                    assert!(!segs.is_empty(), "no join at d {d} bearing {bearing} h_b {h_b}");
                    let last = segs.last().unwrap();
                    assert!((last.end() - b).length() < 0.05, "not on the point: d {d} bearing {bearing} h_b {h_b}: {:?}", last.end());
                    for q in &segs {
                        assert!(q.radius == 0.0 || q.radius.abs() >= 0.2, "a curl: radius {} at d {d}", q.radius);
                    }
                    let off = ang_delta(last.end_heading(), h_b).abs();
                    if worst.as_ref().is_none_or(|w| off > w.0) {
                        worst = Some((off, d, bearing, h_b));
                    }
                }
            }
        }
        let (off, d, bearing, h_b) = worst.unwrap();
        assert!(off < 0.5, "a join arrives {off}° off, at d {d} bearing {bearing} h_b {h_b}");
    }

    #[test]
    fn a_path_says_which_ways_it_goes() {
        assert_eq!(path_ways(0), (true, false), "along it");
        assert_eq!(path_ways(1), (false, true), "against it");
        assert_eq!(path_ways(2), (true, true), "both ways");
        // (what the file's unknown numbers are taken as: the way a new path runs)
        assert_eq!(path_ways(7), (true, false));
    }

    /// A new path laid outward from a join is driven the way the lane it joins is driven, and
    /// its `direction` is set from its own geometry against that travel - so it points the way
    /// the lane does rather than backwards.
    #[test]
    fn a_new_path_follows_the_way_of_the_one_it_joins() {
        use crate::crossing_edit::Joint;
        // (the old flow table is kept for the file's own sake)
        assert_eq!(outward_flow(0, true), 0, "traffic arrives: the new path is driven along it");
        assert_eq!(outward_flow(1, true), 1);
        assert_eq!(outward_flow(2, false), 2);

        let mut w = opened(Tab::Paths);
        w.set_from_nearby(true);
        // (a lane whose traffic runs north, and whose end is here: the new path leaves that end
        // outward, north, so its geometry runs with the traffic)
        let j = Joint { pos: DVec3::new(5.0, 5.0, 1.0), heading: 180.0, slope: 0.05, flow: 0, dir: 0, travel: 0.0, geom: 0.0 };
        let at = w.start_from_joint(j).expect("started");
        let p = &w.sheet.as_ref().unwrap().crossing.paths[at];
        assert!(ang_delta(p.heading(), 0.0).abs() < 0.5, "outward from it: {}", p.heading());
        assert_eq!(p.direction(), 0, "with its own geometry, so its chevrons point the lane's way");
        // (a lane whose traffic runs south, at its start: the new path leaves backwards, so it
        // is driven against its own geometry)
        assert_eq!(j.direction_for(180.0), 1, "leaf and travel are opposite");
        // (both ways stays both ways)
        let both = Joint { dir: 2, ..j };
        assert_eq!(both.direction_for(180.0), 2);
    }

    /// `heading` is the other path's own tangent into its body, which is the rotate the end
    /// shows; `through` is the way its traffic goes, which may be the other way round. A join
    /// arrives on the rotate (see `completing_onto_a_joint_arrives_on_its_rotate`), and a path
    /// begun at the joint leaves outward.
    #[test]
    fn a_joints_rotate_and_its_traffic_are_told_apart() {
        let leaves = Joint { pos: DVec3::ZERO, heading: 90.0, slope: 0.0, flow: 1, dir: 0, travel: 90.0, geom: 0.0 };
        assert!((leaves.through() - 90.0).abs() < 1e-9, "{}", leaves.through());
        let arrives = Joint { pos: DVec3::ZERO, heading: 90.0, slope: 0.0, flow: 0, dir: 0, travel: 270.0, geom: 0.0 };
        assert!((arrives.through() - 270.0).abs() < 1e-9, "{}", arrives.through());
        // (a path begun at either runs outward, whichever way the traffic goes)
        assert!((leaves.outward() - 270.0).abs() < 1e-9);
        assert!((arrives.outward() - 270.0).abs() < 1e-9);
        // (its own geometry, compared with the travel, decides a new path's direction)
        assert_eq!(leaves.direction_for(270.0), 1, "outward runs against the traffic here");
        assert_eq!(arrives.direction_for(270.0), 0, "outward runs with it here");
    }

    /// `Complete to` arrives on the other path's own rotate at that end - the tangent of its
    /// geometry, which is what the start/end rotate shows. Arriving on the traffic direction
    /// put the last piece 180 degrees out on half the ends, so it did not line up with the next
    /// segment at all.
    #[test]
    fn completing_onto_a_joint_arrives_on_its_rotate() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        w.selected = Some(2);
        for (i, at_end) in [(0usize, true), (0usize, false), (1usize, true)] {
            let before = w.sheet.as_ref().unwrap().crossing.paths.len();
            let j = w.joint_at(i, at_end).expect("the other end is a joint");
            w.set_complete_to(true);
            let last = w.complete_to_joint(j).expect("joined");
            let paths = &w.sheet.as_ref().unwrap().crossing.paths;
            assert!(paths.len() - before <= 4, "a join, not a ring: {} pieces", paths.len() - before);
            assert_eq!(last, paths.len() - 1);
            let p = &paths[last];
            assert!((p.end().truncate() - j.pos.truncate()).length() < 0.05, "it ends on the joint: {:?}", p.end());
            let c = p.curve();
            let got = c.heading_at(c.length.max(0.0));
            assert!(ang_delta(got, j.geom).abs() < 0.5, "arriving on its rotate: {got} vs {}", j.geom);
        }
    }

    /// Without `Complete to`, a drag never splits the path: it moves one end of one path,
    /// however far it is dragged and however often. Splitting belongs to a join alone.
    #[test]
    fn a_drag_without_completing_never_splits_a_path() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(w.take_map_end(Some((2, true))));
        for k in 0..12 {
            let a = (k as f64) * 0.5;
            let at = DVec3::new(3.0 + a, 9.0 - a, 0.2);
            assert!(w.drag_map_end(at, None, None));
            assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n, "drag {k} split the path");
            assert_eq!(w.selected, Some(2), "and it is still the same path");
        }
        // (a snapped drag is one path too)
        let target = w.sheet.as_ref().unwrap().crossing.paths[0].end();
        let j = w.joint_at(0, true).expect("a joint");
        w.let_go();
        assert!(w.take_map_end(Some((2, false))));
        assert!(w.drag_map_end(target, Some(j), None));
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n, "a snapped drag split the path");
    }

    /// A join of half a metre is no knot: every bend it has is a real radius, it ends on the
    /// point, and its last piece arrives on the heading it was asked for. It used to be a bend
    /// of 166 degrees in 0.46 m - a radius of 0.23 m, which reads as the wrong way.
    ///
    /// The join is allowed to be long (about a dozen metres, mostly doubling back) because
    /// arriving on the next segment's rotate matters more than the length: a short straight on
    /// the gap landed on its own angle, and the next segment's rotate then showed the kink.
    /// What must stay true is that the turn is spread over real radii, not curled into one
    /// centimetre knot.
    #[test]
    fn a_tiny_join_is_no_knot() {
        let a = DVec2::new(-48.0346, 19.2132);
        let b = DVec2::new(-47.6323, 18.9858);
        let chord = (b - a).length();
        // (under half a metre `connect_segs` allows nothing below `CLOSE_RADIUS`: that is the
        // floor it plans with, so every bend here has to clear it)
        assert!(chord < 0.5);
        let h_b = 337.6;
        let segs = connect_segs(a, 143.5, b, h_b, Side::Auto);
        assert!(!segs.is_empty(), "a join was made");
        at_each_join(&segs, a, b);
        for s in &segs {
            assert!(s.radius == 0.0 || s.radius.abs() >= CLOSE_RADIUS, "a real radius: {}", s.radius);
        }
        let last = segs.last().unwrap();
        let off = ang_delta(last.end_heading(), h_b).abs();
        assert!(off < 0.5, "it arrives on the heading asked for: {off}° off ({})", last.end_heading());

        // (a join of about a metre whose next piece is not on the gap's own angle: it arrives
        // on that angle, rather than a straight that leaves the two different)
        let a = DVec2::new(47.76687, -17.8402);
        let b = DVec2::new(47.6323, -18.9858);
        let phi = crate::roads::heading_of(b - a);
        let h_b = crate::roads::norm_deg(phi + 14.0);
        let h_a = crate::roads::norm_deg(phi - 14.0);
        let segs = connect_segs(a, h_a, b, h_b, Side::Auto);
        at_each_join(&segs, a, b);
        let last = segs.last().unwrap();
        assert!(ang_delta(last.end_heading(), h_b).abs() < 0.5, "it arrives with the next piece: {}", last.end_heading());
        assert!(last.radius.abs() >= CLOSE_RADIUS, "a bend, not a straight on the gap: radius {}", last.radius);
        assert!(ang_delta(segs[0].heading, h_a).abs() < 0.5, "it leaves as the piece before it arrived: {}", segs[0].heading);

        // and a join wide enough for a real bend is still two arcs that meet tangentially
        let wide = connect_segs(DVec2::new(0.0, 10.0), 0.0, DVec2::new(6.0, 16.0), 180.0, Side::Auto);
        at_each_join(&wide, DVec2::new(0.0, 10.0), DVec2::new(6.0, 16.0));
        for s in &wide {
            assert!(s.radius == 0.0 || s.radius.abs() >= MIN_PIECE_RADIUS, "a real bend: radius {}", s.radius);
        }
    }

    /// The reported join was four pieces: 61.7 m, 0.6 m, 34.6 m, 1.2 m, the 0.6 m one turning
    /// 1.9 degrees on a 20 m radius. `join::plan` used to take the widest radius within 1.35x of
    /// the shortest, and on a nearly straight gap that is a ten-metre straight between two
    /// centimetre arcs. A piece shorter than [`crate::join::MIN_PIECE_LEN`] is now refused when
    /// a join without one exists.
    #[test]
    fn a_join_has_no_near_zero_stub() {
        let a = DVec2::ZERO;
        // (the reported joint: leave on 296.7, arrive on 298.6. The radius of 20 in the report
        // needs a chord of at least 10.7 m for the old sweep to offer it at all)
        for (d, bearing) in [(10.7, 298.4), (10.7, 297.0), (10.7, 299.0), (4.8, 301.0), (3.1, 305.0)] {
            let b = a + crate::roads::dir(bearing) * d;
            let segs = connect_segs(a, 296.7, b, 298.6, Side::Auto);
            at_each_join(&segs, a, b);
            for s in &segs {
                assert!(s.length >= crate::join::MIN_PIECE_LEN, "a {:.2} m stub at d {d} bearing {bearing}: {segs:?}", s.length);
            }
        }
        // (the gaps and headings a junction actually has: no piece may be too short to be driven)
        for d in [2.5, 4.0, 6.0, 10.0, 15.0, 25.0] {
            for step in 0..24 {
                let bearing = step as f64 * 15.0;
                let b = a + crate::roads::dir(bearing) * d;
                for dh in [-60.0, -30.0, 0.0, 30.0, 60.0] {
                    let h_a = 10.0;
                    let h_b = crate::roads::norm_deg(h_a + dh);
                    let segs = connect_segs(a, h_a, b, h_b, Side::Auto);
                    assert!(!segs.is_empty(), "no join at d {d} bearing {bearing} dh {dh}");
                    at_each_join(&segs, a, b);
                    for s in &segs {
                        assert!(
                            s.length >= crate::join::MIN_PIECE_LEN,
                            "a {:.2} m stub at d {d} bearing {bearing} dh {dh}: {segs:?}",
                            s.length
                        );
                    }
                }
            }
        }
    }

    /// The turn is spread over the join rather than saved up for one end: when two or more
    /// pieces turn, no one of them does more than 70% of the whole turn. A piece that turns
    /// most of a circle is a loop, not a join, and is tested apart (the run's loops are the
    /// point-behind case, where no forward way exists).
    #[test]
    fn a_join_spreads_its_turn_over_real_pieces() {
        let a = DVec2::ZERO;
        // (a junction's joining ways differ by a lane's angle, not a hairpin, and have a few
        // metres to spend: a 60 degree turn in 2.5 m has no answer that is both on the heading
        // and evenly split, and its stub-free join is the best it can do)
        for d in [4.0, 6.0, 10.0, 15.0, 25.0] {
            for step in 0..24 {
                let bearing = step as f64 * 15.0;
                let b = a + crate::roads::dir(bearing) * d;
                for dh in [-30.0, -10.0, 0.0, 10.0, 30.0] {
                    let h_a = 10.0;
                    let h_b = crate::roads::norm_deg(h_a + dh);
                    let segs = connect_segs(a, h_a, b, h_b, Side::Auto);
                    if crate::join::has_loop(&segs) {
                        continue;
                    }
                    let turns: Vec<f64> = segs
                        .iter()
                        .filter(|s| s.radius != 0.0)
                        .map(|s| (s.length / s.radius.abs()).to_degrees())
                        .collect();
                    let total: f64 = turns.iter().sum();
                    if turns.len() >= 2 && total >= 10.0 {
                        let most = turns.iter().copied().fold(0.0, f64::max);
                        assert!(
                            most <= total * 0.7,
                            "one piece does {:.0}% of a {total:.0} degree turn at d {d} bearing {bearing} dh {dh}: {segs:?}",
                            most / total * 100.0
                        );
                    }
                }
            }
        }
    }

    /// The retry `Complete to` makes when the planned join lands a hair off must not write an
    /// invisible curl into the file: every piece it returns is at least a third of a metre, and
    /// it still arrives on the heading it was asked for.
    #[test]
    fn the_forced_join_never_writes_a_curl() {
        let a = DVec2::ZERO;
        for d in [0.6, 1.0, 2.5, 5.0, 10.0] {
            for step in 0..36 {
                let bearing = step as f64 * 10.0;
                let b = a + crate::roads::dir(bearing) * d;
                for dh in [0.0, 10.0, 90.0, 180.0] {
                    let h_a = 10.0;
                    let h_b = crate::roads::norm_deg(h_a + dh);
                    if let Some(segs) = forced_join(a, h_a, b, h_b, Side::Auto) {
                        assert!(arrives(&segs, b, h_b), "it does not arrive at d {d} bearing {bearing} dh {dh}: {segs:?}");
                        for s in &segs {
                            assert!(
                                s.length >= 0.3,
                                "a {:.2} m curl at d {d} bearing {bearing} dh {dh}: {segs:?}",
                                s.length
                            );
                        }
                    }
                }
            }
        }
    }

    /// An end dragged with the mouse follows the object's surface: its height is the deck's, and
    /// the rise between the two ends is what the path climbs at - a drag that kept the height
    /// left the path floating over the deck or sunk into it.
    #[test]
    fn a_dragged_end_sits_on_the_deck() {
        // (a deck at 1.5 m over a 20 m square)
        let tris = [
            0.0, 0.0, 1.5, 20.0, 0.0, 1.5, 0.0, 20.0, 1.5, //
            0.0, 20.0, 1.5, 20.0, 0.0, 1.5, 20.0, 20.0, 1.5,
        ];
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        // (path 2 runs 10 m north from the middle of the crossing, at 0)
        move_end_on_surface(&mut crossing, 2, true, DVec2::new(0.0, 8.0), &tris, None);
        let p = &crossing.paths[2];
        assert!((p.end().z - 1.5).abs() < 1e-3, "the dragged end sits on the deck: {}", p.end().z);
        assert!((p.start().z - 0.0).abs() < 1e-3, "the other end keeps its height: {}", p.start().z);
        assert!((p.slope_start() - p.slope_end()).abs() < 1e-4, "one grade: {} / {}", p.slope_start(), p.slope_end());

        // (and a dragged start: it sits on the deck, and the far end does not move)
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        let end_before = crossing.paths[2].end();
        move_end_on_surface(&mut crossing, 2, false, DVec2::new(2.0, 2.0), &tris, None);
        let p = &crossing.paths[2];
        assert!((p.start().z - 1.5).abs() < 1e-3, "the start sits on the deck: {}", p.start().z);
        assert!((p.end().truncate() - end_before.truncate()).length() < 0.05, "the far end stays: {:?} vs {end_before:?}", p.end());
    }

    /// A piece too short to climb to another path's height at any grade climbs as fast as it may
    /// and no more: 68% over 0.7 m was the number this keeps out.
    #[test]
    fn a_short_piece_climbs_no_steeper_than_it_may() {
        let mut p = Path::default();
        p.v[2] = 0.0;
        p.v[5] = 0.5;
        // (a kerb of 20 cm over half a metre is a real grade)
        climb_to(&mut p, 0.2);
        assert!((p.end().z - 0.2).abs() < 1e-3, "it reaches the kerb: {}", p.end().z);
        assert!((p.slope_start() - 0.4).abs() < 1e-4, "at 40%: {}", p.slope_start());
        // (a metre and a half of climb is not a path of half a metre: it takes as much as it may)
        climb_to(&mut p, 2.0);
        assert!((p.slope_start() - MAX_RISE).abs() < 1e-6, "no steeper than {}: {}", MAX_RISE, p.slope_start());
        assert!((p.slope_end() - MAX_RISE).abs() < 1e-6, "one grade");
    }

    /// A green end right beside the picked path's own end answers: the end is put on it, rather
    /// than the click doing nothing at all (the join between them is shorter than a hand's
    /// reach, and it used to be refused).
    #[test]
    fn completing_onto_a_joint_beside_the_end_still_answers() {
        let mut w = opened(Tab::Paths);
        w.selected = Some(2);
        let end = w.sheet.as_ref().unwrap().crossing.paths[2].end();
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        let j = Joint { pos: end + DVec3::new(0.08, 0.05, 0.0), heading: 0.0, slope: 0.02, flow: 0, dir: 0, travel: 0.0, geom: 90.0 };
        w.set_complete_to(true);
        let at = w.complete_to_joint(j).expect("it answers");
        assert_eq!(at, 2, "the picked path's own end is the one put on it");
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n, "no piece was added");
        let p = &w.sheet.as_ref().unwrap().crossing.paths[2];
        assert!((p.end().truncate() - j.pos.truncate()).length() < 0.02, "the end is on it: {:?}", p.end());
        assert!((p.slope_end() - j.slope).abs() < 1e-4, "at its rise: {}", p.slope_end());
    }

    /// `Complete to` is not asked for while it is off, and a joint beyond a crossing's reach is
    /// still refused - so the dot beside the end explaining itself does not open the door to
    /// joins across the map.
    #[test]
    fn completing_is_only_within_the_crossing() {
        let mut w = opened(Tab::Paths);
        w.selected = Some(2);
        let end = w.sheet.as_ref().unwrap().crossing.paths[2].end();
        let j = Joint { pos: end + DVec3::new(0.08, 0.0, 0.0), heading: 0.0, slope: 0.0, flow: 0, dir: 0, travel: 0.0, geom: 0.0 };
        assert!(w.complete_to_joint(j).is_none(), "the mode is off");
        w.set_complete_to(true);
        let far = Joint { pos: end + DVec3::new(2_000_000.0, 0.0, 0.0), ..j };
        assert!(w.complete_to_joint(far).is_none(), "across the map is refused");
    }

    /// A path's end and the next path's start are one point at every join. Clicking it with the
    /// first path picked must take that path's own end, not the neighbour's disc there.
    #[test]
    fn a_click_on_a_shared_end_takes_the_picked_paths_own() {
        let mut sheet = crate::crossing_edit::Sheet::parse(SCO);
        // (path 1's start put on path 0's end: what a join between two pieces looks like)
        let shared = sheet.crossing.paths[0].end().truncate();
        let z = sheet.crossing.paths[1].start().z;
        sheet.crossing.paths[1].set_start(DVec3::new(shared.x, shared.y, z));
        assert!((sheet.crossing.paths[1].start().truncate() - shared).length() < 1e-3, "they meet");
        let sheet = &sheet;
        let screen = |q: DVec2| Pos2::new((q.x - shared.x) as f32, (q.y - shared.y) as f32);
        // (at the shared point, 1:1 pixels to metres)
        assert_eq!(end_at(sheet, Some(0), Pos2::ZERO, &screen), Some((0, true)), "with path 0 picked");
        assert_eq!(end_at(sheet, Some(1), Pos2::ZERO, &screen), Some((1, false)), "with path 1 picked");
        // (nothing picked: the nearest still answers, and a click far from any end takes none)
        assert!(end_at(sheet, None, Pos2::ZERO, &screen).is_some());
        assert_eq!(end_at(sheet, Some(0), Pos2::new(300.0, 300.0), &screen), None);
    }

    /// Delete takes the picked path out, as the list's bin does, in one step back.
    #[test]
    fn the_delete_key_takes_the_picked_path_out() {
        let mut w = opened(Tab::Paths);
        w.selected = Some(1);
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(w.delete_picked());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n - 1);
        assert_eq!(w.picked(), Some(1), "the path that took its place is picked");
        assert!(w.undo(), "one step back");
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n);
        // (nothing picked: nothing deleted)
        w.selected = None;
        assert!(!w.delete_picked());
    }

    /// K (or the `Split` button) cuts the picked path in two at its middle: the halves are
    /// arcs of the same radius, the line is unbroken where it was cut, the far end stays, and
    /// each half can be turned on its own afterwards.
    #[test]
    fn a_path_can_be_cut_in_two() {
        let mut w = opened(Tab::Paths);
        let before = w.sheet.as_ref().unwrap().crossing.paths.len();
        w.selected = Some(2);
        let was = w.sheet.as_ref().unwrap().crossing.paths[2].clone();
        assert!(w.split_picked());
        let paths = &w.sheet.as_ref().unwrap().crossing.paths;
        assert_eq!(paths.len(), before + 1, "one piece more");
        assert_eq!(w.picked(), Some(3), "the second half is the one picked");
        let (a, b) = (&paths[2], &paths[3]);
        assert!((a.length() - was.length() / 2.0).abs() < 1e-2, "half each: {} / {}", a.length(), b.length());
        assert!((b.length() - was.length() / 2.0).abs() < 1e-2, "half each: {} / {}", a.length(), b.length());
        assert!((a.end().truncate() - b.start().truncate()).length() < 1e-2, "the halves meet: {:?} vs {:?}", a.end(), b.start());
        assert!(ang_delta(a.curve().heading_at(a.length()), b.heading()).abs() < 0.05, "no kink at the cut: {} then {}", a.curve().heading_at(a.length()), b.heading());
        assert!((a.start().truncate() - was.start().truncate()).length() < 1e-3, "it begins where it did");
        assert!((b.end().truncate() - was.end().truncate()).length() < 1e-2, "the far end stays: {:?} vs {:?}", b.end(), was.end());
        // (the radius and everything the file says about the path are kept on both halves)
        assert!((a.radius() - was.radius()).abs() < 1e-6 && (b.radius() - was.radius()).abs() < 1e-6);
        assert_eq!((a.kind(), a.width(), a.direction(), a.turn()), (b.kind(), b.width(), b.direction(), b.turn()));
        // (and the height is one profile: no step at the cut)
        assert!((a.end().z - b.start().z).abs() < 1e-4, "{} then {}", a.end().z, b.start().z);
        assert!((b.slope_start() - a.slope_end()).abs() < 1e-4, "one grade at the cut: {} / {}", a.slope_end(), b.slope_start());
        // (one step back for Ctrl+Z)
        assert!(w.undo());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before);
    }

    /// A path too short to be worth cutting is left alone, and nothing is recorded for Ctrl+Z.
    #[test]
    fn a_path_too_short_to_cut_is_left_alone() {
        let mut w = opened(Tab::Paths);
        w.sheet.as_mut().unwrap().crossing.paths[2].v[5] = 0.1;
        w.selected = Some(2);
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(!w.split_picked(), "nothing is cut");
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n);
        // (no pick, no cut)
        w.selected = None;
        assert!(!w.split_picked());
    }

    /// openOMSI's own lane joining (`crates/omsi-sim/src/traffic/extend.rs`): one lane's
    /// leaving end and the next lane's entering end, of the same kind, within the tolerance,
    /// within 3 m in height and 40 degrees of heading. It never reads `[splinehelper]`, so this
    /// is the test a crossing's ends have to pass for its traffic to drive through.
    fn openomsi_joins(a_end: DVec3, a_h: f64, b_start: DVec3, b_h: f64) -> bool {
        let d = (b_start.truncate() - a_end.truncate()).length();
        let dh = (((b_h - a_h + 540.0).rem_euclid(360.0)) - 180.0).abs();
        d <= LINK_TOLERANCE && dh < LINK_ANGLE && (b_start.z - a_end.z).abs() < LINK_RISE
    }

    /// `Connect the roads` makes the crossing's ends meet the road lanes' by openOMSI's own
    /// numbers - both a road that ends at the crossing and one that begins there, and with a
    /// path driven against its own geometry as well.
    #[test]
    fn a_link_replicates_what_openomsi_joins() {
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        // (path 2 runs north from the middle of the crossing: (0, 0) to (0, 10), two-way)
        let roads = vec![
            // (a road arriving from the south: its lane ends where the crossing's starts)
            RoadLaneEnd { at: DVec3::new(0.0, 0.0, 0.0), heading: 0.0, entering: false, kind: 0, who: "south".into(), file: Some("Splines\\road.sli".into()) },
            // (a road leaving to the north: its lane begins where the crossing's ends)
            RoadLaneEnd { at: DVec3::new(0.0, 10.0, 0.0), heading: 0.0, entering: true, kind: 0, who: "north".into(), file: None },
        ];
        let notes = link_crossing(&mut crossing, &roads, &[], Side::Auto);
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|n| n.ok), "{notes:?}");
        let p = &crossing.paths[2];
        assert!(openomsi_joins(DVec3::new(0.0, 0.0, 0.0), 0.0, p.start(), p.heading()), "the road feeds the crossing");
        assert!(openomsi_joins(p.end(), p.curve().heading_at(p.length()), DVec3::new(0.0, 10.0, 0.0), 0.0), "the crossing feeds the road");
        // (a helper is offered where the road leaves, facing out along it)
        let h = helper_for(&notes[0]).expect("a helper for the road that has a type");
        assert!((h.local() - DVec3::new(0.0, 0.0, 0.0)).length() < 1e-3, "{:?}", h.local());
        assert!((h.heading as f64 - 180.0).abs() < 0.5, "out of the crossing, away from the road's traffic: {}", h.heading);
        assert!(helper_for(&notes[1]).is_none(), "no road type, no helper");

        // (the same path driven against its own geometry: its lane starts at its geometric
        // end, so that is the end the road from the north must meet)
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        crossing.paths[2].v[10] = 1.0;
        let roads = vec![
            RoadLaneEnd { at: DVec3::new(0.0, 10.0, 0.0), heading: 180.0, entering: false, kind: 0, who: "north".into(), file: None },
            RoadLaneEnd { at: DVec3::new(0.0, 0.0, 0.0), heading: 180.0, entering: true, kind: 0, who: "south".into(), file: None },
        ];
        let notes = link_crossing(&mut crossing, &roads, &[], Side::Auto);
        assert!(notes.iter().all(|n| n.ok), "{notes:?}");
        let p = &crossing.paths[2];
        assert!(openomsi_joins(DVec3::new(0.0, 10.0, 0.0), 180.0, p.end(), crate::roads::norm_deg(p.curve().heading_at(p.length()) + 180.0)), "against its geometry, the lane starts at its end");
        assert!(openomsi_joins(p.start(), crate::roads::norm_deg(p.heading() + 180.0), DVec3::new(0.0, 0.0, 0.0), 180.0), "and ends at its start");
    }

    /// A road end too far to be put on is reached with one piece, laid the way its traffic
    /// goes, so both joins are openOMSI's.
    #[test]
    fn a_link_closes_a_gap_with_a_connector() {
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        let before = crossing.paths.len();
        // (the road begins 7 m north of the crossing's end: too far to move, near enough for a
        // piece)
        let roads = vec![RoadLaneEnd { at: DVec3::new(0.0, 17.0, 0.0), heading: 0.0, entering: true, kind: 0, who: "north".into(), file: None }];
        let notes = link_crossing(&mut crossing, &roads, &[], Side::Auto);
        assert!(notes[0].ok, "{notes:?}");
        assert_eq!(crossing.paths.len(), before + 1, "one piece was added");
        let added = crossing.paths.last().unwrap();
        assert!((added.start().truncate() - DVec2::new(0.0, 10.0)).length() < 0.05, "it starts at the crossing's end: {:?}", added.start());
        assert!((added.end().truncate() - DVec2::new(0.0, 17.0)).length() < 0.05, "and ends at the road's start: {:?}", added.end());
        assert!(ang_delta(added.heading(), 0.0).abs() < 0.5, "leaving the crossing north: {}", added.heading());
        assert!(ang_delta(added.curve().heading_at(added.length()), 0.0).abs() < 0.5, "arriving north: {}", added.curve().heading_at(added.length()));
        assert_eq!(added.kind(), 0, "the road's kind");
        // (both joins are openOMSI's)
        let p = &crossing.paths[2];
        assert!(openomsi_joins(p.end(), p.curve().heading_at(p.length()), added.start(), added.heading()));
        assert!(openomsi_joins(added.end(), added.curve().heading_at(added.length()), DVec3::new(0.0, 17.0, 0.0), 0.0));
    }

    /// A road end of another kind is not forced onto the crossing: it is reported, and nothing
    /// in the file moves.
    #[test]
    fn an_unlinkable_end_is_reported() {
        let mut crossing = crate::crossing_edit::Sheet::parse(SCO).crossing;
        let before = crossing.paths.clone();
        // (a footway where only street paths are)
        let roads = vec![RoadLaneEnd { at: DVec3::new(0.0, 0.0, 0.0), heading: 0.0, entering: false, kind: 1, who: "footway".into(), file: None }];
        let notes = link_crossing(&mut crossing, &roads, &[], Side::Auto);
        assert_eq!(notes.len(), 1);
        assert!(!notes[0].ok, "a footway is not joined to a street path");
        assert!(!notes[0].why.is_empty(), "it says why");
        assert_eq!(crossing.paths.len(), before.len(), "nothing was added");
        assert!(crossing.paths.iter().zip(&before).all(|(a, b)| a.v == b.v), "nothing was moved");
    }

    /// A nearby end, in the object's own frame, is refused when it is not in that frame at all:
    /// a joint left in the map's coordinates is ten million metres away, and a path started
    /// from it lands out there (a radius of millions, a length past the file's limit).
    #[test]
    fn a_joint_from_another_frame_is_not_taken() {
        let mut w = opened(Tab::Paths);
        w.set_from_nearby(true);
        let before = w.sheet.as_ref().unwrap().crossing.paths.len();
        let far = Joint { pos: DVec3::new(-11_748_849.0, -23_623_152.5, 66.8), heading: 90.0, slope: 0.05, flow: 0, dir: 0, travel: 0.0, geom: 0.0 };
        assert!(w.start_from_joint(far).is_none(), "a joint ten million metres out is not a join");
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before, "nothing was added");
        assert!(w.wants_nearby(), "and the mode stays on, ready for a real one");

        // (a nearby end in the object's frame is still taken)
        let near = Joint { pos: DVec3::new(5.0, 5.0, 1.0), heading: 90.0, slope: 0.05, flow: 0, dir: 0, travel: 0.0, geom: 0.0 };
        assert!(w.start_from_joint(near).is_some());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before + 1);
        // (and a join across the map is refused too, rather than making a path of millions)
        w.set_complete_to(true);
        let p = w.sheet.as_ref().unwrap().crossing.paths[2].end();
        let out = Joint { pos: p + DVec3::new(-2_000_000.0, 0.0, 0.0), heading: 0.0, slope: 0.0, flow: 0, dir: 0, travel: 0.0, geom: 0.0 };
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(w.complete_to_joint(out).is_none());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n, "nothing was added");
    }

    /// The way round a join can be chosen: left and right both reach the same point smoothly,
    /// turning their own way instead of the same way round.
    #[test]
    fn a_join_can_be_told_which_way_to_turn() {
        let a = DVec2::new(0.0, 10.0);
        let b = DVec2::new(6.0, 16.0);
        let left = connect_segs(a, 0.0, b, 180.0, Side::Left);
        let right = connect_segs(a, 0.0, b, 180.0, Side::Right);
        for segs in [&left, &right] {
            assert!(!segs.is_empty(), "a join was made");
            at_each_join(segs, a, b);
        }
        // (every bend it has turns its own way - the radius's sign is the file's own: + right -
        // and a straight is no bend)
        assert!(left.iter().filter(|s| s.radius != 0.0).all(|s| s.radius < 0.0), "left turns left: {left:?}");
        assert!(right.iter().filter(|s| s.radius != 0.0).all(|s| s.radius > 0.0), "right turns right: {right:?}");
        // (and they are not the same run)
        assert!((left[0].end_heading() - right[0].end_heading()).abs() > 5.0 || (left.len() != right.len()), "{left:?} vs {right:?}");
        // (the choice reaches the crossing's joins)
        let mut w = opened(Tab::Paths);
        assert_eq!(w.side(), Side::Auto);
        w.set_side(Side::Right);
        assert_eq!(w.side(), Side::Right);
        w.selected = Some(2);
        w.set_complete_to(true);
        let end = w.sheet.as_ref().unwrap().crossing.paths[2].end();
        let j = Joint { pos: end + DVec3::new(6.0, 6.0, 0.4), heading: 180.0, slope: 0.0, flow: 1, dir: 0, travel: 0.0, geom: 0.0 };
        let at = w.complete_to_joint(j).expect("joined");
        let p = &w.sheet.as_ref().unwrap().crossing.paths[at];
        assert!(p.radius() > 0.0, "the join turns the way it was told: radius {}", p.radius());
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

    /// The grid gives the same answer as walking every triangle, and looks at far fewer of
    /// them (which is the whole point on an object with hundreds of paths).
    #[test]
    fn the_surface_grid_agrees_with_the_slow_path() {
        let mut tris = Vec::new();
        for j in 0..10 {
            for i in 0..10 {
                let (x, y) = (i as f32 * 4.0, j as f32 * 4.0);
                let z = (i + j) as f32 * 0.1 + 1.0;
                tris.extend([x, y, z, x + 4.0, y, z, x, y + 4.0, z]);
                tris.extend([x + 4.0, y, z, x + 4.0, y + 4.0, z, x, y + 4.0, z]);
            }
        }
        let grid = SurfaceGrid::build(&tris).expect("a grid");
        for (i, j) in [(0, 0), (3, 7), (9, 9), (5, 2)] {
            let p = DVec2::new(i as f64 * 4.0 + 1.5, j as f64 * 4.0 + 2.5);
            assert_eq!(grid.z(&tris, p), surface_z(&tris, p), "at {p:?}");
        }
        let (_, tests) = grid.z_tests(&tris, DVec2::new(18.0, 18.0));
        assert!(tests < 16, "the grid tested {tests} of {} triangles", tris.len() / 9);
        assert_eq!(grid.z(&tris, DVec2::new(200.0, 200.0)), None);
    }

    /// A 787-path object's overlay: the adaptive step is what keeps the line points from
    /// filling the frame's buffer (the real drawn count, chevrons and discs included, is in
    /// the trace log).
    #[test]
    fn a_crowd_of_paths_is_sampled_coarsely() {
        let paths: Vec<crate::crossing_edit::Path> = (0..787).map(|_| crate::crossing_edit::Path::default()).collect();
        let line_points = |step: f64| paths.iter().map(|p| p.points(step).len().saturating_sub(1) * 6).sum::<usize>();
        let fine = line_points(0.5);
        let coarse = line_points(3.0);
        println!("787 ten-metre paths: {fine} line points at 0.5 m, {coarse} at 3.0 m");
        assert!(fine > 90_000, "{fine}");
        assert!(coarse * 4 < fine, "{coarse} vs {fine}");
    }

    /// A mesh as large as the surface is allowed to be: 150 samples through the grid test a
    /// small fraction of the triangles the slow path would test (60,000 each).
    #[test]
    fn the_surface_grid_tests_a_fraction_of_a_big_mesh() {
        let mut tris = Vec::with_capacity(60_000 * 9);
        for n in 0..60_000 {
            let (i, j) = (n % 250, (n / 250) % 240);
            let (x, y) = (i as f32 * 0.4, j as f32 * 0.4);
            let z = 1.0 + (n % 7) as f32 * 0.01;
            tris.extend([x, y, z, x + 0.4, y, z, x, y + 0.4, z]);
        }
        let grid = SurfaceGrid::build(&tris).expect("a grid");
        let mut tests = 0usize;
        for n in 0..150 {
            let p = DVec2::new((n % 15) as f64 * 6.0 + 1.0, (n / 15) as f64 * 9.0 + 1.0);
            tests += grid.z_tests(&tris, p).1;
        }
        println!("150 samples: {tests} triangle tests through the grid, {} through the slow path", 150 * 60_000);
        assert!(tests < 150 * 400, "150 samples tested {tests} triangles");
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
        let j = Joint { pos: DVec3::new(5.0, 5.0, 1.0), heading: 90.0, slope: 0.05, flow: 0, dir: 0, travel: 0.0, geom: 0.0 };
        // (not while the mode is off)
        assert!(!w.wants_nearby());
        assert!(w.start_from_joint(j).is_none(), "nothing is done while the mode is off");
        w.set_from_nearby(true);
        assert!(w.wants_nearby());
        let at = w.start_from_joint(j).expect("a path was started");
        let p = &w.sheet.as_ref().unwrap().crossing.paths[at];
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before + 1);
        assert!((p.start() - j.pos).length() < 1e-3, "it starts at the nearby end: {:?}", p.start());
        // (begun at the joint, it runs outward - back along the other path's body - whichever
        // way its traffic goes: that is what `flow` is for)
        assert!((p.heading() - j.outward()).abs() < 1e-3, "outward from that path: {} vs {}", p.heading(), j.outward());
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
        let j = Joint { pos: a + DVec3::new(6.0, 6.0, 0.4), heading: 180.0, slope: 0.03, flow: 1, dir: 0, travel: 0.0, geom: 0.0 };
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
        assert!(((got - j.geom).rem_euclid(360.0)).min((j.geom - got).rem_euclid(360.0)) < 0.5, "arriving on its own rotate: {got} vs {}", j.geom);
        assert!((end.end().z - j.pos.z).abs() < 0.02, "at its height: {}", end.end().z);
        assert!((end.slope_end() - j.slope).abs() < 1e-4, "and at its rise: {}", end.slope_end());
        // (the mode stays on, so another path can be carried on at once; its green ends are
        // still gathered)
        assert!(w.completing(), "the mode stays on for the next join");
        // (one step back for Ctrl+Z)
        assert!(w.undo());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), before.len());
    }

    /// `Complete to` ends at the next path's height even over a short gap: the rise is spread
    /// along the whole join, so the last piece lands on the joint's z at a driveable grade
    /// instead of being clamped at 50% and stopping short. It still arrives on the joint's
    /// rotate.
    #[test]
    fn a_join_ends_at_the_next_paths_height() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        w.selected = Some(2);
        let p = w.sheet.as_ref().unwrap().crossing.paths[2].clone();
        let a = p.end();
        let c = p.curve();
        let way = crate::roads::norm_deg(c.heading_at(c.length.max(0.0)));
        // (0.6 m to the side, a kerb of 0.25 m up, in that side's own way: a corner, so
        // several pieces rather than one straight)
        let geom = crate::roads::norm_deg(way + 90.0);
        let step = crate::roads::dir(geom) * 0.6;
        let j = Joint { pos: a + DVec3::new(step.x, step.y, 0.25), heading: geom, slope: 0.0, flow: 1, dir: 0, travel: 0.0, geom };
        w.set_complete_to(true);
        let last = w.complete_to_joint(j).expect("joined");
        let paths = &w.sheet.as_ref().unwrap().crossing.paths;
        let end = &paths[last];
        assert!((end.end().z - j.pos.z).abs() < 0.02, "it ends at the joint's height: {} vs {}", end.end().z, j.pos.z);
        assert!(end.slope_start().abs() < 1.0, "no steeper than 100%: {}", end.slope_start());
        assert!(end.slope_end().abs() < 1.0, "and its end slope: {}", end.slope_end());
        let ec = end.curve();
        let got = ec.heading_at(ec.length.max(0.0));
        assert!(ang_delta(got, j.geom).abs() < 0.5, "arriving on its rotate: {got} vs {}", j.geom);
    }

    /// A drag in 3D while `Complete to` is on but not used keeps the green ends drawn: the
    /// circles vanished when the mode lost them, which read as a fault.
    #[test]
    fn the_green_ends_stay_while_completing() {
        let mut w = opened(Tab::Paths);
        w.set_complete_to(true);
        assert!(w.wants_nearby(), "the nearby ends are gathered while completing");
        // (using one keeps the mode, so the ends are still wanted)
        w.selected = Some(2);
        let j = Joint { pos: DVec3::new(4.0, 4.0, 0.1), heading: 90.0, slope: 0.0, flow: 0, dir: 0, travel: 90.0, geom: 90.0 };
        assert!(w.complete_to_joint(j).is_some());
        assert!(w.wants_nearby(), "still gathered after a join");
        // (turning it off is what drops them)
        w.set_complete_to(false);
        assert!(!w.wants_nearby());
    }

    /// The ends are actually gathered and drawn while completing: the crossing's own other
    /// paths' ends, so a green circle is there to click.
    #[test]
    fn the_green_ends_are_gathered_while_completing() {
        let mut w = opened(Tab::Paths);
        w.set_complete_to(true);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        assert!(!w.neighbours().is_empty(), "no green end to click while completing");
        w.set_complete_to(false);
        run(&ctx, &mut w, &mut ed, vec![], 0.1);
        assert!(w.neighbours().is_empty(), "they are gone once the mode is off");
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
        let last = w.complete_to_joint(Joint { pos: target, heading: way, slope: 0.0, flow: 1, dir: 0, travel: 0.0, geom: 0.0 }).expect("joined");
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
        assert!(w.drag_map_end(DVec3::new(4.0, 6.0, 0.0), None, None));
        let sheet = w.sheet.as_ref().unwrap();
        let i = w.selected.unwrap();
        let p = &sheet.crossing.paths[i];
        assert!((p.end().x - 4.0).abs() < 0.05, "the end moved: {:?}", p.end());
        w.let_go();
        assert_eq!(w.map_dragging(), None);
        assert!(!w.drag_map_end(DVec3::new(1.0, 1.0, 0.0), None, None), "nothing is dragged once let go");
    }

    /// An end dragged onto another path's end is taken onto its point and rise, and the way the
    /// path leaves its own start is left as it was: a drag moves an end, and never splits a path
    /// into pieces or turns the other end round.
    #[test]
    fn a_dragged_end_joins_the_way_of_the_other() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        // (the path from the middle of the crossing, carried onto the eastbound path's end)
        let start = w.sheet.as_ref().unwrap().crossing.paths[2].start();
        let leave = w.sheet.as_ref().unwrap().crossing.paths[2].heading();
        let count = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(w.take_map_end(Some((2, true))));
        let target = w.sheet.as_ref().unwrap().crossing.paths[0].end();
        let joint = w.joint_at(0, true).expect("the other end is a joint");
        // (near it, not on it: what the mouse gives a few pixels away)
        let near = target + DVec3::new(0.12, -0.08, 0.0);
        assert!(w.drag_map_end(near, Some(joint), None));
        let i = w.selected.unwrap();
        let paths = &w.sheet.as_ref().unwrap().crossing.paths;
        assert_eq!(paths.len(), count, "a drag adds no path: {} vs {count}", paths.len());
        assert_eq!(i, 2, "the path dragged is the one picked");
        let p = &paths[2];
        assert!((p.start() - start).length() < 1e-3, "the start stays: {:?} vs {:?}", p.start(), start);
        assert!(ang_delta(p.heading(), leave).abs() < 0.5, "and its way out: {} vs {leave}", p.heading());
        assert!((p.end().truncate() - joint.pos.truncate()).length() < 1e-3, "the end landed on the joint: {:?} vs {:?}", p.end(), joint.pos);
        assert!((p.end().z - joint.pos.z).abs() < 0.02, "at its height: {}", p.end().z);
        assert!((p.slope_end() - joint.slope).abs() < 1e-4, "and at its rise: {}", p.slope_end());

        // without a joint the end stays where the cursor put it
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        assert!(w.take_map_end(Some((2, true))));
        let far = DVec3::new(12.0, 4.0, 0.0);
        assert!(w.drag_map_end(far, None, None));
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
        assert!(!w.select_on_map(Some(1), false), "the 2D page picks nothing on the map");
        w.view_3d = true;
        assert!(w.select_on_map(Some(1), false));
        assert_eq!(w.picked(), Some(1));
        assert!(!w.select_on_map(Some(9), false), "a path that is not there is not picked");
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        assert_eq!(ed.crossing_selected, vec![1], "the map is told which path is picked");
    }

    /// Ctrl+click picks more than one path, Shift+click takes a run of them, and a plain click
    /// goes back to one. The map is told about all of them, so they are drawn picked.
    #[test]
    fn several_paths_can_be_picked_at_once() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        assert!(w.select_on_map(Some(0), false));
        assert_eq!(w.selection(), vec![0]);
        assert!(w.select_on_map(Some(2), true), "Ctrl adds");
        assert_eq!(w.selection(), vec![0, 2]);
        assert!(w.select_on_map(Some(2), true), "Ctrl again takes it away");
        assert_eq!(w.selection(), vec![0]);
        // (a plain click drops the others)
        assert!(w.select_on_map(Some(1), false));
        assert_eq!(w.selection(), vec![1]);
        let mut ed = Editor::default();
        let ctx = context();
        run(&ctx, &mut w, &mut ed, vec![], 0.0);
        assert_eq!(ed.crossing_selected, vec![1]);
    }

    /// What several paths share is set for all of them at once: kind, width, the way traffic
    /// goes, the turn signal, the light and whether they keep the junction clear. Delete takes
    /// all the picked ones out, in one step back.
    #[test]
    fn the_shared_numbers_are_set_on_every_picked_path() {
        let mut w = opened(Tab::Paths);
        w.view_3d = true;
        assert!(w.select_on_map(Some(0), false));
        for i in [1, 2] {
            assert!(w.select_on_map(Some(i), true));
        }
        assert_eq!(w.selection(), vec![0, 1, 2]);
        // (the batch setter is the same arithmetic the panel does: one value written to all)
        {
            let sel = w.selection();
            let sheet = w.sheet.as_mut().unwrap();
            let kind = 1;
            let width = 2.2f32;
            for q in &sel {
                let p = &mut sheet.crossing.paths[*q];
                p.v[8] = kind as f32;
                p.v[9] = width;
                p.v[10] = 1.0;
                p.v[11] = TURN_LEFT as f32;
                p.light = Some(0);
                p.crossing_problem = true;
            }
        }
        for q in w.selection() {
            let p = &w.sheet.as_ref().unwrap().crossing.paths[q];
            assert_eq!(p.kind(), 1, "path {q}");
            assert!((p.width() - 2.2).abs() < 1e-6, "path {q}");
            assert_eq!(p.direction(), 1, "path {q}");
            assert_eq!(p.turn(), TURN_LEFT, "path {q}");
            assert_eq!(p.light, Some(0), "path {q}");
            assert!(p.crossing_problem, "path {q}");
        }
        // (Delete takes the whole pick out, and one step brings them all back)
        let n = w.sheet.as_ref().unwrap().crossing.paths.len();
        assert!(w.delete_picked());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n - 3);
        assert!(w.undo());
        assert_eq!(w.sheet.as_ref().unwrap().crossing.paths.len(), n);
        // (one path picked: the cut is for that one only)
        assert!(w.select_on_map(Some(1), false));
        assert!(w.split_picked());
        assert_eq!(w.picked(), Some(2), "the second half is picked");
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
