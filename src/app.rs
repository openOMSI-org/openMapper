//! The window and its frame: input to the camera and the editor, the interface, drawing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use glam::{DVec2, DVec3};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use crate::controls::{Cmd, Keymap, Preset};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::assets::Assets;
use crate::camera::Camera;
use crate::doc::{tile_origin, tile_size, Document, Sel};
use crate::editor::{Editor, Tool};
use crate::gpu::Gpu;
use crate::i18n::{self, tr};
use crate::install::{self, Prefs, Roots};
use crate::library::Library;
use crate::render::Renderer;
use crate::ui::{self, Action, UiState};

#[path = "app_mesh.rs"]
pub(crate) mod mesh;
pub use mesh::bring_picture;

struct Gfx {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

#[derive(Default)]
struct Mouse {
    pos: (f32, f32),
    left: bool,
    right: bool,
    middle: bool,
    right_moved: f32,
    /// Where the left button went down.
    press: (f32, f32),
    /// Left button dragging the map (pressed on empty ground with the Select tool).
    grab: bool,
    /// Left button with Option / Alt held: turns the camera (for a trackpad).
    alt_orbit: bool,
    /// The world point a drag of the map holds under the cursor.
    grab_anchor: Option<DVec3>,
    over_ui: bool,
    moved: bool,
    /// The cursor is held and hidden while the right button looks round, and the turn comes
    /// from the mouse's own motion (not from where the cursor is, which jumps at the screen's edge).
    look_raw: bool,
    /// The mouse's motion since the last frame, applied once there.
    look_acc: (f64, f64),
}

#[derive(Default)]
struct Keys {
    /// The arrow keys: up, left, down, right (they move over the map in any camera style).
    arrows: [bool; 4],
    /// How long the flying keys have been held (s).
    fly_held: f64,
    w: bool,
    a: bool,
    s: bool,
    d: bool,
    q: bool,
    e: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
}

pub struct App {
    gfx: Option<Gfx>,
    prefs: Prefs,
    roots: Option<Roots>,
    assets: Option<Assets>,
    lib: Library,
    /// Watches the content folders: new and changed objects, splines and textures are read
    /// again by themselves.
    hot: Option<crate::hot::Watcher>,
    ed: Editor,
    /// Whether the selected object's type is openMapper's own (asked once per type).
    mesh_known: Option<(String, bool)>,
    /// OPENMAPPER_EDIT, done once the selected object's source is open.
    hook_edit: Option<String>,
    /// The tilt and heading the camera had before the plan view took over.
    pre_2d: Option<(f64, f64)>,
    /// A cursor held on the ground for a picture (OPENMAPPER_KNIFE), whatever the mouse does.
    pinned_cursor: Option<DVec3>,
    cam: Camera,
    /// The place the loaded window of tiles is round (tile streaming on).
    stream_at: Option<DVec2>,
    /// When the last input came: a window nobody touches redraws slower.
    last_input: Instant,
    /// The tiles loaded now, and what they were made from.
    stream_set: Option<(Option<std::collections::HashSet<crate::doc::TileKey>>, (usize, u64, u64))>,
    ui: UiState,
    mouse: Mouse,
    keys: Keys,
    last: Instant,
    swatches: HashMap<String, egui::TextureHandle>,
    drive: crate::drive::Drive,
    texture_files: Vec<String>,
    /// The ground textures being looked for (on a thread of its own: a big installation's
    /// folders took long enough to freeze the window).
    texture_scan: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    /// The maps of the installation, listed on a thread of their own at the start.
    maps_scan: Option<std::sync::mpsc::Receiver<Vec<install::MapEntry>>>,
    /// How many assets the switch catalogue was made from (made again when the library changes).
    switches_for: usize,
    game: Option<PathBuf>,
    /// Open this map once the window is there (the command line).
    open_at_start: Option<PathBuf>,
    quit: bool,
    /// The 3D view between the panels (points).
    view: egui::Rect,
    frames: u64,
    /// `OPENMAPPER_SHOT=file.png`: save the window's picture after that many frames and end.
    shot: Option<(PathBuf, u64)>,
    autosave: crate::doc::autosave::Timer,
    /// An autosave left by a session that did not end normally, offered back.
    recover: Option<crate::doc::autosave::Meta>,
    /// The Lines window.
    tt: crate::ui_timetable::TimetableUi,
    /// The Traffic and AI window.
    traffic: crate::ui_traffic::TrafficUi,
    pbr: crate::ui_pbr::PbrWindow,
    road_types: crate::ui_roads::RoadTypes,
    cj: crate::ui_cjunction::ComplexJunctions,
    scripts: crate::ui_script::ScriptsUi,
    map_settings: crate::ui_mapsettings::MapSettings,
    calendar: crate::ui_calendar::CalendarWindow,
    signals: crate::ui_signals::SignalWindow,
    chrono: crate::ui_chrono::ChronoWindow,
    /// The Satellite and terrain window and the overlay on the ground.
    geo: crate::ui_geo::GeoUi,
    osm: crate::ui_osm::OsmUi,
    /// Map > GPS tracks to trace.
    trace: crate::ui_trace::TraceWindow,
    /// Map > Check in openOMSI.
    gamecheck: crate::gamecheck::GameCheck,
    /// Getting started.
    guide: crate::ui_guide::Guide,
    /// File > Multiplayer: the session, when there is one.
    collab: crate::collab::Collab,
    updater: crate::updater::Updater,
    /// A full-size picture of the view is taken this frame (View menu, F12).
    snapshot: bool,
    snapshot_done: bool,
    /// An action the user allowed to drop unsaved changes is being carried out.
    unguarded: bool,
}

impl App {
    pub fn new(open: Option<PathBuf>) -> App {
        let prefs = Prefs::load();
        // (OPENMAPPER_LANG=en|ru: checks of either language without a person at the screen)
        i18n::set(i18n::from_code(&std::env::var("OPENMAPPER_LANG").unwrap_or_else(|_| prefs.language.clone())));
        let mut app = App {
            gfx: None,
            prefs,
            roots: None,
            assets: None,
            lib: Library::default(),
            hot: None,
            mesh_known: None,
            hook_edit: None,
            pre_2d: None,
            ed: Editor::default(),
            pinned_cursor: None,
            cam: Camera::default(),
            stream_at: None,
            last_input: Instant::now(),
            stream_set: None,
            ui: UiState::default(),
            mouse: Mouse::default(),
            keys: Keys::default(),
            last: Instant::now(),
            swatches: HashMap::new(),
            drive: Default::default(),
            texture_files: Vec::new(),
            texture_scan: None,
            maps_scan: None,
            switches_for: 0,
            game: install::find_game(),
            open_at_start: open,
            quit: false,
            view: egui::Rect::NOTHING,
            frames: 0,
            tt: crate::ui_timetable::TimetableUi::new(),
            traffic: Default::default(),
            pbr: Default::default(),
            road_types: Default::default(),
            cj: Default::default(),
            scripts: Default::default(),
            map_settings: Default::default(),
            calendar: Default::default(),
            signals: Default::default(),
            chrono: Default::default(),
            geo: Default::default(),
            osm: Default::default(),
            trace: Default::default(),
            gamecheck: Default::default(),
            guide: Default::default(),
            collab: Default::default(),
            updater: crate::updater::Updater::new(),
            snapshot: false,
            snapshot_done: false,
            unguarded: false,
            shot: std::env::var_os("OPENMAPPER_SHOT").map(|p| (PathBuf::from(p), std::env::var("OPENMAPPER_SHOT_FRAMES").ok().and_then(|n| n.parse().ok()).unwrap_or(90))),
            autosave: Default::default(),
            recover: None,
        };
        crate::logging::stage("looking for OMSI 2 and the content");
        app.setup_roots();
        // OPENMAPPER_NEWMAP: the New map dialog open at the start (checks without a person at the screen)
        if std::env::var_os("OPENMAPPER_NEWMAP").is_some() {
            app.ui.new_map = Some(Default::default());
        }
        // OPENMAPPER_TIME="5,12.5": the view's clock running at 5x from 12:30 (checks of the
        // speeds without a person at the screen, as OPENMAPPER_SHOT is)
        if let Some(v) = std::env::var_os("OPENMAPPER_TIME") {
            let s = v.to_string_lossy();
            let mut parts = s.split(',');
            let speed = parts.next().and_then(|p| p.trim().parse::<f32>().ok());
            let hour = parts.next().and_then(|p| p.trim().parse::<f32>().ok());
            if let Some(speed) = speed.filter(|s| s.is_finite() && *s > 0.0) {
                app.ui.time_speed = speed;
            }
            if let Some(hour) = hour.filter(|h| h.is_finite()) {
                app.ui.daytime = Some(hour.rem_euclid(24.0));
                app.ui.time_flows = true;
            }
        }
        // OPENMAPPER_MULTIPLAYER: File > Multiplayer open at the start
        app.collab.open = std::env::var_os("OPENMAPPER_MULTIPLAYER").is_some();
        // OPENMAPPER_HELP=1: View > Controls open (checks of the keys without a person at the
        // screen); OPENMAPPER_KEYS=mapper|omsi|blender: that set of keys (and OMSI 2's mouse)
        if let Ok(k) = std::env::var("OPENMAPPER_KEYS") {
            app.prefs.omsi_controls = k == "omsi";
            app.prefs.keys_set = k;
        }
        if std::env::var_os("OPENMAPPER_HELP").is_some() {
            app.ui.help_open = true;
            app.ui.settings_tab = 3;
        }
        app
    }

    fn setup_roots(&mut self) {
        let Some(omsi) = install::find_omsi_root(&self.prefs) else {
            log::warn!("no OMSI 2 installation found");
            return;
        };
        log::info!("OMSI 2: {}", omsi.display());
        let roots = Roots::new(omsi);
        self.assets = Some(Assets::new(roots.omsi.clone(), roots.content.clone()));
        self.lib.start_scan(&roots);
        // (every map's global.cfg is read to name it: a long list on a slow disk is not waited for)
        let (tx, rx) = std::sync::mpsc::channel();
        let listed = roots.clone();
        std::thread::spawn(move || {
            let _ = tx.send(install::list_maps(&listed));
        });
        self.maps_scan = Some(rx);
        self.ui.favourites = self.prefs.favourites.clone();
        self.ui.recent_assets = self.prefs.recent_assets.clone();
        self.scan_textures(&roots);
        self.ed.content_dir = Some(roots.content.clone());
        self.ed.road_styles = crate::road_maker::Styles::load(&roots.content);
        self.ed.asphalt = crate::junction::asphalt_texture(&roots.omsi);
        self.recover = crate::doc::autosave::pending(&roots.content);
        if let Some(g) = &mut self.gfx {
            g.renderer.roots = vec![roots.content.clone(), roots.omsi.clone()];
        }
        // (not in the screenshot and check runs: nothing there waits for a change)
        self.hot = if self.shot.is_none() { Some(crate::hot::Watcher::start(roots.content.clone(), roots.omsi.clone())) } else { None };
        self.roots = Some(roots);
    }

