//! An object's `[path]`s laid into the world as Omsi.exe lays them, the same way its mesh is:
//! the curve in the object's frame, lifted by the object's `[crossing_heightdeformation]`
//! field where it reaches, then turned and moved by the whole placement (heading, pitch,
//! bank, height). A road object whose field drops its plate down a hill had its mesh draped
//! and its lanes left flat at the plate's base height: metres over (or under) the road it
//! drew, and the lanes of the roads below no longer met them (#120).

use std::path::Path;

use glam::{DVec3, Vec3};
use omsi_geometry::{MeshData, SplineCurve};
use omsi_scenery::SceneryObject;

use crate::place::Pose;

/// The object's `[crossing_heightdeformation]` field, read as the object's model reads it:
/// in the model folder's `model` subfolder first, then beside the model.
pub fn load_field(sco: &SceneryObject, sco_path: &Path) -> Option<MeshData> {
    let f = sco.crossing_height_deformation.as_ref()?;
    let sco_dir = sco_path.parent()?;
    let model_dir = match &sco.model_file {
        Some(m) => omsi_cfg::resolve_path(sco_dir, m).parent()?.to_path_buf(),
        None => sco_dir.to_path_buf(),
    };
    let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), f);
    let p = if omsi_cfg::vfs::is_file(&in_model) { in_model } else { omsi_cfg::resolve_path(&model_dir, f) };
    match omsi_o3d::load_mesh(&p) {
        Ok(m) => Some(omsi_geometry::mesh_from_o3d(&m)),
        Err(e) => {
            log::warn!("crossing height deformation {}: {e}", p.display());
            None
        }
    }
}

/// The height of a `[crossing_heightdeformation]` field over (x, y) in the object's frame,
/// where the field reaches (the highest face there; openOMSI's `field_height`).
pub fn field_height(m: &MeshData, x: f32, y: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
        let det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if det.abs() < 1e-9 {
            continue;
        }
        let l1 = ((b.x - a.x) * (y - a.y) - (x - a.x) * (b.y - a.y)) / det;
        let l2 = ((x - a.x) * (c.y - a.y) - (c.x - a.x) * (y - a.y)) / det;
        let l0 = 1.0 - l1 - l2;
        if l0 >= -1e-4 && l1 >= -1e-4 && l2 >= -1e-4 {
            let h = l0 * a.z + l2 * b.z + l1 * c.z;
            best = Some(best.map_or(h, |o: f32| o.max(h)));
        }
    }
    best
}

/// A path's point `s` metres along it, in the world: lifted by the field where the field
/// reaches (a vertical ray missing it leaves the height as the file gives it, as for the
/// mesh), then placed by the object's pose.
pub fn world_point(c: &SplineCurve, s: f64, pose: &Pose, field: Option<&MeshData>) -> DVec3 {
    let q = c.point_at(s);
    let mut local = Vec3::new(q.x as f32, q.y as f32, q.z as f32);
    if let Some(d) = field.and_then(|f| field_height(f, local.x, local.y)) {
        local.z += d;
    }
    pose.pos + pose.rot.transform_vector3(local).as_dvec3()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field that falls 4 m over 20 m along the object's forward axis: a plate laid down
    /// a hill, as Städtedreieck's road objects are.
    fn slope() -> MeshData {
        let mut m = MeshData::default();
        m.positions = vec![Vec3::new(-10.0, -1.0, 0.0), Vec3::new(10.0, -1.0, 0.0), Vec3::new(10.0, 21.0, -4.4), Vec3::new(-10.0, 21.0, -4.4)];
        m.indices = vec![0, 1, 2, 0, 2, 3];
        m
    }

    fn straight(length: f64) -> SplineCurve {
        crate::lanes::object_path_curve(&omsi_scenery::PathDef { start: [0.0, 0.0, 0.0], end: [0.0, 0.0, length as f32], params: vec![0.0; 8], ..Default::default() })
    }

    #[test]
    fn a_path_follows_its_objects_height_field() {
        let field = slope();
        let pose = Pose::of_object(DVec3::new(100.0, 200.0, 30.0), [0.0, 0.0, 0.0]);
        let c = straight(20.0);
        let a = world_point(&c, 0.0, &pose, Some(&field));
        let b = world_point(&c, 20.0, &pose, Some(&field));
        assert!((a.z - 29.8).abs() < 1e-3, "the start is on the field: {}", a.z);
        assert!((b.z - 25.8).abs() < 1e-3, "and so is the end, 4 m lower: {}", b.z);
        // (without the field the path stays at the plate's own height, as the mesh would)
        assert!((world_point(&c, 20.0, &pose, None).z - 30.0).abs() < 1e-6);
    }

    #[test]
    fn a_path_off_the_field_keeps_its_height_and_the_pose_turns_it() {
        let field = slope();
        // (turned 90° clockwise: the object's forward is east; placed 2 m under the ground
        // the map gives, the field carries none of that away)
        let pose = Pose::of_object(DVec3::new(0.0, 0.0, -2.0), [90.0, 0.0, 0.0]);
        let c = straight(40.0);
        let mid = world_point(&c, 10.0, &pose, Some(&field));
        assert!((mid.x - 10.0).abs() < 1e-3 && mid.y.abs() < 1e-3, "{mid:?}");
        assert!((mid.z - (-2.0 - 0.2 - 2.0)).abs() < 1e-3, "{}", mid.z);
        let far = world_point(&c, 40.0, &pose, Some(&field));
        assert!((far.z + 2.0).abs() < 1e-6, "past the field the path keeps the height it has: {}", far.z);
    }
}
