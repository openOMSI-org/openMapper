//! Map settings (Map menu), as OMSI 2's editor has them: the money and tickets the map
//! uses, its years, the depot buses start from, the repair time, real rail and the seasons
//! of the year.


use egui::{Color32, RichText, Vec2};
use omsi_map::global::Season;

use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::ui::{check, prop_row, TEXT_DIM};

#[derive(Default)]
pub struct MapSettings {
    pub open: bool,
    /// What can be chosen (read when the window opens): money systems, ticket packs, depots.
    lists: Option<(Vec<String>, Vec<String>, Vec<String>)>,
}

/// The files `<root>/<top>/<folder>/*.<ext>` of both roots, as a map names them
/// (`Money\DM\DM.cfg`).
pub(crate) fn content_files(roots: &Roots, top: &str, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for root in [&roots.content, &roots.omsi] {
        let base = omsi_cfg::resolve_path(root, top);
        for d in std::fs::read_dir(&base).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            for f in std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()) {
                if f.extension().map(|x| x.eq_ignore_ascii_case(ext)).unwrap_or(false) {
                    let rel = format!("{top}\\{}\\{}", d.file_name().unwrap_or_default().to_string_lossy(), f.file_name().unwrap_or_default().to_string_lossy());
                    if !v.iter().any(|x| x.eq_ignore_ascii_case(&rel)) {
                        v.push(rel);
                    }
                }
            }
        }
    }
    v.sort_by_key(|s| s.to_lowercase());
    v
}

fn season_name(kind: i32) -> &'static str {
    match kind {
        1 => tr("Spring", "Весна"),
        2 => tr("Autumn", "Осень"),
        3 => tr("Winter", "Зима"),
        4 => tr("Winter with snow", "Зима со снегом"),
        5 => tr("Summer (dry)", "Лето (сухое)"),
        _ => tr("Summer", "Лето"),
    }
}

/// A day of the year (0 = 1 January) as "15 Mar".
fn day_name(day: i32) -> String {
    const LEN: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let months = [tr("Jan", "янв"), tr("Feb", "фев"), tr("Mar", "мар"), tr("Apr", "апр"), tr("May", "мая"), tr("Jun", "июн"), tr("Jul", "июл"), tr("Aug", "авг"), tr("Sep", "сен"), tr("Oct", "окт"), tr("Nov", "ноя"), tr("Dec", "дек")];
    let mut d = day.clamp(0, 370);
    for (m, l) in LEN.iter().enumerate() {
        if d < *l {
            return format!("{} {}", d + 1, months[m]);
        }
        d -= l;
    }
    format!("31 {}", months[11])
}

/// A file chooser of a list (empty: none).
fn choose(ui: &mut egui::Ui, id: &str, value: &mut String, list: &[String]) {
    let shown = if value.is_empty() { tr("(none)", "(нет)").to_string() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(230.0).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), tr("(none)", "(нет)"));
        for f in list {
            ui.selectable_value(value, f.clone(), f);
        }
    });
}

