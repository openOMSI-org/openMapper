//! The crossing path editor: the `[path]`s of a crossing object of the map, drawn from above
//! and edited - where they start and end, how wide, how steep, which way, which light they
//! wait at, which paths they block - then written back into a copy of the object.
//!
//! It does not build a crossing (the complex junction editor does): it changes one that is
//! already on the map, as OMSI 2's own `OmsiObjEditP` does, without touching the original
//! `.sco` in the installation.

use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};
use glam::{DVec2, DVec3};

use crate::assets::Assets;
use crate::crossing_edit::{self, Crossing, Helper, Path, Sheet, TURN_LEFT, TURN_NONE, TURN_RIGHT};
use crate::doc::Sel;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::library::Library;

const STREET: Color32 = Color32::from_rgb(232, 190, 96);
const FOOTWAY: Color32 = Color32::from_rgb(104, 184, 224);
const RAIL: Color32 = Color32::from_rgb(196, 148, 208);
const PICKED: Color32 = Color32::from_rgb(255, 148, 40);
const HANDLE: Color32 = Color32::from_rgb(238, 238, 238);
const SNAPPED: Color32 = Color32::from_rgb(96, 208, 128);
const DIM: Color32 = Color32::from_gray(150);
const BAD: Color32 = Color32::from_rgb(240, 110, 96);
const GOOD: Color32 = Color32::from_rgb(120, 200, 130);

/// How near a click must be to an end to take it (pixels).
const GRAB: f32 = 9.0;

/// The crossing path editor's window and its state.
pub struct CrossingPaths {
    pub open: bool,
    /// The object being edited.
    target: Option<Sel>,
    /// The `.sco` the map names, and where it is.
    rel: String,
    abs: std::path::PathBuf,
    /// The bytes it was read from: the code page it is written back in.
    bytes: Vec<u8>,
    sheet: Option<Sheet>,
    /// The crossing as it was when opened (Revert).
    seen: Crossing,
    /// The path being edited, and the end being dragged of it.
    selected: Option<usize>,
    drag: Option<(usize, bool)>,
    /// The canvas: pixels per metre, and the point of the object at its centre.
    scale: f32,
    centre: DVec2,
    fit: bool,
    /// The file this window writes is already a copy the editor made (so it writes in place).
    copied: bool,
    message: Option<(String, bool)>,
    /// Crossing objects of the map, to pick from when nothing is selected.
    candidates: Vec<((i32, i32), i64, String)>,
}

impl Default for CrossingPaths {
    fn default() -> Self {
        CrossingPaths {
            open: false,
            target: None,
            rel: String::new(),
            abs: Default::default(),
            bytes: Vec::new(),
            sheet: None,
            seen: Crossing::default(),
            selected: None,
            drag: None,
            scale: 14.0,
            centre: DVec2::ZERO,
            fit: true,
            copied: false,
            message: None,
            candidates: Vec::new(),
        }
    }
}

/// Where the mark that a `.sco` is this editor's own copy sits.
fn sidecar(abs: &std::path::Path) -> std::path::PathBuf {
    abs.with_extension("crossing.json")
}

fn kind_color(kind: i32) -> Color32 {
    match kind {
        1 => FOOTWAY,
        2 => RAIL,
        _ => STREET,
    }
}

fn kind_name(kind: i32) -> &'static str {
    match kind {
        1 => tr("Footway", "Тротуар"),
        2 => tr("Rail", "Рельсы"),
        _ => tr("Street", "Проезжая часть"),
    }
}

fn direction_name(d: i32) -> &'static str {
    match d {
        1 => tr("Against", "Против"),
        2 => tr("Both ways", "В обе стороны"),
        _ => tr("Along", "По направлению"),
    }
}

fn turn_name(t: i32) -> &'static str {
    match t {
        TURN_LEFT => tr("Left", "Налево"),
        TURN_RIGHT => tr("Right", "Направо"),
        _ => tr("None", "Нет"),
    }
}

/// What a `[phase]`'s state shows (the game's own groups).
fn aspect_name(state: i32) -> &'static str {
    match state {
        0..=2 => tr("Red", "Красный"),
        3..=5 => tr("Red and yellow", "Красный и жёлтый"),
        6 | 7 => tr("Green", "Зелёный"),
        8 => tr("Green and yellow", "Зелёный и жёлтый"),
        9..=11 => tr("Yellow", "Жёлтый"),
        _ => tr("Off", "Выключен"),
    }
}

const ASPECTS: [i32; 6] = [0, 3, 6, 8, 9, 12];

impl CrossingPaths {
    /// Open on the selected object, or on a crossing picked from the map's list.
    pub fn request(&mut self, ed: &mut Editor, roots: &Roots, lib: &Library) {
        if let Some(Sel::Object(k, id)) = ed.selected {
            if self.open_on(ed, roots, k, id) {
                self.open = true;
                return;
            }
        }
        self.candidates.clear();
        if let Some(doc) = ed.doc.as_ref() {
            for (k, t) in &doc.tiles {
                for o in &t.tile.objects {
                    if self.candidates.len() >= 300 {
                        break;
                    }
                    let crossing = lib.find(&o.file).map(|a| a.category == crate::library::Category::Junctions).unwrap_or(false) || crate::editor::is_own_junction(&o.file);
                    if crossing {
                        self.candidates.push((*k, o.id, o.file.clone()));
                    }
                }
            }
        }
        self.open = true;
        if self.message.is_none() {
            self.message = Some((tr("Select a crossing object, or pick one of these", "Выберите объект перекрёстка или возьмите один из этих").into(), false));
        }
    }

