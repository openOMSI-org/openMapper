//! The Signal routes window (Map menu), as OMSI 2's editor keeps them: for each railway
//! signal the track pieces it protects, its distant signal, the next signal and a speed
//! limit. Pieces and signals are taken from what is selected in the view.

use egui::{RichText, Vec2};
use omsi_map::ailists::SignalRoute;

use crate::doc::Sel;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::library::Library;
use crate::traffic::File;
use crate::ui::{prop_row, Action};
use crate::theme::pal;

#[derive(Default)]
pub struct SignalWindow {
    pub open: bool,
    /// The route shown below the list.
    current: Option<usize>,
}

/// An object's name in the catalogue, else its IDCode.
fn name_of(ed: &Editor, lib: &Library, id: i64) -> String {
    // (a signal may also hang on another object, or stand in a row along a track)
    let file = ed.doc.as_ref().and_then(|d| d.tiles.values().find_map(|t| t.tile.objects.iter().chain(&t.tile.attach_objects).find(|o| o.id == id).map(|o| o.file.clone()).or_else(|| t.tile.spline_attachments.iter().find(|a| a.id == id).map(|a| a.file.clone()))));
    match file {
        Some(f) => format!("{} ({id})", lib.find(&f).map(|a| a.name.clone()).unwrap_or_else(|| f.rsplit('\\').next().unwrap_or(&f).to_string())),
        None => format!("{id} ({})", tr("not on the map", "нет на карте")),
    }
}

