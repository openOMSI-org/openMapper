//! Joining one path's end to another's: the fewest pieces of one radius that leave in the
//! first end's way and arrive in the second's.
//!
//! A join used to be built by stepping forward a piece at a time, turning as far as each piece
//! could, which piles every degree of the turn into the last few centimetres: a metre turning a
//! hundred degrees is a hook, not a junction. What is wanted is the classic `Dubins` answer -
//! turn on one radius, go straight, turn again - which spreads the turn over the whole gap and
//! is at most three pieces.
//!
//! The same three pieces serve both ends of a junction (a turn onto a road, a branch off one)
//! and a free end dragged on the map, where only the arrival point matters.

use glam::DVec2;

use crate::roads::{self, Seg};

/// Which way round a join is to turn. The automatic choice takes the widest arcs it can find;
/// left or right says which way round to go instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    Auto,
    Left,
    Right,
}

impl Side {
    /// Whether a piece turning `turn` degrees (positive right, as the file has it) is the way
    /// this side asks for.
    pub fn allows(self, turn: f64) -> bool {
        match self {
            Side::Auto => true,
            Side::Right => turn >= -1e-6,
            Side::Left => turn <= 1e-6,
        }
    }

    /// +1 right, -1 left, 0 no preference.
    pub fn sign(self) -> f64 {
        match self {
            Side::Left => -1.0,
            Side::Right => 1.0,
            Side::Auto => 0.0,
        }
    }
}

/// A heading as the drawing has it (0 north, clockwise) as the angle the maths has it (0 east,
/// counter-clockwise).
fn math_angle(heading: f64) -> f64 {
    std::f64::consts::FRAC_PI_2 - heading.to_radians()
}

/// The heading that belongs to `angle` (the inverse of [`math_angle`]).
fn heading_of_angle(angle: f64) -> f64 {
    roads::norm_deg((std::f64::consts::FRAC_PI_2 - angle).to_degrees())
}

fn mod_tau(a: f64) -> f64 {
    a.rem_euclid(std::f64::consts::TAU)
}

/// One planned join: its pieces, the radius it turns on, and how long it is all told.
struct Planned {
    radius: f64,
    length: f64,
    segs: Vec<Seg>,
}

/// A join from `start` leaving in `h_a` to `end` arriving in `h_b`, turning on arcs no tighter
/// than `min_radius`, and only the way `side` allows. None when no radius that fits the gap can
/// do it.
///
/// The widest radius that does not much lengthen the join is used: a car turns on the largest
/// radius it can, not the smallest.
pub fn plan(start: DVec2, h_a: f64, end: DVec2, h_b: f64, side: Side, min_radius: f64) -> Option<Vec<Seg>> {
    let chord = (end - start).length();
    if chord < 1e-3 {
        return None;
    }
    let widest = (chord * 1.5 + 4.0).clamp(min_radius, 40.0);
    let mut radii = vec![min_radius];
    for r in [1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 20.0] {
        if r > min_radius + 1e-9 && r <= widest {
            radii.push(r);
        }
    }
    let mut made: Vec<Planned> = Vec::new();
    for r in radii {
        for (s0, s1) in [(1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0)] {
            if let Some(p) = build(start, h_a, end, h_b, r, s0, s1, side) {
                made.push(p);
            }
        }
    }
    let shortest = made.iter().map(|p| p.length).fold(f64::INFINITY, f64::min);
    if !shortest.is_finite() {
        return None;
    }
    // (the widest that is not much longer than the shortest: a gentler curve, if it costs
    // nothing)
    let ceiling = shortest * 1.35 + 1.0;
    made.into_iter().filter(|p| p.length <= ceiling).max_by(|a, b| a.radius.total_cmp(&b.radius)).map(|p| p.segs)
}

