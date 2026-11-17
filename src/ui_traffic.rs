//! The Traffic and AI window: how busy the roads and the stops are over the day
//! (`global.cfg`), the AI's vehicle groups and depots (`ailists.cfg`) and which of them drive
//! about on their own, and the parked cars and the people of the map.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::doc::Document;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::traffic::catalogue::{Catalogue, Item};
use crate::traffic::{self, File};
use crate::theme::pal;
use crate::theme;

const ROAD: Color32 = Color32::from_rgb(232, 122, 50);
const PEOPLE: Color32 = Color32::from_rgb(90, 170, 240);

#[derive(Clone, Copy, PartialEq, Default)]
enum Tab {
    #[default]
    Day,
    Groups,
    Lists,
    Depots,
}

#[derive(Default)]
pub struct TrafficUi {
    pub open: bool,
    tab: Tab,
    catalogue: Option<Catalogue>,
    group: Option<usize>,
    group_name: String,
    filter: String,
    map: Option<std::path::PathBuf>,
    /// The installation (for OMSI 2's own groups).
    roots: Vec<std::path::PathBuf>,
    stock: Option<Vec<omsi_map::AiGroup>>,
    hof: crate::ui_hof::HofUi,
}

/// Typical days to start a curve from: (hour, factor).
const QUIET: &[(f32, f32)] = &[(0.0, 0.1), (5.0, 0.1), (7.0, 0.8), (9.0, 0.6), (16.0, 0.8), (19.0, 0.5), (22.0, 0.2), (24.0, 0.1)];
const TOWN: &[(f32, f32)] = &[(0.0, 0.1), (4.0, 0.05), (6.0, 1.0), (7.0, 1.5), (8.0, 1.0), (10.0, 0.6), (15.0, 0.7), (16.0, 1.2), (17.0, 1.5), (19.0, 0.6), (22.0, 0.3), (24.0, 0.1)];
const FLAT: &[(f32, f32)] = &[(0.0, 1.0), (24.0, 1.0)];

fn small_button(ui: &mut egui::Ui, icon: Icon, hint: &str) -> bool {
    ui.add(egui::Button::new(icon.text(16.0)).frame(false).min_size(Vec2::new(20.0, 20.0))).on_hover_text(hint).clicked()
}

fn check(ui: &mut egui::Ui, on: bool, text: &str) -> bool {
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let size = Vec2::new(24.0 + galley.size().x, galley.size().y.max(20.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let vis = ui.style().interact(&resp);
    let col = if on { pal().accent } else { vis.fg_stroke.color };
    icons::paint(ui.painter(), if on { Icon::CheckOn } else { Icon::CheckOff }, egui::pos2(rect.left() + 9.0, rect.center().y), 20.0, col);
    ui.painter().galley(egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5), galley, vis.text_color());
    resp.clicked()
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(pal().text_dim));
}

/// A picker of catalogue items with a search field; the item chosen.
fn pick<'a>(ui: &mut egui::Ui, id: &str, label: &str, items: &'a [Item], filter: &mut String) -> Option<&'a Item> {
    let mut out = None;
    egui::ComboBox::from_id_salt(id).selected_text(label).width(220.0).height(360.0).show_ui(ui, |ui| {
        ui.add(egui::TextEdit::singleline(filter).hint_text(tr("Search", "Поиск")).desired_width(260.0));
        let f = filter.to_lowercase();
        for it in items.iter().filter(|i| f.is_empty() || i.name.to_lowercase().contains(&f) || i.rel.to_lowercase().contains(&f)).take(400) {
            if ui.selectable_label(false, &it.name).on_hover_text(&it.rel).clicked() {
                out = Some(it);
            }
        }
    });
    out
}

