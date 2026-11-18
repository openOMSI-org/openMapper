//! Where things a tile places indirectly stand, as openOMSI puts them (its `omsi-world`): the
//! objects of `[splineAttachement]` rows along their spline chains, and `[attachObj]` objects
//! hanging on a `[new_attachment]` point of their parent.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use glam::{DVec2, DVec3, Mat4};
use omsi_map::{MapSpline, SplineAttachment};
use omsi_world::map_index::{self, IndexedSpline, MapIndex};

use crate::doc::Document;

pub use omsi_world::map_index::attachment_matrix;

/// A world pose: position, and the rotation of the object's frame.
#[derive(Debug, Clone, Copy)]
pub struct Pose {
    pub pos: DVec3,
    pub rot: Mat4,
}

impl Pose {
    /// A plain map object's pose.
    pub fn of_object(pos: DVec3, rot_deg: [f64; 3]) -> Pose {
        Pose { pos, rot: omsi_geometry::object_rotation(omsi_geometry::map_rotation(rot_deg)) }
    }

    /// The pose's matrix with its position taken relative to `anchor` (f32 keeps
    /// millimetres only near it: Berlin-Spandau lies 4 200 km from the map's origin).
    pub fn matrix_from(&self, anchor: DVec3) -> Mat4 {
        Mat4::from_translation((self.pos - anchor).as_vec3()) * self.rot
    }

    /// The pose of an object hanging on attachment point `attach` of this pose, turned by
    /// its own heading/pitch/bank `own` (D3DX order: the point's rotation, then the object's
    /// own, then the parent's), as Omsi.exe puts it there.
    pub fn attached(&self, attach: &Mat4, own: [f64; 3]) -> Pose {
        map_index::Pose { pos: self.pos, rot: self.rot }.attached(attach, own).into()
    }
}

impl From<map_index::Pose> for Pose {
    fn from(p: map_index::Pose) -> Pose {
        Pose { pos: p.pos, rot: p.rot }
    }
}

/// One object of a spline attachment row.
#[derive(Debug, Clone, Copy)]
pub struct RowObject {
    /// Index of the object in its row.
    pub index: usize,
    pub pose: Pose,
}

/// The objects of row record `att` of a tile whose splines are `splines` (curves starting at
/// `origin`, the tile's corner), where the game lays them (`omsi-world`'s `tile_row_objects`):
/// along the record's own spline and the ones after it in the tile, with `index` - the whole
/// map's chains (see [`row_index`]) - for where a chain goes on beyond the tile. A record whose
/// first object would lie before its own spline places none, as Omsi.exe draws none of it.
pub fn row_objects(att: &SplineAttachment, splines: &[MapSpline], origin: DVec2, index: &MapIndex) -> Vec<RowObject> {
    if att.repeater.is_none() && map_index::row_starts_before_its_spline(att, splines) {
        return Vec::new();
    }
    map_index::tile_row_objects(att, splines, origin, Some(index)).into_iter().map(|(_, o)| RowObject { index: o.index, pose: o.pose.into() }).collect()
}

/// What the rows need of the whole map, as the game's map index holds it: every spline's
/// length and links (a chain read without its own offsets is walked through other tiles, and
/// one that goes on into another tile does not end there), and the first object each repeater
/// carries (the object between two tiles' copies of a row is placed once). Made again when
/// any of that changes (looked over, which is far quicker than making it).
pub fn row_index(doc: &Document) -> Arc<MapIndex> {
    static LAST: Mutex<Option<(u64, Arc<MapIndex>)>> = Mutex::new(None);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for t in doc.tiles.values() {
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            (s.id, s.prev_id, s.next_id, s.length.to_bits(), s.map_chain_offset.map(f64::to_bits)).hash(&mut h);
        }
        for a in t.tile.spline_attachments.iter().filter(|a| a.repeater.is_some()) {
            (a.id, a.spline_index, a.repeater, a.offset[2].to_bits(), a.interval.to_bits()).hash(&mut h);
        }
    }
    let key = h.finish();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, ix)) = last.as_ref().filter(|(k, _)| *k == key) {
        return ix.clone();
    }
    let ix = Arc::new(build_row_index(doc));
    *last = Some((key, ix.clone()));
    ix
}

fn build_row_index(doc: &Document) -> MapIndex {
    let mut ix = MapIndex::default();
    for t in doc.tiles.values() {
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            ix.splines.insert(s.id, IndexedSpline { length: s.length, map_chain_offset: s.map_chain_offset, prev: s.prev_id, next: s.next_id });
        }
    }
    // (the map's count of the first object, or the record's own copy of the row where that
    // says an earlier one: `MapIndex::build`)
    for t in doc.tiles.values() {
        for a in &t.tile.spline_attachments {
            let Some((_, first)) = a.repeater else { continue };
            let Some(s) = t.tile.splines.get(a.spline_index.max(0) as usize) else { continue };
            ix.repeater_first.insert((a.id, s.id), first);
            let Some(local) = ix.splines.get(&s.id).copied().filter(|_| a.interval > 0.0) else { continue };
            let d0 = a.offset[2] - chain_offset(&ix, s.id, local);
            let ours = ((JOINT_TOLERANCE - d0) / a.interval).ceil().max(0.0) as usize;
            ix.repeater_first.insert((a.id, s.id), ours.min(first));
        }
    }
    ix
}

/// How far spline `s` starts from the start of its chain, where a row on it counts its
/// distance from: its own number, else (a tile older than version 11) the walk back along the
/// map's links that the game makes.
pub fn chain_offset_of(doc: &Document, s: &MapSpline) -> f64 {
    s.map_chain_offset.unwrap_or_else(|| walked_chain_offset(&row_index(doc), s))
}

