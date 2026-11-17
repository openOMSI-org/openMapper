//! Every road, track and path end of a map and what it meets: another spline (linked), a
//! junction plate's lane ends, another end lying on it unjoined, or nothing. One answer for
//! the MCP pictures' labels and for the street checks.

use glam::{DVec2, DVec3, Vec3};

use crate::assets::Assets;
use crate::doc::{tile_origin, Document, TileKey};
use crate::library::{Category, Library};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndState {
    /// Joined to the next spline.
    Linked,
    /// Ends on a junction plate (or another object's lanes) across its whole width.
    Plate,
    /// Another free end lies on it (within 0.6 m), not joined.
    Touching,
    /// Nothing there: a dead end, the map's edge or a break.
    Free,
}

#[derive(Debug, Clone)]
pub struct RoadEnd {
    pub tile: TileKey,
    pub id: i64,
    /// The end: false = start, true = end.
    pub at_end: bool,
    pub pos: DVec3,
    /// The heading pointing out of the spline (degrees, 0 = north, clockwise).
    pub heading: f64,
    pub half_width: f64,
    pub cat: Category,
    pub state: EndState,
}

/// Where the lane ends of objects lie (world): junction plates, crossings, car parks.
fn object_lane_ends(doc: &Document, assets: &mut Assets) -> Vec<DVec3> {
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for o in &t.tile.objects {
            let Some(model) = assets.object(&o.file) else { continue };
            if model.paths.is_empty() {
                continue;
            }
            let p = org + DVec2::new(o.pos[0], o.pos[1]);
            let world = if model.abs_height { DVec3::new(p.x, p.y, o.pos[2]) } else { DVec3::new(p.x, p.y, doc.ground(p) + o.pos[2]) };
            let pose = crate::place::Pose::of_object(world, o.rot);
            for q in &model.paths {
                let c = crate::lanes::object_path_curve(q);
                if c.length <= 0.05 {
                    continue;
                }
                for s in [0.0, c.length] {
                    let v = c.point_at(s);
                    out.push(pose.pos + pose.rot.transform_vector3(Vec3::new(v.x as f32, v.y as f32, v.z as f32)).as_dvec3());
                }
            }
        }
    }
    out
}

/// Every end of the roads, tracks and paths (and of technical splines with `technical`).
pub fn road_ends(doc: &Document, lib: &Library, assets: &mut Assets, technical: bool) -> Vec<RoadEnd> {
    assets.load_map_models(doc, 60_000);
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for sp in t.tile.splines.iter().filter(|x| !x.deleted) {
            let asset = lib.find(&sp.file);
            let made = if asset.is_none() { assets.spline(&sp.file) } else { None };
            let cat = asset.map(|a| a.category).or(made.as_ref().map(|m| if m.is_track() { Category::Tracks } else { Category::Roads })).unwrap_or(Category::Decoration);
            if !technical && !matches!(cat, Category::Roads | Category::Tracks | Category::Paths) {
                continue;
            }
            let half = asset.map(|a| a.half_width as f64).or(made.as_ref().map(|m| m.half_width as f64)).unwrap_or(1.0);
            let c = omsi_geometry::SplineCurve::from_map(sp, org);
            let (h0, h1) = (c.heading_at(0.0), c.heading_at(sp.length.max(0.0)));
            out.push(RoadEnd { tile: *k, id: sp.id, at_end: false, pos: c.point_at(0.0), heading: crate::roads::norm_deg(h0 + 180.0), half_width: half, cat, state: if sp.prev_id != 0 { EndState::Linked } else { EndState::Free } });
            out.push(RoadEnd { tile: *k, id: sp.id, at_end: true, pos: c.end_point(), heading: crate::roads::norm_deg(h1), half_width: half, cat, state: if sp.next_id != 0 { EndState::Linked } else { EndState::Free } });
        }
    }
    let lanes = object_lane_ends(doc, assets);
    let free: Vec<DVec3> = out.iter().filter(|e| e.state == EndState::Free).map(|e| e.pos).collect();
    for e in out.iter_mut().filter(|e| e.state == EndState::Free) {
        // (on a plate: object lane ends on the cross line through the end, within its width)
        let h = e.heading.to_radians();
        let (fwd, side) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
        let on_plate = lanes.iter().any(|q| {
            let d = (*q - e.pos).truncate();
            d.dot(fwd).abs() < 0.8 && d.dot(side).abs() <= e.half_width + 0.3 && (q.z - e.pos.z).abs() < 1.5
        });
        if on_plate {
            e.state = EndState::Plate;
        } else if free.iter().filter(|q| (**q - e.pos).truncate().length() < 0.6 && (q.z - e.pos.z).abs() < 0.5).count() > 1 {
            e.state = EndState::Touching;
        }
    }
    out
}