impl TrafficUi {
    /// Open the window on a tab (`OPENMAPPER_TRAFFIC=day|groups|lists`, for checks).
    pub fn open_at(&mut self, tab: &str) {
        self.open = true;
        self.tab = match tab {
            "groups" => Tab::Groups,
            "lists" => Tab::Lists,
            "depots" => Tab::Depots,
            _ => Tab::Day,
        };
        self.group = (self.tab == Tab::Groups).then_some(0);
    }

    /// The depot files' part shown (`stops`, `routes`, else the destinations).
    pub fn depots_part(&mut self, part: &str) {
        self.hof.show_part(part);
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: Option<&Roots>) {
        let Some(dir) = ed.doc.as_ref().map(|d| d.dir.clone()) else {
            self.open = false;
            return;
        };
        if self.map.as_ref() != Some(&dir) {
            let (open, tab, group) = (self.open, self.tab, self.group);
            let part = self.hof.part();
            *self = TrafficUi { open, tab, group, map: Some(dir), ..Default::default() };
            self.hof.show_part(part);
        }
        if !self.open {
            return;
        }
        if self.catalogue.is_none() {
            if let Some(r) = roots {
                let t0 = std::time::Instant::now();
                let c = Catalogue::scan(&[r.content.clone(), r.omsi.clone()]);
                log::info!("traffic: {} vehicles, {} parked cars, {} people ({:.0} ms)", c.vehicles.len(), c.parked.len(), c.humans.len(), t0.elapsed().as_secs_f64() * 1000.0);
                self.catalogue = Some(c);
                self.roots = vec![r.omsi.clone()];
            }
        }
        // (as it was before this frame's edits, for undo)
        let doc = ed.doc.as_ref().unwrap();
        let before = (doc.global.traffic_density_road.clone(), doc.global.traffic_density_passenger.clone(), doc.traffic.clone(), doc.timetable.clone());
        let global_before = (doc.global.clone(), doc.order.clone());
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Traffic, tr("Traffic and AI", "Трафик и ИИ")))
            .id(egui::Id::new("traffic window"))
            .open(&mut open)
            .default_pos([330.0, 90.0])
            .default_size([560.0, 620.0])
            .min_width(460.0)
            .resizable(true)
            .show(ctx, |ui| self.contents(ui, ed.doc.as_mut().unwrap()));
        self.open = open;
        let doc = ed.doc.as_mut().unwrap();
        let density = before.0 != doc.global.traffic_density_road || before.1 != doc.global.traffic_density_passenger;
        if density {
            doc.global_dirty = true;
        }
        let traffic = before.2 != doc.traffic;
        let timetable = !before.3.same_content(&doc.timetable);
        if density || traffic || timetable {
            ed.history.push_snapshot(
                tr("Edit traffic", "Правка трафика"),
                crate::history::Snapshot { global: density.then_some(global_before), traffic: traffic.then_some(before.2), timetable: timetable.then_some(before.3), descriptions: None, groups: None },
            );
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, doc: &mut Document) {
        ui.horizontal(|ui| {
            for (t, icon, label) in [(Tab::Day, Icon::Schedule, tr("Over the day", "По времени суток")), (Tab::Groups, Icon::Traffic, tr("Vehicle groups", "Группы машин")), (Tab::Lists, Icon::Parking, tr("Parked cars and people", "Парковки и люди")), (Tab::Depots, Icon::Flag, tr("Depot files", "Файлы депо"))] {
                if ui.add(egui::Button::selectable(self.tab == t, icons::labelled(icon, label))).clicked() {
                    self.tab = t;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("traffic scroll").show(ui, |ui| match self.tab {
            Tab::Day => day_panel(ui, doc),
            Tab::Groups => self.groups_panel(ui, doc),
            Tab::Lists => self.lists_panel(ui, doc),
            Tab::Depots => self.hof.panel(ui, doc),
        });
    }

    fn groups_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document) {
        hint(ui, tr("The AI's vehicles come in groups. A car group drives about on the roads; a depot sends out the buses of the timetable's tours.", "Машины ИИ собраны в группы. Группа машин ездит по дорогам сама; депо выпускает автобусы на выходы расписания."));
        if doc.traffic.legacy {
            hint(ui, tr("(This map's ailists.cfg is in OMSI 2's first format: it is written in the second one when changed.)", "(ailists.cfg этой карты в первом формате OMSI 2: при изменении он запишется во втором.)"));
        }
        // groups the unscheduled traffic names that are not there (the new map template's
        // NormalCars, Trucks…): no traffic drives
        let missing: Vec<String> = doc.traffic.veh_groups.iter().filter(|(n, _)| !doc.traffic.groups.iter().any(|g| g.name.eq_ignore_ascii_case(n))).map(|(n, _)| n.clone()).collect();
        for name in missing {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("«{name}» {}", tr("is to drive about, but there is no such group.", "должна ездить по дорогам, но такой группы нет."))).color(pal().warn));
                if let Some(g) = self.stock_group(&name) {
                    if ui.button(icons::labelled(Icon::Add, tr("Make it with OMSI 2's vehicles", "Создать с машинами OMSI 2"))).on_hover_text(tr("The group of that name from OMSI 2's own maps", "Группа с этим именем из карт OMSI 2")).clicked() {
                        doc.traffic.groups.push(g);
                        doc.traffic.touch(File::AiLists);
                    }
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                for (i, g) in doc.traffic.groups.iter().enumerate() {
                    let icon = if g.is_depot { Icon::Bus } else { Icon::Traffic };
                    let n = if g.is_depot { g.typgroups.iter().map(|t| t.entries.len()).sum::<usize>() } else { g.vehicles.len() };
                    if ui.add(egui::Button::selectable(self.group == Some(i), icons::labelled(icon, &format!("{}  ({n})", g.name))).min_size(Vec2::new(165.0, 0.0))).clicked() {
                        self.group = Some(i);
                        self.group_name = g.name.clone();
                    }
                }
                ui.add_space(4.0);
                if ui.button(icons::labelled(Icon::Add, tr("New car group", "Новая группа машин"))).clicked() {
                    let name = doc.traffic.unique_name(tr("Cars", "Машины"));
                    doc.traffic.groups.push(traffic::car_group(&name));
                    doc.traffic.set_unscheduled(&name, true, 1, 1.0);
                    doc.traffic.touch(File::AiLists);
                    self.group = Some(doc.traffic.groups.len() - 1);
                    self.group_name = name;
                }
                if ui.button(icons::labelled(Icon::Add, tr("New depot", "Новое депо"))).clicked() {
                    let name = doc.traffic.unique_name(tr("Depot", "Депо"));
                    doc.traffic.groups.push(traffic::stock_depot(&name));
                    doc.traffic.touch(File::AiLists);
                    self.group = Some(doc.traffic.groups.len() - 1);
                    self.group_name = name;
                }
            });
            ui.separator();
            ui.vertical(|ui| match self.group.filter(|g| *g < doc.traffic.groups.len()) {
                Some(i) => {
                if self.group_name.is_empty() {
                    self.group_name = doc.traffic.groups[i].name.clone();
                }
                self.group_detail(ui, doc, i)
            }
                None => {
                    self.group = None;
                    ui.label(RichText::new(tr("Pick a group on the left.", "Выберите группу слева.")).color(pal().text_dim));
                }
            });
        });
    }

    /// A car group of OMSI 2's own maps by name (their vehicles are in the installation).
    fn stock_group(&mut self, name: &str) -> Option<omsi_map::AiGroup> {
        if self.stock.is_none() {
            self.stock = Some(self.roots.iter().flat_map(|r| traffic::stock_groups(r)).collect());
        }
        self.stock.as_ref()?.iter().find(|g| g.name.eq_ignore_ascii_case(name)).cloned()
    }

    fn group_detail(&mut self, ui: &mut egui::Ui, doc: &mut Document, i: usize) {
        let old = doc.traffic.groups[i].name.clone();
        let is_depot = doc.traffic.groups[i].is_depot;
        let used: usize = doc.timetable.lines.iter().flat_map(|l| l.tours.iter()).filter(|t| t.ai_group.trim().eq_ignore_ascii_case(old.trim())).count();
        ui.horizontal(|ui| {
            ui.label(tr("Name", "Имя"));
            let r = ui.add(egui::TextEdit::singleline(&mut self.group_name).desired_width(150.0));
            if r.lost_focus() && self.group_name.trim() != old {
                let new = self.group_name.trim().to_string();
                if new.is_empty() || doc.traffic.groups.iter().any(|g| g.name.eq_ignore_ascii_case(&new)) {
                    self.group_name = old.clone();
                } else {
                    doc.traffic.rename_group(i, &new);
                    // the tours that take their buses from it
                    let mut lines = Vec::new();
                    for l in &mut doc.timetable.lines {
                        for t in l.tours.iter_mut().filter(|t| t.ai_group.trim().eq_ignore_ascii_case(old.trim())) {
                            t.ai_group = new.clone();
                            lines.push(l.name.clone());
                        }
                    }
                    for l in lines {
                        doc.timetable.touch_line(&l);
                    }
                }
            }
            let hint_text = if used > 0 { tr("Delete the group (tours still take their buses from it)", "Удалить группу (выходы ещё берут из неё автобусы)") } else { tr("Delete the group", "Удалить группу") };
            if small_button(ui, Icon::Delete, hint_text) {
                doc.traffic.groups.remove(i);
                doc.traffic.set_unscheduled(&old, false, 1, 1.0);
                doc.traffic.touch(File::AiLists);
                self.group = None;
            }
        });
        if self.group.is_none() {
            return;
        }
        if is_depot {
            if used > 0 {
                hint(ui, &format!("{} {used}", tr("Tours that take their buses from here:", "Выходов, берущих автобусы отсюда:")));
            }
            self.depot_detail(ui, doc, i);
        } else {
            self.cars_detail(ui, doc, i, &old);
        }
    }

    fn cars_detail(&mut self, ui: &mut egui::Ui, doc: &mut Document, i: usize, name: &str) {
        let (on, class, factor) = doc.traffic.unscheduled(name);
        ui.horizontal(|ui| {
            if check(ui, on, tr("Drives about on its own", "Ездит по дорогам сама")) {
                doc.traffic.set_unscheduled(name, !on, class, factor);
            }
            if on {
                let mut f = factor;
                ui.label(tr("as many:", "сколько:"));
                if ui.add(egui::DragValue::new(&mut f).range(0.0..=10.0).speed(0.02).max_decimals(2).prefix("× ")).on_hover_text(tr("Times the traffic over the day", "Множитель трафика по времени суток")).changed() {
                    doc.traffic.set_unscheduled(name, true, class, f);
                }
            }
        });
        if on {
            group_curves(ui, &mut doc.traffic, name);
        }
        ui.add_space(4.0);
        hint(ui, tr("A vehicle with more weight comes along more often.", "Машина с большим весом встречается чаще."));
        let cat = self.catalogue.as_ref();
        let mut remove = None;
        let mut changed = false;
        egui::Grid::new(("cars", i)).num_columns(3).spacing(Vec2::new(8.0, 3.0)).show(ui, |ui| {
            for (k, v) in doc.traffic.groups[i].vehicles.iter_mut().enumerate() {
                let n = cat.map(|c| c.name_of(&v.file).to_string()).unwrap_or_else(|| v.file.clone());
                ui.allocate_ui_with_layout(Vec2::new(210.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(210.0);
                    ui.add(egui::Label::new(n).truncate()).on_hover_text(&v.file);
                });
                changed |= ui.add(egui::DragValue::new(&mut v.weight).range(0.0..=1000.0).speed(0.5).max_decimals(1)).changed();
                if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                    remove = Some(k);
                }
                ui.end_row();
            }
        });
        if let Some(k) = remove {
            doc.traffic.groups[i].vehicles.remove(k);
            changed = true;
        }
        if let Some(c) = &self.catalogue {
            if let Some(it) = pick(ui, "add car", tr("Add a vehicle…", "Добавить машину…"), &c.vehicles, &mut self.filter) {
                doc.traffic.groups[i].vehicles.push(omsi_map::ailists::AiVehicleEntry { file: it.rel.clone(), weight: 10.0, ..Default::default() });
                changed = true;
            }
        }
        if changed {
            doc.traffic.touch(File::AiLists);
        }
    }

    fn depot_detail(&mut self, ui: &mut egui::Ui, doc: &mut Document, i: usize) {
        let mut changed = false;
        let hofs: Vec<String> = match &self.catalogue {
            Some(c) => {
                let mut h: Vec<String> = Vec::new();
                for tg in &doc.traffic.groups[i].typgroups {
                    for x in c.hofs_of(&tg.file) {
                        if !h.contains(&x) {
                            h.push(x);
                        }
                    }
                }
                h
            }
            None => Vec::new(),
        };
        let g = &mut doc.traffic.groups[i];
        ui.horizontal(|ui| {
            ui.label(tr("Depot file (.hof)", "Файл депо (.hof)"));
            let cur = g.hof.clone().unwrap_or_default();
            egui::ComboBox::from_id_salt(("hof", i)).selected_text(if cur.is_empty() { "–".to_string() } else { cur.clone() }).show_ui(ui, |ui| {
                for h in &hofs {
                    if ui.selectable_label(h.eq_ignore_ascii_case(&cur), h).clicked() {
                        g.hof = Some(h.clone());
                        changed = true;
                    }
                }
            });
        });
        if let Some(h) = &g.hof {
            if !hofs.is_empty() && !hofs.iter().any(|x| x.eq_ignore_ascii_case(h)) {
                ui.label(RichText::new(tr("None of its buses has this depot file beside it: they run without their destination signs and fleet data.", "Ни у одного автобуса нет рядом этого файла депо: они поедут без табличек и данных парка.")).small().color(pal().warn));
            }
        }
        hint(ui, tr("The buses it has: a type of bus and the fleet number and plate of each.", "Автобусы депо: тип автобуса, номер и госномер каждого."));
        let cat = self.catalogue.as_ref();
        let mut remove_type = None;
        for (k, tg) in g.typgroups.iter_mut().enumerate() {
            ui.add_space(4.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let n = cat.map(|c| c.name_of(&tg.file).to_string()).unwrap_or_else(|| tg.file.clone());
                    ui.label(icons::labelled(Icon::Bus, &n)).on_hover_text(&tg.file);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_button(ui, Icon::Delete, tr("Take this type out", "Убрать этот тип")) {
                            remove_type = Some(k);
                        }
                    });
                });
                let mut remove = None;
                egui::Grid::new(("fleet", i, k)).num_columns(3).spacing(Vec2::new(6.0, 3.0)).show(ui, |ui| {
                    for (j, e) in tg.entries.iter_mut().enumerate() {
                        changed |= ui.add(egui::TextEdit::singleline(&mut e.number).desired_width(60.0).hint_text(tr("number", "номер"))).changed();
                        changed |= ui.add(egui::TextEdit::singleline(&mut e.registration).desired_width(110.0).hint_text(tr("plate", "госномер"))).changed();
                        if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                            remove = Some(j);
                        }
                        ui.end_row();
                    }
                });
                if let Some(j) = remove {
                    tg.entries.remove(j);
                    changed = true;
                }
                if ui.button(icons::labelled(Icon::Add, tr("Another bus", "Ещё автобус"))).clicked() {
                    let next = tg.entries.iter().filter_map(|e| e.number.trim().parse::<i64>().ok()).max().map(|n| n + 1).unwrap_or(1);
                    tg.entries.push(omsi_map::DepotEntry { number: next.to_string(), ..Default::default() });
                    changed = true;
                }
            });
        }
        if let Some(k) = remove_type {
            g.typgroups.remove(k);
            changed = true;
        }
        if let Some(c) = &self.catalogue {
            let buses: Vec<Item> = c.vehicles.iter().filter(|v| v.rel.to_lowercase().ends_with(".bus")).cloned().collect();
            if let Some(it) = pick(ui, "add bus type", tr("Add a type of bus…", "Добавить тип автобуса…"), &buses, &mut self.filter) {
                g.typgroups.push(omsi_map::ailists::AiTypGroup { file: it.rel.clone(), entries: vec![omsi_map::DepotEntry { number: "1".into(), ..Default::default() }] });
                if g.hof.as_deref().unwrap_or("").is_empty() {
                    g.hof = c.hofs_of(&it.rel).into_iter().next();
                }
                changed = true;
            }
        }
        if changed {
            doc.traffic.touch(File::AiLists);
        }
    }

    fn lists_panel(&mut self, ui: &mut egui::Ui, doc: &mut Document) {
        let Some(c) = &self.catalogue else {
            ui.label(RichText::new(tr("The lists can't be read without the OMSI 2 folder.", "Без папки OMSI 2 списки не прочитать.")).color(pal().warn));
            return;
        };
        let t = &mut doc.traffic;
        for (what, title, help, items, file) in [
            (0, tr("Parked cars", "Припаркованные машины"), tr("What stands on the car parks (Game logic > Car parks). A car listed more often stands there more often.", "Что стоит на парковках (Служебное > Парковки). Машина, записанная чаще, встречается чаще."), &c.parked, File::Parked),
            (1, tr("People", "Люди"), tr("Who walks about and waits at the stops.", "Кто гуляет и ждёт на остановках."), &c.humans, File::Humans),
            (2, tr("Drivers", "Водители"), tr("Who drives the AI's buses.", "Кто водит автобусы ИИ."), &c.humans, File::Drivers),
        ] {
            ui.add_space(6.0);
            ui.label(RichText::new(title).strong().color(pal().text_strong));
            hint(ui, help);
            let list = match what {
                0 => &mut t.parked,
                1 => &mut t.humans,
                _ => &mut t.drivers,
            };
            // (one row a file, with how often it is listed)
            let mut rows: Vec<(String, usize)> = Vec::new();
            for rel in list.iter() {
                match rows.iter_mut().find(|r| r.0.eq_ignore_ascii_case(rel)) {
                    Some(r) => r.1 += 1,
                    None => rows.push((rel.clone(), 1)),
                }
            }
            let mut changed = false;
            egui::Grid::new(("list", what)).num_columns(2).spacing(Vec2::new(10.0, 2.0)).show(ui, |ui| {
                for r in rows.iter_mut() {
                    ui.label(c.name_of(&r.0)).on_hover_text(&r.0);
                    changed |= ui.add(egui::DragValue::new(&mut r.1).range(0..=50).prefix("× ")).on_hover_text(tr("How often it is listed (0 takes it out)", "Сколько раз в списке (0: убрать)")).changed();
                    ui.end_row();
                }
            });
            if let Some(it) = pick(ui, &format!("add list {what}"), tr("Add…", "Добавить…"), items, &mut self.filter) {
                match rows.iter_mut().find(|r| r.0.eq_ignore_ascii_case(&it.rel)) {
                    Some(r) => r.1 += 1,
                    None => rows.push((it.rel.clone(), 1)),
                }
                changed = true;
            }
            if changed {
                *list = rows.iter().flat_map(|(rel, n)| std::iter::repeat_n(rel.clone(), *n)).collect();
            }
            if changed {
                t.touch(file);
            }
        }
    }
}

