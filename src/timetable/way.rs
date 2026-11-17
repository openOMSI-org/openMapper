//! A trip's way as a list of the map's paths, picked by hand as in OMSI 2's editor: the
//! paths a saved track names, where two of them do not join (a gap the game's bus loses its
//! way at), a path clicked put into the way (the paths to it filled in), and the stops found
//! along it.

use std::collections::{BinaryHeap, HashMap, HashSet};

use glam::DVec3;
use omsi_timetable::TrackEntry;

use super::network::{Network, PathKey};
use super::route::{Problem, Route, StopOn, Ways, LANE_CHANGE};
use crate::doc::TileKey;

/// How far from a stop a path of the way may run to serve it (m).
const REACH: f64 = 15.0;

/// How far a way is filled in from a path to one clicked (m).
const FILL: f64 = 3000.0;

/// How far lane `b` starts from where lane `a` ends, where it does not run on from it (m);
/// 0 where it does, or is the lane beside it (a lane change, as OMSI 2's station links make
/// them too). The network links lanes as the game joins paths (see `network`), so a path
/// that is not linked to the one before is where the game's bus loses its way.
pub fn gap(net: &Network, a: usize, b: usize) -> f64 {
    let (la, lb) = (&net.lanes[a], &net.lanes[b]);
    if la.next.contains(&b) || la.side.contains(&b) {
        return 0.0;
    }
    (lb.pts[0] - *la.pts.last().unwrap()).length().max(0.01)
}

/// The places along `lanes` where a path does not join the next: (index of the path before
/// it, metres between them).
pub fn gaps(net: &Network, lanes: &[usize]) -> Vec<(usize, f64)> {
    lanes.windows(2).enumerate().map(|(i, w)| (i, gap(net, w[0], w[1]))).filter(|(_, d)| *d > 0.0).collect()
}

/// The lanes a saved track names, as the network holds them (None: a path the network does
/// not have). Each path is taken the way that joins its neighbours best (a two-way path has
/// a lane each way).
pub fn lanes_of_track(net: &Network, entries: &[TrackEntry], order: &[TileKey]) -> Vec<Option<usize>> {
    let options: Vec<Vec<usize>> = entries
        .iter()
        .map(|e| match e.values[..] {
            [id, path, tile, ..] => order.get(tile as usize).and_then(|t| net.by_key.get(&PathKey { tile: *t, id: id as i64, path: path as usize })).cloned().unwrap_or_default(),
            _ => Vec::new(),
        })
        .collect();
    // (the cheapest choice through the whole list: the gaps, a lane against its path's
    // direction a little more)
    let own = |l: usize| if net.lanes[l].against { 1.0 } else { 0.0 };
    let mut best: Vec<Vec<(f64, usize)>> = Vec::with_capacity(options.len());
    for (i, o) in options.iter().enumerate() {
        let row: Vec<(f64, usize)> = o
            .iter()
            .map(|&l| {
                let prev = if i > 0 { options[i - 1].iter().enumerate().map(|(k, &p)| (best[i - 1][k].0 + gap(net, p, l).min(50.0), k)).min_by(|a, b| a.0.total_cmp(&b.0)) } else { None };
                let (c, k) = prev.unwrap_or((0.0, usize::MAX));
                (c + own(l), k)
            })
            .collect();
        best.push(row);
    }
    let mut out = vec![None; options.len()];
    let mut pick: Option<usize> = None;
    for i in (0..options.len()).rev() {
        let k = match pick {
            Some(k) if k != usize::MAX => k,
            _ => match (0..options[i].len()).min_by(|&a, &b| best[i][a].0.total_cmp(&best[i][b].0)) {
                Some(k) => k,
                None => {
                    pick = None;
                    continue;
                }
            },
        };
        out[i] = Some(options[i][k]);
        pick = Some(best[i][k].1);
    }
    out
}

