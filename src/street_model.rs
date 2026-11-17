//! The street model (see docs/street-model.md): OpenStreetMap taken as evidence, the streets a road
//! designer would draw from it built instead of its ways one by one.
//!
//! This part: layer A (normalise and classify the ways), layer B (streets: chains through
//! degree-2 nodes, divided streets from pairs of one-way ways, sidewalk lines absorbed into the
//! street they run beside) and layer C (each axis fitted as tangent-continuous straights and arcs).

use std::collections::{HashMap, HashSet};

use glam::DVec2;

use crate::roads::{self, Seg};

/// The cell of the grids the network is searched in (m).
const GRID: f64 = 40.0;

/// What a way is for the street model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// A carriageway of a road class (0 main, 1 street, 2 service).
    Carriageway(u8),
    /// A footway or path (built as a path unless it turns out to be a sidewalk or a crossing).
    Foot,
    /// Parking aisles and the like: not a road of its own.
    Parking,
}

/// One way of the input, in world metres.
#[derive(Clone, Debug)]
pub struct RawWay {
    pub id: i64,
    pub tags: HashMap<String, String>,
    pub nodes: Vec<i64>,
    pub pts: Vec<DVec2>,
}

impl RawWay {
    pub fn tag(&self, k: &str) -> &str {
        self.tags.get(k).map(String::as_str).unwrap_or("")
    }
}

/// The role of a way from its tags (None: not part of the model: buildings' outlines etc.).
pub fn role(w: &RawWay) -> Option<Role> {
    let h = w.tag("highway");
    if w.tag("area") == "yes" || w.tag("tunnel") == "building_passage" {
        return None;
    }
    // (a footway on a bridge, in a tunnel or anything off the ground level is a structure: it is
    // not laid on the ground; the map's own decks and stairs carry it. A carriageway there is a
    // road all the same, on its own level: it passes over or under the others, never meeting
    // them where they cross, and its heights are its own)
    let off_ground = !matches!(w.tag("bridge"), "" | "no") || !matches!(w.tag("tunnel"), "" | "no") || !matches!(w.tag("layer"), "" | "0");
    let base = h.trim_end_matches("_link");
    let class = match base {
        "motorway" | "trunk" | "primary" | "secondary" => Some(0),
        "tertiary" | "unclassified" | "residential" | "living_street" => Some(1),
        "service" => Some(2),
        _ => None,
    };
    if let Some(c) = class {
        if off_ground && c == 2 && w.tag("tunnel") != "" && w.tag("tunnel") != "no" {
            return None;
        }
        if c == 2 && matches!(w.tag("service"), "parking_aisle" | "drive-through") {
            return Some(Role::Parking);
        }
        return Some(Role::Carriageway(c));
    }
    (!off_ground && matches!(h, "footway" | "path" | "pedestrian" | "steps" | "cycleway")).then_some(Role::Foot)
}

pub fn is_oneway(w: &RawWay) -> bool {
    matches!(w.tag("oneway"), "yes" | "1" | "true") || w.tag("junction") == "roundabout"
}

/// Default carriageway width (m) of a class when nothing better is known.
pub fn default_width(class: u8, oneway: bool) -> f64 {
    match (class, oneway) {
        (0, false) => 14.0,
        (0, true) => 7.0,
        (1, false) => 7.0,
        (1, true) => 4.0,
        _ => 5.5,
    }
}

/// The smallest corner radius (m) a class's axis may have where the legs allow it.
pub fn min_radius(class: u8) -> f64 {
    match class {
        0 => 40.0,
        1 => 15.0,
        2 => 8.0,
        _ => 5.0,
    }
}
// ---------------------------------------------------------------- layer C: alignment

fn dist_to_seg(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    let l = d.length_squared();
    if l < 1e-12 {
        return (p - a).length();
    }
    let t = ((p - a).dot(d) / l).clamp(0.0, 1.0);
    (p - (a + d * t)).length()
}

/// Douglas–Peucker with tolerance `tol`, the ends kept.
pub fn simplify(p: &[DVec2], tol: f64) -> Vec<DVec2> {
    fn keep(p: &[DVec2], lo: usize, hi: usize, tol: f64, out: &mut Vec<bool>) {
        if hi <= lo + 1 {
            return;
        }
        let (mut far, mut at) = (0.0, lo);
        for i in lo + 1..hi {
            let d = dist_to_seg(p[i], p[lo], p[hi]);
            if d > far {
                far = d;
                at = i;
            }
        }
        if far > tol {
            out[at] = true;
            keep(p, lo, at, tol, out);
            keep(p, at, hi, tol, out);
        }
    }
    if p.len() < 3 {
        return p.to_vec();
    }
    let mut k = vec![false; p.len()];
    k[0] = true;
    *k.last_mut().unwrap() = true;
    keep(p, 0, p.len() - 1, tol, &mut k);
    p.iter().zip(k).filter(|(_, k)| *k).map(|(q, _)| *q).collect()
}

fn turn_at(a: DVec2, v: DVec2, b: DVec2) -> f64 {
    // signed turn (degrees, + right) going a→v→b
    roads::norm_deg(roads::heading_of(b - v) - roads::heading_of(v - a) + 180.0) - 180.0
}
/// Node noise smoothed out of a polyline, its real corners kept: resampled every 2 m, the corners
/// found (the direction turning more than 35° within 8 m either side), each run between corners
/// averaged over 14 m with its ends held, then simplified to 0.25 m.
pub fn smooth(points: &[DVec2]) -> Vec<DVec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let r = resample(points, 2.0);
    let n = r.len();
    if n < 9 {
        return simplify(points, 0.25);
    }
    let k = 4;
    let ang = |i: usize| -> f64 {
        let a = r[i.saturating_sub(k)];
        let b = r[(i + k).min(n - 1)];
        (roads::norm_deg(roads::heading_of(b - r[i]) - roads::heading_of(r[i] - a) + 180.0) - 180.0).abs()
    };
    let mut corners: Vec<usize> = vec![0];
    let mut i = k;
    while i + k < n {
        if ang(i) > 35.0 {
            // the sharpest point of this bend
            let mut best = i;
            let mut j = i;
            while j + k < n && ang(j) > 35.0 {
                if ang(j) > ang(best) {
                    best = j;
                }
                j += 1;
            }
            corners.push(best);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    corners.push(n - 1);
    let mut out: Vec<DVec2> = Vec::new();
    for w in corners.windows(2) {
        let (a, b) = (w[0], w[1]);
        let run = &r[a..=b];
        let m = run.len();
        let half = 3usize;
        let sm: Vec<DVec2> = (0..m)
            .map(|i| {
                if i == 0 || i == m - 1 {
                    return run[i];
                }
                let h = half.min(i).min(m - 1 - i);
                let sum: DVec2 = run[i - h..=i + h].iter().copied().sum();
                sum / (2 * h + 1) as f64
            })
            .collect();
        if !out.is_empty() {
            out.pop();
        }
        out.extend(sm);
    }
    simplify(&out, 0.25)
}

/// The axis through `points` as straights and arcs, tangent-continuous everywhere: the node noise
/// smoothed out ([`smooth`]), each vertex then rounded with the class's radius (a corner, less only
/// where its legs are short) or as widely as its legs allow (a bend); a vertex whose legs are too
/// short for a 3 m curve is dropped rather than left a sharp corner.
pub fn fit(points: &[DVec2], class: u8) -> Vec<Seg> {
    let mut p = smooth(points);
    let rmin = min_radius(class);
    loop {
        let mut worst = None;
        for i in 1..p.len().saturating_sub(1) {
            let t = turn_at(p[i - 1], p[i], p[i + 1]).abs().to_radians();
            if t < 0.2f64.to_radians() {
                continue;
            }
            let room = 0.5 * (p[i] - p[i - 1]).length().min((p[i + 1] - p[i]).length());
            if room / (t * 0.5).tan() < 3.0 {
                worst = Some(i);
                break;
            }
        }
        match worst {
            Some(i) if p.len() > 2 => {
                p.remove(i);
            }
            _ => break,
        }
    }
    let mut out = Vec::new();
    if p.len() < 2 {
        return out;
    }
    let mut from = p[0];
    for i in 1..p.len() - 1 {
        let (a, v, b) = (p[i - 1], p[i], p[i + 1]);
        let (h_in, h_out) = (roads::heading_of(v - a), roads::heading_of(b - v));
        let t = (roads::norm_deg(h_out - h_in + 180.0) - 180.0).abs().to_radians();
        if t < 0.2f64.to_radians() {
            out.extend(roads::straight(from, v));
            from = v;
            continue;
        }
        let room = (0.5 * (v - a).length().min((b - v).length())).min((v - from).length());
        let r_room = room / (t * 0.5).tan();
        // a corner takes the class's radius (less only where its legs are short); a bend is drawn
        // out as widely as its legs allow
        let r = if t > 25f64.to_radians() { r_room.min(rmin) } else { r_room };
        let d = r * (t * 0.5).tan();
        let t1 = v + roads::dir(h_in) * -d;
        let t2 = v + roads::dir(h_out) * d;
        match roads::arc_to_any(t1, h_in, t2) {
            Some(s) if d > 0.05 => {
                out.extend(roads::straight(from, t1));
                out.push(s);
                from = t2;
            }
            _ => {
                out.extend(roads::straight(from, v));
                from = v;
            }
        }
    }
    out.extend(roads::straight(from, *p.last().unwrap()));
    out
}

/// Largest heading break (degrees) between consecutive pieces.
/// [`fit`], tangent-continuous for certain: where the fit leaves a break (a step or a kink at
/// a joint), the line is laid instead as biarcs through its smoothed points, which meet G1 by
/// construction.
pub fn fit_g1(points: &[DVec2], class: u8) -> Vec<Seg> {
    let s = fit(points, class);
    if !s.is_empty() && max_break(&s) < 0.02 && max_gap(&s) < 0.01 {
        return s;
    }
    // (its ends stay where they are: they are the model's nodes)
    let mut sm = resample(&smooth(points), 4.0);
    if let (Some(a), Some(b), true) = (points.first(), points.last(), sm.len() >= 2) {
        let n = sm.len();
        sm[0] = *a;
        sm[n - 1] = *b;
    }
    match crate::ed_author::chain_through(&sm, None, None) {
        Ok(c) if !c.is_empty() => c,
        _ => s,
    }
}

/// The largest step between one piece's end and the next one's start (m).
pub fn max_gap(segs: &[Seg]) -> f64 {
    segs.windows(2).map(|w| (w[0].end() - w[1].start).length()).fold(0.0, f64::max)
}

pub fn max_break(segs: &[Seg]) -> f64 {
    segs.windows(2).map(|w| (roads::norm_deg(w[1].heading - w[0].end_heading() + 180.0) - 180.0).abs()).fold(0.0, f64::max)
}

// ---------------------------------------------------------------- layers A and B: streets

/// A street of the model: its axis (world), class, divided (median width) or one-way, cross-section.
#[derive(Clone, Debug)]
pub struct Street {
    pub ways: Vec<i64>,
    pub name: String,
    pub class: u8,
    pub oneway: bool,
    /// Distance between the two carriageways' axes for a divided street (0: not divided).
    pub median: f64,
    pub width: f64,
    /// Sidewalk widths left and right of the axis (in its direction), from absorbed sidewalk lines.
    pub sidewalk: [Option<f64>; 2],
    pub axis: Vec<DVec2>,
    /// Its end nodes (model node ids).
    pub ends: [usize; 2],
    /// A slip road (`*_link`): it leaves or joins a road beside it, never across.
    pub link: bool,
}

/// Where a one-way road joins another (a merge) or leaves it (a diverge) at a shallow angle:
/// built as a road designer lays a slip road in, never as a T. The slip road comes alongside
/// the main road, edge to edge, and one long plate carries both into the road beyond (a
/// merge: the acceleration lane and its taper), or the road before into both (a diverge).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Merge {
    pub node: usize,
    /// The main road running into the node, and on out of it.
    pub main_in: usize,
    pub main_out: usize,
    /// The slip road: it ends at the node (a merge) or starts there (a diverge).
    pub ramp: usize,
    pub diverge: bool,
    /// The slip road lies to the right of the main road's way (else to its left).
    pub right: bool,
}

