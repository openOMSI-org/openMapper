//! Every trip of the map in one list, whichever line or tour drives it (#71): a trip no
//! tour drives stays there to be edited, used or deleted.

use super::*;

/// How many departures of the tours drive trip `name`.
fn departures(tt: &Timetable, name: &str) -> usize {
    tt.lines.iter().flat_map(|l| l.tours.iter()).flat_map(|t| t.trips.iter()).filter(|x| x.trip.eq_ignore_ascii_case(name)).count()
}

impl TimetableUi {
    pub(super) fn show_all_trips(&mut self, tt: &Timetable) {
        self.all_trips = true;
        self.line = None;
        self.tour = None;
        self.picking = false;
        self.way_edit = false;
        if self.trip.as_ref().map(|t| tt.trip(t).is_none()).unwrap_or(false) {
            self.select_trip(None);
        }
    }

    pub(super) fn all_trips_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document, toasts: &mut Vec<crate::editor::Toast>, actions: &mut Vec<Action>) {
        heading(ui, tr("All trips", "Все рейсы"));
        ui.label(RichText::new(tr("Every trip of the map, whichever line shows it or tour drives it. A trip no tour drives is kept, and can be edited, given to a tour or deleted here.", "Все рейсы карты, какая бы линия их ни показывала и какой бы выход ни ездил. Рейс, по которому не ездит ни один выход, сохраняется: его можно править, дать выходу или удалить здесь.")).small().color(TEXT_DIM));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.trip_filter).hint_text(tr("Search", "Поиск")).desired_width(180.0));
            if ui.button(icons::labelled(Icon::Add, tr("New trip", "Новый рейс"))).clicked() {
                let name = doc.timetable.add_trip("", tr("Trip", "Рейс"), Vec::new());
                self.select_trip(Some(name));
                self.picking = true;
            }
        });
        let f = self.trip_filter.to_lowercase();
        let mut names: Vec<(String, String, usize, usize)> = doc
            .timetable
            .trips
            .iter()
            .filter(|t| f.is_empty() || t.name.to_lowercase().contains(&f) || t.line.to_lowercase().contains(&f) || t.terminus.to_lowercase().contains(&f))
            .map(|t| (t.name.clone(), t.line.trim().to_string(), t.stations.len().max(t.stations_legacy.len()), departures(&doc.timetable, &t.name)))
            .collect();
        names.sort_by_key(|x| x.0.to_lowercase());
        let unused = doc.timetable.trips.iter().filter(|t| departures(&doc.timetable, &t.name) == 0).count();
        ui.label(RichText::new(format!("{} {} · {} {}", doc.timetable.trips.len(), tr("trips", "рейсов"), unused, tr("driven by no tour", "без выходов"))).small().color(TEXT_DIM));
        egui::ScrollArea::vertical().id_salt("tt all trips").max_height(240.0).show(ui, |ui| {
            egui::Grid::new("tt all trips grid").num_columns(4).striped(true).spacing(Vec2::new(10.0, 2.0)).show(ui, |ui| {
                for (name, line, stops, deps) in &names {
                    let on = self.trip.as_deref().map(|x| x.eq_ignore_ascii_case(name)).unwrap_or(false);
                    let warn = self.problems.contains_key(&lower(name));
                    if ui.add(egui::Button::selectable(on, icons::labelled(if warn { Icon::Warning } else { Icon::Route }, name))).clicked() && !on {
                        self.select_trip(Some(name.clone()));
                    }
                    ui.label(RichText::new(if line.is_empty() { "–".to_string() } else { format!("{} {line}", tr("line", "линия")) }).color(TEXT_DIM));
                    ui.label(RichText::new(format!("{stops} {}", tr("stops", "ост."))).color(TEXT_DIM));
                    if *deps == 0 {
                        ui.label(RichText::new(tr("no tour", "нет выходов")).color(WARN));
                    } else {
                        ui.label(RichText::new(format!("{deps} {}", tr("departures", "отпр."))).color(TEXT_DIM));
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(t) = self.trip.clone() {
            if doc.timetable.trip(&t).is_some() {
                ui.add_space(4.0);
                let line = doc.timetable.trip(&t).map(|x| x.line.trim().to_string()).unwrap_or_default();
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.trip_panel(ui, doc, &line, &t, toasts, actions);
                });
            } else {
                self.trip = None;
            }
        }
    }
}