/// The two curves over the day, drawn and edited.
fn day_panel(ui: &mut egui::Ui, doc: &mut Document) {
    hint(ui, tr("How busy the map is at each hour: the road traffic, and the passengers at the stops. 1 is normal; the game goes along the line between the points.", "Насколько оживлена карта в каждый час: машины на дорогах и пассажиры на остановках. 1: обычно; игра идёт по линии между точками."));
    let g = &mut doc.global;
    if plot(ui, &mut g.traffic_density_road, &mut g.traffic_density_passenger) {
        g.traffic_density_road.sort_by(|a, b| a.0.total_cmp(&b.0));
        g.traffic_density_passenger.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    ui.columns(2, |cols| {
        for (k, title, color) in [(0, tr("Road traffic", "Машины на дорогах"), ROAD), (1, tr("Passengers", "Пассажиры"), PEOPLE)] {
            let ui = &mut cols[k];
            let curve = if k == 0 { &mut g.traffic_density_road } else { &mut g.traffic_density_passenger };
            curve_table(ui, k, title, color, curve);
        }
    });
}

/// A group's own curves over the day, by days of the week (`unsched_trafficdens.txt`).
fn group_curves(ui: &mut egui::Ui, t: &mut crate::traffic::Traffic, name: &str) {
    let Some(gi) = t.density.iter().position(|g| g.name.eq_ignore_ascii_case(name)) else { return };
    let before = t.density[gi].densities.clone();
    let g = &mut t.density[gi];
    egui::CollapsingHeader::new(icons::labelled(Icon::Schedule, tr("Its own traffic over the day", "Свой трафик по времени суток"))).id_salt(("group curves", name.to_lowercase())).default_open(!g.densities.is_empty()).show(ui, |ui| {
        hint(ui, tr("Without one the group follows the map's road traffic. A curve can be for some days of the week only.", "Без неё группа следует трафику карты. Кривая может быть только для некоторых дней недели."));
        let mut remove = None;
        for (k, (mask, curve)) in g.densities.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(tr("Days:", "Дни:"));
                // 0: every day, else +1 weekdays, +2 Saturday, +4 Sunday
                for (bit, label) in [(1, tr("Mo–Fr", "Пн–Пт")), (2, tr("Sa", "Сб")), (4, tr("Su", "Вс"))] {
                    let on = *mask == 0 || *mask & bit != 0;
                    if check(ui, on, label) {
                        let m = if *mask == 0 { 7 } else { *mask };
                        let m = m ^ bit;
                        // (at least one kind of day)
                        if m != 0 {
                            *mask = if m == 7 { 0 } else { m };
                        }
                    }
                }
                if small_button(ui, Icon::Delete, tr("Take this curve out", "Убрать эту кривую")) {
                    remove = Some(k);
                }
            });
            curve_table(ui, 10 + k, tr("Traffic", "Трафик"), ROAD, curve);
            ui.separator();
        }
        if let Some(k) = remove {
            g.densities.remove(k);
        }
        if ui.button(icons::labelled(Icon::Add, tr("A curve", "Кривая"))).clicked() {
            let free = if g.densities.is_empty() { 0 } else { 2 };
            g.densities.push((free, TOWN.to_vec()));
        }
    });
    if t.density[gi].densities != before {
        t.touch(File::Density);
    }
}

