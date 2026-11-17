//! The drawing board for a road type's markings: the roadway as the texture lies on it (the
//! road's width across, a stretch of it along), drawn on by hand with a pen, straight and dashed
//! lines, rectangles, arrows and an eraser. What is drawn goes over the roadway texture and the
//! lines list of the road maker (see `road_maker::Hand`) and into the road type when it is made.

use std::sync::Arc;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::i18n::tr;
use crate::road_maker::{arrow_polygon, Hand, RoadSpec, HAND_H, HAND_W, WHITE, YELLOW};
use crate::theme::pal;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pen,
    Line,
    Dashed,
    Rect,
    Arrow,
    Eraser,
}

pub struct MarkCanvas {
    pub open: bool,
    tool: Tool,
    color: [u8; 3],
    /// The pen's and lines' width (m).
    width: f32,
    dash: f32,
    gap: f32,
    snap: bool,
    back: bool,
    start: Option<(f32, f32)>,
    last: Option<(f32, f32)>,
    undo: Vec<Option<Arc<Hand>>>,
    /// The picture shown, with what it was made from.
    shown: Option<(u64, egui::TextureHandle)>,
    rev: u64,
}

impl Default for MarkCanvas {
    fn default() -> Self {
        MarkCanvas { open: false, tool: Tool::Pen, color: WHITE, width: 0.15, dash: 3.0, gap: 6.0, snap: true, back: false, start: None, last: None, undo: Vec::new(), shown: None, rev: 0 }
    }
}

fn hash_of(spec: &RoadSpec, rev: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // (the spec without the drawing, which is `rev`)
    serde_json::to_string(spec).unwrap_or_default().hash(&mut h);
    rev.hash(&mut h);
    h.finish()
}

impl MarkCanvas {
    fn snapshot(&mut self, spec: &RoadSpec) {
        self.undo.push(spec.hand.clone());
        if self.undo.len() > 30 {
            self.undo.remove(0);
        }
    }

