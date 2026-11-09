//! Undo and redo: every edit keeps the tiles it changed (and the global settings, the
//! timetable) as they were before it, and stepping back swaps them in again.

use std::collections::{BTreeMap, BTreeSet};

use omsi_map::GlobalCfg;

use crate::doc::{next_rev, Document, TileDoc, TileKey};

#[derive(Clone)]
struct Step {
    label: String,
    /// Each tile before the edit (None: it did not exist).
    tiles: BTreeMap<TileKey, Option<TileDoc>>,
    global: Option<(GlobalCfg, Vec<TileKey>)>,
    timetable: Option<crate::timetable::Timetable>,
    traffic: Option<crate::traffic::Traffic>,
    descriptions: Option<crate::descriptions::Descriptions>,
    groups: Option<Vec<crate::mapgroups::MapGroup>>,
    /// When it was made (edits of one kind in quick succession are one step).
    at: std::time::Instant,
}

/// What a window's edit changed, as it was before (see `History::push_snapshot`).
#[derive(Default)]
pub struct Snapshot {
    pub global: Option<(GlobalCfg, Vec<TileKey>)>,
    pub timetable: Option<crate::timetable::Timetable>,
    pub traffic: Option<crate::traffic::Traffic>,
    pub descriptions: Option<crate::descriptions::Descriptions>,
    pub groups: Option<Vec<crate::mapgroups::MapGroup>>,
}

/// Snapshots of one kind closer together than this make one step (typing a name, dragging
/// a value).
const MERGE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The parts of the map the kept edits, undos and redos changed (multiplayer passes them on).
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Changed {
    pub tiles: BTreeSet<TileKey>,
    pub global: bool,
    pub timetable: bool,
    pub traffic: bool,
    pub descriptions: bool,
    pub groups: bool,
}

impl Changed {
    pub fn is_empty(&self) -> bool {
        *self == Changed::default()
    }

    fn add(&mut self, s: &Step) {
        self.tiles.extend(s.tiles.keys().copied());
        self.global |= s.global.is_some();
        self.timetable |= s.timetable.is_some();
        self.traffic |= s.traffic.is_some();
        self.descriptions |= s.descriptions.is_some();
        self.groups |= s.groups.is_some();
    }

