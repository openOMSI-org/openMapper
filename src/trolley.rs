//! A trolleybus map's overhead wire, as openOMSI reads it (`trolley.txt` beside `global.cfg`,
//! see openOMSI's `docs/TROLLEYBUS.md`): the network's voltage and wire spacing, sections fed by
//! substations, spans (the centre line of the pair of wires, in the direction the positive
//! wire is on the right of), switches where three spans meet (a frog: power, indicator, radio
//! or fixed, its first position straight, to the branch or random) and insulators where two
//! sections meet. OMSI 2 has no wire of its own and ignores the file.
//!
//! Spans are laid over the kerb lane of roads, over chosen lanes of a junction, or read off the
//! map's wire splines as the game would; their joints are found as the game finds them (ends
//! within `JOIN`), so every frog gets a switch to set.

use glam::{DVec2, DVec3};

use crate::doc::{tile_origin, Document, Sel};
use crate::i18n::tr;

/// Ends closer than this are one joint (openOMSI's `omsi_trolley::wire::JOIN`).
pub const JOIN: f64 = 1.2;
/// A switch belongs to the joint of three spans nearest it within this (the game's own reach).
const SWITCH_REACH: f64 = 3.0;
/// An insulator belongs to the joint nearest it within this.
const INSULATOR_REACH: f64 = 2.0;
/// The dead stretch where two sections meet when no insulator says how long (m).
pub const DEFAULT_INSULATOR: f64 = 0.8;

#[derive(Debug, Clone, PartialEq)]
pub struct Wire {
    /// Nominal line voltage (V).
    pub voltage: f64,
    /// Distance between the two wires (m).
    pub spacing: f64,
    /// Unevenness of the contact wire (m).
    pub roughness: f64,
    /// `[trolley_net]` lines openMapper does not edit (the arcs' rules), kept as they are.
    pub net_extra: Vec<String>,
    pub sections: Vec<Section>,
    pub spans: Vec<Span>,
    pub switches: Vec<Switch>,
    pub insulators: Vec<Insulator>,
    /// Blocks openMapper does not edit (`[trolley_sounds]`…), kept as they are.
    pub other: Vec<String>,
}

impl Default for Wire {
    fn default() -> Wire {
        Wire { voltage: 600.0, spacing: 0.6, roughness: 0.0012, net_extra: Vec::new(), sections: Vec::new(), spans: Vec::new(), switches: Vec::new(), insulators: Vec::new(), other: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub id: i32,
    pub name: String,
    /// A substation feeds it.
    pub live: bool,
    pub regen_kw: f64,
    pub feeder_ohm: f64,
}

impl Section {
    pub fn new(id: i32) -> Section {
        Section { id, name: String::new(), live: true, regen_kw: 0.0, feeder_ohm: 0.05 }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub section: i32,
    /// Its own distance between the wires, else the network's.
    pub spacing: Option<f64>,
    /// The centre line of the pair, world coordinates (x east, y north, z the wires' height).
    pub points: Vec<DVec3>,
    /// The IDCodes of the wire splines and masts openMapper laid on the map for it (kept in
    /// the file as a comment the game skips): they go when the span goes.
    pub laid: Vec<i64>,
}

impl Span {
    pub fn length(&self) -> f64 {
        self.points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The current the bus draws past the contact plate: power on takes the branch.
    Power,
    /// The bus's indicator on the branch's side takes the branch.
    Indicator,
    /// Set from the cab (left / right request) or by the route code.
    Radio,
    /// Never moves.
    Fixed,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Power, Mode::Indicator, Mode::Radio, Mode::Fixed];

    fn key(self) -> &'static str {
        match self {
            Mode::Power => "power",
            Mode::Indicator => "indicator",
            Mode::Radio => "radio",
            Mode::Fixed => "fixed",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Power => tr("power (current on: branch)", "по току (с током: ответвление)"),
            Mode::Indicator => tr("indicator", "по указателю поворота"),
            Mode::Radio => tr("radio", "по радио"),
            Mode::Fixed => tr("fixed", "неподвижная"),
        }
    }
}

/// Where a switch stands when the game first reads the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Initial {
    Straight,
    Branch,
    /// Drawn at random (straight for everybody in an online game).
    Random,
}

impl Initial {
    pub const ALL: [Initial; 3] = [Initial::Straight, Initial::Branch, Initial::Random];

