//! Walking along a road piece by piece, as OMSI 2's editor's next / previous spline: the
//! piece joined at the selected one's end (or start) is selected and the camera goes there,
//! so a long road is checked or changed one piece after another without hunting for it.

use crate::doc::{Sel, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;

impl Editor {
    /// The piece linked at the end (`forward`) or the start of the selected road piece; with
    /// `add` it joins the selection, else it is selected alone. Where the camera should go
    /// then is in `look_request`. Whether there was one.
    pub fn select_linked(&mut self, forward: bool, add: bool) -> bool {
        let Some(doc) = self.doc.as_ref() else { return false };
        // (from the piece in the properties; with several selected, from the last one taken)
        let from = match self.selected {
            Some(Sel::Spline(k, id)) => Some((k, id)),
            _ => self.selection().into_iter().rev().find_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }),
        };
        let Some((k, id)) = from else {
            self.toast(tr("Select a road piece first", "Сначала выберите кусок дороги"));
            return false;
        };
        let Some(next) = linked(doc, k, id, forward) else {
            self.toast(if forward { tr("Nothing is joined at its end", "К его концу ничего не присоединено") } else { tr("Nothing is joined at its start", "К его началу ничего не присоединено") });
            return false;
        };
        let at = doc.spline(next.0, next.1).map(|s| omsi_geometry::SplineCurve::from_map(s, crate::doc::tile_origin(next.0)).point_at(s.length * 0.5).truncate());
        let sel = Sel::Spline(next.0, next.1);
        let mut v = if add { self.selection() } else { Vec::new() };
        v.retain(|s| *s != sel);
        v.push(sel);
        self.set_selection(v);
        self.selected = Some(sel);
        self.look_request = at;
        true
    }
}

/// The piece joined at the end (`forward`) or the start of piece `id` on tile `k`.
pub fn linked(doc: &crate::doc::Document, k: TileKey, id: i64, forward: bool) -> Option<(TileKey, i64)> {
    let s = doc.spline(k, id)?;
    let other = if forward { s.next_id } else { s.prev_id };
    if other == 0 || other == id {
        return None;
    }
    doc.find_spline(other).map(|(k2, _)| (k2, other))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_map::tile::MapSpline;

    fn piece(id: i64, prev: i64, next: i64, x: f64) -> MapSpline {
        MapSpline { file: "Splines\\road.sli".into(), id, prev_id: prev, next_id: next, pos: [x, 10.0, 0.0], heading: 90.0, length: 20.0, ..Default::default() }
    }

    /// Next and previous follow the links, a piece's own end left free gives none.
    #[test]
    fn steps_along_the_links() {
        let mut doc = crate::selection::tests::flat_map(1);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.splines.extend([piece(1, 0, 2, 0.0), piece(2, 1, 3, 20.0), piece(3, 2, 0, 40.0)]);
        assert_eq!(linked(&doc, (0, 0), 1, true), Some(((0, 0), 2)));
        assert_eq!(linked(&doc, (0, 0), 2, false), Some(((0, 0), 1)));
        assert_eq!(linked(&doc, (0, 0), 3, true), None);
        assert_eq!(linked(&doc, (0, 0), 1, false), None);
    }

    #[test]
    fn selects_and_adds() {
        let mut ed = Editor::default();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.extend([piece(1, 0, 2, 0.0), piece(2, 1, 3, 20.0), piece(3, 2, 0, 40.0)]);
        ed.doc = Some(doc);
        ed.set_selection(vec![Sel::Spline((0, 0), 1)]);
        assert!(ed.select_linked(true, false));
        assert_eq!(ed.selection(), vec![Sel::Spline((0, 0), 2)]);
        assert!(ed.look_request.is_some());
        assert!(ed.select_linked(true, true));
        assert_eq!(ed.selection().len(), 2);
        assert!(!ed.select_linked(true, false));
    }
}
