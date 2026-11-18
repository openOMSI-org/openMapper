//! The ruler's area: with the ruler on, `Shift` + clicks put the corners of an outline (a
//! square, a car park, a wood to plant), closed back to the first corner; its area and
//! perimeter seen from above are written in the view and in the status bar. A plain click
//! starts a plain measurement again.

use egui::{Color32, CornerRadius, Vec2};
use glam::{DVec2, DVec3};

use crate::editor::Editor;
use crate::i18n::tr;

/// The area (m², seen from above) and the perimeter (m, closed) of an outline.
pub fn area_perimeter(pts: &[DVec2]) -> (f64, f64) {
    if pts.len() < 2 {
        return (0.0, 0.0);
    }
    let n = pts.len();
    let mut twice = 0.0;
    let mut per = 0.0;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        twice += a.x * b.y - b.x * a.y;
        per += a.distance(b);
    }
    (if n >= 3 { twice.abs() * 0.5 } else { 0.0 }, per)
}

/// An area as it reads best: m² below a hectare, else hectares too.
pub fn area_text(a: f64) -> String {
    if a >= 10_000.0 { format!("{a:.0} m² ({:.2} ha)", a / 10_000.0) } else { format!("{a:.1} m²") }
}

impl Editor {
    /// The outline's corners, the cursor as the next one while the ruler is on.
    pub fn ruler_outline(&self) -> Vec<DVec3> {
        if self.ruler.is_none() {
            return Vec::new();
        }
        let mut v = self.ruler_area.clone();
        if let (false, Some(c), true, true) = (v.is_empty(), self.cursor, self.cursor_in_view, self.ruler.is_some()) {
            v.push(c);
        }
        v
    }

    /// The outline measured: (area, perimeter), once it has three corners.
    pub fn ruler_measure(&self) -> Option<(f64, f64)> {
        let v = self.ruler_outline();
        (v.len() >= 3).then(|| area_perimeter(&v.iter().map(|p| p.truncate()).collect::<Vec<_>>()))
    }

    /// A click with the ruler on: `Shift` adds a corner to the outline, else the outline goes.
    pub fn ruler_area_click(&mut self, at: DVec3, shift: bool) -> bool {
        if shift {
            self.ruler_area.push(at);
            self.ruler = Some((None, None));
            return true;
        }
        self.ruler_area.clear();
        false
    }
}

/// The outline in the view: its edges, closed to the first corner, and its corners.
pub fn draw(x: &mut crate::render::FrameExtras, cam: &crate::camera::Camera, ed: &Editor, width: f32) {
    let v = ed.ruler_outline();
    if v.len() < 2 {
        return;
    }
    let lift = DVec3::Z * 0.5;
    let colour = [1.0, 0.85, 0.2, 1.0];
    for i in 0..v.len() {
        let closing = i + 1 == v.len();
        if closing && v.len() < 3 {
            break;
        }
        let (a, b) = (v[i], v[(i + 1) % v.len()]);
        crate::editor::line(&mut x.overlay, cam, a + lift, b + lift, if closing { width * 0.5 } else { width }, colour);
    }
    for p in &ed.ruler_area {
        crate::editor::disc(&mut x.overlay, cam, *p + lift, width * 2.5, colour);
    }
}

/// The area and perimeter written over the outline's middle.
pub fn paint(ctx: &egui::Context, ed: &Editor, cam: &crate::camera::Camera, view: egui::Rect) {
    let Some((a, p)) = ed.ruler_measure() else { return };
    let v = ed.ruler_outline();
    let mid = v.iter().fold(DVec3::ZERO, |s, q| s + *q) / v.len() as f64;
    let Some((x, y)) = crate::selection::project(cam, mid + DVec3::Z, view.width(), view.height()) else { return };
    let at = view.min + Vec2::new(x, y);
    if !view.contains(at) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("ruler area"))).with_clip_rect(view);
    let galley = painter.layout_no_wrap(format!("{}  {} {p:.1} m", area_text(a), tr("perimeter", "периметр")), egui::FontId::monospace(12.0), Color32::BLACK);
    let r = egui::Rect::from_center_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), Color32::from_rgb(255, 216, 51));
    painter.galley(r.min + Vec2::new(5.0, 2.0), galley, Color32::BLACK);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_and_a_triangle() {
        let sq = [DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0), DVec2::new(10.0, 10.0), DVec2::new(0.0, 10.0)];
        assert_eq!(area_perimeter(&sq), (100.0, 40.0));
        let mut back = sq;
        back.reverse();
        assert_eq!(area_perimeter(&back).0, 100.0);
        let tri = [DVec2::new(0.0, 0.0), DVec2::new(3.0, 0.0), DVec2::new(0.0, 4.0)];
        assert_eq!(area_perimeter(&tri), (6.0, 12.0));
        assert_eq!(area_text(25_000.0), "25000 m² (2.50 ha)");
    }

    /// Shift + clicks make the outline; a plain click lets it go.
    #[test]
    fn shift_clicks_make_an_outline() {
        let mut ed = Editor::default();
        ed.toggle_ruler();
        for p in [(0.0, 0.0), (20.0, 0.0), (20.0, 5.0)] {
            assert!(ed.ruler_area_click(DVec3::new(p.0, p.1, 0.0), true));
        }
        assert_eq!(ed.ruler_measure(), Some((50.0, 20.0 + 5.0 + 425f64.sqrt())));
        assert!(!ed.ruler_area_click(DVec3::ZERO, false));
        assert!(ed.ruler_measure().is_none());
    }
}
