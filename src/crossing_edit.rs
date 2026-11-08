//! A crossing object's `[path]`s, its blocking and its traffic light program.
//!
//! The object is read as text: the blocks this editor owns are taken out of the `.sco`,
//! changed as values, and written back where they stood. Everything else in the file (its
//! meshes, its materials, its scripts, the lines openOMSI does not know) is kept exactly as
//! it was - serialising a struct back would drop all of it.
//!
//! The numbers of a `[path]` are, in the file's order:
//! `X Y Z heading radius length slope-start slope-end kind width direction turn`, where the
//! slopes are rise per metre and the turn is 0 none, 2 left, 3 right (the game's own codes:
//! `omsi-scene`'s lanes read 2 as `turn = 1`, its left). `[path_2]` carries the same twelve
//! and two more openOMSI does not use, kept as they are.

use std::path::{Path as FsPath, PathBuf};

use glam::{DVec2, DVec3};
use omsi_scenery::{TrafficLight, TrafficLightPhase};

/// No turn signal.
pub const TURN_NONE: i32 = 0;
/// The file's code for a left turn (the game's own `turn = 1`).
pub const TURN_LEFT: i32 = 2;
/// The file's code for a right turn (the game's own `turn = 2`).
pub const TURN_RIGHT: i32 = 3;

/// The keywords this editor reads and writes. Every one of them is taken out of the text and
/// put back at the place of the first of them; the rest of the file is untouched.
const OWNED: [&str; 13] = [
    "path",
    "path_2",
    "switchdir",
    "blockpath",
    "crossingproblem",
    "use_traffic_light",
    "traffic_lights_group",
    "traffic_light",
    "phase",
    "approachdist",
    "traffic_light_stop",
    "traffic_light_jump",
    "splinehelper",
];

fn owned(keyword: &str) -> bool {
    OWNED.contains(&keyword)
}

/// How many lines of values an owned keyword has, as the game reads it: what follows them is
/// free text, which stays in the file.
fn value_count(keyword: &str) -> usize {
    match keyword {
        "path" => 12,
        "path_2" => 14,
        "splinehelper" => 7,
        "traffic_light_jump" => 4,
        "traffic_light_stop" => 3,
        "blockpath" | "phase" => 2,
        "crossingproblem" => 0,
        _ => 1,
    }
}

/// Its first value is a text, which may be an empty line.
fn starts_with_text(keyword: &str) -> bool {
    matches!(keyword, "splinehelper" | "traffic_light")
}

/// Where a block's values end among the lines after its keyword.
fn values_end(keyword: &str, block: &[String]) -> usize {
    let mut left = value_count(keyword);
    let mut at = 0;
    if left > 0 && starts_with_text(keyword) && !block.is_empty() {
        left -= 1;
        at = 1;
    }
    while left > 0 && at < block.len() {
        if !block[at].trim().is_empty() {
            left -= 1;
        }
        at += 1;
    }
    at
}

/// A number as short as it reads back the same (a slope of 0.00004 stays one), never `-0`.
pub fn num(v: f32) -> String {
    if v == 0.0 {
        "0".into()
    } else {
        v.to_string()
    }
}

fn heading_deg(v: DVec2) -> f64 {
    crate::roads::heading_of(v)
}

fn norm_deg(d: f64) -> f64 {
    d.rem_euclid(360.0)
}

/// One `[path]` of the object.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// Written as `[path_2]`, whose two extra numbers `extra` holds.
    pub second: bool,
    pub extra: [f32; 2],
    /// `X Y Z heading radius length slope-start slope-end kind width direction turn`.
    pub v: [f32; 12],
    /// `[switchdir]`: the points of a rail path.
    pub switch_dir: Option<i32>,
    /// `[blockpath] path mode`: the other paths of the object this one blocks while taken.
    pub blocks: Vec<(i32, i32)>,
    /// `[crossingproblem]`: a vehicle on it keeps the junction clear.
    pub crossing_problem: bool,
    /// `[use_traffic_light]`: which of this `.sco`'s lights it waits at (0-based).
    pub light: Option<i32>,
}

impl Default for Path {
    /// A ten metre straight piece of street, 3.5 m wide, one way.
    fn default() -> Self {
        Path {
            second: false,
            extra: [0.0; 2],
            v: [0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 3.5, 0.0, 0.0],
            switch_dir: None,
            blocks: Vec::new(),
            crossing_problem: false,
            light: None,
        }
    }
}

impl Path {
    pub fn start(&self) -> DVec3 {
        DVec3::new(self.v[0] as f64, self.v[1] as f64, self.v[2] as f64)
    }

    pub fn heading(&self) -> f64 {
        self.v[3] as f64
    }

    pub fn radius(&self) -> f64 {
        self.v[4] as f64
    }

    pub fn length(&self) -> f64 {
        self.v[5] as f64
    }

    /// Rise per metre at the start.
    pub fn slope_start(&self) -> f64 {
        self.v[6] as f64
    }

    /// Rise per metre at the end.
    pub fn slope_end(&self) -> f64 {
        self.v[7] as f64
    }

    pub fn kind(&self) -> i32 {
        self.v[8] as i32
    }

    pub fn width(&self) -> f64 {
        self.v[9] as f64
    }

    /// 0 along the path, 1 against it, 2 both ways.
    pub fn direction(&self) -> i32 {
        self.v[10] as i32
    }

    pub fn turn(&self) -> i32 {
        self.v[11] as i32
    }

    /// The path as openOMSI reads an object's `[path]`.
    pub fn def(&self) -> omsi_scenery::PathDef {
        omsi_scenery::PathDef {
            kind: self.kind(),
            start: [self.v[0], self.v[1], self.v[2]],
            end: [self.v[3], self.v[4], self.v[5]],
            width: self.v[9],
            direction: self.direction(),
            params: self.v.to_vec(),
        }
    }

    /// The line it draws, in the object's own frame. The same curve the map shows for it.
    pub fn curve(&self) -> omsi_geometry::SplineCurve {
        crate::lanes::object_path_curve(&self.def())
    }

    /// Where it ends, in the object's own frame (slopes included).
    pub fn end(&self) -> DVec3 {
        let c = self.curve();
        c.point_at(c.length.max(0.0))
    }

    /// Points along it for drawing, `step` metres apart.
    pub fn points(&self, step: f64) -> Vec<DVec3> {
        self.points_by_turn(step, 180.0)
    }

