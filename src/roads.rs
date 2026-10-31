//! Laying roads the easy way: click where a road starts and where it goes, and the editor
//! works out the arcs. A road continues smoothly from whatever end it starts at, meets an
//! end it is dropped on with a smooth S or bend (a biarc), is cut into OMSI-sized spline
//! pieces and linked to its neighbours.

use glam::{DVec2, DVec3};
use omsi_geometry::SplineCurve;

/// Unit vector of a heading (degrees, 0 = north, clockwise).
pub fn dir(h: f64) -> DVec2 {
    SplineCurve::dir(h)
}

/// Right of a heading.
pub fn right(h: f64) -> DVec2 {
    let d = dir(h);
    DVec2::new(d.y, -d.x)
}

/// Heading of a direction vector.
pub fn heading_of(v: DVec2) -> f64 {
    norm_deg(v.x.atan2(v.y).to_degrees())
}

pub fn norm_deg(h: f64) -> f64 {
    h.rem_euclid(360.0)
}

/// One arc or straight in the plane.
#[derive(Debug, Clone, Copy)]
pub struct Seg {
    pub start: DVec2,
    pub heading: f64,
    pub length: f64,
    /// 0 straight, > 0 turns right (m).
    pub radius: f64,
}

impl Seg {
    pub fn curve(&self) -> SplineCurve {
        SplineCurve {
            start: DVec3::new(self.start.x, self.start.y, 0.0),
            heading_deg: self.heading,
            length: self.length,
            radius: self.radius,
            grad_start: 0.0,
            grad_end: 0.0,
            delta_h: None,
            cant_start: 0.0,
            cant_end: 0.0,
            skew_start: 0.0,
            skew_end: 0.0,
            tex_offset: 0.0,
            seed: 0,
            half_cant_width: 0.0,
        }
    }

    pub fn point(&self, s: f64) -> DVec2 {
        self.curve().point_at(s).truncate()
    }

    pub fn heading_at(&self, s: f64) -> f64 {
        norm_deg(self.curve().heading_at(s))
    }

    pub fn end(&self) -> DVec2 {
        self.point(self.length)
    }

    pub fn end_heading(&self) -> f64 {
        self.heading_at(self.length)
    }

    /// The part from `a` to `b` along it.
    pub fn piece(&self, a: f64, b: f64) -> Seg {
        Seg { start: self.point(a), heading: self.heading_at(a), length: b - a, radius: self.radius }
    }
}

/// The most a road drawn to meet an end may turn in all (radians): about 200 degrees.
pub const MAX_TURN: f64 = 3.5;

/// The smallest radius a drawn road may have (m).
pub const MIN_RADIUS: f64 = 6.0;

/// The arc that leaves `p` heading `h` and ends at `q` (a straight when `q` lies ahead).
pub fn arc_to(p: DVec2, h: f64, q: DVec2) -> Option<Seg> {
    let d = q - p;
    let fwd = d.dot(dir(h));
    let side = d.dot(right(h));
    let dist = d.length();
    if dist < 0.5 {
        return None;
    }
    if side.abs() < 1e-4 * dist {
        return (fwd > 0.0).then_some(Seg { start: p, heading: h, length: fwd, radius: 0.0 });
    }
    let r = (fwd * fwd + side * side) / (2.0 * side);
    let sweep = 2.0 * side.atan2(fwd);
    let length = r * sweep;
    // (a loop back past a half turn is not what anyone means)
    if r.abs() < MIN_RADIUS || sweep.abs() > std::f64::consts::PI * 1.01 || length <= 0.0 {
        return None;
    }
    Some(Seg { start: p, heading: h, length, radius: r })
}

/// A straight from `p` to `q`.
pub fn straight(p: DVec2, q: DVec2) -> Option<Seg> {
    let d = q - p;
    (d.length() > 0.5).then(|| Seg { start: p, heading: heading_of(d), length: d.length(), radius: 0.0 })
}

