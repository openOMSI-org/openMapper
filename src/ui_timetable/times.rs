//! A trip's time profiles (#116): the trip's time and the time from stop to stop of each,
//! and how each stop is served (#71: OMSI 2's five ways).

use super::*;
use crate::timetable::profile;
use crate::theme::pal;

/// A profile for a trip that has none (OMSI's editor names it so).
fn standard(minutes: f32) -> omsi_timetable::TripProfile {
    omsi_timetable::TripProfile { name: "standard".into(), factor: minutes, ..Default::default() }
}

impl TimetableUi {
    /// The trip's profiles, the one shown with its time from stop to stop (`legs`: the metres
    /// between the stops; `route_len`: the way's length, to guess the time from).
    pub(super) fn times_section(&mut self, ui: &mut egui::Ui, tt: &mut Timetable, name: &str, stations: &[i64], legs: &[f64], route_len: Option<f64>) {
        let Some(t) = tt.trip(name).cloned() else { return };
        let n = stations.len();
        let count = t.profiles.len().max(1);
        if self.profile >= count {
            self.profile = 0;
        }
        let pi = self.profile;
        let mut p = t.profiles.get(pi).cloned().unwrap_or_else(|| standard(10.0));
        if !(p.factor > 0.0) {
            p.factor = 10.0;
        }
        let before = p.clone();
        let mut add = false;
        let mut remove = false;
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Time profile", "Профиль времени")).on_hover_text(tr("A trip can have several profiles (rush hour, night…): each tour's departure says which one it runs to", "У рейса может быть несколько профилей (час пик, ночь…): каждое отправление выхода указывает, по какому оно идёт"));
            egui::ComboBox::from_id_salt(("tt profile", lower(name))).selected_text(format!("{} · {}", p.name.trim(), mmss(p.factor))).width(170.0).show_ui(ui, |ui| {
                for (k, x) in t.profiles.iter().enumerate() {
                    if ui.selectable_label(k == pi, format!("{} · {}", x.name.trim(), mmss(x.factor))).clicked() {
                        self.profile = k;
                    }
                }
            });
            ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(90.0)).on_hover_text(tr("The profile's name", "Имя профиля"));
            if small_button(ui, Icon::Copy, tr("A new profile, a copy of this one", "Новый профиль, копия этого")) {
                add = true;
            }
            if count > 1 && small_button(ui, Icon::Delete, tr("Delete this profile (its departures run to the first)", "Удалить этот профиль (его отправления пойдут по первому)")) {
                remove = true;
            }
        });
        // the trip's time
        ui.horizontal(|ui| {
            ui.label(tr("Trip time", "Время в пути"));
            let mut m = p.factor;
            if ui.add(egui::DragValue::new(&mut m).range(0.5..=600.0).speed(0.25).max_decimals(2).suffix(tr(" min", " мин"))).changed() {
                profile::set_total(&mut p, n, m);
            }
            if let Some(len) = route_len {
                let guess = ((len / 1000.0 / BUS_SPEED * 60.0) + 0.4 * n as f64).ceil().max(1.0) as f32;
                if ui.button(format!("{} {guess:.0}", tr("Guess:", "Оценка:"))).on_hover_text(tr("From the route's length at the pace of a town bus", "По длине пути и темпу городского автобуса")).clicked() {
                    profile::set_total(&mut p, n, guess);
                }
            }
        });
        // from stop to stop
        if n >= 2 {
            let times = timetable::stop_times(n, legs, &p);
            let mut set: Option<(usize, Option<f32>)> = None;
            let mut leg: Option<(usize, f32)> = None;
            let mut clear = false;
            egui::CollapsingHeader::new(icons::labelled(Icon::Schedule, tr("Times from stop to stop", "Время от остановки до остановки"))).id_salt(("tt times", lower(name))).default_open(true).show(ui, |ui| {
                ui.label(RichText::new(tr("The time from the stop before can be set at each stop: the times after it move along, the trip's time with them. Times not set follow the distance between the fixed ones; at a fixed stop a bus that is early waits.", "У каждой остановки можно задать время от предыдущей: следующие времена сдвигаются, а с ними и время рейса. Незаданные времена следуют расстоянию между закреплёнными; на закреплённой остановке автобус, пришедший раньше, ждёт.")).small().color(pal().text_dim));
                egui::Grid::new(("tt stop times", lower(name))).num_columns(4).striped(true).spacing(Vec2::new(12.0, 3.0)).show(ui, |ui| {
                    ui.label(RichText::new(tr("Stop", "Остановка")).small().color(pal().text_dim));
                    ui.label(RichText::new(tr("From the stop before", "От предыдущей")).small().color(pal().text_dim));
                    ui.label(RichText::new(tr("After departure", "После отправления")).small().color(pal().text_dim));
                    ui.label("");
                    ui.end_row();
                    for (i, (a, d, fixed)) in times.iter().enumerate() {
                        let label = format!("{:>2}. {}", i + 1, self.stop_name(tt, stations[i]));
                        ui.allocate_ui_with_layout(Vec2::new(220.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width(220.0);
                            ui.add(egui::Label::new(label).truncate());
                        });
                        let at = if i + 1 == n { *a } else { *d };
                        if i > 0 {
                            let mut m = *a - times[i - 1].1;
                            if ui.add(egui::DragValue::new(&mut m).range(0.0..=600.0).speed(0.05).max_decimals(2).suffix(tr(" min", " мин"))).on_hover_text(tr("Minutes from the stop before to this one", "Минуты от предыдущей остановки до этой")).changed() {
                                leg = Some((i, m));
                            }
                        } else {
                            ui.label("");
                        }
                        ui.label(RichText::new(format!("+{}", mmss(at))).color(if *fixed { pal().text_strong } else { pal().text_dim }));
                        if i > 0 && i + 1 < n && check(ui, *fixed, tr("fixed", "закреплено")) {
                            set = Some((i, if *fixed { None } else { Some((at * 2.0).round() / 2.0) }));
                        } else if i == 0 || i + 1 == n {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
                if times.iter().skip(1).take(n.saturating_sub(2)).any(|t| t.2) && ui.button(tr("Spread all by distance", "Распределить по расстоянию")).clicked() {
                    clear = true;
                }
            });
            if clear {
                p.man_arr_time.clear();
                p.man_dep_time.clear();
            }
            if let Some((i, m)) = set {
                timetable::set_stop_time(&mut p, n, i, m);
            }
            if let Some((i, m)) = leg {
                profile::set_leg(&mut p, n, legs, i, m);
            }
        }
        if add {
            if let Some(k) = tt.add_profile(name, pi) {
                self.profile = k;
            }
            return;
        }
        if remove {
            tt.remove_profile(name, pi);
            self.profile = 0;
            return;
        }
        if p.name.trim().is_empty() {
            p.name = before.name.clone();
        }
        if p != before {
            let tm = tt.trip_mut(name).unwrap();
            if tm.profiles.is_empty() {
                tm.profiles.push(standard(10.0));
            }
            if let Some(slot) = tm.profiles.get_mut(pi) {
                *slot = p;
            }
            tt.touch_trip(name);
        }
    }

    /// How the bus serves each stop on this trip (`[profile_otherstopping]`, alike in all its
    /// profiles).
    pub(super) fn serve_section(&mut self, ui: &mut egui::Ui, tt: &mut Timetable, name: &str, stations: &[i64]) {
        let Some(t) = tt.trip(name).cloned() else { return };
        let n = stations.len();
        if n < 2 {
            return;
        }
        let p = t.profiles.get(self.profile).or(t.profiles.first()).cloned().unwrap_or_default();
        let mut serve: Option<(usize, i32)> = None;
        egui::CollapsingHeader::new(icons::labelled(Icon::Bus, tr("How the stops are served", "Как обслуживаются остановки"))).id_salt(("tt serve", lower(name))).default_open(false).show(ui, |ui| {
            ui.label(RichText::new(tr("As in OMSI 2's editor. Hover a choice to see what the bus does.", "Как в редакторе OMSI 2. Наведите на вариант, чтобы увидеть, что делает автобус.")).small().color(pal().text_dim));
            egui::Grid::new(("tt serve grid", lower(name))).num_columns(2).striped(true).spacing(Vec2::new(12.0, 3.0)).show(ui, |ui| {
                for i in 0..n {
                    ui.allocate_ui_with_layout(Vec2::new(220.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(220.0);
                        ui.add(egui::Label::new(format!("{:>2}. {}", i + 1, self.stop_name(tt, stations[i]))).truncate());
                    });
                    let cur = p.other_stopping.iter().find(|x| x.0 == i as i32).map(|x| x.1).unwrap_or(0);
                    let mut v = cur;
                    egui::ComboBox::from_id_salt(("tt serve one", lower(name), i)).selected_text(crate::stops::StopKind::of_serve(cur).unwrap_or("?")).width(250.0).show_ui(ui, |ui| {
                        for k in crate::stops::StopKind::SERVE {
                            ui.selectable_value(&mut v, k, crate::stops::StopKind::of_serve(k).unwrap_or("?")).on_hover_text(crate::stops::StopKind::serve_hint(k));
                        }
                    })
                    .response
                    .on_hover_text(crate::stops::StopKind::serve_hint(cur));
                    if v != cur {
                        serve = Some((i, v));
                    }
                    ui.end_row();
                }
            });
        });
        if let Some((i, v)) = serve {
            let tm = tt.trip_mut(name).unwrap();
            if tm.profiles.is_empty() {
                tm.profiles.push(standard(10.0));
            }
            for p in &mut tm.profiles {
                p.other_stopping.retain(|x| x.0 != i as i32);
                if v != 0 {
                    p.other_stopping.push((i as i32, v));
                }
                p.other_stopping.sort_by_key(|x| x.0);
            }
            tt.touch_trip(name);
        }
    }
}
