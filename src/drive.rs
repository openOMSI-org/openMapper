//! Test drive: a window to choose where the bus appears (where the camera is, a start point
//! or a bus stop), which vehicle (or none), the time and the weather, and the one openOMSI
//! session that comes of it. The session is kept track of: while it runs the button of the
//! bottom bar closes it, and a second one is not started beside it.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};

use crate::doc::{tile_origin, Document};
use crate::i18n::tr;
use crate::install::Roots;
use crate::theme::pal;

/// Whether a session started from here is running (the bottom bar reads it).
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// What the window remembers between drives (and between runs of the program).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DriveSettings {
    /// The vehicle's file (`Vehicles/MAN_SD200/MAN_SD80.bus`); empty: the first one found.
    pub bus: String,
    /// No vehicle: on foot where the bus would stand.
    pub on_foot: bool,
    /// Time of day, minutes after midnight.
    pub minutes: u32,
    /// A weather file (`Weather/Fog.owt`), `natural`, `cycle`; empty: the map's own.
    pub weather: String,
    /// How many AI vehicles round the player, 0 to 100.
    pub traffic: u32,
    pub passengers: bool,
}

impl Default for DriveSettings {
    fn default() -> Self {
        DriveSettings { bus: String::new(), on_foot: false, minutes: 9 * 60, weather: String::new(), traffic: 30, passengers: true }
    }
}

/// Where the player appears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spawn {
    /// On the ground the camera looks at, facing the way the camera does.
    Camera,
    /// A start point of the map (its place in global.cfg's list).
    Start(usize),
    /// A bus stop (its object's id).
    Stop(i64),
    /// A free camera where the view is; no vehicle.
    View,
}

/// What a click on Start asks for.
#[derive(Debug, Clone)]
pub struct Request {
    pub spawn: Spawn,
    pub settings: DriveSettings,
}

/// A vehicle the installation has.
#[derive(Debug, Clone)]
struct Bus {
    rel: String,
    label: String,
}

#[derive(Default)]
pub struct Drive {
    open: bool,
    spawn: Option<Spawn>,
    buses: Vec<Bus>,
    weathers: Vec<String>,
    /// The vehicles and weathers are looked for when the window opens.
    read: bool,
    stop_filter: String,
    child: Option<Child>,
}