    /// Points along it for drawing: `step` metres apart, and never coarser than `max_turn`
    /// degrees a piece. Laid out by the metre alone, an arc is a polygon and a bend shows its
    /// corners - 3 m across an 8 m radius is 21° a piece, a hard elbow; by the turn too, a
    /// straight line stays as few points as before while a bend is as smooth as it needs.
    pub fn points_by_turn(&self, step: f64, max_turn: f64) -> Vec<DVec3> {
        let c = self.curve();
        if !(c.length > 1e-6) {
            return vec![self.start()];
        }
        // (a broken length - a path of millions of metres - would ask for millions of points,
        // and the drawing would stop answering: the count is capped)
        const MOST: usize = 4000;
        let by_length = (c.length / step.max(0.2)).ceil().max(1.0);
        let turn = if c.radius.abs() > 1e-6 { (c.length / c.radius.abs()).to_degrees() } else { 0.0 };
        let by_turn = if max_turn > 1e-6 { (turn / max_turn).ceil() } else { 0.0 };
        let n = (by_length.max(by_turn).max(1.0) as usize).min(MOST);
        (0..=n).map(|i| c.point_at(c.length * i as f64 / n as f64)).collect()
    }

    /// Move the start to `start`, keeping the end where it is.
    pub fn set_start(&mut self, start: DVec3) {
        let end = self.end();
        self.v[0] = start.x as f32;
        self.v[1] = start.y as f32;
        self.v[2] = start.z as f32;
        self.set_end(end);
    }

    /// Put the end at `end` (x, y used; the height stays the path's own, the slopes give it).
    /// A straight path turns and stretches; an arc keeps its radius and turns to pass there,
    /// until the chord is longer than the diameter, when the radius grows to fit.
    pub fn set_end(&mut self, end: DVec3) {
        let s = self.start().truncate();
        let chord = end.truncate() - s;
        let c = chord.length();
        if c < 1e-6 {
            return;
        }
        let phi = heading_deg(chord);
        let r = self.radius();
        if r.abs() < 1e-6 {
            self.v[3] = norm_deg(phi) as f32;
            self.v[5] = c as f32;
            return;
        }
        let mut rr = r.abs();
        if c > 2.0 * rr {
            // the end is farther than the arc can reach: widen it
            rr = c / 2.0;
            self.v[4] = (rr * r.signum()) as f32;
        }
        let turn = self.v[4].signum() as f64;
        let half = (c / (2.0 * rr)).clamp(-1.0, 1.0).asin();
        self.v[3] = norm_deg(phi - turn * half.to_degrees()) as f32;
        self.v[5] = (rr * 2.0 * half) as f32;
    }

    /// A copy moved sideways by `dx` in the object's frame, so a second lane can be made.
    pub fn shifted(&self, dx: f64) -> Path {
        let mut p = self.clone();
        p.v[0] += dx as f32;
        p
    }

    /// Turn the path so that it arrives at its end in `end_deg`, its start staying where it
    /// is: the arc that does that (a straight one when the way already is the chord's).
    ///
    /// A chord's way is the mean of the tangents at its two ends, so an arc that ends in
    /// `end_deg` has turned by twice the angle between the chord and it.
    pub fn fit_end_tangent(&mut self, end_deg: f64) {
        let chord = self.end().truncate() - self.start().truncate();
        let c = chord.length();
        if c < 1e-6 {
            return;
        }
        let phi = heading_deg(chord);
        // (the heading 0 is north here: `dir` and `heading_of` are the pair that agrees)
        let d = norm_deg(end_deg - phi + 180.0) - 180.0;
        if d.abs() < 1e-3 {
            self.v[3] = norm_deg(phi) as f32;
            self.v[4] = 0.0;
            self.v[5] = c as f32;
            return;
        }
        // (nearly opposite the chord, the arc that meets it is most of a circle: its radius
        // and length grow without bound - a path of millions of metres. A straight to the
        // same end is kept instead.)
        let half = d.abs().to_radians();
        let r = c / (2.0 * half.sin());
        let length = 2.0 * half * r;
        if !length.is_finite() || !r.is_finite() || length > c * 4.0 {
            self.v[3] = norm_deg(phi) as f32;
            self.v[4] = 0.0;
            self.v[5] = c as f32;
            return;
        }
        self.v[3] = norm_deg(2.0 * phi - end_deg) as f32;
        self.v[4] = (r * d.signum()) as f32;
        self.v[5] = length as f32;
    }

    /// Turn the path so that it leaves its start in `start_deg`, its end staying where it is.
    pub fn fit_start_tangent(&mut self, start_deg: f64) {
        let chord = self.end().truncate() - self.start().truncate();
        let c = chord.length();
        if c < 1e-6 {
            return;
        }
        let phi = heading_deg(chord);
        let d = norm_deg(phi - start_deg + 180.0) - 180.0;
        if d.abs() < 1e-3 {
            self.v[3] = norm_deg(start_deg) as f32;
            self.v[4] = 0.0;
            self.v[5] = c as f32;
            return;
        }
        let half = d.abs().to_radians();
        let r = c / (2.0 * half.sin());
        let length = 2.0 * half * r;
        if !length.is_finite() || !r.is_finite() || length > c * 8.0 {
            // (a point almost directly behind cannot be reached without a loop: the widest turn
            // that still leaves in `start_deg` is taken, and the end stops where that arc
            // reaches. Turning the path round to face the point would silently change the
            // rotate the user set)
            let half = 75f64.to_radians();
            let r = c / (2.0 * half.sin());
            self.v[3] = norm_deg(start_deg) as f32;
            self.v[4] = (r * d.signum()) as f32;
            self.v[5] = (2.0 * half * r) as f32;
            return;
        }
        self.v[3] = norm_deg(start_deg) as f32;
        self.v[4] = (r * d.signum()) as f32;
        self.v[5] = length as f32;
    }

    /// Put the end's height at `z` and the rise there at `slope`, the start's height staying
    /// as it is: the rise is taken as linear, so the start's slope follows from the two.
    pub fn set_end_height_and_slope(&mut self, z: f64, slope: f64) {
        let l = self.length();
        if !(l > 1e-3) {
            return;
        }
        let s0 = 2.0 * (z - self.start().z) / l - slope;
        self.v[6] = s0 as f32;
        self.v[7] = slope as f32;
    }

