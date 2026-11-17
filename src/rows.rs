//! Objects along a road: lamps, trees, bollards, posts put down every so many metres beside
//! a road or a chain of them, as OMSI's own spline attachments (`[splineAttachement]` rows),
//! so they follow the road and stay editable as a row.

use omsi_map::SplineAttachment;

use crate::doc::{Document, Sel, TileKey};

/// How a row is laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowParams {
    /// Between two objects (m).
    pub interval: f64,
    /// Beside the road's centre line, right of the way it is laid (m).
    pub offset: f64,
    /// A second row on the other side, turned round.
    pub both: bool,
    /// Kept free at the start and the end (m).
    pub margin_start: f64,
    pub margin_end: f64,
    /// Above the road (m).
    pub height: f64,
    /// The objects' own heading against the road's (degrees).
    pub turn: f64,
    /// Lean with the road's gradient and cant.
    pub tilt: bool,
}

impl Default for RowParams {
    fn default() -> Self {
        RowParams { interval: 25.0, offset: 6.0, both: false, margin_start: 5.0, margin_end: 5.0, height: 0.0, turn: 0.0, tilt: false }
    }
}

/// One spline of a chain in the order it is walked: whether it is walked against its own
/// direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub key: TileKey,
    pub id: i64,
    pub reversed: bool,
    pub length: f64,
}

/// The selected splines as chains (each in the order its links run, from one end); splines
/// not linked to the others make chains of their own.
pub fn chains(doc: &Document, sels: &[Sel]) -> Vec<Vec<Step>> {
    let set: Vec<(TileKey, i64)> = sels.iter().filter_map(|s| if let Sel::Spline(k, id) = s { doc.spline(*k, *id).map(|_| (*k, *id)) } else { None }).collect();
    let ids: Vec<i64> = set.iter().map(|s| s.1).collect();
    let inside = |id: i64| id != 0 && ids.contains(&id);
    let mut seen: Vec<i64> = Vec::new();
    let mut out = Vec::new();
    // (from a spline with no selected neighbour on one side first; a ring from anywhere)
    // (and rather from a chain's start, so it is walked the way it is laid)
    let mut starts: Vec<(TileKey, i64)> = set.iter().copied().filter(|(k, id)| doc.spline(*k, *id).map(|s| !inside(s.prev_id)).unwrap_or(false)).collect();
    starts.extend(set.iter().copied().filter(|(k, id)| doc.spline(*k, *id).map(|s| !inside(s.next_id)).unwrap_or(false)));
    starts.extend(set.iter().copied());
    for (k, id) in starts {
        if seen.contains(&id) {
            continue;
        }
        let s = doc.spline(k, id).unwrap();
        // walked from the end that has nothing selected beyond it
        let reversed = inside(s.prev_id) && !inside(s.next_id);
        let mut chain = vec![Step { key: k, id, reversed, length: s.length }];
        seen.push(id);
        let mut cur = (id, reversed);
        loop {
            let c = doc.find_spline(cur.0).unwrap().1;
            let next = if cur.1 { c.prev_id } else { c.next_id };
            if !inside(next) || seen.contains(&next) {
                break;
            }
            let Some((nk, n)) = doc.find_spline(next) else { break };
            let rev = n.next_id == cur.0 && n.prev_id != cur.0;
            chain.push(Step { key: nk, id: n.id, reversed: rev, length: n.length });
            seen.push(n.id);
            cur = (n.id, rev);
        }
        out.push(chain);
    }
    out
}

/// The rows for objects every `p.interval` along a chain: (tile, spline, the row; its
/// `spline_index` and `id` are the caller's).
pub fn plan(doc: &Document, chain: &[Step], file: &str, p: &RowParams) -> Vec<(TileKey, i64, SplineAttachment)> {
    let total: f64 = chain.iter().map(|s| s.length).sum();
    let interval = p.interval.max(0.5);
    let last = total - p.margin_end.max(0.0);
    let first = p.margin_start.max(0.0);
    let mut out = Vec::new();
    if last < first {
        return out;
    }
    let mut a = 0.0;
    for (i, st) in chain.iter().enumerate() {
        let b = a + st.length;
        let end_here = i + 1 == chain.len();
        // the places along the chain that fall on this spline
        let j0 = ((a - first) / interval - 1e-9).ceil().max(0.0) as i64;
        let mut local: Vec<f64> = Vec::new();
        let mut j = j0;
        loop {
            let g = first + j as f64 * interval;
            if g > last + 1e-9 || g > b + 1e-9 || (!end_here && g >= b - 1e-9) {
                break;
            }
            if g >= a - 1e-9 {
                local.push(if st.reversed { b - g } else { g - a }.clamp(0.0, st.length));
            }
            j += 1;
        }
        a = b;
        if local.is_empty() {
            continue;
        }
        local.sort_by(|x, y| x.total_cmp(y));
        let Some(s) = doc.spline(st.key, st.id) else { continue };
        let chain_offset = crate::place::chain_offset_of(doc, s);
        let range = (local.len() - 1) as f64 * interval;
        // (right of the way walked is left of a spline walked backwards)
        let (side, turn) = if st.reversed { (-p.offset, p.turn + 180.0) } else { (p.offset, p.turn) };
        let row = |side: f64, turn: f64| SplineAttachment {
            file: file.to_string(),
            spline_index: -1,
            offset: [side, p.height, chain_offset + local[0]],
            rot: [crate::roads::norm_deg(turn), 0.0, 0.0],
            interval,
            range,
            tilt: p.tilt,
            ..Default::default()
        };
        out.push((st.key, st.id, row(side, turn)));
        if p.both && p.offset.abs() > 1e-6 {
            out.push((st.key, st.id, row(-side, turn + 180.0)));
        }
    }
    out
}