    fn init_gfx(&mut self, el: &ActiveEventLoop) -> anyhow::Result<()> {
        let mut attrs = Window::default_attributes().with_title("openMapper").with_inner_size(winit::dpi::LogicalSize::new(std::env::var("OPENMAPPER_WIDTH").ok().and_then(|w| w.parse().ok()).unwrap_or(1440.0), 900.0)).with_min_inner_size(winit::dpi::LogicalSize::new(900.0, 600.0)).with_window_icon(winit::window::Icon::from_rgba(crate::icon::rgba(64), 64, 64).ok());
        // (a screenshot run stays on top: a window under another gets no frames to take)
        if self.shot.is_some() {
            attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop).with_active(false);
        }
        crate::logging::stage("creating the window");
        let window = Arc::new(el.create_window(attrs)?);
        crate::logging::stage("starting the graphics card");
        self.cam.tune = self.prefs.cam_tune.sane();
        self.cam.fly_speed = self.cam.tune.fly_speed;
        let mut gpu = Gpu::new(window.clone(), crate::gpu::Api::of(&self.prefs.graphics_api))?;
        crate::logging::stage("first frames");
        let quality = crate::render::quality::Quality::from_code(&self.prefs.graphics);
        gpu.set_samples(quality.samples());
        let mut renderer = Renderer::new(&gpu, quality);
        if let Some(r) = &self.roots {
            renderer.roots = vec![r.content.clone(), r.omsi.clone()];
        }
        let egui_ctx = egui::Context::default();
        ui::style(&egui_ctx);
        if self.prefs.ui_scale > 0.0 {
            egui_ctx.set_zoom_factor(self.prefs.ui_scale.clamp(0.5, 2.0));
        }
        // OPENMAPPER_UI_SCALE=<factor>: the interface this size (a long panel whole in a check)
        if let Some(z) = std::env::var("OPENMAPPER_UI_SCALE").ok().and_then(|z| z.parse::<f32>().ok()) {
            egui_ctx.set_zoom_factor(z.clamp(0.5, 2.0));
        }
        let egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(gpu.device.limits().max_texture_dimension_2d as usize));
        let egui_renderer = egui_wgpu::Renderer::new(&gpu.device, gpu.config.format, egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, predictable_texture_filtering: false });
        log::info!("graphics: {}", gpu.adapter_name);
        self.gfx = Some(Gfx { window, gpu, renderer, egui_ctx, egui_state, egui_renderer });
        if let Some(p) = self.open_at_start.take() {
            self.open_map(&p);
        }
        Ok(())
    }

    // ------------------------------------------------------------------ actions

    fn open_map(&mut self, cfg: &Path) {
        let Some(roots) = &self.roots else { return };
        // (what was open is saved or its changes were dropped; an autosave of an earlier
        // session still offered is not this map's)
        if self.ed.doc.is_some() && self.recover.is_none() {
            crate::doc::autosave::clear(&roots.content);
        }
        // (a big map is read and its tiles made ready for the view here, the window waiting:
        // the log says so if that wait is long enough for the watchdog to write)
        crate::logging::busy("opening the map");
        let loaded = Document::load(roots, cfg, |_| {});
        match loaded {
            Ok(doc) => {
                self.prefs.push_recent(cfg);
                self.after_open(doc);
            }
            Err(e) => self.ed.error(format!("{}: {e}", tr("Could not open the map", "Не удалось открыть карту"))),
        }
        crate::logging::busy("");
    }

    /// The camera kept for the map open now, to open it there next time.
    fn remember_view(&mut self) {
        if let Some(doc) = &self.ed.doc {
            let c = &self.cam;
            self.prefs.remember_view(&doc.dir, [c.target.x, c.target.y, c.target.z, c.yaw, c.pitch, c.distance]);
        }
    }

    /// Once, the first time a map is opened with this version: load the map 100 tiles at a
    /// time, as OMSI 2's editor does, or all of it (the default).
    fn ask_tile_streaming(&mut self) {
        if self.prefs.tile_streaming.is_some() || self.shot.is_some() || std::env::var_os("OPENMAPPER_SHOT").is_some() {
            return;
        }
        let yes = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title("openMapper")
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_description(tr(
                "Load a big map 100 tiles at a time, as OMSI 2's own editor does?\n\nOnly the 100 tiles round the camera are kept in graphics memory; the others are hidden until you click on the empty zone, where the next 100 are loaded. It keeps very big maps light. You can change it any time in View > Load 100 tiles at a time.",
                "Загружать большую карту по 100 тайлов, как в редакторе OMSI 2?\n\nВ графической памяти остаются только 100 тайлов вокруг камеры, остальные скрыты, пока вы не кликнете по пустой зоне: там загрузятся следующие 100. Это сильно облегчает очень большие карты. Можно изменить в любой момент: Вид > Загружать по 100 тайлов.",
            ))
            .show();
        self.prefs.tile_streaming = Some(yes == rfd::MessageDialogResult::Yes);
        self.prefs.save();
    }

    /// With tile streaming on, a click on ground whose tile is not loaded loads the 100 tiles
    /// round that place instead (and is not a click on the map).
    fn stream_click(&mut self) -> bool {
        let Some(doc) = &self.ed.doc else { return false };
        let Some((Some(set), _)) = &self.stream_set else { return false };
        let Some(p) = self.grab_point() else { return false };
        let at = DVec2::new(p.x, p.y);
        let k = crate::doc::tile_of(at);
        if set.contains(&k) || !doc.tiles.contains_key(&k) {
            return false;
        }
        self.stream_at = Some(at);
        self.ed.toast(format!("{} {}", stream_count(self.prefs.stream_tiles), tr("tiles are loaded round there, the others are hidden", "тайлов загружено вокруг этого места, остальные скрыты")));
        true
    }

    fn after_open(&mut self, doc: Document) {
        self.remember_view();
        // look at the map's middle (or its editor camera)
        let keys: Vec<_> = doc.tiles.keys().copied().collect();
        let mut c = DVec2::ZERO;
        for k in &keys {
            c += tile_origin(*k) + DVec2::splat(tile_size() * 0.5);
        }
        if !keys.is_empty() {
            c /= keys.len() as f64;
        }
        // (the tile nearest the middle: a map may be spread out)
        let near = keys.iter().map(|k| tile_origin(*k) + DVec2::splat(tile_size() * 0.5)).min_by(|a, b| (*a - c).length().total_cmp(&(*b - c).length())).unwrap_or(c);
        let h = doc.ground(near);
        self.cam.jump_to(DVec3::new(near.x, near.y, h), if keys.len() > 4 { 420.0 } else { 300.0 });
        // the map's own editor camera ([mapcam]: tile x, y, x, height, y in the tile, heading,
        // pitch, distance), where OMSI's editor opens it
        let mc = &doc.global.map_cam;
        if mc.len() == 8 {
            let (x, y) = omsi_map::tile_local_to_world(mc[0] as i32, mc[1] as i32, mc[2], mc[4]);
            let p = DVec2::new(x, y);
            if doc.height_at(p).is_some() {
                self.cam.jump_to(DVec3::new(x, y, doc.ground(p)), (mc[7] * 4.0).clamp(150.0, 600.0));
                self.cam.yaw = mc[5];
                self.cam.pitch = 40.0;
            }
        }
        // where it was left last time (not for checks without a person at the screen)
        if let (Some(v), None) = (self.prefs.last_view(&doc.dir), &self.shot) {
            if doc.height_at(DVec2::new(v[0], v[1])).is_some() {
                self.cam.jump_to(DVec3::new(v[0], v[1], v[2]), v[5]);
                self.cam.yaw = v[3];
                self.cam.pitch = v[4];
                self.cam.snap();
            }
        }
        log::info!("camera at ({:.1}, {:.1}, {:.1}), tile size {:.2} m, {} tiles", self.cam.target.x, self.cam.target.y, self.cam.target.z, tile_size(), keys.len());
        if let Some(g) = &mut self.gfx {
            g.renderer.reset_map();
        }
        self.ed.open(doc);
        self.ed.set_tool(Tool::Select);
        // (the loaded window of tiles starts round where the map opened)
        self.stream_at = None;
        self.stream_set = None;
        self.ask_tile_streaming();
        // OPENMAPPER_CAM=x,y,distance,yaw,pitch (checks without a person at the screen)
        if let Some(v) = std::env::var("OPENMAPPER_CAM").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 5) {
            let h = self.ed.doc.as_ref().map(|d| d.ground(DVec2::new(v[0], v[1]))).unwrap_or(0.0);
            self.cam.jump_to(DVec3::new(v[0], v[1], h), v[2]);
            self.cam.yaw = v[3];
            self.cam.pitch = v[4];
        }
        // OPENMAPPER_SELECT=x,y[,spline|chain]: the object (or the spline starting) nearest there
        // selected (checks of its properties)
        let select = std::env::var("OPENMAPPER_SELECT").unwrap_or_default();
        if let (Some(v), Some(doc)) = (Some(select.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 2), self.ed.doc.as_ref()) {
            let at = DVec2::new(v[0], v[1]);
            let near = |k: crate::doc::TileKey, p: [f64; 3]| (crate::doc::tile_origin(k) + DVec2::new(p[0], p[1]) - at).length();
            let best = if select.ends_with("spline") || select.ends_with("chain") {
                doc.tiles.iter().flat_map(|(k, t)| t.tile.splines.iter().filter(|s| !s.deleted).map(move |s| (Sel::Spline(*k, s.id), near(*k, s.pos)))).min_by(|a, b| a.1.total_cmp(&b.1))
            } else {
                doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().map(move |o| (Sel::Object(*k, o.id), near(*k, o.pos)))).min_by(|a, b| a.1.total_cmp(&b.1))
            };
            if let Some((sel, _)) = best {
                self.ed.select(Some(sel));
                // (`chain`: all the road, as a double click selects it)
                if let (Sel::Spline(k, id), true) = (sel, select.ends_with("chain")) {
                    self.ed.select_chain(k, id);
                }
            }
        }
        // OPENMAPPER_JUNCTION_END=start|end: the first junction piece that fits the road
        // OPENMAPPER_SELECT selected put onto its start or its far end
        if let (Ok(which), Some(Sel::Spline(k, id))) = (std::env::var("OPENMAPPER_JUNCTION_END"), self.ed.selected) {
            while self.lib.scanning {
                self.lib.poll();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let file = self.ed.doc.as_ref().and_then(|d| d.spline(k, id)).map(|s| s.file.clone()).unwrap_or_default();
            let piece = self.lib.junction_pieces(&file).0.first().map(|a| a.rel.clone());
            if let (Some(piece), Some(assets)) = (piece, self.assets.as_mut()) {
                log::info!("junction end: {piece}: {:?}", self.ed.junction_at_end(assets, &self.lib, k, id, which == "end", &piece));
            }
        }
        // OPENMAPPER_SHAPE=<n>,x,y: shape n of `primitives::ALL` put there as an object of its
        // own and selected; OPENMAPPER_EDIT=vertex|edge|face[,all][,<op>]: its mesh edited
        // (`signal` or `lamp` in place of n: a signal head or a street lamp; several split by
        // `;` are all selected, the first in the properties)
        let mut placed = Vec::new();
        for shape in std::env::var("OPENMAPPER_SHAPE").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
            let made = match shape.split(',').next().unwrap_or("") {
                "signal" => Some(crate::signal_maker::signal_head(&[])),
                "lamp" => Some(crate::signal_maker::street_lamp()),
                n => n.parse::<usize>().ok().and_then(|i| openmapper_mesh::primitives::ALL.get(i).copied()).map(crate::editable::primitive_source),
            };
            let v: Vec<f64> = shape.split(',').skip(1).filter_map(|x| x.trim().parse::<f64>().ok()).collect();
            let (Some(src), true, Some(roots)) = (made, v.len() >= 2, self.roots.clone()) else { continue };
            let rel = crate::editable::new_rel(&roots, "Checks", &crate::editable::safe_name(&src.name));
            if crate::editable::save(&roots, &rel, &src).is_ok() {
                let at = DVec2::new(v[0], v[1]);
                let z = self.ed.doc.as_ref().map(|d| d.ground(at)).unwrap_or(0.0) + v.get(2).copied().unwrap_or(0.0);
                placed.extend(self.add_map_object_at(&rel, DVec3::new(at.x, at.y, z)));
            }
        }
        if !placed.is_empty() {
            self.ed.set_selection(placed);
        }
        // OPENMAPPER_MESHOP=union|difference|intersection|slice|join|separate: done to the
        // selection (after OPENMAPPER_SHAPE)
        if let (Ok(op), Some(roots)) = (std::env::var("OPENMAPPER_MESHOP"), self.roots.clone()) {
            use crate::meshops::Bool;
            let r = match op.as_str() {
                "join" => crate::meshops::join(&mut self.ed, &roots, &self.lib).map(|_| ()),
                "separate" => self.ed.selected.map(|s| crate::meshops::separate_parts(&mut self.ed, &roots, &self.lib, s).map(|_| ())).unwrap_or(Ok(())),
                o => {
                    let b = match o {
                        "union" => Bool::Union,
                        "intersection" => Bool::Intersection,
                        "slice" => Bool::Slice,
                        _ => Bool::Difference,
                    };
                    crate::meshops::boolean(&mut self.ed, &roots, &self.lib, b, false).map(|_| ())
                }
            };
            if let Err(e) = r {
                log::warn!("mesh op {op}: {e}");
            }
        }
        self.hook_edit = std::env::var("OPENMAPPER_EDIT").ok();
        // OPENMAPPER_XFORM=move|rotate|scale: the Select tool's handles shown on the selection
        if let Ok(x) = std::env::var("OPENMAPPER_XFORM") {
            use crate::editor::Xform;
            self.ed.xform = match x.as_str() {
                "move" => Xform::Move,
                "rotate" => Xform::Rotate,
                "scale" => Xform::Scale,
                _ => Xform::Select,
            };
            if let (Some(Sel::Object(k, id)), Some(doc)) = (self.ed.selected, self.ed.doc.as_ref()) {
                if let Some(o) = doc.object(k, id) {
                    let org = crate::doc::tile_origin(k);
                    log::info!("selected {} at ({:.1}, {:.1})", o.file, org.x + o.pos[0], org.y + o.pos[1]);
                }
            }
        }
        // OPENMAPPER_JOIN_NEXT=<n>: the spline OPENMAPPER_SELECT selected joined with the next
        // piece n times; OPENMAPPER_CUT=x,y: the road there cut in two
        if let Some(n) = std::env::var("OPENMAPPER_JOIN_NEXT").ok().and_then(|v| v.parse::<usize>().ok()) {
            if let Some(Sel::Spline(k, id)) = self.ed.selected {
                for _ in 0..n {
                    self.ed.join_with_next(k, id);
                }
            }
        }
        if let Some(v) = std::env::var("OPENMAPPER_CUT").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 2) {
            self.ed.cut_road_at(DVec2::new(v[0], v[1]));
        }
        // OPENMAPPER_PAINT_ALONG=<layer>,<width>[,under]: that layer painted along what
        // OPENMAPPER_SELECT selected
        if let Ok(a) = std::env::var("OPENMAPPER_PAINT_ALONG") {
            let v: Vec<&str> = a.split(',').collect();
            if let (Some(l), Some(w)) = (v.first().and_then(|x| x.parse().ok()), v.get(1).and_then(|x| x.parse().ok())) {
                // (the catalogue read first: the roads' widths come from it)
                while self.lib.scanning {
                    self.lib.poll();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                self.ed.paint_along = crate::roadpaint::Along { layer: l, width: w, under: v.get(2) == Some(&"under"), ..Default::default() };
                let n = self.ed.paint_along_selected(&self.lib);
                log::info!("painted along: {n} texels");
            }
        }
        // OPENMAPPER_STOP=x,y[,one]: a bus stop placed there with the Game logic tool (on both
        // sides unless `one`)
        if let Some(v) = std::env::var("OPENMAPPER_STOP").ok().map(|a| a.split(',').map(str::to_string).collect::<Vec<_>>()) {
            while self.lib.scanning {
                self.lib.poll();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if let (Some(x), Some(y), Some(assets)) = (v.first().and_then(|x| x.parse::<f64>().ok()), v.get(1).and_then(|y| y.parse::<f64>().ok()), self.assets.as_mut()) {
                self.ed.set_tool(Tool::Logic);
                self.ed.set_logic(crate::editor::LogicMode::Stops);
                self.ed.stop_both_sides = v.get(2).map(|s| s != "one").unwrap_or(true);
                self.ed.object_type = self.lib.assets.iter().find(|a| a.is_bus_stop).map(|a| a.rel.clone());
                let z = self.ed.doc.as_ref().map(|d| d.ground(DVec2::new(x, y))).unwrap_or(0.0);
                // (the roads' models loaded first: their lanes are what it stands by)
                for _ in 0..500 {
                    assets.poll();
                    if self.ed.doc.as_ref().map(|d| crate::lanes::near(d, assets, DVec2::new(x, y), 25.0).0.iter().filter(|l| l.kind == 0).count() >= 2).unwrap_or(true) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                self.ed.cursor_in_view = true;
                self.ed.hover_point(assets, &self.lib, DVec3::new(x, y, z));
                self.ed.left_press(assets, &self.lib, false);
                self.ed.hover_point(assets, &self.lib, DVec3::new(x, y, z));
            }
        }
        // OPENMAPPER_WIRE=<spline>: that wire laid on what OPENMAPPER_SELECT selected
        if let Ok(w) = std::env::var("OPENMAPPER_WIRE") {
            let n = self.ed.wire_along(&w);
            log::info!("wire: {n} pieces");
        }
        // OPENMAPPER_HIDE=trees,objects,splines,technical: those layers hidden
        if let Ok(h) = std::env::var("OPENMAPPER_HIDE") {
            self.ed.show_technical = !h.split(',').any(|x| x.trim() == "technical");
            for (i, name) in ["trees", "objects", "splines"].iter().enumerate() {
                self.ed.hide[i] = h.split(',').any(|x| x.trim() == *name);
            }
        }
        // OPENMAPPER_SETTINGS=1: the map settings open (=calendar: the calendar and place)
        match std::env::var("OPENMAPPER_SETTINGS").as_deref() {
            Ok("calendar") => self.calendar.open = true,
            Ok("signals") => self.signals.open = true,
            Ok("chrono") => self.chrono.open = true,
            Ok(_) => self.map_settings.open = true,
            Err(_) => {}
        }
        // OPENMAPPER_GROUPS=1: the groups open
        if std::env::var_os("OPENMAPPER_GROUPS").is_some() {
            self.ui.groups.open = true;
        }
        // OPENMAPPER_VIEWS=<n>: the saved views open (n: view n gone to)
        if let Ok(v) = std::env::var("OPENMAPPER_VIEWS") {
            self.ui.views.open = true;
            self.ui.views.follow(self.ed.doc.as_ref().map(|d| d.dir.as_path()));
            if let Some(n) = v.parse::<usize>().ok().filter(|n| *n > 0) {
                self.ui.views.go(n - 1, &mut self.cam);
                self.cam.snap();
            }
        }
        // OPENMAPPER_CJ=<layout.json>: the complex junction editor open on that drawing
        if let Ok(p) = std::env::var("OPENMAPPER_CJ") {
            if let Some(l) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<crate::cjunction::Layout>(&t).ok()) {
                self.cj.layout = l;
            }
            self.cj.open = true;
            // OPENMAPPER_CJ_LANES=1: with the lanes drawn
            self.cj.show_lanes(std::env::var_os("OPENMAPPER_CJ_LANES").is_some());
            // OPENMAPPER_CJ_PREVIEW=1: on the Preview tab
            self.cj.show_preview(std::env::var_os("OPENMAPPER_CJ_PREVIEW").is_some());
        }
        // OPENMAPPER_MAKER=1: the Road types window open on a new road type in the road maker
        // (`pocket`: with a bus pocket)
        if let Ok(m) = std::env::var("OPENMAPPER_MAKER") {
            self.road_types.open_maker(m == "pocket");
            if m == "marked" {
                self.road_types.mark_maker();
            }
            if m == "canvas" {
                self.road_types.mark_maker();
                self.road_types.mark_canvas_sample();
            }
        }
        // OPENMAPPER_PALETTE=<text>: the command search open with that typed
        if let Ok(q) = std::env::var("OPENMAPPER_PALETTE") {
            self.ui.palette.show_with(&q);
        }
        // OPENMAPPER_MERGE=<global.cfg>[,add]: Tiles from another map open on that map (`add`:
        // all its tiles added beside this one)
        if let Ok(a) = std::env::var("OPENMAPPER_MERGE") {
            let (cfg, add) = a.strip_suffix(",add").map(|c| (c, true)).unwrap_or((&a, false));
            if let Some(r) = &self.roots {
                self.ui.merge.show(install::list_maps(r));
            }
            self.ui.merge.open_on(cfg.into(), &mut self.ed, &self.lib, add);
        }
        // OPENMAPPER_TRACE=<file>[,build]: GPS tracks to trace open with that file loaded (and
        // its first line built into a road of the first road type, after OPENMAPPER_GEO)
        if let Ok(a) = std::env::var("OPENMAPPER_TRACE") {
            let (f, build) = a.strip_suffix(",build").map(|f| (f, true)).unwrap_or((&a, false));
            self.trace.add_file(std::path::Path::new(f));
            self.trace.open = true;
            if build {
                while self.lib.scanning {
                    self.lib.poll();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if let Some(rt) = self.lib.assets.iter().find(|x| x.category == crate::library::Category::Roads && x.half_width > 3.0).map(|x| x.rel.clone()) {
                    self.trace.build_first(rt);
                }
            }
        }
        // OPENMAPPER_GAMECHECK=1: Check in openOMSI run at the start
        if std::env::var_os("OPENMAPPER_GAMECHECK").is_some() {
            self.game_check();
        }
        // OPENMAPPER_DRIVE=1: the Test drive window open
        if std::env::var_os("OPENMAPPER_DRIVE").is_some() {
            self.drive.button();
        }
        // OPENMAPPER_HISTORY=1: the edit history open
        if std::env::var_os("OPENMAPPER_HISTORY").is_some() {
            self.ui.history_open = true;
        }
        // OPENMAPPER_LEVEL=1: the ground levelled under what OPENMAPPER_SELECT selected
        if std::env::var_os("OPENMAPPER_LEVEL").is_some() {
            self.ed.level_request = true;
        }
        // OPENMAPPER_MENU=x,y[,gx,gy]: the view's right-click menu open there (window points),
        // as if clicked on the ground at world gx, gy
        if let Some(v) = std::env::var("OPENMAPPER_MENU").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 2 || v.len() == 4) {
            let ground = (v.len() == 4).then(|| glam::DVec3::new(v[2], v[3], 0.0));
            self.ui.view_menu = Some(crate::ui_menu::ViewMenu { at: egui::pos2(v[0] as f32, v[1] as f32), ground });
        }
        // OPENMAPPER_RULER=x0,y0,x1,y1: the ruler laid between those points; OPENMAPPER_GRID=<step>:
        // grid snapping on, the cursor at the camera's target
        if let Some(v) = std::env::var("OPENMAPPER_RULER").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 4) {
            if let Some(doc) = &self.ed.doc {
                let p = |x: f64, y: f64| DVec3::new(x, y, doc.ground(DVec2::new(x, y)));
                self.ed.ruler = Some((Some(p(v[0], v[1])), Some(p(v[2], v[3]))));
            }
        }
        if let Some(s) = std::env::var("OPENMAPPER_GRID").ok().and_then(|s| s.parse::<f64>().ok()) {
            self.ed.snap_grid = true;
            self.ed.grid_step = s;
            self.ed.cursor = Some(self.cam.target);
            self.ed.cursor_in_view = true;
        }
        // OPENMAPPER_KNIFE=x,y: the knife on, the cursor there
        if let Some(v) = std::env::var("OPENMAPPER_KNIFE").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 2) {
            self.ed.toggle_cutting();
            if let Some(doc) = &self.ed.doc {
                self.pinned_cursor = Some(DVec3::new(v[0], v[1], doc.ground(DVec2::new(v[0], v[1]))));
            }
        }
        // OPENMAPPER_OUTLINER=[text]: the Select panel's Objects tab, that searched for
        if let Ok(q) = std::env::var("OPENMAPPER_OUTLINER") {
            self.ui.outliner.show_with(&q);
        }
        // OPENMAPPER_FIND=<text>: Find in map open with that typed
        if let Ok(q) = std::env::var("OPENMAPPER_FIND") {
            self.ui.find.show_with(&q);
        }
        // OPENMAPPER_CHECK=1: the map check open from the start
        if std::env::var_os("OPENMAPPER_CHECK").is_some() {
            self.ui.check.open = true;
            self.ui.check.issues = None;
        }
        // OPENMAPPER_SCRIPTS=1: the Scripts window open from the start
        if std::env::var_os("OPENMAPPER_SCRIPTS").is_some() {
            self.scripts.open = true;
        }
        // OPENMAPPER_TIMETABLE=<line>: the Lines window open on that line (checks without a person at the screen)
        if let Ok(l) = std::env::var("OPENMAPPER_TIMETABLE") {
            self.tt.open_at(&l);
        }
        // OPENMAPPER_GUIDE: Getting started open
        if std::env::var_os("OPENMAPPER_GUIDE").is_some() {
            self.guide.open = true;
        }
        // OPENMAPPER_GROW=<n>: n rings of new tiles round the map (checks of bigger maps)
        if let Some(n) = std::env::var("OPENMAPPER_GROW").ok().and_then(|n| n.parse::<usize>().ok()) {
            for _ in 0..n {
                self.ed.grow_map();
            }
        }
        // OPENMAPPER_GEO=lat,lon[,load][,hide]: placed there, the Satellite and terrain window open
        if let Ok(a) = std::env::var("OPENMAPPER_GEO") {
            self.geo.scripted(&mut self.ed, &self.prefs, &a);
        }
        // OPENMAPPER_OSM=<main type>[;<street type>[;<stop>]]: roads and stops from OpenStreetMap built (after
        // OPENMAPPER_GEO placed the map)
        if let Ok(t) = std::env::var("OPENMAPPER_OSM") {
            // (the catalogue read first: the buildings' models come from it)
            while self.lib.scanning {
                self.lib.poll();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            self.osm.scripted(&self.ed, &self.lib, self.geo.georef(), &t);
        }
        // OPENMAPPER_TRAFFIC=day|groups|lists|depots[,make][,stops|routes]: the Traffic and AI
        // window open on that tab; `make`: a depot file made from the map's lines
        if let Ok(t) = std::env::var("OPENMAPPER_TRAFFIC") {
            let mut parts = t.split(',');
            let tab = parts.next().unwrap_or("");
            let rest: Vec<&str> = parts.collect();
            let make = rest.contains(&"make");
            self.traffic.open_at(tab);
            if let Some(p) = rest.iter().find(|p| **p != "make") {
                self.traffic.depots_part(p);
            }
            if let (true, Some(doc)) = (make, self.ed.doc.as_mut()) {
                let n = crate::ui_hof::make_from_lines(doc);
                log::info!("depot file made from the lines: {} destinations, {} stops, {} routes", n.0, n.1, n.2);
            }
        }
        // OPENMAPPER_TOOL=roads|objects|logic|terrain|paint (checks without a person at the screen)
        match std::env::var("OPENMAPPER_TOOL").as_deref() {
            Ok("roads") => self.ed.set_tool(Tool::Roads),
            Ok("objects") => self.ed.set_tool(Tool::Objects),
            Ok("logic") => self.ed.set_tool(Tool::Logic),
            Ok("terrain") => self.ed.set_tool(Tool::Terrain),
            Ok("paint") => self.ed.set_tool(Tool::Paint),
            _ => {}
        }
        // OPENMAPPER_SEASON=<texture folder>: the view in that season (Winter, WinterSnow, Fall…)
        if let Ok(f) = std::env::var("OPENMAPPER_SEASON") {
            self.set_season(Some(Box::leak(f.into_boxed_str())));
        }
        // OPENMAPPER_DAYTIME=<hour>: the view lit at that hour
        if let Some(h) = std::env::var("OPENMAPPER_DAYTIME").ok().and_then(|h| h.parse::<f32>().ok()) {
            self.ui.daytime = Some(h);
        }
        // OPENMAPPER_HEIGHTMAP=<picture>[,apply]: the Terrain tool with that picture to import
        // (laid on the map at once with `apply`)
        if let Ok(a) = std::env::var("OPENMAPPER_HEIGHTMAP") {
            let (file, apply) = a.strip_suffix(",apply").map(|f| (f, true)).unwrap_or((&a, false));
            self.ed.set_tool(Tool::Terrain);
            self.ui.heightmap.open(&mut self.ed, file.into());
            if apply {
                self.ui.heightmap.apply(&mut self.ed);
            }
        }
        // OPENMAPPER_LAY=1: every road selected and laid on the ground
        if std::env::var_os("OPENMAPPER_LAY").is_some() {
            self.ed.select_all();
            self.ed.lay_selected_on_ground();
        }
        // OPENMAPPER_PLANT=<plant>;x,y;x,y;…[;open]: the plant brush's outline clicked round
        // there and filled (left open with `open`)
        if let Ok(a) = std::env::var("OPENMAPPER_PLANT") {
            let mut parts = a.split(';');
            self.ed.set_tool(Tool::Objects);
            self.ed.plant_brush = true;
            self.ed.plant_area = true;
            self.ed.object_type = parts.next().map(str::to_string);
            let mut open = false;
            for p in parts {
                let v: Vec<f64> = p.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                match v[..] {
                    [x, y] => self.ed.plant_outline.push(DVec2::new(x, y)),
                    _ => open |= p == "open",
                }
            }
            if let (false, Some(assets)) = (open, self.assets.as_mut()) {
                let n = self.ed.finish_plant_outline(assets);
                log::info!("plant outline: {n} planted");
            }
        }
        // OPENMAPPER_PUT=<object>;x,y: one object put there (and selected), for checks of what it looks like
        if let Ok(a) = std::env::var("OPENMAPPER_PUT") {
            let mut parts = a.split(';');
            let rel = parts.next().unwrap_or("").to_string();
            let v: Vec<f64> = parts.next().unwrap_or("").split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let [x, y] = v[..] {
                let z = self.ed.doc.as_ref().map(|d| d.ground(DVec2::new(x, y))).unwrap_or(0.0);
                let placed = self.add_map_object_at(&rel, DVec3::new(x, y, z));
                log::info!("put: {} placed", placed.is_some());
            }
        }
        // OPENMAPPER_AUTOPAINT=<layer>: the Paint tool's rule by slope and height applied to that layer
        if let Some(layer) = std::env::var("OPENMAPPER_AUTOPAINT").ok().and_then(|l| l.parse::<usize>().ok()) {
            self.ed.set_tool(Tool::Paint);
            self.ed.paint_layer = layer;
            if let Some(doc) = self.ed.doc.as_mut() {
                crate::autopaint::apply(doc, &mut self.ed.history, layer, &self.ui.autopaint);
            }
        }
    }

    fn new_map(&mut self, folder: &str, friendly: &str, size: i32, options: &crate::ui_newmap::NewMapOptions) {
        let Some(roots) = &self.roots else { return };
        if self.ed.doc.is_some() && self.recover.is_none() {
            crate::doc::autosave::clear(&roots.content);
        }
        let mut folder = folder.to_string();
        // (a folder of that name may be there already)
        let base = folder.clone();
        let mut i = 2;
        while roots.content.join("maps").join(&folder).exists() || roots.omsi.join("maps").join(&folder).exists() {
            folder = format!("{base} {i}");
            i += 1;
        }
        let mut doc = Document::new_map(roots, &folder, friendly, size);
        if let (Some(g), Some(base)) = (&options.ground, doc.global.ground_textures.first_mut()) {
            base.texture = g.clone();
        }
        let g = &mut doc.global;
        g.description = options.description.trim().to_string();
        g.left_hand_traffic = options.left_hand;
        g.real_rail = options.real_rail;
        g.years = (options.years.0.min(options.years.1), options.years.0.max(options.years.1));
        g.real_year_offset = options.year_offset;
        if !options.money.is_empty() {
            g.money_system = options.money.clone();
        }
        if !options.tickets.is_empty() {
            g.ticket_pack = options.tickets.clone();
        }
        for d in &mut g.traffic_density_road {
            d.1 *= options.traffic;
        }
        for d in &mut g.traffic_density_passenger {
            d.1 *= options.passengers;
        }
        use crate::ui_newmap::Seasons;
        use omsi_map::global::Season;
        g.seasons = match options.seasons {
            Seasons::Summer => Vec::new(),
            // (days from 1 January: spring from March, autumn from mid September, winter from
            // December to February)
            Seasons::Four => vec![Season { kind: 4, start_day: 0, end_day: 59 }, Season { kind: 1, start_day: 59, end_day: 151 }, Season { kind: 2, start_day: 258, end_day: 334 }, Season { kind: 4, start_day: 334, end_day: 366 }],
            Seasons::Mild => vec![Season { kind: 3, start_day: 0, end_day: 59 }, Season { kind: 1, start_day: 59, end_day: 151 }, Season { kind: 2, start_day: 258, end_day: 334 }, Season { kind: 3, start_day: 334, end_day: 366 }],
        };
        if options.height != 0.0 {
            for t in doc.tiles.values_mut() {
                for h in t.terrain.heights.iter_mut() {
                    *h = options.height;
                }
                t.touch_terrain();
            }
        }
        self.after_open(doc);
        self.ed.set_tool(Tool::Roads);
        if options.real_place {
            self.osm.open = true;
            self.ed.toast(tr("Find the place in the window and build from it", "Найдите место в окне и постройте карту по нему"));
        } else {
            self.ed.toast(tr("Your map is ready. Start with a road!", "Карта готова. Начните с дороги!"));
        }
        self.guide.map_made(&self.prefs);
    }

    /// Save the map and pack it into a .zip (`crate::export`) where the user says.
    fn export(&mut self) {
        if !self.save() {
            return;
        }
        let (Some(doc), Some(roots)) = (&self.ed.doc, &self.roots) else { return };
        let folder = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into());
        let Some(out) = rfd::FileDialog::new().add_filter("zip", &["zip"]).set_file_name(format!("{folder}.zip")).set_title(tr("Export the map for sharing", "Экспорт карты для обмена")).save_file() else { return };
        match crate::export::export(doc, roots, &out) {
            Ok((plan, bytes)) => {
                self.ed.toast(format!("{} {} ({} {}, {:.1} MB)", tr("Exported:", "Экспортировано:"), out.display(), plan.files.len(), tr("files", "файлов"), bytes as f64 / 1e6));
                if !plan.missing.is_empty() {
                    self.ed.error(format!("{} {}", plan.missing.len(), tr("files the map names were not found: the readme lists them", "файлов, которые называет карта, не найдено: они перечислены в readme")));
                }
            }
            Err(e) => self.ed.error(format!("{}: {e}", tr("Could not export", "Не удалось экспортировать"))),
        }
    }

    /// The sample on the new map: a ring road with a bus line (`crate::sample`).
    fn sample(&mut self) {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (Some(assets), Some(roots)) = (self.assets.as_mut(), self.roots.as_ref()) else { return };
        let centre = self.ed.doc.as_ref().map(|d| {
            let n = d.tiles.len().max(1) as f64;
            d.tiles.keys().fold(DVec2::ZERO, |a, k| a + tile_origin(*k) + DVec2::splat(tile_size() * 0.5)) / n
        });
        let Some(centre) = centre else { return };
        match crate::sample::build(&mut self.ed, assets, &self.lib, roots, centre) {
            Ok(()) => {
                self.ed.history.clear();
                self.cam.jump_to(DVec3::new(centre.x, centre.y, 0.0), 380.0);
                self.cam.yaw = 0.0;
                self.cam.pitch = 50.0;
                self.ed.toast(tr("A ring road with line 1 on it: save, then try it in openOMSI (F5).", "Кольцевая дорога с линией 1: сохраните и попробуйте в openOMSI (F5)."));
            }
            Err(e) => self.ed.error(format!("{}: {e}", tr("The sample could not be made", "Пример не получился"))),
        }
    }

    /// `OPENMAPPER_POCKET`: a bus pocket laid on the sample's ring (a check of the bay tool).
    fn demo_pocket(&mut self, at: &str) {
        let v: Vec<f64> = at.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let Some(doc) = self.ed.doc.as_ref() else { return };
        let n = doc.tiles.len().max(1) as f64;
        let centre = doc.tiles.keys().fold(DVec2::ZERO, |a, k| a + tile_origin(*k) + DVec2::splat(tile_size() * 0.5)) / n;
        self.ed.pocket_width = v.get(2).copied().unwrap_or(3.0) as f32;
        self.ed.bus_pocket = true;
        self.ed.set_tool(Tool::Roads);
        let Some(assets) = self.assets.as_mut() else { return };
        let p = centre + DVec2::new(v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(-70.0));
        let z = self.ed.doc.as_ref().map(|d| d.ground(p)).unwrap_or(0.0);
        self.ed.hover_point(assets, &self.lib, DVec3::new(p.x, p.y, z));
        self.ed.pocket_click(assets);
        for t in &self.ed.toasts {
            log::info!("pocket check: toast ({}) {}", t.error, t.text);
        }
        if let Some(doc) = self.ed.doc.as_ref() {
            for (k, t) in &doc.tiles {
                let org = tile_origin(*k);
                for s in t.tile.splines.iter().filter(|s| !s.deleted && (DVec2::new(org.x + s.pos[0], org.y + s.pos[1]) - p).length() < 80.0) {
                    log::info!("pocket check: spline {} {} at ({:.1}, {:.1}) h {:.1} len {:.1} r {:.1} mirror {} prev {} next {}", s.id, s.file, org.x + s.pos[0], org.y + s.pos[1], s.heading, s.length, s.radius, s.mirror, s.prev_id, s.next_id);
                }
                for o in t.tile.objects.iter().filter(|o| (DVec2::new(org.x + o.pos[0], org.y + o.pos[1]) - p).length() < 80.0) {
                    log::info!("pocket check: object {} at ({:.1}, {:.1})", o.file, org.x + o.pos[0], org.y + o.pos[1]);
                }
            }
        }
        self.cam.jump_to(DVec3::new(p.x + 14.0, p.y - 6.0, 0.0), 95.0);
        self.cam.yaw = 0.0;
        self.cam.pitch = 80.0;
        self.save();
    }

    fn save(&mut self) -> bool {
        if self.collab.is_guest() {
            self.ed.error(tr("In multiplayer the host saves the map. Leave the session to save this as a copy of your own.", "В совместной правке карту сохраняет хост. Выйдите из сессии, чтобы сохранить копию для себя."));
            return false;
        }
        // (a scenario's things being edited on the map are not the map's)
        self.chrono.end_session(&mut self.ed);
        let Some(doc) = self.ed.doc.as_mut() else { return false };
        self.ed.history.commit();
        crate::editor::sync_entry_points(doc);
        self.tt.before_save(doc, self.roots.as_ref());
        // (the signal routes' pieces counted again among their tiles' paths: tiles change)
        if let Some(r) = &self.roots {
            if !doc.traffic.signal_routes.is_empty() {
                let mut routes = std::mem::take(&mut doc.traffic.signal_routes);
                let changed = crate::signals::reindex(&mut routes, doc, r);
                doc.traffic.signal_routes = routes;
                if changed {
                    doc.traffic.touch(crate::traffic::File::SignalRoutes);
                }
            }
        }
        crate::logging::busy("saving the map");
        let saved = crate::doc::save(doc);
        crate::logging::busy("");
        match saved {
            Ok(()) => {
                omsi_cfg::content_changed();
                if let (Some(r), None) = (&self.roots, &self.recover) {
                    crate::doc::autosave::clear(&r.content);
                }
                self.autosave.reset();
                let cfg = doc.dir.join("global.cfg");
                let dir = doc.dir.display().to_string();
                let has_start = !doc.global.entry_points.is_empty();
                self.prefs.push_recent(&cfg);
                if let Some(r) = &self.roots {
                    self.ui.maps = install::list_maps(r);
                }
                self.ed.toast(format!("{} {dir}", tr("Saved:", "Сохранено:")));
                // (junction plates nothing uses any more, from earlier sessions)
                let gone = self.ed.clean_junction_files();
                if gone > 0 {
                    log::info!("removed {gone} unused junction file(s)");
                }
                if !has_start {
                    self.ed.toast(tr("Tip: place a start point (Game logic > Start points) so the bus has somewhere to appear", "Совет: поставьте точку старта (Служебное > Точки старта), чтобы автобусу было где появиться"));
                }
                true
            }
            Err(e) => {
                self.ed.error(format!("{}: {e}", tr("Could not save (the changes are still here, nothing is lost)", "Не удалось сохранить (изменения остались здесь, ничего не потеряно)")));
                false
            }
        }
    }

    /// Bring an autosave's changes back (or drop them).
    fn recover_autosave(&mut self, yes: bool, meta: &crate::doc::autosave::Meta) {
        let Some(roots) = &self.roots else { return };
        if !yes {
            crate::doc::autosave::clear(&roots.content);
            return;
        }
        match crate::doc::autosave::restore(roots, meta) {
            Ok(doc) => {
                self.after_open(doc);
                self.ed.toast(tr("The unsaved changes are back. Save to keep them.", "Несохранённые изменения возвращены. Сохраните, чтобы оставить их."));
            }
            Err(e) => self.ed.error(format!("{}: {e}", tr("Could not restore the changes", "Не удалось вернуть изменения"))),
        }
    }

    /// Map > Check in openOMSI: saved, then run by the game offscreen with its checks, its
    /// picture taken from the view as it is.
    fn game_check(&mut self) {
        if self.game.is_none() {
            self.ed.error(tr("openOMSI was not found", "openOMSI не найдена"));
            return;
        }
        if !self.save() {
            return;
        }
        let (Some(game), Some(roots), Some(doc)) = (&self.game, &self.roots, &self.ed.doc) else { return };
        let folder = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let eye = self.cam.wanted_eye();
        let cam = [eye.x, eye.y, eye.z, self.cam.yaw, -self.cam.pitch, (self.cam.fov_y as f64).to_degrees()];
        self.gamecheck.start(game, roots, &folder, cam, doc.tiles.len());
    }

    /// openOMSI's arguments for a drive from the start point nearest the view (its place in
    /// global.cfg's list); None, with a word why, when the map has none.
    fn nearest_entry_args(&mut self) -> Option<Vec<String>> {
        let doc = self.ed.doc.as_ref()?;
        let here = self.cam.target.truncate();
        let at = |id: i64| doc.tiles.iter().find_map(|(k, t)| t.tile.objects.iter().find(|o| o.id == id).map(|o| tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1])));
        let best = doc.global.entry_points.iter().enumerate().filter_map(|(i, e)| at(e.object_id).map(|p| (i, (p - here).length(), e.name.clone()))).min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((i, _, name)) => {
                self.ed.toast(format!("{} «{name}»", tr("From the start point", "С точки старта")));
                Some(vec!["--entry".into(), i.to_string()])
            }
            None => {
                self.ed.error(tr("The map has no start point: place one with Game logic > Start points", "На карте нет точки старта: поставьте её в «Служебное» > «Точки старта»"));
                None
            }
        }
    }

    /// openOMSI's arguments for a free camera where the view is.
    fn view_args(&self) -> Option<Vec<String>> {
        let eye = self.cam.wanted_eye();
        Some(vec!["--view".into(), "free".into(), "--cam".into(), format!("{:.2},{:.2},{:.2},{:.1},{:.1}", eye.x, eye.y, eye.z, self.cam.yaw, -self.cam.pitch)])
    }

    fn play(&mut self, extra: Vec<String>) -> bool {
        // (one session at a time: a second click or key press must not start another)
        if self.drive.is_running() {
            self.ed.toast(tr("A test drive is running: close it first", "Тест-драйв уже запущен: сначала закройте его"));
            return false;
        }
        if !self.save() {
            return false;
        }
        let (Some(game), Some(roots), Some(doc)) = (&self.game, &self.roots, &self.ed.doc) else { return false };
        let folder = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let mut cmd = std::process::Command::new(game);
        cmd.arg("--root").arg(&roots.omsi).arg("--map").arg(format!("maps/{folder}/global.cfg")).args(&extra);
        if let Some(dir) = game.parent() {
            cmd.current_dir(dir);
        }
        cmd.env("OMSI_CONTENT", &roots.content);
        match cmd.spawn() {
            Ok(child) => {
                self.drive.started(child);
                self.ed.toast(tr("Starting openOMSI…", "Запускаем openOMSI…"));
                true
            }
            Err(e) => {
                self.ed.error(format!("openOMSI: {e}"));
                false
            }
        }
    }

    /// A test drive in OMSI 2 itself: the saved map and the files made for it are copied into
    /// the installation (asked once: the installation is otherwise never written to) and
    /// OMSI 2 is started (on Windows; elsewhere it is only copied).
    fn play_original(&mut self) {
        let Some(root) = self.roots.as_ref().map(|r| r.omsi.clone()) else { return };
        if self.ed.doc.is_none() {
            return;
        }
        if !self.prefs.omsi2_copy_ok {
            let yes = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("openMapper")
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_description(format!(
                    "{}\n\n{}",
                    tr("To drive the map in OMSI 2 itself it has to be copied into the OMSI 2 folder (Maps, and the objects, roads and textures made for it). Nothing that is there is replaced except earlier copies of this map. Copy it there?", "Чтобы проехать карту в самом OMSI 2, её нужно скопировать в папку OMSI 2 (Maps, а также объекты, дороги и текстуры, сделанные для неё). Ничего из имеющегося не заменяется, кроме прежних копий этой карты. Копировать?"),
                    root.display()
                ))
                .show();
            if yes != rfd::MessageDialogResult::Yes {
                return;
            }
            self.prefs.omsi2_copy_ok = true;
            self.prefs.save();
        }
        if !self.save() {
            return;
        }
        let (Some(roots), Some(doc)) = (&self.roots, &self.ed.doc) else { return };
        let plan = crate::export::plan(doc, roots);
        let folder = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        match crate::export::install_into(&plan, &root) {
            Ok((copied, same)) => {
                log::info!("OMSI 2 test drive: {copied} files copied into {}, {same} already there", root.display());
                match crate::install::omsi2_exe(&root) {
                    Some(exe) if cfg!(windows) => {
                        let mut cmd = std::process::Command::new(&exe);
                        cmd.current_dir(&root);
                        match cmd.spawn() {
                            Ok(_) => self.ed.toast(format!("{} \"{folder}\"", tr("Starting OMSI 2: choose the map", "Запускаем OMSI 2: выберите карту"))),
                            Err(e) => self.ed.error(format!("OMSI 2: {e}")),
                        }
                    }
                    _ => self.ed.toast(format!("{} {}", tr("The map is copied into the OMSI 2 folder; OMSI 2 itself runs on Windows only:", "Карта скопирована в папку OMSI 2; сам OMSI 2 работает только в Windows:"), root.display())),
                }
            }
            Err(e) => self.ed.error(format!("{}: {e}", tr("Could not copy the map into OMSI 2", "Не удалось скопировать карту в OMSI 2"))),
        }
    }

    /// Content files changed on disk: the catalogue, models and pictures are read again.
    /// The view in a season's textures: the models and the ground read again with them.
    fn set_season(&mut self, f: Option<&'static str>) {
        self.ui.season = f;
        omsi_texture::set_season_folder(f.map(str::to_string));
        if let Some(a) = &mut self.assets {
            a.clear();
        }
        if let Some(g) = &mut self.gfx {
            g.renderer.reset_models();
        }
    }

    /// Look for the ground textures on a thread of its own; `poll_textures` takes the result.
    fn scan_textures(&mut self, roots: &Roots) {
        let (tx, rx) = std::sync::mpsc::channel();
        let roots = roots.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ground_texture_files(&roots));
        });
        self.texture_scan = Some(rx);
    }

    fn poll_textures(&mut self) {
        if let Some(rx) = &self.maps_scan {
            match rx.try_recv() {
                Ok(list) => {
                    self.ui.maps = list;
                    self.maps_scan = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.maps_scan = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.texture_scan {
            match rx.try_recv() {
                Ok(files) => {
                    self.texture_files = files;
                    self.texture_scan = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.texture_scan = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Wait for the ground textures (scripted runs that need them at once).
    fn wait_textures(&mut self) {
        if let Some(rx) = self.texture_scan.take() {
            if let Ok(files) = rx.recv() {
                self.texture_files = files;
            }
        }
    }

    fn reload_content(&mut self) {
        let Some(roots) = &self.roots else { return };
        omsi_cfg::content_changed();
        self.lib.start_scan(roots);
        if let Some(a) = &mut self.assets {
            a.clear();
        }
        if let Some(g) = &mut self.gfx {
            g.renderer.reset_models();
            g.renderer.thumbs.clear();
        }
        let roots = roots.clone();
        self.scan_textures(&roots);
        self.ui.pbr_made = None;
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        let Some(roots) = &self.roots else { return };
        let mut placed = None;
        for p in paths {
            match crate::import::import(&p, &roots.content) {
                Ok(r) => {
                    self.ed.toast(r.message);
                    if let Some(o) = r.objects.first() {
                        placed = Some(o.clone());
                    }
                }
                Err(e) => self.ed.error(e.to_string()),
            }
        }
        // look again: the new things show in the catalogue
        self.reload_content();
        if let (Some(o), true) = (placed, self.ed.doc.is_some()) {
            self.ed.set_tool(Tool::Objects);
            self.ed.object_type = Some(o);
            self.ed.toast(tr("Click on the map to place it", "Кликните по карте, чтобы поставить"));
        }
    }

    fn do_action(&mut self, a: Action, el: &ActiveEventLoop) {
        let dirty = self.ed.doc.as_ref().map(|d| d.is_dirty()).unwrap_or(false);
        let guarded = matches!(a, Action::Close | Action::Open(_) | Action::OpenDialog | Action::NewMap { .. } | Action::Quit | Action::RestoreBackup(_));
        if guarded {
            self.chrono.end_session(&mut self.ed);
        }
        if guarded && dirty && !self.unguarded {
            self.ui.confirm = Some(a);
            return;
        }
        match a {
            Action::Force(inner) => {
                // the user chose to drop the changes: not asked again. (The map keeps them,
                // marked as changes, until it is really replaced: a file dialog cancelled or a
                // map that does not open leaves it as it was.)
                self.unguarded = true;
                self.do_action(*inner, el);
                self.unguarded = false;
            }
            Action::SaveThen(inner) => {
                if self.save() {
                    self.do_action(Action::Force(inner), el);
                }
            }
            Action::NewMap { folder, friendly, size, options } => self.new_map(&folder, &friendly, size, &options),
            Action::Open(p) => self.open_map(&p),
            Action::OpenDialog => {
                let start = self.roots.as_ref().map(|r| r.content.join("maps"));
                let mut dlg = rfd::FileDialog::new().add_filter("OMSI map", &["cfg"]).set_title(tr("Open a map (its global.cfg)", "Открыть карту (её global.cfg)"));
                if let Some(s) = start {
                    dlg = dlg.set_directory(s);
                }
                if let Some(p) = dlg.pick_file() {
                    self.open_map(&p);
                }
            }
            Action::Save => {
                self.save();
            }
            Action::RestoreBackup(b) => {
                let Some(dir) = self.ed.doc.as_ref().map(|d| d.dir.clone()) else { return };
                match crate::doc::safe::restore(&dir, &b) {
                    Ok(n) => {
                        omsi_cfg::content_changed();
                        if let Some(d) = self.ed.doc.as_mut() {
                            // (what is on disk now is what gets opened)
                            d.global_dirty = false;
                            d.timetable.dirty = false;
                            d.tiles.values_mut().for_each(|t| t.dirty = false);
                        }
                        self.open_map(&dir.join("global.cfg"));
                        self.ed.toast(format!("{} ({n})", tr("The map is back as it was before that save", "Карта возвращена к виду до того сохранения")));
                    }
                    Err(e) => self.ed.error(format!("{}: {e}", tr("Could not go back", "Не удалось вернуть"))),
                }
            }
            Action::ExportPlan => {
                if let Some(doc) = &self.ed.doc {
                    let name = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into());
                    if let Some(out) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).set_file_name(format!("{name} plan.svg")).set_title(tr("Export a plan of the map", "Экспорт плана карты")).save_file() {
                        match std::fs::write(&out, crate::plan::svg(doc, &self.lib, &omsi_cfg::content_roots())) {
                            Ok(()) => {
                                self.ed.toast(format!("{} {}", tr("Plan saved:", "План сохранён:"), out.display()));
                                reveal(&out);
                            }
                            Err(e) => self.ed.error(format!("{}: {e}", out.display())),
                        }
                    }
                }
            }
            Action::MergeTiles => {
                if let Some(r) = &self.roots {
                    self.ui.merge.show(install::list_maps(r));
                }
            }
            Action::Close => {
                self.remember_view();
                if let (Some(r), None) = (&self.roots, &self.recover) {
                    crate::doc::autosave::clear(&r.content);
                }
                self.ed.doc = None;
                self.ed.history.clear();
                if let Some(r) = &self.roots {
                    self.ui.maps = install::list_maps(r);
                }
            }
            Action::Quit => {
                self.remember_view();
                self.quit = true;
                el.exit();
            }
            Action::BindKey(cmd, chord) => {
                let preset = Preset::of(&self.prefs.keys_set, self.prefs.omsi_controls);
                crate::controls::bind(&mut self.prefs.keys, preset, cmd, chord);
                self.prefs.save();
            }
            Action::ResetKeys => {
                self.prefs.keys.clear();
                self.prefs.save();
            }
            // (the bottom bar's button and F5: close the session that runs, else ask what to drive)
            Action::Play => self.drive.button(),
            Action::DriveStart(req) => {
                let view = self.view_args().unwrap_or_default();
                let args = match &self.ed.doc {
                    Some(doc) => crate::drive::args(&req, doc, self.cam.target.truncate(), self.cam.yaw, view),
                    None => return,
                };
                self.prefs.drive = req.settings.clone();
                self.prefs.save();
                if self.play(args) {
                    self.guide.driven = true;
                }
            }
            Action::PlayOriginal => self.play_original(),
            Action::PlayHere(look) => {
                let args = if look { self.view_args() } else { self.nearest_entry_args() };
                if let Some(a) = args {
                    if self.play(a) {
                        self.guide.driven = true;
                    }
                }
            }
            Action::Undo => self.ed.undo(),
            Action::Redo => self.ed.redo(),
            Action::Mesh(a) => self.mesh_action(a),
            Action::Import => {
                let files = rfd::FileDialog::new().add_filter(tr("Mods, models and pictures", "Моды, модели и картинки"), &["zip", "obj", "gltf", "glb", "fbx", "x", "o3d", "sco", "sli", "png", "jpg", "jpeg", "webp", "bmp", "tga", "tif"]).set_title(tr("Import assets", "Импорт ассетов")).pick_files();
                if let Some(f) = files {
                    self.import(f);
                } else if let Some(d) = rfd::FileDialog::new().set_title(tr("…or choose a mod folder", "…или выберите папку мода")).pick_folder() {
                    self.import(vec![d]);
                }
            }
            Action::CreatePrimitive(i) => {
                let (Some(roots), Some(p)) = (&self.roots, openmapper_mesh::primitives::ALL.get(i).copied()) else { return };
                let map = self.ed.doc.as_ref().and_then(|d| d.dir.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Shared".into());
                let mut src = crate::editable::primitive_source(p);
                src.name = crate::ui_mesh::primitive_label(&p).to_string();
                let rel = crate::editable::new_rel(roots, &map, p.name());
                match crate::editable::save(roots, &rel, &src) {
                    Ok(()) => {
                        self.lib.start_scan(roots);
                        if self.ed.doc.is_some() {
                            self.ed.set_tool(Tool::Objects);
                            self.ed.object_type = Some(rel);
                            self.ed.toast(tr("Click on the map to put it there", "Кликните по карте, чтобы поставить"));
                        }
                    }
                    Err(e) => self.ed.error(format!("{}: {e}", tr("Could not make the shape", "Не удалось создать фигуру"))),
                }
            }
            Action::ChooseOmsi => {
                if let Some(d) = rfd::FileDialog::new().set_title(tr("Choose the OMSI 2 folder", "Выберите папку OMSI 2")).pick_folder() {
                    let ok = omsi_cfg::install_search::root_guesses(&d).into_iter().find(|g| install::is_omsi_root(g));
                    match ok {
                        Some(r) => {
                            self.prefs.omsi_root = r.to_string_lossy().to_string();
                            self.prefs.save();
                            self.setup_roots();
                        }
                        None => self.ed.error(tr("That is not an OMSI 2 folder (Omsi.exe, maps and Vehicles must be in it)", "Это не папка OMSI 2 (в ней должны быть Omsi.exe, maps и Vehicles)")),
                    }
                }
            }
            Action::SetLang(l) => {
                i18n::set(l);
                // (a script the interface font has not is drawn by a system font)
                if let Some(g) = &self.gfx {
                    g.egui_ctx.set_fonts(crate::icons::fonts());
                }
                self.prefs.language = i18n::code(l).to_string();
                self.prefs.save();
                if let Some(r) = &self.roots {
                    // (category names are in the language)
                    self.lib.start_scan(r);
                }
            }
            Action::SetGraphics(q) => {
                self.prefs.graphics = q.code().to_string();
                self.prefs.save();
                // (edge smoothing is built into the renderer's pipelines: it is made again,
                // and fills itself in from the loaded models within a few frames)
                if let Some(g) = &mut self.gfx {
                    for t in g.renderer.thumbs.values().flatten() {
                        g.egui_renderer.free_texture(&t.id);
                    }
                    g.gpu.set_samples(q.samples());
                    let roots = std::mem::take(&mut g.renderer.roots);
                    g.renderer = Renderer::new(&g.gpu, q);
                    g.renderer.roots = roots;
                }
            }
            Action::PbrForObject(rel) => {
                if let (Some(a), Some(r)) = (self.assets.as_mut(), &self.roots) {
                    let textures = object_textures(a, &rel);
                    if textures.is_empty() {
                        self.ed.toast(tr("This object has no textures to make PBR maps of", "У этого объекта нет текстур для PBR-карт"));
                    } else {
                        self.pbr.open_for(textures, r);
                    }
                }
            }
            Action::RoadTypes => self.road_types.open = !self.road_types.open,
            Action::ComplexJunctions => self.cj.open = !self.cj.open,
            Action::PlaceComplex(layout, turn) => {
                let at = self.cam.target.truncate();
                if let Some(assets) = self.assets.as_mut() {
                    self.ed.place_layout(assets, &layout, at, turn);
                }
            }
            Action::SaveComplexObject(layout) => {
                let mut layout = *layout;
                layout.baked = None;
                let (Some(roots), Some(assets)) = (self.roots.clone(), self.assets.as_mut()) else { return };
                match crate::cjunction::bake(&roots, &layout, assets, false) {
                    Ok(_) => {
                        self.reload_content();
                        self.cj.succeeded(tr("Saved as an object", "Сохранён как объект").into());
                        self.ed.toast(tr("Saved as an object: it is in the catalogue under Objects, to place like any other", "Сохранён как объект: он в каталоге «Объекты», ставится как любой другой"));
                    }
                    Err(e) => self.cj.failed(e.to_string()),
                }
            }
            Action::SaveCustomJunction(layout) => {
                let mut layout = *layout;
                let (Some(roots), Some(assets)) = (self.roots.clone(), self.assets.as_mut()) else { return };
                match crate::cjunction::bake(&roots, &layout, assets, true).and_then(|rel| {
                    layout.baked = Some(rel.clone());
                    layout.save(&roots.content)?;
                    Ok(rel)
                }) {
                    Ok(rel) => {
                        self.ed.custom_layouts = None;
                        self.cj.saved_as(rel);
                        self.reload_content();
                        self.ed.toast(tr("Custom junction saved: the Roads tool's automatic junctions use it where roads meet as it was drawn", "Свой перекрёсток сохранён: автоматические перекрёстки «Дорог» ставят его там, где дороги сходятся как нарисовано"));
                    }
                    Err(e) => self.cj.failed(e.to_string()),
                }
            }
            Action::Scripts => self.scripts.open = !self.scripts.open,
            Action::ScriptedLight(b, color, range) => {
                if let Err(e) = self.new_scripted_light(b, color, range) {
                    self.ed.error(e.to_string());
                }
            }
            Action::PlaceAsset(rel) => {
                self.ed.set_tool(Tool::Objects);
                self.ed.object_type = Some(rel);
                self.ed.place_heading = 0.0;
                if let Some(a) = self.assets.as_mut() {
                    self.ed.place_now(a, &self.lib);
                }
            }
            Action::JunctionAtEnd(k, id, at_end, piece) => {
                if let Some(a) = self.assets.as_mut() {
                    if let Err(e) = self.ed.junction_at_end(a, &self.lib, k, id, at_end, &piece) {
                        self.ed.error(e);
                    }
                }
            }
            Action::RevealMap(cfg) => reveal(&cfg),
            Action::CopyMap(cfg) => {
                if let Some(r) = &self.roots {
                    match copy_map(r, &cfg) {
                        Ok(to) => {
                            self.ui.maps = install::list_maps(r);
                            self.ui.welcome_pick = Some(to);
                        }
                        Err(e) => self.ed.error(format!("{}: {e:#}", tr("Could not copy the map", "Не удалось скопировать карту"))),
                    }
                }
            }
            Action::BinMap(cfg) => {
                if let Some(r) = &self.roots {
                    match bin_map(&r.content, &cfg) {
                        Ok(()) => {
                            self.ui.maps = install::list_maps(r);
                            self.ui.welcome_pick = None;
                        }
                        Err(e) => self.ed.error(format!("{}: {e:#}", tr("Could not move the map", "Не удалось перенести карту"))),
                    }
                }
            }
            Action::RevealAsset(rel) => {
                if let Some(r) = &self.roots {
                    reveal(&r.resolve(&rel));
                }
            }
            Action::DeleteAsset(rel) => {
                if let Some(r) = &self.roots {
                    match bin_asset(&r.content, &rel) {
                        Ok(()) => self.reload_content(),
                        Err(e) => self.ed.error(e.to_string()),
                    }
                }
            }
            Action::FlattenTerrain => {
                let n = self.ed.flatten_terrain();
                self.ed.toast(format!("{} {n}", tr("Terrain flattened; splines laid on it:", "Рельеф убран; сплайнов уложено:")));
            }
            Action::DetachLanes => {
                if let (Some(a), Some(r)) = (self.assets.as_mut(), &self.roots) {
                    self.ed.detach_lanes(a, r);
                }
            }
            Action::PbrForSpline(rel) => {
                if let (Some(a), Some(r)) = (self.assets.as_mut(), &self.roots) {
                    let textures = spline_textures(a, &rel);
                    if textures.is_empty() {
                        self.ed.toast(tr("This road type has no textures to make PBR maps of", "У этого типа дороги нет текстур для PBR-карт"));
                    } else {
                        self.pbr.open_for(textures, r);
                    }
                }
            }
            Action::PbrMaker => {
                if let (Some(f), Some(r)) = (rfd::FileDialog::new().add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "jpeg", "bmp", "tga", "dds"]).pick_file(), &self.roots) {
                    self.pbr.open_for(vec![f], r);
                }
            }
            Action::SetSeason(f) => self.set_season(f),
            Action::SetAlpha(file, mesh, texture, index, alpha) => {
                let path = self.roots.as_ref().map(|r| omsi_cfg::resolve_path(&r.content, &file));
                match path.map(|p| crate::import::set_alpha(&p, mesh, &texture, index, alpha)) {
                    Some(Ok(())) => {
                        self.reload_content();
                        self.ui.pbr_made = None;
                    }
                    Some(Err(e)) => self.ed.error(e.to_string()),
                    None => {}
                }
            }
            Action::ReloadContent(msg) => {
                self.reload_content();
                self.ed.toast(msg);
            }
            Action::SetStreamTiles(n) => {
                self.prefs.stream_tiles = n;
                self.prefs.tile_streaming = Some(true);
                self.prefs.save();
                self.stream_at = None;
                self.stream_set = None;
                self.ed.toast(format!("{} {}", stream_count(n), tr("tiles round the camera are loaded; click on the empty zone to load the next ones", "тайлов вокруг камеры загружено; клик по пустой зоне загрузит следующие")));
            }
            Action::SetTileStreaming(on) => {
                self.prefs.tile_streaming = Some(on);
                self.prefs.save();
                self.stream_at = None;
                self.stream_set = None;
                self.ed.toast(if on { tr("Only 100 tiles round the camera are loaded; click on the empty zone to load the next 100", "Загружены только 100 тайлов вокруг камеры; клик по пустой зоне загрузит следующие 100") } else { tr("All tiles are loaded", "Загружены все тайлы") }.to_string());
            }
            Action::SetOmsiControls(on) => {
                self.prefs.omsi_controls = on;
                self.prefs.keys_set = if on { "omsi" } else { "mapper" }.into();
                self.prefs.save();
                self.ed.toast(if on {
                    tr("OMSI 2's editor: the middle button turns, the right button held and moved zooms, a right click goes there (Shift: menu); G moves the selection, N makes a new object, Enter confirms, the tile grid is on Shift+G", "Редактор OMSI 2: средняя кнопка вращает, правая зажатая с движением приближает, правый клик переходит туда (Shift: меню); G двигает выделенное, N создаёт объект, Enter подтверждает, сетка тайлов на Shift+G").to_string()
                } else {
                    tr("openMapper's own controls: the camera as chosen under Camera, the keys as in View → Controls", "Собственное управление openMapper: камера как выбрано в «Камера», клавиши как в «Вид → Управление»").to_string()
                });
            }
            Action::SetKeySet(p) => {
                self.prefs.keys_set = p.id().to_string();
                // (OMSI 2's editor is its mouse and its keys together; the other two sets go
                // with openMapper's own mouse)
                self.prefs.omsi_controls = p == Preset::Omsi;
                self.prefs.save();
                self.ed.toast(match p {
                    Preset::Blender => tr("Blender's keys: G moves the selection, Shift+S scales it, X deletes, Tab edits the mesh, Shift+A adds an object, Shift+D duplicates, F3 searches. W A S D and Q E move the camera as they always do", "Клавиши Blender: G двигает выделенное, Shift+S масштабирует, X удаляет, Tab правит меш, Shift+A добавляет объект, Shift+D дублирует, F3 ищет. W A S D и Q E двигают камеру, как всегда"),
                    _ => tr("The keys as in View → Controls", "Клавиши как в «Вид → Управление»"),
                }.to_string());
            }
            Action::SetCamTune(t) => {
                let t = t.sane();
                self.cam.fly_speed = t.fly_speed;
                self.cam.tune = t;
                self.prefs.cam_tune = t;
                self.prefs.save();
            }
            Action::SetGraphicsApi(a) => {
                self.prefs.graphics_api = a.code().to_string();
                self.prefs.save();
                self.ed.toast(tr("The graphics API changes the next time openMapper starts", "Графический API сменится при следующем запуске openMapper").to_string());
            }
            Action::SetCamera(c) => {
                self.prefs.camera = c;
                self.prefs.save();
            }
            Action::SetScale(z) => {
                self.prefs.ui_scale = z;
                self.prefs.save();
                if let Some(g) = &self.gfx {
                    g.egui_ctx.set_zoom_factor(z);
                }
            }
            Action::Copy => {
                self.ed.copy(&self.lib);
            }
            Action::Cut => self.ed.cut(&self.lib),
            Action::Paste => self.ed.paste(&self.lib),
            Action::PasteAt(at) => {
                if at.is_some() {
                    self.ed.cursor = at;
                }
                self.ed.paste(&self.lib);
            }
            Action::SavedGroups => self.ui.groups.open = true,
            Action::Duplicate => self.ed.duplicate(&self.lib),
            Action::SelectAll => self.ed.select_all(),
            Action::BuildRoad => {
                if let Some(assets) = self.assets.as_mut() {
                    self.ed.confirm_road(assets, &self.lib);
                }
            }
            Action::FocusSelection => {
                if let (Some(sel), Some(doc)) = (self.ed.selected, &self.ed.doc) {
                    let p = match sel {
                        Sel::Object(k, id) => doc.object(k, id).map(|o| doc.object_world(k, o, false)),
                        Sel::Spline(k, id) => doc.spline(k, id).map(|s| omsi_geometry::SplineCurve::from_map(s, tile_origin(k)).point_at(s.length * 0.5)),
                        Sel::Attached(k, id) => crate::selection::attachment_first(doc, k, id).map(|f| f.pose.pos),
                    };
                    if let Some(p) = p {
                        self.cam.target = p;
                        self.cam.distance = self.cam.distance.min(120.0);
                    }
                }
            }
            Action::Timetable => self.tt.open = !self.tt.open,
            Action::Traffic => self.traffic.open = !self.traffic.open,
            Action::Geo => self.geo.open = !self.geo.open,
            Action::Osm => self.osm.open = !self.osm.open,
            Action::Trace => self.trace.open = !self.trace.open,
            Action::GameCheck => self.game_check(),
            Action::Guide => self.guide.open = !self.guide.open,
            Action::Multiplayer => self.collab.open = !self.collab.open,
            Action::Updates => self.updater.window = true,
            Action::Export => self.export(),
            Action::ToggleFavourite(rel) => {
                self.prefs.toggle_favourite(&rel);
                self.ui.favourites = self.prefs.favourites.clone();
            }
            Action::UsedAsset(rel) => {
                self.prefs.push_recent_asset(&rel);
                self.ui.recent_assets = self.prefs.recent_assets.clone();
            }
            Action::ImportMapPicture => {
                let Some(file) = rfd::FileDialog::new().add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "jpeg", "bmp", "tga", "webp", "gif"]).set_title(tr("Import the map's thumbnail", "Импорт картинки карты")).pick_file() else { return };
                if let Some(doc) = &self.ed.doc {
                    let done = image::open(&file).map_err(anyhow::Error::from).and_then(|img| {
                        let img = img.to_rgba8();
                        let crop = [0, 0, img.width(), img.height()];
                        save_map_picture(img, crop, &doc.dir.join("picture.jpg"))
                    });
                    match done {
                        Ok(()) => self.ed.toast(tr("The map's thumbnail is set: openOMSI shows it where maps are chosen.", "Картинка карты задана: openOMSI показывает её при выборе карты.")),
                        Err(e) => self.ed.error(format!("{}: {e}", tr("Could not use the picture", "Не удалось использовать картинку"))),
                    }
                }
            }
            Action::ViewPicture => self.snapshot = true,
            Action::MapSettings => self.map_settings.open = true,
            Action::Calendar => self.calendar.open = true,
            Action::SignalRoutes => self.signals.open = true,
            Action::Chrono => self.chrono.open = true,
            Action::OpenLines => self.tt.open = true,
            Action::OpenTraffic => self.traffic.open = true,
            Action::Depots => self.traffic.open_at("depots"),
            Action::SetMapCamera => {
                if let Some(doc) = self.ed.doc.as_mut() {
                    let p = self.cam.target.truncate();
                    let k = crate::doc::tile_of(p);
                    let l = p - tile_origin(k);
                    self.ed.history.push_snapshot(tr("Map camera", "Камера карты"), crate::history::Snapshot { global: Some((doc.global.clone(), doc.order.clone())), ..Default::default() });
                    doc.global.map_cam = vec![k.0 as f64, k.1 as f64, l.x, self.cam.target.z, l.y, self.cam.yaw, self.cam.pitch, self.cam.distance / 4.0];
                    doc.global_dirty = true;
                    self.ed.toast(tr("The map opens with this view from now on.", "Теперь карта открывается с этого вида."));
                }
            }
            Action::LookAt(p) => {
                if let Some(doc) = &self.ed.doc {
                    self.cam.target = DVec3::new(p.x, p.y, doc.ground(p));
                }
            }
            Action::SetGroundTexture(layer, detail, rel) => self.ed.set_ground_texture(layer, detail, &rel),
            Action::AddGroundTexture(rel) => {
                if let Some(doc) = self.ed.doc.as_mut() {
                    self.ed.history.begin(tr("Add ground texture", "Добавление текстуры земли"));
                    self.ed.history.touch_global(doc);
                    let detail = if rel.to_lowercase().contains("gras") { "Texture\\gras_det.bmp" } else { "Texture\\noise_low.bmp" };
                    doc.global.ground_textures.push(omsi_map::GroundTex { texture: rel, detail_texture: detail.into(), params: [8.0, 16.0, 4.0] });
                    doc.sync_mask_slots();
                    doc.global_dirty = true;
                    doc.global_rev = crate::doc::next_rev();
                    self.ed.history.commit();
                    self.ed.paint_layer = doc.global.ground_textures.len() - 1;
                    self.ed.set_tool(Tool::Paint);
                }
            }
        }
    }

    // ------------------------------------------------------------------ input

    fn key(&mut self, code: Option<KeyCode>, logical: &Key, pressed: bool, el: &ActiveEventLoop) {
        let map = Keymap::of(&self.prefs.keys, Preset::of(&self.prefs.keys_set, self.prefs.omsi_controls));
        if let Some(code) = code {
            if pressed && self.ui.key_capture.is_some() {
                self.capture_key(code);
                return;
            }
            // (the edit mode of a mesh takes its keys first, Blender's where it can)
            if pressed && self.mesh_key(code) {
                return;
            }
            let held = [(Cmd::Forward, 0), (Cmd::Back, 1), (Cmd::Left, 2), (Cmd::Right, 3), (Cmd::TurnLeft, 4), (Cmd::TurnRight, 5)];
            for (cmd, which) in held {
                if map.is_key(cmd, code) {
                    match which {
                        0 => self.keys.w = pressed,
                        1 => self.keys.s = pressed,
                        2 => self.keys.a = pressed,
                        3 => self.keys.d = pressed,
                        4 => self.keys.q = pressed,
                        _ => self.keys.e = pressed,
                    }
                }
            }
            match code {
                KeyCode::ArrowUp => self.keys.arrows[0] = pressed,
                KeyCode::ArrowLeft => self.keys.arrows[1] = pressed,
                KeyCode::ArrowDown => self.keys.arrows[2] = pressed,
                KeyCode::ArrowRight => self.keys.arrows[3] = pressed,
                _ => {}
            }
        }
        if !pressed || self.ed.doc.is_none() {
            return;
        }
        // (a grab holds the keyboard, as Blender's does: X, Y, Z lock it, numbers say how far,
        // Enter or Esc lets it go; Ctrl is left to the rest, for an undo)
        if self.ed.grab.is_some() && !self.keys.ctrl {
            self.grab_key(code);
            return;
        }
        let ctrl = self.keys.ctrl;
        let shift = self.keys.shift;
        let mut actions = Vec::new();
        let cmd = code.and_then(|c| map.find(c, ctrl, shift));
        match code {
            Some(KeyCode::KeyY) if ctrl => actions.push(Action::Redo),
            Some(KeyCode::KeyP) if ctrl && shift => self.ui.palette.show(),
            // Ctrl+1…9: a saved view gone to (Shift: the view kept there)
            Some(k @ (KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9)) if ctrl => {
                let n = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9].iter().position(|d| *d == k).unwrap_or(0);
                self.ui.views.follow(self.ed.doc.as_ref().map(|d| d.dir.as_path()));
                if shift {
                    let at = self.ui.views.store(n, &self.cam);
                    self.ed.toast(format!("{} {}", tr("View kept as", "Вид запомнен как"), at + 1));
                } else if !self.ui.views.go(n, &mut self.cam) {
                    self.ed.toast(format!("{} {} (Ctrl+Shift+{})", tr("No view kept as", "Нет вида под номером"), n + 1, n + 1));
                }
            }
            // Alt + arrows: the selection a step (the grid's; Shift: 10 cm) along the map's axis
            // nearest to the way the arrow points on the screen
            Some(k @ (KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight)) if self.keys.alt && self.ed.selected.is_some() => {
                let (f, r, _) = self.cam.axes();
                let ground = |v: DVec3| DVec2::new(v.x, v.y).normalize_or(DVec2::Y);
                let dir = match k {
                    KeyCode::ArrowUp => ground(f),
                    KeyCode::ArrowDown => -ground(f),
                    KeyCode::ArrowRight => ground(r),
                    _ => -ground(r),
                };
                let axis = if dir.x.abs() > dir.y.abs() { DVec2::new(dir.x.signum(), 0.0) } else { DVec2::new(0.0, dir.y.signum()) };
                let step = if shift { 0.1 } else { self.ed.grid_step };
                self.ed.move_selection_by(axis * step);
            }
            // Alt + Page Up / Down: the selection up or down 10 cm (Shift: 1 m)
            Some(k @ (KeyCode::PageUp | KeyCode::PageDown)) if self.keys.alt && self.ed.selected.is_some() => {
                let step = if shift { 1.0 } else { 0.1 };
                self.ed.raise_selection(if k == KeyCode::PageUp { step } else { -step });
            }
            Some(KeyCode::Backspace) | Some(KeyCode::Delete) => self.ed.delete_selected(),
            _ => {
                if let Some(c) = cmd {
                    self.run_cmd(c, shift, &mut actions);
                }
            }
        }
        if let Key::Named(NamedKey::Escape) = logical {
            if self.ui.view_menu.take().is_some() {
                // (only the view's menu closes)
            } else if std::mem::take(&mut self.ed.cutting) {
                // (only the knife goes, the selection stays)
            } else if self.ed.link_pick.take().is_some() {
                // (only the linking is called off, the object stays selected)
            } else if self.ed.tool != Tool::Select && !self.ed_has_pending() {
                self.ed.set_tool(Tool::Select);
            } else {
                // (Select: Esc lets the selection go; another tool calls off what it was doing)
                if self.ed.right_click() {
                    self.ed.select(None);
                }
                self.ed.cancel();
            }
        }
        for a in actions {
            self.do_action(a, el);
        }
    }

    /// The keys a grab takes (Blender's G in the Blender and OMSI key sets): the axis to lock
    /// it to, the distance typed, and the end of it. Everything else is held off meanwhile.
    fn grab_key(&mut self, code: Option<KeyCode>) {
        let Some(code) = code else { return };
        let digit = |c: KeyCode| -> Option<u8> {
            let rows = [
                [KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9],
                [KeyCode::Numpad0, KeyCode::Numpad1, KeyCode::Numpad2, KeyCode::Numpad3, KeyCode::Numpad4, KeyCode::Numpad5, KeyCode::Numpad6, KeyCode::Numpad7, KeyCode::Numpad8, KeyCode::Numpad9],
            ];
            rows.iter().find_map(|r| r.iter().position(|k| *k == c).map(|n| n as u8))
        };
        match code {
            KeyCode::Escape => {
                self.ed.grab_cancel();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.ed.grab_confirm();
            }
            KeyCode::KeyX => self.ed.grab_axis(0),
            KeyCode::KeyY => self.ed.grab_axis(1),
            KeyCode::KeyZ => self.ed.grab_axis(2),
            KeyCode::Backspace => self.ed.grab_backspace(),
            KeyCode::Minus | KeyCode::NumpadSubtract => self.ed.grab_type('-'),
            KeyCode::Period | KeyCode::Comma | KeyCode::NumpadDecimal => self.ed.grab_type('.'),
            _ => {
                if let Some(n) = digit(code) {
                    self.ed.grab_type((b'0' + n) as char);
                }
            }
        }
    }

    /// Puts the command being set on the key just pressed (Esc: leaves it as it was).
    fn capture_key(&mut self, code: KeyCode) {
        if code == KeyCode::Escape {
            self.ui.key_capture = None;
        } else if crate::controls::bindable(code) {
            if let Some(cmd) = self.ui.key_capture.take() {
                let chord = crate::controls::Chord { code, ctrl: self.keys.ctrl, shift: self.keys.shift };
                let preset = Preset::of(&self.prefs.keys_set, self.prefs.omsi_controls);
                crate::controls::bind(&mut self.prefs.keys, preset, cmd, Some(chord));
                self.prefs.save();
            }
        }
    }

    fn run_cmd(&mut self, c: Cmd, shift: bool, actions: &mut Vec<Action>) {
        match c {
            // (held keys: the camera takes them every frame)
            Cmd::Forward | Cmd::Back | Cmd::Left | Cmd::Right | Cmd::TurnLeft | Cmd::TurnRight => {}
            Cmd::Undo => actions.push(Action::Undo),
            Cmd::Redo => actions.push(Action::Redo),
            Cmd::Save => actions.push(Action::Save),
            Cmd::Copy => actions.push(Action::Copy),
            Cmd::Cut => actions.push(Action::Cut),
            Cmd::Paste => actions.push(Action::Paste),
            Cmd::Duplicate => actions.push(Action::Duplicate),
            Cmd::SelectAll => actions.push(Action::SelectAll),
            Cmd::Find => self.ui.find.show(),
            Cmd::Commands => self.ui.palette.show(),
            Cmd::ToolSelect => self.ed.set_tool(Tool::Select),
            Cmd::ToolRoads => self.ed.set_tool(Tool::Roads),
            Cmd::ToolObjects => self.ed.set_tool(Tool::Objects),
            Cmd::ToolLogic => self.ed.set_tool(Tool::Logic),
            Cmd::ToolTerrain => self.ed.set_tool(Tool::Terrain),
            Cmd::ToolPaint => self.ed.set_tool(Tool::Paint),
            Cmd::ToolTiles => self.ed.set_tool(Tool::Tiles),
            Cmd::Rotate => self.ed.rotate(if shift { -self.ed.angle_step } else { self.ed.angle_step }),
            Cmd::Ruler => self.ed.toggle_ruler(),
            Cmd::Knife => self.ed.toggle_cutting(),
            Cmd::EditMesh => self.toggle_edit_mode(),
            Cmd::JoinObjects => self.mesh_action(crate::ui_mesh::MeshAction::Join),
            Cmd::XformMove | Cmd::XformRotate | Cmd::XformScale => {
                use crate::editor::Xform;
                let x = match c { Cmd::XformMove => Xform::Move, Cmd::XformRotate => Xform::Rotate, _ => Xform::Scale };
                // (Blender's G, in the Blender and OMSI key sets: the selection is taken hold of
                // and follows the pointer at once; X, Y, Z lock it. Nothing selected: the
                // handles come up instead, as openMapper has them)
                let grab = x == Xform::Move && matches!(Preset::of(&self.prefs.keys_set, self.prefs.omsi_controls), Preset::Omsi | Preset::Blender);
                if grab {
                    self.ed.set_tool(Tool::Select);
                }
                if !(grab && self.ed.grab_begin(self.mouse.pos)) {
                    self.ed.set_tool(Tool::Select);
                    self.ed.xform = if self.ed.xform == x { Xform::Select } else { x };
                }
            }
            Cmd::BuildRoad => actions.push(Action::BuildRoad),
            // (Enter, as OMSI 2's editor has it: what is being drawn is finished)
            Cmd::Confirm => actions.push(Action::BuildRoad),
            Cmd::NewObject => self.ed.new_object(&self.lib),
            Cmd::TileGrid => self.ed.show_grid = !self.ed.show_grid,
            Cmd::Group => self.ed.group_selection(),
            Cmd::Ungroup => self.ed.ungroup_selection(),
            Cmd::LookDown => {
                self.cam.yaw = (self.cam.yaw / 360.0).round() * 360.0;
                self.cam.pitch = 89.0;
            }
            Cmd::View2d => self.ui.view_2d = !self.ui.view_2d,
            Cmd::Assets => self.ui.assets.open = !self.ui.assets.open,
            Cmd::Overview => self.ui.minimap.open = !self.ui.minimap.open,
            Cmd::Lines => self.tt.open = !self.tt.open,
            Cmd::Traffic => self.traffic.open = !self.traffic.open,
            Cmd::Imagery => self.geo.open = !self.geo.open,
            Cmd::BrushSmaller => self.ed.brush_radius = (self.ed.brush_radius / 1.2).max(3.0),
            Cmd::BrushLarger => self.ed.brush_radius = (self.ed.brush_radius * 1.2).min(150.0),
            Cmd::Delete => self.ed.delete_selected(),
            Cmd::Focus => actions.push(Action::FocusSelection),
            Cmd::TestDrive => actions.push(if shift { Action::PlayHere(false) } else { Action::Play }),
            Cmd::Picture => actions.push(Action::ViewPicture),
        }
    }

    fn ed_has_pending(&self) -> bool {
        self.ed.selected.is_some() || !self.ed.plant_outline.is_empty() || self.ed.ramp_start.is_some()
    }

    /// The camera moves like a fly-through (Unity / Unreal) rather than over a map.
    /// The camera works as Unity's scene view (the default), else as a map editor's.
    fn fly_style(&self) -> bool {
        !self.prefs.omsi_controls && self.prefs.camera != "map"
    }

    /// What a drag of the map holds: the point under the cursor, else where the cursor's ray
    /// meets the level of the point looked at.
    fn grab_point(&self) -> Option<DVec3> {
        // (the cursor's own ray at the cursor point's level: several trackpad moves come
        // within one frame, before the cursor point is found again)
        let (o, d) = self.view_ray()?;
        let z = self.ed.cursor.map(|c| c.z).unwrap_or(self.cam.target.z);
        (d.z.abs() > 1e-3 && (z - o.z) / d.z > 0.0).then(|| o + d * ((z - o.z) / d.z))
    }

    fn view_ray(&self) -> Option<(DVec3, DVec3)> {
        let v = self.view;
        if !v.is_positive() {
            return None;
        }
        Some(self.cam.ray(self.mouse.pos.0 - v.min.x, self.mouse.pos.1 - v.min.y, v.width(), v.height()))
    }

    // ------------------------------------------------------------------ frame

    fn frame(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let gap = (now - self.last).as_secs_f64();
        // (a frame that took far longer than a frame: the first ones build the map's picture
        // and its models, so only those after them are worth a word)
        if gap > 0.4 && self.frames > 10 {
            log::warn!("a frame took {gap:.2} s (frame {})", self.frames);
        }
        let dt = (now - self.last).as_secs_f64().min(0.1);
        self.last = now;
        // (the view's hour when the View menu lets it run: a minute of the map's day a
        // second at 1x, as many times that as the testing speed chosen)
        if self.ui.time_flows {
            if let Some(h) = &mut self.ui.daytime {
                *h = crate::render::daylight::hour_after_at(*h, dt as f32, self.ui.time_speed);
            }
        }
        if let Some(a) = &mut self.assets {
            a.poll();
        }
        self.lib.poll();
        // (what a press on the selection does: OMSI 2's and Blender's key sets move it with G
        // alone, so a press there only picks, as it does in those editors)
        self.ed.press_drags = Preset::of(&self.prefs.keys_set, self.prefs.omsi_controls) == Preset::Mapper;
        self.collab.tick(&mut self.ed, self.roots.as_ref());
        if let Some(doc) = self.collab.take_joined() {
            self.after_open(doc);
            self.collab.joined_open(&mut self.ed);
        }
        // (what a running openOMSI writes into the content folder is not the catalogue's news)
        let game_runs = self.drive.is_running();
        if self.hot.as_ref().map(|h| h.poll()).unwrap_or(false) && !game_runs {
            log::info!("content files changed on disk: reading them again");
            self.reload_content();
            self.ed.toast(tr("Files changed on disk: the catalogue and models are updated", "Файлы на диске изменились: каталог и модели обновлены"));
        }
        // (the switches on offer, once the library is read)
        // (once per library: a library with no switch in it is not looked through every frame)
        if !self.lib.scanning && self.switches_for != self.lib.assets.len() {
            self.switches_for = self.lib.assets.len();
            self.ed.switch_catalogue = crate::switches::catalogue(&self.lib);
        }
        self.poll_textures();
        // camera
        // (looking round: all the mouse moved since the last frame, turned at once)
        if self.mouse.look_raw {
            let (dx, dy) = std::mem::take(&mut self.mouse.look_acc);
            if dx != 0.0 || dy != 0.0 {
                self.cam.look_pixels((dx.clamp(-400.0, 400.0) * 0.8) as f32, (dy.clamp(-400.0, 400.0) * 0.8) as f32);
                self.mouse.right_moved += (dx.abs() + dy.abs()) as f32;
            }
        }
        let fwd = (self.keys.w as i32 - self.keys.s as i32) as f64;
        let side = (self.keys.d as i32 - self.keys.a as i32) as f64;
        let turn = (self.keys.e as i32 - self.keys.q as i32) as f64;
        let ar = self.keys.arrows;
        let (afwd, aside) = ((ar[0] as i32 - ar[2] as i32) as f64, (ar[3] as i32 - ar[1] as i32) as f64);
        let typing = self.gfx.as_ref().map(|g| g.egui_ctx.egui_wants_keyboard_input()).unwrap_or(false);
        // (Alt + arrows move the selection, not the camera)
        let keys_on = !typing && !self.keys.ctrl && !(self.keys.alt && self.ed.selected.is_some() && ar.iter().any(|a| *a));
        if keys_on && self.ui.view_2d {
            // (the plan view: the keys always move over the map, nothing turns)
            let (m_fwd, m_side) = ((fwd + afwd).clamp(-1.0, 1.0), (side + aside).clamp(-1.0, 1.0));
            if m_fwd != 0.0 || m_side != 0.0 {
                self.cam.drive(m_fwd, m_side, 0.0, self.keys.shift, dt);
            }
        } else if keys_on && (self.fly_style() || self.mouse.right) {
            // W A S D / Q E fly only while the right button is held (Q down, E up); the arrows move
            if self.mouse.right && (fwd != 0.0 || side != 0.0 || turn != 0.0) {
                self.keys.fly_held += dt;
                self.cam.fly(fwd, side, turn, self.keys.shift, dt, self.keys.fly_held);
            } else {
                self.keys.fly_held = 0.0;
            }
            if afwd != 0.0 || aside != 0.0 {
                self.cam.fly(afwd, aside, 0.0, self.keys.shift, dt, 0.0);
            }
        } else if keys_on && (fwd != 0.0 || side != 0.0 || turn != 0.0 || afwd != 0.0 || aside != 0.0) {
            // (map style: over the map, Q E turn)
            self.cam.drive((fwd + afwd).clamp(-1.0, 1.0), (side + aside).clamp(-1.0, 1.0), turn, self.keys.shift, dt);
        }
        if let Some(doc) = &self.ed.doc {
            let h = doc.ground(self.cam.wanted_eye().truncate());
            self.cam.keep_above(h);
        }
        if crate::render::profile::orbit() {
            self.cam.yaw += dt * 12.0;
        }
        // the plan view: straight down, north up, whatever the mouse did to the tilt and the turn;
        // leaving it brings the tilt back
        self.cam.plan = self.ui.view_2d;
        if self.ui.view_2d {
            if self.pre_2d.is_none() {
                self.pre_2d = Some((self.cam.pitch, self.cam.yaw));
            }
            self.cam.pitch = 89.5;
            self.cam.yaw = (self.cam.yaw / 360.0).round() * 360.0;
        } else if let Some((p, y)) = self.pre_2d.take() {
            self.cam.pitch = p;
            self.cam.yaw = y;
        }
        let cam_moving = self.cam.is_moving();
        self.cam.update(dt);
        // the Roads tool starts with an ordinary two-lane road, so the first click builds
        if self.ed.tool == Tool::Roads && self.ed.road_type.is_none() && !self.lib.scanning {
            let roads: Vec<&crate::library::Asset> = self.lib.assets.iter().filter(|a| a.category == crate::library::Category::Roads).collect();
            let pick = roads.iter().find(|a| a.rel.to_lowercase().contains("2spur") && a.lanes.1 > 0).or_else(|| roads.first());
            self.ed.road_type = pick.map(|a| a.rel.clone());
        }
        // the editor
        let t_ed = Instant::now();
        self.ed.cursor_in_view = !self.mouse.over_ui || self.pinned_cursor.is_some();
        self.ed.set_gizmo_view(&self.cam, self.view.width(), self.view.height(), self.mouse.pos.0 - self.view.min.x, self.mouse.pos.1 - self.view.min.y);
        if let Some(p) = self.pinned_cursor {
            self.ed.cursor = Some(p);
        }
        if self.ed.doc.is_some() && (self.mouse.moved || cam_moving || self.mouse.left || self.assets.as_ref().map(|a| a.pending > 0).unwrap_or(false)) {
            if let (Some((o, d)), Some(assets)) = (self.view_ray(), self.assets.as_mut()) {
                if !self.mouse.over_ui {
                    self.ed.hover_ray(assets, &self.lib, o, d);
                }
            }
            self.mouse.moved = false;
        }
        // (a grab: the selection follows the pointer, from where it was taken)
        if self.ed.grab.is_some() {
            if let Some(at) = self.ed.cursor {
                // (the pieces a grab carries: a whole chain can be many, and what each frame
                // of it costs is what a big map answers slowly with)
                let t = Instant::now();
                crate::logging::busy("the grab: the selection following the pointer");
                self.ed.grab_follow(at, self.mouse.pos);
                crate::logging::busy("");
                let ms = t.elapsed().as_millis();
                if ms > 50 {
                    log::warn!("the grab took {ms} ms");
                }
            }
        }
        if let (Some((pieces, at)), Some(assets), Some(roots)) = (self.ed.lane_detach_request.take(), self.assets.as_mut(), &self.roots) {
            self.ed.set_selection(pieces.into_iter().map(|(k, id)| Sel::Spline(k, id)).collect());
            self.ed.detach_lanes(assets, roots);
            self.ed.pick_ai_path_near(assets, at);
        }
        if let Some(assets) = self.assets.as_mut() {
            self.ed.tick(assets, dt, self.mouse.left && !self.mouse.over_ui);
            if let Some(roots) = &self.roots {
                self.ed.apply_scales(roots);
            }
        }
        self.mesh_frame();

        let Some(mut g) = self.gfx.take() else { return };
        g.renderer.profile.add("editor", crate::render::profile::ms(t_ed));
        let t_ui = Instant::now();
        // the interface
        let raw = g.egui_state.take_egui_input(&g.window);
        let mut actions: Vec<Action> = Vec::new();
        let mut wanted_thumbs: Vec<(String, bool)> = Vec::new();
        let mut wanted_swatches: Vec<String> = Vec::new();
        let loading = self.assets.as_ref().map(|a| a.pending).unwrap_or(0);
        let mut view: Option<(egui::Rect, bool)> = None;
        let gpu_name = g.gpu.adapter_name.clone();
        let mut recover_answer = None;
        let mut restart_now = false;
        // (the selected object's PBR state for its properties: looked up when it changes)
        let sel_file = match (self.ed.selected, &self.ed.doc) {
            (Some(Sel::Object(k, id)), Some(doc)) => crate::selection::any_object(doc, k, id).map(|o| (o.file.clone(), false)),
            (Some(Sel::Spline(k, id)), Some(doc)) => doc.spline(k, id).map(|s| (s.file.clone(), true)),
            (Some(Sel::Attached(k, id)), Some(doc)) => doc.attachment(k, id).map(|a| (a.file.clone(), false)),
            _ => None,
        };
        if let (Some((f, spline)), Some(a), Some(r)) = (sel_file, self.assets.as_mut(), &self.roots) {
            if self.ui.pbr_made.as_ref().map(|(rel, _)| rel != &f).unwrap_or(true) {
                let textures = if spline { spline_textures(a, &f) } else { object_textures(a, &f) };
                // (not loaded yet: asked again next frame)
                if !textures.is_empty() {
                    let made = textures.iter().any(|t| crate::ui_pbr::is_made(t, r));
                    self.ui.pbr_made = Some((f.clone(), made));
                }
                // (the model's size and textures for the properties)
                self.ui.obj_info = if spline { None } else { a.object(&f) }.map(|m| crate::ui::ObjInfo {
                    file: f.clone(),
                    size: (m.hi - m.lo).to_array(),
                    triangles: m.parts.iter().map(|p| p.mesh.indices.len() / 3).sum(),
                    textures: textures.iter().map(|t| (t.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), crate::ui_pbr::is_made(t, r) || !omsi_texture::pbr::find(t).is_empty())).collect(),
                    pieces: m
                        .parts
                        .iter()
                        .map(|p| crate::ui::PieceInfo {
                            triangles: p.mesh.indices.len() / 3,
                            materials: {
                                let mut seen: HashMap<String, i32> = HashMap::new();
                                p.materials
                                    .iter()
                                    .map(|mat| {
                                        let name = mat.texture.as_ref().and_then(|t| t.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                        let n = seen.entry(name.to_lowercase()).or_insert(-1);
                                        *n += 1;
                                        (name, *n, match mat.alpha {
                                            crate::assets::Alpha::Opaque => 0,
                                            crate::assets::Alpha::Test => 1,
                                            crate::assets::Alpha::Blend => 2,
                                        })
                                    })
                                    .collect()
                            },
                        })
                        .collect(),
                    labels: m.labels.clone(),
                    paths: m.paths.len(),
                    attach_points: m.attachments.len(),
                    lights: m.light_names(),
                    light_head: m.light_head,
                });
            }
        }
        // (the program of the object the selected one takes its state from)
        self.ui.parent_lights = match (self.ed.selected, &self.ed.doc, self.assets.as_mut()) {
            (Some(Sel::Object(k, id)), Some(doc), Some(a)) => crate::selection::any_object(doc, k, id)
                .and_then(|o| o.var_parent)
                .and_then(|p| crate::selection::object_by_id(doc, p))
                .and_then(|(_, p)| a.object(&p.file).map(|m| (p.file.clone(), m.light_names()))),
            _ => None,
        };
        // (the overview map stays out of the map's picture)
        let minimap_open = self.ui.minimap.open;
        if self.snapshot {
            self.ui.minimap.open = false;
        }
        let mut guide: Vec<crate::ui_guide::Go> = Vec::new();
        self.geo.fill_bridge(&mut self.ui.geo, &self.ed);
        self.ui.minimap.imagery = self.geo.minimap_tiles();
        g.renderer.profile.add("ui.before", crate::render::profile::ms(t_ui));
        let full = g.egui_ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            if self.ed.doc.is_none() {
                ui::welcome(ui, &mut self.ui, &self.prefs, self.roots.as_ref().map(|r| r.omsi.as_path()), &mut actions);
            } else {
                let can_undo = self.ed.history.can_undo();
                let can_redo = self.ed.history.can_redo();
                let cursor = self.ed.cursor;
                view = Some(ui::editor(
                    ui,
                    &mut self.ui,
                    ui::EditorUi {
                        ed: &mut self.ed,
                        lib: &self.lib,
                        thumbs: &g.renderer.thumbs,
                        wanted: &mut wanted_thumbs,
                        swatches: &self.swatches,
                        wanted_swatches: &mut wanted_swatches,
                        can_undo,
                        can_redo,
                        game_found: self.game.is_some(),
                        texture_files: &self.texture_files,
                        prefs: &self.prefs,
                        status: ui::Status { cursor, loading, gpu: gpu_name.clone(), cam: Some(crate::ui_minimap::CamView { yaw: self.cam.yaw, distance: self.cam.distance, eye: self.cam.wanted_eye().truncate() }) },
                    },
                    &mut actions,
                ));
            }
            // (the pictures taken of the view are clean of them)
            if !self.snapshot {
                crate::ui_edit::paint_names(&ctx, &self.ed, &self.lib, &self.cam, self.view);
                if let (Some(sel), Some(doc), Some(a)) = (self.ed.path_numbers.filter(|s| self.ed.selected == Some(*s)), self.ed.doc.as_ref(), self.assets.as_mut()) {
                    crate::ui_edit::paint_path_numbers(&ctx, &crate::lanes::of_one(doc, a, sel), &self.cam, self.view);
                }
            }
            crate::ui_edit::paint_box(&ctx, &self.ed);
            crate::ui_edit::paint_ruler(&ctx, &self.ed, &self.cam, self.view);
            crate::ui_edit::paint_cut(&ctx, &self.ed, &self.cam, self.view);
            crate::collab::presence::paint(&ctx, &self.collab, &self.ed, &self.cam, self.view);
            crate::ui_menu::show(&ctx, &mut self.ui.view_menu, &mut self.ed, &self.lib, &mut actions);
            crate::ui_edit::road_entry(&ctx, &mut self.ed, self.view, &mut actions);
            if self.ed.doc.is_some() {
                if let Some(doc) = self.ed.doc.as_ref() {
                    if let Some(req) = self.drive.window(&ctx, doc, self.roots.as_ref(), &mut self.prefs.drive) {
                        actions.push(Action::DriveStart(req));
                    }
                }
                self.tt.window(&ctx, &mut self.ed, self.roots.as_ref(), &mut actions);
                self.traffic.window(&ctx, &mut self.ed, self.roots.as_ref());
                self.pbr.window(&ctx, self.roots.as_ref(), &mut actions);
                self.road_types.window(&ctx, &mut self.ed, &self.lib, self.roots.as_ref(), &mut actions);
                self.cj.window(&ctx, &mut self.ed, &self.lib, self.roots.as_ref().map(|r| &r.content), self.assets.as_mut(), &mut actions);
                let folder = self.map_folder();
                self.scripts.window(&ctx, &mut self.ed, self.roots.as_ref(), &folder, &mut actions);
                self.map_settings.window(&ctx, &mut self.ed, self.roots.as_ref());
                self.calendar.window(&ctx, &mut self.ed);
                self.chrono.window(&ctx, &mut self.ed, &self.lib, self.roots.as_ref(), &mut actions);
                self.signals.window(&ctx, &mut self.ed, &self.lib, self.roots.as_ref(), &mut actions);
                self.geo.window(&ctx, &mut self.ed, &mut self.prefs);
                self.ui.views.follow(self.ed.doc.as_ref().map(|d| d.dir.as_path()));
                self.ui.views.window(&ctx, &mut self.cam);
                self.osm.window(&ctx, &mut self.ed, &self.lib, self.geo.georef());
                self.trace.window(&ctx, &mut self.ed, &self.lib, self.geo.georef());
                // (within 12 m of a side with no tile beyond, or 2 m of a road's dead end: the
                // game samples a lane's last point on the very edge of its road)
                let at_edge = |p: DVec2| {
                    let Some(d) = self.ed.doc.as_ref() else { return false };
                    [DVec2::X, -DVec2::X, DVec2::Y, -DVec2::Y].iter().any(|v| !d.tiles.contains_key(&crate::doc::tile_of(p + *v * 12.0)))
                        || d.tiles.iter().any(|(k, t)| {
                            t.tile.splines.iter().filter(|s| !s.deleted && (s.prev_id == 0 || s.next_id == 0)).any(|s| {
                                let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(*k));
                                (s.prev_id == 0 && (c.point_at(0.0).truncate() - p).length() < 2.0) || (s.next_id == 0 && (c.end_point().truncate() - p).length() < 2.0)
                            })
                        })
                };
                if self.gamecheck.window(&ctx, &at_edge) {
                    actions.push(Action::GameCheck);
                }
                if let Some(p) = self.gamecheck.look.take() {
                    actions.push(Action::LookAt(p));
                }
                if let Some(p) = self.ed.look_request.take() {
                    actions.push(Action::LookAt(p));
                }
                if let Some(p) = self.geo.look.take() {
                    actions.push(Action::LookAt(p));
                }
                guide = self.guide.window(&ctx, &self.ed, &self.lib, &self.prefs, self.game.is_some(), self.ui.check.errors_left() == Some(0));
            }
            crate::ui_uv::window(&ctx, &mut self.ui.uv, &mut self.ui.uv_open, &mut self.ed, self.roots.as_ref());
            crate::ui_collab::window(&ctx, &mut self.collab, &mut self.ed);
            crate::ui_newmap::dialog(&ctx, &mut self.ui.new_map, self.roots.as_ref(), &self.texture_files, &self.swatches, &mut wanted_swatches, &mut actions);
            ui::confirm_unsaved(&ctx, &mut self.ui, &mut actions);
            if let Some(m) = &self.recover {
                recover_answer = crate::doc::autosave::offer(&ctx, m);
            }
            let unsaved = self.ed.doc.as_ref().map(|d| d.is_dirty()).unwrap_or(false);
            restart_now = self.updater.frame(&ctx, &mut self.prefs, unsaved) && self.recover.is_none();
        });
        match view {
            Some((rect, hovered)) => {
                self.view = rect;
                self.mouse.over_ui = !hovered || g.egui_ctx.egui_is_using_pointer();
            }
            None => {
                self.view = egui::Rect::NOTHING;
                self.mouse.over_ui = true;
            }
        }
        self.geo.take_bridge(&mut self.ui.geo, &mut self.ed, &self.prefs);
        // a GPS track built into a road: its runs over the map's tiles, as a street of the type chosen
        if let (Some((pts, rt)), Some(assets), Some(doc)) = (self.trace.build.take(), self.assets.as_mut(), self.ed.doc.as_ref()) {
            let mut runs: Vec<Vec<DVec2>> = vec![Vec::new()];
            for p in pts {
                if doc.tiles.contains_key(&crate::doc::tile_of(p)) {
                    runs.last_mut().unwrap().push(p);
                } else if !runs.last().unwrap().is_empty() {
                    runs.push(Vec::new());
                }
            }
            let roads: Vec<crate::osm::Road> = runs.into_iter().filter(|r| r.len() >= 2).map(|r| crate::osm::Road { class: crate::osm::Class::Street, oneway: false, points: crate::osm::simplify(&r) }).collect();
            if roads.is_empty() {
                self.ed.error(tr("The track does not run over the map's tiles", "Трек не проходит по тайлам карты"));
            } else {
                let mark = self.ed.history.mark();
                let (n, j) = self.ed.build_osm(assets, &roads, &[(crate::osm::Class::Street, rt)], None);
                self.ed.history.merge_since(mark, tr("Road along a GPS track", "Дорога по GPS-треку"));
                self.ed.toast(format!("{} {n} · {} {j}", tr("Roads built:", "Дорог построено:"), tr("junctions:", "перекрёстков:")));
            }
        }
        if let (Some(b), Some(assets)) = (self.osm.build.take(), self.assets.as_mut()) {
            let mark = self.ed.history.mark();
            let (roads, junctions) = self.ed.build_osm(assets, &b.roads, &b.types, b.oneway_type.as_deref());
            if let (Some((areas, islands, depth)), Some(doc)) = (&b.water, self.ed.doc.as_mut()) {
                crate::osm_water::apply(doc, &mut self.ed.history, areas, islands, *depth);
            }
            let stops = b.stop_type.map(|t| self.ed.place_osm_stops(assets, &b.stops, &t)).unwrap_or(0);
            let mut text = format!("{} {roads} · {} {junctions} · {} {stops}", tr("Roads built:", "Дорог построено:"), tr("junctions:", "перекрёстков:"), tr("stops:", "остановок:"));
            if let (false, Some(doc), Some(roots)) = (b.lines.is_empty(), self.ed.doc.as_mut(), &self.roots) {
                let before = crate::history::Snapshot { timetable: Some(doc.timetable.clone()), traffic: Some(doc.traffic.clone()), ..Default::default() };
                let added = crate::osm_lines::add(doc, vec![roots.content.clone(), roots.omsi.clone()], &b.lines);
                if added.trips > 0 {
                    self.ed.history.push_snapshot(tr("Lines from OpenStreetMap", "Линии из OpenStreetMap"), before);
                }
                text += &format!(" · {} {} · {} {}/{}", tr("lines:", "линий:"), added.lines, tr("trips found their way:", "рейсов с путём:"), added.routed, added.trips);
            }
            if !b.buildings.is_empty() {
                let n = self.ed.place_osm_buildings(assets, &b.buildings, &b.pool);
                text += &format!(" · {} {n}/{}", tr("buildings:", "зданий:"), b.buildings.len());
            }
            if !b.trees.is_empty() {
                let n = self.ed.plant_osm(&b.trees, &b.mix);
                text += &format!(" · {} {n}", tr("trees:", "деревьев:"));
            }
            // a start point where there is none, so the town can be driven at once
            if let (Some(doc), Some(roots)) = (self.ed.doc.as_mut(), &self.roots) {
                if doc.global.entry_points.is_empty() && roads > 0 {
                    if let Some((at, heading)) = crate::sample::start_spot(doc, roots) {
                        let k = crate::doc::tile_of(at);
                        self.ed.history.begin(tr("Start point", "Точка старта"));
                        self.ed.history.touch_tile(doc, k);
                        self.ed.history.touch_global(doc);
                        crate::sample::add_start(doc, at, heading, tr("Start", "Старт"));
                        self.ed.history.commit();
                        text += &format!(" · {}", tr("a start point", "точка старта"));
                    }
                }
            }
            // (all of it one step: undone at once)
            self.ed.history.merge_since(mark, tr("From OpenStreetMap", "Из OpenStreetMap"));
            log::info!("osm: {text}");
            self.ed.toast(text);
        }
        g.egui_state.handle_platform_output(&g.window, full.platform_output.clone());
        self.load_swatches(&g.egui_ctx, wanted_swatches);

        // the interface's textures go up whether or not this frame is drawn (a font atlas
        // lost with a skipped frame is never sent again)
        for (id, delta) in &full.textures_delta.set {
            g.egui_renderer.update_texture(&g.gpu.device, &g.gpu.queue, *id, delta);
        }
        g.renderer.profile.add("ui", crate::render::profile::ms(t_ui));
        let t_acquire = Instant::now();
        // draw
        let frame = match g.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => Some(f),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                let s = g.window.inner_size();
                g.gpu.resize(s.width, s.height);
                None
            }
            _ => None,
        };
        g.renderer.profile.add("acquire", crate::render::profile::ms(t_acquire));
        if let Some(frame) = frame {
            let view = frame.texture.create_view(&Default::default());
            let scene_view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(g.gpu.scene_format), ..Default::default() });
            let mut enc = g.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
            // (the lamps' programs run on the editor's own seconds, so the testing speed
            // shortens their cycle too: the phase still follows the program's own time)
            g.renderer.time = crate::render::daylight::seconds_after(g.renderer.time, dt as f32, self.ui.time_speed);
            if let (Some(doc), Some(assets)) = (&self.ed.doc, self.assets.as_mut()) {
                let t_x = Instant::now();
                let mut extras = self.ed.extras(&self.cam, assets);
                self.tt.overlay(&self.ed, &self.cam, &mut extras);
                g.renderer.profile.add("extras", crate::render::profile::ms(t_x));
                let t_geo = Instant::now();
                self.geo.feed(&mut g.renderer.imagery, &g.gpu, &self.ed, DVec2::new(self.cam.target.x, self.cam.target.y));
                g.renderer.profile.add("geo.feed", crate::render::profile::ms(t_geo));
                g.renderer.daytime = self.ui.daytime.map(|h| crate::render::daylight::at_map(h, &doc.traffic.timezone));
                if let Some((set, stamp)) = stream_window(doc, self.prefs.tile_streaming == Some(true), stream_count(self.prefs.stream_tiles), &mut self.stream_at, &self.cam, self.stream_set.as_ref().map(|s| s.1)) {
                    g.renderer.set_live(set.clone());
                    self.stream_set = Some((set, stamp));
                }
                g.renderer.sync(&g.gpu, doc, assets, &extras, &self.cam);
                let ppp = full.pixels_per_point;
                let (w, h) = (g.gpu.config.width as f32, g.gpu.config.height as f32);
                let v = self.view;
                let x0 = (v.min.x * ppp).clamp(0.0, w - 1.0);
                let y0 = (v.min.y * ppp).clamp(0.0, h - 1.0);
                let vp = [x0, y0, (v.width() * ppp).min(w - x0).max(1.0), (v.height() * ppp).min(h - y0).max(1.0)];
                g.renderer.draw(&g.gpu, &mut enc, &scene_view, &self.cam, &extras, vp);
                if crate::render::profile::gpu() {
                    g.renderer.time_gpu(&g.gpu, &self.cam, &extras, vp);
                }
            } else {
                // the start screen: a calm backdrop
                let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.035, g: 0.04, b: 0.05, a: 1.0 }), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            if let Some(assets) = self.assets.as_mut() {
                wanted_thumbs.dedup();
                g.renderer.render_thumbs(&g.gpu, &mut g.egui_renderer, assets, &wanted_thumbs, 4);
            }
            // the interface over it
            let ppp = full.pixels_per_point;
            let t_tess = Instant::now();
            let prims = g.egui_ctx.tessellate(full.shapes, ppp);
            g.renderer.profile.add("egui tessellate", crate::render::profile::ms(t_tess));
            let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [g.gpu.config.width, g.gpu.config.height], pixels_per_point: ppp };
            let t_up = Instant::now();
            let extra = g.egui_renderer.update_buffers(&g.gpu.device, &g.gpu.queue, &mut enc, &prims, &sd);
            g.renderer.profile.add("egui buffers", crate::render::profile::ms(t_up));
            {
                let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let mut pass = pass.forget_lifetime();
                g.egui_renderer.render(&mut pass, &prims, &sd);
            }
            let t_submit = Instant::now();
            g.gpu.queue.submit(extra.into_iter().chain([enc.finish()]));
            // (freed only after the frame that may still draw them is submitted: a texture
            // dropped by the interface in a frame it was drawn in is destroyed, and a draw
            // of a destroyed texture is a fatal wgpu validation error)
            for id in &full.textures_delta.free {
                g.egui_renderer.free_texture(id);
            }
            g.renderer.profile.add("submit", crate::render::profile::ms(t_submit));

            g.renderer.profile.add("frame cpu", crate::render::profile::ms(now));
            g.renderer.profile.add("frame interval", dt as f32 * 1000.0);
            g.renderer.profile.end_frame();
            self.frames += 1;
            crate::logging::beat();
            // (the start is behind: a wait now is not "first frames")
            if self.frames == 3 {
                crate::logging::busy("");
            }
            // OPENMAPPER_SAVE=1: what the checks above did saved at the third frame (a larger
            // number: at that frame, after what is built a few frames in)
            let save_at = std::env::var("OPENMAPPER_SAVE").ok().map(|v| v.parse::<u64>().ok().filter(|n| *n > 1).unwrap_or(3));
            if save_at == Some(self.frames as u64) {
                self.save();
            }
            // OPENMAPPER_PRESS=G:Y:2:Enter: those keys pressed one a frame, from
            // OPENMAPPER_PRESS_AT (the frame, 30 unless given), what it came to logged: the
            // keys checked with no one at the screen
            if let Ok(s) = std::env::var("OPENMAPPER_PRESS") {
                let at = std::env::var("OPENMAPPER_PRESS_AT").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(30);
                let n = self.frames.wrapping_sub(at);
                if self.frames >= at && n % 2 == 0 {
                    if let Some(code) = s.split(':').filter(|x| !x.is_empty()).nth((n / 2) as usize).map(|x| if x.eq_ignore_ascii_case("Escape") || x.eq_ignore_ascii_case("Esc") { Some(KeyCode::Escape) } else { crate::controls::code_of(x) }).unwrap_or(None) {
                        let t = std::time::Instant::now();
                        let key = Key::Named(NamedKey::Space);
                        self.key(Some(code), &key, true, el);
                        self.key(Some(code), &key, false, el);
                        let ms = t.elapsed().as_millis();
                        let grab = self.ed.grab.as_ref().map(|g| (g.axis_name(), g.delta.to_array()));
                        let sel = self.ed.doc.as_ref().and_then(|d| crate::selection::origin_of(d, self.ed.selected?)).map(|o| o.pos.to_array());
                        log::info!("press {} in {ms} ms: grab {grab:?}, the selection at {sel:?}, readout {:?}", crate::controls::code_name(code), self.ed.drag_readout());
                    }
                }
            }
            // OPENMAPPER_DRAG=dx,dy: with a grab going, the pointer walked that many points a
            // frame and the cursor with it (a drag with no one at the screen: what a grab of
            // a big selection costs is seen in the frame times it logs)
            if let Ok(d) = std::env::var("OPENMAPPER_DRAG").map(|c| c.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).map_err(|_| ()).and_then(|v| if v.len() == 2 { Ok(v) } else { Err(()) }) {
                if self.ed.grab.is_some() {
                    self.mouse.pos.0 += d[0];
                    self.mouse.pos.1 += d[1];
                    self.mouse.moved = true;
                    // (no pointer of a person here: the cursor is walked over the map by the
                    // few centimetres a frame the points stand for)
                    if let Some(c) = self.ed.cursor {
                        self.ed.cursor = Some(c + DVec3::new(d[0] as f64 * 0.02, d[1] as f64 * 0.02, 0.0));
                    }
                }
            }
            // OPENMAPPER_SELECT_ALL=o|s|*: every object (o), every road (s) or both selected at
            // the second frame (a check of a move of a whole map, with no one at the screen)
            if self.frames == 2 {
                if let Ok(what) = std::env::var("OPENMAPPER_SELECT_ALL") {
                    if let Some(doc) = self.ed.doc.as_ref() {
                        let mut v = Vec::new();
                        for (k, t) in &doc.tiles {
                            if what.contains('o') || what == "*" {
                                v.extend(t.tile.objects.iter().map(|o| Sel::Object(*k, o.id)));
                            }
                            if what.contains('s') || what == "*" {
                                v.extend(t.tile.splines.iter().map(|s| Sel::Spline(*k, s.id)));
                            }
                        }
                        log::info!("select all: {} things", v.len());
                        self.ed.set_selection(v);
                    }
                }
            }
            if self.frames == 3 {
                if let Ok(d) = std::env::var("OPENMAPPER_DEMO") {
                    if d == "T" {
                        self.demo_t();
                    } else if d.starts_with("SAMPLE:") {
                        // OPENMAPPER_DEMO=SAMPLE:<name>: a new map with the sample, saved
                        let name = d.trim_start_matches("SAMPLE:").to_string();
                        while self.lib.scanning {
                            self.lib.poll();
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        self.new_map(&name, &name, 3, &crate::ui_newmap::NewMapOptions { left_hand: std::env::var_os("OPENMAPPER_LHT").is_some(), seasons: crate::ui_newmap::Seasons::Summer, ..Default::default() });
                        self.sample();
                        // OPENMAPPER_POCKET=<dx>,<dy>[,<pocket width>]: a bus pocket pressed that far from
                        // the map's middle (a bay of 3 m or the width)
                        if let Ok(p) = std::env::var("OPENMAPPER_POCKET") {
                            self.demo_pocket(&p);
                        }
                        self.save();
                    } else if d == "RING" {
                        self.demo_ring();
                    } else if d == "JOIN" {
                        self.demo_join();
                    } else {
                        self.demo(&d);
                        // OPENMAPPER_TIMETABLE with the demo: stops and a line on its roads
                        if std::env::var_os("OPENMAPPER_TIMETABLE").is_some() {
                            self.tt.demo(&mut self.ed, self.roots.as_ref());
                            self.save();
                        }
                    }
                }
            }
            if self.frames == 4 && std::env::var_os("OPENMAPPER_NOGUIDE").is_some() {
                self.guide.open = false;
            }
            if self.snapshot {
                self.snapshot = false;
                self.ui.minimap.open = minimap_open;
                let ppp = full.pixels_per_point;
                let v = self.view;
                let crop = [(v.min.x * ppp) as u32, (v.min.y * ppp) as u32, (v.width() * ppp) as u32, (v.height() * ppp) as u32];
                let name = self.ed.doc.as_ref().map(|d| d.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).unwrap_or_default();
                let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Pictures").join("openMapper");
                let path = dir.join(format!("{name} {}.png", crate::doc::safe::stamp()));
                let done = read_frame(&g.gpu, &frame.texture).and_then(|img| {
                    let (w, h) = (crop[2].min(img.width().saturating_sub(crop[0])), crop[3].min(img.height().saturating_sub(crop[1])));
                    anyhow::ensure!(w > 8 && h > 8, "the view is too small");
                    std::fs::create_dir_all(&dir)?;
                    image::DynamicImage::ImageRgba8(image::imageops::crop_imm(&img, crop[0], crop[1], w, h).to_image()).to_rgb8().save(&path)?;
                    Ok(())
                });
                match done {
                    Ok(()) => self.ed.toast(format!("{} {}", tr("Picture saved:", "Картинка сохранена:"), path.display())),
                    Err(e) => self.ed.error(format!("{}: {e}", tr("Could not save the picture", "Не удалось сохранить картинку"))),
                }
            }
            if let Some((path, at)) = &self.shot {
                // (a running imagery load is waited for: it ends by itself)
                let ready = !self.geo.busy() && (self.frames >= *at && self.assets.as_ref().map(|a| a.pending == 0).unwrap_or(true) || self.frames >= at * 6);
                // OPENMAPPER_SNAPSHOT=1: View > Save a picture of the view taken first (checks)
                if ready && std::env::var_os("OPENMAPPER_SNAPSHOT").is_some() && !self.snapshot_done {
                    self.snapshot_done = true;
                    self.snapshot = true;
                } else if ready {
                    match save_shot(&g.gpu, &frame.texture, path) {
                        Ok(()) => log::info!("screenshot: {}", path.display()),
                        Err(e) => log::error!("screenshot: {e}"),
                    }
                    self.shot = None;
                    el.exit();
                }
            }
            g.window.pre_present_notify();
            frame.present();
        }
        let title = match &self.ed.doc {
            Some(d) => format!("{}{} - openMapper", if d.is_dirty() { "• " } else { "" }, d.display_name()),
            None => "openMapper".to_string(),
        };
        g.window.set_title(&title);
        self.gfx = Some(g);
        for go in guide {
            use crate::ui_guide::Go;
            match go {
                Go::Tool(t) => self.ed.set_tool(t),
                Go::Logic(m) => {
                    self.ed.set_tool(Tool::Logic);
                    self.ed.set_logic(m);
                }
                Go::Lines => self.tt.open = true,
                Go::Traffic => self.traffic.open = true,
                Go::Save => actions.push(Action::Save),
                Go::Check => {
                    self.ui.check.open = true;
                    self.ui.check.issues = None;
                }
                Go::Play => actions.push(Action::Play),
                Go::LookAt(p) => actions.push(Action::LookAt(p)),
                Go::Auto(on) => {
                    self.prefs.guide_off = !on;
                    self.prefs.save();
                }
            }
        }
        if restart_now {
            self.remember_view();
            crate::updater::restart();
        }
        if let Some(yes) = recover_answer {
            if let Some(meta) = self.recover.take() {
                self.recover_autosave(yes, &meta);
            }
        }
        for a in actions {
            self.do_action(a, el);
        }
        // the changes kept every few minutes (not in the middle of an edit, nor while an
        // earlier session's autosave is still offered: there is one, it would be written over)
        let editing = self.mouse.left || self.ed.history.is_open() || self.recover.is_some() || self.chrono.in_session();
        self.autosave.tick(self.ed.doc.as_ref(), self.roots.as_ref().map(|r| r.content.as_path()), editing);
    }

    /// A scripted session (`OPENMAPPER_DEMO=<map name>`): a new map with roads, objects and
    /// paint, saved; shows that every tool works without a hand on the mouse.
    fn demo(&mut self, name: &str) {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.wait_textures();
        self.new_map(name, name, 3, &Default::default());
        let Some(assets) = self.assets.as_mut() else { return };
        let road = self.lib.find("Splines\\Marcel\\str_2spur_8m_borkumer1.sli").map(|a| a.rel.clone()).or_else(|| self.lib.assets.iter().find(|a| a.category == crate::library::Category::Roads).map(|a| a.rel.clone()));
        self.ed.set_tool(Tool::Roads);
        self.ed.road_type = road;
        // a straight, then a bend, then a branch from the first road's start
        let click = |ed: &mut Editor, assets: &mut Assets, lib: &Library, x: f64, y: f64| {
            let z = ed.doc.as_ref().map(|d| d.ground(DVec2::new(x, y))).unwrap_or(0.0);
            ed.hover_point(assets, lib, DVec3::new(x, y, z));
            ed.left_press(assets, lib, false);
            ed.tick(assets, 0.0, false);
        };
        for (x, y) in [(-200.0, 20.0), (0.0, 20.0), (160.0, 120.0), (200.0, 300.0)] {
            click(&mut self.ed, assets, &self.lib, x, y);
            // (the spline type must be loaded to know the road's width)
            let _ = assets.spline(self.ed.road_type.as_deref().unwrap_or(""));
        }
        self.ed.right_click();
        // a branch that runs into the middle of the first road (it is cut there)
        while self.ed.road_type.as_deref().map(|r| assets.spline(r).is_none()).unwrap_or(false) {
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let before: usize = self.ed.doc.as_ref().map(|d| d.tiles.values().map(|t| t.tile.splines.len()).sum()).unwrap_or(0);
        click(&mut self.ed, assets, &self.lib, -100.0, -90.0);
        click(&mut self.ed, assets, &self.lib, -100.0, 18.0);
        let after: usize = self.ed.doc.as_ref().map(|d| d.tiles.values().map(|t| t.tile.splines.len()).sum()).unwrap_or(0);
        log::info!("demo: branch into the middle of a road: {before} splines before, {after} after, still drawing: {}", self.ed.road_start_active());
        self.ed.right_click();
        // a road straight across the bend (an X junction)
        click(&mut self.ed, assets, &self.lib, 80.0, -120.0);
        click(&mut self.ed, assets, &self.lib, 80.0, 200.0);
        self.ed.right_click();
        if let Some(d) = &self.ed.doc {
            let j: Vec<String> = d.tiles.values().flat_map(|t| t.tile.objects.iter()).filter(|o| o.file.contains("junction_")).map(|o| o.file.clone()).collect();
            log::info!("demo: {} junction objects: {:?}", j.len(), j);
            let n: usize = d.tiles.values().map(|t| t.tile.splines.len()).sum();
            log::info!("demo: {n} splines; messages: {:?}", self.ed.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>());
        }
        // terrain: a hill
        self.ed.set_tool(Tool::Terrain);
        self.ed.brush_radius = 60.0;
        self.ed.terrain_mode = crate::editor::TerrainMode::Raise;
        self.ed.hover_point(assets, &self.lib, DVec3::new(-150.0, 200.0, 0.0));
        self.ed.left_press(assets, &self.lib, false);
        for _ in 0..60 {
            self.ed.tick(assets, 0.05, true);
        }
        self.ed.tick(assets, 0.0, false);
        // objects: houses along the road
        let house = self.lib.assets.iter().find(|a| a.category == crate::library::Category::Objects && a.dir.to_lowercase().contains("build")).map(|a| a.rel.clone());
        let tree = self.lib.assets.iter().find(|a| a.category == crate::library::Category::Trees).map(|a| a.rel.clone());
        let stop = self.lib.assets.iter().find(|a| a.is_bus_stop).map(|a| a.rel.clone());
        let start = self.lib.assets.iter().find(|a| a.is_entry_point).map(|a| a.rel.clone());
        self.ed.set_tool(Tool::Objects);
        for (rel, pts) in [(house, vec![(-150.0, 45.0), (-100.0, 45.0), (-50.0, 45.0)]), (tree, vec![(-120.0, -20.0), (-90.0, -30.0), (-60.0, -15.0), (40.0, -40.0)]), (stop, vec![(-20.0, 27.0)]), (start, vec![(-180.0, 18.0)])] {
            self.ed.object_type = rel;
            for (x, y) in pts {
                click(&mut self.ed, assets, &self.lib, x, y);
            }
        }
        // paint: a second ground texture
        if let Some(t) = self.texture_files.iter().find(|t| t.to_lowercase().contains("kopfstein") || t.to_lowercase().contains("sand")).cloned() {
            let el_free = Action::AddGroundTexture(t);
            if let Action::AddGroundTexture(rel) = el_free {
                if let Some(doc) = self.ed.doc.as_mut() {
                    doc.global.ground_textures.push(omsi_map::GroundTex { texture: rel, detail_texture: "Texture\\noise_low.bmp".into(), params: [8.0, 16.0, 4.0] });
                    doc.sync_mask_slots();
                }
                self.ed.paint_layer = 1;
            }
            self.ed.set_tool(Tool::Paint);
            self.ed.brush_radius = 35.0;
            self.ed.hover_point(assets, &self.lib, DVec3::new(60.0, -80.0, 0.0));
            self.ed.left_press(assets, &self.lib, false);
            for _ in 0..30 {
                self.ed.tick(assets, 0.05, true);
            }
            self.ed.tick(assets, 0.0, false);
        }
        self.ed.set_tool(Tool::Roads);
        self.save();
        self.cam.jump_to(DVec3::new(-30.0, 80.0, 0.0), 330.0);
        self.cam.yaw = 15.0;
        self.cam.pitch = 42.0;
    }

    /// OPENMAPPER_HILL=x,y,radius,seconds: a hill raised with the terrain brush (before the
    /// roads are drawn, or after them with OPENMAPPER_HILL_AFTER).
    fn demo_hill(&mut self) {
        let Some(assets) = self.assets.as_mut() else { return };
        if let Some(v) = std::env::var("OPENMAPPER_HILL").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 4) {
            self.ed.set_tool(Tool::Terrain);
            self.ed.brush_radius = v[2] as f32;
            self.ed.terrain_mode = crate::editor::TerrainMode::Raise;
            self.ed.hover_point(assets, &self.lib, DVec3::new(v[0], v[1], 0.0));
            self.ed.left_press(assets, &self.lib, false);
            for _ in 0..(v[3] / 0.05) as usize {
                self.ed.tick(assets, 0.05, true);
            }
            self.ed.tick(assets, 0.0, false);
        }
    }

    /// `OPENMAPPER_DEMO=T`: one short road and another one run into its middle.
    fn demo_t(&mut self) {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.new_map("T check", "T check", 1, &Default::default());
        if std::env::var_os("OPENMAPPER_HILL_AFTER").is_none() {
            self.demo_hill();
        }
        let Some(assets) = self.assets.as_mut() else { return };
        self.ed.set_tool(Tool::Roads);
        let want = std::env::var("OPENMAPPER_ROAD").unwrap_or_else(|_| "Splines\\Marcel\\str_2spur_8m_borkumer1.sli".into());
        self.ed.road_type = self.lib.find(&want).map(|a| a.rel.clone());
        while self.ed.road_type.as_deref().map(|r| assets.spline(r).is_none()).unwrap_or(false) {
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let click = |ed: &mut Editor, assets: &mut Assets, lib: &Library, x: f64, y: f64| {
            ed.hover_point(assets, lib, DVec3::new(x, y, 0.0));
            ed.left_press(assets, lib, false);
            ed.tick(assets, 0.0, false);
        };
        let main: Vec<(f64, f64)> = std::env::var("OPENMAPPER_MAIN").ok().map(|v| v.split(';').filter_map(|p| { let mut it = p.split(','); Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?)) }).collect()).unwrap_or(vec![(60.0, 150.0), (240.0, 150.0)]);
        let branch: Vec<(f64, f64)> = std::env::var("OPENMAPPER_BRANCH").ok().map(|v| v.split(';').filter_map(|p| { let mut it = p.split(','); Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?)) }).collect()).unwrap_or(vec![(150.0, 260.0), (150.0, 151.0)]);
        for (x, y) in main {
            click(&mut self.ed, assets, &self.lib, x, y);
        }
        self.ed.right_click();
        // OPENMAPPER_BRANCH_ROAD: the branch of another road type
        if let Some(b) = std::env::var("OPENMAPPER_BRANCH_ROAD").ok().and_then(|b| self.lib.find(&b).map(|a| a.rel.clone())) {
            while assets.spline(&b).is_none() && !assets.spline_failed(&b) {
                assets.poll();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            self.ed.road_type = Some(b);
            self.ed.inherit_type = false;
        }
        for (x, y) in branch {
            click(&mut self.ed, assets, &self.lib, x, y);
        }
        self.ed.right_click();
        if std::env::var_os("OPENMAPPER_HILL_AFTER").is_some() {
            self.demo_hill();
        }
        // OPENMAPPER_PATH=x,y;x,y...: an AI car path drawn with the Logic tool
        if let Ok(v) = std::env::var("OPENMAPPER_PATH") {
            let pts: Vec<(f64, f64)> = v.split(';').filter_map(|p| { let mut it = p.split(','); Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?)) }).collect();
            let Some(assets) = self.assets.as_mut() else { return };
            let file = "Splines\\Marcel\\invis_street.sli";
            while assets.spline(file).is_none() && !assets.spline_failed(file) {
                assets.poll();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            self.ed.set_tool(Tool::Logic);
            self.ed.set_logic(crate::editor::LogicMode::Paths);
            self.ed.set_path_type(file.to_string());
            for (x, y) in pts {
                self.ed.hover_point(assets, &self.lib, DVec3::new(x, y, 0.0));
                log::info!("path check: snap at ({x}, {y}): {:?}", self.ed.road_snap.map(|a| (a.pos.x, a.pos.y, a.heading)));
                self.ed.left_press(assets, &self.lib, false);
                self.ed.tick(assets, 0.0, false);
            }
            self.ed.right_click();
        }
        if let Some(d) = &self.ed.doc {
            for t in d.tiles.values() {
                for s in &t.tile.splines {
                    log::info!("T check: spline {} at ({:.1}, {:.1}) heading {:.0} length {:.1} prev {} next {}", s.id, s.pos[0], s.pos[1], s.heading, s.length, s.prev_id, s.next_id);
                }
                for o in &t.tile.objects {
                    log::info!("T check: object {}", o.file);
                }
            }
        }
        // OPENMAPPER_CAM=x,y,distance,yaw,pitch: where to look at the result
        if let Some(v) = std::env::var("OPENMAPPER_CAM").ok().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 5) {
            self.cam.jump_to(DVec3::new(v[0], v[1], 0.0), v[2]);
            self.cam.yaw = v[3];
            self.cam.pitch = v[4];
        }
    }

    /// `OPENMAPPER_DEMO=JOIN` (with a map on the command line): a road run into the middle
    /// of one of the map's own roads.
    /// `OPENMAPPER_DEMO=RING`: a new map with two roads crossing and a roundabout laid on
    /// their crossing (checks of the roundabout, also in openOMSI).
    fn demo_ring(&mut self) {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.new_map("ring", "ring", 3, &Default::default());
        let Some(assets) = self.assets.as_mut() else { return };
        self.ed.set_tool(Tool::Roads);
        self.ed.road_type = self.lib.find("Splines\\Marcel\\str_2spur_8m_borkumer1.sli").map(|a| a.rel.clone());
        while self.ed.road_type.as_deref().map(|r| assets.spline(r).is_none()).unwrap_or(false) {
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let click = |ed: &mut Editor, assets: &mut Assets, lib: &Library, x: f64, y: f64| {
            let z = ed.doc.as_ref().map(|d| d.ground(DVec2::new(x, y))).unwrap_or(0.0);
            ed.hover_point(assets, lib, DVec3::new(x, y, z));
            ed.left_press(assets, lib, false);
            ed.tick(assets, 0.0, false);
        };
        for (a, b) in [((-250.0, 10.0), (250.0, 10.0)), ((10.0, -250.0), (10.0, 250.0))] {
            click(&mut self.ed, assets, &self.lib, a.0, a.1);
            click(&mut self.ed, assets, &self.lib, b.0, b.1);
            self.ed.right_click();
        }
        self.ed.roundabout = true;
        click(&mut self.ed, assets, &self.lib, 10.0, 10.0);
        click(&mut self.ed, assets, &self.lib, 45.0, 10.0);
        self.ed.roundabout = false;
        let n: usize = self.ed.doc.as_ref().map(|d| d.tiles.values().map(|t| t.tile.splines.len()).sum()).unwrap_or(0);
        log::info!("demo: ring: {n} splines; messages: {:?}", self.ed.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>());
    }

    fn demo_join(&mut self) {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let Some(assets) = self.assets.as_mut() else { return };
        // (every spline type of the map loaded first)
        let files: Vec<String> = self.ed.doc.as_ref().map(|d| d.tiles.values().flat_map(|t| t.tile.splines.iter().map(|s| s.file.clone())).collect()).unwrap_or_default();
        for f in &files {
            let _ = assets.spline(f);
        }
        while assets.pending > 0 {
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for f in &files {
            let _ = assets.spline(f);
        }
        let Some(doc) = self.ed.doc.as_ref() else { return };
        // a street spline near the camera, long enough
        let cam = self.cam.target.truncate();
        let mut best: Option<(f64, DVec2, f64, String)> = None;
        for (k, t) in &doc.tiles {
            for sp in &t.tile.splines {
                if sp.length < 40.0 {
                    continue;
                }
                let Some(m) = assets.spline(&sp.file) else { continue };
                if !m.joinable() {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
                let p = c.point_at(sp.length / 2.0).truncate();
                let d = (p - cam).length();
                if best.as_ref().map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, p, c.heading_at(sp.length / 2.0), sp.file.clone()));
                }
            }
        }
        let Some((_, p, h, file)) = best else {
            log::warn!("join check: no road found");
            return;
        };
        log::info!("join check: target road {file} at ({:.1}, {:.1}) heading {h:.0}", p.x, p.y);
        self.ed.set_tool(Tool::Roads);
        let from = p + crate::roads::right(h) * 70.0;
        let click = |ed: &mut Editor, assets: &mut Assets, lib: &Library, q: DVec2| {
            let z = ed.doc.as_ref().map(|d| d.ground(q)).unwrap_or(0.0);
            ed.hover_point(assets, lib, DVec3::new(q.x, q.y, z));
            ed.left_press(assets, lib, false);
            ed.tick(assets, 0.0, false);
        };
        self.ed.road_type = Some(file.clone());
        if std::env::var_os("OPENMAPPER_JOIN_DRY").is_none() {
            click(&mut self.ed, assets, &self.lib, from);
            click(&mut self.ed, assets, &self.lib, p + crate::roads::right(h) * 1.0);
        }
        log::info!("join check: road type {:?}; messages {:?}", self.ed.road_type, self.ed.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>());
        self.cam.jump_to(DVec3::new(p.x, p.y, self.ed.doc.as_ref().map(|d| d.ground(p)).unwrap_or(0.0)), 70.0);
        self.cam.pitch = 35.0;
        self.cam.yaw = h + 90.0 + 30.0;
    }

    /// Small pictures of ground textures for the interface.
    fn load_swatches(&mut self, ctx: &egui::Context, wanted: Vec<String>) {
        let (Some(assets), Some(roots)) = (self.assets.as_mut(), self.roots.as_ref()) else { return };
        let map_dirs: Vec<PathBuf> = self.ed.doc.as_ref().map(|d| [Some(d.dir.clone()), d.source.clone()].into_iter().flatten().collect()).unwrap_or_default();
        for rel in wanted.into_iter().take(12) {
            let key = rel.to_lowercase();
            if self.swatches.contains_key(&key) {
                continue;
            }
            let path = map_dirs.iter().map(|d| omsi_cfg::resolve_path(d, &rel)).chain([roots.resolve(&rel)]).find(|p| omsi_cfg::vfs::is_file(p));
            let Some(path) = path else { continue };
            let Some(img) = assets.texture(&path) else { continue };
            let mut small = img.image.clone();
            while small.width > 96 {
                small = crate::assets::half(&small);
            }
            let ci = egui::ColorImage::from_rgba_unmultiplied([small.width as usize, small.height as usize], &small.rgba.chunks(4).flat_map(|c| [c[0], c[1], c[2], 255]).collect::<Vec<u8>>());
            let h = ctx.load_texture(format!("swatch:{key}"), ci, egui::TextureOptions::LINEAR);
            self.swatches.insert(key, h);
        }
    }
}