/// The way along `lanes` through the stops at `positions`: each stop where the way passes
/// it, after the one before. The way may run on before the first stop and after the last.
pub fn along(net: &Network, lanes: &[usize], positions: &[Option<DVec3>], left_hand: bool) -> Route {
    let ways = lanes.first().map(|l| if net.lanes[*l].rail { Ways::Rail } else { Ways::Road });
    let mut r = Route { lanes: lanes.to_vec(), ways, by_hand: true, ..Default::default() };
    let mut from = (0usize, f64::MIN);
    let mut found: Vec<Option<(usize, StopOn)>> = Vec::new();
    for (i, p) in positions.iter().enumerate() {
        let Some(p) = p else {
            r.problems.push(Problem::NoObject(i));
            found.push(None);
            continue;
        };
        // the first lanes from where the last stop was that pass it, the nearest of them
        let mut pick: Option<(usize, StopOn, f64)> = None;
        for k in from.0..lanes.len() {
            let lane = &net.lanes[lanes[k]];
            let (a, off, right) = lane.project(p.truncate());
            let ok = off <= REACH && (k > from.0 || a >= from.1);
            if !ok {
                if pick.is_some() {
                    break;
                }
                continue;
            }
            let score = off + if right != left_hand { 0.0 } else { 25.0 };
            if pick.as_ref().map(|x| score < x.2).unwrap_or(true) {
                pick = Some((k, StopOn { lane: lanes[k], along: a, off }, score));
            }
        }
        match pick {
            Some((k, s, _)) => {
                from = (k, s.along);
                found.push(Some((k, s)));
            }
            None => {
                r.problems.push(Problem::OffWay(i));
                found.push(None);
            }
        }
    }
    if found.iter().any(|f| f.is_none()) {
        return r;
    }
    r.stops = found.into_iter().flatten().collect();
    for w in r.stops.windows(2) {
        let ((a, sa), (b, sb)) = (w[0], w[1]);
        let mut len = sb.along - sa.along;
        for &l in &lanes[a..b] {
            len += net.lanes[l].length();
        }
        r.legs.push(len.max(0.0));
    }
    for (k, w) in r.stops.windows(2).enumerate() {
        if lanes[w[0].0..=w[1].0].iter().any(|l| net.lanes[*l].against) {
            r.against.push(k);
        }
    }
    r
}

#[derive(PartialEq)]
struct Node(f64, usize);
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0)
    }
}

/// What driving a lane costs the filling in (m; against its path's direction far more).
fn cost(net: &Network, l: usize) -> f64 {
    let lane = &net.lanes[l];
    if lane.against {
        lane.length() * 4.0 + 150.0
    } else {
        lane.length()
    }
}

/// The lanes reached from the end of lane `a` within [`FILL`]: (cost to their start, the
/// lane before), on along the lanes or across to the lane beside.
fn search_from(net: &Network, a: usize) -> HashMap<usize, (f64, usize)> {
    let mut best: HashMap<usize, (f64, usize)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let mut done = HashSet::new();
    for &n in net.lanes[a].next.iter().chain(&net.lanes[a].side) {
        best.insert(n, (0.0, a));
        heap.push(Node(0.0, n));
    }
    while let Some(Node(c, l)) = heap.pop() {
        if !done.insert(l) || c > FILL {
            continue;
        }
        let lane = &net.lanes[l];
        let steps = lane.next.iter().map(|&n| (n, c + cost(net, l))).chain(lane.side.iter().map(|&n| (n, c + LANE_CHANGE)));
        for (n, cn) in steps {
            if best.get(&n).map(|x| cn < x.0).unwrap_or(true) {
                best.insert(n, (cn, l));
                heap.push(Node(cn, n));
            }
        }
    }
    best
}

/// The lanes from whose end lane `b` is reached within [`FILL`]: (cost from their end to
/// `b`'s start, the lane after).
fn search_to(net: &Network, b: usize) -> HashMap<usize, (f64, usize)> {
    let mut prev: HashMap<usize, Vec<(usize, bool)>> = HashMap::new();
    for (i, l) in net.lanes.iter().enumerate() {
        for &n in &l.next {
            prev.entry(n).or_default().push((i, false));
        }
        for &n in &l.side {
            prev.entry(n).or_default().push((i, true));
        }
    }
    let mut best: HashMap<usize, (f64, usize)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let mut done = HashSet::new();
    for &(p, _) in prev.get(&b).map(|v| v.as_slice()).unwrap_or(&[]) {
        best.insert(p, (0.0, b));
        heap.push(Node(0.0, p));
    }
    while let Some(Node(c, l)) = heap.pop() {
        if !done.insert(l) || c > FILL {
            continue;
        }
        for &(p, side) in prev.get(&l).map(|v| v.as_slice()).unwrap_or(&[]) {
            let cp = c + if side { LANE_CHANGE } else { cost(net, l) };
            if best.get(&p).map(|x| cp < x.0).unwrap_or(true) {
                best.insert(p, (cp, l));
                heap.push(Node(cp, p));
            }
        }
    }
    best
}