/// One curve's points as a table, with typical days to start from.
fn curve_table(ui: &mut egui::Ui, k: usize, title: &str, color: Color32, curve: &mut Vec<(f32, f32)>) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(color));
        ui.menu_button(tr("Typical day…", "Типичный день…"), |ui| {
            for (name, c) in [(tr("Quiet village", "Тихая деревня"), QUIET), (tr("Town with rush hours", "Город с часами пик"), TOWN), (tr("The same all day", "Весь день одинаково"), FLAT)] {
                if ui.button(name).clicked() {
                    *curve = c.to_vec();
                }
            }
        });
    });
    if curve.is_empty() {
        hint(ui, tr("No curve: 1 all day.", "Кривой нет: весь день 1."));
    }
    let mut remove = None;
    let mut sort = false;
    egui::Grid::new(("curve", k)).num_columns(3).spacing(Vec2::new(8.0, 2.0)).show(ui, |ui| {
        for (j, (h, d)) in curve.iter_mut().enumerate() {
            let r = ui.add(egui::DragValue::new(h).range(0.0..=24.0).speed(0.05).max_decimals(2).custom_formatter(|v, _| {
                let m = (v * 60.0).round() as i64;
                format!("{}:{:02}", m / 60, m % 60)
            }));
            sort |= r.drag_stopped() || r.lost_focus();
            ui.add(egui::DragValue::new(d).range(0.0..=10.0).speed(0.01).max_decimals(2).prefix("× "));
            if small_button(ui, Icon::Delete, tr("Take the point out", "Убрать точку")) {
                remove = Some(j);
            }
            ui.end_row();
        }
    });
    if let Some(j) = remove {
        curve.remove(j);
    }
    if ui.button(icons::labelled(Icon::Add, tr("Point", "Точка"))).clicked() {
        let h = curve.last().map(|p| (p.0 + 2.0).min(24.0)).unwrap_or(12.0);
        curve.push((h, 1.0));
        sort = true;
    }
    if sort {
        curve.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
}

