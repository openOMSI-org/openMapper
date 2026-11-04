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

/// A number as OMSI writes it: four decimals, no trailing zeros, never `-0`.
pub fn num(v: f32) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.to_string()
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
        let c = self.curve();
        if !(c.length > 1e-6) {
            return vec![self.start()];
        }
        let n = (c.length / step.max(0.2)).ceil().max(1.0) as usize;
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
}

/// An object's `.sco` opened for editing: the parts this editor owns, and the rest of the
/// file exactly as it was.
#[derive(Clone, Debug)]
pub struct Sheet {
    /// The file's lines with the owned blocks taken out.
    rest: Vec<String>,
    /// Where in `rest` the owned blocks stood.
    at: usize,
    pub crossing: Crossing,
}

impl Sheet {
    pub fn parse(text: &str) -> Sheet {
        let lines = omsi_cfg::split_lines(text);
        let keys: Vec<Option<String>> = lines.iter().map(|l| omsi_cfg::keyword_of(l).map(str::to_lowercase)).collect();
        let mut sheet = Sheet { rest: Vec::new(), at: 0, crossing: Crossing::default() };
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
            at.get_or_insert(sheet.rest.len());
            // the values stop before the blank lines that separate it from the next block
            let mut end = j;
            while end > i + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            sheet.take(key, &lines[i + 1..end]);
            sheet.rest.extend(lines[end..j].iter().cloned());
            i = j;
        }
        sheet.at = at.unwrap_or(sheet.rest.len());
        sheet
    }

    fn take(&mut self, key: &str, lines: &[String]) {
        let v: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
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
                if let Some(p) = self.crossing.paths.last_mut() {
                    p.switch_dir = Some(n(0));
                }
            }
            "blockpath" => {
                if let Some(p) = self.crossing.paths.last_mut() {
                    p.blocks.push((n(0), n(1)));
                }
            }
            "crossingproblem" => {
                if let Some(p) = self.crossing.paths.last_mut() {
                    p.crossing_problem = true;
                }
            }
            "use_traffic_light" => {
                if let Some(p) = self.crossing.paths.last_mut() {
                    p.light = Some(n(0));
                }
            }
            "traffic_lights_group" => self.crossing.cycle = Some(f(0)),
            "traffic_light" => {
                let name = v.first().map(|s| s.to_string()).unwrap_or_default();
                self.crossing.lights.push(TrafficLight { name, ..Default::default() });
            }
            "phase" => {
                if let Some(l) = self.crossing.lights.last_mut() {
                    l.phases.push(TrafficLightPhase { state: n(0), duration: f(1) });
                }
            }
            "approachdist" => {
                if let Some(l) = self.crossing.lights.last_mut() {
                    l.approach_dist = Some(f(0));
                }
            }
            "traffic_light_stop" => self.crossing.stops.push([f(0), f(1), f(2)]),
            "traffic_light_jump" => self.crossing.jumps.push([f(0), f(1), f(2), f(3)]),
            "splinehelper" => {
                let spline = v.first().map(|s| s.to_string()).unwrap_or_default();
                self.crossing.helpers.push(Helper { spline, x: f(1), up: f(2), fwd: f(3), heading: f(4), length: f(5), radius: f(6) });
            }
            _ => {}
        }
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
    let (b, _, _) = page.encoding().encode(text);
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
        return Some("its paths are in a file of their own (`[paths]`), which this editor does not read");
    }
    Some("it has no path")
}

/// Copy the whole object folder - its `.sco`, its model, its textures, its scripts - into
/// `Sceneryobjects/openMapper/<map>/<name>/` and give the `.sco`'s new relative path. Only
/// copying the `.sco` would leave its model and its scripts behind: they are named relative
/// to the file's own folder.
pub fn copy_into(roots: &crate::install::Roots, rel: &str, map: &str, name: &str) -> anyhow::Result<String> {
    let from = find(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: {}", crate::i18n::tr("file not found", "файл не найден")))?;
    let src = from.parent().map(FsPath::to_path_buf).unwrap_or_default();
    let new_rel = crate::editable::new_rel(roots, map, name);
    let dst = omsi_cfg::resolve_path(&roots.content, &new_rel);
    let dst_dir = dst.parent().map(FsPath::to_path_buf).unwrap_or_default();
    crate::editable::copy_tree_missing(&src, &dst_dir);
    // the original `.sco` copied along under its own name goes: this one is the object
    if from.file_name() != dst.file_name() {
        if let Some(n) = from.file_name() {
            let _ = std::fs::remove_file(dst_dir.join(n));
        }
    }
    Ok(new_rel)
}

/// Write the text into `abs`, encoded as the file it came from was.
pub fn write(abs: &FsPath, text: &str, original: &[u8]) -> anyhow::Result<()> {
    crate::doc::safe::write_atomic(abs, &encode_like(text, original))?;
    Ok(())
}

/// Read a written `.sco` back and check that the game would still find everything: the paths
/// and the lights are the ones just written, and every mesh the file names is there.
pub fn verify(abs: &FsPath, crossing: &Crossing) -> anyhow::Result<()> {
    let sco = omsi_scenery::SceneryObject::load(abs).map_err(|e| anyhow::anyhow!("{e}"))?;
    if sco.paths_file.is_some() {
        anyhow::bail!("the written object names its paths in another file");
    }
    if sco.paths.len() != crossing.paths.len() {
        anyhow::bail!("{} paths were written, {} were read back", crossing.paths.len(), sco.paths.len());
    }
    if sco.traffic_lights.len() != crossing.lights.len() {
        anyhow::bail!("{} lights were written, {} were read back", crossing.lights.len(), sco.traffic_lights.len());
    }
    if sco.spline_helpers.len() != crossing.helpers.len() {
        anyhow::bail!("{} helpers were written, {} were read back", crossing.helpers.len(), sco.spline_helpers.len());
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
mod tests;