    /// Put the start's height at `z` and the rise there at `slope`, the end's height staying
    /// as it is.
    pub fn set_start_height_and_slope(&mut self, z: f64, slope: f64) {
        let end_z = self.end().z;
        let l = self.length();
        self.v[2] = z as f32;
        self.v[6] = slope as f32;
        if l > 1e-3 {
            self.v[7] = (2.0 * (end_z - z) / l - slope) as f32;
        }
    }

    /// The heading traffic moves in at one of the path's ends: the way the geometry runs there,
    /// or the other way when the path is driven against it (both ways: along it).
    pub fn travel_at(&self, at_end: bool) -> f64 {
        let c = self.curve();
        let h = if at_end { c.heading_at(c.length.max(0.0)) } else { c.heading_at(0.0) };
        if self.direction() == 1 { norm_deg(h + 180.0) } else { h }
    }
}

/// An end of another path a drag has snapped onto, with the way that path runs away from it
/// and the rise it has there: what the dragged path matches, so two pieces meet not only at
/// the point but in their way and their height.
#[derive(Clone, Copy, Debug)]
pub struct Joint {
    pub pos: DVec3,
    /// The heading the other path leaves the joint in: away from the joint along it, whichever
    /// end of it this is.
    pub heading: f64,
    /// Rise per metre along the other path at the joint.
    pub slope: f64,
    /// The direction a new path laid outward from this joint should have: 0 when the other
    /// path's traffic arrives at the joint and drives on into the new one, 1 when it leaves the
    /// joint (so the new path is driven towards the joint), 2 when both.
    pub flow: i32,
    /// The other path's own `direction` (0 along, 1 against, 2 both). A new path laid out of
    /// this joint takes it as it stands: a path carried on from another is driven the way that
    /// one is, not turned round by the geometry of the joint.
    pub dir: i32,
    /// The heading traffic actually moves in at the joint, along the other path: `heading` is
    /// only the way into the body, and a path may be driven either way along it.
    pub travel: f64,
    /// The other path's own geometric tangent there, *along* it - which is exactly the rotate
    /// its end shows in the panel. A join arrives on this, so its last piece lines up with the
    /// next segment; `heading` is the other way round at a path's end.
    pub geom: f64,
    /// The other path's kind (street, footway, track). A path started from this end is the
    /// same kind, not always a street.
    pub kind: i32,
}

impl Joint {
    /// The way traffic crosses this joint - the other path's own travel there.
    pub fn through(self) -> f64 {
        self.travel
    }

    /// The way a path *begun* at this joint runs: back along the other path's body, so it
    /// leaves the joint outward rather than along the path it is put on.
    pub fn outward(self) -> f64 {
        norm_deg(self.heading + 180.0)
    }

    /// The same end, arrived at the other way round. The geometry, the way into the body and
    /// the way traffic moves there all turn about, and the slope changes sign: what was a rise
    /// along the path is a fall when the join comes in against it.
    pub fn reversed(self) -> Joint {
        Joint {
            heading: norm_deg(self.heading + 180.0),
            slope: -self.slope,
            travel: norm_deg(self.travel + 180.0),
            geom: norm_deg(self.geom + 180.0),
            ..self
        }
    }

    /// The `direction` for a path whose geometry leaves the joint in `heading`: 0 when that runs
    /// with the traffic here, 1 when it runs against it, 2 when the other path is both ways.
    /// This is what keeps a path drawn out of a lane pointing the same way the lane does instead
    /// of backwards.
    pub fn direction_for(self, heading: f64) -> i32 {
        if self.dir == 2 {
            return 2;
        }
        let d = norm_deg(heading - self.travel + 180.0) - 180.0;
        if d.abs() <= 90.0 { 0 } else { 1 }
    }
}

/// The two places a complete-to click can take at one end. `dir` is the heading the first
/// mark points, in the same frame as `pos` (the map's, when the end is drawn on the map).
/// The joint handed back is not turned: the crossing's paths are written in the object's frame.
pub fn arrival_choices(pos: DVec3, j: Joint, gap: f64, dir: f64) -> [(DVec3, Joint); 2] {
    let d = crate::roads::dir(dir) * gap;
    let off = DVec3::new(d.x, d.y, 0.0);
    [(pos + off, j), (pos - off, j.reversed())]
}

/// `local` heading (object frame) as it lies in the world, through the same matrix the
/// crossing's paths are drawn with. An arrow drawn with the bare local heading points
/// across the road whenever the object itself is turned.
pub fn heading_in_frame(local: f64, m: &glam::Mat4) -> f64 {
    let d = crate::roads::dir(local);
    let w = m.transform_vector3(glam::Vec3::new(d.x as f32, d.y as f32, 0.0)).as_dvec3().truncate();
    if w.length_squared() < 1e-8 { local } else { crate::roads::heading_of(w) }
}

/// How long the arrow on a nearby end is (m), from how far the camera is. The same length in
/// both nearby-path modes, so the two modes are told apart by one arrow or two, not by a
/// different kind of mark.
pub fn end_mark_len(distance: f64) -> f64 {
    (distance * 0.007).clamp(0.32, 1.15)
}

/// The `direction` a path whose geometry leaves a joint in `heading` should have, given the
/// heading traffic moves in there (`travel`) and whether the other path is both ways.
pub fn direction_towards(heading: f64, travel: f64, both: bool) -> i32 {
    if both {
        return 2;
    }
    let d = norm_deg(heading - travel + 180.0) - 180.0;
    if d.abs() <= 90.0 { 0 } else { 1 }
}

