//! The Depot files tab of the Traffic and AI window: the map's `.hof` files, made from its
//! lines with a click, their destinations, stop names and IBIS routes edited here.

use egui::{RichText, Vec2};

use crate::doc::Document;
use crate::hof::{self, Hof};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::traffic::File;
use crate::theme::pal;


#[derive(Clone, Copy, PartialEq, Default)]
enum Part {
    #[default]
    Termini,
    Stops,
    Routes,
}

#[derive(Default)]
pub struct HofUi {
    pick: usize,
    part: Part,
    filter: String,
    /// The file name being typed (renamed once it is a good one).
    file_name: Option<String>,
}

fn small_button(ui: &mut egui::Ui, icon: Icon, hint: &str) -> bool {
    ui.add(egui::Button::new(icon.text(16.0)).frame(false).min_size(Vec2::new(20.0, 20.0))).on_hover_text(hint).clicked()
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(pal().text_dim));
}

/// A file name for a new depot file: the map's folder's, else with a number.
fn new_file_name(doc: &Document) -> String {
    let base = doc.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Depot".into());
    let taken = |n: &str| doc.traffic.depots.iter().any(|h| h.file.eq_ignore_ascii_case(n));
    if !taken(&base) {
        return base;
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap()
}

/// A new depot file made from the map's lines (`OPENMAPPER_TRAFFIC=depots,make`): how
/// many destinations, stops and routes it got.
pub fn make_from_lines(doc: &mut Document) -> (usize, usize, usize) {
    let mut h = Hof::new(&new_file_name(doc));
    let n = h.fill_from(&doc.timetable);
    doc.traffic.depots.push(h);
    doc.traffic.touch(File::Depots);
    n
}

/// A depot file called `name` made from the map's lines (empty: named after the map's
/// folder); one of the name the map has gets what the lines need. Its name.
pub fn make_named(doc: &mut Document, name: &str) -> String {
    let folder = doc.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Depot".into());
    let file = if name.trim().is_empty() { folder } else { name.trim().to_string() };
    if let Some(h) = doc.traffic.depots.iter_mut().find(|h| h.file.eq_ignore_ascii_case(&file)) {
        h.fill_from(&doc.timetable);
    } else {
        let mut h = Hof::new(&file);
        h.fill_from(&doc.timetable);
        doc.traffic.depots.push(h);
    }
    doc.traffic.touch(File::Depots);
    file
}

/// The map's own depot files a depot group uses get what the lines need (lines added from
/// OpenStreetMap).
pub fn follow_lines(doc: &mut Document) {
    let used: Vec<String> = doc.traffic.depots().filter_map(|g| g.hof.clone()).collect();
    let tt = &doc.timetable;
    let mut changed = false;
    for h in doc.traffic.depots.iter_mut().filter(|h| used.iter().any(|u| u.eq_ignore_ascii_case(&h.file))) {
        changed |= h.fill_from(tt) != (0, 0, 0);
    }
    if changed {
        doc.traffic.touch(File::Depots);
    }
}

/// What the map's lines need that the file lacks: destinations, stops, routes.
fn missing(h: &Hof, doc: &Document) -> (usize, usize, usize) {
    h.clone().fill_from(&doc.timetable)
}

impl HofUi {
    pub fn part(&self) -> &'static str {
        match self.part {
            Part::Termini => "termini",
            Part::Stops => "stops",
            Part::Routes => "routes",
        }
    }

    pub fn show_part(&mut self, part: &str) {
        self.part = match part {
            "stops" => Part::Stops,
            "routes" => Part::Routes,
            _ => Part::Termini,
        };
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, doc: &mut Document) {
        let before = doc.traffic.depots.clone();
        self.contents(ui, doc);
        if doc.traffic.depots != before {
            doc.traffic.touch(File::Depots);
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, doc: &mut Document) {
        hint(ui, tr(
            "A depot file tells the buses of the map their destinations, the names their displays and announcements give the stops, and the routes the IBIS knows (the line, the route's code, its stops). OMSI 2 finds it in the map's folder, openOMSI in the content's HOFs folder: saving puts it in both.",
            "Файл депо сообщает автобусам карты их конечные, имена остановок для табло и объявлений и маршруты, которые знает IBIS (линия, код маршрута, его остановки). OMSI 2 ищет его в папке карты, openOMSI в папке HOFs контента: при сохранении он попадает в обе.",
        ));
        ui.add_space(4.0);
        let has_trips = doc.timetable.trips.iter().any(|t| t.stations.len() >= 2);
        if doc.traffic.depots.is_empty() {
            ui.label(RichText::new(tr("The map has no depot file.", "У карты нет файла депо.")).color(pal().warn));
            ui.horizontal(|ui| {
                if ui.add_enabled(has_trips, egui::Button::new(icons::labelled(Icon::Add, tr("Make one from the lines", "Создать по линиям")))).on_disabled_hover_text(tr("The map has no trips yet (Map > Lines and timetable)", "У карты ещё нет рейсов (Карта > Линии и расписание)")).clicked() {
                    self.add(doc, true);
                }
                if ui.button(icons::labelled(Icon::NewFile, tr("Empty one", "Пустой"))).clicked() {
                    self.add(doc, false);
                }
            });
            return;
        }
        self.pick = self.pick.min(doc.traffic.depots.len() - 1);
        ui.horizontal(|ui| {
            let cur = doc.traffic.depots[self.pick].file_name();
            egui::ComboBox::from_id_salt("hof pick").selected_text(cur).width(200.0).show_ui(ui, |ui| {
                for (i, h) in doc.traffic.depots.iter().enumerate() {
                    if ui.selectable_label(i == self.pick, h.file_name()).clicked() {
                        self.pick = i;
                        self.file_name = None;
                    }
                }
            });
            if ui.button(icons::labelled(Icon::Add, tr("New", "Новый"))).on_hover_text(tr("A new depot file from the map's lines", "Новый файл депо по линиям карты")).clicked() {
                self.add(doc, has_trips);
            }
            if small_button(ui, Icon::Delete, tr("Delete this depot file", "Удалить этот файл депо")) {
                doc.traffic.depots.remove(self.pick);
                self.pick = self.pick.saturating_sub(1);
                self.file_name = None;
            }
        });
        if doc.traffic.depots.is_empty() {
            return;
        }
        let i = self.pick;
        // what the lines need that is not there
        let need = missing(&doc.traffic.depots[i], doc);
        if need != (0, 0, 0) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} {} {}, {} {}, {} {}", tr("The lines need", "Линиям не хватает:"), need.0, tr("destinations", "конечных"), need.1, tr("stops", "остановок"), need.2, tr("routes", "маршрутов"))).color(pal().warn));
                if ui.button(icons::labelled(Icon::Add, tr("Add them", "Добавить"))).on_hover_text(tr("What is there stays as it is", "Существующее не меняется")).clicked() {
                    doc.traffic.depots[i].fill_from(&doc.timetable);
                }
            });
        }
        // where it is used: the AI's depot groups name it (the player's bus comes from the
        // one Map settings names)
        let file = doc.traffic.depots[i].file.clone();
        ui.horizontal_wrapped(|ui| {
            let users: Vec<String> = doc.traffic.depots().filter(|g| g.hof.as_deref().unwrap_or("").eq_ignore_ascii_case(&file)).map(|g| g.name.clone()).collect();
            if users.is_empty() {
                ui.label(RichText::new(tr("No depot group uses it.", "Ни одна группа депо его не использует.")).color(pal().warn));
            } else {
                ui.label(RichText::new(format!("{} {}", tr("Used by the depot", "Используется депо"), users.join(", "))).color(pal().text_dim));
            }
            let others: Vec<usize> = doc.traffic.groups.iter().enumerate().filter(|(_, g)| g.is_depot && !g.hof.as_deref().unwrap_or("").eq_ignore_ascii_case(&file)).map(|(k, _)| k).collect();
            if !others.is_empty() && ui.button(icons::labelled(Icon::DoneAll, tr("Use it for every depot", "Для всех депо"))).on_hover_text(tr("The AI's depot groups (Vehicle groups tab) take their buses' destinations from it", "Группы депо ИИ (вкладка «Группы машин») берут из него конечные своих автобусов")).clicked() {
                for k in others {
                    doc.traffic.groups[k].hof = Some(file.clone());
                }
                doc.traffic.touch(File::AiLists);
            }
        });
        ui.add_space(4.0);
        egui::Grid::new("hof head").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
            ui.label(tr("File", "Файл"));
            ui.horizontal(|ui| {
                let typed = self.file_name.get_or_insert_with(|| file.clone());
                ui.add(egui::TextEdit::singleline(typed).desired_width(180.0));
                ui.label(".hof");
                let t = typed.trim().to_string();
                let ok = crate::timetable::valid_file_name(&t) && !doc.traffic.depots.iter().enumerate().any(|(k, h)| k != i && h.file.eq_ignore_ascii_case(&t));
                if t != file {
                    if ok {
                        if ui.button(icons::labelled(Icon::Edit, tr("Rename", "Переименовать"))).clicked() {
                            for g in doc.traffic.groups.iter_mut().filter(|g| g.hof.as_deref().unwrap_or("").eq_ignore_ascii_case(&file)) {
                                g.hof = Some(t.clone());
                            }
                            doc.traffic.touch(File::AiLists);
                            doc.traffic.depots[i].file = t;
                        }
                    } else {
                        ui.label(RichText::new(tr("not a usable name", "недопустимое имя")).color(pal().warn));
                    }
                }
            });
            ui.end_row();
            let h = &mut doc.traffic.depots[i];
            ui.label(tr("Name", "Название"));
            ui.add(egui::TextEdit::singleline(&mut h.name).desired_width(220.0));
            ui.end_row();
            ui.label(tr("Service trip", "Служебный рейс"));
            ui.add(egui::TextEdit::singleline(&mut h.service_trip).desired_width(220.0)).on_hover_text(tr("The destination a bus shows on its way to and from the depot", "Конечная, которую автобус показывает по пути в депо и из него"));
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let h = &doc.traffic.depots[i];
            for (p, icon, label, n) in [(Part::Termini, Icon::Signpost, tr("Destinations", "Конечные"), h.termini.len()), (Part::Stops, Icon::Bus, tr("Stops", "Остановки"), h.stops.len()), (Part::Routes, Icon::Route, tr("Routes", "Маршруты"), h.routes.len())] {
                if ui.add(egui::Button::selectable(self.part == p, icons::labelled(icon, &format!("{label} ({n})")))).clicked() {
                    self.part = p;
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(tr("Search", "Поиск")).desired_width(120.0));
        });
        ui.separator();
        let f = self.filter.trim().to_lowercase();
        let h = &mut doc.traffic.depots[i];
        match self.part {
            Part::Termini => termini(ui, h, &f),
            Part::Stops => stops(ui, h, &f),
            Part::Routes => routes(ui, h, &f),
        }
    }

    fn add(&mut self, doc: &mut Document, from_lines: bool) {
        let mut h = Hof::new(&new_file_name(doc));
        if from_lines {
            h.fill_from(&doc.timetable);
        }
        doc.traffic.depots.push(h);
        self.pick = doc.traffic.depots.len() - 1;
        self.file_name = None;
    }
}