/// The lanes between `a` and `b` along a [`search_from`] of `a`.
fn path_from(found: &HashMap<usize, (f64, usize)>, a: usize, b: usize) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    let mut at = found.get(&b)?.1;
    while at != a {
        out.push(at);
        at = found.get(&at)?.1;
        if out.len() > 100_000 {
            return None;
        }
    }
    out.reverse();
    Some(out)
}

/// The lanes between `a` and `b` along a [`search_to`] of `b`.
fn path_to(found: &HashMap<usize, (f64, usize)>, a: usize, b: usize) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    let mut at = found.get(&a)?.1;
    while at != b {
        out.push(at);
        at = found.get(&at)?.1;
        if out.len() > 100_000 {
            return None;
        }
    }
    Some(out)
}

/// The way with lane `l` (a path clicked) in it, the lanes to it filled in: in place of the
/// lanes it is a way round (another branch at a junction), else after the end or before the
/// start. A lane of the way clicked is taken out (at the ends the way is shorter; in between
/// it leaves a gap to mend).
pub fn with_lane(net: &Network, lanes: &[usize], l: usize) -> Vec<usize> {
    if let Some(i) = lanes.iter().position(|x| *x == l) {
        let mut out = lanes.to_vec();
        out.remove(i);
        return out;
    }
    let Some(&last) = lanes.last() else { return vec![l] };
    let from_l = search_from(net, l);
    let to_l = search_to(net, l);
    // a way round: from a lane of the way to `l` and on to a later one, as little of the way
    // left out as can be (and not more than a detour would be: a click far along a ring road
    // adds to the end)
    let mut best: Option<(usize, usize)> = None;
    let mut reach_before: Option<usize> = None;
    for j in 0..lanes.len() {
        if let Some(i) = reach_before {
            if from_l.contains_key(&lanes[j]) && best.map(|b| j - i < b.1 - b.0).unwrap_or(true) {
                best = Some((i, j));
            }
        }
        if to_l.contains_key(&lanes[j]) {
            reach_before = Some(j);
        }
    }
    if let Some((i, j)) = best {
        let left_out: f64 = lanes[i + 1..j].iter().map(|x| net.lanes[*x].length()).sum();
        let detour = to_l[&lanes[i]].0 + net.lanes[l].length() + from_l[&lanes[j]].0;
        if let (true, Some(before), Some(after)) = (left_out <= detour * 2.0 + 300.0, path_to(&to_l, lanes[i], l), path_from(&from_l, l, lanes[j])) {
            let mut out = lanes[..=i].to_vec();
            out.extend(before);
            out.push(l);
            out.extend(after);
            out.extend_from_slice(&lanes[j..]);
            return out;
        }
    }
    // after the end
    if let Some(w) = path_to(&to_l, last, l) {
        let mut out = lanes.to_vec();
        out.extend(w);
        out.push(l);
        return out;
    }
    // before the start
    if let Some(w) = path_from(&from_l, l, lanes[0]) {
        let mut out = vec![l];
        out.extend(w);
        out.extend_from_slice(lanes);
        return out;
    }
    // nowhere joined: at the end, a gap before it
    let mut out = lanes.to_vec();
    out.push(l);
    out
}

/// The lane of the trip's kind of way nearest a point clicked at `p`, within `reach` metres
/// (of a road's two lanes the one whose side of the road `p` lies on).
pub fn lane_at(net: &Network, p: DVec3, reach: f64, ways: Ways, left_hand: bool) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, l) in net.lanes.iter().enumerate() {
        if l.rail != (ways == Ways::Rail) {
            continue;
        }
        let (_, off, right) = l.project(p.truncate());
        if off > reach {
            continue;
        }
        // (the way traffic drives it; a one-way path against its direction only when asked
        // for by a click right on it)
        let score = off + if right != left_hand { 0.0 } else { 0.5 } + if l.against { 1.0 } else { 0.0 };
        if best.map(|b| score < b.1).unwrap_or(true) {
            best = Some((i, score));
        }
    }
    best.map(|b| b.0)
}

/// The way of trip `name` through its stops at `positions`: its paths picked by hand where
/// they are, else as found along the roads.
pub fn of_trip(net: &Network, tt: &super::Timetable, name: &str, order: &[TileKey], positions: &[Option<DVec3>], left_hand: bool) -> Route {
    if tt.by_hand(name) {
        if let Some(lanes) = tt.track(name).and_then(|t| lanes_of_track(net, &t.entries, order).into_iter().collect::<Option<Vec<usize>>>()) {
            return along(net, &lanes, positions, left_hand);
        }
    }
    super::route::find(net, positions, left_hand, super::route::ways_of(tt, name, net, order))
}