/// The `direction` (0 along, 1 against, 2 both) a new path laid outward from a joint of another
/// path should have. `at_end`: the joint is at that path's end rather than its start.
///
/// Traffic that arrives at the joint drives on into the new path, along its geometry; traffic
/// that leaves the joint drives into the new path from its far end, against its geometry.
pub fn outward_flow(dir: i32, at_end: bool) -> i32 {
    match dir {
        2 => 2,
        // (at the end: 0 along that path arrives here, 1 against leaves from the end;
        // at the start it is the other way round)
        0 => {
            if at_end {
                0
            } else {
                1
            }
        }
        1 => {
            if at_end {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// A `[splinehelper]`: a place where a spline of the map snaps onto this object. OMSI writes
/// its place as `x`, height, forward (height before forward, not x y z), then the heading out
/// of the object there, the length and the radius.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Helper {
    pub spline: String,
    pub x: f32,
    pub up: f32,
    pub fwd: f32,
    pub heading: f32,
    pub length: f32,
    pub radius: f32,
}

impl Helper {
    /// The place in the object's frame, as openMapper uses (x, y forward, z up).
    pub fn local(&self) -> DVec3 {
        DVec3::new(self.x as f64, self.fwd as f64, self.up as f64)
    }
}

/// Everything of an object's `.sco` this editor changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Crossing {
    pub paths: Vec<Path>,
    pub lights: Vec<TrafficLight>,
    /// `[traffic_lights_group]`: the cycle in seconds (0 or absent: the phases' own sum).
    pub cycle: Option<f32>,
    /// `[traffic_light_stop] light time if_request`.
    pub stops: Vec<[f32; 3]>,
    /// `[traffic_light_jump] light time if_request jump_to`.
    pub jumps: Vec<[f32; 4]>,
    pub helpers: Vec<Helper>,
}

impl Crossing {
    /// Remove path `i`; the later paths move up and what names them follows.
    pub fn remove_path(&mut self, i: usize) {
        if i >= self.paths.len() {
            return;
        }
        self.paths.remove(i);
        let n = i as i32;
        for p in &mut self.paths {
            p.blocks.retain(|(a, _)| *a != n);
            for (a, _) in &mut p.blocks {
                if *a > n {
                    *a -= 1;
                }
            }
        }
    }

    /// Move path `from` to `to`, renumbering the `[blockpath]`s that name paths.
    pub fn move_path(&mut self, from: usize, to: usize) {
        let len = self.paths.len();
        if from == to || from >= len || to >= len {
            return;
        }
        let p = self.paths.remove(from);
        self.paths.insert(to, p);
        let map = |old: i32| -> Option<i32> {
            let old = old.max(0) as usize;
            if old == from {
                return (to < len).then_some(to as i32);
            }
            let mut k = if old > from { old - 1 } else { old };
            if k >= to {
                k += 1;
            }
            (k < len).then_some(k as i32)
        };
        for (i, p) in self.paths.iter_mut().enumerate() {
            p.blocks = p
                .blocks
                .iter()
                .filter_map(|(a, b)| {
                    let k = map(*a)?;
                    // (a path blocking itself is no rule at all)
                    (i != to || k as usize != to).then_some((k, *b))
                })
                .collect();
        }
    }

    /// Remove light `i`; the paths that waited at it keep waiting at none, the later lights
    /// move up.
    pub fn remove_light(&mut self, i: usize) {
        if i >= self.lights.len() {
            return;
        }
        self.lights.remove(i);
        let n = i as i32;
        for p in &mut self.paths {
            p.light = match p.light {
                Some(l) if l == n => None,
                Some(l) if l > n => Some(l - 1),
                other => other,
            };
        }
        self.stops.retain(|s| s[0] as i32 != n);
        for s in &mut self.stops {
            if s[0] as i32 > n {
                s[0] -= 1.0;
            }
        }
        self.jumps.retain(|j| j[0] as i32 != n);
        for j in &mut self.jumps {
            if j[0] as i32 > n {
                j[0] -= 1.0;
            }
        }
    }

    /// The path whose end is nearest `p` within `reach` (the object's frame), for snapping:
    /// the end of the path, whether its end or its start, and the place.
    pub fn nearest_end(&self, p: DVec3, reach: f64, skip: usize) -> Option<(usize, bool, DVec3)> {
        let mut best: Option<(f64, usize, bool, DVec3)> = None;
        for (i, q) in self.paths.iter().enumerate() {
            if i == skip {
                continue;
            }
            for (at_end, at) in [(true, q.end()), (false, q.start())] {
                let d = (at.truncate() - p.truncate()).length();
                if d <= reach && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, i, at_end, at));
                }
            }
        }
        best.map(|b| (b.1, b.2, b.3))
    }

    /// The nearest end of another path, with the way that path runs away from it and the rise
    /// it has there: a path putting its own end on it can meet it in more than the point.
    pub fn nearest_joint(&self, p: DVec3, reach: f64, skip: usize) -> Option<Joint> {
        let (i, at_end, pos) = self.nearest_end(p, reach, skip)?;
        let q = &self.paths[i];
        let c = q.curve();
        // (a path whose own end is here runs *into* the joint: away from it is the other way)
        let (heading, slope) = if at_end {
            (norm_deg(c.heading_at(c.length.max(0.0)) + 180.0), q.slope_end())
        } else {
            (norm_deg(c.heading_at(0.0)), q.slope_start())
        };
        let geom = if at_end { c.heading_at(c.length.max(0.0)) } else { c.heading_at(0.0) };
        Some(Joint { pos, heading, slope, flow: outward_flow(q.direction(), at_end), dir: q.direction(), travel: q.travel_at(at_end), geom, kind: q.kind() })
    }

    /// The text of the blocks this editor owns, in the order it writes them.
    pub fn section(&self) -> String {
        let mut out = String::new();
        for p in &self.paths {
            out.push_str(if p.second { "[path_2]\n" } else { "[path]\n" });
            for x in p.v {
                out.push_str(&num(x));
                out.push('\n');
            }
            if p.second {
                for x in p.extra {
                    out.push_str(&num(x));
                    out.push('\n');
                }
            }
            if let Some(d) = p.switch_dir {
                out.push_str("[switchdir]\n");
                out.push_str(&d.to_string());
                out.push('\n');
            }
            for (a, b) in &p.blocks {
                out.push_str("[blockpath]\n");
                out.push_str(&a.to_string());
                out.push('\n');
                out.push_str(&b.to_string());
                out.push('\n');
            }
            if p.crossing_problem {
                out.push_str("[crossingproblem]\n");
            }
            if let Some(l) = p.light {
                out.push_str("[use_traffic_light]\n");
                out.push_str(&l.to_string());
                out.push('\n');
            }
            out.push('\n');
        }
        for h in &self.helpers {
            out.push_str("[splinehelper]\n");
            out.push_str(h.spline.trim());
            out.push('\n');
            for x in [h.x, h.up, h.fwd, h.heading, h.length, h.radius] {
                out.push_str(&num(x));
                out.push('\n');
            }
            out.push('\n');
        }
        if let Some(secs) = self.cycle {
            out.push_str("[traffic_lights_group]\n");
            out.push_str(&num(secs));
            out.push('\n');
        }
        for l in &self.lights {
            out.push_str("[traffic_light]\n");
            out.push_str(l.name.trim());
            out.push('\n');
            for ph in &l.phases {
                out.push_str("[phase]\n");
                out.push_str(&ph.state.to_string());
                out.push('\n');
                out.push_str(&num(ph.duration));
                out.push('\n');
            }
            if let Some(d) = l.approach_dist {
                out.push_str("[approachdist]\n");
                out.push_str(&num(d));
                out.push('\n');
            }
        }
        for s in &self.stops {
            out.push_str("[traffic_light_stop]\n");
            for x in s {
                out.push_str(&num(*x));
                out.push('\n');
            }
        }
        for j in &self.jumps {
            out.push_str("[traffic_light_jump]\n");
            for x in j {
                out.push_str(&num(*x));
                out.push('\n');
            }
        }
        out
    }

    /// The sum of the longest light's phases: what the cycle is when it is not given.
    pub fn phases_sum(&self) -> f32 {
        self.lights.iter().map(|l| l.phases.iter().map(|p| p.duration.max(0.0)).sum::<f32>()).fold(0.0, f32::max)
    }

    /// The cycle the game runs it by.
    pub fn cycle_len(&self) -> f32 {
        match self.cycle {
            Some(c) if c > 0.0 => c,
            _ => self.phases_sum().max(1.0),
        }
    }

    /// Whether the file states the cycle itself (`[traffic_lights_group]`), rather than
    /// leaving it to the longest light's phases.
    pub fn has_cycle(&self) -> bool {
        matches!(self.cycle, Some(c) if c > 0.0)
    }

    /// Where phase `phase` of light `light` starts: the phases before it laid end to end.
    pub fn phase_start(&self, light: usize, phase: usize) -> f32 {
        self.lights.get(light).map(|l| l.phases.iter().take(phase).map(|p| p.duration.max(0.0)).sum()).unwrap_or(0.0)
    }

    /// The phases of light `light` as they run over the cycle - what a timeline draws. A phase
    /// of no length takes no room; the last one holds to the cycle's end when the phases are
    /// shorter than it; what runs past the cycle is cut off.
    pub fn segments(&self, light: usize) -> Vec<Segment> {
        let len = self.cycle_len();
        let Some(l) = self.lights.get(light) else { return Vec::new() };
        let mut out = Vec::new();
        let mut t = 0.0f32;
        for (i, ph) in l.phases.iter().enumerate() {
            if t >= len {
                break;
            }
            let last = i + 1 == l.phases.len();
            let own = t + ph.duration.max(0.0);
            let end = if last { own.max(len) } else { own }.min(len);
            if end > t {
                out.push(Segment { light, phase: i, start: t, own_end: own.min(end), end, state: ph.state });
            }
            t = end;
        }
        out
    }

    /// Every light's phases, one light's after another.
    pub fn timeline(&self) -> Vec<Segment> {
        (0..self.lights.len()).flat_map(|l| self.segments(l)).collect()
    }

    /// The phase of light `light` showing at `t` seconds into the cycle.
    pub fn phase_at(&self, light: usize, t: f32) -> Option<usize> {
        self.segments(light).into_iter().find(|s| t >= s.start && t < s.end).map(|s| s.phase)
    }

    /// Make phase `phase` of light `light` end at `end` seconds into the cycle, the way its
    /// block is dragged in a timeline. Rounded to whole seconds, never before its own start
    /// and - when the file states the cycle - never past the cycle's end.
    pub fn resize_phase(&mut self, light: usize, phase: usize, end: f32) -> bool {
        let stated = self.has_cycle();
        let cycle = self.cycle_len();
        let start = self.phase_start(light, phase);
        let Some(l) = self.lights.get_mut(light) else { return false };
        if phase >= l.phases.len() {
            return false;
        }
        let want = (end - start).round().max(0.0);
        let want = if stated { want.min((cycle - start).max(0.0).round()) } else { want };
        if (l.phases[phase].duration - want).abs() < 1e-6 {
            return false;
        }
        l.phases[phase].duration = want;
        true
    }

    /// A new light: red for half its run, green for the other half. Its index.
    pub fn add_light(&mut self) -> usize {
        self.lights.push(TrafficLight {
            name: format!("Light {}", self.lights.len() + 1),
            phases: vec![TrafficLightPhase { state: 0, duration: 20.0 }, TrafficLightPhase { state: 6, duration: 20.0 }],
            approach_dist: None,
        });
        self.lights.len() - 1
    }

    /// A new red phase in light `light`, before phase `at` (at its end when `at` is past it).
    pub fn insert_phase(&mut self, light: usize, at: usize) -> Option<usize> {
        let l = self.lights.get_mut(light)?;
        let at = at.min(l.phases.len());
        l.phases.insert(at, TrafficLightPhase { state: 0, duration: 10.0 });
        Some(at)
    }

    /// Remove phase `phase` of light `light`.
    pub fn remove_phase(&mut self, light: usize, phase: usize) -> bool {
        let Some(l) = self.lights.get_mut(light) else { return false };
        if phase >= l.phases.len() {
            return false;
        }
        l.phases.remove(phase);
        true
    }
}