/// A join whose arrival way is free: the shortest of the ways into the point, tried every few
/// degrees of arrival. This is what a free end dragged on the map asks for.
pub fn plan_to_point(start: DVec2, h_a: f64, end: DVec2, side: Side, min_radius: f64) -> Option<Vec<Seg>> {
    let mut best: Option<(f64, Vec<Seg>)> = None;
    let mut deg = 0.0;
    while deg < 360.0 {
        if let Some(segs) = plan(start, h_a, end, deg, side, min_radius) {
            let length: f64 = segs.iter().map(|s| s.length).sum();
            if best.as_ref().is_none_or(|(was, _)| length < *was) {
                best = Some((length, segs));
            }
        }
        deg += 5.0;
    }
    best.map(|(_, segs)| segs)
}

/// The pieces of one radius `r` turning `s0` at the start and `s1` at the end (`+1` turns left
/// as the maths has it, `-1` right): the start's circle and the end's joined by their common
/// tangent - a straight along that tangent, and an arc on each circle.
fn build(start: DVec2, h_a: f64, end: DVec2, h_b: f64, r: f64, s0: f64, s1: f64, side: Side) -> Option<Planned> {
    let a0 = math_angle(h_a);
    let a1 = math_angle(h_b);
    // (the centre of a circle turned on the left is on the left of the way: `p + s r n`)
    let c0 = start + DVec2::new(-a0.sin(), a0.cos()) * (s0 * r);
    let c1 = end + DVec2::new(-a1.sin(), a1.cos()) * (s1 * r);
    let (t, straight) = if (s0 - s1).abs() < 1e-9 {
        // (the same way round: the tangent runs parallel to the line between the centres)
        let w = c1 - c0;
        let d = w.length();
        if d < 1e-6 {
            return None;
        }
        (w / d, d)
    } else {
        // (opposite ways: the tangent between them, of which there are two)
        let u = c1 - c0;
        let d = u.length();
        let k = r * (s0 - s1);
        if d < k.abs() + 1e-9 {
            return None;
        }
        let theta = u.y.atan2(u.x);
        let delta = (k / d).asin();
        let mut best: Option<(f64, DVec2)> = None;
        for psi in [theta + delta, theta + std::f64::consts::PI - delta] {
            let dir = DVec2::new(psi.cos(), psi.sin());
            let l = (u + DVec2::new(-psi.sin(), psi.cos()) * k).dot(dir);
            if l < -1e-9 {
                continue;
            }
            let l = l.max(0.0);
            if best.as_ref().is_none_or(|(was, _)| l < *was) {
                best = Some((l, dir));
            }
        }
        let (l, dir) = best?;
        (dir, l)
    };
    let n = DVec2::new(-t.y, t.x);
    // (the point of a circle whose way is `t` is its centre less the radius to the left of it)
    let tp0 = c0 - n * (s0 * r);
    let tp1 = c1 - n * (s1 * r);
    if (tp1 - tp0).length() > straight + 1e-6 {
        return None;
    }
    let d0 = mod_tau(s0 * ((tp0 - c0).y.atan2((tp0 - c0).x) - (start - c0).y.atan2((start - c0).x)));
    let d1 = mod_tau(s1 * ((end - c1).y.atan2((end - c1).x) - (tp1 - c1).y.atan2((tp1 - c1).x)));
    // (`s = +1` is a left turn as the maths has it, and the file writes a left turn as a
    // negative radius)
    let turn0 = -s0 * d0.to_degrees();
    let turn1 = -s1 * d1.to_degrees();
    if !side.allows(turn0) || !side.allows(turn1) {
        return None;
    }
    let across = heading_of_angle(t.y.atan2(t.x));
    let mut segs = Vec::new();
    if d0 > 1e-7 {
        segs.push(Seg { start, heading: roads::norm_deg(h_a), length: r * d0, radius: -s0 * r });
    }
    if straight > 1e-7 {
        let at = segs.last().map(|s: &Seg| s.end()).unwrap_or(start);
        segs.push(Seg { start: at, heading: across, length: straight, radius: 0.0 });
    }
    if d1 > 1e-7 {
        let at = segs.last().map(|s: &Seg| s.end()).unwrap_or(tp1);
        segs.push(Seg { start: at, heading: across, length: r * d1, radius: -s1 * r });
    }
    let last = segs.last()?;
    if (last.end() - end).length() > 0.02 {
        return None;
    }
    let off = roads::norm_deg(last.end_heading() - h_b);
    if off.min(360.0 - off) > 0.5 {
        return None;
    }
    let length: f64 = segs.iter().map(|s| s.length).sum();
    Some(Planned { radius: r, length, segs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_each_join(segs: &[Seg], start: DVec2, end: DVec2) {
        assert!((segs[0].start - start).length() < 1e-6);
        for w in segs.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-6, "a gap at a join");
            let d = roads::norm_deg(w[0].end_heading() - w[1].heading);
            assert!(d.min(360.0 - d) < 0.5, "a kink: {} then {}", w[0].end_heading(), w[1].heading);
        }
        assert!((segs.last().unwrap().end() - end).length() < 0.02, "it does not end on the point: {:?}", segs.last().unwrap().end());
    }

    #[test]
    fn a_join_leaves_and_arrives_the_way_it_was_asked() {
        let a = DVec2::new(0.0, 0.0);
        let b = DVec2::new(6.0, 16.0);
        let segs = plan(a, 0.0, b, 180.0, Side::Auto, 1.5).expect("a join");
        at_each_join(&segs, a, b);
        assert!(segs.len() <= 3, "at most three pieces: {}", segs.len());
        for s in &segs {
            assert!(s.radius == 0.0 || s.radius.abs() >= 1.5, "radius {}", s.radius);
        }
    }

    #[test]
    fn the_turn_is_spread_over_the_gap_not_piled_at_its_end() {
        // (this used to come out as a long run and then a very short piece doing all the
        // turning: no piece may be shorter than a tenth of the gap)
        let a = DVec2::new(0.0, 0.0);
        let b = DVec2::new(14.0, 3.0);
        let segs = plan(a, 0.0, b, 200.0, Side::Auto, 1.5).expect("a join");
        at_each_join(&segs, a, b);
        let gap = (b - a).length();
        for s in &segs {
            assert!(s.length >= gap * 0.1, "a stub of {} m in a {} m join", s.length, gap);
        }
    }

    #[test]
    fn a_side_is_kept() {
        let a = DVec2::new(0.0, 10.0);
        let b = DVec2::new(6.0, 16.0);
        let left = plan(a, 0.0, b, 180.0, Side::Left, 1.5).expect("a left join");
        let right = plan(a, 0.0, b, 180.0, Side::Right, 1.5).expect("a right join");
        for segs in [&left, &right] {
            at_each_join(segs, a, b);
        }
        // (every bend it has turns one way: a straight is no bend)
        assert!(left.iter().filter(|s| s.radius != 0.0).all(|s| s.radius < 0.0), "left turns left: {left:?}");
        assert!(right.iter().filter(|s| s.radius != 0.0).all(|s| s.radius > 0.0), "right turns right: {right:?}");
        assert!(left.iter().any(|s| s.radius < 0.0), "a left join does turn left");
        assert!(right.iter().any(|s| s.radius > 0.0), "a right join does turn right");
    }

    #[test]
    fn a_point_in_line_is_one_straight() {
        let segs = plan(DVec2::ZERO, 0.0, DVec2::new(0.0, 12.0), 0.0, Side::Auto, 1.5).expect("a join");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].radius, 0.0);
        assert!((segs[0].length - 12.0).abs() < 1e-6);
    }

    #[test]
    fn a_free_end_takes_the_shortest_way_in() {
        let segs = plan_to_point(DVec2::ZERO, 0.0, DVec2::new(0.0, 12.0), Side::Auto, 1.5).expect("a join");
        assert_eq!(segs.len(), 1, "straight ahead is one piece: {}", segs.len());
        let behind = plan_to_point(DVec2::ZERO, 0.0, DVec2::new(-6.0, -2.0), Side::Auto, 1.5).expect("a join");
        at_each_join(&behind, DVec2::ZERO, DVec2::new(-6.0, -2.0));
        assert!(behind.len() <= 3, "not a long run: {}", behind.len());
    }
}