/// The textures a map's ground can be painted with: the installation's and the content
/// folder's `Texture` folders (pictures only).
fn ground_texture_files(roots: &Roots) -> Vec<String> {
    // the installation's ground textures first; then those of the objects and splines
    // (ballast, cobbles, slabs: maps paint with them too, Spandau with `Schotter.bmp`)
    let mut out: Vec<String> = Vec::new();
    let mut more: Vec<String> = Vec::new();
    let image = |p: &Path| {
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        let lower = p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        // (night maps, light maps, normal and PBR maps are not ground)
        matches!(ext.as_str(), "bmp" | "dds" | "jpg" | "png" | "tga") && !lower.contains("_night") && !lower.contains("_lm") && !lower.ends_with("_nn") && !lower.contains("_normal")
    };
    let add = |list: &mut Vec<String>, rel: String| {
        if !list.iter().any(|o| o.eq_ignore_ascii_case(&rel)) {
            list.push(rel);
        }
    };
    // every `texture` folder below a folder, as paths relative to the root
    fn texture_dirs(dir: &Path, rel: &str, out: &mut Vec<(PathBuf, String)>, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let r = format!("{rel}\\{name}");
            if name.eq_ignore_ascii_case("texture") {
                out.push((p, r));
            } else if depth < 6 {
                texture_dirs(&p, &r, out, depth + 1);
            }
        }
    }
    for r in roots.all() {
        let Ok(rd) = std::fs::read_dir(r) else { continue };
        for e in rd.flatten() {
            let top = e.file_name().to_string_lossy().to_string();
            if !e.path().is_dir() {
                continue;
            }
            let mut dirs = Vec::new();
            if top.eq_ignore_ascii_case("texture") {
                dirs.push((e.path(), "Texture".to_string()));
            } else if top.eq_ignore_ascii_case("sceneryobjects") || top.eq_ignore_ascii_case("splines") {
                texture_dirs(&e.path(), &top, &mut dirs, 0);
            } else {
                continue;
            }
            let main = top.eq_ignore_ascii_case("texture");
            for (dir, rel) in dirs {
                let Ok(files) = std::fs::read_dir(&dir) else { continue };
                for f in files.flatten() {
                    if image(&f.path()) {
                        let r = format!("{rel}\\{}", f.file_name().to_string_lossy());
                        if main { add(&mut out, r) } else { add(&mut more, r) }
                    }
                }
            }
        }
    }
    out.sort_by_key(|s| s.to_lowercase());
    more.sort_by_key(|s| s.to_lowercase());
    out.extend(more);
    out
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.init_gfx(el) {
                log::error!("could not open the window: {e:#}");
                let _ = rfd::MessageDialog::new().set_title("openMapper").set_description(format!("{}\n\n{e:#}", tr("The graphics could not be started.", "Не удалось запустить графику."))).set_level(rfd::MessageLevel::Error).show();
                el.exit();
            }
        }
        el.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(g) = self.gfx.as_mut() else { return };
        // (a screenshot run takes no input: keys typed into another window must not reach it)
        if self.shot.is_some() && matches!(event, WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } | WindowEvent::CursorMoved { .. } | WindowEvent::Ime(_)) {
            return;
        }
        if !matches!(event, WindowEvent::RedrawRequested | WindowEvent::Occluded(_)) {
            self.last_input = Instant::now();
        }
        let resp = g.egui_state.on_window_event(&g.window, &event);
        let scale = g.window.scale_factor() as f32;
        match event {
            WindowEvent::CloseRequested => {
                self.do_action(Action::Quit, el);
            }
            WindowEvent::Resized(s) => g.gpu.resize(s.width, s.height),
            WindowEvent::RedrawRequested => {
                self.frame(el);
            }
            WindowEvent::ModifiersChanged(m) => {
                self.keys.shift = m.state().shift_key();
                // (Shift while drawing a road: a straight)
                self.ed.straight_lock = self.keys.shift;
                self.ed.replan = true;
                self.keys.ctrl = m.state().control_key() || m.state().super_key();
                self.ed.ctrl_held = self.keys.ctrl;
                self.ed.shift_held = self.keys.shift;
                let alt = m.state().alt_key();
                // (Alt / Option goes down: the camera will orbit round the point under the cursor;
                // Unity orbits round the point looked at)
                if alt && !self.keys.alt && !self.mouse.over_ui && !self.fly_style() {
                    if let Some(c) = self.ed.cursor {
                        self.cam.set_pivot(c);
                    }
                }
                self.keys.alt = alt;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                let code = match event.physical_key {
                    PhysicalKey::Code(c) => Some(c),
                    _ => None,
                };
                let mut typing = g.egui_ctx.egui_wants_keyboard_input();
                // (Ctrl/Cmd + Z, Y: the map's undo even from a text field, which lets go of
                // the keyboard; its own text undo would only undo half of the edit)
                if typing && pressed && self.keys.ctrl && matches!(code, Some(KeyCode::KeyZ) | Some(KeyCode::KeyY)) {
                    g.egui_ctx.memory_mut(|m| m.stop_text_input());
                    typing = false;
                }
                if !typing || !pressed {
                    self.key(code, &event.logical_key, pressed, el);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x as f32 / scale, position.y as f32 / scale);
                let (dx, dy) = (p.0 - self.mouse.pos.0, p.1 - self.mouse.pos.1);
                self.mouse.pos = p;
                self.mouse.moved = true;
                if let Some(b) = &mut self.ed.select_box {
                    b[1] = p;
                }
                let h = self.view.height().max(1.0);
                let unity = self.fly_style();
                if self.mouse.right {
                    // (Unity: the eye looks round, Alt + right drag zooms; map style: the view
                    // turns round the point the button went down on, the map following the hand)
                    if self.ui.view_2d {
                        self.cam.pan_screen(dx, dy, h);
                    } else if self.prefs.omsi_controls {
                        // (OMSI 2's editor: held and moved down zooms in, up zooms out)
                        self.cam.zoom(dy * 0.04, self.ed.cursor);
                    } else if unity && self.keys.alt {
                        self.cam.dolly((dx - dy) * 0.02);
                    } else if !self.mouse.look_raw {
                        self.cam.look_pixels(dx, dy);
                    }
                    self.mouse.right_moved += dx.abs() + dy.abs();
                } else if self.mouse.alt_orbit {
                    self.cam.orbit_pixels(if unity { dx } else { -dx }, dy);
                }
                if self.mouse.middle {
                    if self.prefs.omsi_controls && !self.ui.view_2d {
                        // (OMSI 2's editor: the middle button turns the view)
                        self.cam.orbit_pixels(-dx, dy);
                    } else {
                        self.cam.pan_screen(dx, dy, h);
                    }
                } else if self.mouse.grab {
                    match (self.mouse.grab_anchor, self.view_ray()) {
                        (Some(a), Some((o, d))) => self.cam.grab(a, o, d),
                        _ => self.cam.pan_pixels(dx, dy, h),
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                // (the 3D view is an egui panel too, so egui calls every click its own: whether
                // the mouse is over the view is what counts)
                let _ = resp.consumed;
                let over_ui = self.mouse.over_ui;
                match button {
                    MouseButton::Left => {
                        if pressed && !over_ui && self.ed.grab.is_some() {
                            // (a click lets the grab go, as Blender's does)
                            self.ed.grab_confirm();
                        } else if pressed && !over_ui && self.prefs.tile_streaming == Some(true) && !self.keys.alt && self.stream_click() {
                            // (loaded another window of tiles)
                        } else if pressed && !over_ui {
                            // (a handle of the Move / Rotate / Scale gizmo is taken, wherever it is)
                            let on_gizmo = self.ed.gizmo_under_cursor();
                            if self.keys.alt {
                                // Option + drag: turn the camera
                                self.mouse.alt_orbit = true;
                            } else if self.ed.mesh.as_ref().map(|m| m.editing).unwrap_or(false) {
                                // (the edit mode: a press picks or takes a handle; on nothing it
                                // draws a box)
                                let picks = on_gizmo || self.ed.mesh.as_ref().map(|m| m.modal_active() || m.hover.is_some()).unwrap_or(false);
                                if picks {
                                    self.mouse.left = true;
                                    self.mouse.press = self.mouse.pos;
                                    if let Some(assets) = self.assets.as_mut() {
                                        self.ed.left_press(assets, &self.lib, self.keys.shift);
                                    }
                                } else {
                                    self.ed.select_box = Some([self.mouse.pos, self.mouse.pos]);
                                }
                            } else if on_gizmo {
                                self.mouse.left = true;
                                self.mouse.press = self.mouse.pos;
                                if let Some(assets) = self.assets.as_mut() {
                                    self.ed.left_press(assets, &self.lib, self.keys.shift);
                                }
                            } else if self.ed.tool == Tool::Select && self.ed.hover.is_none() && self.ed.ruler.is_none() && (self.keys.shift || self.keys.ctrl || self.fly_style()) {
                                // (Unity: a drag on nothing draws a selection box)
                                // Shift / Ctrl + drag on empty ground: a selection box
                                self.ed.select_box = Some([self.mouse.pos, self.mouse.pos]);
                            } else if self.ed.tool == Tool::Select && self.ed.hover.is_none() && self.ed.ruler.is_none() {
                                // drag the map by its empty ground
                                self.mouse.grab = true;
                                self.mouse.grab_anchor = self.grab_point();
                                self.ed.select(None);
                            } else {
                                self.mouse.left = true;
                                self.mouse.press = self.mouse.pos;
                                if let Some(assets) = self.assets.as_mut() {
                                    self.ed.left_press(assets, &self.lib, self.keys.shift);
                                }
                            }
                        } else if !pressed {
                            if self.ed.select_box.is_some() {
                                let v = self.view;
                                if let Some(assets) = self.assets.as_mut() {
                                    self.ed.finish_box(assets, &self.cam, (v.min.x, v.min.y, v.width(), v.height()), self.keys.shift);
                                }
                                self.ed.select_box = None;
                            }
                            if self.mouse.left {
                                let (dx, dy) = (self.mouse.pos.0 - self.mouse.press.0, self.mouse.pos.1 - self.mouse.press.1);
                                let dragged = dx * dx + dy * dy > 12.0 * 12.0;
                                if let Some(assets) = self.assets.as_mut() {
                                    self.ed.left_release(assets, &self.lib, dragged);
                                }
                            }
                            self.mouse.left = false;
                            self.mouse.grab = false;
                            self.mouse.alt_orbit = false;
                            self.mouse.grab_anchor = None;
                        }
                    }
                    MouseButton::Right => {
                        if pressed && !over_ui && self.ed.grab.is_some() {
                            // (a right click calls the grab off, as Blender's does)
                            self.ed.grab_cancel();
                        } else if pressed && !over_ui {
                            self.mouse.right = true;
                            self.mouse.right_moved = 0.0;
                            self.mouse.look_acc = (0.0, 0.0);
                            if !self.ui.view_2d && !self.keys.alt && !self.prefs.omsi_controls {
                                let ok = g.window.set_cursor_grab(winit::window::CursorGrabMode::Locked).or_else(|_| g.window.set_cursor_grab(winit::window::CursorGrabMode::Confined)).is_ok();
                                if ok {
                                    g.window.set_cursor_visible(false);
                                }
                                self.mouse.look_raw = ok;
                            }
                        } else if !pressed && self.mouse.right {
                            self.mouse.right = false;
                            if self.mouse.look_raw {
                                self.mouse.look_raw = false;
                                let _ = g.window.set_cursor_grab(winit::window::CursorGrabMode::None);
                                g.window.set_cursor_visible(true);
                                let _ = g.window.set_cursor_position(winit::dpi::LogicalPosition::new(self.mouse.pos.0 as f64, self.mouse.pos.1 as f64));
                            }
                            let menu = self.mouse.right_moved < 5.0 && self.ed.right_click();
                            if menu && self.prefs.omsi_controls && !self.keys.shift {
                                // (OMSI 2's editor: a right click on the ground goes there; Shift opens the menu)
                                if let Some(c) = self.ed.cursor {
                                    let d = self.cam.distance;
                                    self.cam.jump_to(c, d);
                                }
                            } else if menu {
                                self.ui.view_menu = Some(crate::ui_menu::ViewMenu { at: egui::pos2(self.mouse.pos.0, self.mouse.pos.1), ground: self.ed.cursor });
                            }
                        }
                    }
                    MouseButton::Middle => {
                        self.mouse.middle = pressed && !over_ui;
                        self.mouse.grab_anchor = if self.mouse.middle { self.grab_point() } else { None };
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.mouse.over_ui {
                    return;
                }
                // (a bevel or loop cut going on: the wheel sets its segments or cuts)
                if let Some(m) = self.ed.mesh.as_mut().filter(|m| m.modal_active()) {
                    let y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => (p.y / 30.0) as f32,
                    };
                    if y.abs() >= 0.5 {
                        m.modal_count(y.signum() as i32);
                    }
                    return;
                }
                let steps = match delta {
                    // (flying: the wheel sets the speed)
                    MouseScrollDelta::LineDelta(_, y) if self.mouse.right => {
                        self.cam.fly_faster(y);
                        self.prefs.cam_tune.fly_speed = self.cam.fly_speed;
                        self.ed.toast(format!("{} {:.0} m/s", tr("Flying speed", "Скорость полёта"), self.cam.fly_speed));
                        return;
                    }
                    MouseScrollDelta::LineDelta(_, y) if self.fly_style() && !self.keys.ctrl && !(self.keys.shift && (matches!(self.ed.tool, Tool::Terrain | Tool::Paint) || (self.ed.tool == Tool::Objects && self.ed.plant_brush))) => {
                        // Unity: the wheel moves towards the point looked at, at once
                        self.cam.dolly(y);
                        return;
                    }
                    MouseScrollDelta::LineDelta(_, y) => y,
                    // a trackpad: two fingers move the map (Option + two fingers turn it),
                    // pinching zooms
                    MouseScrollDelta::PixelDelta(p) => {
                        let (dx, dy) = (p.x as f32 / scale, p.y as f32 / scale);
                        if self.keys.alt {
                            self.cam.orbit_pixels(-dx, -dy);
                        } else if self.fly_style() && !self.keys.ctrl && !self.keys.shift {
                            // Unity on a trackpad: two fingers up and down zoom, sideways pan
                            self.cam.dolly(dy * 0.04);
                            self.cam.pan_screen(dx, 0.0, self.view.height().max(1.0));
                        } else if self.keys.ctrl {
                            self.ed.rotate(dy as f64 * 0.5);
                        } else if self.keys.shift && (matches!(self.ed.tool, Tool::Terrain | Tool::Paint) || (self.ed.tool == Tool::Objects && self.ed.plant_brush)) {
                            self.ed.brush_radius = (self.ed.brush_radius * 1.01f32.powf(dy)).clamp(3.0, 150.0);
                        } else {
                            // (the ground under the cursor moves with the fingers)
                            let v = self.view;
                            let (px, py) = (self.mouse.pos.0 - v.min.x, self.mouse.pos.1 - v.min.y);
                            match self.grab_point() {
                                Some(a) if v.is_positive() => {
                                    let (o, d) = self.cam.ray(px + dx, py + dy, v.width(), v.height());
                                    self.cam.grab(a, o, d);
                                }
                                _ => self.cam.pan_pixels(-dx, -dy, v.height().max(1.0)),
                            }
                        }
                        return;
                    }
                };
                if self.keys.ctrl {
                    self.ed.rotate(steps as f64 * 15.0);
                } else if self.keys.shift && (matches!(self.ed.tool, Tool::Terrain | Tool::Paint) || (self.ed.tool == Tool::Objects && self.ed.plant_brush)) {
                    self.ed.brush_radius = (self.ed.brush_radius * 1.1f32.powf(steps)).clamp(3.0, 150.0);
                } else {
                    self.cam.zoom(steps, self.ed.cursor);
                }
            }
            WindowEvent::PinchGesture { delta, .. } => {
                if !self.mouse.over_ui {
                    if self.fly_style() {
                        self.cam.dolly(delta as f32 * 9.0);
                    } else {
                        self.cam.zoom(delta as f32 * 9.0, self.ed.cursor);
                    }
                }
            }
            WindowEvent::RotationGesture { delta, .. } => {
                if !self.mouse.over_ui {
                    self.cam.orbit_pixels(-delta / 0.35, 0.0);
                }
            }
            WindowEvent::DroppedFile(p) => {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                // (a GPS track: to trace; a picture dropped with the Tiles tool on: the clicked
                // tile's own picture)
                if ["gpx", "kml", "geojson"].contains(&ext.as_str()) && self.ed.doc.is_some() {
                    self.trace.add_file(&p);
                    self.trace.open = true;
                } else if self.ed.tool != Tool::Tiles || !self.geo.drop_picture(&p, &mut self.ed) {
                    self.import(vec![p]);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: winit::event::DeviceEvent) {
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            if self.mouse.right && self.mouse.look_raw {
                self.mouse.look_acc.0 += delta.0;
                self.mouse.look_acc.1 += delta.1;
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // (nobody at the mouse or keys, the view still: a few frames a second are enough)
        let idle = self.last_input.elapsed().as_secs_f32() > 3.0 && !self.cam.is_moving() && !self.lib.scanning && self.assets.as_ref().map(|a| a.pending == 0).unwrap_or(true) && self.shot.is_none() && !crate::render::profile::enabled();
        if idle {
            if self.last.elapsed().as_millis() >= 50 {
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + std::time::Duration::from_millis(50)));
            return;
        }
        el.set_control_flow(ControlFlow::Poll);
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
        // (a hidden window gets no redraws: the screenshot and profiling runs go on anyway)
        if self.shot.is_some() || crate::render::profile::enabled() {
            if self.last.elapsed().as_millis() > 250 {
                self.frame(el);
            }
            // (a timer, so the event loop wakes even when the system lets it sleep)
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + std::time::Duration::from_millis(5)));
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        let _ = self.quit;
        // (a normal end: the changes were saved or dropped when asked; an autosave still
        // offered and not answered stays for the next start)
        if let (Some(r), None) = (&self.roots, &self.recover) {
            crate::doc::autosave::clear(&r.content);
        }
        self.prefs.save();
    }
}

/// The most tiles loaded at a time with tile streaming on.
const STREAM_TILES: usize = 100;

/// How many tiles a window of streaming holds, as the settings say (0: the usual 100).
fn stream_count(setting: u32) -> usize {
    if setting == 0 { STREAM_TILES } else { setting.clamp(4, 2000) as usize }
}

/// The tiles to load, when that changed: the nearest `count` to the place the window
/// is round (the camera's, until a click on the empty zone moves it), so they lie in a circle.
/// `Some((None, _))`: everything (streaming off, or a map that small).
fn stream_window(doc: &Document, on: bool, count: usize, at: &mut Option<DVec2>, cam: &Camera, seen: Option<(usize, u64, u64)>) -> Option<(Option<std::collections::HashSet<crate::doc::TileKey>>, (usize, u64, u64))> {
    let centre = if on { *at.get_or_insert(DVec2::new(cam.target.x, cam.target.y)) } else { *at = None; DVec2::ZERO };
    let stamp = if on { (doc.tiles.len() + count * 1_000_000_000, centre.x.to_bits(), centre.y.to_bits()) } else { (0, 0, 0) };
    if seen == Some(stamp) {
        return None;
    }
    if !on || doc.tiles.len() <= count {
        return Some((None, stamp));
    }
    let half = DVec2::splat(tile_size() * 0.5);
    let mut near: Vec<(f64, crate::doc::TileKey)> = doc.tiles.keys().map(|k| ((tile_origin(*k) + half - centre).length_squared(), *k)).collect();
    near.select_nth_unstable_by(count, |a, b| a.0.total_cmp(&b.0));
    Some((Some(near[..count].iter().map(|n| n.1).collect()), stamp))
}

/// Write the window's picture to a PNG (the screenshot mode).
/// Show a file in the system's file manager.
fn reveal(p: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn();
}

/// A copy of a map in the content folder under a free folder name ("<folder> copy", …), its
/// name marked as a copy; where its global.cfg is.
fn copy_map(roots: &Roots, cfg: &Path) -> anyhow::Result<PathBuf> {
    let src = cfg.parent().ok_or_else(|| anyhow::anyhow!("no folder"))?;
    let folder = src.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Map".into());
    let maps = roots.content.join("maps");
    let free = (1..).map(|i| if i == 1 { format!("{folder} {}", tr("copy", "копия")) } else { format!("{folder} {} {i}", tr("copy", "копия")) }).find(|n| !maps.join(n).exists()).unwrap();
    let mut doc = Document::load(roots, cfg, |_| {})?;
    doc.dir = maps.join(&free);
    doc.source = Some(src.to_path_buf());
    let name = if doc.global.friendly_name.is_empty() { doc.global.name.clone() } else { doc.global.friendly_name.clone() };
    doc.global.friendly_name = format!("{name} ({})", tr("copy", "копия"));
    doc.global_dirty = true;
    crate::doc::save(&mut doc)?;
    Ok(doc.dir.join("global.cfg"))
}

/// A content folder map's folder moved into `openMapper\bin\maps` (never the installation's).
fn bin_map(content: &Path, cfg: &Path) -> anyhow::Result<()> {
    let dir = cfg.parent().ok_or_else(|| anyhow::anyhow!("no folder"))?;
    anyhow::ensure!(dir.starts_with(content.join("maps")) && dir.is_dir(), "{}", tr("Only maps of the content folder can be moved", "Переносить можно только карты папки контента"));
    let bin = content.join("openMapper").join("bin").join("maps");
    std::fs::create_dir_all(&bin)?;
    let name = dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let to = (1..).map(|i| if i == 1 { bin.join(&name) } else { bin.join(format!("{name} {i}")) }).find(|p| !p.exists()).unwrap();
    std::fs::rename(dir, to)?;
    Ok(())
}

/// Move a content folder asset's file into `openMapper\bin` (same place below it), so it
/// can be put back by hand. Only ever the content folder's own files.
fn bin_asset(content: &Path, rel: &str) -> anyhow::Result<()> {
    let p = omsi_cfg::resolve_path(content, rel);
    anyhow::ensure!(p.starts_with(content) && p.is_file(), "{}", tr("Only assets of the content folder can be deleted", "Удалять можно только ассеты папки контента"));
    let to = content.join("openMapper").join("bin").join(p.strip_prefix(content)?);
    std::fs::create_dir_all(to.parent().unwrap())?;
    std::fs::rename(&p, &to)?;
    Ok(())
}

/// The texture files a road type is drawn with, each once.
fn spline_textures(assets: &mut Assets, rel: &str) -> Vec<PathBuf> {
    let Some(m) = assets.spline(rel) else { return Vec::new() };
    let mut out: Vec<PathBuf> = Vec::new();
    for t in m.materials.iter().filter_map(|mat| mat.texture.clone()) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// The texture files an object's model is drawn with, each once.
fn object_textures(assets: &mut Assets, rel: &str) -> Vec<PathBuf> {
    let Some(m) = assets.object(rel) else { return Vec::new() };
    let mut out: Vec<PathBuf> = Vec::new();
    for t in m.parts.iter().flat_map(|p| p.materials.iter()).filter_map(|mat| mat.texture.clone()) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn save_shot(gpu: &Gpu, tex: &wgpu::Texture, path: &Path) -> anyhow::Result<()> {
    read_frame(gpu, tex)?.save(path)?;
    Ok(())
}

/// OMSI's map picture: 370 × 280, the middle of the view cut to that shape.
fn save_map_picture(img: image::RgbaImage, crop: [u32; 4], path: &Path) -> anyhow::Result<()> {
    let (w, h) = (crop[2].min(img.width().saturating_sub(crop[0])), crop[3].min(img.height().saturating_sub(crop[1])));
    anyhow::ensure!(w > 8 && h > 8, "the view is too small");
    let (pw, ph) = (370u32, 280u32);
    let (cw, ch) = if w * ph > h * pw { (h * pw / ph, h) } else { (w, w * ph / pw) };
    let (x, y) = (crop[0] + (w - cw) / 2, crop[1] + (h - ch) / 2);
    let cut = image::imageops::crop_imm(&img, x, y, cw, ch).to_image();
    let small = image::imageops::resize(&cut, pw, ph, image::imageops::FilterType::Triangle);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::DynamicImage::ImageRgba8(small).to_rgb8().save_with_format(path, image::ImageFormat::Jpeg)?;
    Ok(())
}

/// The window's picture as it was drawn.
fn read_frame(gpu: &Gpu, tex: &wgpu::Texture) -> anyhow::Result<image::RgbaImage> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shot"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range();
    let bgra = matches!(tex.format().remove_srgb_suffix(), wgpu::TextureFormat::Bgra8Unorm);
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y * row + x * 4) as usize;
            let p = &data[i..i + 4];
            let px = if bgra { [p[2], p[1], p[0], 255] } else { [p[0], p[1], p[2], 255] };
            img.put_pixel(x, y, image::Rgba(px));
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_picture_is_omsi_sized() {
        let dir = std::env::temp_dir().join(format!("openmapper-picture-{}", std::process::id()));
        let path = dir.join("picture.jpg");
        let img = image::RgbaImage::from_pixel(1600, 900, image::Rgba([40, 120, 60, 255]));
        super::save_map_picture(img, [200, 50, 1200, 800], &path).unwrap();
        let back = image::open(&path).unwrap();
        assert_eq!((back.width(), back.height()), (370, 280));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_map_is_copied_under_a_new_name_and_moved_to_the_bin() {
        let roots = crate::doc::tests::scratch("copymap");
        let mut doc = crate::doc::tests::sample_map(&roots);
        crate::doc::save(&mut doc).unwrap();
        let cfg = doc.dir.join("global.cfg");
        let copy = copy_map(&roots, &cfg).unwrap();
        assert!(copy.parent().unwrap().ends_with(format!("Roundtrip {}", tr("copy", "копия"))));
        let back = Document::load(&roots, &copy, |_| {}).unwrap();
        assert_eq!(back.tiles.len(), doc.tiles.len());
        assert!(back.global.friendly_name.ends_with(&format!("({})", tr("copy", "копия"))));
        // a second copy takes the next free name
        let again = copy_map(&roots, &cfg).unwrap();
        assert_ne!(again, copy);
        bin_map(&roots.content, &copy).unwrap();
        assert!(!copy.exists());
        assert!(roots.content.join("openMapper").join("bin").join("maps").join(copy.parent().unwrap().file_name().unwrap()).join("global.cfg").is_file());
        // never a map outside the content folder
        assert!(bin_map(&roots.content.join("maps"), &roots.omsi.join("maps").join("x").join("global.cfg")).is_err());
        let _ = std::fs::remove_dir_all(&roots.content);
    }
}