/// One run of a light's program on a timeline: phase `phase` of light `light`, from `start`
/// to `end` seconds into the cycle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub light: usize,
    pub phase: usize,
    pub start: f32,
    /// Where the phase's own duration ends (the last one may hold past it).
    pub own_end: f32,
    pub end: f32,
    pub state: i32,
}

/// An object's `.sco` opened for editing: the parts this editor owns, and the rest of the
/// file exactly as it was.
#[derive(Clone, Debug)]
pub struct Sheet {
    /// The file's lines with the owned blocks taken out.
    rest: Vec<String>,
    /// Where in `rest` the owned blocks stood.
    at: usize,
    /// The file's lines end in CR LF, as OMSI's own do.
    crlf: bool,
    pub crossing: Crossing,
}

impl Sheet {
    pub fn parse(text: &str) -> Sheet {
        let lines = omsi_cfg::split_lines(text);
        let keys: Vec<Option<String>> = lines.iter().map(|l| omsi_cfg::keyword_of(l).map(str::to_lowercase)).collect();
        let mut sheet = Sheet { rest: Vec::new(), at: 0, crlf: text.contains("\r\n"), crossing: Crossing::default() };
        let mut at: Option<usize> = None;
        let mut i = 0;
        while i < lines.len() {
            let Some(key) = keys[i].as_deref() else {
                sheet.rest.push(lines[i].clone());
                i += 1;
                continue;
            };
            // a block runs to the next keyword line (its blank lines included)
            let mut j = i + 1;
            while j < lines.len() && keys[j].is_none() {
                j += 1;
            }
            if !owned(key) {
                sheet.rest.extend(lines[i..j].iter().cloned());
                i = j;
                continue;
            }
            // the values stop before the blank lines that separate it from the next block
            let mut end = j;
            while end > i + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            let values = i + 1 + values_end(key, &lines[i + 1..end]);
            if sheet.take(key, &lines[i + 1..values]) {
                at.get_or_insert(sheet.rest.len());
                // (a comment after its values, and the blank lines, stay in the file)
                sheet.rest.extend(lines[values..j].iter().cloned());
            } else {
                // (nothing before it to belong to: it stays where it means what it meant,
                // ahead of the blocks written back)
                let place = at.unwrap_or(sheet.rest.len());
                sheet.rest.splice(place..place, lines[i..j].iter().cloned());
                if let Some(a) = at.as_mut() {
                    *a += j - i;
                }
            }
            i = j;
        }
        sheet.at = at.unwrap_or(sheet.rest.len());
        sheet
    }