/// Two arcs from `p` (heading `hp`) to `q` arriving with heading `hq`, meeting tangentially
/// halfway (the equal-distance biarc).
pub fn biarc(p: DVec2, hp: f64, q: DVec2, hq: f64) -> Option<Vec<Seg>> {
    let t1 = dir(hp);
    let t2 = dir(hq);
    let v = q - p;
    if v.length() < 1.0 {
        return None;
    }
    let t = t1 + t2;
    let denom = 2.0 * (1.0 - t1.dot(t2));
    let vt = v.dot(t);
    let d = if denom.abs() < 1e-9 {
        // parallel tangents
        let k = v.dot(t2);
        if k.abs() < 1e-9 {
            return None;
        }
        v.dot(v) / (4.0 * k)
    } else {
        (-vt + (vt * vt + denom * v.dot(v)).sqrt()) / denom
    };
    if !(d > 0.0) || !d.is_finite() {
        return None;
    }
    let joint = (p + t1 * d + q - t2 * d) * 0.5;
    let a = arc_to(p, hp, joint)?;
    let b = arc_to(joint, a.end_heading(), q)?;
    // the second arc must arrive along hq
    let miss = (norm_deg(b.end_heading() - hq + 180.0) - 180.0).abs();
    if miss > 2.0 {
        return None;
    }
    Some(vec![a, b])
}

/// Where a road may start or end: a free end of a spline, or a spline helper point of a
/// junction object. `heading` points away from what is there (the way a road goes on).
#[derive(Debug, Clone, Copy, Default)]
pub struct Anchor {
    pub pos: DVec3,
    pub heading: f64,
    /// The spline (tile, id, at its end rather than its start), when it is one.
    pub spline: Option<((i32, i32), i64, bool)>,
    /// A point in the middle of a road: the new road joins it there and the road is cut in
    /// two at that point (tile, id, distance along it, its half width).
    pub mid: Option<((i32, i32), i64, f64, f64)>,
}

/// The road the user is drawing, from its start to the cursor.
pub fn plan(start: &Anchor, start_fixed: bool, end: DVec2, end_anchor: Option<&Anchor>) -> Option<Vec<Seg>> {
    let p = start.pos.truncate();
    match (start_fixed, end_anchor) {
        (true, Some(e)) => {
            let hq = norm_deg(e.heading + 180.0);
            // (a curve that turns most of the way round to meet the end is a loop, not a
            // road: the end is reached some other way or not at all)
            biarc(p, start.heading, e.pos.truncate(), hq)
                .filter(|segs| segs.iter().map(|s| if s.radius == 0.0 { 0.0 } else { (s.length / s.radius).abs() }).sum::<f64>() < MAX_TURN)
                .or_else(|| {
                    // (one arc only where it arrives about along the end's own line)
                    let arc = arc_to(p, start.heading, e.pos.truncate())?;
                    ((norm_deg(arc.end_heading() - hq + 180.0) - 180.0).abs() < 30.0).then(|| vec![arc])
                })
        }
        (true, None) => arc_to(p, start.heading, end).map(|s| vec![s]),
        (false, Some(e)) => {
            // drawn backwards from the end it meets, then turned round
            let back = arc_to(e.pos.truncate(), e.heading, p)?;
            Some(vec![reverse(&back)])
        }
        (false, None) => straight(p, end).map(|s| vec![s]),
    }
}

/// The same arc run the other way.
pub fn reverse(s: &Seg) -> Seg {
    Seg { start: s.end(), heading: norm_deg(s.end_heading() + 180.0), length: s.length, radius: -s.radius }
}

/// Cut the segments into pieces no longer than `max_len`.
pub fn split(segs: &[Seg], max_len: f64) -> Vec<Seg> {
    let mut out = Vec::new();
    for s in segs {
        let n = (s.length / max_len).ceil().max(1.0) as usize;
        let step = s.length / n as f64;
        for i in 0..n {
            out.push(s.piece(step * i as f64, step * (i + 1) as f64));
        }
    }
    out
}

pub fn total_length(segs: &[Seg]) -> f64 {
    segs.iter().map(|s| s.length).sum()
}

/// A spline that starts at `start` and runs `length` metres can come within `slack` of `p`
/// (its end lies somewhere on that length, so anything further away cannot meet `p`).
pub fn can_reach(start: DVec2, length: f64, p: DVec2, slack: f64) -> bool {
    (start - p).length() <= length.max(0.0) + slack
}

/// The tile square at `origin` (side `side`) comes within `slack` of `p`.
pub fn tile_near(origin: DVec2, side: f64, p: DVec2, slack: f64) -> bool {
    let closest = DVec2::new(p.x.clamp(origin.x, origin.x + side), p.y.clamp(origin.y, origin.y + side));
    (closest - p).length() <= slack
}