impl SignalWindow {
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, roots: Option<&Roots>, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let (Some(roots), Some(doc)) = (roots, ed.doc.as_ref()) else { return };
        let before = doc.traffic.signal_routes.clone();
        let mut routes = before.clone();
        // the one object selected, if it is one; what is selected, as pieces
        let sel_object = match ed.selection().as_slice() {
            [Sel::Object(k, id)] => crate::selection::any_object(doc, *k, *id).map(|o| (*id, lib.find(&o.file).map(|a| a.name.to_lowercase().contains("signal") || a.rel.to_lowercase().contains("sig")).unwrap_or(false))),
            _ => None,
        };
        let pieces = crate::signals::pieces_of(doc, &ed.selection(), roots);
        // (a signal selected in the view opens its first route)
        if let Some((id, _)) = sel_object {
            if self.current.map(|i| routes.get(i).map(|r| r.signal.0 != id && r.dist_signal != Some(id)).unwrap_or(true)).unwrap_or(true) {
                if let Some(i) = routes.iter().position(|r| r.signal.0 == id) {
                    self.current = Some(i);
                }
            }
        }
        let mut show: Option<Vec<Sel>> = None;
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Traffic, tr("Signal routes", "Маршруты сигналов"))).id(egui::Id::new("signal routes")).open(&mut open).default_pos([330.0, 90.0]).default_size([480.0, 560.0]).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("Each railway signal clears when a train wants the track pieces of its route and none stands on them.", "Сигнал открывается, когда поезду нужны пути его маршрута и на них никто не стоит.")).small().color(pal().text_dim));
            ui.horizontal(|ui| {
                let can = sel_object.is_some();
                if ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::Add, tr("New route for the selected signal", "Новый маршрут для выбранного сигнала")))).on_disabled_hover_text(tr("Select a signal (one object) in the view first", "Сначала выберите в окне вида сигнал (один объект)")).clicked() {
                    if let Some((id, _)) = sel_object {
                        routes.push(SignalRoute { kind: 0, signal: (id, 1), ..Default::default() });
                        self.current = Some(routes.len() - 1);
                    }
                }
            });
            if let Some((_, false)) = sel_object {
                ui.label(RichText::new(tr("The selected object does not look like a signal (its name says nothing of one): check it is.", "Выбранный объект не похож на сигнал (в названии этого нет): проверьте.")).small().color(pal().warn));
            }
            ui.separator();
            egui::ScrollArea::vertical().id_salt("routes").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
                if routes.is_empty() {
                    ui.label(RichText::new(tr("No routes yet.", "Маршрутов пока нет.")).color(pal().text_dim));
                }
                for (i, r) in routes.iter().enumerate() {
                    let text = format!("{i}: {} · {} {}{}", name_of(ed, lib, r.signal.0), r.entries.len(), tr("pieces", "участков"), r.speed_limit.map(|v| format!(" · {v:.0} km/h")).unwrap_or_default());
                    if ui.selectable_label(self.current == Some(i), text).clicked() {
                        self.current = Some(i);
                        if let Some(d) = ed.doc.as_ref() {
                            show = Some(crate::signals::selection_of(d, r));
                        }
                    }
                }
            });
            ui.separator();
            let Some(i) = self.current.filter(|i| *i < routes.len()) else { return };
            let mut remove = false;
            {
                let r = &mut routes[i];
                ui.label(RichText::new(format!("{} {i}", tr("Route", "Маршрут"))).strong().color(pal().accent));
                egui::Grid::new("route").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                    prop_row(ui, tr("Signal", "Сигнал"), |ui| {
                        ui.label(name_of(ed, lib, r.signal.0));
                        if let Some((id, _)) = sel_object.filter(|s| s.0 != r.signal.0) {
                            if ui.small_button(icons::labelled(Icon::Dropper, tr("the selected", "выбранный"))).clicked() {
                                r.signal.0 = id;
                            }
                        }
                    });
                    prop_row(ui, tr("Aspect", "Показание"), |ui| {
                        ui.add(egui::DragValue::new(&mut r.signal.1).range(0..=9)).on_hover_text(tr("What the signal shows when this route is clear: 1 proceed, 2 proceed slowly (with a speed limit)…", "Что показывает сигнал, когда маршрут свободен: 1 — ехать, 2 — ехать медленно (с ограничением)…"));
                    });
                    for (label, v) in [(tr("Distant signal", "Предсигнал"), &mut r.dist_signal), (tr("Next signal", "Следующий сигнал"), &mut r.next_signal)] {
                        prop_row(ui, label, |ui| {
                            ui.label(v.map(|id| name_of(ed, lib, id)).unwrap_or_else(|| tr("none", "нет").to_string()));
                            if let Some((id, _)) = sel_object.filter(|s| Some(s.0) != *v && s.0 != r.signal.0) {
                                if ui.small_button(icons::labelled(Icon::Dropper, tr("the selected", "выбранный"))).clicked() {
                                    *v = Some(id);
                                }
                            }
                            if v.is_some() && icons::small(ui, Icon::Close, tr("None", "Нет")).clicked() {
                                *v = None;
                            }
                        });
                    }
                    prop_row(ui, tr("Speed limit", "Ограничение"), |ui| {
                        let mut on = r.speed_limit.is_some();
                        if ui.checkbox(&mut on, "").changed() {
                            r.speed_limit = on.then_some(40.0);
                            r.signal.1 = if on { 2 } else { 1 };
                        }
                        if let Some(v) = &mut r.speed_limit {
                            ui.add(egui::DragValue::new(v).range(5.0..=300.0).suffix(" km/h"));
                        }
                    });
                    prop_row(ui, tr("Kind", "Вид"), |ui| {
                        ui.add(egui::DragValue::new(&mut r.kind).range(0..=9)).on_hover_text(tr("OMSI's route kind (the stock maps have 0 and 1)", "Вид маршрута OMSI (на стоковых картах 0 и 1)"));
                    });
                });
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{}: {}", tr("Track pieces", "Участки пути"), r.entries.len())).strong());
                ui.horizontal_wrapped(|ui| {
                    let label = format!("{} ({})", tr("Take the selected track", "Взять выбранные пути"), pieces.len());
                    if ui.add_enabled(!pieces.is_empty(), egui::Button::new(label)).on_hover_text(tr("Every rail path of the selected tracks and switches (double-click a track: all of it); replaces the pieces", "Все рельсовые пути выбранных рельсов и стрелок (двойной клик по пути — весь); заменяет участки")).clicked() {
                        r.entries = pieces.iter().map(|(id, p)| [*id, *p, 0, 0]).collect();
                    }
                    if ui.add_enabled(!pieces.is_empty(), egui::Button::new(icons::labelled(Icon::Add, tr("Add them", "Добавить их")))).clicked() {
                        for (id, p) in &pieces {
                            if !r.entries.iter().any(|e| e[0] == *id && e[1] == *p) {
                                r.entries.push([*id, *p, 0, 0]);
                            }
                        }
                    }
                    if ui.button(icons::labelled(Icon::Focus, tr("Show", "Показать"))).on_hover_text(tr("Select the signal and its pieces in the view", "Выделить сигнал и его участки в окне вида")).clicked() {
                        if let Some(d) = ed.doc.as_ref() {
                            show = Some(crate::signals::selection_of(d, r));
                        }
                    }
                });
                ui.add_space(6.0);
                if ui.button(icons::labelled(Icon::Delete, tr("Delete the route", "Удалить маршрут"))).clicked() {
                    remove = true;
                }
            }
            if remove {
                routes.remove(i);
                self.current = None;
            }
        });
        self.open = open;
        if let Some(v) = show {
            if !v.is_empty() {
                ed.set_selection(v);
                actions.push(Action::FocusSelection);
            }
        }
        if routes != before {
            // (the pieces' places among their tiles' paths, as the game counts them)
            if let Some(doc) = ed.doc.as_ref() {
                crate::signals::reindex(&mut routes, doc, roots);
            }
            let Some(doc) = ed.doc.as_mut() else { return };
            let old = doc.traffic.clone();
            doc.traffic.signal_routes = routes;
            doc.traffic.touch(File::SignalRoutes);
            ed.history.push_snapshot(tr("Signal routes", "Маршруты сигналов"), crate::history::Snapshot { traffic: Some(old), ..Default::default() });
        }
    }
}