fn texts(ui: &mut egui::Ui, id: (&str, usize), strings: &mut [String], label: fn(usize) -> String) {
    egui::Grid::new(id).num_columns(2).spacing(Vec2::new(8.0, 2.0)).show(ui, |ui| {
        for (k, s) in strings.iter_mut().enumerate() {
            ui.label(RichText::new(label(k)).small().color(pal().text_dim));
            ui.add(egui::TextEdit::singleline(s).font(egui::TextStyle::Monospace).desired_width(200.0));
            ui.end_row();
        }
    });
}

fn termini(ui: &mut egui::Ui, h: &mut Hof, f: &str) {
    let mut remove = None;
    let codes: Vec<i32> = h.termini.iter().map(|t| t.code).collect();
    for (k, t) in h.termini.iter_mut().enumerate() {
        if !f.is_empty() && !t.ident.to_lowercase().contains(f) && !t.code.to_string().contains(f) {
            continue;
        }
        let twice = codes.iter().filter(|c| **c == t.code).count() > 1;
        let title = format!("{:>3}   {}{}", t.code, t.ident, if t.all_exit { "  ·  ".to_string() + tr("nobody gets on", "без посадки") } else { String::new() });
        egui::CollapsingHeader::new(RichText::new(title).color(if twice { pal().warn } else { ui.visuals().text_color() })).id_salt(("hof terminus", k)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Code", "Код"));
                ui.add(egui::DragValue::new(&mut t.code).range(0..=999));
                if twice {
                    ui.label(RichText::new(tr("another destination has this code", "этот код есть у другой конечной")).color(pal().warn));
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr("Name", "Название"));
                ui.add(egui::TextEdit::singleline(&mut t.ident).desired_width(200.0)).on_hover_text(tr("As a trip's terminus is named in the timetable: the stop where everybody gets off", "Как названа конечная рейса в расписании: остановка, где все выходят"));
            });
            ui.checkbox(&mut t.all_exit, tr("Nobody gets on (service trip, empty sign)", "Без посадки (служебный рейс, пустой указатель)"));
            texts(ui, ("hof terminus texts", k), &mut t.strings, hof::terminus_label);
            if ui.button(icons::labelled(Icon::Delete, tr("Delete destination", "Удалить конечную"))).clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        h.termini.remove(k);
    }
    ui.add_space(4.0);
    if ui.button(icons::labelled(Icon::Add, tr("Add destination", "Добавить конечную"))).clicked() {
        let t = hof::terminus_for(h, h.free_code(), tr("New destination", "Новая конечная"));
        h.termini.push(t);
    }
}