    /// Take in a block's values. False: it has no path or light before it to belong to.
    fn take(&mut self, key: &str, lines: &[String]) -> bool {
        let text_first = starts_with_text(key) && !lines.is_empty();
        let v: Vec<&str> = lines.iter().enumerate().map(|(i, l)| (i, l.trim())).filter(|(i, l)| !l.is_empty() || (text_first && *i == 0)).map(|(_, l)| l).collect();
        let f = |i: usize| v.get(i).copied().map(omsi_cfg::parse_f32).unwrap_or(0.0);
        let n = |i: usize| v.get(i).copied().map(omsi_cfg::parse_i32).unwrap_or(0);
        match key {
            "path" | "path_2" => {
                let mut p = Path { second: key == "path_2", ..Default::default() };
                for (i, slot) in p.v.iter_mut().enumerate() {
                    *slot = f(i);
                }
                p.extra = [f(12), f(13)];
                self.crossing.paths.push(p);
            }
            "switchdir" => {
                let Some(p) = self.crossing.paths.last_mut() else { return false };
                p.switch_dir = Some(n(0));
            }
            "blockpath" => {
                let Some(p) = self.crossing.paths.last_mut() else { return false };
                p.blocks.push((n(0), n(1)));
            }
            "crossingproblem" => {
                let Some(p) = self.crossing.paths.last_mut() else { return false };
                p.crossing_problem = true;
            }
            "use_traffic_light" => {
                let Some(p) = self.crossing.paths.last_mut() else { return false };
                p.light = Some(n(0));
            }
            "traffic_lights_group" => self.crossing.cycle = Some(f(0)),
            "traffic_light" => {
                let name = v.first().map(|s| s.to_string()).unwrap_or_default();
                self.crossing.lights.push(TrafficLight { name, ..Default::default() });
            }
            "phase" => {
                let Some(l) = self.crossing.lights.last_mut() else { return false };
                l.phases.push(TrafficLightPhase { state: n(0), duration: f(1) });
            }
            "approachdist" => {
                let Some(l) = self.crossing.lights.last_mut() else { return false };
                l.approach_dist = Some(f(0));
            }
            "traffic_light_stop" => self.crossing.stops.push([f(0), f(1), f(2)]),
            "traffic_light_jump" => self.crossing.jumps.push([f(0), f(1), f(2), f(3)]),
            "splinehelper" => {
                let spline = v.first().map(|s| s.to_string()).unwrap_or_default();
                self.crossing.helpers.push(Helper { spline, x: f(1), up: f(2), fwd: f(3), heading: f(4), length: f(5), radius: f(6) });
            }
            _ => {}
        }
        true
    }

    /// The whole `.sco` with the owned blocks written back where they stood.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for l in self.rest.iter().take(self.at) {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str(&self.crossing.section());
        for l in self.rest.iter().skip(self.at) {
            out.push_str(l);
            out.push('\n');
        }
        if self.crlf {
            out = out.replace('\n', "\r\n");
        }
        out
    }

    /// Whether anything the editor owns was found in the file at all.
    pub fn has_paths(&self) -> bool {
        !self.crossing.paths.is_empty()
    }
}

/// Encode `text` the way the file it comes from was encoded: its byte-order mark, or the
/// code page it was detected in. Writing everything as Windows-1252 would lose Cyrillic.
pub fn encode_like(text: &str, original: &[u8]) -> Vec<u8> {
    if original.starts_with(&[0xFF, 0xFE]) {
        let mut out = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        return out;
    }
    if original.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let mut out = vec![0xEF, 0xBB, 0xBF];
        out.extend_from_slice(text.as_bytes());
        return out;
    }
    let page = omsi_cfg::codepage::detect(original);
    let (b, _, lost) = page.encoding().encode(text);
    if lost {
        // (a name that code page cannot hold: with a byte order mark nothing is lost)
        let mut out = vec![0xEF, 0xBB, 0xBF];
        out.extend_from_slice(text.as_bytes());
        return out;
    }
    b.into_owned()
}

/// Where the object's file is: the content folder first, then the installation.
pub fn find(roots: &crate::install::Roots, rel: &str) -> Option<PathBuf> {
    [omsi_cfg::resolve_path(&roots.content, rel), omsi_cfg::resolve_path(&roots.omsi, rel)].into_iter().find(|p| p.is_file())
}

/// Read an object's `.sco`: its bytes (for the code page) and the sheet.
pub fn open(roots: &crate::install::Roots, rel: &str) -> anyhow::Result<(PathBuf, Vec<u8>, Sheet)> {
    let path = find(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: {}", crate::i18n::tr("file not found", "файл не найден")))?;
    let bytes = std::fs::read(&path)?;
    let text = omsi_cfg::decode_text(&bytes);
    Ok((path, bytes, Sheet::parse(&text)))
}

/// Why this object cannot be edited here, if it cannot.
pub fn why_not(roots: &crate::install::Roots, rel: &str) -> Option<&'static str> {
    let path = find(roots, rel)?;
    let text = omsi_cfg::decode_text(&std::fs::read(path).ok()?);
    if Sheet::parse(&text).has_paths() {
        return None;
    }
    // `[paths]`: the paths live in a file of their own, which openMapper does not read
    if omsi_cfg::split_lines(&text).iter().any(|l| omsi_cfg::keyword_of(l).map(|k| k.eq_ignore_ascii_case("paths")).unwrap_or(false)) {
        return Some(crate::i18n::tr("its paths are in a file of their own (`[paths]`), which this editor does not read", "его пути в отдельном файле (`[paths]`), который этот редактор не читает"));
    }
    Some(crate::i18n::tr("it has no path", "у него нет путей"))
}

