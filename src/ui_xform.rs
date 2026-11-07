//! Location, rotation and scale as numbers, as in Blender's Transform panel: a row for each
//! axis with the axis' colour, the whole width of the panel to drag or type in.

use egui::{Color32, RichText};

use crate::i18n::tr;

fn axis_color(i: usize) -> Color32 {
    let c = crate::gizmo::AXIS_COLORS[i];
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

/// Three numbers, x, y and z, one under the other; which of them changed.
#[allow(clippy::too_many_arguments)]
pub fn xyz(ui: &mut egui::Ui, values: &mut [f64; 3], speed: f64, suffix: &str, decimals: usize, range: Option<(f64, f64)>, hints: [&str; 3]) -> [bool; 3] {
    let mut changed = [false; 3];
    ui.vertical(|ui| {
        for i in 0..3 {
            ui.horizontal(|ui| {
                // (the axis letter takes a fixed width: X, Y and Z are not the same width in a
                // proportional font, so the boxes beside them began at three different places
                // and the panel looked a little out of line)
                ui.allocate_ui_with_layout(egui::vec2(AXIS_LETTER_W, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(RichText::new(["X", "Y", "Z"][i]).strong().color(axis_color(i)));
                });
                let mut dv = egui::DragValue::new(&mut values[i]).speed(speed).max_decimals(decimals).min_decimals(decimals.min(2)).suffix(suffix);
                if let Some((lo, hi)) = range {
                    dv = dv.range(lo..=hi);
                }
                let w = field_width(ui.available_width());
                let r = ui.add_sized([w, 20.0], dv);
                let r = if hints[i].is_empty() { r } else { r.on_hover_text(hints[i]) };
                changed[i] = r.changed();
            });
        }
    });
    changed
}

/// The width of the axis letter in an `X`/`Y`/`Z` row: one width for all three, so the boxes
/// beside them line up.
const AXIS_LETTER_W: f32 = 16.0;

/// The widest a transform row's number field grows: past this the row leaves margin beside it,
/// rather than the field taking the whole width of the Grid's second column and running into
/// the panel's edge.
const AXIS_FIELD_MAX: f32 = 240.0;

/// The width a number field takes when the row has `room` points to give it: the room less a
/// little, never under a few digits, never wider than [`AXIS_FIELD_MAX`].
fn field_width(room: f32) -> f32 {
    (room - 4.0).clamp(60.0, AXIS_FIELD_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields are held between a readable minimum and a maximum, so a row never reaches past
    /// its panel however wide the panel is (a field at `available_width` was what pushed the
    /// properties panel's boxes past its right edge).
    #[test]
    fn a_field_is_held_between_its_limits() {
        assert_eq!(field_width(10.0), 60.0);
        assert_eq!(field_width(1000.0), AXIS_FIELD_MAX);
        assert!((field_width(154.0) - 150.0).abs() < 1e-3);
        // (a narrow panel still shows a field, and one wider than the maximum does not grow)
        for room in [50.0f32, 120.0, 260.0, 400.0] {
            let w = field_width(room);
            assert!((60.0..=AXIS_FIELD_MAX).contains(&w), "{w} for {room} of room");
        }
    }

    /// The three rows of a transform are one width, in a panel of any width: nothing of a row
    /// reaches past what it was given.
    #[test]
    fn the_axis_rows_stay_inside_their_panel() {
        for panel in [220.0f32, 270.0, 420.0] {
            let mut taken = 0.0f32;
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(Default::default(), |ui| {
                ui.scope(|ui| {
                    ui.set_max_width(panel);
                    let mut v = [1.0f64, 2.0, 3.0];
                    let inner = ui.allocate_ui(egui::vec2(panel, 60.0), |ui| {
                        let _ = xyz(ui, &mut v, 0.05, " m", 2, None, ["", "", ""]);
                        ui.min_rect().width()
                    });
                    taken = inner.inner;
                });
            });
            assert!(taken <= panel + 0.5, "the rows take {taken} in a {panel} panel");
        }
    }
}

/// An object's own size along its axes. A size dragged or typed is held here until the
/// drag ends or the typing is done (each new size makes files): `(done, size)` meanwhile and
/// then. The whole is kept in proportion while `uniform` is on.
pub fn scale(ui: &mut egui::Ui, id: egui::Id, now: [f64; 3], uniform: &mut bool) -> Option<(bool, [f64; 3])> {
    let held: Option<[f64; 3]> = ui.data(|d| d.get_temp(id));
    let base = held.unwrap_or(now);
    let mut v = base;
    let mut out = None;
    ui.vertical(|ui| {
        let mut busy = false;
        let mut finished = false;
        for i in 0..3 {
            ui.horizontal(|ui| {
                // (one width for X, Y and Z, as in `xyz`: the boxes line up)
                ui.allocate_ui_with_layout(egui::vec2(AXIS_LETTER_W, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(RichText::new(["X", "Y", "Z"][i]).strong().color(axis_color(i)));
                });
                let w = field_width(ui.available_width());
                let r = ui.add_sized([w, 20.0], egui::DragValue::new(&mut v[i]).speed(0.01).range(0.01..=100.0).max_decimals(3).min_decimals(2).prefix("× "));
                if r.changed() && *uniform {
                    let f = v[i] / base[i].max(1e-6);
                    v = [base[0] * f, base[1] * f, base[2] * f];
                }
                busy |= r.dragged() || r.has_focus();
                finished |= r.drag_stopped() || r.lost_focus();
            });
        }
        if v != base || held.is_some() {
            if finished || !busy {
                ui.data_mut(|d| d.remove::<[f64; 3]>(id));
                out = Some((true, v));
            } else {
                ui.data_mut(|d| d.insert_temp(id, v));
                out = Some((false, v));
            }
        }
        ui.horizontal(|ui| {
            ui.checkbox(uniform, tr("Keep proportions", "Сохранять пропорции"));
            if (now[0] - 1.0).abs() + (now[1] - 1.0).abs() + (now[2] - 1.0).abs() > 1e-4 && ui.small_button("1 : 1").on_hover_text(tr("Back to its own size", "Вернуть исходный размер")).clicked() {
                out = Some((true, [1.0; 3]));
            }
        });
    });
    out
}