    /// Read the object's `.sco` for editing. False (and a message) when it has no paths.
    fn open_on(&mut self, ed: &Editor, roots: &Roots, k: (i32, i32), id: i64) -> bool {
        let Some(doc) = ed.doc.as_ref() else { return false };
        let Some(o) = doc.object(k, id) else { return false };
        let rel = o.file.clone();
        if let Some(why) = crossing_edit::why_not(roots, &rel) {
            self.message = Some((format!("{}: {}", &rel, why), true));
            self.sheet = None;
            return false;
        }
        match crossing_edit::open(roots, &rel) {
            Ok((abs, bytes, sheet)) => {
                self.copied = sidecar(&abs).is_file();
                self.target = Some(Sel::Object(k, id));
                self.rel = rel;
                self.abs = abs;
                self.bytes = bytes;
                self.seen = sheet.crossing.clone();
                self.selected = (!sheet.crossing.paths.is_empty()).then_some(0);
                self.sheet = Some(sheet);
                self.fit = true;
                self.drag = None;
                self.message = None;
                true
            }
            Err(e) => {
                self.message = Some((e.to_string(), true));
                self.sheet = None;
                false
            }
        }
    }

    /// The window.
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, roots: Option<&Roots>, assets: Option<&mut Assets>, actions: &mut Vec<crate::ui::Action>) {
        let _ = (lib, actions);
        if !self.open {
            if ed.crossing_draft.is_some() {
                ed.crossing_draft = None;
            }
            return;
        }
        let mut open = self.open;
        if self.sheet.is_some() {
            // the map shows the paths as they are here, not as the file still has them
            if let (Some(Sel::Object(k, id)), Some(sheet)) = (self.target, self.sheet.as_ref()) {
                ed.crossing_draft = Some((k, id, sheet.crossing.paths.clone()));
            }
        }
        let screen = ctx.content_rect().size();
        let size = Vec2::new((screen.x * 0.86).clamp(820.0, 1500.0), (screen.y * 0.86).clamp(540.0, 1000.0));
        egui::Window::new(icons::labelled(Icon::Route, tr("Crossing paths", "Пути перекрёстка")))
            .open(&mut open)
            .default_size(size)
            .min_size(Vec2::new(760.0, 480.0))
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    egui::ScrollArea::vertical().id_salt("crossing side").auto_shrink([false, false]).max_width(320.0).show(ui, |ui| {
                        ui.set_width(304.0);
                        self.side(ui, ed, roots);
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        self.toolbar(ui, ed, roots, assets);
                        self.canvas(ui);
                    });
                });
            });
        self.open = open;
        if !self.open {
            ed.crossing_draft = None;
        }
    }

    /// The left side: the paths, and the one picked.
    fn side(&mut self, ui: &mut egui::Ui, ed: &mut Editor, roots: Option<&Roots>) {
        let Some(mut sheet) = self.sheet.take() else {
            self.no_target(ui, ed, roots);
            return;
        };
        self.sheet_ui(ui, ed, &mut sheet);
        self.sheet = Some(sheet);
    }

    fn no_target(&mut self, ui: &mut egui::Ui, ed: &mut Editor, roots: Option<&Roots>) {
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Pick a crossing object of the map", "Возьмите объект перекрёстка на карте")).strong());
        ui.label(RichText::new(tr("The paths, the blocking and the traffic light program of an existing crossing are edited here. The object is copied into openMapper's own folder; the original file is left as it is.", "Здесь правят пути, блокировки и программу светофора готового перекрёстка. Объект копируется в папку openMapper, исходный файл остаётся как был.")).small().color(DIM));
        if let Some((msg, bad)) = &self.message {
            ui.colored_label(if *bad { BAD } else { DIM }, msg);
        }
        ui.add_space(6.0);
        let mut pick = None;
        egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(80.0)).show(ui, |ui| {
            for (k, id, rel) in &self.candidates {
                let name = std::path::Path::new(&rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| rel.clone());
                if ui.selectable_label(false, name).on_hover_text(format!("{rel}\n{}, {}", k.0, k.1)).clicked() {
                    pick = Some((*k, *id));
                }
            }
        });
        if let (Some((k, id)), Some(roots)) = (pick, roots) {
            if self.open_on(ed, roots, k, id) {
                self.message = None;
            }
        }
    }

    fn sheet_ui(&mut self, ui: &mut egui::Ui, ed: &mut Editor, sheet: &mut Sheet) {
        let Some(Sel::Object(..)) = self.target else { return };
        ui.add_space(4.0);
        let name = std::path::Path::new(&self.rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| self.rel.clone());
        ui.label(RichText::new(name).strong());
        ui.label(RichText::new(if self.copied { tr("openMapper's copy", "Копия openMapper") } else { tr("in its own file (a copy is made on saving)", "в своём файле (копия делается при сохранении)") }).small().color(DIM));
        ui.add_space(4.0);

        let mut changed = false;
        // ---- the paths
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ({})", tr("Paths", "Пути"), sheet.crossing.paths.len())).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::labelled(Icon::Add, "")).on_hover_text(tr("Add a path", "Добавить путь")).clicked() {
                    let mut p = Path::default();
                    if let Some(i) = self.selected {
                        if let Some(q) = sheet.crossing.paths.get(i) {
                            p = q.shifted(q.width().max(1.0));
                        }
                    }
                    sheet.crossing.paths.push(p);
                    self.selected = Some(sheet.crossing.paths.len() - 1);
                    changed = true;
                }
                if ui.small_button(icons::labelled(Icon::Copy, "")).on_hover_text(tr("Duplicate", "Дублировать")).clicked() {
                    if let Some(q) = self.selected.and_then(|i| sheet.crossing.paths.get(i)).cloned() {
                        sheet.crossing.paths.push(q);
                        self.selected = Some(sheet.crossing.paths.len() - 1);
                        changed = true;
                    }
                }
            });
        });
        let mut remove = None;
        let mut move_to = None;
        egui::ScrollArea::vertical().id_salt("crossing paths").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
            for i in 0..sheet.crossing.paths.len() {
                let p = &sheet.crossing.paths[i];
                let mut text = format!("#{i}  {}  {:.1} m", kind_name(p.kind()), p.length());
                if p.turn() != TURN_NONE {
                    text.push_str(&format!("  {}", turn_name(p.turn())));
                }
                if let Some(l) = p.light {
                    text.push_str(&format!("  L{l}"));
                }
                if !p.blocks.is_empty() {
                    text.push_str(&format!("  ⛔{}", p.blocks.len()));
                }
                let on = self.selected == Some(i);
                ui.horizontal(|ui| {
                    if ui.selectable_label(on, RichText::new(text).color(kind_color(p.kind()))).clicked() {
                        self.selected = Some(i);
                    }
                    if i > 0 && ui.small_button("↑").on_hover_text(tr("Move up", "Выше")).clicked() {
                        move_to = Some((i, i - 1));
                    }
                    if i + 1 < sheet.crossing.paths.len() && ui.small_button("↓").on_hover_text(tr("Move down", "Ниже")).clicked() {
                        move_to = Some((i, i + 1));
                    }
                    if ui.small_button(icons::labelled(Icon::Delete, "")).on_hover_text(tr("Remove this path", "Удалить этот путь")).clicked() {
                        remove = Some(i);
                    }
                });
            }
        });
        if let Some((from, to)) = move_to {
            sheet.crossing.move_path(from, to);
            self.selected = Some(to);
            changed = true;
        }
        if let Some(i) = remove {
            sheet.crossing.remove_path(i);
            self.selected = if sheet.crossing.paths.is_empty() { None } else { Some(i.min(sheet.crossing.paths.len() - 1)) };
            changed = true;
        }

        // ---- the numbers of the picked path
        if let Some(i) = self.selected.filter(|i| *i < sheet.crossing.paths.len()) {
            ui.separator();
            let crossing = &mut sheet.crossing;
            let p = &mut crossing.paths[i];
            egui::Grid::new("crossing numbers").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                ui.label(RichText::new(tr("Start", "Начало")).small().color(DIM));
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut p.v[0]).speed(0.05).prefix("x ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut p.v[1]).speed(0.05).prefix("y ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut p.v[2]).speed(0.05).prefix("z ")).changed();
                });
                ui.end_row();
                ui.label(RichText::new(tr("Heading", "Направление")).small().color(DIM));
                let mut h = p.heading();
                if ui.add(egui::DragValue::new(&mut h).speed(0.5).suffix("°")).changed() {
                    p.v[3] = h as f32;
                    changed = true;
                }
                ui.end_row();
                ui.label(RichText::new(tr("Radius", "Радиус")).small().color(DIM));
                let mut r = p.radius();
                if ui.add(egui::DragValue::new(&mut r).speed(0.1).suffix(" m")).on_hover_text(tr("0 is a straight line, above 0 turns right, below 0 left", "0 — прямая, больше 0 — направо, меньше 0 — налево")).changed() {
                    p.v[4] = r as f32;
                    changed = true;
                }
                ui.end_row();
                ui.label(RichText::new(tr("Length", "Длина")).small().color(DIM));
                let mut l = p.length();
                if ui.add(egui::DragValue::new(&mut l).speed(0.1).range(0.0..=2000.0).suffix(" m")).changed() {
                    p.v[5] = l as f32;
                    changed = true;
                }
                ui.end_row();
                ui.label(RichText::new(tr("Slope", "Уклон")).small().color(DIM));
                ui.horizontal(|ui| {
                    let mut a = p.slope_start() * 100.0;
                    let mut b = p.slope_end() * 100.0;
                    if ui.add(egui::DragValue::new(&mut a).speed(0.05).suffix(" %")).changed() {
                        p.v[6] = (a / 100.0) as f32;
                        changed = true;
                    }
                    if ui.add(egui::DragValue::new(&mut b).speed(0.05).suffix(" %")).changed() {
                        p.v[7] = (b / 100.0) as f32;
                        changed = true;
                    }
                });
                ui.end_row();
                ui.label(RichText::new(tr("Kind", "Тип")).small().color(DIM));
                let mut k = p.v[8] as i32;
                let before = k;
                egui::ComboBox::from_id_salt("crossing kind").selected_text(kind_name(k)).width(150.0).show_ui(ui, |ui| {
                    for n in [0, 1, 2] {
                        ui.selectable_value(&mut k, n, kind_name(n));
                    }
                });
                p.v[8] = k as f32;
                changed |= k != before;
                ui.end_row();
                ui.label(RichText::new(tr("Width", "Ширина")).small().color(DIM));
                changed |= ui.add(egui::DragValue::new(&mut p.v[9]).speed(0.05).range(0.0..=30.0).suffix(" m")).changed();
                ui.end_row();
                ui.label(RichText::new(tr("Direction", "Направление движения")).small().color(DIM));
                let mut d = p.v[10] as i32;
                let before = d;
                egui::ComboBox::from_id_salt("crossing direction").selected_text(direction_name(d)).width(150.0).show_ui(ui, |ui| {
                    for n in [0, 1, 2] {
                        ui.selectable_value(&mut d, n, direction_name(n));
                    }
                });
                p.v[10] = d as f32;
                changed |= d != before;
                ui.end_row();
                ui.label(RichText::new(tr("Turn signal", "Поворот")).small().color(DIM));
                let mut t = p.v[11] as i32;
                let before = t;
                egui::ComboBox::from_id_salt("crossing turn").selected_text(turn_name(t)).width(150.0).show_ui(ui, |ui| {
                    for n in [TURN_NONE, TURN_LEFT, TURN_RIGHT] {
                        ui.selectable_value(&mut t, n, turn_name(n));
                    }
                });
                p.v[11] = t as f32;
                changed |= t != before;
                ui.end_row();
                ui.label(RichText::new(tr("Light", "Светофор")).small().color(DIM));
                let names: Vec<(i32, String)> = std::iter::once((-1, tr("None", "Нет").to_string())).chain(crossing.lights.iter().enumerate().map(|(n, l)| (n as i32, format!("#{n} {}", if l.name.trim().is_empty() { "-" } else { l.name.trim() })))).collect();
                let mut cur = p.light.unwrap_or(-1);
                let before = cur;
                egui::ComboBox::from_id_salt("crossing light").selected_text(names.iter().find(|(n, _)| *n == cur).map(|(_, s)| s.clone()).unwrap_or_else(|| tr("None", "Нет").into())).width(150.0).show_ui(ui, |ui| {
                    for (n, s) in &names {
                        ui.selectable_value(&mut cur, *n, s);
                    }
                });
                if cur != before {
                    p.light = (cur >= 0).then_some(cur);
                    changed = true;
                }
                ui.end_row();
            });
            changed |= ui.checkbox(&mut sheet.crossing.paths[i].crossing_problem, tr("Keeps the junction clear", "Держит перекрёсток свободным")).on_hover_text(tr("A vehicle waiting on this path holds the crossing empty for the others (`[crossingproblem]`)", "Машина на этом пути держит перекрёсток свободным для других (`[crossingproblem]`)")).changed();
            // blocking: which other paths wait while this one is taken
            let others: Vec<(usize, String)> = (0..sheet.crossing.paths.len()).filter(|j| *j != i).map(|j| (j, format!("#{j} {}", kind_name(sheet.crossing.paths[j].kind())))).collect();
            if !others.is_empty() {
                let n = sheet.crossing.paths[i].blocks.len();
                egui::CollapsingHeader::new(RichText::new(format!("{} ({n})", tr("Blocks", "Блокирует"))).small()).id_salt("crossing blocks").default_open(n > 0).show(ui, |ui| {
                    for (j, label) in &others {
                        let mut on = sheet.crossing.paths[i].blocks.iter().any(|(a, _)| *a == *j as i32);
                        if ui.checkbox(&mut on, label).changed() {
                            let p = &mut sheet.crossing.paths[i];
                            p.blocks.retain(|(a, _)| *a != *j as i32);
                            if on {
                                p.blocks.push((*j as i32, 0));
                            }
                            changed = true;
                        }
                    }
                });
            }
        }

        // ---- the traffic lights
        ui.separator();
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Traffic lights", "Светофоры"), sheet.crossing.lights.len())).strong()).id_salt("crossing lights").default_open(false).show(ui, |ui| {
            let mut cycle = sheet.crossing.cycle.unwrap_or(0.0) > 0.0;
            ui.horizontal(|ui| {
                if ui.checkbox(&mut cycle, tr("Cycle", "Цикл")).on_hover_text(tr("`[traffic_lights_group]`: the seconds of one run. Unset, the longest light's phases add up to it", "`[traffic_lights_group]`: секунды одного круга. Если не задан — сумма фаз самой длинной программы")).changed() {
                    sheet.crossing.cycle = if cycle { Some(sheet.crossing.phases_sum().max(1.0)) } else { None };
                    changed = true;
                }
                if let Some(c) = sheet.crossing.cycle.as_mut() {
                    changed |= ui.add(egui::DragValue::new(c).speed(0.5).range(1.0..=600.0).suffix(" s")).changed();
                }
            });
            let sum = sheet.crossing.phases_sum();
            let len = sheet.crossing.cycle_len();
            if (sum - len).abs() > 0.5 {
                ui.label(RichText::new(tr("The phases do not add up to the cycle: the last one holds to its end", "Фазы не складываются в цикл: последняя длится до его конца")).small().color(DIM));
            }
            let mut gone = None;
            for (n, l) in sheet.crossing.lights.iter_mut().enumerate() {
                egui::CollapsingHeader::new(RichText::new(format!("#{n} {}", if l.name.trim().is_empty() { tr("(no name)", "(без имени)") } else { l.name.trim() })).small()).id_salt(("crossing light", n)).default_open(false).show(ui, |ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(160.0)).changed();
                    let mut ad = l.approach_dist.unwrap_or(0.0);
                    let mut on = l.approach_dist.is_some();
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, tr("Detector", "Детектор")).on_hover_text(tr("`[approachdist]`: how far before the stop line a vehicle registers a request", "`[approachdist]`: за сколько метров до стоп-линии машина запрашивает")).changed() {
                            l.approach_dist = if on { Some(if ad > 0.0 { ad } else { 10.0 }) } else { None };
                            ad = l.approach_dist.unwrap_or(0.0);
                            changed = true;
                        }
                        if l.approach_dist.is_some() && ui.add(egui::DragValue::new(&mut ad).speed(0.2).range(0.0..=200.0).suffix(" m")).changed() {
                            l.approach_dist = Some(ad.max(0.0));
                            changed = true;
                        }
                    });
                    let mut phase_gone = None;
                    for (q, ph) in l.phases.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let mut st = ph.state;
                            egui::ComboBox::from_id_salt(("crossing phase", n, q)).selected_text(format!("{st}: {}", aspect_name(st))).width(130.0).show_ui(ui, |ui| {
                                for a in ASPECTS {
                                    ui.selectable_value(&mut st, a, format!("{a}: {}", aspect_name(a)));
                                }
                            });
                            if st != ph.state {
                                ph.state = st;
                                changed = true;
                            }
                            changed |= ui.add(egui::DragValue::new(&mut ph.duration).speed(0.2).range(0.0..=600.0).suffix(" s")).changed();
                            if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                                phase_gone = Some(q);
                            }
                        });
                    }
                    if let Some(q) = phase_gone {
                        l.phases.remove(q);
                        changed = true;
                    }
                    if ui.button(icons::labelled(Icon::Add, tr("A phase", "Фаза"))).clicked() {
                        l.phases.push(omsi_scenery::TrafficLightPhase { state: 0, duration: 10.0 });
                        changed = true;
                    }
                    if ui.small_button(icons::labelled(Icon::Delete, tr("Remove this light", "Удалить этот светофор"))).clicked() {
                        gone = Some(n);
                    }
                });
            }
            if let Some(n) = gone {
                sheet.crossing.remove_light(n);
                changed = true;
            }
            if ui.button(icons::labelled(Icon::Add, tr("A light", "Светофор"))).clicked() {
                sheet.crossing.lights.push(omsi_scenery::TrafficLight { name: format!("Light {}", sheet.crossing.lights.len() + 1), phases: vec![omsi_scenery::TrafficLightPhase { state: 0, duration: 20.0 }, omsi_scenery::TrafficLightPhase { state: 6, duration: 20.0 }], approach_dist: None });
                changed = true;
            }
            if sheet.crossing.lights.is_empty() {
                ui.label(RichText::new(tr("A crossing with lights needs the signal heads on the map: the game only runs a program that heads are linked to", "Перекрёстку со светофором нужны головки на карте: игра запускает программу, только если к ней привязаны головки")).small().color(DIM));
            }
            // the request loops, as the numbers they are
            egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Request loops", "Петли запроса"), sheet.crossing.stops.len() + sheet.crossing.jumps.len())).small()).id_salt("crossing stops").default_open(false).show(ui, |ui| {
                let mut stop_gone = None;
                for (n, s) in sheet.crossing.stops.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("stop").small().color(DIM));
                        changed |= ui.add(egui::DragValue::new(&mut s[0]).speed(0.2).prefix("L")).changed();
                        changed |= ui.add(egui::DragValue::new(&mut s[1]).speed(0.2).suffix(" s")).changed();
                        changed |= ui.add(egui::DragValue::new(&mut s[2]).range(0.0..=1.0).speed(0.1).prefix("req ")).changed();
                        if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                            stop_gone = Some(n);
                        }
                    });
                }
                if let Some(n) = stop_gone {
                    sheet.crossing.stops.remove(n);
                    changed = true;
                }
                let mut jump_gone = None;
                for (n, j) in sheet.crossing.jumps.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("jump").small().color(DIM));
                        changed |= ui.add(egui::DragValue::new(&mut j[0]).speed(0.2).prefix("L")).changed();
                        changed |= ui.add(egui::DragValue::new(&mut j[1]).speed(0.2).suffix(" s")).changed();
                        changed |= ui.add(egui::DragValue::new(&mut j[2]).range(0.0..=1.0).speed(0.1).prefix("req ")).changed();
                        changed |= ui.add(egui::DragValue::new(&mut j[3]).speed(0.2).prefix("→ ").suffix(" s")).changed();
                        if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                            jump_gone = Some(n);
                        }
                    });
                }
                if let Some(n) = jump_gone {
                    sheet.crossing.jumps.remove(n);
                    changed = true;
                }
            });
        });

        // ---- the spline helpers
        ui.separator();
        let mut helper_gone = None;
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Spline helpers", "Точки привязки сплайнов"), sheet.crossing.helpers.len())).strong()).id_salt("crossing helpers").default_open(false).show(ui, |ui| {
            ui.label(RichText::new(tr("Where a road of the map snaps onto the crossing. x, height, forward in the object's frame", "Где дорога карты пристыковывается к перекрёстку. x, высота, вперёд в системе объекта")).small().color(DIM));
            for (n, h) in sheet.crossing.helpers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{n}")).small().color(DIM));
                    changed |= ui.add(egui::TextEdit::singleline(&mut h.spline).desired_width(120.0)).on_hover_text(tr("The road type it takes (a `.sli`)", "Тип дороги, который он принимает (`.sli`)")).changed();
                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                        helper_gone = Some(n);
                    }
                });
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut h.x).speed(0.05).prefix("x ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.fwd).speed(0.05).prefix("f ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.up).speed(0.05).prefix("h ")).changed();
                });
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut h.heading).speed(0.5).suffix("°")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.length).speed(0.05).prefix("len ")).changed();
                    changed |= ui.add(egui::DragValue::new(&mut h.radius).speed(0.1).prefix("r ")).changed();
                });
            }
            if let Some(n) = helper_gone {
                sheet.crossing.helpers.remove(n);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button(icons::labelled(Icon::Add, tr("A helper", "Точка"))).clicked() {
                    let spline = ed.road_type.clone().unwrap_or_default();
                    sheet.crossing.helpers.push(Helper { spline, x: 0.0, up: 0.0, fwd: 0.0, heading: 0.0, length: 0.1, radius: 0.0 });
                    changed = true;
                }
                if ui.button(tr("At the picked end", "У выбранного конца")).on_hover_text(tr("A helper where the picked path ends, facing out of the crossing", "Точка там, где кончается выбранный путь, наружу от перекрёстка")).clicked() {
                    if let Some(p) = self.selected.and_then(|i| sheet.crossing.paths.get(i)) {
                        let at = p.end();
                        let c = p.curve();
                        let heading = c.heading_at(c.length) as f32;
                        let spline = ed.road_type.clone().unwrap_or_default();
                        sheet.crossing.helpers.push(Helper { spline, x: at.x as f32, up: at.z as f32, fwd: at.y as f32, heading, length: 0.1, radius: 0.0 });
                        changed = true;
                    }
                }
            });
        });

        if changed {
            // (nothing to do: the view and the map both draw the sheet as it is each frame;
            // the file is only written on Save)
            let _ = changed;
        }
    }

    /// Above the view: save, revert, the zoom.
    fn toolbar(&mut self, ui: &mut egui::Ui, ed: &mut Editor, roots: Option<&Roots>, mut assets: Option<&mut Assets>) {
        let has = self.sheet.is_some();
        let mut save = false;
        let mut revert = false;
        ui.horizontal(|ui| {
            let label = if self.copied { tr("Save", "Сохранить") } else { tr("Save as a copy", "Сохранить копию") };
            let hint = if self.copied {
                tr("Write the paths into openMapper's copy of the object", "Записать пути в копию объекта openMapper")
            } else {
                tr("The object's folder is copied into openMapper's, the paths written there, and the map's object pointed at it: the original file is not touched", "Папка объекта копируется в openMapper, пути пишутся туда, объект карты начинает ссылаться на неё: исходный файл не трогается")
            };
            save = ui.add_enabled(has, egui::Button::new(icons::labelled(Icon::Save, label))).on_hover_text(hint).clicked();
            revert = ui.add_enabled(has, egui::Button::new(icons::labelled(Icon::Undo, tr("Revert", "Отменить")))).on_hover_text(tr("Back to how the object was when it was opened (the file is not touched)", "Обратно к тому, каким объект был при открытии (файл не трогается)")).clicked();
            ui.separator();
            if ui.button("−").on_hover_text(tr("Zoom out", "Отдалить")).clicked() {
                self.scale = (self.scale / 1.35).clamp(0.6, 400.0);
            }
            if ui.button("+").on_hover_text(tr("Zoom in", "Приблизить")).clicked() {
                self.scale = (self.scale * 1.35).clamp(0.6, 400.0);
            }
            if ui.button(tr("Fit", "Вписать")).on_hover_text(tr("The whole crossing in view", "Весь перекрёсток в окне")).clicked() {
                self.fit = true;
            }
        });
        ui.label(RichText::new(tr("Drag an end to move it. Ctrl snaps it to another path's end. Wheel zooms, right or middle drag pans", "Тяните конец, чтобы сдвинуть его. Ctrl притягивает к концу другого пути. Колесо — масштаб, правая/средняя кнопка — сдвиг")).small().color(DIM));
        if let Some((msg, bad)) = self.message.clone() {
            ui.label(RichText::new(msg).small().color(if bad { BAD } else { GOOD }));
        }
        if revert {
            if let Some(sheet) = self.sheet.as_mut() {
                sheet.crossing = self.seen.clone();
            }
            self.drag = None;
            self.message = Some((tr("Back to how it was opened", "Обратно к состоянию при открытии").into(), false));
        }
        if save {
            if let Some(roots) = roots {
                if let Some(assets) = assets.as_deref_mut() {
                    self.save(ed, roots, assets);
                }
            }
        }
    }

    fn save(&mut self, ed: &mut Editor, roots: &Roots, assets: &mut Assets) {
        let Some(Sel::Object(k, id)) = self.target else { return };
        let Some(sheet) = self.sheet.as_ref() else { return };
        let crossing = sheet.crossing.clone();
        let text = sheet.text();
        let (rel, abs) = if self.copied {
            (self.rel.clone(), self.abs.clone())
        } else {
            let map = ed.doc.as_ref().map(crate::junction::folder_name).unwrap_or_else(|| "map".into());
            let stem = std::path::Path::new(&self.rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "crossing".into());
            match crossing_edit::copy_into(roots, &self.rel, &map, &stem) {
                Ok(r) => {
                    let a = omsi_cfg::resolve_path(&roots.content, &r);
                    (r, a)
                }
                Err(e) => {
                    self.message = Some((e.to_string(), true));
                    return;
                }
            }
        };
        if let Err(e) = crossing_edit::write(&abs, &text, &self.bytes) {
            self.message = Some((e.to_string(), true));
            return;
        }
        if let Err(e) = crossing_edit::verify(&abs, &crossing) {
            self.message = Some((e.to_string(), true));
            return;
        }
        // (the mark that this file is the editor's own: the next save writes it in place)
        let _ = std::fs::write(sidecar(&abs), format!("{{\"from\": {:?}}}", self.rel.replace('\\', "\\\\")));
        let retarget = !self.copied;
        self.copied = true;
        self.rel = rel.clone();
        self.abs = abs.clone();
        if let Ok(b) = std::fs::read(&abs) {
            self.bytes = b;
        }
        self.seen = crossing;
        crate::hot::own_change();
        assets.objects.remove(&rel.to_lowercase());
        let _ = assets.object(&rel);
        if retarget {
            ed.set_object_file(k, id, &rel);
        }
        self.message = Some((tr("Saved", "Сохранено").into(), false));
    }

    // ------------------------------------------------------------ the view

    fn fit_to(&mut self, size: Vec2, sheet: &Sheet) {
        let mut lo = DVec2::splat(f64::MAX);
        let mut hi = DVec2::splat(f64::MIN);
        for p in &sheet.crossing.paths {
            for q in p.points(1.0) {
                lo = lo.min(q.truncate());
                hi = hi.max(q.truncate());
            }
        }
        for h in &sheet.crossing.helpers {
            lo = lo.min(h.local().truncate());
            hi = hi.max(h.local().truncate());
        }
        if lo.x > hi.x {
            lo = DVec2::splat(-15.0);
            hi = DVec2::splat(15.0);
        }
        let centre = (lo + hi) / 2.0;
        let span = (hi - lo).max(DVec2::splat(12.0)) * 1.25;
        let s = (size.x as f64 / span.x).min(size.y as f64 / span.y);
        self.centre = centre;
        self.scale = (s as f32).clamp(0.6, 400.0);
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let Some(mut sheet) = self.sheet.take() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(tr("Nothing to edit yet", "Пока нечего править")).color(DIM));
            });
            return;
        };
        self.canvas_ui(ui, &mut sheet);
        self.sheet = Some(sheet);
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui, sheet: &mut Sheet) {
        let size = Vec2::new(ui.available_width().max(320.0), ui.available_height().max(320.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if self.fit {
            self.fit = false;
            self.fit_to(rect.size(), sheet);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let pinch = ui.input(|i| i.zoom_delta());
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            let factor = (1.0 + scroll * 0.0025).max(0.2) * pinch;
            if factor != 1.0 {
                if let Some(h) = resp.hover_pos() {
                    let (scale, centre) = (self.scale, self.centre);
                    let before = DVec2::new(centre.x + (h.x - rect.center().x) as f64 / scale as f64, centre.y - (h.y - rect.center().y) as f64 / scale as f64);
                    self.scale = (self.scale * factor).clamp(0.6, 400.0);
                    let after = DVec2::new(centre.x + (h.x - rect.center().x) as f64 / self.scale as f64, centre.y - (h.y - rect.center().y) as f64 / self.scale as f64);
                    self.centre += before - after;
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            self.centre -= DVec2::new(d.x as f64, -d.y as f64) / self.scale as f64;
        }
        let (scale, centre) = (self.scale, self.centre);
        let to_screen = move |p: DVec2| Pos2::new(rect.center().x + ((p.x - centre.x) * scale as f64) as f32, rect.center().y - ((p.y - centre.y) * scale as f64) as f32);
        let from_screen = move |q: Pos2| DVec2::new(centre.x + (q.x - rect.center().x) as f64 / scale as f64, centre.y - (q.y - rect.center().y) as f64 / scale as f64);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, Color32::from_rgb(30, 34, 31));
        // the grid: metres close, tens away
        {
            let step = if scale >= 14.0 { 1.0 } else if scale >= 4.0 { 5.0 } else if scale >= 1.2 { 10.0 } else { 50.0 };
            let (lo, hi) = (from_screen(rect.left_bottom()), from_screen(rect.right_top()));
            let line = |a: DVec2, b: DVec2, zero: bool, major: bool| painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(if zero { 1.5 } else { 1.0 }, Color32::from_white_alpha(if zero { 70 } else if major { 26 } else { 12 })));
            let mut x = (lo.x / step).floor() * step;
            while x <= hi.x {
                line(DVec2::new(x, lo.y), DVec2::new(x, hi.y), x == 0.0, (x / (step * 5.0)).fract().abs() < 1e-6);
                x += step;
            }
            let mut y = (lo.y / step).floor() * step;
            while y <= hi.y {
                line(DVec2::new(lo.x, y), DVec2::new(hi.x, y), y == 0.0, (y / (step * 5.0)).fract().abs() < 1e-6);
                y += step;
            }
        }
        // the object's own frame: x to the right, y (forward) up the page
        painter.line_segment([to_screen(DVec2::new(-3.0, 0.0)), to_screen(DVec2::new(3.0, 0.0))], Stroke::new(1.4, Color32::from_rgb(150, 90, 90)));
        painter.line_segment([to_screen(DVec2::new(0.0, -3.0)), to_screen(DVec2::new(0.0, 3.0))], Stroke::new(1.4, Color32::from_rgb(90, 150, 90)));

        // the paths
        for (i, p) in sheet.crossing.paths.iter().enumerate() {
            let on = self.selected == Some(i);
            let base = if on { PICKED } else { kind_color(p.kind()) };
            let pts = p.points(0.5);
            // its edges, so the width is seen
            let half = (p.width() / 2.0).max(0.0);
            let mut left = Vec::with_capacity(pts.len());
            let mut right = Vec::with_capacity(pts.len());
            for (n, q) in pts.iter().enumerate() {
                let t = if n + 1 < pts.len() { pts[n + 1] - *q } else if n > 0 { *q - pts[n - 1] } else { DVec3::Y };
                let t = t.truncate();
                let len = t.length();
                let tangent = if len > 1e-9 { t / len } else { DVec2::Y };
                let normal = DVec2::new(tangent.y, -tangent.x);
                left.push(q.truncate() + normal * half);
                right.push(q.truncate() - normal * half);
            }
            let edge = Stroke::new(1.0, base.gamma_multiply(0.55));
            for w in left.windows(2) {
                painter.line_segment([to_screen(w[0]), to_screen(w[1])], edge);
            }
            for w in right.windows(2) {
                painter.line_segment([to_screen(w[0]), to_screen(w[1])], edge);
            }
            for w in pts.windows(2) {
                painter.line_segment([to_screen(w[0].truncate()), to_screen(w[1].truncate())], Stroke::new(if on { 2.0 } else { 1.4 }, base));
            }
            // the way it goes
            let dir_arrow = |at: DVec2, along: DVec2, color: Color32| {
                let len = along.length();
                if len < 1e-6 {
                    return;
                }
                let d = along / len;
                let n = DVec2::new(-d.y, d.x);
                let tip = at + d * (10.0 / scale as f64);
                painter.add(egui::Shape::convex_polygon(vec![to_screen(tip), to_screen(at + n * (4.0 / scale as f64)), to_screen(at - n * (4.0 / scale as f64))], color, Stroke::NONE));
            };
            if let (Some(a), Some(b)) = (pts.first(), pts.last()) {
                let mid = (a.truncate() + b.truncate()) / 2.0;
                match p.direction() {
                    1 => dir_arrow(mid, a.truncate() - b.truncate(), base),
                    2 => {
                        dir_arrow(mid, b.truncate() - a.truncate(), base);
                        dir_arrow(mid, a.truncate() - b.truncate(), base);
                    }
                    _ => dir_arrow(mid, b.truncate() - a.truncate(), base),
                }
            }
            // its ends
            for at in [p.start().truncate(), p.end().truncate()] {
                let s = to_screen(at);
                painter.circle_filled(s, if on { 4.0 } else { 3.0 }, if on { HANDLE } else { base });
                painter.circle_stroke(s, 6.0, Stroke::new(1.0, Color32::from_white_alpha(if on { 120 } else { 40 })));
            }
        }
        // the spline helpers: where roads snap on
        for h in &sheet.crossing.helpers {
            let at = to_screen(h.local().truncate());
            painter.circle_stroke(at, 5.0, Stroke::new(1.6, SNAPPED));
            let d = crate::roads::dir(h.heading as f64);
            painter.line_segment([at, to_screen(h.local().truncate() + d * 4.0)], Stroke::new(1.4, SNAPPED));
        }

        // ---- the mouse
        let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let at = |pos: Pos2| -> DVec2 { from_screen(pos) };
        let handle = |sheet: &Sheet, pos: Pos2| -> Option<(usize, bool)> {
            let mut best: Option<(f32, usize, bool)> = None;
            for (i, p) in sheet.crossing.paths.iter().enumerate() {
                for (at_end, q) in [(true, p.end()), (false, p.start())] {
                    let d = to_screen(q.truncate()).distance(pos);
                    if d <= GRAB && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, i, at_end));
                    }
                }
            }
            best.map(|b| (b.1, b.2))
        };
        let path_at = |sheet: &Sheet, pos: Pos2| -> Option<usize> {
            let mut best: Option<(f32, usize)> = None;
            for (i, p) in sheet.crossing.paths.iter().enumerate() {
                for q in p.points(0.5) {
                    let d = to_screen(q.truncate()).distance(pos);
                    if d <= GRAB && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, i));
                    }
                }
            }
            best.map(|b| b.1)
        };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                self.drag = handle(sheet, pos);
                if self.drag.is_none() {
                    self.selected = path_at(sheet, pos).or(self.selected);
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let (Some((i, at_end)), Some(pos)) = (self.drag, resp.interact_pointer_pos()) {
                if i < sheet.crossing.paths.len() {
                    let mut q = at(pos);
                    let mut snapped: Option<f64> = None;
                    if ctrl {
                        if let Some((j, other_end, hit)) = sheet.crossing.nearest_end(DVec3::new(q.x, q.y, 0.0), crate::lanes::LINK, i) {
                            q = hit.truncate();
                            // (the way out of the other path is the way out of this one)
                            let c = sheet.crossing.paths[j].curve();
                            snapped = Some(if other_end { c.heading_at(c.length) } else { c.heading_at(0.0) });
                        }
                    }
                    let p = &mut sheet.crossing.paths[i];
                    if at_end {
                        p.set_end(DVec3::new(q.x, q.y, p.end().z));
                        if let (Some(out), true) = (snapped, p.radius().abs() < 1e-6) {
                            // (arriving along it: the same way round)
                            p.v[3] = crate::roads::norm_deg(out + 180.0) as f32;
                            let chord = (p.end().truncate() - p.start().truncate()).length();
                            p.v[5] = chord as f32;
                        }
                    } else {
                        p.set_start(DVec3::new(q.x, q.y, p.start().z));
                        if let (Some(out), true) = (snapped, p.radius().abs() < 1e-6) {
                            let end = p.end();
                            p.v[3] = crate::roads::norm_deg(out) as f32;
                            p.set_end(end);
                        }
                    }
                }
            }
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag = None;
        }
        if resp.clicked() && self.drag.is_none() {
            if let Some(pos) = resp.interact_pointer_pos() {
                self.selected = path_at(sheet, pos);
            }
        }
        if resp.hovered() {
            if let Some(pos) = resp.hover_pos() {
                if handle(sheet, pos).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }
        // the picked path's ends, again, over everything: these are what is dragged
        if let Some(i) = self.selected.filter(|i| *i < sheet.crossing.paths.len()) {
            let p = &sheet.crossing.paths[i];
            for at in [p.start().truncate(), p.end().truncate()] {
                painter.circle_stroke(to_screen(at), 9.0, Stroke::new(1.6, PICKED));
            }
        }
    }
}