impl MapSettings {
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: Option<&Roots>) {
        if !self.open {
            self.lists = None;
            return;
        }
        let (Some(doc), Some(roots)) = (ed.doc.as_mut(), roots) else { return };
        let lists = self.lists.get_or_insert_with(|| {
            // ([standarddepot] names one of the AI's depot groups: the player's bus comes
            // from it)
            let deps: Vec<String> = doc.traffic.depots().map(|g| g.name.clone()).collect();
            (content_files(roots, "Money", "cfg"), content_files(roots, "TicketPacks", "otp"), deps)
        });
        let before = doc.global.clone();
        let g = &mut doc.global;
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Settings, tr("Map settings", "Настройки карты"))).id(egui::Id::new("map settings")).open(&mut open).default_pos([340.0, 100.0]).default_size([440.0, 520.0]).resizable(true).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("map settings grid").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                    prop_row(ui, tr("Money", "Валюта"), |ui| choose(ui, "money", &mut g.money_system, &lists.0));
                    prop_row(ui, tr("Tickets", "Билеты"), |ui| choose(ui, "tickets", &mut g.ticket_pack, &lists.1));
                    prop_row(ui, tr("Depot", "Депо"), |ui| {
                        if lists.2.is_empty() && g.standard_depot.is_empty() {
                            ui.label(RichText::new(tr("none: the map has no depot group (Traffic and AI)", "нет: у карты нет группы депо (Трафик и ИИ)")).color(TEXT_DIM));
                        } else {
                            choose(ui, "depot", &mut g.standard_depot, &lists.2);
                        }
                    });
                    prop_row(ui, tr("Years", "Годы"), |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut g.years.0).range(1900..=2100));
                            ui.label("–");
                            ui.add(egui::DragValue::new(&mut g.years.1).range(1900..=2100));
                        });
                    });
                    prop_row(ui, tr("Year offset", "Сдвиг года"), |ui| {
                        ui.add(egui::DragValue::new(&mut g.real_year_offset).range(-200..=200)).on_hover_text(tr("Added to today's year when the game starts at the real date (Berlin-Spandau: -25 for the 1980s)", "Прибавляется к текущему году при старте игры в реальную дату (Berlin-Spandau: -25, чтобы были 80-е)"));
                    });
                    prop_row(ui, tr("Repair time", "Время ремонта"), |ui| {
                        ui.add(egui::DragValue::new(&mut g.repair_time_min).range(0.0..=240.0).speed(0.5).suffix(tr(" min", " мин")));
                    });
                    prop_row(ui, tr("Rail", "Рельсы"), |ui| {
                        check(ui, &mut g.real_rail, tr("real rail (trams and trains follow their tracks' switches)", "настоящие рельсы (трамваи и поезда идут по стрелкам)"));
                    });
                    prop_row(ui, tr("Helper", "Помощник"), |ui| {
                        check(ui, &mut g.dyn_helper_active, tr("dynamic helper active (the original editor's map option, kept as it is)", "динамический помощник включён (параметр карты из оригинального редактора)"));
                    });
                });
                ui.add_space(10.0);
                ui.label(RichText::new(tr("Seasons", "Времена года")).strong());
                ui.label(RichText::new(tr("Summer where none is set. The days count from 1 January.", "Где ничего не задано — лето. Дни считаются от 1 января.")).small().color(TEXT_DIM));
                let mut remove = None;
                for (i, s) in g.seasons.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("season", i)).selected_text(season_name(s.kind)).width(130.0).show_ui(ui, |ui| {
                            for k in 0..=5 {
                                ui.selectable_value(&mut s.kind, k, season_name(k));
                            }
                        });
                        ui.add(egui::DragValue::new(&mut s.start_day).range(0..=370).custom_formatter(|v, _| day_name(v as i32)));
                        ui.label("–");
                        ui.add(egui::DragValue::new(&mut s.end_day).range(0..=370).custom_formatter(|v, _| day_name(v as i32)));
                        if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    g.seasons.remove(i);
                }
                if ui.button(icons::labelled(Icon::Add, tr("Add a season", "Добавить время года"))).clicked() {
                    let start = g.seasons.iter().map(|s| s.end_day).max().unwrap_or(0).min(360);
                    g.seasons.push(Season { kind: 3, start_day: start, end_day: (start + 30).min(370) });
                }
                if g.seasons.iter().any(|s| s.end_day <= s.start_day) {
                    ui.label(RichText::new(tr("A season ends before it starts", "Время года кончается раньше, чем начинается")).color(Color32::from_rgb(230, 160, 60)));
                }
            });
        });
        self.open = open;
        if doc.global != before {
            let order = doc.order.clone();
            ed.history.push_snapshot(tr("Map settings", "Настройки карты"), crate::history::Snapshot { global: Some((before, order)), timetable: None, traffic: None, descriptions: None, groups: None });
            if let Some(doc) = ed.doc.as_mut() {
                doc.global_dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_of_the_year_are_named() {
        assert_eq!(day_name(0), "1 Jan");
        assert_eq!(day_name(38), "8 Feb");
        assert_eq!(day_name(364), "31 Dec");
    }
}