/// How far the roads tool marks free ends (m): what is on screen, and no further than 500,
/// so a map of tens of thousands of splines is not walked and drawn in full every frame.
pub fn end_marks_reach(distance: f64) -> f64 {
    (distance * 1.5).clamp(80.0, 500.0)
}

/// A junction helper stands within this of its object's tile (m). Further than
/// `within + HELPER_REACH` and the tile cannot hold a snap.
pub const HELPER_REACH: f64 = 200.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arc_ends_where_asked() {
        let p = DVec2::new(10.0, 10.0);
        let q = DVec2::new(60.0, 80.0);
        let a = arc_to(p, 0.0, q).unwrap();
        assert!((a.end() - q).length() < 1e-6, "{:?}", a.end());
        assert!(a.radius > 0.0, "q lies to the right: a right turn");
    }

    #[test]
    fn a_biarc_meets_both_tangents() {
        let p = DVec2::new(0.0, 0.0);
        let q = DVec2::new(40.0, 100.0);
        let segs = biarc(p, 0.0, q, 0.0).unwrap();
        assert_eq!(segs.len(), 2);
        assert!((segs[1].end() - q).length() < 1e-6);
        assert!((norm_deg(segs[1].end_heading() + 180.0) - 180.0).abs() < 1e-6);
        // tangent at the joint
        assert!((segs[0].end_heading() - segs[1].heading).abs() < 1e-9);
    }

    #[test]
    fn an_end_is_not_met_by_a_loop() {
        // an end at (0, 0) facing east, the new road starting beside it facing east too: it
        // would have to go all the way round
        let end = Anchor { pos: DVec3::ZERO, heading: 90.0, ..Default::default() };
        let start = Anchor { pos: DVec3::new(0.0, 12.0, 0.0), heading: 90.0, ..Default::default() };
        if let Some(segs) = plan(&start, true, end.pos.truncate(), Some(&end)) {
            let turn: f64 = segs.iter().map(|s| if s.radius == 0.0 { 0.0 } else { (s.length / s.radius).abs() }).sum();
            assert!(turn < MAX_TURN, "{turn}");
        }
        // a plain S-bend is still made
        let start = Anchor { pos: DVec3::new(-60.0, 20.0, 0.0), heading: 90.0, ..Default::default() };
        let end = Anchor { pos: DVec3::ZERO, heading: 270.0, ..Default::default() };
        assert!(plan(&start, true, end.pos.truncate(), Some(&end)).is_some());
    }

    #[test]
    fn split_keeps_the_shape() {
        let a = arc_to(DVec2::ZERO, 90.0, DVec2::new(100.0, -60.0)).unwrap();
        let parts = split(&[a], 30.0);
        assert!(parts.len() >= 4);
        let last = parts.last().unwrap();
        assert!((last.end() - a.end()).length() < 1e-6);
    }

    #[test]
    fn a_far_spline_is_not_a_snap_and_a_near_one_is() {
        let p = DVec2::ZERO;
        // the next district: 2 km off and only 40 m long, it cannot meet the cursor
        assert!(!can_reach(DVec2::new(2_000.0, 0.0), 40.0, p, 5.0));
        // the same start, long enough that its far end can still land on the cursor
        assert!(can_reach(DVec2::new(2_000.0, 0.0), 2_000.0, p, 5.0));
        // a road that starts where the cursor is
        assert!(can_reach(DVec2::new(3.0, 0.0), 20.0, p, 5.0));
        // the cursor's own tile, and not one a kilometre away (a helper cannot stand 200 m
        // outside its tile and still be within 5 m of the cursor)
        assert!(tile_near(DVec2::ZERO, 300.0, DVec2::new(10.0, 20.0), 5.0));
        assert!(!tile_near(DVec2::new(1_000.0, 0.0), 300.0, p, 5.0 + HELPER_REACH));
    }

    #[test]
    fn end_marks_stay_on_screen_and_stop_at_half_a_kilometre() {
        assert_eq!(end_marks_reach(10.0), 80.0, "zoomed in, still a neighbourhood");
        assert_eq!(end_marks_reach(100.0), 150.0);
        assert_eq!(end_marks_reach(10_000.0), 500.0, "zoomed out, the whole map is not marked");
    }
}
