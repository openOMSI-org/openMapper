//! Where the Roads tool catches the end of a spline without AI paths (a kerb, a wall, a fence).
//!
//! A road's line is its middle and its lanes start there, so its end is caught at the line. A
//! kerb's line may lie metres beside what is drawn (it is laid along a road's middle and drawn
//! at the road's edge): its end is caught where it is seen, the middle of its profile. A spline
//! of the same type goes on from the line's end, so the two run on as one; another type is put
//! with the middle of its own profile on that point.

use glam::DVec3;

use crate::assets::SplineModel;
use crate::roads;

/// The middle of what a spline type draws, metres to the right of its line (`mirror`: as a
/// mirrored spline draws it). Nothing for a type with AI paths (its line is its middle) or one
/// that draws nothing.
pub fn profile_middle(m: &SplineModel, mirror: bool) -> f64 {
    if !m.def.paths.is_empty() {
        return 0.0;
    }
    let xs = || m.def.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.x as f64);
    let (lo, hi) = (xs().fold(f64::INFINITY, f64::min), xs().fold(f64::NEG_INFINITY, f64::max));
    if !lo.is_finite() || !hi.is_finite() {
        return 0.0;
    }
    let mid = (lo + hi) * 0.5;
    if mirror {
        -mid
    } else {
        mid
    }
}

/// A free end caught: where it is seen (what the cursor is measured to) and where the new
/// spline's line goes.
///
/// `line_end` is the end of the old spline's line, `along` the way that spline runs there (its
/// own heading), `outward` the way out of it (the anchor's heading), `end_mid` the middle of its
/// profile ([`profile_middle`]) and `new_mid` that of the type drawn. `starting`: the new spline
/// leaves from here (else it arrives here). `same_type`: the new spline is of the old one's type
/// and goes on from its line.
pub fn catch(line_end: DVec3, along: f64, outward: f64, end_mid: f64, new_mid: f64, starting: bool, same_type: bool) -> (DVec3, DVec3) {
    let r = roads::right(along);
    let seen = line_end + DVec3::new(r.x, r.y, 0.0) * end_mid;
    if same_type {
        return (seen, line_end);
    }
    // (the way the new spline runs there: out of the end when it starts at it, into it when it
    // ends at it. Its profile's middle is that far to the right of its own line)
    let travel = if starting { outward } else { roads::norm_deg(outward + 180.0) };
    let n = roads::right(travel);
    (seen, seen - DVec3::new(n.x, n.y, 0.0) * new_mid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    fn kerb(x0: f32, x1: f32) -> SplineModel {
        SplineModel {
            def: omsi_scenery::Spline {
                profiles: vec![omsi_scenery::SplineProfile {
                    texture: 0,
                    points: vec![
                        omsi_scenery::SplineProfilePoint { x: x0, z: 0.0, ..Default::default() },
                        omsi_scenery::SplineProfilePoint { x: x1, z: 0.15, ..Default::default() },
                    ],
                }],
                ..Default::default()
            },
            materials: Vec::new(),
            half_width: x0.abs().max(x1.abs()),
        }
    }

    #[test]
    fn a_kerbs_end_is_where_it_is_drawn() {
        let k = kerb(3.0, 3.4);
        assert!((profile_middle(&k, false) - 3.2).abs() < 1e-6);
        assert!((profile_middle(&k, true) + 3.2).abs() < 1e-6, "a mirrored kerb is drawn on the left");
        // (a kerb running north, its line ending at (0, 10): seen 3.2 m east of it)
        let (seen, _) = catch(DVec3::new(0.0, 10.0, 0.0), 0.0, 0.0, 3.2, 0.0, true, false);
        assert!((seen.truncate() - DVec2::new(3.2, 10.0)).length() < 1e-9, "{seen:?}");
    }

    #[test]
    fn a_road_keeps_its_line() {
        let mut road = kerb(-3.5, 3.5);
        road.def.paths.push(Default::default());
        assert_eq!(profile_middle(&road, false), 0.0);
    }

    #[test]
    fn the_same_kerb_goes_on_from_the_line_and_another_meets_its_middle() {
        let end = DVec3::new(0.0, 10.0, 0.0);
        // (the same type: line to line, so the two run on as one)
        let (_, pos) = catch(end, 0.0, 0.0, 3.2, 3.2, true, true);
        assert!((pos - end).length() < 1e-9);
        // (a fence, drawn on its line, goes on from the kerb's middle)
        let (seen, pos) = catch(end, 0.0, 0.0, 3.2, 0.0, true, false);
        assert!((pos - seen).length() < 1e-9);
        // (another kerb whose middle is 1 m right of its line, going on north: its line 1 m
        // left of the old one's middle, so the two middles meet)
        let (seen, pos) = catch(end, 0.0, 0.0, 3.2, 1.0, true, false);
        assert!((pos.truncate() - DVec2::new(2.2, 10.0)).length() < 1e-9, "{pos:?}");
        let r = roads::right(0.0);
        assert!((pos.truncate() + r * 1.0 - seen.truncate()).length() < 1e-9, "the middles meet");
        // (the same kerb drawn to that end, arriving from the north: running south, its middle
        // is on its right, the west, so its line lies east of the old middle)
        let (seen, pos) = catch(end, 0.0, 0.0, 3.2, 1.0, false, false);
        let travel = 180.0;
        let r = roads::right(travel);
        assert!((pos.truncate() + r * 1.0 - seen.truncate()).length() < 1e-9, "the middles meet: {pos:?} {seen:?}");
        assert!((pos.truncate() - DVec2::new(4.2, 10.0)).length() < 1e-9, "{pos:?}");
    }
}
