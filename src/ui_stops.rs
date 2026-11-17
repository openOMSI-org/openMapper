//! A bus stop's panel: its kind and the people waiting, getting off, the length and the side
//! (see [`crate::stops`]), and the kind a new stop gets.

use egui::{RichText, Vec2};

use crate::i18n::tr;
use crate::stops::{Side, StopKind, StopSettings};
use crate::ui::prop_row;
use crate::theme::pal;

/// The kind of stop the Game logic tool places next.
pub fn kind_picker(ui: &mut egui::Ui, kind: &mut StopKind) {
    ui.label(RichText::new(tr("Kind of stop", "Вид остановки")).color(pal().text_dim));
    ui.horizontal_wrapped(|ui| {
        for k in StopKind::ALL {
            ui.selectable_value(kind, k, k.label()).on_hover_text(k.hint());
        }
    });
    ui.add_space(4.0);
}

/// The settings of the stop whose strings are `labels`, edited in place; whether they changed.
pub fn settings(ui: &mut egui::Ui, labels: &mut Vec<String>) -> bool {
    let mut s = StopSettings::read(labels);
    let was = s.clone();
    ui.add_space(4.0);
    egui::Grid::new("stop settings").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
        prop_row(ui, tr("Kind", "Вид"), |ui| {
            let mut kind = s.kind;
            egui::ComboBox::from_id_salt("stop kind").selected_text(kind.label()).width(170.0).show_ui(ui, |ui| {
                for k in StopKind::ALL {
                    ui.selectable_value(&mut kind, k, k.label()).on_hover_text(k.hint());
                }
            });
            if kind != s.kind {
                s.set_kind(kind);
            }
        });
        prop_row(ui, tr("Waiting, at most", "Ждут, не больше"), |ui| {
            ui.add(egui::DragValue::new(&mut s.enter_max).range(0..=200).speed(0.2));
        });
        prop_row(ui, tr("Waiting, at least", "Ждут, не меньше"), |ui| {
            ui.add(egui::DragValue::new(&mut s.enter_min).range(0..=s.enter_max.max(0)).speed(0.2));
        });
        prop_row(ui, tr("Getting off", "Выходят"), |ui| {
            ui.add(egui::DragValue::new(&mut s.exit).range(0..=500).speed(0.2)).on_hover_text(tr("How many of those riding get off here, compared with the other stops: twice the number, twice the riders", "Сколько из едущих выходит здесь по сравнению с другими остановками: вдвое больше число — вдвое больше людей"));
        });
        prop_row(ui, tr("Length", "Длина"), |ui| {
            ui.add(egui::DragValue::new(&mut s.length).range(5.0..=150.0).speed(0.5).suffix(" m")).on_hover_text(tr("How far along the stop people wait and a bus may stand", "На каком протяжении ждут люди и может стоять автобус"));
        });
        prop_row(ui, tr("Platform", "Платформа"), |ui| {
            let mut side = s.side;
            egui::ComboBox::from_id_salt("stop side").selected_text(side.label()).width(170.0).show_ui(ui, |ui| {
                for v in [Side::Right, Side::Left, Side::Both] {
                    ui.selectable_value(&mut side, v, v.label());
                }
            });
            s.side = side;
        });
    });
    ui.label(RichText::new(s.kind.hint()).small().color(pal().text_dim));
    if s == was {
        return false;
    }
    s.write(labels);
    true
}

/// What the pair row of a stop asks for.
pub enum PairAction {
    /// The stop leaves its pair: it gets a name of its own.
    Split,
    /// A name another far stop has: made different.
    MakeUnique,
}

/// Under a stop's name: whether it is a pair with the stop across the road (the two keep one
/// name; renaming one renames the other) and the button to split it, and a warning when
/// another stop far from it has its name.
pub fn pair(ui: &mut egui::Ui, stops: &[crate::stops::Found], id: i64) -> Option<PairAction> {
    let mut out = None;
    let twins = crate::stops::pair_of(stops, id);
    if !twins.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} {}", crate::icons::Icon::Swap.ch(), tr("Paired with the stop across the road: one name for both", "В паре с остановкой напротив: одно имя на обе"))).small().color(pal().text_dim));
            if ui.small_button(tr("Split", "Разделить")).on_hover_text(tr("The stop gets a name of its own (the number is changed in the name box), and renaming one no longer renames the other", "Остановка получает своё имя (число правится в поле имени), и переименование одной больше не переименовывает другую")).clicked() {
                out = Some(PairAction::Split);
            }
        });
    }
    let clashes = crate::stops::clashes(stops, id);
    if !clashes.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} {}", crate::icons::Icon::Warning.ch(), tr("Another stop far from here has this name", "Это имя носит другая остановка вдали отсюда"))).small().color(pal().warn));
            if ui.small_button(tr("Make it different", "Сделать другим")).clicked() {
                out = Some(PairAction::MakeUnique);
            }
        });
    }
    out
}