    fn key(self) -> &'static str {
        match self {
            Initial::Straight => "straight",
            Initial::Branch => "branch",
            Initial::Random => "random",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Initial::Straight => tr("straight", "прямо"),
            Initial::Branch => tr("to the branch", "на ответвление"),
            Initial::Random => tr("at random", "случайно"),
        }
    }
}

/// The ways a radio route code can set a switch.
pub const WAYS: [&str; 4] = ["straight", "branch", "left", "right"];

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    pub name: String,
    pub pos: DVec3,
    pub mode: Mode,
    /// Power / indicator: the contact plate's distance before the frog (m).
    pub trigger: f64,
    /// Power: the current that counts as power on (A).
    pub sense: f64,
    pub invert: bool,
    pub initial: Initial,
    /// Radio: a route code and the way it takes (one of `WAYS`).
    pub routes: Vec<(u32, String)>,
    /// Radio: only vehicles sending on this channel set it.
    pub channel: Option<u32>,
    /// Radio: reception radius (m).
    pub range: f64,
}

impl Switch {
    pub fn at(pos: DVec3) -> Switch {
        Switch { name: String::new(), pos, mode: Mode::Power, trigger: 9.0, sense: 12.0, invert: false, initial: Initial::Straight, routes: Vec::new(), channel: None, range: 150.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Insulator {
    pub pos: DVec3,
    pub length: f64,
}

// ------------------------------------------------------------------------- the file

fn words(l: &str) -> Vec<&str> {
    l.split_whitespace().collect()
}

fn nums(l: &str) -> Vec<f64> {
    l.split_whitespace().skip(1).filter_map(|w| w.parse::<f64>().ok()).collect()
}

/// The comment line of a span naming what openMapper laid for it.
const LAID: &str = "// openmapper laid";

/// `trolley.txt` read (what it does not know kept as it is).
pub fn parse(text: &str) -> Wire {
    let mut w = Wire::default();
    let mut block = String::new();
    // (a block openMapper does not edit, as written: its header and lines)
    let mut raw: Option<Vec<String>> = None;
    let mut ins: Option<Insulator> = None;
    let flush = |w: &mut Wire, raw: &mut Option<Vec<String>>, ins: &mut Option<Insulator>| {
        if let Some(r) = raw.take() {
            w.other.push(r.join("\n"));
        }
        if let Some(i) = ins.take() {
            w.insulators.push(i);
        }
    };
    for line in text.lines() {
        if let (Some(ids), "trolley_span") = (line.trim().strip_prefix(LAID), block.as_str()) {
            if let Some(s) = w.spans.last_mut() {
                s.laid.extend(ids.split_whitespace().filter_map(|x| x.parse::<i64>().ok()));
            }
            continue;
        }
        let l = line.split("//").next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        if let Some(k) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            flush(&mut w, &mut raw, &mut ins);
            block = k.trim().to_ascii_lowercase();
            match block.as_str() {
                "trolley_net" => {}
                "trolley_section" => w.sections.push(Section::new(w.sections.len() as i32 + 1)),
                "trolley_span" => w.spans.push(Span::default()),
                "trolley_switch" => w.switches.push(Switch::at(DVec3::ZERO)),
                "trolley_insulator" => ins = Some(Insulator { pos: DVec3::ZERO, length: DEFAULT_INSULATOR }),
                _ => raw = Some(vec![line.trim().to_string()]),
            }
            continue;
        }
        if let Some(r) = raw.as_mut() {
            r.push(line.trim().to_string());
            continue;
        }
        let ws = words(l);
        let key = ws[0].to_ascii_lowercase();
        let n = nums(l);
        let first = ws.get(1).copied().unwrap_or("");
        let rest = l[ws[0].len()..].trim().to_string();
        let yes = !matches!(first, "0" | "false" | "no" | "off");
        match block.as_str() {
            "trolley_net" => match key.as_str() {
                "voltage" => w.voltage = n.first().copied().unwrap_or(600.0),
                "spacing" => w.spacing = n.first().copied().unwrap_or(0.6),
                "roughness" => w.roughness = n.first().copied().unwrap_or(0.0012),
                _ => w.net_extra.push(l.to_string()),
            },
            "trolley_section" => {
                let s = w.sections.last_mut().unwrap();
                match key.as_str() {
                    "id" => s.id = n.first().copied().unwrap_or(0.0) as i32,
                    "name" => s.name = rest,
                    "live" => s.live = yes,
                    "regen_kw" => s.regen_kw = n.first().copied().unwrap_or(0.0),
                    "feeder_ohm" => s.feeder_ohm = n.first().copied().unwrap_or(0.05),
                    _ => {}
                }
            }
            "trolley_span" => {
                let s = w.spans.last_mut().unwrap();
                match key.as_str() {
                    "section" => s.section = n.first().copied().unwrap_or(0.0) as i32,
                    "spacing" => s.spacing = n.first().copied(),
                    "p" if n.len() >= 3 => s.points.push(DVec3::new(n[0], n[1], n[2])),
                    _ => {}
                }
            }
            "trolley_switch" => {
                let s = w.switches.last_mut().unwrap();
                match key.as_str() {
                    "name" => s.name = rest,
                    "pos" if n.len() >= 3 => s.pos = DVec3::new(n[0], n[1], n[2]),
                    "mode" => {
                        s.mode = match first.to_ascii_lowercase().as_str() {
                            "indicator" | "blinker" => Mode::Indicator,
                            "radio" => Mode::Radio,
                            "fixed" | "manual" => Mode::Fixed,
                            _ => Mode::Power,
                        }
                    }
                    "trigger" => s.trigger = n.first().copied().unwrap_or(9.0),
                    "sense" => s.sense = n.first().copied().unwrap_or(12.0),
                    "invert" => s.invert = yes,
                    "initial" => s.initial = Initial::ALL.into_iter().find(|i| i.key().eq_ignore_ascii_case(first)).unwrap_or(Initial::Straight),
                    "route" => {
                        if let (Ok(code), Some(way)) = (first.parse::<u32>(), ws.get(2).map(|x| x.to_ascii_lowercase()).filter(|x| WAYS.contains(&x.as_str()))) {
                            s.routes.push((code, way));
                        }
                    }
                    "channel" => s.channel = first.parse().ok(),
                    "range" => s.range = n.first().copied().unwrap_or(150.0),
                    _ => {}
                }
            }
            "trolley_insulator" => {
                if let Some(i) = ins.as_mut() {
                    match key.as_str() {
                        "pos" if n.len() >= 3 => i.pos = DVec3::new(n[0], n[1], n[2]),
                        "length" => i.length = n.first().copied().unwrap_or(DEFAULT_INSULATOR),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut w, &mut raw, &mut ins);
    w
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn point(p: DVec3) -> String {
    format!("{} {} {}", num(p.x), num(p.y), num(p.z))
}

/// The wire as `trolley.txt`.
pub fn text(w: &Wire) -> String {
    let mut s = String::from("// openMapper: the map's overhead wire for openOMSI's trolleybuses (OMSI 2 ignores it)\n\n");
    s += &format!("[trolley_net]\nvoltage {}\nspacing {}\nroughness {}\n", num(w.voltage), num(w.spacing), w.roughness);
    for l in &w.net_extra {
        s += l;
        s.push('\n');
    }
    s.push('\n');
    for c in &w.sections {
        s += &format!("[trolley_section]\nid {}\n", c.id);
        if !c.name.trim().is_empty() {
            s += &format!("name {}\n", c.name.trim());
        }
        s += &format!("live {}\nregen_kw {}\nfeeder_ohm {}\n\n", c.live as i32, num(c.regen_kw), num(c.feeder_ohm));
    }
    for sp in w.spans.iter().filter(|sp| sp.points.len() >= 2) {
        s += &format!("[trolley_span]\nsection {}\n", sp.section);
        if let Some(v) = sp.spacing {
            s += &format!("spacing {}\n", num(v));
        }
        for p in &sp.points {
            s += &format!("p {}\n", point(*p));
        }
        if !sp.laid.is_empty() {
            s += &format!("{LAID} {}\n", sp.laid.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" "));
        }
        s.push('\n');
    }
    for sw in &w.switches {
        s += "[trolley_switch]\n";
        if !sw.name.trim().is_empty() {
            s += &format!("name {}\n", sw.name.trim());
        }
        s += &format!("pos {}\nmode {}\n", point(sw.pos), sw.mode.key());
        match sw.mode {
            Mode::Power | Mode::Indicator => {
                s += &format!("trigger {}\nsense {}\ninvert {}\n", num(sw.trigger), num(sw.sense), sw.invert as i32);
            }
            Mode::Radio => {
                for (code, way) in &sw.routes {
                    s += &format!("route {code} {way}\n");
                }
                if let Some(c) = sw.channel {
                    s += &format!("channel {c}\n");
                }
                s += &format!("range {}\n", num(sw.range));
            }
            Mode::Fixed => {}
        }
        s += &format!("initial {}\n\n", sw.initial.key());
    }
    for i in &w.insulators {
        s += &format!("[trolley_insulator]\npos {}\nlength {}\n\n", point(i.pos), num(i.length));
    }
    for o in &w.other {
        s += o;
        s += "\n\n";
    }
    s
}

// ------------------------------------------------------------------------- joints

/// Span ends meeting in one point (within `JOIN`): a plain joint (two), a frog (three), an end
/// of the wire (one), or more than the game can make anything of.
#[derive(Debug, Clone)]
pub struct Joint {
    pub pos: DVec3,
    /// (span, its far end)
    pub ends: Vec<(usize, bool)>,
}

pub fn joints(w: &Wire) -> Vec<Joint> {
    let mut out: Vec<Joint> = Vec::new();
    let cell = |p: DVec3| ((p.x / JOIN).floor() as i64, (p.y / JOIN).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, sp) in w.spans.iter().enumerate() {
        if sp.points.len() < 2 {
            continue;
        }
        for far in [false, true] {
            let p = if far { *sp.points.last().unwrap() } else { sp.points[0] };
            let c = cell(p);
            let near = (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (c.0 + dx, c.1 + dy))).filter_map(|k| grid.get(&k)).flatten().copied().find(|j| out[*j].pos.distance(p) <= JOIN);
            match near {
                Some(j) => out[j].ends.push((i, far)),
                None => {
                    grid.entry(c).or_default().push(out.len());
                    out.push(Joint { pos: p, ends: vec![(i, far)] });
                }
            }
        }
    }
    out
}

/// The switch set for a frog at `p`, if there is one.
pub fn switch_at(w: &Wire, p: DVec3) -> Option<usize> {
    (0..w.switches.len()).filter(|i| w.switches[*i].pos.distance(p) < SWITCH_REACH).min_by(|a, b| w.switches[*a].pos.distance(p).total_cmp(&w.switches[*b].pos.distance(p)))
}

/// Every frog given a switch (set to the game's default) where it has none yet; switches
/// whose frog went keep their settings (and are named among the problems). How many new.
pub fn add_missing_switches(w: &mut Wire) -> usize {
    let frogs: Vec<DVec3> = joints(w).into_iter().filter(|j| j.ends.len() == 3).map(|j| j.pos).collect();
    let mut n = 0;
    for p in frogs {
        if switch_at(w, p).is_none() {
            w.switches.push(Switch::at(p));
            n += 1;
        }
    }
    n
}

/// Where two sections meet: (the joint, its insulator's length, the stored insulator if any).
pub fn section_breaks(w: &Wire, joints: &[Joint]) -> Vec<(DVec3, f64, Option<usize>)> {
    joints
        .iter()
        .filter(|j| j.ends.iter().any(|e| w.spans[e.0].section != w.spans[j.ends[0].0].section))
        .map(|j| {
            let stored = (0..w.insulators.len()).find(|i| w.insulators[*i].pos.distance(j.pos) < INSULATOR_REACH);
            (j.pos, stored.map(|i| w.insulators[i].length).unwrap_or(DEFAULT_INSULATOR), stored)
        })
        .collect()
}

/// What the game will not make sense of.
pub fn problems(w: &Wire) -> Vec<(DVec3, String)> {
    let js = joints(w);
    let mut out = Vec::new();
    for j in js.iter().filter(|j| j.ends.len() > 3) {
        out.push((j.pos, format!("{} {}", j.ends.len(), tr("spans meet in one point: the game makes no switch there (a frog is three)", "пролётов сходятся в одной точке: игра не сделает здесь стрелку (стрелка — это три)"))));
    }
    for s in &w.switches {
        if !js.iter().any(|j| j.ends.len() == 3 && j.pos.distance(s.pos) < SWITCH_REACH) {
            out.push((s.pos, format!("{} «{}»: {}", tr("Switch", "Стрелка"), s.name, tr("no three spans meet there any more", "здесь больше не сходятся три пролёта"))));
        }
    }
    for sp in &w.spans {
        if !w.sections.is_empty() && !w.sections.iter().any(|c| c.id == sp.section) {
            if let Some(p) = sp.points.first() {
                out.push((*p, format!("{} {} {}", tr("A span is in section", "Пролёт в секции"), sp.section, tr("that the network does not have", "которой нет в сети"))));
            }
        }
    }
    out
}

// ------------------------------------------------------------------------- laying it

/// How new wire is laid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lay {
    /// The wires' height over the road (m).
    pub height: f64,
    /// Over the kerb lane each way (0), only the way the road runs (1), only against it (2).
    pub ways: u8,
    pub section: i32,
    /// Traffic keeps left: the kerb lane is the leftmost.
    pub left_hand: bool,
}

impl Default for Lay {
    fn default() -> Lay {
        Lay { height: 5.6, ways: 0, section: 1, left_hand: false }
    }
}

/// A lane's points the way traffic goes on it.
fn travel(l: &crate::lanes::Lane) -> Vec<DVec3> {
    let mut v = l.pts.clone();
    if l.direction == 1 {
        v.reverse();
    }
    v
}

fn lifted(pts: &[DVec3], h: f64) -> Vec<DVec3> {
    pts.iter().map(|p| *p + DVec3::Z * h).collect()
}

/// Spans over the kerb lane of each selected road (each way, as `lay` says), one span a road
/// piece and way: the pieces of a road meet end to end, so their spans join.
pub fn over_roads(doc: &Document, assets: &mut crate::assets::Assets, sel: &[Sel], lay: Lay) -> Vec<(Span, Option<RoadLane>)> {
    let mut out = Vec::new();
    for s in sel {
        let Sel::Spline(k, id) = *s else { continue };
        let Some(sp) = doc.spline(k, id) else { continue };
        let curve = omsi_geometry::SplineCurve::from_map(sp, tile_origin(k));
        let along = (curve.end_point() - curve.point_at(0.0)).truncate();
        let centre: Vec<DVec2> = (0..=32).map(|i| curve.point_at(sp.length * i as f64 / 32.0).truncate()).collect();
        // (each street lane: its points the way traffic goes, whether it goes with the road, and
        // how far right of its own way it lies off the road's middle)
        let mut lanes: Vec<(Vec<DVec3>, bool, f64)> = Vec::new();
        for l in crate::lanes::of_one(doc, assets, *s).into_iter().filter(|l| l.kind == 0) {
            let pts = travel(&l);
            if pts.len() < 2 {
                continue;
            }
            let dir = (*pts.last().unwrap() - pts[0]).truncate();
            let with = dir.dot(along) >= 0.0;
            let mid = pts.len() / 2;
            let t = (pts[mid.min(pts.len() - 1)] - pts[mid.saturating_sub(1)]).truncate().normalize_or_zero();
            let m = pts[mid].truncate();
            let c = centre.iter().copied().min_by(|a, b| a.distance(m).total_cmp(&b.distance(m))).unwrap_or(m);
            let right = DVec2::new(t.y, -t.x);
            lanes.push((pts, with, (m - c).dot(right)));
        }
        for with in [true, false] {
            let wanted = match lay.ways {
                1 => with,
                2 => !with,
                _ => true,
            };
            if !wanted {
                continue;
            }
            let kerb = lanes.iter().filter(|l| l.1 == with).max_by(|a, b| if lay.left_hand { b.2.total_cmp(&a.2) } else { a.2.total_cmp(&b.2) });
            if let Some(l) = kerb {
                // (across the road's own way, for a wire spline laid as a copy of the road)
                let x = if with { l.2 } else { -l.2 };
                out.push((Span { section: lay.section, points: lifted(&l.0, lay.height), ..Default::default() }, Some(RoadLane { tile: k, id, x, height: lay.height })));
            }
        }
    }
    out
}

/// The road piece a span runs over and where across it: the wire is shown as a copy of the
/// road with a wire profile that far right of its line (`trolley_kit`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadLane {
    pub tile: crate::doc::TileKey,
    pub id: i64,
    pub x: f64,
    pub height: f64,
}

/// The street lanes of an object (a junction): its path number and points the way traffic
/// goes.
pub fn object_lanes(doc: &Document, assets: &mut crate::assets::Assets, s: Sel) -> Vec<(usize, Vec<DVec3>)> {
    crate::lanes::of_one(doc, assets, s).into_iter().filter(|l| l.kind == 0 && l.pts.len() >= 2).map(|l| (l.index, travel(&l))).collect()
}

/// Spans over the chosen lanes of an object.
pub fn over_lanes(lanes: &[(usize, Vec<DVec3>)], chosen: &[usize], lay: Lay) -> Vec<(Span, Option<RoadLane>)> {
    lanes.iter().filter(|l| chosen.contains(&l.0)).map(|l| (Span { section: lay.section, points: lifted(&l.1, lay.height), ..Default::default() }, None)).collect()
}

/// A spline type that is a pair of wires (two thin strips high over the road, 0.2 to 1.2 m
/// apart, as trolleybus maps for OMSI 2 draw their wire): the pair's middle across the
/// spline, its spacing and height. openOMSI reads the same splines as the wire when a map has
/// no `trolley.txt`.
pub fn wire_profile(def: &omsi_scenery::sli::Spline) -> Option<(f64, f64, f64)> {
    let mut strips: Vec<(f64, f64)> = Vec::new();
    for p in &def.profiles {
        if p.points.len() < 2 {
            continue;
        }
        let (x0, x1) = p.points.iter().fold((f32::MAX, f32::MIN), |a, q| (a.0.min(q.x), a.1.max(q.x)));
        let zmin = p.points.iter().fold(f32::MAX, |a, q| a.min(q.z));
        let zmax = p.points.iter().fold(f32::MIN, |a, q| a.max(q.z));
        if x1 - x0 <= 0.2 && zmin >= 3.0 && zmax - zmin <= 0.25 {
            strips.push((((x0 + x1) * 0.5) as f64, ((zmin + zmax) * 0.5) as f64));
        }
    }
    if def.profiles.len() > 3 || strips.len() != 2 {
        return None;
    }
    let spacing = (strips[0].0 - strips[1].0).abs();
    (0.2..=1.2).contains(&spacing).then(|| ((strips[0].0 + strips[1].0) * 0.5, spacing, (strips[0].1 + strips[1].1) * 0.5))
}

/// Spans along the map's wire splines (each piece one span at its wires' middle and height).
pub fn from_wire_splines(doc: &Document, assets: &mut crate::assets::Assets, section: i32) -> Vec<(Span, Option<RoadLane>)> {
    let mut types: std::collections::HashMap<String, Option<(f64, f64, f64)>> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        for sp in t.tile.splines.iter().filter(|s| !s.deleted && !s.file.trim().is_empty() && s.length >= 0.5) {
            let key = sp.file.to_lowercase();
            let prof = *types.entry(key).or_insert_with(|| assets.spline_wait(&sp.file, 3000).and_then(|m| wire_profile(&m.def)));
            let Some((mid, spacing, h)) = prof else { continue };
            let curve = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
            let x = if sp.mirror { -mid } else { mid };
            let n = (sp.length / 5.0).ceil().max(1.0) as usize;
            let points = (0..=n).map(|i| curve.offset_point(sp.length * i as f64 / n as f64, x, h)).collect();
            out.push((Span { section, spacing: ((spacing - 0.6).abs() > 0.01).then_some(spacing), points, ..Default::default() }, None));
        }
    }
    out
}

/// New spans into the wire, leaving out those that lie over one it has (both ends within
/// half a metre): laying it twice over a road gives no doubles. Those that went in, with their
/// places in the wire.
pub fn add_spans<T>(w: &mut Wire, new: Vec<(Span, T)>) -> Vec<(usize, T)> {
    let mut n = Vec::new();
    for (s, extra) in new.into_iter().filter(|s| s.0.points.len() >= 2) {
        let (a, b) = (s.points[0], *s.points.last().unwrap());
        let same = w.spans.iter().any(|o| o.points.len() >= 2 && o.points[0].distance(a) < 0.5 && o.points.last().unwrap().distance(b) < 0.5);
        if !same {
            if !w.sections.iter().any(|c| c.id == s.section) {
                w.sections.push(Section::new(s.section));
            }
            w.spans.push(s);
            n.push((w.spans.len() - 1, extra));
        }
    }
    add_missing_switches(w);
    n
}

/// The span nearest a point (seen from above), within `reach`: its index and the distance.
pub fn span_near(w: &Wire, p: DVec2, reach: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, sp) in w.spans.iter().enumerate() {
        for s in sp.points.windows(2) {
            let (a, b) = (s[0].truncate(), s[1].truncate());
            let ab = b - a;
            let t = if ab.length_squared() > 1e-9 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            let d = (a + ab * t).distance(p);
            if d <= reach && best.map_or(true, |x| d < x.1) {
                best = Some((i, d));
            }
        }
    }
    best.map(|b| b.0)
}

/// The map's wire made `after` (None: none, trolley.txt taken away), with what shows it on
/// the map (see `trolley_kit::apply`), one undo step.
pub fn set(ed: &mut crate::editor::Editor, roots: Option<&crate::install::Roots>, after: Option<Wire>) {
    if ed.doc.as_ref().is_none_or(|d| d.traffic.trolley == after) {
        return;
    }
    crate::trolley_kit::apply(ed, roots, None, after, Vec::new(), None);
}

/// A map's wire, or a new one with a first section.
pub fn or_new(w: Option<&Wire>) -> Wire {
    w.cloned().unwrap_or_else(|| Wire { sections: vec![Section::new(1)], ..Default::default() })
}

// ------------------------------------------------------------------------- the view

/// What the view shows of the wire.
#[derive(Default)]
pub struct WireView {
    pub show: bool,
    /// Clicks in the view pick a span or a switch (the window takes `click`).
    pub picking: bool,
    pub click: Option<DVec3>,
    pub span: Option<usize>,
    pub switch: Option<usize>,
    /// Lanes the wire may be laid over, shown while they are chosen.
    pub preview: Vec<Vec<DVec3>>,
}

/// A section's colour in the view (they repeat after six).
fn section_colour(i: usize) -> [f32; 4] {
    const C: [[f32; 4]; 6] = [[0.95, 0.75, 0.2, 1.0], [0.3, 0.8, 1.0, 1.0], [0.55, 0.95, 0.45, 1.0], [1.0, 0.5, 0.85, 1.0], [0.75, 0.6, 1.0, 1.0], [1.0, 0.6, 0.35, 1.0]];
    C[i % C.len()]
}

/// The wire drawn near the camera: each span as its two wires in its section's colour, the
/// switches, the insulators and the joints the game cannot use.
pub fn draw(out: &mut crate::render::FrameExtras, cam: &crate::camera::Camera, w: &Wire, view: &WireView, width: f32, dot: f32) {
    use crate::editor::{disc, line};
    let reach = (cam.distance * 3.0).clamp(150.0, 4000.0);
    let near = |p: DVec3| p.truncate().distance(cam.target.truncate()) < reach;
    for (i, sp) in w.spans.iter().enumerate() {
        if !sp.points.iter().any(|p| near(*p)) {
            continue;
        }
        let colour = section_colour(w.sections.iter().position(|c| c.id == sp.section).unwrap_or(0));
        let picked = view.span == Some(i);
        let half = sp.spacing.unwrap_or(w.spacing) * 0.5;
        let wd = if picked { width * 1.6 } else { width * 0.6 };
        let c = if picked { [1.0, 1.0, 1.0, 1.0] } else { colour };
        for s in sp.points.windows(2) {
            let d = (s[1] - s[0]).truncate().normalize_or_zero();
            let side = DVec3::new(d.y, -d.x, 0.0) * half;
            line(&mut out.overlay, cam, s[0] + side, s[1] + side, wd, c);
            line(&mut out.overlay, cam, s[0] - side, s[1] - side, wd * 0.7, [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, c[3]]);
        }
        // (an arrow head at the middle: the way the span runs, the positive wire on its right)
        if sp.points.len() >= 2 {
            let m = sp.points.len() / 2;
            let (a, b) = (sp.points[m.saturating_sub(1).min(sp.points.len() - 2)], sp.points[(m.max(1)).min(sp.points.len() - 1)]);
            let mid = (a + b) * 0.5;
            let d = (b - a).truncate().normalize_or_zero();
            let back = DVec3::new(-d.x, -d.y, 0.0) * 1.2;
            let side = DVec3::new(d.y, -d.x, 0.0) * 0.6;
            line(&mut out.overlay, cam, mid, mid + back + side, wd, c);
            line(&mut out.overlay, cam, mid, mid + back - side, wd, c);
        }
    }
    for l in &view.preview {
        for s in l.windows(2) {
            line(&mut out.overlay, cam, s[0] + DVec3::Z * 0.3, s[1] + DVec3::Z * 0.3, width, [1.0, 1.0, 1.0, 0.9]);
        }
    }
    let js = joints(w);
    for j in js.iter().filter(|j| near(j.pos)) {
        match j.ends.len() {
            3 => {
                let set = switch_at(w, j.pos);
                let picked = set.is_some() && set == view.switch;
                disc(&mut out.overlay, cam, j.pos, dot * if picked { 5.0 } else { 3.2 }, if picked { [1.0, 1.0, 1.0, 1.0] } else { [1.0, 0.85, 0.1, 1.0] });
            }
            n if n > 3 => disc(&mut out.overlay, cam, j.pos, dot * 3.5, [1.0, 0.25, 0.2, 1.0]),
            _ => {}
        }
    }
    for (p, _, _) in section_breaks(w, &js) {
        if near(p) {
            disc(&mut out.overlay, cam, p, dot * 2.6, [0.95, 0.95, 0.95, 1.0]);
            disc(&mut out.overlay, cam, p, dot * 1.6, [0.2, 0.2, 0.2, 1.0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[trolley_net]\nvoltage 750\nspacing 0.6\nroughness 0.0012\nfrog_arc_chance 0.5\n\n[trolley_section]\nid 1\nname Substation A\nlive 1\nregen_kw 0\nfeeder_ohm 0.05\n\n[trolley_section]\nid 2\nlive 0\n\n[trolley_span]\nsection 1\np 0 0 6\np 50 0 6\n\n[trolley_span]\nsection 1\np 50 0 6\np 100 0 6\n\n[trolley_span]\nsection 2 // the branch\np 50.3 0.2 6\np 80 30 6\n\n[trolley_switch]\nname Seeburger junction\npos 50 0 6\nmode radio\ninitial random\nroute 158 branch\nchannel 2\nrange 120\n\n[trolley_sounds]\nfrog sounds\\frog.wav\n";

    /// Read and written again it says the same (the sounds and the arc rules kept as they are).
    #[test]
    fn the_file_comes_back_the_same() {
        let w = parse(SAMPLE);
        assert_eq!(w.voltage, 750.0);
        assert_eq!(w.sections.len(), 2);
        assert!(!w.sections[1].live);
        assert_eq!(w.spans.len(), 3);
        assert_eq!(w.spans[2].section, 2);
        let s = &w.switches[0];
        assert_eq!((s.mode, s.initial, s.channel, s.range), (Mode::Radio, Initial::Random, Some(2), 120.0));
        assert_eq!(s.routes, vec![(158, "branch".to_string())]);
        assert_eq!(w.net_extra, vec!["frog_arc_chance 0.5".to_string()]);
        assert_eq!(w.other.len(), 1);
        let again = parse(&text(&w));
        assert_eq!(again, w);
        assert!(text(&w).contains("initial random\n"));
    }

    /// The three spans meeting make a frog, which has its switch; the sections meeting there
    /// have an insulator of the default length.
    #[test]
    fn frogs_and_section_breaks_are_found() {
        let mut w = parse(SAMPLE);
        let js = joints(&w);
        assert_eq!(js.iter().filter(|j| j.ends.len() == 3).count(), 1);
        assert_eq!(js.iter().filter(|j| j.ends.len() == 1).count(), 3);
        assert_eq!(add_missing_switches(&mut w), 0);
        let breaks = section_breaks(&w, &js);
        assert_eq!(breaks.len(), 1);
        assert_eq!(breaks[0].1, DEFAULT_INSULATOR);
        assert!(problems(&w).is_empty());
        // a fourth span at the frog: the game cannot use it, and it is said
        w.spans.push(Span { section: 1, spacing: None, points: vec![DVec3::new(50.0, 0.0, 6.0), DVec3::new(50.0, -40.0, 6.0)], ..Default::default() });
        assert_eq!(problems(&w).len(), 2);
    }

    /// A span laid twice is there once; a new frog gets a switch.
    #[test]
    fn spans_go_in_once_and_frogs_get_switches() {
        let mut w = Wire::default();
        let a = Span { section: 1, spacing: None, points: vec![DVec3::new(0.0, 0.0, 6.0), DVec3::new(30.0, 0.0, 6.0)], ..Default::default() };
        let b = Span { section: 1, spacing: None, points: vec![DVec3::new(30.0, 0.0, 6.0), DVec3::new(60.0, 0.0, 6.0)], ..Default::default() };
        let c = Span { section: 1, spacing: None, points: vec![DVec3::new(30.0, 0.0, 6.0), DVec3::new(50.0, 20.0, 6.0)], ..Default::default() };
        assert_eq!(add_spans(&mut w, vec![(a.clone(), ()), (b, ())]).len(), 2);
        assert_eq!(add_spans(&mut w, vec![(a, ())]).len(), 0);
        assert!(w.switches.is_empty());
        assert_eq!(add_spans(&mut w, vec![(c, ())]).len(), 1);
        assert_eq!(w.switches.len(), 1);
        assert_eq!(w.sections.len(), 1);
        assert_eq!(span_near(&w, DVec2::new(45.0, 1.0), 2.0), Some(1));
    }
}
