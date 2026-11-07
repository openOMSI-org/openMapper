//! Drawing a road by numbers: a length, a radius or a heading typed while it is drawn, and a
//! straight lock. What is not typed comes from the cursor.

use glam::DVec2;

use crate::roads::{self, Seg};

/// What was typed (None: from the cursor).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Typed {
    /// The road's length (m).
    pub length: Option<f64>,
    /// Its radius (m; 0: straight). From a free start, a negative one turns left.
    pub radius: Option<f64>,
    /// From a free start the heading it sets off with; going on from a road, the heading it
    /// arrives with (degrees, 0 north, clockwise).
    pub heading: Option<f64>,
}

impl Typed {
    pub fn any(&self) -> bool {
        self.length.is_some() || self.radius.is_some() || self.heading.is_some()
    }
}

fn turn_rad(from: f64, to: f64, right: bool) -> f64 {
    let d = roads::norm_deg(if right { to - from } else { from - to });
    d.to_radians()
}

/// The road from `p` towards `cursor`: `h` the heading it must leave with (going on from a
/// road), `straight` the straight lock.
pub fn plan(p: DVec2, h: Option<f64>, cursor: DVec2, t: &Typed, straight: bool) -> Option<Vec<Seg>> {
    // (a number typed with a zero too many: no road longer than any map)
    plan_any(p, h, cursor, t, straight).filter(|s| roads::total_length(s) <= MAX_LENGTH)
}

/// The longest road drawn at once (m).
pub const MAX_LENGTH: f64 = 20_000.0;

fn plan_any(p: DVec2, h: Option<f64>, cursor: DVec2, t: &Typed, straight: bool) -> Option<Vec<Seg>> {
    let to = cursor - p;
    let straight = straight || t.radius.map(|r| r.abs() < 1e-6).unwrap_or(false);
    match h {
        _ if straight => {
            // along the typed heading, else on from the road, else at the cursor
            let d = t.heading.map(roads::dir).or(h.map(roads::dir)).unwrap_or_else(|| to.normalize_or_zero());
            if d.length() < 0.5 {
                return None;
            }
            let len = t.length.unwrap_or_else(|| to.dot(d));
            (len > 0.5).then(|| vec![Seg { start: p, heading: roads::heading_of(d), length: len, radius: 0.0 }])
        }
        Some(h0) => {
            let right = to.dot(roads::right(h0)) >= 0.0;
            match (t.radius, t.heading, t.length) {
                (Some(r), _, _) => {
                    let r = r.abs();
                    let len = match (t.length, t.heading) {
                        (Some(l), _) => l,
                        (None, Some(hh)) => turn_rad(h0, hh, right) * r,
                        // to the point of the circle nearest the cursor
                        (None, None) => {
                            let c = p + roads::right(h0) * if right { r } else { -r };
                            let (a, b) = (p - c, cursor - c);
                            let mut sweep = a.perp_dot(b).atan2(a.dot(b));
                            // (clockwise is a right turn)
                            sweep = if right { -sweep } else { sweep };
                            if sweep < 0.0 {
                                sweep += std::f64::consts::TAU;
                            }
                            sweep * r
                        }
                    };
                    (len > 0.5).then(|| vec![Seg { start: p, heading: h0, length: len, radius: if right { r } else { -r } }])
                }
                (None, Some(hh), Some(l)) => {
                    // the turn spread evenly over the length
                    let right = roads::norm_deg(hh - h0) <= 180.0;
                    let a = turn_rad(h0, hh, right);
                    if a < 1e-6 {
                        return Some(vec![Seg { start: p, heading: h0, length: l, radius: 0.0 }]);
                    }
                    let r = l / a;
                    (r.is_finite() && r > 0.0 && l > 0.5).then(|| vec![Seg { start: p, heading: h0, length: l, radius: if right { r } else { -r } }])
                }
                (None, Some(hh), None) => roads::biarc(p, h0, cursor, hh),
                (None, None, Some(l)) => {
                    let s = roads::arc_to(p, h0, cursor)?;
                    (l > 0.5).then(|| vec![Seg { length: l, ..s }])
                }
                (None, None, None) => roads::arc_to(p, h0, cursor).map(|s| vec![s]),
            }
        }
        None => match (t.heading, t.radius) {
            // a heading to set off with: from there as going on from a road
            (Some(hh), Some(_)) => plan(p, Some(hh), cursor, &Typed { heading: None, ..*t }, false),
            (Some(_), None) => plan(p, None, cursor, t, true),
            // a radius alone: the arc of that radius through the cursor
            (None, Some(r)) => {
                let d = to.length();
                let ra = r.abs();
                if d < 0.5 || d > 2.0 * ra {
                    return None;
                }
                let half = (d / (2.0 * ra)).asin().to_degrees();
                let h0 = roads::heading_of(to) + if r < 0.0 { half } else { -half };
                let s = roads::arc_to(p, roads::norm_deg(h0), cursor)?;
                Some(vec![match t.length {
                    Some(l) => Seg { length: l, ..s },
                    None => s,
                }])
            }
            (None, None) => plan(p, None, cursor, t, true),
        },
    }
}