/// Put planned rows on the map with new IDCodes. Returns how many rows were added.
pub fn add(doc: &mut Document, rows: Vec<(TileKey, i64, SplineAttachment)>) -> usize {
    let mut n = 0;
    for (k, id, mut r) in rows {
        let new = doc.new_id();
        let Some(t) = doc.tiles.get_mut(&k) else { continue };
        let Some(idx) = t.tile.splines.iter().position(|s| s.id == id && !s.deleted) else { continue };
        r.id = new;
        r.spline_index = idx as i32;
        t.tile.spline_attachments.push(r);
        t.touch_content();
        n += 1;
    }
    n
}

/// The rows on a spline: their places in the tile's list.
pub fn rows_on(doc: &Document, k: TileKey, id: i64) -> Vec<usize> {
    let Some(t) = doc.tiles.get(&k) else { return Vec::new() };
    let Some(idx) = t.tile.splines.iter().position(|s| s.id == id && !s.deleted) else { return Vec::new() };
    t.tile.spline_attachments.iter().enumerate().filter(|(_, a)| a.spline_index == idx as i32).map(|(i, _)| i).collect()
}

/// Take a row away, with whatever hangs on its objects.
pub fn remove(doc: &mut Document, k: TileKey, row: usize) {
    let Some(t) = doc.tiles.get_mut(&k) else { return };
    if row >= t.tile.spline_attachments.len() {
        return;
    }
    let a = t.tile.spline_attachments.remove(row);
    let mut gone = vec![a.id];
    let mut i = 0;
    while i < gone.len() {
        let p = gone[i];
        gone.extend(t.tile.attach_objects.iter().filter(|o| o.parent_id == Some(p)).map(|o| o.id));
        i += 1;
    }
    t.tile.attach_objects.retain(|o| !gone.contains(&o.id));
    t.touch_content();
}

/// A new interval for a row: it reaches as far as it did.
pub fn set_interval(a: &mut SplineAttachment, interval: f64) {
    let interval = interval.max(0.5);
    let reach = a.range.max(0.0);
    a.range = (reach / interval + 1e-6).floor() * interval;
    a.interval = interval;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::tile_origin;
    use crate::selection::tests::{add_spline, flat_map};
    use glam::DVec2;

    /// Where the objects of every row on the map stand (world, x and y).
    fn placed(doc: &Document) -> Vec<DVec2> {
        let mut v = Vec::new();
        let rows = crate::place::row_index(doc);
        for (k, t) in &doc.tiles {
            for a in &t.tile.spline_attachments {
                v.extend(crate::place::row_objects(a, &t.tile.splines, tile_origin(*k), &rows).iter().map(|o| o.pose.pos.truncate()));
            }
        }
        v.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        v
    }

    #[test]
    fn a_row_runs_on_evenly_along_a_chain_walked_either_way() {
        let mut doc = flat_map(1);
        // north from (100, 0): 1 forwards, 2 laid backwards (it ends where 1 ends), 3 forwards
        add_spline(&mut doc, (0, 0), 1, 100.0, 0.0, 0.0, 30.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 70.0, 180.0, 40.0, 3, 1);
        add_spline(&mut doc, (0, 0), 3, 100.0, 70.0, 0.0, 30.0, 2, 0);
        let sels = [Sel::Spline((0, 0), 3), Sel::Spline((0, 0), 1), Sel::Spline((0, 0), 2)];
        let ch = chains(&doc, &sels);
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].iter().map(|s| (s.id, s.reversed)).collect::<Vec<_>>(), vec![(1, false), (2, true), (3, false)]);
        let p = RowParams { interval: 10.0, offset: 5.0, margin_start: 5.0, margin_end: 5.0, ..Default::default() };
        let rows = plan(&doc, &ch[0], "lamp.sco", &p);
        assert_eq!(add(&mut doc, rows), 3);
        let got = placed(&doc);
        // every 10 m from 5 to 95, all 5 m to the east (right of the way north)
        assert_eq!(got.len(), 10, "{got:?}");
        for (i, q) in got.iter().enumerate() {
            assert!((q.y - (5.0 + 10.0 * i as f64)).abs() < 1e-6 && (q.x - 105.0).abs() < 1e-6, "{i}: {q:?}");
        }
    }

    #[test]
    fn both_sides_and_removing_a_row() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 0.0, 0.0, 40.0, 0, 0);
        let ch = chains(&doc, &[Sel::Spline((0, 0), 1)]);
        let p = RowParams { interval: 20.0, offset: 4.0, both: true, margin_start: 0.0, margin_end: 0.0, ..Default::default() };
        let rows = plan(&doc, &ch[0], "tree.sco", &p);
        add(&mut doc, rows);
        let got = placed(&doc);
        assert_eq!(got.len(), 6);
        assert!(got.iter().filter(|q| (q.x - 96.0).abs() < 1e-6).count() == 3);
        let rows = rows_on(&doc, (0, 0), 1);
        assert_eq!(rows.len(), 2);
        remove(&mut doc, (0, 0), rows[1]);
        assert_eq!(placed(&doc).len(), 3);
    }

    #[test]
    fn a_new_interval_keeps_the_reach() {
        let mut a = SplineAttachment { interval: 10.0, range: 40.0, ..Default::default() };
        set_interval(&mut a, 15.0);
        assert_eq!((a.interval, a.range), (15.0, 30.0));
    }
}
