//! Lining up and spacing objects: the selected objects put at equal steps between the two
//! farthest apart (a row of posts, lamps or trees put straight after placing them by eye), on
//! the straight line between them or each only moved along it, its distance off the line
//! kept.

use glam::DVec2;

use crate::doc::Sel;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::selection::{self, Origin};

/// Where each object goes: the two farthest apart stay, the others follow in their order along
/// the line between them at equal steps; `onto` puts them on the line, else each keeps its
/// distance to the side of it. (object, its move)
pub fn spread(objects: &[Origin], onto: bool) -> Vec<(Sel, DVec2)> {
    let n = objects.len();
    if n < 3 {
        return Vec::new();
    }
    let mut ends = (0, 1, -1.0);
    for i in 0..n {
        for j in i + 1..n {
            let d = objects[i].pos.distance_squared(objects[j].pos);
            if d > ends.2 {
                ends = (i, j, d);
            }
        }
    }
    let (a, b) = (objects[ends.0].pos, objects[ends.1].pos);
    let len = a.distance(b);
    if len < 1e-6 {
        return Vec::new();
    }
    let axis = (b - a) / len;
    let mut order: Vec<(f64, &Origin)> = objects.iter().map(|o| ((o.pos - a).dot(axis), o)).collect();
    order.sort_by(|x, y| x.0.total_cmp(&y.0));
    order
        .iter()
        .enumerate()
        .map(|(i, (t, o))| {
            let want = len * i as f64 / (n - 1) as f64;
            let to = if onto { a + axis * want } else { o.pos + axis * (want - t) };
            (o.sel, to - o.pos)
        })
        .collect()
}

impl Editor {
    /// The selected objects spaced evenly between the two farthest apart, put on the line
    /// between them (`onto`) or moved only along it; one undo step. How many moved.
    pub fn line_up(&mut self, onto: bool) -> usize {
        let sels: Vec<Sel> = self.selection().into_iter().filter(|s| matches!(s, Sel::Object(..))).collect();
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let from: Vec<Origin> = sels.iter().filter_map(|s| selection::origin_of(doc, *s)).collect();
        let moves = spread(&from, onto);
        if moves.is_empty() {
            self.toast(tr("Select three objects or more (the two farthest apart stay)", "Выберите три объекта или больше (два самых дальних останутся на месте)"));
            return 0;
        }
        self.history.begin(if onto { tr("Line up", "Выстроить в линию") } else { tr("Space evenly", "Расставить поровну") });
        let mut now = Vec::new();
        let mut moved = 0;
        for (s, delta) in moves {
            let Some(o) = self.doc.as_ref().and_then(|d| selection::origin_of(d, s)) else { continue };
            if delta.length() < 1e-4 {
                now.push(s);
                continue;
            }
            let arms = self.arms_for(&[s]);
            let doc = self.doc.as_mut().unwrap();
            now.extend(selection::transform(doc, &mut self.history, &[o], DVec2::ZERO, 0.0, delta));
            self.carry(&arms, DVec2::ZERO, 0.0, delta, 0.0);
            moved += 1;
        }
        self.set_selection_unique(now);
        self.finish_step();
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: i64, x: f64, y: f64) -> Origin {
        Origin { sel: Sel::Object((0, 0), id), pos: DVec2::new(x, y), heading: 0.0 }
    }

    /// The ends stay, the others go to equal steps between them in their order.
    #[test]
    fn spaced_between_the_farthest_two() {
        let objs = [at(1, 0.0, 0.0), at(2, 13.0, 2.0), at(3, 30.0, 0.0), at(4, 4.0, -1.0)];
        let on: Vec<DVec2> = objs.iter().map(|o| o.pos + spread(&objs, true).iter().find(|m| m.0 == o.sel).unwrap().1).collect();
        assert!(on[0].distance(DVec2::new(0.0, 0.0)) < 1e-9);
        assert!(on[3].distance(DVec2::new(10.0, 0.0)) < 1e-9);
        assert!(on[1].distance(DVec2::new(20.0, 0.0)) < 1e-9);
        assert!(on[2].distance(DVec2::new(30.0, 0.0)) < 1e-9);
        // along the line only: each keeps its distance to the side
        let along: Vec<DVec2> = objs.iter().map(|o| o.pos + spread(&objs, false).iter().find(|m| m.0 == o.sel).unwrap().1).collect();
        assert!(along[1].distance(DVec2::new(20.0, 2.0)) < 1e-9);
        assert!(along[3].distance(DVec2::new(10.0, -1.0)) < 1e-9);
        assert!(spread(&objs[..2], true).is_empty());
    }

    #[test]
    fn lines_up_selected_objects_in_one_step() {
        let mut ed = Editor::default();
        let mut doc = crate::selection::tests::flat_map(1);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        for (id, x, y) in [(1, 10.0, 10.0), (2, 23.0, 12.0), (3, 40.0, 10.0)] {
            t.tile.objects.push(omsi_map::MapObject { file: "Sceneryobjects\\a.sco".into(), id, pos: [x, y, 0.0], ..Default::default() });
        }
        ed.doc = Some(doc);
        ed.set_selection(vec![Sel::Object((0, 0), 1), Sel::Object((0, 0), 2), Sel::Object((0, 0), 3)]);
        assert_eq!(ed.line_up(true), 1);
        let o = ed.doc.as_ref().unwrap().object((0, 0), 2).unwrap();
        assert!((o.pos[0] - 25.0).abs() < 1e-6 && (o.pos[1] - 10.0).abs() < 1e-6);
        ed.undo();
        let o = ed.doc.as_ref().unwrap().object((0, 0), 2).unwrap();
        assert!((o.pos[0] - 23.0).abs() < 1e-6);
    }
}