    fn hand<'a>(spec: &'a mut RoadSpec) -> &'a mut Hand {
        Arc::make_mut(spec.hand.get_or_insert_with(|| Arc::new(Hand::default())))
    }

    pub fn window(&mut self, ctx: &egui::Context, spec: &mut RoadSpec) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("Draw road markings", "Рисовать разметку")).id(egui::Id::new("mark canvas")).open(&mut open).default_size(Vec2::new(940.0, 600.0)).show(ctx, |ui| {
            let (cw, len) = (spec.canvas_width(), spec.mark_period());
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    for (t, label) in [(Tool::Pen, tr("Pen", "Кисть")), (Tool::Line, tr("Straight line", "Прямая линия")), (Tool::Dashed, tr("Dashed line", "Прерывистая линия")), (Tool::Rect, tr("Rectangle", "Прямоугольник")), (Tool::Arrow, tr("Arrow", "Стрелка")), (Tool::Eraser, tr("Eraser", "Ластик"))] {
                        if ui.add_sized([170.0, 24.0], egui::Button::new(label).selected(self.tool == t)).clicked() {
                            self.tool = t;
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        for c in [WHITE, YELLOW, [200, 50, 45], [60, 110, 200]] {
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                            ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                            if self.color == c {
                                ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
                            }
                            if resp.clicked() {
                                self.color = c;
                            }
                        }
                        ui.color_edit_button_srgb(&mut self.color);
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr("Width", "Ширина"));
                        ui.add(egui::DragValue::new(&mut self.width).speed(0.01).range(0.03..=2.0).fixed_decimals(2).suffix(" m"));
                    });
                    if self.tool == Tool::Dashed {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut self.dash).speed(0.1).range(0.3..=12.0).fixed_decimals(1).suffix(" m")).on_hover_text(tr("Dash length", "Длина штриха"));
                            ui.add(egui::DragValue::new(&mut self.gap).speed(0.1).range(0.3..=20.0).fixed_decimals(1).suffix(" m")).on_hover_text(tr("Gap between dashes", "Промежуток между штрихами"));
                        });
                    }
                    if self.tool == Tool::Arrow {
                        ui.checkbox(&mut self.back, tr("Pointing back", "Назад"));
                    }
                    ui.checkbox(&mut self.snap, tr("Snap to 5 cm", "Шаг 5 см"));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.undo.is_empty(), egui::Button::new(tr("Undo", "Отмена"))).clicked() {
                            if let Some(h) = self.undo.pop() {
                                spec.hand = h;
                                self.rev += 1;
                            }
                        }
                        if ui.button(tr("Clear", "Очистить")).clicked() {
                            self.snapshot(spec);
                            spec.hand = None;
                            self.rev += 1;
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("Drag to draw. The picture is the roadway as the texture lies on it, up is the way of the road; it repeats along, so what leaves at the top comes in at the bottom.", "Рисуйте протяжкой. Картинка — проезжая часть так, как на ней лежит текстура, вверх — по ходу дороги; она повторяется вдоль, и что уходит вверх, входит снизу.")).small().color(pal().text_dim));
                });
                ui.separator();
                self.board(ui, spec, cw, len);
            });
        });
        if !open {
            self.open = false;
        }
    }

    fn board(&mut self, ui: &mut egui::Ui, spec: &mut RoadSpec, cw: f32, len: f32) {
        let avail = ui.available_size();
        let k = ((avail.x - 10.0) / cw).min((avail.y - 10.0) / len).clamp(4.0, 200.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw * k, len * k), Sense::click_and_drag());
        // the picture: the texture with the lines and the drawing on it
        let key = hash_of(spec, self.rev);
        if self.shown.as_ref().map(|s| s.0 != key).unwrap_or(true) {
            if let Some(img) = spec.painted_roadway() {
                let ci = egui::ColorImage::from_rgba_unmultiplied([HAND_W as usize, HAND_H as usize], img.as_raw());
                match &mut self.shown {
                    Some((k0, h)) => {
                        h.set(ci, egui::TextureOptions::LINEAR);
                        *k0 = key;
                    }
                    None => self.shown = Some((key, ui.ctx().load_texture("road marks board", ci, egui::TextureOptions::LINEAR))),
                }
            }
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, pal().view);
        match &self.shown {
            Some((_, h)) => {
                // (row 0 is the road's start, at the bottom: the way of the road goes up)
                egui::Image::new((h.id(), rect.size())).uv(Rect::from_min_max(Pos2::new(0.0, 1.0), Pos2::new(1.0, 0.0))).paint_at(ui, rect);
            }
            None => {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("Pick a roadway texture first", "Сначала выберите текстуру проезжей части"), egui::FontId::proportional(14.0), pal().view_text_dim);
                return;
            }
        }
        // a metre grid, faint, and the 5 m marks
        let to_screen = |m: (f32, f32)| Pos2::new(rect.left() + m.0 * k, rect.bottom() - m.1 * k);
        let mut x = 1.0;
        while x < cw {
            painter.line_segment([to_screen((x, 0.0)), to_screen((x, len))], Stroke::new(1.0, Color32::from_white_alpha(10)));
            x += 1.0;
        }
        let mut y = 1.0;
        while y < len {
            painter.line_segment([to_screen((0.0, y)), to_screen((cw, y))], Stroke::new(1.0, Color32::from_white_alpha(if (y as i32) % 5 == 0 { 28 } else { 10 })));
            y += 1.0;
        }
        painter.text(rect.left_top() + Vec2::new(4.0, 2.0), egui::Align2::LEFT_TOP, format!("{cw:.1} × {len:.1} m"), egui::FontId::proportional(11.0), pal().view_text);

        let snap = self.snap;
        let to_m = |p: Pos2| -> (f32, f32) {
            let (mut x, mut y) = ((p.x - rect.left()) / k, (rect.bottom() - p.y) / k);
            if snap {
                x = (x / 0.05).round() * 0.05;
                y = (y / 0.05).round() * 0.05;
            }
            (x.clamp(0.0, cw), y.clamp(0.0, len))
        };
        let (color, r, erase) = (self.color, self.width / 2.0, self.tool == Tool::Eraser);
        let cur = resp.interact_pointer_pos().or(resp.hover_pos()).map(to_m);
        // the brush's footprint under the pointer
        if let (Some(m), true) = (cur, resp.hovered()) {
            painter.circle_stroke(to_screen(m), (r * k).max(2.0), Stroke::new(1.0, Color32::from_white_alpha(160)));
        }
        let dashes = |h: &mut Hand, a: (f32, f32), b: (f32, f32), dash: f32, gap: f32| {
            let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1e-6);
            let at = |t: f32| (a.0 + (b.0 - a.0) * t / l, a.1 + (b.1 - a.1) * t / l);
            let mut t = 0.0;
            while t < l {
                h.stroke(cw, len, at(t), at((t + dash).min(l)), r, color, false);
                t += dash + gap;
            }
        };
        if resp.drag_started() {
            self.snapshot(spec);
            self.start = cur;
            self.last = cur;
        }
        if resp.dragged() {
            if let (Some(m), Some(l)) = (cur, self.last) {
                if matches!(self.tool, Tool::Pen | Tool::Eraser) && m != l {
                    Self::hand(spec).stroke(cw, len, l, m, r, color, erase);
                    self.last = Some(m);
                    self.rev += 1;
                }
            }
            // the shape being drawn
            if let (Some(a), Some(b)) = (self.start, cur) {
                let ghost = Stroke::new((self.width * k).max(1.5), Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 170));
                match self.tool {
                    Tool::Line | Tool::Dashed => {
                        painter.line_segment([to_screen(a), to_screen(b)], ghost);
                    }
                    Tool::Rect => {
                        painter.rect_stroke(Rect::from_two_pos(to_screen(a), to_screen(b)), 0.0, Stroke::new(1.5, Color32::from_white_alpha(200)), egui::StrokeKind::Middle);
                    }
                    _ => {}
                }
            }
        }
        if resp.drag_stopped() {
            if let (Some(a), Some(b)) = (self.start.take(), cur) {
                let h = Self::hand(spec);
                match self.tool {
                    Tool::Line => h.stroke(cw, len, a, b, r, color, false),
                    Tool::Dashed => dashes(h, a, b, self.dash, self.gap),
                    Tool::Rect => h.fill(cw, len, &[a, (b.0, a.1), b, (a.0, b.1)], color, false),
                    _ => {}
                }
                self.rev += 1;
            }
            self.last = None;
        }
        if resp.clicked() {
            if let Some(m) = cur {
                match self.tool {
                    Tool::Pen | Tool::Eraser => {
                        self.snapshot(spec);
                        Self::hand(spec).stroke(cw, len, m, m, r, color, erase);
                    }
                    Tool::Arrow => {
                        self.snapshot(spec);
                        Self::hand(spec).fill(cw, len, &arrow_polygon(m, self.back), color, false);
                    }
                    _ => {}
                }
                self.rev += 1;
            }
        }
    }
}