/// Both curves over 24 hours; a point can be dragged up and down (and along). True when
/// a drag ended.
fn plot(ui: &mut egui::Ui, road: &mut [(f32, f32)], people: &mut [(f32, f32)]) -> bool {
    let mut released = false;
    let w = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 150.0), Sense::hover());
    let inner = Rect::from_min_max(rect.min + Vec2::new(28.0, 6.0), rect.max - Vec2::new(6.0, 18.0));
    let top = road.iter().chain(people.iter()).map(|p| p.1).fold(2.0f32, f32::max).ceil();
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, pal().canvas);
    let at = |h: f32, d: f32| Pos2::new(inner.left() + inner.width() * h / 24.0, inner.bottom() - inner.height() * d / top);
    for h in (0..=24).step_by(3) {
        let x = at(h as f32, 0.0).x;
        p.line_segment([Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())], Stroke::new(1.0, pal().grid));
        p.text(Pos2::new(x, inner.bottom() + 2.0), egui::Align2::CENTER_TOP, format!("{h}"), egui::FontId::proportional(10.0), pal().text_dim);
    }
    for d in 0..=top as i32 {
        let y = at(0.0, d as f32).y;
        p.line_segment([Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)], Stroke::new(1.0, if d == 1 { pal().grid_major } else { pal().grid }));
        p.text(Pos2::new(inner.left() - 4.0, y), egui::Align2::RIGHT_CENTER, format!("{d}"), egui::FontId::proportional(10.0), pal().text_dim);
    }
    for (k, curve, color) in [(0, road, ROAD), (1, people, PEOPLE)] {
        let pts: Vec<Pos2> = curve.iter().map(|q| at(q.0, q.1)).collect();
        if pts.len() >= 2 {
            p.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, theme::readable(color))));
        }
        for (j, q) in curve.iter_mut().enumerate() {
            let c = at(q.0, q.1);
            let r = ui.interact(Rect::from_center_size(c, Vec2::splat(12.0)), ui.id().with(("pt", k, j)), Sense::drag());
            if r.dragged() {
                let d = r.drag_delta();
                q.0 = (q.0 + d.x / inner.width() * 24.0).clamp(0.0, 24.0);
                q.1 = (q.1 - d.y / inner.height() * top).clamp(0.0, 10.0);
            }
            released |= r.drag_stopped();
            let hot = r.hovered() || r.dragged();
            p.circle_filled(c, if hot { 5.0 } else { 3.5 }, color);
            if hot {
                let m = (q.0 * 60.0).round() as i64;
                p.text(c + Vec2::new(0.0, -8.0), egui::Align2::CENTER_BOTTOM, format!("{}:{:02}  × {:.2}", m / 60, m % 60, q.1), egui::FontId::proportional(11.0), pal().ink);
            }
        }
    }
    released
}