    /// Whether the step changed any of these.
    fn meets(&self, s: &Step) -> bool {
        s.tiles.keys().any(|k| self.tiles.contains(k)) || (self.global && s.global.is_some()) || (self.timetable && s.timetable.is_some()) || (self.traffic && s.traffic.is_some()) || (self.descriptions && s.descriptions.is_some()) || (self.groups && s.groups.is_some())
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    open: Option<Step>,
    /// How many steps were ever kept (for `mark` / `merge_since`).
    kept: u64,
    /// What changed since `take_changed`.
    changed: Changed,
}

const LIMIT: usize = 100;

impl History {
    /// Start an edit (ends one still open).
    pub fn begin(&mut self, label: &str) {
        self.commit();
        self.open = Some(Step { label: label.to_string(), tiles: BTreeMap::new(), global: None, timetable: None, traffic: None, descriptions: None, groups: None, at: std::time::Instant::now() });
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Keep a tile as it is now, before the open edit changes it (once per edit).
    pub fn touch_tile(&mut self, doc: &Document, k: TileKey) {
        if let Some(s) = &mut self.open {
            s.tiles.entry(k).or_insert_with(|| doc.tiles.get(&k).cloned());
        }
    }

    /// Keep the global settings and the tile list.
    pub fn touch_global(&mut self, doc: &Document) {
        if let Some(s) = &mut self.open {
            if s.global.is_none() {
                s.global = Some((doc.global.clone(), doc.order.clone()));
            }
        }
    }

    /// Drop the open edit unkept (it changed nothing).
    pub fn cancel(&mut self) {
        self.open = None;
    }

    /// Drop the open edit and put back what it touched, as it was when the edit began (for a
    /// tool that found out half-way that it cannot finish).
    pub fn rollback(&mut self, doc: &mut Document) {
        if let Some(s) = self.open.take() {
            self.changed.add(&s);
            let _ = apply(doc, s);
        }
    }

    /// Close the open edit (nothing kept: it is dropped).
    pub fn commit(&mut self) {
        if let Some(s) = self.open.take() {
            if s.tiles.is_empty() && s.global.is_none() && s.timetable.is_none() && s.traffic.is_none() && s.descriptions.is_none() && s.groups.is_none() {
                return;
            }
            self.changed.add(&s);
            self.undo.push(s);
            self.kept += 1;
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// Where the history stands now, for `merge_since`.
    pub fn mark(&mut self) -> u64 {
        self.commit();
        self.kept
    }

    /// The steps kept since `mark` made one, under `label` (a run of edits that belong
    /// together: undone at once). Each thing is kept as it was before the first of them.
    pub fn merge_since(&mut self, mark: u64, label: &str) {
        self.commit();
        let n = (self.kept.saturating_sub(mark) as usize).min(self.undo.len());
        if n < 2 {
            if let (1, Some(last)) = (n, self.undo.last_mut()) {
                last.label = label.to_string();
            }
            return;
        }
        let steps: Vec<Step> = self.undo.split_off(self.undo.len() - n);
        let mut merged = Step { label: label.to_string(), tiles: BTreeMap::new(), global: None, timetable: None, traffic: None, descriptions: None, groups: None, at: std::time::Instant::now() };
        for s in steps {
            for (k, t) in s.tiles {
                merged.tiles.entry(k).or_insert(t);
            }
            merged.global = merged.global.or(s.global);
            merged.timetable = merged.timetable.or(s.timetable);
            merged.traffic = merged.traffic.or(s.traffic);
            merged.descriptions = merged.descriptions.or(s.descriptions);
            merged.groups = merged.groups.or(s.groups);
        }
        self.undo.push(merged);
        // (they count as one step now: a mark taken before them, for a merge round this one,
        // must not reach past them into the steps before)
        self.kept -= n as u64 - 1;
    }

    /// A finished edit of a window, given as the state before it: one undo step, or part of
    /// the last one when that was of the same kind a moment ago.
    pub fn push_snapshot(&mut self, label: &str, before: Snapshot) {
        self.commit();
        self.changed.global |= before.global.is_some();
        self.changed.timetable |= before.timetable.is_some();
        self.changed.traffic |= before.traffic.is_some();
        self.changed.descriptions |= before.descriptions.is_some();
        self.changed.groups |= before.groups.is_some();
        if let Some(last) = self.undo.last_mut() {
            let same = last.label == label && last.tiles.is_empty() && last.global.is_some() == before.global.is_some() && last.timetable.is_some() == before.timetable.is_some() && last.traffic.is_some() == before.traffic.is_some() && last.descriptions.is_some() == before.descriptions.is_some() && last.groups.is_some() == before.groups.is_some();
            if same && self.redo.is_empty() && last.at.elapsed() < MERGE {
                last.at = std::time::Instant::now();
                return;
            }
        }
        if before.global.is_none() && before.timetable.is_none() && before.traffic.is_none() && before.descriptions.is_none() && before.groups.is_none() {
            return;
        }
        self.undo.push(Step { label: label.to_string(), tiles: BTreeMap::new(), global: before.global, timetable: before.timetable, traffic: before.traffic, descriptions: before.descriptions, groups: before.groups, at: std::time::Instant::now() });
        self.kept += 1;
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.open.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The steps undo goes back through (the latest last), and those redo brings again (the
    /// next first).
    pub fn steps(&self) -> (Vec<&str>, Vec<&str>) {
        let mut undo: Vec<&str> = self.undo.iter().map(|s| s.label.as_str()).collect();
        if let Some(o) = &self.open {
            undo.push(o.label.as_str());
        }
        (undo, self.redo.iter().rev().map(|s| s.label.as_str()).collect())
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn undo(&mut self, doc: &mut Document) {
        self.commit();
        if let Some(s) = self.undo.pop() {
            self.changed.add(&s);
            let back = apply(doc, s);
            self.redo.push(back);
        }
    }

    pub fn redo(&mut self, doc: &mut Document) {
        self.commit();
        if let Some(s) = self.redo.pop() {
            self.changed.add(&s);
            let back = apply(doc, s);
            self.undo.push(back);
        }
    }

    /// The object files the tiles kept for undo and redo name (what a step can bring back).
    pub fn object_files(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().chain(&self.redo).chain(&self.open).flat_map(|s| s.tiles.values().flatten()).flat_map(|t| t.tile.objects.iter().map(|o| o.file.as_str()))
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open = None;
        self.changed = Changed::default();
    }

    /// What the kept edits, undos and redos changed since the last call.
    pub fn take_changed(&mut self) -> Changed {
        std::mem::take(&mut self.changed)
    }

    /// Forget every step that changed any of these (somebody else changed them: undoing such a
    /// step would put back what was there before their edit). The edit still open goes too.
    pub fn forget(&mut self, parts: &Changed) {
        if parts.is_empty() {
            return;
        }
        if self.open.as_ref().map(|s| parts.meets(s)).unwrap_or(false) {
            self.open = None;
        }
        self.undo.retain(|s| !parts.meets(s));
        self.redo.retain(|s| !parts.meets(s));
    }
}

/// Put a step's state into the document; returns the state it replaced (the opposite step).
fn apply(doc: &mut Document, s: Step) -> Step {
    let mut back = Step { label: s.label.clone(), tiles: BTreeMap::new(), global: None, timetable: None, traffic: None, descriptions: None, groups: None, at: s.at };
    for (k, t) in s.tiles {
        let now = match t {
            Some(mut t) => {
                // new revisions: whatever shows the tile builds it again
                t.terrain_rev = next_rev();
                t.mask_rev = next_rev();
                t.content_rev = next_rev();
                t.dirty = true;
                doc.tiles.insert(k, t)
            }
            None => doc.tiles.remove(&k),
        };
        back.tiles.insert(k, now);
    }
    if let Some((g, order)) = s.global {
        // (IDCodes given out stay given out: junction files are named by them)
        let next = doc.global.next_id_code.max(g.next_id_code);
        back.global = Some((std::mem::replace(&mut doc.global, g), std::mem::replace(&mut doc.order, order)));
        doc.global.next_id_code = next;
        doc.global_rev = next_rev();
    }
    if let Some(t) = s.timetable {
        let old = std::mem::replace(&mut doc.timetable, t);
        doc.timetable.restored(&old);
        back.timetable = Some(old);
    }
    if let Some(t) = s.traffic {
        back.traffic = Some(std::mem::replace(&mut doc.traffic, t));
        doc.traffic.restored();
    }
    if let Some(d) = s.descriptions {
        let old = std::mem::replace(&mut doc.descriptions, d);
        doc.descriptions.restored(&old);
        back.descriptions = Some(old);
    }
    if let Some(g) = s.groups {
        back.groups = Some(std::mem::replace(&mut doc.groups, g));
    }
    doc.global_dirty = true;
    back
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IDCodes handed out stay used after an undo: a junction made again would else get the
    /// number of the one undone and write over its files (which a saved map may still name).
    #[test]
    fn undo_does_not_hand_out_an_idcode_again() {
        let mut doc = crate::selection::tests::flat_map(1);
        let mut h = History::default();
        h.begin("t");
        h.touch_tile(&doc, (0, 0));
        h.touch_global(&doc);
        let first = doc.new_id();
        h.commit();
        h.undo(&mut doc);
        assert!(doc.new_id() > first);
        h.redo(&mut doc);
        assert!(doc.new_id() > first + 1);
    }

    #[test]
    fn what_changed_is_told_once() {
        let mut doc = crate::selection::tests::flat_map(2);
        let mut h = History::default();
        h.begin("e");
        h.touch_tile(&doc, (1, 0));
        doc.tiles.get_mut(&(1, 0)).unwrap().terrain.heights[0] = 5.0;
        // (an edit still open is not told)
        assert!(h.take_changed().is_empty());
        h.commit();
        let c = h.take_changed();
        assert_eq!(c.tiles.iter().copied().collect::<Vec<_>>(), vec![(1, 0)]);
        assert!(!c.global && h.take_changed().is_empty());
        h.undo(&mut doc);
        assert_eq!(h.take_changed().tiles.len(), 1);
        h.push_snapshot("g", Snapshot { groups: Some(Vec::new()), ..Default::default() });
        assert!(h.take_changed().groups);
    }

    #[test]
    fn steps_on_tiles_somebody_else_changed_are_forgotten() {
        let mut doc = crate::selection::tests::flat_map(2);
        let mut h = History::default();
        for k in [(0, 0), (1, 0), (0, 0)] {
            h.begin("e");
            h.touch_tile(&doc, k);
            doc.tiles.get_mut(&k).unwrap().terrain.heights[0] += 1.0;
            h.commit();
        }
        h.forget(&Changed { tiles: [(0, 0)].into(), ..Default::default() });
        // (the edit of tile 1, 0 alone stays)
        assert_eq!(h.steps().0.len(), 1);
        h.undo(&mut doc);
        assert_eq!(doc.tiles[&(1, 0)].terrain.heights[0], 0.0);
        assert_eq!(doc.tiles[&(0, 0)].terrain.heights[0], 2.0);
    }

    #[test]
    fn edits_merged_are_undone_at_once() {
        let mut doc = crate::selection::tests::flat_map(2);
        let mut h = History::default();
        let height = |doc: &Document, k: TileKey| doc.tiles[&k].terrain.heights[0];
        h.begin("before");
        h.touch_tile(&doc, (0, 0));
        doc.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = 1.0;
        h.commit();
        let mark = h.mark();
        for (k, v) in [((0, 0), 2.0), ((1, 0), 3.0), ((0, 0), 4.0)] {
            h.begin("part");
            h.touch_tile(&doc, k);
            doc.tiles.get_mut(&k).unwrap().terrain.heights[0] = v;
            h.commit();
        }
        h.merge_since(mark, "all");
        assert_eq!(h.steps().0.len(), 2);
        h.undo(&mut doc);
        // (each tile as it was before the first of them)
        assert_eq!((height(&doc, (0, 0)), height(&doc, (1, 0))), (1.0, 0.0));
        h.redo(&mut doc);
        assert_eq!((height(&doc, (0, 0)), height(&doc, (1, 0))), (4.0, 3.0));
        // a merge inside a merge: the outer one takes in only what came after its mark
        let outer = h.mark();
        for _ in 0..2 {
            let inner = h.mark();
            for v in [5.0, 6.0, 7.0] {
                h.begin("inner part");
                h.touch_tile(&doc, (0, 0));
                doc.tiles.get_mut(&(0, 0)).unwrap().terrain.heights[0] = v;
                h.commit();
            }
            h.merge_since(inner, "inner");
        }
        h.merge_since(outer, "outer");
        assert_eq!(h.steps().0, vec!["before", "all", "outer"]);
        h.undo(&mut doc);
        assert_eq!(height(&doc, (0, 0)), 4.0, "the outer step undone, the one before it kept");
    }
}