/// The model's network: nodes (merged OSM nodes) and streets.
#[derive(Clone, Debug, Default)]
pub struct Network {
    pub nodes: Vec<DVec2>,
    pub streets: Vec<Street>,
    /// Footways kept as paths (sidewalk lines and crossings taken out).
    pub paths: Vec<Vec<DVec2>>,
    /// Ways recognised as sidewalks: (way id, street index, side 0 left / 1 right, offset from the axis).
    pub sidewalks: Vec<(i64, usize, usize, f64)>,
    /// Footways crossing a carriageway: (street index, point).
    pub crossings: Vec<(usize, DVec2)>,
    /// The paths as a network: their meeting points and the pieces between them.
    pub path_nodes: Vec<DVec2>,
    pub path_edges: Vec<(Vec<DVec2>, [usize; 2])>,
    /// Merges and diverges of one-way roads (see `Merge`).
    pub merges: Vec<Merge>,
}

impl Network {
    /// Degree of a node (street ends there).
    pub fn degree(&self, n: usize) -> usize {
        self.streets.iter().map(|s| s.ends.iter().filter(|e| **e == n).count()).sum()
    }

    /// The way along a street's axis at one of its ends, taken over `over` metres (the
    /// direction the axis runs in, start to end).
    pub fn way_at(&self, si: usize, at_end: bool, over: f64) -> DVec2 {
        let a = &self.streets[si].axis;
        let pts: Vec<DVec2> = if at_end { a.iter().rev().copied().collect() } else { a.clone() };
        let mut run = 0.0;
        let mut q = pts[pts.len() - 1];
        for w in pts.windows(2) {
            run += (w[1] - w[0]).length();
            if run >= over {
                q = w[1];
                break;
            }
        }
        let d = (q - pts[0]).normalize_or_zero();
        if at_end { -d } else { d }
    }

    /// The point `back` metres along a street's axis from one of its ends.
    pub fn point_from(&self, si: usize, at_end: bool, back: f64) -> DVec2 {
        let a = &self.streets[si].axis;
        let pts: Vec<DVec2> = if at_end { a.iter().rev().copied().collect() } else { a.clone() };
        let mut run = 0.0;
        for w in pts.windows(2) {
            let l = (w[1] - w[0]).length();
            if run + l >= back {
                return w[0] + (w[1] - w[0]) * ((back - run) / l.max(1e-9));
            }
            run += l;
        }
        pts[pts.len() - 1]
    }

    /// The merges and diverges of the network: nodes where exactly three one-way streets
    /// end, two running in and one out (a merge) or one in and two out (a diverge), the main
    /// road going on nearly straight (within 30°) and the other coming in or leaving at a
    /// shallow angle (within 50°). The main road is the one that is no slip road, else the
    /// wider, else the straighter.
    pub fn find_merges(&self) -> Vec<Merge> {
        let mut at: Vec<Vec<(usize, bool)>> = vec![Vec::new(); self.nodes.len()];
        for (i, s) in self.streets.iter().enumerate() {
            if s.ends[0] == s.ends[1] {
                continue;
            }
            at[s.ends[0]].push((i, false));
            at[s.ends[1]].push((i, true));
        }
        let mut out = Vec::new();
        for (node, e) in at.iter().enumerate() {
            if e.len() != 3 || e.iter().any(|(i, _)| !self.streets[*i].oneway) {
                continue;
            }
            // (a street's end at the node: traffic runs into the node; its start: out of it)
            let ins: Vec<usize> = e.iter().filter(|x| x.1).map(|x| x.0).collect();
            let outs: Vec<usize> = e.iter().filter(|x| !x.1).map(|x| x.0).collect();
            let diverge = match (ins.len(), outs.len()) {
                (2, 1) => false,
                (1, 2) => true,
                _ => continue,
            };
            let (one, two) = if diverge { (ins[0], &outs) } else { (outs[0], &ins) };
            let t = self.way_at(one, diverge, 15.0);
            let along = |si: usize| self.way_at(si, !diverge, 15.0).dot(t);
            let rank = |si: usize| {
                let s = &self.streets[si];
                (!s.link as u8, (s.width * 10.0) as i64, (along(si) * 1000.0) as i64)
            };
            let (main, ramp) = if rank(two[0]) >= rank(two[1]) { (two[0], two[1]) } else { (two[1], two[0]) };
            if along(main) < 30f64.to_radians().cos() || along(ramp) < 50f64.to_radians().cos() {
                continue;
            }
            // (the side: where the slip road is 20 m off the node)
            let p = self.point_from(ramp, !diverge, 20.0) - self.nodes[node];
            let right = p.dot(DVec2::new(t.y, -t.x)) > 0.0;
            let (main_in, main_out) = if diverge { (one, main) } else { (main, one) };
            out.push(Merge { node, main_in, main_out, ramp, diverge, right });
        }
        out
    }
}

fn polyline_dist(p: DVec2, line: &[DVec2]) -> (f64, usize, f64) {
    // (distance, segment, signed side: + right of the line's direction)
    let mut best = (f64::MAX, 0, 0.0);
    for (i, w) in line.windows(2).enumerate() {
        let d = dist_to_seg(p, w[0], w[1]);
        if d < best.0 {
            let dir = (w[1] - w[0]).normalize_or_zero();
            let side = (p - w[0]).perp_dot(dir);
            best = (d, i, side);
        }
    }
    best
}

fn resample(line: &[DVec2], step: f64) -> Vec<DVec2> {
    let mut out = vec![line[0]];
    let mut carry = 0.0;
    for w in line.windows(2) {
        let l = (w[1] - w[0]).length();
        let mut s = step - carry;
        while s <= l {
            out.push(w[0] + (w[1] - w[0]) * (s / l));
            s += step;
        }
        carry = l - (s - step);
    }
    if (*out.last().unwrap() - *line.last().unwrap()).length() > 0.1 {
        out.push(*line.last().unwrap());
    }
    out
}