impl Drive {
    /// The bottom bar's button: close the session that runs, else ask what to drive.
    pub fn button(&mut self) {
        if self.is_running() {
            self.close();
        } else {
            self.open = true;
            self.read = false;
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.poll();
        self.child.is_some()
    }

    /// Notice a session that has ended.
    pub fn poll(&mut self) {
        if let Some(c) = &mut self.child {
            if !matches!(c.try_wait(), Ok(None)) {
                self.child = None;
            }
        }
        RUNNING.store(self.child.is_some(), Ordering::Relaxed);
    }

    /// End the session.
    pub fn close(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        RUNNING.store(false, Ordering::Relaxed);
    }

    /// Keep the process just started.
    pub fn started(&mut self, child: Child) {
        self.child = Some(child);
        RUNNING.store(true, Ordering::Relaxed);
        self.open = false;
    }

    /// The window. Returns the drive asked for.
    pub fn window(&mut self, ctx: &egui::Context, doc: &Document, roots: Option<&Roots>, settings: &mut DriveSettings) -> Option<Request> {
        if !self.open {
            return None;
        }
        if !self.read {
            self.read = true;
            if let Some(r) = roots {
                self.buses = buses(r);
                self.weathers = weathers(r);
            }
            if self.spawn.is_none() {
                self.spawn = Some(if doc.global.entry_points.is_empty() { Spawn::Camera } else { Spawn::Start(0) });
            }
        }
        let mut go = None;
        let mut open = true;
        let stops = crate::stops::all(doc);
        egui::Window::new(tr("Test drive", "Тест-драйв")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
            ui.set_width(380.0);
            let spawn = self.spawn.get_or_insert(Spawn::Camera);
            let name_of = |s: &Spawn| match s {
                Spawn::Camera => tr("Where the camera looks", "Где смотрит камера").to_string(),
                Spawn::Start(i) => doc.global.entry_points.get(*i).map(|e| format!("{} «{}»", tr("Start point", "Точка старта"), e.name)).unwrap_or_else(|| tr("Start point", "Точка старта").to_string()),
                Spawn::Stop(id) => stops.iter().find(|f| f.id == *id).map(|f| format!("{} «{}»", tr("Bus stop", "Остановка"), f.name)).unwrap_or_else(|| tr("Bus stop", "Остановка").to_string()),
                Spawn::View => tr("See this view in openOMSI", "Посмотреть этот вид в openOMSI").to_string(),
            };
            egui::Grid::new("drive grid").num_columns(2).spacing(egui::Vec2::new(10.0, 8.0)).show(ui, |ui| {
                ui.label(RichText::new(tr("Start from", "Откуда ехать")).color(pal().text_dim));
                egui::ComboBox::from_id_salt("drive spawn").width(260.0).selected_text(name_of(spawn)).show_ui(ui, |ui| {
                    ui.selectable_value(spawn, Spawn::Camera, name_of(&Spawn::Camera));
                    for (i, e) in doc.global.entry_points.iter().enumerate() {
                        let s = Spawn::Start(i);
                        ui.selectable_value(spawn, s, format!("{} «{}»", tr("Start point", "Точка старта"), e.name));
                    }
                    ui.selectable_value(spawn, Spawn::View, name_of(&Spawn::View));
                    if !stops.is_empty() {
                        ui.separator();
                        ui.add(egui::TextEdit::singleline(&mut self.stop_filter).hint_text(tr("Search", "Поиск")).desired_width(240.0));
                        let q = self.stop_filter.trim().to_lowercase();
                        let mut shown = 0;
                        for f in stops.iter().filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q)) {
                            if shown >= 60 {
                                break;
                            }
                            shown += 1;
                            ui.selectable_value(spawn, Spawn::Stop(f.id), format!("{} «{}»", tr("Bus stop", "Остановка"), f.name));
                        }
                    }
                });
                ui.end_row();

                let free_camera = *spawn == Spawn::View;
                ui.add_enabled_ui(!free_camera, |ui| ui.label(RichText::new(tr("Vehicle", "Транспорт")).color(pal().text_dim)));
                ui.add_enabled_ui(!free_camera, |ui| {
                    let current = if settings.on_foot {
                        tr("No vehicle (on foot)", "Без транспорта (пешком)").to_string()
                    } else {
                        self.buses.iter().find(|b| b.rel.eq_ignore_ascii_case(&settings.bus)).or(self.buses.first()).map(|b| b.label.clone()).unwrap_or_else(|| tr("No vehicle (on foot)", "Без транспорта (пешком)").to_string())
                    };
                    egui::ComboBox::from_id_salt("drive bus").width(260.0).selected_text(current).show_ui(ui, |ui| {
                        if ui.selectable_label(settings.on_foot, tr("No vehicle (on foot)", "Без транспорта (пешком)")).clicked() {
                            settings.on_foot = true;
                        }
                        for b in &self.buses {
                            let chosen = !settings.on_foot && (b.rel.eq_ignore_ascii_case(&settings.bus) || (settings.bus.is_empty() && self.buses.first().map(|f| f.rel == b.rel).unwrap_or(false)));
                            if ui.selectable_label(chosen, &b.label).clicked() {
                                settings.bus = b.rel.clone();
                                settings.on_foot = false;
                            }
                        }
                    });
                });
                ui.end_row();

                ui.label(RichText::new(tr("Time of day", "Время суток")).color(pal().text_dim));
                ui.horizontal(|ui| {
                    let (mut h, mut m) = (settings.minutes / 60 % 24, settings.minutes % 60);
                    ui.add(egui::DragValue::new(&mut h).range(0..=23).custom_formatter(|v, _| format!("{:02}", v as u32)));
                    ui.label(":");
                    ui.add(egui::DragValue::new(&mut m).range(0..=59).custom_formatter(|v, _| format!("{:02}", v as u32)));
                    settings.minutes = h * 60 + m;
                });
                ui.end_row();

                ui.label(RichText::new(tr("Weather", "Погода")).color(pal().text_dim));
                let weather_name = |w: &str| match w {
                    "" => tr("The map's own", "Как в карте").to_string(),
                    "natural" => tr("Natural weather", "Естественная погода").to_string(),
                    "cycle" => tr("Weather cycle", "Смена погоды").to_string(),
                    f => Path::new(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| f.to_string()),
                };
                egui::ComboBox::from_id_salt("drive weather").width(260.0).selected_text(weather_name(&settings.weather)).show_ui(ui, |ui| {
                    for w in ["", "natural", "cycle"] {
                        ui.selectable_value(&mut settings.weather, w.to_string(), weather_name(w));
                    }
                    for w in &self.weathers {
                        ui.selectable_value(&mut settings.weather, w.clone(), weather_name(w));
                    }
                });
                ui.end_row();

                ui.label(RichText::new(tr("Traffic", "Движение")).color(pal().text_dim));
                ui.add(egui::Slider::new(&mut settings.traffic, 0..=100));
                ui.end_row();

                ui.label("");
                ui.checkbox(&mut settings.passengers, tr("Passengers at the stops", "Пассажиры на остановках"));
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let start = ui.add(egui::Button::new(RichText::new(tr("Start", "Старт")).strong().color(Color32::WHITE)).fill(Color32::from_rgb(46, 134, 255)));
                if start.clicked() || ctx.input(|i| i.key_pressed(egui::Key::Enter)) && !ctx.egui_wants_keyboard_input() {
                    // (no vehicle chosen yet: the first one, as the list shows it)
                    if !settings.on_foot && settings.bus.is_empty() {
                        settings.bus = self.buses.first().map(|b| b.rel.clone()).unwrap_or_default();
                    }
                    go = Some(Request { spawn: *spawn, settings: settings.clone() });
                }
                if ui.button(tr("Cancel", "Отмена")).clicked() {
                    self.open = false;
                }
            });
        });
        if !open {
            self.open = false;
        }
        if go.is_some() {
            self.open = false;
        }
        go
    }
}