/// Parse a number typed in a field (a comma is a decimal point too; empty: none).
pub fn parse(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(segs: &[Seg]) -> (DVec2, f64) {
        let l = segs.last().unwrap();
        (l.end(), roads::norm_deg(l.end_heading()))
    }

    #[test]
    fn a_typed_length_from_a_free_start_goes_towards_the_cursor() {
        let s = plan(DVec2::ZERO, None, DVec2::new(30.0, 40.0), &Typed { length: Some(10.0), ..Default::default() }, false).unwrap();
        assert!((end(&s).0 - DVec2::new(6.0, 8.0)).length() < 1e-9);
    }

    #[test]
    fn the_straight_lock_goes_on_along_the_road() {
        let s = plan(DVec2::ZERO, Some(90.0), DVec2::new(50.0, 20.0), &Typed::default(), true).unwrap();
        assert!((end(&s).0 - DVec2::new(50.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn a_radius_and_a_heading_make_that_curve() {
        // going north, a right turn of radius 20 to heading east: a quarter circle
        let s = plan(DVec2::ZERO, Some(0.0), DVec2::new(30.0, 10.0), &Typed { radius: Some(20.0), heading: Some(90.0), ..Default::default() }, false).unwrap();
        let (q, h) = end(&s);
        assert!((q - DVec2::new(20.0, 20.0)).length() < 1e-6, "{q:?}");
        assert!((h - 90.0).abs() < 1e-6);
        // a left one with the cursor on the left
        let s = plan(DVec2::ZERO, Some(0.0), DVec2::new(-30.0, 10.0), &Typed { radius: Some(20.0), heading: Some(270.0), ..Default::default() }, false).unwrap();
        assert!((end(&s).0 - DVec2::new(-20.0, 20.0)).length() < 1e-6);
    }

    #[test]
    fn a_radius_alone_ends_nearest_the_cursor() {
        let s = plan(DVec2::ZERO, Some(0.0), DVec2::new(40.0, 20.0), &Typed { radius: Some(20.0), ..Default::default() }, false).unwrap();
        // the circle round (20, 0): the cursor lies due east of... its point nearest it
        let q = end(&s).0;
        assert!(((q - DVec2::new(20.0, 0.0)).length() - 20.0).abs() < 1e-6);
        assert!((q - DVec2::new(20.0, 0.0)).normalize().dot((DVec2::new(40.0, 20.0) - DVec2::new(20.0, 0.0)).normalize()) > 0.999);
    }

    #[test]
    fn a_length_and_a_heading_spread_the_turn() {
        let s = plan(DVec2::ZERO, Some(0.0), DVec2::new(10.0, 10.0), &Typed { length: Some(31.4159265), heading: Some(90.0), ..Default::default() }, false).unwrap();
        assert!((s[0].radius - 20.0).abs() < 1e-6);
    }

    #[test]
    fn numbers_typed_either_way() {
        assert_eq!(parse(" 12,5 "), Some(12.5));
        assert_eq!(parse(""), None);
        assert_eq!(parse("x"), None);
    }

    /// A number typed with a zero too many does not plan a road of hundreds of kilometres
    /// (split into 50 m pieces and shown every frame, it held the window).
    #[test]
    fn a_road_beyond_any_map_is_not_planned() {
        assert!(plan(DVec2::ZERO, None, DVec2::new(30.0, 40.0), &Typed { length: Some(1e8), ..Default::default() }, false).is_none());
        assert!(plan(DVec2::ZERO, Some(0.0), DVec2::new(30.0, 40.0), &Typed { radius: Some(1e7), heading: Some(90.0), ..Default::default() }, false).is_none());
    }
}