/// `..` taken out of a path by its words alone.
fn tidy(p: &FsPath) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// The files an object's text names, found where the game looks from `dir`, and those their own
/// lists name (a model's `.cfg`, a sound's): meshes, scripts, variable lists, sounds.
fn named_files(text: &str, dir: &FsPath, depth: u32, out: &mut Vec<PathBuf>) {
    for line in omsi_cfg::split_lines(text) {
        let line = line.trim();
        if line.is_empty() || omsi_cfg::keyword_of(line).is_some() || line.parse::<f64>().is_ok() || !line.contains('.') {
            continue;
        }
        let found = [dir.to_path_buf(), dir.join("model"), dir.join("script"), dir.join("sound")].iter().map(|b| omsi_cfg::resolve_path(b, line)).find(|p| p.is_file());
        let Some(found) = found.map(|p| tidy(&p)) else { continue };
        if out.contains(&found) {
            continue;
        }
        out.push(found.clone());
        let list = found.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg") || x.eq_ignore_ascii_case("txt"));
        if list && depth > 0 {
            if let (Ok(b), Some(parent)) = (std::fs::read(&found), found.parent()) {
                named_files(&omsi_cfg::decode_text(&b), parent, depth - 1, out);
            }
        }
    }
}

fn copy_tree(from: &FsPath, to: &FsPath) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dst)?;
        } else if !dst.exists() {
            std::fs::copy(e.path(), &dst).map_err(|err| anyhow::anyhow!("{}: {err}", e.path().display()))?;
        }
    }
    Ok(())
}