fn stops(ui: &mut egui::Ui, h: &mut Hof, f: &str) {
    let mut remove = None;
    for (k, s) in h.stops.iter_mut().enumerate() {
        if !f.is_empty() && !s.ident.to_lowercase().contains(f) {
            continue;
        }
        egui::CollapsingHeader::new(&s.ident).id_salt(("hof stop", k)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Name", "Название"));
                ui.add(egui::TextEdit::singleline(&mut s.ident).desired_width(200.0)).on_hover_text(tr("As the timetable names the stop", "Как остановка названа в расписании"));
            });
            texts(ui, ("hof stop texts", k), &mut s.strings, hof::stop_label);
            if ui.button(icons::labelled(Icon::Delete, tr("Delete stop", "Удалить остановку"))).clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        h.stops.remove(k);
    }
    ui.add_space(4.0);
    if ui.button(icons::labelled(Icon::Add, tr("Add stop", "Добавить остановку"))).clicked() {
        let s = hof::stop_for(h, tr("New stop", "Новая остановка"));
        h.stops.push(s);
    }
}

fn routes(ui: &mut egui::Ui, h: &mut Hof, f: &str) {
    let termini: Vec<(String, String)> = h.termini.iter().map(|t| (t.code.to_string(), t.ident.clone())).collect();
    let known: Vec<String> = h.stops.iter().map(|s| s.ident.trim().to_lowercase()).collect();
    let stop_names: Vec<String> = h.stops.iter().map(|s| s.ident.clone()).collect();
    let mut remove = None;
    for (k, r) in h.routes.iter_mut().enumerate() {
        let to = termini.iter().find(|(c, _)| *c == r.terminus.trim()).map(|(_, n)| n.clone());
        if !f.is_empty() && !r.line.to_lowercase().contains(f) && !r.code.contains(f) && !r.name.to_lowercase().contains(f) && !r.stops.iter().any(|s| s.to_lowercase().contains(f)) {
            continue;
        }
        let title = format!("{:>5}   {} {}  »  {}", r.code, tr("line", "линия"), r.line.trim(), to.clone().unwrap_or_else(|| "?".into()));
        egui::CollapsingHeader::new(RichText::new(title).color(if to.is_none() { pal().warn } else { ui.visuals().text_color() })).id_salt(("hof route", k)).default_open(k == 0 && f.is_empty()).show(ui, |ui| {
            egui::Grid::new(("hof route grid", k)).num_columns(2).spacing(Vec2::new(8.0, 3.0)).show(ui, |ui| {
                ui.label(tr("Code", "Код"));
                ui.add(egui::TextEdit::singleline(&mut r.code).desired_width(80.0)).on_hover_text(tr("What the driver types: the line's number and two digits (7601: line 76, route 01)", "Что набирает водитель: номер линии и две цифры (7601: линия 76, маршрут 01)"));
                ui.end_row();
                ui.label(tr("Line", "Линия"));
                ui.add(egui::TextEdit::singleline(&mut r.line).desired_width(80.0));
                ui.end_row();
                ui.label(tr("IBIS text", "Текст IBIS"));
                ui.add(egui::TextEdit::singleline(&mut r.name).font(egui::TextStyle::Monospace).desired_width(220.0));
                ui.end_row();
                ui.label(tr("Destination", "Конечная"));
                egui::ComboBox::from_id_salt(("hof route to", k)).selected_text(to.clone().unwrap_or_else(|| format!("? ({})", r.terminus))).width(200.0).show_ui(ui, |ui| {
                    for (c, n) in &termini {
                        if ui.selectable_label(*c == r.terminus.trim(), format!("{c:>3}  {n}")).clicked() {
                            r.terminus = c.clone();
                        }
                    }
                });
                ui.end_row();
            });
            ui.label(RichText::new(tr("Stops in order", "Остановки по порядку")).strong());
            let mut op = None;
            let n = r.stops.len();
            egui::Grid::new(("hof route stops", k)).num_columns(3).spacing(Vec2::new(6.0, 2.0)).show(ui, |ui| {
                for (j, s) in r.stops.iter().enumerate() {
                    ui.label(RichText::new(format!("{:>2}.", j + 1)).monospace().color(pal().text_dim));
                    let unknown = !known.contains(&s.trim().to_lowercase());
                    let l = ui.label(if unknown { RichText::new(s).color(pal().warn) } else { RichText::new(s) });
                    if unknown {
                        l.on_hover_text(tr("Not in the Stops list: the displays have no texts for it", "Нет в списке остановок: у табло нет для неё текстов"));
                    }
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(j > 0, |ui| {
                            if small_button(ui, Icon::Raise, tr("Move up", "Выше")) {
                                op = Some((j, -1));
                            }
                        });
                        ui.add_enabled_ui(j + 1 < n, |ui| {
                            if small_button(ui, Icon::Lower, tr("Move down", "Ниже")) {
                                op = Some((j, 1));
                            }
                        });
                        if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                            op = Some((j, 0));
                        }
                    });
                    ui.end_row();
                }
            });
            match op {
                Some((j, 0)) => {
                    r.stops.remove(j);
                }
                Some((j, d)) => r.stops.swap(j, (j as i32 + d) as usize),
                None => {}
            }
            egui::ComboBox::from_id_salt(("hof route add", k)).selected_text(tr("Add a stop…", "Добавить остановку…")).width(200.0).height(320.0).show_ui(ui, |ui| {
                for s in &stop_names {
                    if ui.selectable_label(false, s).clicked() {
                        r.stops.push(s.clone());
                    }
                }
            });
            if ui.button(icons::labelled(Icon::Delete, tr("Delete route", "Удалить маршрут"))).clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        h.routes.remove(k);
    }
    ui.add_space(4.0);
    if ui.button(icons::labelled(Icon::Add, tr("Add route", "Добавить маршрут"))).clicked() {
        let code = h.routes.iter().filter_map(|r| r.code.trim().parse::<u32>().ok()).max().map(|c| c + 1).unwrap_or(101);
        h.routes.push(hof::Route { code: code.to_string(), line: (code / 100).to_string(), ..Default::default() });
    }
    if h.routes.is_empty() {
        hint(ui, tr("No routes: the IBIS knows the destinations' codes only. Add them from the lines above.", "Маршрутов нет: IBIS знает только коды конечных. Добавьте их по линиям выше."));
    }
}