/// Layers A and B: the ways read into a network of streets.
pub fn network(ways: &[RawWay]) -> Network {
    let mut net = Network::default();
    // nodes merged within 1.5 m
    let mut node_of: HashMap<i64, usize> = HashMap::new();
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut add = |id: i64, p: DVec2, net: &mut Network| -> usize {
        if let Some(n) = node_of.get(&id) {
            return *n;
        }
        let c = ((p.x / 3.0).floor() as i64, (p.y / 3.0).floor() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &n in grid.get(&(c.0 + dx, c.1 + dy)).into_iter().flatten() {
                    if (net.nodes[n] - p).length() < 1.5 {
                        node_of.insert(id, n);
                        return n;
                    }
                }
            }
        }
        net.nodes.push(p);
        let n = net.nodes.len() - 1;
        grid.entry(c).or_default().push(n);
        node_of.insert(id, n);
        n
    };
    // carriageway ways split into edges at nodes used by another carriageway way (or their ends)
    let carr: Vec<&RawWay> = ways.iter().filter(|w| matches!(role(w), Some(Role::Carriageway(_)))).collect();
    let links: HashSet<i64> = carr.iter().filter(|w| w.tag("highway").ends_with("_link")).map(|w| w.id).collect();
    let mut uses: HashMap<i64, usize> = HashMap::new();
    for w in &carr {
        for n in &w.nodes {
            *uses.entry(*n).or_default() += 1;
        }
        *uses.entry(w.nodes[0]).or_default() += 1;
        *uses.entry(*w.nodes.last().unwrap()).or_default() += 1;
    }
    struct Edge {
        way: i64,
        class: u8,
        oneway: bool,
        name: String,
        nodes: [usize; 2],
        pts: Vec<DVec2>,
    }
    impl Edge {
        fn clone_edge(&self) -> Edge {
            Edge { way: self.way, class: self.class, oneway: self.oneway, name: self.name.clone(), nodes: self.nodes, pts: self.pts.clone() }
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    for w in &carr {
        let Some(Role::Carriageway(class)) = role(w) else { continue };
        let mut start = 0;
        for i in 1..w.nodes.len() {
            if i == w.nodes.len() - 1 || uses[&w.nodes[i]] > 1 {
                let a = add(w.nodes[start], w.pts[start], &mut net);
                let b = add(w.nodes[i], w.pts[i], &mut net);
                let mut pts: Vec<DVec2> = w.pts[start..=i].to_vec();
                pts[0] = net.nodes[a];
                *pts.last_mut().unwrap() = net.nodes[b];
                if a != b {
                    edges.push(Edge { way: w.id, class, oneway: is_oneway(w), name: w.tag("name").to_string(), nodes: [a, b], pts });
                }
                start = i;
            }
        }
    }
    // crossings without a node (layer A: planarised): two carriageways of one level crossing,
    // or one crossing itself (a drive's loop drawn as a figure of eight), get a node there and
    // are cut at it. Bridges, tunnels and other layers pass over and under.
    let levels: HashMap<i64, String> = ways
        .iter()
        .map(|w| {
            let l = if w.tag("bridge") != "" && w.tag("bridge") != "no" || w.tag("tunnel") != "" && w.tag("tunnel") != "no" {
                format!("x{}", w.id)
            } else {
                w.tag("layer").trim_start_matches('+').replace("-0", "0")
            };
            (w.id, if l == "0" { String::new() } else { l })
        })
        .collect();
    let level = |way: i64| -> &str { levels.get(&way).map(String::as_str).unwrap_or("") };
    let cross = |a: DVec2, b: DVec2, c: DVec2, d: DVec2| -> Option<(f64, f64)> {
        let (r, q) = (b - a, d - c);
        let den = r.perp_dot(q);
        if den.abs() < 1e-9 {
            return None;
        }
        let t = (c - a).perp_dot(q) / den;
        let u = (c - a).perp_dot(r) / den;
        (t > 1e-6 && t < 1.0 - 1e-6 && u > 1e-6 && u < 1.0 - 1e-6).then_some((t, u))
    };
    // (all of them found at once, the segments put in a grid of 40 m cells, then every edge cut
    // at each of its crossings in order along it: as many as a whole city has)
    {
        let mut grid: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            for a in 0..e.pts.len() - 1 {
                let (lo, hi) = (e.pts[a].min(e.pts[a + 1]), e.pts[a].max(e.pts[a + 1]));
                for x in (lo.x / GRID).floor() as i64..=(hi.x / GRID).floor() as i64 {
                    for y in (lo.y / GRID).floor() as i64..=(hi.y / GRID).floor() as i64 {
                        grid.entry((x, y)).or_default().push((i, a));
                    }
                }
            }
        }
        let mut cells: Vec<&(i64, i64)> = grid.keys().collect();
        cells.sort();
        // (where each edge is cut: segment, how far along it, the node)
        let mut cuts: HashMap<usize, Vec<(usize, f64, usize)>> = HashMap::new();
        let mut seen: HashSet<(usize, usize, usize, usize)> = HashSet::new();
        let mut made: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for c in cells {
            let list = &grid[c];
            for x in 0..list.len() {
                for y in x + 1..list.len() {
                    let ((i, a), (j, b)) = if list[x] <= list[y] { (list[x], list[y]) } else { (list[y], list[x]) };
                    if i == j && b <= a + 1 {
                        continue;
                    }
                    if level(edges[i].way) != level(edges[j].way) || !seen.insert((i, a, j, b)) {
                        continue;
                    }
                    let (pi, pj) = (&edges[i].pts, &edges[j].pts);
                    let Some((t, u)) = cross(pi[a], pi[a + 1], pj[b], pj[b + 1]) else { continue };
                    let x = pi[a] + (pi[a + 1] - pi[a]) * t;
                    // (not at a node they already share)
                    if edges[i].nodes.iter().chain(edges[j].nodes.iter()).any(|n| (net.nodes[*n] - x).length() < 1.5) {
                        continue;
                    }
                    // (three roads crossing at one place: one node)
                    let k = ((x.x / 3.0).floor() as i64, (x.y / 3.0).floor() as i64);
                    let near = (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (k.0 + dx, k.1 + dy))).flat_map(|q| made.get(&q).cloned().unwrap_or_default()).find(|m| (net.nodes[*m] - x).length() < 1.5);
                    let m = match near {
                        Some(m) => m,
                        None => {
                            net.nodes.push(x);
                            let m = net.nodes.len() - 1;
                            made.entry(k).or_default().push(m);
                            m
                        }
                    };
                    cuts.entry(i).or_default().push((a, t, m));
                    cuts.entry(j).or_default().push((b, u, m));
                }
            }
        }
        let mut out: Vec<Edge> = Vec::with_capacity(edges.len() + cuts.len() * 2);
        for (i, e) in edges.into_iter().enumerate() {
            let Some(mut c) = cuts.remove(&i) else {
                out.push(e);
                continue;
            };
            c.sort_by(|p, q| (p.0, p.1).partial_cmp(&(q.0, q.1)).unwrap_or(std::cmp::Ordering::Equal));
            c.dedup_by(|p, q| p.2 == q.2 && p.0 == q.0);
            let piece = |nodes: [usize; 2], pts: Vec<DVec2>| Edge { way: e.way, class: e.class, oneway: e.oneway, name: e.name.clone(), nodes, pts };
            let (mut from, mut pts) = (e.nodes[0], vec![e.pts[0]]);
            let mut k = 0;
            for a in 0..e.pts.len() - 1 {
                while k < c.len() && c[k].0 == a {
                    let x = net.nodes[c[k].2];
                    pts.push(x);
                    out.push(piece([from, c[k].2], std::mem::replace(&mut pts, vec![x])));
                    from = c[k].2;
                    k += 1;
                }
                pts.push(e.pts[a + 1]);
            }
            out.push(piece([from, e.nodes[1]], pts));
        }
        edges = out;
    }
    // loose ends: an edge ending (degree 1) just short of, or just past, another carriageway it
    // points at is joined to it there as a T (OSM often leaves a drive unconnected to its street);
    // the other edge is cut at the joint
    // (the edges near each place found in a grid, the nodes' degrees kept as they change; the
    // search goes on from the edge last joined)
    let cell_of = |p: DVec2| ((p.x / GRID).floor() as i64, (p.y / GRID).floor() as i64);
    let put = |grid: &mut HashMap<(i64, i64), Vec<usize>>, i: usize, e: &Edge| {
        for w in e.pts.windows(2) {
            let (lo, hi) = (cell_of(w[0].min(w[1])), cell_of(w[0].max(w[1])));
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    let v = grid.entry((x, y)).or_default();
                    if v.last() != Some(&i) {
                        v.push(i);
                    }
                }
            }
        }
    };
    let mut near_grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        put(&mut near_grid, i, e);
    }
    let mut deg_of: HashMap<usize, usize> = HashMap::new();
    for e in &edges {
        for n in e.nodes {
            *deg_of.entry(n).or_default() += 1;
        }
    }
    let mut resume = 0;
    loop {
        let mut done = None;
        'find: for i in resume..edges.len() {
            for end in 0..2 {
                let n = edges[i].nodes[end];
                if deg_of.get(&n).copied().unwrap_or(0) != 1 {
                    continue;
                }
                let pts = &edges[i].pts;
                let (p, q) = if end == 0 { (pts[0], pts[1]) } else { (pts[pts.len() - 1], pts[pts.len() - 2]) };
                let dir = (p - q).normalize_or_zero();
                let my_half = default_width(edges[i].class, edges[i].oneway) * 0.5;
                let c = cell_of(p);
                let mut cand: Vec<usize> = (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (c.0 + dx, c.1 + dy))).filter_map(|k| near_grid.get(&k)).flatten().copied().collect();
                cand.sort_unstable();
                cand.dedup();
                for j in cand {
                    // (and only to one of its own level: never up onto a bridge over it)
                    if j == i || edges[j].nodes.contains(&n) || level(edges[j].way) != level(edges[i].way) {
                        continue;
                    }
                    let (d, k, _) = polyline_dist(p, &edges[j].pts);
                    let reach = default_width(edges[j].class, edges[j].oneway) * 0.5 + my_half + 4.0;
                    if d < 0.5 || d > reach {
                        continue;
                    }
                    let (a, b) = (edges[j].pts[k], edges[j].pts[k + 1]);
                    let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                    let foot = a + (b - a) * t;
                    // (it points at the other one, not along it)
                    if (foot - p).normalize_or_zero().dot(dir) < 0.5 && d > 1.5 {
                        continue;
                    }
                    // (near an end of the other one: joined to that node instead)
                    let (ja, jb) = (edges[j].nodes[0], edges[j].nodes[1]);
                    let at_node = [ja, jb].into_iter().find(|m| (net.nodes[*m] - foot).length() < 6.0);
                    done = Some((i, end, j, k, foot, at_node));
                    break 'find;
                }
            }
        }
        let Some((i, end, j, k, foot, at_node)) = done else { break };
        resume = i;
        let n_old = edges[i].nodes[end];
        *deg_of.entry(n_old).or_default() -= 1;
        let m = match at_node {
            Some(m) => m,
            None => {
                net.nodes.push(foot);
                let m = net.nodes.len() - 1;
                // the other edge cut at the joint
                let e = &edges[j];
                // (no point twice: a joint on a vertex is that vertex)
                let mut first: Vec<DVec2> = e.pts[..=k].to_vec();
                if (*first.last().unwrap() - foot).length() < 0.1 {
                    first.pop();
                }
                first.push(foot);
                let mut second = vec![foot];
                second.extend(e.pts[k + 1..].iter().copied().filter(|q| (*q - foot).length() >= 0.1));
                if first.len() < 2 || second.len() < 2 {
                    net.nodes.pop();
                    // (the joint falls on the other one's end: that node)
                    let m = if first.len() < 2 { e.nodes[0] } else { e.nodes[1] };
                    let to = net.nodes[m];
                    let ei = &mut edges[i];
                    if end == 0 {
                        ei.pts.insert(0, to);
                        ei.nodes[0] = m;
                    } else {
                        ei.pts.push(to);
                        ei.nodes[1] = m;
                    }
                    *deg_of.entry(m).or_default() += 1;
                    put(&mut near_grid, i, &edges[i]);
                    continue;
                }
                let (way, class, oneway, name, nodes) = (e.way, e.class, e.oneway, e.name.clone(), e.nodes);
                edges[j] = Edge { way, class, oneway, name: name.clone(), nodes: [nodes[0], m], pts: first };
                edges.push(Edge { way, class, oneway, name, nodes: [m, nodes[1]], pts: second });
                *deg_of.entry(m).or_default() += 2;
                let last = edges.len() - 1;
                put(&mut near_grid, last, &edges[last]);
                m
            }
        };
        *deg_of.entry(m).or_default() += 1;
        // this edge carried on to the joint
        let to = net.nodes[m];
        let e = &mut edges[i];
        if end == 0 {
            e.pts.insert(0, to);
            e.nodes[0] = m;
        } else {
            e.pts.push(to);
            e.nodes[1] = m;
        }
        put(&mut near_grid, i, &edges[i]);
    }
    // two roads leaving one node at a sharp angle (under 30°) run as one until they are their
    // widths apart, and fork there: a short common trunk, then a Y (else their axes cross just
    // past the node and no plate can be made between them)
    // (which edges end at each node, found again after every change; the search goes on from
    // the node last changed, a node that cannot be done is left as it is)
    let incidence = |edges: &Vec<Edge>, n: usize| -> Vec<Vec<(usize, usize)>> {
        let mut v: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
        for (i, e) in edges.iter().enumerate() {
            for (k, x) in e.nodes.iter().enumerate() {
                v[*x].push((i, k));
            }
        }
        v
    };
    let mut resume = 0;
    for _ in 0..100_000 {
        let inc = incidence(&edges, net.nodes.len());
        let mut job = None;
        'fork: for n in resume..net.nodes.len() {
            let at: Vec<(usize, usize)> = inc[n].clone();
            if at.len() < 2 {
                continue;
            }
            let outward = |i: usize, k: usize| -> Vec<DVec2> { let mut p = resample(&edges[i].pts, 0.5); if k == 1 { p.reverse(); } p };
            for x in 0..at.len() {
                for y in x + 1..at.len() {
                    let ((i, ki), (j, kj)) = (at[x], at[y]);
                    // (one-way ones are the two halves of a loop or a divided road: theirs is a
                    // different shape, see the loops and the divided streets)
                    if i == j || edges[i].oneway || edges[j].oneway {
                        continue;
                    }
                    let (pi, pj) = (outward(i, ki), outward(j, kj));
                    if pi.len() < 6 || pj.len() < 6 {
                        continue;
                    }
                    let (di, dj) = ((pi[4] - pi[0]).normalize_or_zero(), (pj[4] - pj[0]).normalize_or_zero());
                    if di.dot(dj) < 30f64.to_radians().cos() {
                        continue;
                    }
                    let need = (default_width(edges[i].class, edges[i].oneway) + default_width(edges[j].class, edges[j].oneway)) * 0.5 + 1.0;
                    // (the first place along the first where the second is `need` away)
                    let Some(ti) = (1..pi.len()).find(|&t| polyline_dist(pi[t], &pj).0 >= need) else { continue };
                    if ti as f64 * 0.5 > 60.0 || ti + 2 >= pi.len() {
                        continue;
                    }
                    let (_, tj, _) = polyline_dist(pi[ti], &pj);
                    let tj = (tj + 1).min(pj.len() - 2);
                    job = Some((n, i, ki, j, kj, pi, pj, ti, tj));
                    break 'fork;
                }
            }
        }
        let Some((n, i, ki, j, kj, pi, pj, ti, tj)) = job else { break };
        resume = n;
        let m_pt = (pi[ti] + pj[tj]) * 0.5;
        net.nodes.push(m_pt);
        let m = net.nodes.len() - 1;
        let trunk_pts: Vec<DVec2> = vec![net.nodes[n], (pi[ti / 2] + pj[tj / 2]) * 0.5, m_pt];
        let (class, oneway, name, way) = (edges[i].class.min(edges[j].class), false, edges[i].name.clone(), edges[i].way);
        for (e, k, pts, t) in [(i, ki, &pi, ti), (j, kj, &pj, tj)] {
            let mut rest: Vec<DVec2> = vec![m_pt];
            rest.extend(pts[t..].iter().copied());
            let far = edges[e].nodes[1 - k];
            if k == 0 {
                edges[e].pts = rest;
                edges[e].nodes = [m, far];
            } else {
                rest.reverse();
                edges[e].pts = rest;
                edges[e].nodes = [far, m];
            }
        }
        edges.push(Edge { way, class, oneway, name, nodes: [n, m], pts: trunk_pts });
    }
    // a side road leaving a through street at a sharp angle (under 45°) is brought square to it:
    // its axis left where it is 8 m clear of the street's carriageway, a new joint on the street
    // across from there (as a road designer lays a side road in; a plate cannot be made between
    // arms a few degrees apart)
    let mut resume = 0;
    let mut left_as_is: HashSet<usize> = HashSet::new();
    for _ in 0..100_000 {
        let inc = incidence(&edges, net.nodes.len());
        let mut job = None;
        'node: for n in resume..net.nodes.len() {
            let at: Vec<(usize, usize)> = inc[n].clone();
            if at.len() != 3 || left_as_is.contains(&n) {
                continue;
            }
            let out_dir = |i: usize, k: usize| -> DVec2 {
                let p = &edges[i].pts;
                let (a, b) = if k == 0 { (p[0], p[(p.len() - 1).min(4)]) } else { (p[p.len() - 1], p[p.len().saturating_sub(5)]) };
                (b - a).normalize_or_zero()
            };
            for side in 0..3 {
                let (si, sk) = at[side];
                let others: Vec<(usize, usize)> = at.iter().enumerate().filter(|(j, _)| *j != side).map(|(_, x)| *x).collect();
                let (m1, m2) = (others[0], others[1]);
                if edges[si].class < edges[m1.0].class.max(edges[m2.0].class) {
                    continue;
                }
                // (a slip road, or one-way roads meeting one-way roads, is a merge or a diverge:
                // laid in alongside the road, never squared to it, see `Merge`)
                if links.contains(&edges[si].way) || (edges[si].oneway && edges[m1.0].oneway && edges[m2.0].oneway) {
                    continue;
                }
                let (d1, d2, ds) = (out_dir(m1.0, m1.1), out_dir(m2.0, m2.1), out_dir(si, sk));
                if d1.dot(d2) > -0.85 {
                    continue;
                }
                let sharp = ds.dot(d1).max(ds.dot(d2));
                if sharp < 45f64.to_radians().cos() {
                    continue;
                }
                let main: Vec<DVec2> = {
                    let mut a = edges[m1.0].pts.clone();
                    if m1.1 == 0 {
                        a.reverse();
                    }
                    let mut b = edges[m2.0].pts.clone();
                    if m2.1 == 1 {
                        b.reverse();
                    }
                    a.extend(b.into_iter().skip(1));
                    a
                };
                let clear = default_width(edges[m1.0].class, edges[m1.0].oneway).max(default_width(edges[m2.0].class, edges[m2.0].oneway)) * 0.5 + 8.0;
                let mut sp = edges[si].pts.clone();
                if sk == 1 {
                    sp.reverse();
                }
                let Some(pi) = (1..sp.len()).find(|&i| polyline_dist(sp[i], &main).0 >= clear) else { continue };
                if poly_len(&sp[pi..]) < 5.0 {
                    continue;
                }
                job = Some((n, side, si, sk, m1, m2, sp, pi));
                break 'node;
            }
        }
        let Some((n, _side, si, sk, m1, m2, sp, pi)) = job else { break };
        resume = n;
        let p = sp[pi];
        // the foot on the street: on whichever of its two edges is nearer
        let (e, k, foot) = [m1, m2].iter().map(|(i, _)| {
            let (d, k, _) = polyline_dist(p, &edges[*i].pts);
            let (a, b) = (edges[*i].pts[k], edges[*i].pts[k + 1]);
            let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
            (d, *i, k, a + (b - a) * t)
        }).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| (x.1, x.2, x.3)).unwrap();
        // (too near the old node: the joint stays there)
        if (foot - net.nodes[n]).length() < 3.0 {
            left_as_is.insert(n);
            continue;
        }
        net.nodes.push(foot);
        let m = net.nodes.len() - 1;
        let old = edges[e].clone_edge();
        let mut first: Vec<DVec2> = old.pts[..=k].to_vec();
        if (*first.last().unwrap() - foot).length() < 0.1 {
            first.pop();
        }
        first.push(foot);
        let mut second = vec![foot];
        second.extend(old.pts[k + 1..].iter().copied().filter(|q| (*q - foot).length() >= 0.1));
        if first.len() < 2 || second.len() < 2 {
            net.nodes.pop();
            left_as_is.insert(n);
            continue;
        }
        edges[e] = Edge { way: old.way, class: old.class, oneway: old.oneway, name: old.name.clone(), nodes: [old.nodes[0], m], pts: first };
        edges.push(Edge { way: old.way, class: old.class, oneway: old.oneway, name: old.name, nodes: [m, old.nodes[1]], pts: second });
        // the side road from the new joint, square off the street, on from where it is clear
        let mut np = vec![foot];
        np.extend_from_slice(&sp[pi..]);
        let far = edges[si].nodes[1 - sk];
        edges[si].pts = if sk == 0 { np } else { np.into_iter().rev().collect() };
        if sk == 0 {
            edges[si].nodes = [m, far];
        } else {
            edges[si].nodes = [far, m];
        }
    }
    // streets: edges chained through degree-2 nodes of the same class (and name); a one-way
    // street only on along its own way (two one-way roads running into each other are two)
    struct Chain {
        edges: Vec<usize>,
        pts: Vec<DVec2>,
        ends: [usize; 2],
        class: u8,
        oneway: bool,
        link: bool,
        name: String,
    }
    let mut used = vec![false; edges.len()];
    let inc = incidence(&edges, net.nodes.len());
    let deg = |n: usize, _: &Vec<Edge>| inc[n].len();
    let mut chains: Vec<Chain> = Vec::new();
    for i in 0..edges.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let e = &edges[i];
        let mut chain: Vec<usize> = vec![i];
        let mut pts = e.pts.clone();
        let mut ends = e.nodes;
        // grow forward and backward through degree-2 nodes
        for forward in [true, false] {
            loop {
                let n = if forward { ends[1] } else { ends[0] };
                if deg(n, &edges) != 2 {
                    break;
                }
                let along = |k: usize| !e.oneway || (edges[k].nodes[0] == n) == forward;
                let Some(k) = inc[n].iter().map(|x| x.0).find(|&k| !used[k] && edges[k].class == e.class && edges[k].oneway == e.oneway && along(k)) else { break };
                used[k] = true;
                chain.push(k);
                let mut q = edges[k].pts.clone();
                if (edges[k].nodes[0] == n) != forward {
                    q.reverse();
                }
                let other = if edges[k].nodes[0] == n { edges[k].nodes[1] } else { edges[k].nodes[0] };
                if forward {
                    pts.extend_from_slice(&q[1..]);
                    ends[1] = other;
                } else {
                    let mut q = q;
                    q.pop();
                    q.extend_from_slice(&pts);
                    pts = q;
                    ends[0] = other;
                }
                if other == ends[if forward { 0 } else { 1 }] {
                    break;
                }
            }
        }
        let link = chain.iter().all(|k| links.contains(&edges[*k].way));
        chains.push(Chain { edges: chain, pts, ends, class: e.class, oneway: e.oneway, link, name: e.name.clone() });
    }
    // divided streets: two one-way streets of one class (not slip roads) running opposite at
    // 6–30 m from each other, and ending across from each other (each one's start by the
    // other's end). One that ends where only it meets another road (a slip road joining one
    // carriageway, a side road coming in from one side) is no half of a divided street there:
    // both stay one-way roads, and every road meeting them has its end at a real node.
    let mut partner: HashMap<usize, usize> = HashMap::new();
    let mut root: Vec<usize> = (0..net.nodes.len()).collect();
    fn find(r: &mut Vec<usize>, x: usize) -> usize {
        let mut x = x;
        while r[x] != x {
            r[x] = r[r[x]];
            x = r[x];
        }
        x
    }
    // (nor is a piece of carriageway between slip roads, in an interchange: where a slip road
    // meets a carriageway is a merge or a diverge of that carriageway alone; and a short piece
    // is no street of its own)
    let link_at: HashSet<usize> = edges.iter().filter(|e| links.contains(&e.way)).flat_map(|e| e.nodes).collect();
    let pairable = |c: &Chain| c.oneway && !c.link && c.class <= 1 && c.ends[0] != c.ends[1] && !c.ends.iter().any(|n| link_at.contains(n)) && poly_len(&c.pts) >= 40.0;
    // (a chain's partner is looked for among those passing near it: a grid of where they run)
    let mut chain_grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (j, c) in chains.iter().enumerate() {
        if !pairable(c) {
            continue;
        }
        for p in resample(&c.pts, 10.0) {
            let v = chain_grid.entry(cell_of(p)).or_default();
            if v.last() != Some(&j) {
                v.push(j);
            }
        }
    }
    for i in 0..chains.len() {
        if partner.contains_key(&i) || !pairable(&chains[i]) {
            continue;
        }
        let a = &chains[i];
        let si = resample(&a.pts, 5.0);
        let da = (a.pts[a.pts.len() - 1] - a.pts[0]).normalize_or_zero();
        let mut best: Option<(usize, f64)> = None;
        let mut cand: Vec<usize> = si.iter().map(|p| cell_of(*p)).flat_map(|c| (-1..=1).flat_map(move |dx| (-1..=1).map(move |dy| (c.0 + dx, c.1 + dy)))).filter_map(|k| chain_grid.get(&k)).flatten().copied().collect();
        cand.sort_unstable();
        cand.dedup();
        for j in cand {
            let b = &chains[j];
            if i == j || partner.contains_key(&j) || !pairable(b) || b.class != a.class {
                continue;
            }
            let db = (b.pts[b.pts.len() - 1] - b.pts[0]).normalize_or_zero();
            if da.dot(db) >= -0.7 {
                continue;
            }
            let near: Vec<f64> = si.iter().map(|p| polyline_dist(*p, &b.pts).0).collect();
            let ok = near.iter().filter(|d| (6.0..30.0).contains(*d)).count();
            if ok * 10 < si.len() * 7 {
                continue;
            }
            let mean = near.iter().sum::<f64>() / near.len() as f64;
            let reach = (mean * 1.5).max(20.0);
            let (m0, m1) = ((net.nodes[a.ends[0]] - net.nodes[b.ends[1]]).length(), (net.nodes[a.ends[1]] - net.nodes[b.ends[0]]).length());
            if m0 > reach || m1 > reach {
                continue;
            }
            if best.map_or(true, |b| mean < b.1) {
                best = Some((j, mean));
            }
        }
        if let Some((j, _)) = best {
            partner.insert(i, j);
            partner.insert(j, i);
            // (the two ends across from each other are one node of the model: where the
            // street's middle line ends)
            for (x, y) in [(chains[i].ends[0], chains[j].ends[1]), (chains[i].ends[1], chains[j].ends[0])] {
                let (rx, ry) = (find(&mut root, x), find(&mut root, y));
                if rx != ry {
                    root[rx] = ry;
                }
            }
        }
    }
    // the nodes made one: at the middle of those they were made of
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for n in 0..net.nodes.len() {
        let r = find(&mut root, n);
        members.entry(r).or_default().push(n);
    }
    for (r, m) in &members {
        if m.len() > 1 {
            net.nodes[*r] = m.iter().map(|n| net.nodes[*n]).sum::<DVec2>() / m.len() as f64;
        }
    }
    let mut done = vec![false; chains.len()];
    for i in 0..chains.len() {
        if done[i] {
            continue;
        }
        done[i] = true;
        let c = &chains[i];
        let ends = [find(&mut root, c.ends[0]), find(&mut root, c.ends[1])];
        // (a piece between two carriageways made one node lies inside the junction there:
        // the crossing street's stretch over the median)
        if ends[0] == ends[1] && c.ends[0] != c.ends[1] && poly_len(&c.pts) < 60.0 {
            continue;
        }
        let (mut oneway, mut median, mut width, mut pts) = (c.oneway, 0.0, default_width(c.class, c.oneway), c.pts.clone());
        let mut ways: Vec<usize> = c.edges.clone();
        if let Some(&j) = partner.get(&i) {
            done[j] = true;
            // middle line: each point of this one paired with the nearest of the other
            let other = &chains[j].pts;
            let along = resample(&pts, 5.0);
            let mut mid: Vec<DVec2> = along.iter().map(|p| {
                let (_, k, _) = polyline_dist(*p, other);
                let (a, b) = (other[k], other[k + 1]);
                let t = ((*p - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                (*p + a + (b - a) * t) * 0.5
            }).collect();
            median = mid.iter().zip(&along).map(|(m, p)| (*m - *p).length() * 2.0).sum::<f64>() / mid.len().max(1) as f64;
            let n = mid.len();
            mid[0] = net.nodes[ends[0]];
            mid[n - 1] = net.nodes[ends[1]];
            pts = mid;
            oneway = false;
            width = default_width(c.class, false);
            ways.extend(chains[j].edges.iter().copied());
        }
        net.streets.push(Street {
            ways: ways.iter().map(|k| edges[*k].way).collect::<HashSet<_>>().into_iter().collect(),
            name: c.name.clone(),
            class: c.class,
            oneway,
            median,
            width,
            sidewalk: [None, None],
            axis: pts,
            ends,
            link: c.link,
        });
    }
    net.merges = net.find_merges();
    net.paths_from(ways);
    net
}

impl Network {
    /// Half the width a street takes up from its axis, sidewalks included (its corridor).
    pub fn corridor(&self, si: usize) -> f64 {
        let s = &self.streets[si];
        let walk = s.sidewalk[0].unwrap_or(0.0).max(s.sidewalk[1].unwrap_or(0.0)).max(match s.class { 2 => 0.0, _ => 2.25 });
        s.width * 0.5 + s.median * 0.5 + walk
    }

    /// The footways read into the model: stretches running beside a street become that street's
    /// sidewalk; what is left is cut where it enters a street's corridor (ending at its edge, a
    /// crossing noted where a path goes right across a street); paths lying on a longer one are
    /// dropped; path ends near another path are joined to it.
    fn paths_from(&mut self, ways: &[RawWay]) {
        let mut pieces: Vec<Vec<DVec2>> = Vec::new();
        let mut walks: Vec<(i64, usize, usize, f64)> = Vec::new();
        let mut new_cross: Vec<(usize, DVec2)> = Vec::new();
        let mut passes: Vec<(bool, Vec<DVec2>, Vec<bool>)> = Vec::new();
        for w in ways.iter().filter(|w| role(w) == Some(Role::Foot)) {
            if w.pts.len() < 2 {
                continue;
            }
            let samples = resample(&w.pts, 1.0);
            // each sample: beside which street (and side, offset), if any
            let tag: Vec<Option<(usize, usize, f64)>> = samples.iter().map(|p| {
                let mut best: Option<(usize, usize, f64, f64)> = None;
                for (si, s) in self.streets.iter().enumerate() {
                    if s.class > 1 {
                        continue;
                    }
                    let half = s.width * 0.5 + s.median * 0.5;
                    let (d, k, side) = polyline_dist(*p, &s.axis);
                    // (beside it: off the carriageway, within 12 m, and running along it)
                    if d > half + 0.5 && d < half + 12.0 && k + 1 < s.axis.len() {
                        if best.map_or(true, |b| d < b.3) {
                            best = Some((si, (side > 0.0) as usize, d, d));
                        }
                    }
                }
                best.map(|b| (b.0, b.1, b.2))
            }).collect();
            // runs of samples beside one street, along it (the path's direction within 25° of the
            // street's), at least 15 m long: sidewalk
            let along = |i: usize, si: usize| -> bool {
                let a = samples[i.saturating_sub(2)];
                let b = samples[(i + 2).min(samples.len() - 1)];
                let (_, k, _) = polyline_dist(samples[i], &self.streets[si].axis);
                let ax = &self.streets[si].axis;
                let sd = (ax[k + 1] - ax[k]).normalize_or_zero();
                ((b - a).normalize_or_zero().dot(sd)).abs() > 0.9
            };
            let mut keep = vec![true; samples.len()];
            let mut i = 0;
            while i < samples.len() {
                if let Some((si, side, _)) = tag[i].filter(|t| along(i, t.0)) {
                    let mut j = i;
                    while j + 1 < samples.len() && tag[j + 1].map(|t| t.0 == si && t.1 == side).unwrap_or(false) && along(j + 1, si) {
                        j += 1;
                    }
                    if j - i >= 15 {
                        let off = (i..=j).map(|k| tag[k].unwrap().2).sum::<f64>() / (j - i + 1) as f64;
                        walks.push((w.id, si, side, off));
                        for k in i..=j {
                            keep[k] = false;
                        }
                    }
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            passes.push((w.tag("footway") == "crossing", samples, keep));
        }
        for (wid, si, side, off) in walks {
            self.sidewalks.push((wid, si, side, off));
            let s = &mut self.streets[si];
            // (the line runs along the sidewalk's middle; the lawn strip between it and the kerb is
            // not sidewalk: a third of the distance beyond the carriageway, 1.5–4 m)
            let wdt = ((off - s.width * 0.5 - s.median * 0.5) * 0.66).clamp(1.5, 4.0);
            s.sidewalk[side] = Some(s.sidewalk[side].map_or(wdt, |x: f64| x.max(wdt)));
        }
        for (is_crossing, samples, keep) in passes {
            if is_crossing {
                continue;
            }
            // the rest, cut where it is inside a street's corridor
            let inside = |p: DVec2| -> Option<usize> {
                (0..self.streets.len()).find(|&si| polyline_dist(p, &self.streets[si].axis).0 < self.corridor(si))
            };
            let mut cur: Vec<DVec2> = Vec::new();
            let mut last_in: Option<usize> = None;
            for (k, p) in samples.iter().enumerate() {
                let ins = inside(*p);
                if keep[k] && ins.is_none() {
                    if cur.is_empty() {
                        if let Some(si) = last_in {
                            let _ = si;
                        }
                    }
                    cur.push(*p);
                } else {
                    if cur.len() >= 4 {
                        pieces.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
                if let Some(si) = ins {
                    // a path going right across a street (both sides outside): a crossing
                    if last_in != Some(si) {
                        let back = samples[..k].iter().rev().take(40).any(|q| inside(*q).is_none());
                        let fwd = samples[k..].iter().take(60).skip_while(|q| inside(**q) == Some(si)).next().map(|q| inside(*q).is_none()).unwrap_or(false);
                        if back && fwd {
                            let (_, kk, _) = polyline_dist(*p, &self.streets[si].axis);
                            let ax = &self.streets[si].axis;
                            let (a, b) = (ax[kk], ax[kk + 1]);
                            let t = ((*p - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                            new_cross.push((si, a + (b - a) * t));
                        }
                    }
                }
                last_in = ins;
            }
            if cur.len() >= 4 {
                pieces.push(cur);
            }
        }
        self.crossings.extend(new_cross);
        // a piece lying along a longer one (within 1.5 m for most of it) is the same path drawn twice
        pieces.sort_by(|a, b| poly_len(b).total_cmp(&poly_len(a)));
        let mut kept: Vec<Vec<DVec2>> = Vec::new();
        for p in pieces {
            let dup = kept.iter().any(|q| p.iter().filter(|x| polyline_dist(**x, q).0 < 1.5).count() * 10 >= p.len() * 8);
            if !dup {
                kept.push(p);
            }
        }
        // ends within 3 m of another path joined onto it
        for i in 0..kept.len() {
            for end in [0usize, 1] {
                let e = if end == 0 { kept[i][0] } else { *kept[i].last().unwrap() };
                let mut best: Option<(f64, DVec2)> = None;
                for (j, q) in kept.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let (d, k, _) = polyline_dist(e, q);
                    if d < 3.0 && d > 0.05 && best.map_or(true, |b| d < b.0) {
                        let (a, b) = (q[k], q[k + 1]);
                        let t = ((e - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                        best = Some((d, a + (b - a) * t));
                    }
                }
                if let Some((_, at)) = best {
                    if end == 0 { kept[i].insert(0, at) } else { kept[i].push(at) }
                }
            }
        }
        self.paths = kept.into_iter().map(|p| simplify(&p, 0.3)).collect();
        self.path_graph();
        // crossings closer than 6 m on one street are one
        let mut cs: Vec<(usize, DVec2)> = Vec::new();
        for c in self.crossings.drain(..) {
            if !cs.iter().any(|d| d.0 == c.0 && (d.1 - c.1).length() < 6.0) {
                cs.push(c);
            }
        }
        self.crossings = cs;
    }
}

impl Network {
    /// The paths cut into a network: ends within 1.5 m are one node; a path whose end lies on
    /// another's middle cuts that one there (a T).
    fn path_graph(&mut self) {
        let mut nodes: Vec<DVec2> = Vec::new();
        let node = |p: DVec2, nodes: &mut Vec<DVec2>| -> usize {
            if let Some(i) = nodes.iter().position(|q| (*q - p).length() < 1.5) {
                return i;
            }
            nodes.push(p);
            nodes.len() - 1
        };
        // the points every path is cut at: its own ends and other paths' ends lying on it
        let ends: Vec<DVec2> = self.paths.iter().flat_map(|p| [p[0], *p.last().unwrap()]).collect();
        let mut edges = Vec::new();
        for p in &self.paths {
            // positions along p (segment index, point) where another path ends on it
            let mut cuts: Vec<(usize, f64, DVec2)> = Vec::new();
            for e in &ends {
                if (*e - p[0]).length() < 1.5 || (*e - *p.last().unwrap()).length() < 1.5 {
                    continue;
                }
                let (d, k, _) = polyline_dist(*e, p);
                if d < 0.6 {
                    let (a, b) = (p[k], p[k + 1]);
                    let t = ((*e - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                    cuts.push((k, t, a + (b - a) * t));
                }
            }
            cuts.sort_by(|x, y| (x.0, x.1).partial_cmp(&(y.0, y.1)).unwrap());
            let mut cur = vec![p[0]];
            let mut c = 0;
            for k in 0..p.len() - 1 {
                while c < cuts.len() && cuts[c].0 == k {
                    let q = cuts[c].2;
                    if (q - *cur.last().unwrap()).length() > 0.3 {
                        cur.push(q);
                    }
                    if cur.len() >= 2 && poly_len(&cur) > 1.0 {
                        let (a, b) = (node(cur[0], &mut nodes), node(q, &mut nodes));
                        edges.push((std::mem::take(&mut cur), [a, b]));
                    }
                    cur = vec![q];
                    c += 1;
                }
                if (p[k + 1] - *cur.last().unwrap()).length() > 0.3 {
                    cur.push(p[k + 1]);
                }
            }
            if cur.len() >= 2 && poly_len(&cur) > 1.0 {
                let (a, b) = (node(cur[0], &mut nodes), node(*cur.last().unwrap(), &mut nodes));
                edges.push((cur, [a, b]));
            }
        }
        // (each piece starts and ends exactly at its nodes)
        for (pts, ab) in edges.iter_mut() {
            pts[0] = nodes[ab[0]];
            *pts.last_mut().unwrap() = nodes[ab[1]];
        }
        self.path_nodes = nodes;
        self.path_edges = edges;
    }
}

impl Network {
    /// The paths made to fit the streets as finally laid (after the imagery moved and widened
    /// them) and to run smoothly: an end inside a street's corridor is cut back to its edge, an
    /// end stopping short of it (within 2.5 m) is carried on to it, so every path meets the
    /// sidewalk at its outer edge and never the carriageway; then pieces meeting two at a node
    /// are made one, so a path bends through such a point instead of being joined there at
    /// an angle.
    pub fn settle_paths(&mut self) {
        let corr: Vec<f64> = (0..self.streets.len()).map(|si| self.corridor(si)).collect();
        // (the axes as they will be laid: the fit rounds corners off the OSM line)
        let laid: Vec<Vec<DVec2>> = self
            .streets
            .iter()
            .map(|st| {
                let segs = fit_g1(&st.axis, st.class);
                let mut v: Vec<DVec2> = segs.iter().flat_map(|g| { let n = (g.length.ceil() as usize).max(1); (0..n).map(move |i| g.point(g.length * i as f64 / n as f64)) }).collect();
                if let Some(g) = segs.last() {
                    v.push(g.end());
                }
                if v.len() < 2 { st.axis.clone() } else { v }
            })
            .collect();
        let street_at = |p: DVec2| -> Option<(f64, DVec2, f64)> {
            // (the nearest corridor edge: how far inside (+) or outside (-), the axis point)
            let mut best: Option<(f64, DVec2, f64)> = None;
            for (si, ax) in laid.iter().enumerate() {
                let (d, k, _) = polyline_dist(p, ax);
                let (a, b) = (ax[k], ax[(k + 1).min(ax.len() - 1)]);
                let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                let q = a + (b - a) * t;
                let inside = corr[si] - d;
                if best.map_or(true, |x| inside > x.0) {
                    best = Some((inside, q, d));
                }
            }
            best
        };
        let mut out: Vec<Vec<DVec2>> = Vec::new();
        for (pts, _) in &self.path_edges {
            let mut r = resample(pts, 0.5);
            // cut back what lies inside a corridor, from either end
            for _ in 0..2 {
                while r.len() >= 2 && street_at(r[r.len() - 1]).map(|x| x.0 > -0.05).unwrap_or(false) {
                    r.pop();
                }
                r.reverse();
            }
            if r.len() < 2 || poly_len(&r) < 2.0 {
                continue;
            }
            // carried on to a corridor's edge when it stops just short of it
            for _ in 0..2 {
                let e = r[r.len() - 1];
                if let Some((inside, q, d)) = street_at(e) {
                    if inside < -0.05 && inside > -2.5 && d > 1e-6 {
                        let to = e + (q - e) / d * (-inside + 0.05);
                        r.push(to);
                    }
                }
                r.reverse();
            }
            out.push(simplify(&r, 0.2));
        }
        // the nodes again, from the ends as they are now
        let mut nodes: Vec<DVec2> = Vec::new();
        let mut node = |p: DVec2| -> usize {
            if let Some(i) = nodes.iter().position(|q| (*q - p).length() < 1.5) {
                return i;
            }
            nodes.push(p);
            nodes.len() - 1
        };
        let mut edges: Vec<(Vec<DVec2>, [usize; 2])> = out.into_iter().map(|p| { let ab = [node(p[0]), node(*p.last().unwrap())]; (p, ab) }).collect();
        // (a piece coming back to where it began, short: what is left of a path cut at a street
        // on both sides, a spike, not a ring)
        edges.retain(|(p, ab)| ab[0] != ab[1] || poly_len(p) >= 15.0);
        // pieces meeting two at a node: one path through it
        loop {
            let mut deg = vec![0usize; nodes.len()];
            for (_, ab) in &edges {
                deg[ab[0]] += 1;
                deg[ab[1]] += 1;
            }
            let Some(n) = (0..nodes.len()).find(|&n| deg[n] == 2 && edges.iter().filter(|(_, ab)| ab[0] == n || ab[1] == n).count() == 2) else { break };
            let ix: Vec<usize> = (0..edges.len()).filter(|&i| edges[i].1[0] == n || edges[i].1[1] == n).collect();
            let (i, j) = (ix[0], ix[1]);
            let (mut a, aab) = edges[i].clone();
            let (mut b, bab) = edges[j].clone();
            // (a runs into n, b runs out of it)
            let a0 = if aab[1] == n { aab[0] } else { a.reverse(); aab[1] };
            let b1 = if bab[0] == n { bab[1] } else { b.reverse(); bab[0] };
            a.extend(b.into_iter().skip(1));
            let (hi, lo) = (i.max(j), i.min(j));
            edges.remove(hi);
            edges.remove(lo);
            edges.push((a, [a0, b1]));
        }
        for (pts, ab) in edges.iter_mut() {
            pts[0] = nodes[ab[0]];
            *pts.last_mut().unwrap() = nodes[ab[1]];
        }
        self.path_nodes = nodes;
        self.path_edges = edges;
    }
}

fn poly_len(p: &[DVec2]) -> f64 {
    p.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn way(id: i64, tags: &[(&str, &str)], pts: &[(f64, f64)], nodes: &[i64]) -> RawWay {
        RawWay { id, tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), nodes: nodes.to_vec(), pts: pts.iter().map(|p| DVec2::new(p.0, p.1)).collect() }
    }

    #[test]
    fn a_noisy_straight_street_is_one_smooth_line() {
        // OSM nodes zig-zagging ±0.8 m around a straight line every 7 m
        let pts: Vec<DVec2> = (0..30).map(|i| DVec2::new(i as f64 * 7.0, if i % 2 == 0 { 0.8 } else { -0.8 })).collect();
        let s = fit(&pts, 1);
        assert!(max_break(&s) < 0.01 && max_gap(&s) < 0.01, "{s:?}");
        // no wobble: the curvature changes sign at most a couple of times
        let signs: Vec<f64> = s.iter().filter(|x| x.radius != 0.0).map(|x| x.radius.signum()).collect();
        let flips = signs.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(flips <= 2, "{flips} flips: {s:?}");
    }

    #[test]
    fn a_corner_gets_at_least_the_class_radius_when_its_legs_allow() {
        let pts = [DVec2::new(0.0, 0.0), DVec2::new(0.0, 80.0), DVec2::new(80.0, 80.0)];
        let s = fit(&pts, 1);
        assert!(max_break(&s) < 0.01);
        let arc = s.iter().find(|x| x.radius != 0.0).expect("an arc");
        assert!(arc.radius.abs() >= 15.0 - 1e-6, "{arc:?}");
        // a short-legged yard corner is still rounded, never left sharp
        let pts = [DVec2::new(0.0, 0.0), DVec2::new(0.0, 7.0), DVec2::new(9.0, 7.0), DVec2::new(9.0, 30.0)];
        let s = fit(&pts, 2);
        assert!(max_break(&s) < 0.01, "{s:?}");
    }

    #[test]
    fn two_one_way_ways_make_one_divided_street_and_sidewalks_are_absorbed() {
        let ways = vec![
            way(1, &[("highway", "secondary"), ("oneway", "yes"), ("name", "A")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "secondary"), ("oneway", "yes"), ("name", "A")], &[(200.0, 12.0), (0.0, 12.0)], &[3, 4]),
            way(3, &[("highway", "footway")], &[(0.0, 22.0), (200.0, 22.0)], &[5, 6]),
            way(4, &[("highway", "footway")], &[(100.0, -40.0), (100.0, 60.0)], &[7, 8]),
        ];
        let net = network(&ways);
        assert_eq!(net.streets.len(), 1, "{:?}", net.streets);
        let s = &net.streets[0];
        assert!(!s.oneway && (s.median - 12.0).abs() < 0.5, "{s:?}");
        assert!(s.axis.iter().all(|p| (p.y - 6.0).abs() < 0.5));
        assert_eq!(net.sidewalks.len(), 1);
        // the footway across the street: two paths ending at the corridor's edge, and a crossing
        assert_eq!(net.paths.len(), 2, "{:?}", net.paths);
        let c = net.corridor(0);
        assert!(net.paths.iter().flatten().all(|p| (p.y - 6.0).abs() >= c - 1.01), "{c} {:?}", net.paths);
        assert_eq!(net.crossings.len(), 1);
    }

    /// A divided main road, its eastbound carriageway joined by a slip road from the south at
    /// x = 200 (and its westbound one not).
    fn merge_ways() -> Vec<RawWay> {
        vec![
            way(1, &[("highway", "primary"), ("oneway", "yes")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "primary"), ("oneway", "yes")], &[(200.0, 0.0), (400.0, 0.0)], &[2, 3]),
            way(3, &[("highway", "primary"), ("oneway", "yes")], &[(400.0, 15.0), (0.0, 15.0)], &[4, 5]),
            way(4, &[("highway", "primary_link"), ("oneway", "yes")], &[(20.0, -90.0), (90.0, -50.0), (150.0, -15.0), (200.0, 0.0)], &[6, 7, 8, 2]),
        ]
    }

    #[test]
    fn a_slip_road_joining_one_carriageway_makes_a_merge_and_no_divided_street() {
        let net = network(&merge_ways());
        // (the carriageway it joins is cut there, the other is not: no halves of one street)
        assert!(net.streets.iter().all(|s| s.median == 0.0 && s.oneway), "{:?}", net.streets.iter().map(|s| (s.median, s.oneway, s.ends)).collect::<Vec<_>>());
        let n = net.nodes.iter().position(|p| (*p - DVec2::new(200.0, 0.0)).length() < 0.1).unwrap();
        assert_eq!(net.degree(n), 3);
        assert_eq!(net.merges.len(), 1, "{:?}", net.merges);
        let m = net.merges[0];
        assert!(m.node == n && !m.diverge && m.right, "{m:?}");
        assert!(net.streets[m.ramp].link && !net.streets[m.main_in].link && !net.streets[m.main_out].link);
        assert_eq!(net.streets[m.main_in].ends[1], n);
        assert_eq!(net.streets[m.main_out].ends[0], n);
        // (every free end is a far end of the area, none inside another road)
        for (k, p) in net.nodes.iter().enumerate() {
            if net.degree(k) == 1 {
                assert!(p.x < 1.0 || p.x > 399.0 || p.y < -80.0, "free end at {p:?}");
            }
        }
    }

    #[test]
    fn a_slip_road_is_not_squared_to_the_road_it_joins() {
        let net = network(&merge_ways());
        let m = net.merges[0];
        // (it still comes in at its own shallow angle)
        let d = net.way_at(m.ramp, true, 15.0);
        assert!(d.dot(DVec2::X) > 45f64.to_radians().cos(), "{d:?}");
    }

    #[test]
    fn a_divided_street_with_a_side_road_on_one_side_only_is_two_one_way_roads() {
        let mut ways = merge_ways();
        ways[3] = way(4, &[("highway", "residential")], &[(200.0, -60.0), (200.0, 0.0)], &[6, 2]);
        let net = network(&ways);
        let n = net.nodes.iter().position(|p| (*p - DVec2::new(200.0, 0.0)).length() < 0.1).unwrap();
        assert_eq!(net.degree(n), 3);
        assert!(net.merges.is_empty());
        // but one meeting both carriageways at their ends is one divided street
        let ways = vec![
            way(1, &[("highway", "primary"), ("oneway", "yes")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(3, &[("highway", "primary"), ("oneway", "yes")], &[(200.0, 15.0), (0.0, 15.0)], &[4, 5]),
            way(5, &[("highway", "residential")], &[(200.0, -60.0), (200.0, 0.0), (200.0, 15.0), (200.0, 80.0)], &[6, 2, 4, 7]),
        ];
        let net = network(&ways);
        let div: Vec<&Street> = net.streets.iter().filter(|s| s.median > 0.0).collect();
        assert_eq!(div.len(), 1, "{:?}", net.streets.iter().map(|s| (s.median, s.ends)).collect::<Vec<_>>());
        // (its end is the crossing street's node, and the street's stretch over the median is gone)
        let e = div[0].ends.iter().map(|e| net.nodes[*e]).find(|p| p.x > 100.0).unwrap();
        assert!((e - DVec2::new(200.0, 7.5)).length() < 0.5, "{e:?}");
        let x = net.nodes.iter().position(|p| (*p - e).length() < 0.01).unwrap();
        assert_eq!(net.degree(x), 3, "{:?}", net.streets.iter().map(|s| (s.class, s.ends)).collect::<Vec<_>>());
    }

    #[test]
    fn a_street_split_into_ways_is_one_street_and_a_t_junction_has_degree_three() {
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)], &[1, 2, 3]),
            way(2, &[("highway", "residential")], &[(100.0, 0.0), (150.0, 0.0)], &[3, 4]),
            way(3, &[("highway", "service")], &[(50.0, 0.0), (50.0, 40.0)], &[2, 5]),
        ];
        let net = network(&ways);
        assert_eq!(net.streets.iter().filter(|s| s.class == 1).count(), 2, "split at the T node only");
        let t = net.nodes.iter().position(|p| (*p - DVec2::new(50.0, 0.0)).length() < 0.1).unwrap();
        assert_eq!(net.degree(t), 3);
    }

    #[test]
    fn a_footbridge_and_its_stairs_are_not_laid_on_the_ground() {
        let ways = vec![
            way(1, &[("highway", "footway"), ("bridge", "yes"), ("layer", "1")], &[(0.0, 0.0), (120.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "steps"), ("layer", "1")], &[(0.0, 20.0), (0.0, 0.0)], &[3, 1]),
            way(3, &[("highway", "footway")], &[(0.0, 60.0), (0.0, 20.0)], &[4, 3]),
        ];
        let net = network(&ways);
        assert!(net.paths.iter().all(|p| p.iter().all(|q| q.y > 10.0)), "{:?}", net.paths);
    }

    #[test]
    fn carriageways_crossing_without_a_node_get_one_and_bridges_do_not() {
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "service")], &[(100.0, -60.0), (100.0, 60.0)], &[3, 4]),
        ];
        let net = network(&ways);
        let x = net.nodes.iter().position(|p| (*p - DVec2::new(100.0, 0.0)).length() < 0.5).expect("a node at the crossing");
        assert_eq!(net.degree(x), 4);
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "service"), ("bridge", "yes"), ("layer", "1")], &[(100.0, -60.0), (100.0, 60.0)], &[3, 4]),
        ];
        let net = network(&ways);
        assert!(net.nodes.iter().all(|p| (*p - DVec2::new(100.0, 0.0)).length() > 0.5), "a bridge passes over");
        // (and is a road all the same, laid on its own level)
        assert!(net.streets.iter().any(|s| s.class == 2 && s.axis.iter().all(|q| (q.x - 100.0).abs() < 0.5)), "{:?}", net.streets);
        // a drive in a tunnel under a building is no road of the map
        let ways = vec![way(2, &[("highway", "service"), ("tunnel", "building_passage")], &[(100.0, -60.0), (100.0, 60.0)], &[3, 4])];
        assert!(network(&ways).streets.is_empty());
        // a drive drawn as a figure of eight: one node where it crosses itself
        let ways = vec![way(1, &[("highway", "service")], &[(0.0, 0.0), (40.0, 40.0), (40.0, 0.0), (0.0, 40.0), (0.0, 80.0)], &[1, 2, 3, 4, 5])];
        let net = network(&ways);
        let x = net.nodes.iter().position(|p| (*p - DVec2::new(20.0, 20.0)).length() < 0.5).expect("a node where it crosses itself");
        assert_eq!(net.degree(x), 4, "{:?}", net.streets.iter().map(|s| s.ends).collect::<Vec<_>>());
    }

    #[test]
    fn two_roads_leaving_a_node_at_a_sharp_angle_fork_after_a_common_trunk() {
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "service")], &[(100.0, 0.0), (95.0, 60.0)], &[3, 4]),
            way(3, &[("highway", "service")], &[(100.0, 0.0), (110.0, 60.0)], &[3, 5]),
        ];
        let ways = { let mut w = ways; w[1].nodes[0] = 6; w[2].nodes[0] = 6; w[0].nodes = vec![1, 6, 2]; w[0].pts.insert(1, DVec2::new(100.0, 0.0)); w };
        let net = network(&ways);
        // the two service roads no longer start at the street's node: a trunk takes them there
        let n0 = net.nodes.iter().position(|p| (*p - DVec2::new(100.0, 0.0)).length() < 0.5).unwrap();
        assert_eq!(net.degree(n0), 3, "{:?}", net.streets.iter().map(|s| (s.class, s.ends)).collect::<Vec<_>>());
        let fork = net.nodes.iter().enumerate().find(|(k, p)| *k != n0 && p.y > 5.0 && p.y < 50.0 && net.degree(*k) == 3);
        assert!(fork.is_some(), "a Y further up");
    }

    #[test]
    fn a_side_road_leaving_at_a_sharp_angle_is_brought_square() {
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (100.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "residential")], &[(100.0, 0.0), (200.0, 0.0)], &[2, 3]),
            way(3, &[("highway", "service")], &[(100.0, 0.0), (140.0, 12.0), (180.0, 40.0)], &[2, 4, 5]),
        ];
        let net = network(&ways);
        let side = net.streets.iter().find(|s| s.class == 2).unwrap();
        let (a, b) = (side.axis[0], side.axis[1]);
        let ang = (b - a).normalize().dot(DVec2::X).abs();
        assert!(ang < 0.5, "meets the street at {:.0} deg: {:?}", ang.acos().to_degrees(), side.axis);
        assert!((a - DVec2::new(100.0, 0.0)).length() > 3.0);
    }

    #[test]
    fn a_drive_ending_short_of_a_street_is_joined_to_it_as_a_t() {
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "service")], &[(100.0, 60.0), (100.0, 5.0)], &[3, 4]),
        ];
        let net = network(&ways);
        let t = net.nodes.iter().position(|p| (*p - DVec2::new(100.0, 0.0)).length() < 0.5).expect("a node on the street under the drive");
        assert_eq!(net.degree(t), 3, "{:?}", net.streets.iter().map(|s| s.ends).collect::<Vec<_>>());
        // a parallel drive beside it is left alone
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "service")], &[(20.0, 8.0), (120.0, 8.0)], &[3, 4]),
        ];
        let net = network(&ways);
        assert!(net.nodes.iter().all(|n| net.degree(net.nodes.iter().position(|q| q == n).unwrap()) <= 2));
    }

    #[test]
    fn settled_paths_bend_through_two_way_nodes_and_stop_at_the_corridor() {
        // a yard path drawn as two ways meeting at a corner, its far end running into a street
        let ways = vec![
            way(1, &[("highway", "residential")], &[(0.0, 0.0), (200.0, 0.0)], &[1, 2]),
            way(2, &[("highway", "footway")], &[(60.0, 80.0), (60.0, 40.0)], &[3, 4]),
            way(3, &[("highway", "footway")], &[(60.0, 40.0), (100.0, 40.0), (100.0, 3.0)], &[4, 5, 6]),
        ];
        let mut net = network(&ways);
        net.settle_paths();
        assert_eq!(net.path_edges.len(), 1, "one path through the two-way node: {:?}", net.path_edges);
        let c = net.corridor(0);
        let (pts, _) = &net.path_edges[0];
        for p in pts {
            assert!(p.y.abs() >= c - 0.1, "inside the corridor ({c}): {p:?}");
        }
        // it reaches the corridor's edge (the sidewalk), not stopping short
        let lowest = pts.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        assert!(lowest - c < 0.2, "{lowest} vs {c}");
        // fitted, it runs smoothly through the corner
        let segs = fit_g1(pts, 3);
        assert!(max_break(&segs) < 0.02 && max_gap(&segs) < 0.01, "{segs:?}");
    }
}