/// The vehicles of the content folder and the installation: `Vehicles/<folder>/<name>.bus`.
fn buses(roots: &Roots) -> Vec<Bus> {
    let mut out: Vec<Bus> = Vec::new();
    for root in roots.all() {
        let Some(vehicles) = child_dir(root, "Vehicles") else { continue };
        let Ok(rd) = std::fs::read_dir(&vehicles) else { continue };
        for dir in rd.flatten().filter(|e| e.path().is_dir()) {
            let folder = dir.file_name().to_string_lossy().to_string();
            let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                if !name.to_lowercase().ends_with(".bus") {
                    continue;
                }
                let rel = format!("Vehicles/{folder}/{name}");
                if !out.iter().any(|b| b.rel.eq_ignore_ascii_case(&rel)) {
                    out.push(Bus { label: format!("{folder} / {}", name.trim_end_matches(".bus").trim_end_matches(".BUS")), rel });
                }
            }
        }
    }
    out.sort_by_cached_key(|b| b.label.to_lowercase());
    out
}

/// The weather files: `Weather/<name>.owt`.
fn weathers(roots: &Roots) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for root in roots.all() {
        let Some(dir) = child_dir(root, "Weather") else { continue };
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for f in rd.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.to_lowercase().ends_with(".owt") {
                let rel = format!("Weather/{name}");
                if !out.iter().any(|w| w.eq_ignore_ascii_case(&rel)) {
                    out.push(rel);
                }
            }
        }
    }
    out.sort_by_cached_key(|w| w.to_lowercase());
    out
}

/// A folder below `root` by name, whatever its letters' case.
fn child_dir(root: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

/// openOMSI's arguments for a drive. `cam` is the point looked at and the camera's heading
/// (degrees clockwise from north, the vehicle's too); `eye` the camera's place and tilt for
/// the free view.
pub fn args(req: &Request, doc: &Document, target: glam::DVec2, yaw: f64, view: Vec<String>) -> Vec<String> {
    let s = &req.settings;
    let mut a: Vec<String> = Vec::new();
    if req.spawn == Spawn::View {
        // (the camera alone starts a session without the menu)
        a.extend(view);
    } else {
        a.push("--no-menu".into());
        if s.on_foot {
            a.push("--on-foot".into());
        }
        match req.spawn {
            Spawn::Camera => a.extend(["--spawn".into(), format!("{:.2},{:.2},{:.1}", target.x, target.y, yaw.rem_euclid(360.0))]),
            Spawn::Start(i) => a.extend(["--entry".into(), i.to_string()]),
            Spawn::Stop(id) => {
                if let Some((k, o)) = crate::selection::object_by_id(doc, id) {
                    let at = tile_origin(k) + glam::DVec2::new(o.pos[0], o.pos[1]);
                    let heading = o.rot[0];
                    // (a stop stands beside the road, its platform on one side: the bus is
                    // put a lane's width to the other)
                    let side = match o.extra.get(5).map(|v| v.trim()) {
                        Some("1") => 1.0,
                        Some("2") => 0.0,
                        _ => -1.0,
                    };
                    let h = heading.to_radians();
                    let right = glam::DVec2::new(h.cos(), -h.sin());
                    let at = at + right * side * 3.0;
                    a.extend(["--spawn".into(), format!("{:.2},{:.2},{:.1}", at.x, at.y, heading.rem_euclid(360.0))]);
                }
            }
            Spawn::View => {}
        }
        if !s.on_foot && !s.bus.is_empty() {
            a.extend(["--bus".into(), s.bus.clone()]);
        }
    }
    a.extend(["--time".into(), format!("{:02}:{:02}", s.minutes / 60 % 24, s.minutes % 60)]);
    if !s.weather.is_empty() {
        a.extend(["--weather".into(), s.weather.clone()]);
    }
    a.extend(["--traffic".into(), s.traffic.min(100).to_string()]);
    if s.passengers {
        a.push("--passengers".into());
    }
    a
}