/// Copy the object - its `.sco`, its model, its textures, its scripts - into
/// `Sceneryobjects/openMapper/<map>/<name>/` and give the `.sco`'s new relative path. Only
/// copying the `.sco` would leave its model and its scripts behind: they are named relative
/// to the file's own folder. A folder of its own comes whole. A folder it shares with other
/// objects (openMapper's own junctions are flat files of the map's folder, where the copy
/// itself goes) gives only the files this one names, and the textures.
pub fn copy_into(roots: &crate::install::Roots, rel: &str, map: &str, name: &str) -> anyhow::Result<String> {
    let from = find(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: {}", crate::i18n::tr("file not found", "файл не найден")))?;
    let src = tidy(from.parent().unwrap_or(FsPath::new("")));
    let mut named = Vec::new();
    named_files(&omsi_cfg::decode_text(&std::fs::read(&from)?), &src, 3, &mut named);
    if let Some(out) = named.iter().find(|p| !p.starts_with(&src)) {
        anyhow::bail!("{}: {}", out.display(), crate::i18n::tr("the object names this file outside its own folder, which a copy would lose", "объект называет этот файл вне своей папки, копия его потеряет"));
    }
    let new_rel = crate::editable::new_rel(roots, map, name);
    let dst = omsi_cfg::resolve_path(&roots.content, &new_rel);
    let dst_dir = tidy(dst.parent().unwrap_or(FsPath::new("")));
    let others = std::fs::read_dir(&src)?.flatten().any(|e| {
        let p = e.path();
        p != from && p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sco"))
    });
    let copied = (|| -> anyhow::Result<()> {
        if others || dst_dir.starts_with(&src) {
            std::fs::create_dir_all(&dst_dir)?;
            for f in &named {
                let to = dst_dir.join(f.strip_prefix(&src)?);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(f, &to).map_err(|err| anyhow::anyhow!("{}: {err}", f.display()))?;
            }
            for e in std::fs::read_dir(&src)?.flatten() {
                if e.path().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case("texture") {
                    copy_tree(&e.path(), &dst_dir.join(e.file_name()))?;
                }
            }
        } else {
            copy_tree(&src, &dst_dir)?;
            // the original `.sco` copied along under its own name goes: this one is the object
            if from.file_name() != dst.file_name() {
                if let Some(n) = from.file_name() {
                    std::fs::remove_file(dst_dir.join(n))?;
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&dst_dir);
        return Err(e);
    }
    Ok(new_rel)
}

/// Take away a copy `copy_into` made, when it did not come to be used. Only ever one of
/// openMapper's own.
pub fn discard_copy(roots: &crate::install::Roots, rel: &str) {
    let dst = omsi_cfg::resolve_path(&roots.content, rel);
    let own = tidy(&omsi_cfg::resolve_path(&roots.content, "Sceneryobjects\\openMapper"));
    if let Some(dir) = dst.parent().map(tidy).filter(|d| d.starts_with(&own) && *d != own) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Write the text into `abs`, encoded as the file it came from was.
pub fn write(abs: &FsPath, text: &str, original: &[u8]) -> anyhow::Result<()> {
    crate::doc::safe::write_atomic(abs, &encode_like(text, original))?;
    Ok(())
}

/// Write the text beside `abs`, read it back, and only if it checks out put it in place: a
/// save that fails leaves the file as it was.
pub fn write_checked(abs: &FsPath, text: &str, original: &[u8], crossing: &Crossing) -> anyhow::Result<()> {
    let trial = abs.with_extension("crossing-check.sco");
    let done = write(&trial, text, original).and_then(|_| verify(&trial, crossing)).and_then(|_| std::fs::rename(&trial, abs).map_err(Into::into));
    if done.is_err() {
        let _ = std::fs::remove_file(&trial);
    }
    done
}

/// The backup of `abs` taken before it is written over: the file's own name with `.bak` after
/// it, in its own folder (`Kreuzung.sco.bak`).
pub fn bak_path(abs: &FsPath) -> std::path::PathBuf {
    let mut s = abs.as_os_str().to_os_string();
    s.push(".bak");
    std::path::PathBuf::from(s)
}

/// Write the text into `abs` itself, the file as it stood before the first such save kept as
/// `abs.bak`: the crossing editor's ordinary save. A `.bak` that is there already is left as it
/// is - it holds the file from before the editor's first save (or somebody else's backup), and
/// a second save must not put the first one's text in its place. False when the text is the
/// file's own already (nothing is written, and the backup is left as it was).
pub fn write_in_place(abs: &FsPath, text: &str, original: &[u8], crossing: &Crossing) -> anyhow::Result<bool> {
    let encoded = encode_like(text, original);
    if encoded == original {
        return Ok(false);
    }
    // (checked beside the file first: a save that fails never touches what is there)
    let trial = abs.with_extension("crossing-check.sco");
    let checked = write(&trial, text, original).and_then(|_| verify(&trial, crossing));
    if let Err(e) = checked {
        let _ = std::fs::remove_file(&trial);
        return Err(e);
    }
    // (the file as it stands becomes the backup, unless there is one, then the checked one
    // takes its place)
    let bak = bak_path(abs);
    if !bak.exists() {
        let kept = std::fs::copy(abs, &bak).map_err(|e| anyhow::anyhow!("{}: {e}", bak.display()));
        if let Err(e) = kept {
            let _ = std::fs::remove_file(&trial);
            return Err(e);
        }
    }
    if let Err(e) = std::fs::rename(&trial, abs) {
        let _ = std::fs::remove_file(&trial);
        return Err(e.into());
    }
    Ok(true)
}

/// Take a file's `.bak` away: true when there was one.
pub fn remove_bak(abs: &FsPath) -> bool {
    std::fs::remove_file(bak_path(abs)).is_ok()
}

/// The backup the editor made of a file, if it is one it noted (so clearing never takes a
/// `.bak` somebody else made).
pub fn bak_of(abs: &FsPath, roots: &crate::install::Roots) -> Option<std::path::PathBuf> {
    let b = bak_path(abs);
    baks::list(roots).contains(&b).then_some(b)
}

/// The list of `.bak`s this editor itself made, kept beside its content so clearing them can
/// be offered without scanning a whole installation.
pub mod baks {
    use std::path::{Path, PathBuf};

    fn list_file(roots: &crate::install::Roots) -> PathBuf {
        omsi_cfg::resolve_path(&roots.content, "openMapper").join("crossing-baks.txt")
    }

    /// Note a backup as the editor's own.
    pub fn note(roots: &crate::install::Roots, bak: &Path) {
        let f = list_file(roots);
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut lines = std::fs::read_to_string(&f).unwrap_or_default();
        let entry = bak.to_string_lossy().to_string();
        if lines.lines().any(|l| l == entry) {
            return;
        }
        if !lines.is_empty() && !lines.ends_with('\n') {
            lines.push('\n');
        }
        lines.push_str(&entry);
        lines.push('\n');
        let _ = std::fs::write(&f, lines);
    }

    /// The backups the editor made that are still there.
    pub fn list(roots: &crate::install::Roots) -> Vec<PathBuf> {
        let text = std::fs::read_to_string(list_file(roots)).unwrap_or_default();
        text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).filter(|p| p.is_file()).collect()
    }

    /// Take them away: how many went.
    pub fn clear(roots: &crate::install::Roots) -> usize {
        let mut gone = 0;
        for p in list(roots) {
            if std::fs::remove_file(&p).is_ok() {
                gone += 1;
            }
        }
        let _ = std::fs::remove_file(list_file(roots));
        gone
    }
}

/// Read a written `.sco` back and check that the game would still find everything: the paths
/// and the lights are the ones just written, and every mesh the file names is there.
pub fn verify(abs: &FsPath, crossing: &Crossing) -> anyhow::Result<()> {
    let sco = omsi_scenery::SceneryObject::load(abs).map_err(|e| anyhow::anyhow!("{e}"))?;
    if sco.paths_file.is_some() || sco.paths.len() != crossing.paths.len() || sco.traffic_lights.len() != crossing.lights.len() || sco.spline_helpers.len() != crossing.helpers.len() {
        anyhow::bail!("{}", crate::i18n::tr("what was written does not read back the same", "записанное читается не так, как записано"));
    }
    let dir = abs.parent().map(FsPath::to_path_buf).unwrap_or_default();
    let model = dir.join("model");
    let mut named: Vec<String> = sco.model.meshes.iter().map(|m| m.file.clone()).collect();
    if let Some(f) = &sco.model_file {
        named.push(f.clone());
    }
    for f in named {
        if omsi_cfg::vfs::is_file(&omsi_cfg::resolve_path(&dir, &f)) || omsi_cfg::vfs::is_file(&omsi_cfg::resolve_path(&model, &f)) {
            continue;
        }
        anyhow::bail!("{}: {}", crate::i18n::tr("a file the object names is missing", "файл, названный объектом, потерян"), f);
    }
    Ok(())
}

#[cfg(test)]
mod arrival {
    use super::*;
    use glam::DVec3;

    #[test]
    fn reversing_a_joint_turns_the_way_about_and_the_slope_over() {
        let j = Joint { pos: DVec3::new(1.0, 2.0, 3.0), heading: 20.0, slope: 0.04, flow: 1, dir: 0, travel: 20.0, geom: 10.0, kind: 0 };
        let r = j.reversed();
        assert!((r.geom - 190.0).abs() < 1e-6 && (r.heading - 200.0).abs() < 1e-6 && (r.travel - 200.0).abs() < 1e-6);
        assert!((r.slope + 0.04).abs() < 1e-9);
        assert_eq!((r.pos, r.flow, r.dir), (j.pos, j.flow, j.dir));
        let back = r.reversed();
        assert!((back.geom - j.geom).abs() < 1e-6 && (back.slope - j.slope).abs() < 1e-9);
    }

    #[test]
    fn the_two_arrivals_sit_opposite_each_other() {
        let j = Joint { pos: DVec3::ZERO, heading: 0.0, slope: 0.1, flow: 0, dir: 0, travel: 0.0, geom: 0.0, kind: 0 };
        let [a, b] = arrival_choices(DVec3::new(4.0, 5.0, 1.0), j, 1.2, j.geom);
        assert!((a.0 - b.0).length() > 2.0, "the two marks are apart");
        assert!((a.1.geom - j.geom).abs() < 1e-6 && (a.1.slope - 0.1).abs() < 1e-9);
        assert!((b.1.geom - 180.0).abs() < 1e-6 && (b.1.slope + 0.1).abs() < 1e-9);
        assert_eq!(a.1.pos, j.pos);
        // (a joint's heading is the object's: turning the object's north onto the world's
        // east must turn the arrow with it)
        let m = glam::Mat4::from_cols(glam::Vec4::new(0.0, -1.0, 0.0, 0.0), glam::Vec4::new(1.0, 0.0, 0.0, 0.0), glam::Vec4::Z, glam::Vec4::W);
        let h = heading_in_frame(0.0, &m);
        assert!((h - 90.0).abs() < 0.1, "north in the object is east on the map, got {h}");
    }
}

#[cfg(test)]
mod tests;
