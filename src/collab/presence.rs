//! Where the others work: each editor sends its cursor on the ground, what it has selected
//! and its tool (a few times a second at most, only when it changed); the view draws the
//! others' cursors with their names and their selection in their colours.

use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Stroke, Vec2};
use glam::DVec3;
use openmapper_collab::Presence;

use crate::doc::{tile_origin, Sel};
use crate::editor::Editor;

/// The least time between two presences sent.
const EVERY: Duration = Duration::from_millis(100);
/// The same presence sent again this often (somebody standing still stays drawn).
const AGAIN: Duration = Duration::from_secs(3);
/// How long the others' presence is drawn after it was last heard.
pub const SEEN_FOR: Duration = Duration::from_secs(10);
/// Selected things sent at most (a box over a town selects thousands).
const MAX_SELECTED: usize = 200;

/// This editor's presence now.
pub fn mine(ed: &Editor) -> Presence {
    // (to a tenth of a metre: the cursor's jitter is not worth sending)
    let cursor = ed.cursor.filter(|_| ed.cursor_in_view).map(|c| [(c.x * 10.0).round() / 10.0, (c.y * 10.0).round() / 10.0, (c.z * 10.0).round() / 10.0]);
    let selected = ed
        .selected
        .iter()
        .chain(&ed.also)
        .take(MAX_SELECTED)
        .map(|s| match *s {
            Sel::Object(k, id) | Sel::Spline(k, id) | Sel::Attached(k, id) => (k.0, k.1, id),
        })
        .collect();
    Presence { cursor, selected, tool: ed.tool.label().to_string() }
}

/// Keeps what was sent last: a change goes out at most every `EVERY`, the same every `AGAIN`.
#[derive(Default)]
pub struct Sender {
    last: Option<(Instant, Presence)>,
}

impl Sender {
    /// The presence to send now, if any.
    pub fn next(&mut self, now: Instant, p: Presence) -> Option<Presence> {
        if let Some((at, last)) = &self.last {
            let since = now.duration_since(*at);
            if since < EVERY || (*last == p && since < AGAIN) {
                return None;
            }
        }
        self.last = Some((now, p.clone()));
        Some(p)
    }
}

/// The others' cursors and selections over the view.
pub fn paint(ctx: &egui::Context, c: &super::Collab, ed: &Editor, cam: &crate::camera::Camera, view: egui::Rect) {
    let (Some(l), Some(doc)) = (c.live(), ed.doc.as_ref()) else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("collab presence"))).with_clip_rect(view);
    let at = |p: DVec3| crate::selection::project(cam, p, view.width(), view.height()).map(|(x, y)| view.min + Vec2::new(x, y));
    for (id, (p, heard)) in &l.seen {
        if *id == l.you.id || heard.elapsed() > SEEN_FOR {
            continue;
        }
        let Some(peer) = l.peers.iter().find(|q| q.id == *id) else { continue };
        let color = Color32::from_rgb(peer.color[0], peer.color[1], peer.color[2]);
        for &(x, y, sid) in &p.selected {
            let k = (x, y);
            if let Some(o) = doc.object(k, sid) {
                if let Some(s) = at(doc.object_world(k, o, false) + DVec3::Z * 0.5) {
                    painter.circle_stroke(s, 9.0, Stroke::new(2.5, color));
                }
            } else if let Some(sp) = doc.spline(k, sid) {
                let curve = omsi_geometry::SplineCurve::from_map(sp, tile_origin(k));
                let pts: Vec<egui::Pos2> = (0..=12).filter_map(|i| at(curve.point_at(sp.length * i as f64 / 12.0) + DVec3::Z * 0.3)).collect();
                if pts.len() > 1 {
                    painter.add(egui::Shape::line(pts, Stroke::new(3.0, color)));
                }
            }
        }
        let Some(cur) = p.cursor else { continue };
        let Some(s) = at(DVec3::new(cur[0], cur[1], cur[2])) else { continue };
        if !view.contains(s) {
            continue;
        }
        painter.circle_filled(s, 5.0, color);
        painter.circle_stroke(s, 5.0, Stroke::new(1.5, Color32::WHITE));
        // (the name as plain text, with the tool in hand)
        // (on the player's own colour, in black or white as reads on it: a pale colour took
        // white text out of sight)
        let fill = color.gamma_multiply(0.92);
        let ink = crate::theme::ink_on(color);
        let galley = painter.layout_no_wrap(format!("{} · {}", peer.name, p.tool), egui::FontId::proportional(12.0), ink);
        let r = egui::Rect::from_min_size(s + Vec2::new(9.0, -8.0), galley.size() + Vec2::new(8.0, 4.0));
        painter.rect_filled(r, CornerRadius::same(3), fill);
        painter.galley(r.min + Vec2::new(4.0, 2.0), galley, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_goes_when_it_changed_and_not_too_often() {
        let mut s = Sender::default();
        let t = Instant::now();
        let p = |x: f64| Presence { cursor: Some([x, 0.0, 0.0]), ..Default::default() };
        assert!(s.next(t, p(1.0)).is_some());
        // (the same again: nothing; another too soon: nothing)
        assert!(s.next(t + Duration::from_millis(500), p(1.0)).is_none());
        assert!(s.next(t + Duration::from_millis(50), p(2.0)).is_none());
        assert!(s.next(t + Duration::from_millis(150), p(2.0)).is_some());
        // (standing still: told again now and then, well within the time it is drawn)
        assert!(s.next(t + Duration::from_millis(3200), p(2.0)).is_some());
        assert!(AGAIN * 2 < SEEN_FOR);
    }

    #[test]
    fn the_selection_and_the_cursor_are_told() {
        let mut ed = Editor::default();
        ed.selected = Some(Sel::Object((1, -2), 77));
        ed.also = vec![Sel::Spline((0, 0), 5)];
        ed.cursor = Some(DVec3::new(10.04, 20.06, 3.0));
        ed.cursor_in_view = true;
        let p = mine(&ed);
        assert_eq!(p.selected, vec![(1, -2, 77), (0, 0, 5)]);
        assert_eq!(p.cursor, Some([10.0, 20.1, 3.0]));
        // (a cursor off the view is not where they work)
        ed.cursor_in_view = false;
        assert_eq!(mine(&ed).cursor, None);
        ed.also = (0..1000).map(|i| Sel::Object((0, 0), i)).collect();
        assert_eq!(mine(&ed).selected.len(), MAX_SELECTED);
    }
}