/// The same walk with the map's index at hand: the lengths before `s` along the links.
pub fn walked_chain_offset(ix: &MapIndex, s: &MapSpline) -> f64 {
    chain_offset(ix, s.id, IndexedSpline { length: s.length, map_chain_offset: None, prev: s.prev_id, next: s.next_id })
}

/// One object standing on a joint belongs to the spline before (`omsi-world`'s).
const JOINT_TOLERANCE: f64 = 0.01;

/// How far spline `id` starts from the start of its chain: its own number, else the lengths
/// before it along the links (`omsi-world`'s `chain_offset_from`).
fn chain_offset(ix: &MapIndex, id: i64, mut spline: IndexedSpline) -> f64 {
    if let Some(offset) = spline.map_chain_offset {
        return offset;
    }
    let (mut cur, mut forward, mut acc) = (id, true, 0.0);
    let mut seen = vec![id];
    loop {
        let prev_id = if forward { spline.prev } else { spline.next };
        let Some(p) = ix.splines.get(&prev_id) else { break };
        if seen.contains(&prev_id) || seen.len() > 500 {
            break;
        }
        if p.next == cur {
            forward = true;
        } else if p.prev == cur {
            forward = false;
        }
        acc += p.length;
        seen.push(prev_id);
        cur = prev_id;
        spline = *p;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{tile_origin, tile_size};
    use crate::selection::tests::{add_spline, flat_map};

    fn row(id: i64, spline_index: i32, d: f64, interval: f64, range: f64, repeater: Option<(usize, usize)>) -> SplineAttachment {
        SplineAttachment { file: "Sceneryobjects\\lamp.sco".into(), id, spline_index, offset: [0.0, 0.0, d], interval, range, repeater, ..Default::default() }
    }

    /// Every object the rows of the map place (world x, y).
    fn placed(doc: &Document) -> Vec<DVec2> {
        let rows = row_index(doc);
        let mut v = Vec::new();
        for (k, t) in &doc.tiles {
            for a in &t.tile.spline_attachments {
                v.extend(row_objects(a, &t.tile.splines, tile_origin(*k), &rows).iter().map(|o| o.pose.pos.truncate()));
            }
        }
        v
    }

    /// A road running east from tile (0, 0) into tile (1, 0) with a row on it every 100 m, the
    /// next tile's spline `chain` along the chain and its copy of the row (a repeater) laid
    /// from `repeater_d`, the map counting object `first` as the first on it.
    fn border_row(chain: f64, master_d: f64, repeater_d: f64, first: usize) -> Document {
        let mut doc = flat_map(2);
        let ts = tile_size();
        add_spline(&mut doc, (0, 0), 1, 0.0, 150.0, 90.0, ts, 0, 2);
        add_spline(&mut doc, (1, 0), 2, 0.0, 150.0, 90.0, ts, 1, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].map_chain_offset = Some(0.0);
        doc.tiles.get_mut(&(1, 0)).unwrap().tile.splines[0].map_chain_offset = Some(chain);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(row(9, 0, master_d, 100.0, 500.0, None));
        doc.tiles.get_mut(&(1, 0)).unwrap().tile.spline_attachments.push(row(9, 0, repeater_d, 100.0, 500.0, Some((0, first))));
        doc
    }

    /// The row's object at a tile border stands there once, whichever side of the joint the
    /// map's rounding of the chain puts it, and also when neither tile's copy of the row reaches
    /// it by itself (the next copy starts with the object after it), as openOMSI places it.
    #[test]
    fn a_row_object_on_a_tile_border_is_placed_once() {
        let ts = tile_size();
        for (chain, master_d, repeater_d, first) in [(ts + 0.0004, 0.0, 0.0, 4), (ts - 0.0004, 0.0, 0.0, 3), (ts, 0.5, 0.0, 4)] {
            let got = placed(&border_row(chain, master_d, repeater_d, first));
            let at_border = got.iter().filter(|p| (p.x - ts).abs() < 0.6).count();
            assert_eq!(at_border, 1, "chain {chain}, rows from {master_d} and {repeater_d}: {got:?}");
            assert_eq!(got.len(), 6, "every 100 m from 0 to 500: {got:?}");
        }
    }

    /// A tile written without the chain's distances (older than version 11) has a row on a
    /// spline going on from one in the tile before counted from the start of the chain there,
    /// as the game walks it; a row the game refuses (its first object before its own spline)
    /// places nothing.
    #[test]
    fn a_row_counts_along_its_chain_through_the_tiles_and_one_before_its_spline_places_nothing() {
        let mut doc = flat_map(2);
        let ts = tile_size();
        add_spline(&mut doc, (0, 0), 1, 0.0, 150.0, 90.0, ts, 0, 2);
        add_spline(&mut doc, (1, 0), 2, 0.0, 150.0, 90.0, 100.0, 1, 3);
        add_spline(&mut doc, (1, 0), 3, 100.0, 150.0, 90.0, 100.0, 2, 0);
        doc.tiles.get_mut(&(1, 0)).unwrap().tile.spline_attachments.push(row(9, 0, ts + 10.0, 0.0, 0.0, None));
        let got = placed(&doc);
        assert_eq!(got.len(), 1);
        assert!((got[0] - DVec2::new(ts + 10.0, 150.0)).length() < 1e-6, "{got:?}");
        // (on spline 3, 100 m into its tile's chain, at 50 m: 50 m before it)
        doc.tiles.get_mut(&(1, 0)).unwrap().tile.spline_attachments[0] = row(9, 1, 50.0, 0.0, 0.0, None);
        assert!(placed(&doc).is_empty());
    }
}