#[cfg(test)]
/// The model of a list of ways in world metres (JSON: [{id, tags, nodes, pts}]) as JSON, for
/// looking at it (`OPENMAPPER_MODEL_IN` / `OPENMAPPER_MODEL_OUT`, see the ignored test).
pub fn model_json(input: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(input).map_err(|e| e.to_string())?;
    let ways: Vec<RawWay> = v.as_array().ok_or("a list")?.iter().filter_map(|w| {
        let tags = w.get("tags")?.as_object()?.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
        let nodes = w.get("nodes")?.as_array()?.iter().filter_map(|n| n.as_i64()).collect::<Vec<_>>();
        let pts = w.get("pts")?.as_array()?.iter().filter_map(|p| Some(DVec2::new(p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect::<Vec<_>>();
        (nodes.len() == pts.len() && pts.len() >= 2).then(|| RawWay { id: w.get("id").and_then(|i| i.as_i64()).unwrap_or(0), tags, nodes, pts })
    }).collect();
    let net = network(&ways);
    Ok(net_json(&net).to_string())
}

/// The model as JSON, for looking at it: nodes with their degree, streets (fitted axis
/// sampled every 2 m), paths, sidewalks, crossings.
pub fn net_json(net: &Network) -> serde_json::Value {
    let streets: Vec<serde_json::Value> = net.streets.iter().map(|s| {
        let segs = fit(&s.axis, s.class);
        let pts: Vec<[f64; 2]> = segs.iter().flat_map(|g| {
            let n = (g.length / 2.0).ceil().max(1.0) as usize;
            (0..n).map(move |i| { let q = g.point(g.length * i as f64 / n as f64); [q.x, q.y] })
        }).chain(segs.last().map(|g| { let q = g.end(); [q.x, q.y] })).collect();
        serde_json::json!({ "class": s.class, "oneway": s.oneway, "link": s.link, "median": s.median, "width": s.width, "sidewalk": s.sidewalk, "name": s.name,
            "ends": s.ends, "osm": s.axis.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(), "fit": pts, "max_break": max_break(&segs),
            "min_radius": segs.iter().filter(|g| g.radius != 0.0).map(|g| g.radius.abs()).fold(f64::MAX, f64::min) })
    }).collect();
    let degree: Vec<usize> = (0..net.nodes.len()).map(|n| net.degree(n)).collect();
    serde_json::json!({ "nodes": net.nodes.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(), "degree": degree, "streets": streets,
        "paths": net.paths.iter().map(|p| p.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "sidewalks": net.sidewalks.iter().map(|s| serde_json::json!({"way": s.0, "street": s.1, "side": s.2, "offset": s.3})).collect::<Vec<_>>(),
        "crossings": net.crossings.iter().map(|c| serde_json::json!({"street": c.0, "at": [c.1.x, c.1.y]})).collect::<Vec<_>>(), "merges": net.merges })
}

#[cfg(test)]
mod real {
    #[test]
    #[ignore]
    fn model_of_a_real_area() {
        let (Ok(i), Ok(o)) = (std::env::var("OPENMAPPER_MODEL_IN"), std::env::var("OPENMAPPER_MODEL_OUT")) else { return };
        let out = super::model_json(&std::fs::read_to_string(i).unwrap()).unwrap();
        std::fs::write(o, out).unwrap();
    }
}

// ---------------------------------------------------------------- imagery: widths and offsets

/// What the imagery says about a street: carriageway width and how far the real carriageway's
/// middle lies to the right of the OSM axis (m), from how many cross-sections.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Measure {
    pub width: f64,
    pub offset: f64,
    pub samples: usize,
}

/// Whether a pixel looks like asphalt: grey (low saturation), neither dark shadow nor bright pavement.
pub fn asphalt(c: [u8; 3]) -> bool {
    let (r, g, b) = (c[0] as i32, c[1] as i32, c[2] as i32);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let mean = (r + g + b) / 3;
    mx - mn <= 22 && (55..=165).contains(&mean) && !(g > r + 6 && g > b + 6)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// The carriageway measured across the fitted axis every 6 m (20 m kept clear of the ends, where
/// junctions are): the run of asphalt around the axis (gaps up to 1.5 m bridged: parked cars,
/// markings), sections where it is narrower than 3 m or wider than 30 m (trees over it, a square)
/// left out. None with fewer than 4 sections.
pub fn measure(axis: &[DVec2], class: u8, colour: &dyn Fn(DVec2) -> Option<[u8; 3]>) -> Option<Measure> {
    let segs = fit(axis, class);
    let total: f64 = segs.iter().map(|s| s.length).sum();
    if total < 30.0 {
        return None;
    }
    let (mut widths, mut offs) = (Vec::new(), Vec::new());
    let mut s = 20.0;
    while s < total - 20.0 {
        // the point and heading at s
        let mut acc = 0.0;
        let mut at = None;
        for g in &segs {
            if s <= acc + g.length {
                at = Some((g.point(s - acc), g.heading_at(s - acc)));
                break;
            }
            acc += g.length;
        }
        s += 6.0;
        let Some((p, h)) = at else { continue };
        let rt = roads::right(h);
        let n = 80; // ±20 m every 0.5 m
        let flags: Vec<Option<bool>> = (-n..=n).map(|i| colour(p + rt * (i as f64 * 0.5)).map(asphalt)).collect();
        if flags.iter().any(|f| f.is_none()) {
            continue;
        }
        let f: Vec<bool> = flags.into_iter().map(|f| f.unwrap()).collect();
        // bridge gaps of up to 3 samples (1.5 m)
        let mut g = f.clone();
        let mut i = 0;
        while i < g.len() {
            if !g[i] {
                let j = (i..g.len()).find(|&k| g[k]).unwrap_or(g.len());
                if i > 0 && j < g.len() && j - i <= 3 {
                    for k in i..j {
                        g[k] = true;
                    }
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
        // the run nearest the axis (within 4 m of it)
        let mid = n as usize;
        let start = (0..=8).flat_map(|d| [mid.saturating_sub(d), (mid + d).min(g.len() - 1)]).find(|&k| g[k]);
        let Some(k) = start else { continue };
        let lo = (0..=k).rev().take_while(|&q| g[q]).last().unwrap_or(k);
        let hi = (k..g.len()).take_while(|&q| g[q]).last().unwrap_or(k);
        let (a, b) = ((lo as f64 - n as f64) * 0.5, (hi as f64 - n as f64) * 0.5);
        let w = b - a + 0.5;
        if (3.0..30.0).contains(&w) && lo > 0 && hi < g.len() - 1 {
            widths.push(w);
            offs.push((a + b) * 0.5);
        }
    }
    if widths.len() < 4 {
        return None;
    }
    let samples = widths.len();
    Some(Measure { width: median(&mut widths), offset: median(&mut offs), samples })
}

/// The network's streets measured on the imagery: each street's carriageway width taken from it
/// (rounded to 0.5 m), and its axis moved onto the real carriageway's middle where OSM lies more
/// than 1 m off it. Divided streets keep their pair's geometry. What was measured, by street.
pub fn apply_imagery(net: &mut Network, colour: &dyn Fn(DVec2) -> Option<[u8; 3]>) -> Vec<Option<Measure>> {
    let mut out = Vec::new();
    // each street's lateral shift onto its real carriageway (none where OSM is within 1 m)
    let mut shift: Vec<Option<f64>> = Vec::new();
    for s in net.streets.iter_mut() {
        if s.median > 0.0 {
            out.push(None);
            shift.push(None);
            continue;
        }
        let m = measure(&s.axis, s.class, colour);
        if let Some(m) = m {
            s.width = (m.width * 2.0).round() / 2.0;
        }
        shift.push(m.and_then(|m| (m.offset.abs() > 1.0 && m.offset.abs() < 8.0).then_some(m.offset)));
        out.push(m);
    }
    // the nodes move by the mean of their streets' shifts (as vectors), so streets meeting at a
    // node still meet; along each street the shift blends from its nodes' to its own over 20 m
    let n = net.nodes.len();
    let mut acc = vec![(DVec2::ZERO, 0usize); n];
    let vec_of = |s: &Street, sh: f64, at_end: bool| -> DVec2 {
        let a = &s.axis;
        let d = if at_end { a[a.len() - 1] - a[a.len() - 2] } else { a[1] - a[0] }.normalize_or_zero();
        DVec2::new(d.y, -d.x) * sh
    };
    for (s, sh) in net.streets.iter().zip(&shift) {
        for (k, at_end) in [(0usize, false), (1, true)] {
            let v = sh.map(|x| vec_of(s, x, at_end)).unwrap_or(DVec2::ZERO);
            acc[s.ends[k]].0 += v;
            acc[s.ends[k]].1 += 1;
        }
    }
    let node_shift: Vec<DVec2> = acc.iter().map(|(v, c)| if *c > 0 { *v / *c as f64 } else { DVec2::ZERO }).collect();
    for (s, sh) in net.streets.iter_mut().zip(&shift) {
        let a = s.axis.clone();
        let len: Vec<f64> = std::iter::once(0.0).chain(a.windows(2).scan(0.0, |t, w| { *t += (w[1] - w[0]).length(); Some(*t) })).collect();
        let total = *len.last().unwrap();
        let (n0, n1) = (node_shift[s.ends[0]], node_shift[s.ends[1]]);
        for i in 0..a.len() {
            let d = (a[(i + 1).min(a.len() - 1)] - a[i.saturating_sub(1)]).normalize_or_zero();
            let own = DVec2::new(d.y, -d.x) * sh.unwrap_or(0.0);
            let w0 = (1.0 - len[i] / 20.0).clamp(0.0, 1.0);
            let w1 = (1.0 - (total - len[i]) / 20.0).clamp(0.0, 1.0);
            let base = own * (1.0 - w0.max(w1));
            s.axis[i] = a[i] + base + n0 * w0 * (1.0 - w1) + n1 * w1 * (1.0 - w0) + (n0 + n1) * 0.5 * (w0 * w1);
        }
    }
    for (i, p) in net.nodes.iter_mut().enumerate() {
        *p += node_shift[i];
    }
    // one street, one width: the measured widths of the pieces sharing a name and class, their median
    let mut groups: HashMap<(String, u8, bool), Vec<f64>> = HashMap::new();
    for (s, m) in net.streets.iter().zip(&out) {
        if let (Some(_), false) = (m, s.name.is_empty()) {
            groups.entry((s.name.clone(), s.class, s.oneway)).or_default().push(s.width);
        }
    }
    for s in net.streets.iter_mut() {
        if let Some(v) = groups.get_mut(&(s.name.clone(), s.class, s.oneway)) {
            s.width = median(v);
        }
        // (a service drive measured wide has the car park beside it in its asphalt)
        let cap = match s.class { 0 => 24.0, 1 => 16.0, _ => 7.0 };
        s.width = s.width.min(cap);
    }
    out
}

/// Where two streets meet end to end at a node and go on into each other (within 35°), both
/// leave it along one shared tangent (the mean of their directions): a short straight lead put at
/// each street's end, so the fitted axes join without a kink.
pub fn smooth_joints(net: &mut Network) {
    let n = net.nodes.len();
    let mut ends: Vec<Vec<(usize, bool)>> = vec![Vec::new(); n];
    for (i, s) in net.streets.iter().enumerate() {
        ends[s.ends[0]].push((i, false));
        ends[s.ends[1]].push((i, true));
    }
    for (node, e) in ends.iter().enumerate() {
        if e.len() != 2 {
            continue;
        }
        let out = |(i, at_end): (usize, bool)| -> DVec2 {
            let a = &net.streets[i].axis;
            let (p, q) = if at_end { (a[a.len() - 1], a[a.len().saturating_sub(3).max(0)]) } else { (a[0], a[2.min(a.len() - 1)]) };
            (q - p).normalize_or_zero()
        };
        let (d1, d2) = (out(e[0]), out(e[1]));
        if (-d1).dot(d2) < 35f64.to_radians().cos() {
            continue;
        }
        let t = (d2 - d1).normalize_or_zero();
        let p = net.nodes[node];
        for (k, (i, at_end)) in e.iter().enumerate() {
            let dir = if k == 0 { -t } else { t };
            let a = &mut net.streets[*i].axis;
            let len: f64 = a.windows(2).map(|w| (w[1] - w[0]).length()).sum();
            let lead = p + dir * (len / 3.0).min(8.0);
            if *at_end {
                let m = a.len();
                a.insert(m - 1, lead);
            } else {
                a.insert(1, lead);
            }
        }
    }
}

/// The heights a road designer gave the roads where they leave the ground (bridges, ramps,
/// cuttings): each way of the input as a line of points with their height. A road laid along
/// one takes its height there; two roads crossing one over the other are told apart by the way
/// they run.
#[derive(Clone, Debug, Default)]
pub struct Heights {
    /// (point, height, way, place along the way's points)
    pts: Vec<(DVec2, f64, usize, usize)>,
    /// Each way's points, by index into `pts`.
    ways: Vec<Vec<usize>>,
    /// Each way's OpenStreetMap id (0: not known).
    osm: Vec<i64>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

impl Heights {
    /// From the ways' lines of [x, y, z] points, each with its OpenStreetMap id.
    pub fn of_ways(lines: &[(i64, Vec<[f64; 3]>)]) -> Heights {
        let mut h = Heights::default();
        for (id, line) in lines.iter().filter(|l| l.1.len() >= 2) {
            let w = h.ways.len();
            h.osm.push(*id);
            let mut idx = Vec::new();
            for (k, q) in line.iter().enumerate() {
                h.pts.push((DVec2::new(q[0], q[1]), q[2], w, k));
                idx.push(h.pts.len() - 1);
            }
            h.ways.push(idx);
        }
        for (i, (p, ..)) in h.pts.iter().enumerate() {
            h.grid.entry(((p.x / 16.0).floor() as i64, (p.y / 16.0).floor() as i64)).or_default().push(i);
        }
        h
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// The height at `p` of the way passing there running along `heading` (or against it),
    /// within 14 m: on the nearest of its segments that runs within 32° of that, interpolated;
    /// on the given ways only (their OpenStreetMap ids; none given: any way). A
    /// street takes the heights of its own ways, never those of a road beside it or over it.
    pub fn at_on(&self, p: DVec2, heading: f64, ways: &[i64]) -> Option<f64> {
        let dir = roads::dir(heading);
        let c = ((p.x / 16.0).floor() as i64, (p.y / 16.0).floor() as i64);
        let mut best: Option<(f64, f64)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in self.grid.get(&(c.0 + dx, c.1 + dy)).into_iter().flatten() {
                    let (a, za, w, k) = self.pts[i];
                    if !ways.is_empty() && !ways.contains(&self.osm[w]) {
                        continue;
                    }
                    let Some(&j) = self.ways[w].get(k + 1) else { continue };
                    let (b, zb, ..) = self.pts[j];
                    let seg = b - a;
                    let len = seg.length();
                    if len < 1e-6 || (seg / len).dot(dir).abs() < 0.85 {
                        continue;
                    }
                    let t = ((p - a).dot(seg) / (len * len)).clamp(0.0, 1.0);
                    let d = (a + seg * t - p).length();
                    if d <= 14.0 && best.map_or(true, |x| d < x.0) {
                        best = Some((d, za + (zb - za) * t));
                    }
                }
            }
        }
        best.map(|x| x.1)
    }
}

/// A spline type a street may be built with: its carriageway width, one way, sidewalks.
#[derive(Clone, Debug)]
pub struct TypeChoice {
    pub file: String,
    pub width: f64,
    pub oneway: bool,
    pub sidewalks: bool,
    pub classes: Vec<u8>,
}

/// The type for a street: of its class, one-way as it is, with sidewalks where a street has them
/// (main and street classes, or sidewalks found), the carriageway nearest its width.
pub fn choose<'a>(s: &Street, types: &'a [TypeChoice]) -> Option<&'a TypeChoice> {
    let walks = s.class < 2 || s.sidewalk.iter().any(|w| w.is_some());
    types
        .iter()
        .filter(|t| t.classes.contains(&s.class) && t.oneway == s.oneway)
        .min_by(|a, b| {
            let cost = |t: &TypeChoice| (t.width - s.width).abs() + if t.sidewalks != walks { 2.5 } else { 0.0 };
            cost(a).total_cmp(&cost(b))
        })
}

#[cfg(test)]
mod imagery_tests {
    use super::*;

    #[test]
    fn a_street_is_measured_on_a_picture_and_moved_onto_its_carriageway() {
        // a picture: grass, and asphalt 12 m wide whose middle is 3 m north of the OSM axis y = 0
        let colour = |p: DVec2| -> Option<[u8; 3]> { Some(if (p.y - 3.0).abs() <= 6.0 { [110, 110, 112] } else { [70, 120, 60] }) };
        let axis = vec![DVec2::new(0.0, 0.0), DVec2::new(200.0, 0.0)];
        let m = measure(&axis, 1, &colour).unwrap();
        assert!((m.width - 12.5).abs() < 1.0 && (m.offset + 3.0).abs() < 0.5, "{m:?}");
        let mut net = Network { nodes: vec![axis[0], axis[1]], streets: vec![Street { ways: vec![], name: String::new(), class: 1, oneway: false, median: 0.0, width: 7.0, sidewalk: [None, None], axis, ends: [0, 1], link: false }], ..Default::default() };
        apply_imagery(&mut net, &colour);
        assert!((net.streets[0].width - 12.5).abs() < 1.0);
        assert!(net.streets[0].axis.iter().all(|p| (p.y - 3.0).abs() < 0.5), "{:?}", net.streets[0].axis);
        let types = vec![
            TypeChoice { file: "8".into(), width: 8.0, oneway: false, sidewalks: true, classes: vec![1] },
            TypeChoice { file: "12".into(), width: 12.0, oneway: false, sidewalks: true, classes: vec![1] },
            TypeChoice { file: "14".into(), width: 14.0, oneway: false, sidewalks: true, classes: vec![0, 1] },
        ];
        assert_eq!(choose(&net.streets[0], &types).unwrap().file, "12");
    }
}

// ---------------------------------------------------------------- road types made for the streets

/// The road type a street's cross-section asks for, made with the road maker: lanes from its
/// width (about 3.4 m each), Russian markings (none in yards; a dashed middle line on two lanes;
/// a double solid middle and dashed lines between lanes of one way from four lanes), no edge
/// lines (kerbs), its sidewalks (a kerb strip only on a yard drive), a raised middle strip on a
/// divided street.
pub fn road_spec(s: &Street, asphalt: &std::path::Path, paving: &std::path::Path) -> crate::road_maker::RoadSpec {
    use crate::road_maker::{MarkStyle, Marking, RoadSpec, WHITE};
    let w = s.width.clamp(3.0, 24.0);
    // (two-way: as many lanes each way)
    let (fwd, back) = if s.oneway { (((w / 3.4).round() as u8).max(1), 0) } else { let k = ((w / 6.8).round() as u8).max(1); (k, k) };
    let lane = (w / (fwd + back) as f64).clamp(2.5, 3.9) as f32;
    let walk = |x: Option<f64>| -> f32 { if s.class < 2 { x.unwrap_or(3.0).clamp(1.5, 4.0) as f32 } else { x.map(|x| x.clamp(1.5, 3.0) as f32).unwrap_or(0.3) } };
    let (fl, fr) = (walk(s.sidewalk[0]), walk(s.sidewalk[1]));
    let mut spec = RoadSpec {
        name: String::new(),
        lanes_fwd: fwd,
        lanes_back: back,
        lane_width: lane,
        median: if s.median > 0.0 { (s.median - w * 0.5).clamp(0.0, 10.0) as f32 } else { 0.0 },
        median_raised: s.median > 0.0,
        footway: fl,
        footway_right: Some(fr),
        kerb: 0.15,
        roadway_tex: asphalt.to_path_buf(),
        footway_tex: paving.to_path_buf(),
        median_tex: paving.to_path_buf(),
        marked: true,
        ..Default::default()
    };
    let rw = spec.roadway_width();
    let offset = spec.centerline();
    let line = |x: f32, style: MarkStyle| Marking { x, style, color: WHITE, ..Default::default() };
    let mut m = Vec::new();
    if s.class < 2 {
        if fwd > 0 && back > 0 {
            let mid = spec.median_width();
            if mid > 0.0 {
                m.push(line(offset - mid / 2.0 - 0.2, MarkStyle::Solid));
                m.push(line(offset + mid / 2.0 + 0.2, MarkStyle::Solid));
            } else if fwd + back >= 4 {
                m.push(line(offset, MarkStyle::Double));
            } else {
                m.push(line(offset, MarkStyle::Dashed));
            }
            for i in 1..back {
                m.push(line(offset - mid / 2.0 - i as f32 * lane, MarkStyle::Dashed));
            }
            for i in 1..fwd {
                m.push(line(offset + mid / 2.0 + i as f32 * lane, MarkStyle::Dashed));
            }
        } else {
            for i in 1..(fwd + back) {
                m.push(line(-rw / 2.0 + i as f32 * lane, MarkStyle::Dashed));
            }
        }
    }
    spec.markings = m;
    spec.name = format!("MN2018_{}{}_{}x{:.2}_m{:.1}_f{:.1}_{:.1}", ["main", "street", "yard"][s.class.min(2) as usize], if s.oneway { "_ow" } else { "" }, fwd + back, lane, spec.median, fl, fr).replace('.', "p");
    spec
}
