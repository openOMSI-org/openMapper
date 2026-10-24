//! The interface, laid out like a desktop editor: a menu bar, a toolbar, the tool's panel
//! on the left (catalogue, brushes, layers), the properties of what is selected on the
//! right, a status bar at the bottom, and the 3D view in the middle. Dialogs are plain
//! windows. Help for whatever the mouse is over goes to the status bar.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align, Color32, CornerRadius, Frame, Id, Layout, Margin, RichText, Sense, Stroke, Vec2};

use crate::doc::Sel;
use crate::editor::{Editor, LogicMode, TerrainMode, Tool};
use crate::i18n::{self, tr, Lang};
use crate::icons::{self, Icon};
use crate::install::{MapEntry, Prefs};
use crate::library::{Category, Kind, Library};
use crate::render::Thumb;

pub const ACCENT: Color32 = Color32::from_rgb(232, 122, 50);
const BG: Color32 = Color32::from_rgb(36, 37, 40);
const BG_DARK: Color32 = Color32::from_rgb(27, 28, 30);
pub(crate) const BG_FIELD: Color32 = Color32::from_rgb(22, 23, 25);
const BORDER: Color32 = Color32::from_rgb(18, 18, 20);
const HEADER: Color32 = Color32::from_rgb(45, 46, 50);
pub(crate) const TEXT_DIM: Color32 = Color32::from_rgb(140, 143, 150);
pub(crate) const SELECT: Color32 = Color32::from_rgb(64, 86, 122);

/// What the interface asks the application to do.
pub enum Action {
    BindKey(crate::controls::Cmd, Option<crate::controls::Chord>),
    ResetKeys,
    NewMap { folder: String, friendly: String, size: i32, options: crate::ui_newmap::NewMapOptions },
    Open(PathBuf),
    OpenDialog,
    Save,
    Close,
    /// A new object of one of the shapes (`primitives::ALL`), to put on the map.
    CreatePrimitive(usize),
    /// Something done to openMapper's own objects (edit mode, join, booleans, export...).
    Mesh(crate::ui_mesh::MeshAction),
    Play,
    /// A test drive from the start point nearest the view (false), or the view itself seen
    /// in openOMSI with a free camera (true).
    PlayHere(bool),
    /// A test drive in OMSI 2 itself (the map and what it needs copied into its folder).
    PlayOriginal,
    /// Paste at this ground point (the view's menu).
    PasteAt(Option<glam::DVec3>),
    /// The groups kept for other maps.
    SavedGroups,
    Undo,
    Redo,
    Import,
    ChooseOmsi,
    SetLang(Lang),
    /// Place this object at the cursor (a picture dropped from the asset browser).
    PlaceAsset(String),
    /// Show an asset's file in the system's file manager.
    RevealAsset(String),
    /// Move an asset of the content folder into openMapper's bin.
    DeleteAsset(String),
    /// Welcome list: a map's folder shown in the file manager (its global.cfg).
    RevealMap(PathBuf),
    /// Welcome list: a copy of a map made in the content folder under a new name.
    CopyMap(PathBuf),
    /// Welcome list: a content folder map moved into openMapper's bin.
    BinMap(PathBuf),
    /// The Road types window (road maker, junction styles).
    RoadTypes,
    /// The PBR window on an object's textures.
    PbrForObject(String),
    /// The PBR window on a road type's textures.
    PbrForSpline(String),
    /// The selected roads' lanes made into AI paths of their own.
    DetachLanes,
    /// The relief of the whole map taken away, roads and paths laid on the flat ground.
    FlattenTerrain,
    /// The PBR window on a picture chosen from disk.
    PbrMaker,
    /// Content files changed (maps made, imported): read the catalogue and models again,
    /// then say so.
    ReloadContent(String),
    /// A material of an imported object made opaque, cut out or blended: the object's file,
    /// its mesh, the material's texture and use number, the alpha.
    SetAlpha(String, usize, String, i32, u8),
    /// How the mouse moves the camera ("map" or "fly").
    SetCamera(String),
    SetTileStreaming(bool),
    /// The interface's zoom (1 = as the system has it).
    SetScale(f32),
    FocusSelection,
    /// Star a catalogue entry or take its star away.
    ToggleFavourite(String),
    /// A catalogue entry was chosen (for the recently used list).
    UsedAsset(String),
    /// The 3D view becomes the map's picture (`picture.jpg`).
    ImportMapPicture,
    /// A picture of the view, full size, into the Pictures folder.
    ViewPicture,
    /// The map settings window (money, tickets, years, depot, seasons).
    MapSettings,
    /// The calendar and place window (holidays, time zone, summer time, number plates).
    Calendar,
    /// The signal routes window (railway signals and the track they protect).
    SignalRoutes,
    /// The chrono scenarios window (the map over the years).
    Chrono,
    /// The Lines window opened.
    OpenLines,
    /// The Traffic and AI window opened.
    OpenTraffic,
    /// The Traffic and AI window opened on the depot files.
    Depots,
    /// The map opens with the camera as it is now (`[mapcam]`).
    SetMapCamera,
    Copy,
    Cut,
    Paste,
    Duplicate,
    SelectAll,
    /// Build the road drawn so far.
    BuildRoad,
    /// Move the camera over this point (the overview map).
    LookAt(glam::DVec2),
    AddGroundTexture(String),
    /// A ground layer's texture (or its detail texture: true) changed.
    SetGroundTexture(usize, bool, String),
    /// Open or close the Lines window.
    Timetable,
    /// Put the map's folder back as it was before the save kept in this backup (and the
    /// later ones), then open it again.
    RestoreBackup(PathBuf),
    /// Open or close the Traffic and AI window.
    Traffic,
    /// Open or close the Satellite and terrain window.
    Geo,
    /// Open or close Roads from OpenStreetMap.
    Osm,
    /// View > Season: the textures of a season (`None`: summer's).
    SetSeason(Option<&'static str>),
    /// View > Graphics quality.
    SetGraphics(crate::render::quality::Quality),
    /// Open or close the Getting started guide.
    Guide,
    /// Open or close File > Multiplayer.
    Multiplayer,
    /// Help > Updates.
    Updates,
    /// Map > Tiles from another map (the maps to choose from are listed first).
    MergeTiles,
    /// Open or close GPS tracks to trace.
    Trace,
    /// Save and run the map in openOMSI offscreen with its checks.
    GameCheck,
    /// Save the map and pack it into a .zip to share.
    Export,
    /// A plan of the map as an SVG picture.
    ExportPlan,
    /// Do it without asking about unsaved changes.
    Force(Box<Action>),
    /// Save, then do it; nothing more if the save fails.
    SaveThen(Box<Action>),
    Quit,
}

/// State of the interface that lives between frames.
pub struct UiState {
    pub search: String,
    /// The plan view: looking straight down, north up, the tilt and the turn held.
    pub view_2d: bool,
    /// What a bake of an openMapper object makes; whether booleans keep their cutters; the
    /// UV editor open; the lamps of a signal head to make.
    pub bake: crate::ui_mesh::BakeChoice,
    pub keep_cutters: bool,
    pub uv_open: bool,
    pub uv: crate::ui_uv::UvEditor,
    pub signal_lamps: Vec<crate::editable::Lamp>,
    pub makers: crate::makers::Settings,
    /// Scaling an object in the properties keeps its proportions.
    pub scale_uniform: bool,
    /// Saved views (View menu, Ctrl+1…9).
    pub views: crate::ui_views::Views,
    /// Search for a command (Ctrl+K).
    pub palette: crate::ui_palette::Palette,
    /// Map > Tiles from another map.
    pub merge: crate::ui_merge::MergeWindow,
    pub object_cat: Option<Category>,
    pub spline_cat: Option<Category>,
    pub new_map: Option<crate::ui_newmap::NewMapForm>,
    pub help_open: bool,
    /// The command whose key is being chosen (the next key pressed becomes it).
    pub key_capture: Option<crate::controls::Cmd>,
    pub about_open: bool,
    pub texture_picker: Option<String>,
    /// What the texture picker chooses for: a ground layer's texture or detail texture
    /// (layer, detail), or (None) a new layer.
    pub texture_pick_for: Option<(usize, bool)>,
    pub maps: Vec<MapEntry>,
    /// Waiting for an answer about unsaved changes before doing this.
    pub confirm: Option<Action>,
    /// Help for what the mouse is over, shown in the status bar.
    pub status_help: Option<String>,
    pub welcome_pick: Option<PathBuf>,
    pub minimap: crate::ui_minimap::Minimap,
    pub assets: crate::ui_assets::AssetBrowser,
    pub check: crate::check::Checker,
    pub find: crate::ui_find::Find,
    pub groups: crate::ui_groups::Groups,
    /// The window of the map's earlier versions (backups of its saves).
    pub versions_open: bool,
    /// The window of the edits made (undo and redo by a click).
    pub history_open: bool,
    /// The preferences' starred and recently used assets, for the catalogue.
    pub favourites: Vec<String>,
    pub recent_assets: Vec<String>,
    /// The selected object's file and whether its textures' PBR maps were made here.
    pub pbr_made: Option<(String, bool)>,
    /// The selected object's model, for its properties.
    pub obj_info: Option<ObjInfo>,
    /// The lights of the program the selected object's `[varparent]` runs (its file, the
    /// lights' names), for a signal head's properties.
    pub parent_lights: Option<(String, Vec<String>)>,
    /// The map's `picture.jpg` as shown in the map's properties, with its file time.
    map_picture: Option<(PathBuf, Option<std::time::SystemTime>, Option<egui::TextureHandle>)>,
    /// The satellite overlay as the View menu and the Tiles tool show it.
    pub geo: crate::ui_geo::Bridge,
    /// The Objects tab of the Select panel: everything on the map by folder.
    pub outliner: crate::ui_outliner::Outliner,
    /// The view's right-click menu, while it is open.
    pub view_menu: Option<crate::ui_menu::ViewMenu>,
    /// The Terrain tool's height map export and import.
    pub heightmap: crate::heightmap::HeightMapUi,
    /// The Paint tool's rule by slope and height.
    pub autopaint: crate::autopaint::Rule,
    /// The hour the view is lit at (View menu; None: plain day as openOMSI's).
    pub daytime: Option<f32>,
    /// The season the view shows (View menu; None: summer).
    pub season: Option<&'static str>,
    /// The maps' pictures on the welcome screen, small (None: the map has none).
    welcome_pictures: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            search: String::new(),
            view_2d: false,
            bake: Default::default(),
            keep_cutters: false,
            uv_open: false,
            uv: Default::default(),
            signal_lamps: vec![crate::editable::Lamp::Red, crate::editable::Lamp::Yellow, crate::editable::Lamp::Green],
            makers: Default::default(),
            scale_uniform: true,
            views: Default::default(),
            palette: Default::default(),
            merge: Default::default(),
            object_cat: None,
            spline_cat: None,
            new_map: None,
            help_open: false,
            key_capture: None,
            about_open: false,
            texture_picker: None,
            texture_pick_for: None,
            maps: Vec::new(),
            confirm: None,
            status_help: None,
            welcome_pick: None,
            minimap: crate::ui_minimap::Minimap::new(),
            assets: Default::default(),
            check: Default::default(),
            find: Default::default(),
            groups: Default::default(),
            versions_open: false,
            history_open: false,
            favourites: Vec::new(),
            recent_assets: Vec::new(),
            pbr_made: None,
            obj_info: None,
            parent_lights: None,
            map_picture: None,
            geo: Default::default(),
            outliner: Default::default(),
            view_menu: None,
            heightmap: Default::default(),
            autopaint: Default::default(),
            daytime: None,
            season: None,
            welcome_pictures: HashMap::new(),
        }
    }
}

/// One mesh of a model.
#[derive(Clone)]
pub struct PieceInfo {
    pub triangles: usize,
    /// Each material: its texture's name, which use of that texture it is, and its alpha (0
    /// opaque, 1 cut out, 2 blended).
    pub materials: Vec<(String, i32, u8)>,
}

/// What the properties show of a selected object's model.
#[derive(Clone)]
pub struct ObjInfo {
    pub file: String,
    pub size: [f32; 3],
    pub triangles: usize,
    /// Its textures' names and whether they have PBR maps.
    pub textures: Vec<(String, bool)>,
    /// What its labels mean (empty: it reads none; an empty name: unnamed).
    pub labels: Vec<String>,
    /// Its meshes, as the file had them, with the transparency of their materials.
    pub pieces: Vec<PieceInfo>,
    /// How many paths (lanes) it has.
    pub paths: usize,
    /// How many points it has to hang other objects on (`[new_attachment]`).
    pub attach_points: usize,
    /// Its traffic light program's lights (a crossing).
    pub lights: Vec<String>,
    /// A signal head showing one light of its parent's program.
    pub light_head: bool,
}

/// Facts for the status bar.
#[derive(Default)]
pub struct Status {
    pub cursor: Option<glam::DVec3>,
    pub loading: usize,
    pub gpu: String,
    pub cam: Option<crate::ui_minimap::CamView>,
}

pub fn style(ctx: &egui::Context) {
    ctx.set_fonts(icons::fonts());
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.faint_bg_color = Color32::from_rgb(40, 41, 45);
    v.extreme_bg_color = BG_FIELD;
    v.text_edit_bg_color = Some(BG_FIELD);
    v.code_bg_color = BG_DARK;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(3);
    v.window_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(110) };
    v.popup_shadow = egui::Shadow { offset: [0, 2], blur: 6, spread: 0, color: Color32::from_black_alpha(100) };
    v.selection.bg_fill = SELECT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = ACCENT;
    v.widgets.noninteractive.bg_fill = BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(24, 24, 27));
    v.widgets.inactive.bg_fill = Color32::from_rgb(56, 58, 63);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(52, 54, 59);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(24, 24, 27));
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(66, 68, 74);
    v.widgets.hovered.bg_fill = Color32::from_rgb(70, 72, 78);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(90, 92, 100));
    v.widgets.active.weak_bg_fill = Color32::from_rgb(78, 80, 88);
    v.widgets.active.bg_fill = Color32::from_rgb(78, 80, 88);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(3);
    }
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.spacing.item_spacing = Vec2::new(6.0, 4.0);
        s.spacing.button_padding = Vec2::new(7.0, 3.0);
        s.spacing.interact_size = Vec2::new(36.0, 22.0);
        s.spacing.slider_width = 150.0;
        s.spacing.menu_margin = Margin::same(4);
        s.spacing.window_margin = Margin::same(10);
        use egui::{FontId, TextStyle};
        s.text_styles.insert(TextStyle::Body, FontId::proportional(13.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(13.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(16.0));
        s.interaction.tooltip_delay = 10.0;
    });
    ctx.set_theme(egui::Theme::Dark);
}

/// Note help for the status bar when the mouse is over `r`.
fn help(st: &mut UiState, r: &egui::Response, text: impl Into<String>) {
    if r.hovered() {
        st.status_help = Some(text.into());
    }
}

/// A flat toolbar button with an icon; `on` draws it pressed.
fn tool_button(ui: &mut egui::Ui, icon: Icon, on: bool, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if on {
        Color32::from_rgb(70, 74, 82)
    } else if resp.hovered() && enabled {
        Color32::from_rgb(56, 58, 64)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), fill);
    if on {
        ui.painter().rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, Color32::from_rgb(98, 102, 112)), egui::StrokeKind::Inside);
        ui.painter().line_segment([rect.left_bottom() + Vec2::new(4.0, -1.5), rect.right_bottom() + Vec2::new(-4.0, -1.5)], Stroke::new(2.0, ACCENT));
    }
    let fg = if !enabled {
        Color32::from_gray(85)
    } else if on {
        Color32::WHITE
    } else {
        Color32::from_gray(205)
    };
    icons::paint(ui.painter(), icon, rect.center(), 20.0, fg);
    resp
}

/// A check box drawn with Material Symbols' own boxes (egui's are drawn with lines).
pub(crate) fn check(ui: &mut egui::Ui, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let galley = text.into().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let (icon, gap) = (18.0, 6.0);
    let size = Vec2::new(icon + gap + galley.size().x, galley.size().y.max(icon + 2.0));
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let vis = ui.style().interact(&resp);
    let col = if *on { ACCENT } else { vis.fg_stroke.color };
    icons::paint(ui.painter(), if *on { Icon::CheckOn } else { Icon::CheckOff }, egui::pos2(rect.left() + icon * 0.5, rect.center().y), icon + 2.0, col);
    let text_color = vis.text_color();
    ui.painter().galley(egui::pos2(rect.left() + icon + gap, rect.center().y - galley.size().y * 0.5), galley, text_color);
    resp
}

/// A radio button drawn with Material Symbols' own circles.
fn radio(ui: &mut egui::Ui, on: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let galley = text.into().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let (icon, gap) = (18.0, 6.0);
    let size = Vec2::new(icon + gap + galley.size().x, galley.size().y.max(icon + 2.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let vis = ui.style().interact(&resp);
    let col = if on { ACCENT } else { vis.fg_stroke.color };
    icons::paint(ui.painter(), if on { Icon::RadioOn } else { Icon::RadioOff }, egui::pos2(rect.left() + icon * 0.5, rect.center().y), icon + 2.0, col);
    let text_color = vis.text_color();
    ui.painter().galley(egui::pos2(rect.left() + icon + gap, rect.center().y - galley.size().y * 0.5), galley, text_color);
    resp
}

/// The open / closed arrow of a collapsing group, as Material Symbols draws it.
fn group_icon(ui: &mut egui::Ui, openness: f32, resp: &egui::Response) {
    let col = ui.style().interact(resp).fg_stroke.color;
    icons::paint(ui.painter(), if openness > 0.5 { Icon::ExpandMore } else { Icon::ChevronRight }, resp.rect.center(), 18.0, col);
}

/// The drop-down arrow of a combo box, as Material Symbols draws it.
fn combo_icon(ui: &egui::Ui, rect: egui::Rect, vis: &egui::style::WidgetVisuals, _open: bool) {
    icons::paint(ui.painter(), Icon::ExpandMore, rect.center(), 18.0, vis.fg_stroke.color);
}

pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    Frame::new().fill(HEADER).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title.to_uppercase()).small().strong().color(Color32::from_gray(190)));
    });
}

fn panel_frame() -> Frame {
    Frame::new().fill(BG).inner_margin(Margin::ZERO).stroke(Stroke::new(1.0, BORDER))
}

fn menu_item(ui: &mut egui::Ui, icon: Option<Icon>, text: &str, shortcut: &str, enabled: bool) -> bool {
    let label = match icon {
        Some(i) => egui::WidgetText::from(icons::labelled(i, text)),
        None => egui::WidgetText::from(format!("      {text}")),
    };
    let b = egui::Button::new(label).shortcut_text(shortcut);
    ui.add_enabled(enabled, b).clicked()
}

fn lang_menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.menu_button(tr("Language", "Язык"), |ui| {
        for (l, n) in crate::i18n::LANGUAGES {
            if radio(ui, i18n::get() == l, n).clicked() {
                actions.push(Action::SetLang(l));
            }
        }
    });
    ui.menu_button(tr("Interface size", "Размер интерфейса"), |ui| {
        let now = ui.ctx().zoom_factor();
        for z in [0.8f32, 0.9, 1.0, 1.1, 1.25, 1.5] {
            if radio(ui, (now - z).abs() < 0.01, format!("{:.0} %", z * 100.0)).clicked() {
                actions.push(Action::SetScale(z));
            }
        }
    });
}

// ---------------------------------------------------------------------------- start page

pub fn welcome(root: &mut egui::Ui, st: &mut UiState, prefs: &Prefs, omsi: Option<&std::path::Path>, actions: &mut Vec<Action>) {
    egui::Panel::top("menu").frame(Frame::new().fill(BG_DARK).inner_margin(Margin::symmetric(6, 2))).show(root, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File", "Файл"), |ui| {
                if menu_item(ui, Some(Icon::NewFile), tr("New map…", "Новая карта…"), "", omsi.is_some()) {
                    st.new_map = Some(Default::default());
                }
                if menu_item(ui, Some(Icon::Open), tr("Open map…", "Открыть карту…"), "", omsi.is_some()) {
                    actions.push(Action::OpenDialog);
                }
                if menu_item(ui, Some(Icon::Help), tr("Controls…", "Управление…"), "", true) {
                    st.help_open = true;
                }
                if menu_item(ui, Some(Icon::Import), tr("Import assets…", "Импорт ассетов…"), "", omsi.is_some()) {
                    actions.push(Action::Import);
                }
                if menu_item(ui, Some(Icon::Walk), tr("Multiplayer…", "Совместная правка…"), "", omsi.is_some()) {
                    actions.push(Action::Multiplayer);
                }
                ui.separator();
                if menu_item(ui, Some(Icon::Settings), tr("OMSI 2 folder…", "Папка OMSI 2…"), "", true) {
                    actions.push(Action::ChooseOmsi);
                }
                ui.separator();
                if menu_item(ui, None, tr("Quit", "Выход"), "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button(tr("View", "Вид"), |ui| lang_menu(ui, actions));
            ui.menu_button(tr("Help", "Справка"), |ui| {
                if menu_item(ui, Some(Icon::Import), tr("Updates…", "Обновления…"), "", true) {
                    actions.push(Action::Updates);
                }
                if menu_item(ui, Some(Icon::Info), tr("About openMapper", "О программе"), "", true) {
                    st.about_open = true;
                }
            });
        });
    });
    egui::Panel::bottom("status").exact_size(22.0).frame(Frame::new().fill(BG_DARK).inner_margin(Margin::symmetric(8, 3))).show(root, |ui| {
        ui.horizontal(|ui| {
            match omsi {
                Some(p) => ui.label(RichText::new(format!("OMSI 2: {}", p.display())).small().color(TEXT_DIM)),
                None => ui.label(RichText::new(tr("OMSI 2 not found", "OMSI 2 не найдена")).small().color(Color32::from_rgb(230, 150, 90))),
            };
        });
    });
    egui::CentralPanel::default().frame(Frame::new().fill(Color32::from_rgb(30, 31, 34))).show(root, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.5 - 230.0).max(20.0));
            Frame::new().fill(BG).stroke(Stroke::new(1.0, BORDER)).corner_radius(CornerRadius::same(4)).show(ui, |ui| {
                ui.set_width(660.0);
                Frame::new().fill(HEADER).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(Icon::Map.text(30.0).color(ACCENT));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("openMapper").size(20.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(format!("{} · v{}", tr("Map editor for openOMSI", "Редактор карт для openOMSI"), env!("CARGO_PKG_VERSION"))).small().color(TEXT_DIM));
                        });
                    });
                });
                Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
                    if omsi.is_none() {
                        ui.label(tr("openMapper builds maps from the content of an installed OMSI 2. Choose its folder (the one with Omsi.exe, maps and Vehicles).", "openMapper строит карты из контента установленной OMSI 2. Укажите её папку (где лежат Omsi.exe, maps и Vehicles)."));
                        ui.add_space(8.0);
                        if ui.button(icons::labelled(Icon::Open, tr("Choose OMSI 2 folder…", "Выбрать папку OMSI 2…"))).clicked() {
                            actions.push(Action::ChooseOmsi);
                        }
                        return;
                    }
                    ui.horizontal_top(|ui| {
                        // the maps
                        ui.vertical(|ui| {
                            ui.set_width(430.0);
                            ui.label(RichText::new(tr("Maps", "Карты")).strong());
                            Frame::new().fill(BG_FIELD).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::ScrollArea::vertical().min_scrolled_height(340.0).max_height(340.0).auto_shrink([false, false]).show(ui, |ui| {
                                    // a recent map of the installation that was since saved opens
                                    // as its copy in the content folder, so it is listed once
                                    let folder_of = |p: &std::path::Path| p.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().to_lowercase());
                                    let mut recent: Vec<PathBuf> = Vec::new();
                                    for p in prefs.recent.iter().map(PathBuf::from).filter(|p| p.is_file()) {
                                        let p = st.maps.iter().find(|m| m.in_content && folder_of(&m.cfg) == folder_of(&p) && !p.starts_with(m.cfg.parent().unwrap_or(&m.cfg))).map(|m| m.cfg.clone()).unwrap_or(p);
                                        if !recent.contains(&p) {
                                            recent.push(p);
                                        }
                                    }
                                    let mut rows: Vec<(String, PathBuf, &str)> = Vec::new();
                                    for p in &recent {
                                        let name = st.maps.iter().find(|m| m.cfg == *p).map(|m| m.name.clone()).unwrap_or_else(|| p.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
                                        let kind = match st.maps.iter().find(|m| m.cfg == *p) {
                                            Some(m) if !m.in_content => tr("recent · OMSI 2", "недавняя · OMSI 2"),
                                            _ => tr("recent", "недавняя"),
                                        };
                                        rows.push((name, p.clone(), kind));
                                    }
                                    for m in &st.maps {
                                        if recent.contains(&m.cfg) {
                                            continue;
                                        }
                                        rows.push((m.name.clone(), m.cfg.clone(), if m.in_content { tr("content folder", "папка контента") } else { tr("OMSI 2 (saved as copy)", "OMSI 2 (сохр. копией)") }));
                                    }
                                    // (one picture read a frame: the list shows at once)
                                    let mut read_one = false;
                                    for (i, (name, cfg, kind)) in rows.iter().enumerate() {
                                        let on = st.welcome_pick.as_ref() == Some(cfg);
                                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::click());
                                        let fill = if on { SELECT } else if resp.hovered() { Color32::from_gray(40) } else if i % 2 == 1 { Color32::from_rgb(28, 29, 32) } else { Color32::TRANSPARENT };
                                        ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
                                        let pic = egui::Rect::from_min_size(rect.min + Vec2::new(6.0, 5.0), Vec2::new(70.0, 44.0));
                                        if !st.welcome_pictures.contains_key(cfg) && !read_one && ui.is_rect_visible(rect) {
                                            read_one = true;
                                            let tex = cfg.parent().and_then(|d| image::open(d.join("picture.jpg")).ok()).map(|img| {
                                                let img = img.thumbnail(160, 100).to_rgba8();
                                                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                                                ui.ctx().load_texture(format!("welcome {}", cfg.display()), ci, egui::TextureOptions::LINEAR)
                                            });
                                            st.welcome_pictures.insert(cfg.clone(), tex);
                                            ui.ctx().request_repaint();
                                        }
                                        match st.welcome_pictures.get(cfg) {
                                            Some(Some(t)) => {
                                                // (cut to the box's shape, not squeezed)
                                                let [w, h] = t.size().map(|v| v as f32);
                                                let (bw, bh) = (pic.width(), pic.height());
                                                let uv = if w / h > bw / bh {
                                                    let f = (bw / bh) / (w / h);
                                                    egui::Rect::from_min_max(egui::pos2(0.5 - f / 2.0, 0.0), egui::pos2(0.5 + f / 2.0, 1.0))
                                                } else {
                                                    let f = (w / h) / (bw / bh);
                                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.5 - f / 2.0), egui::pos2(1.0, 0.5 + f / 2.0))
                                                };
                                                ui.painter().image(t.id(), pic, uv, Color32::WHITE);
                                            }
                                            _ => {
                                                ui.painter().rect_filled(pic, CornerRadius::same(2), Color32::from_rgb(36, 38, 42));
                                                icons::paint(ui.painter(), Icon::Map, pic.center(), 22.0, if on { Color32::WHITE } else { TEXT_DIM });
                                            }
                                        }
                                        ui.painter().text(egui::pos2(pic.right() + 10.0, rect.top() + 18.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(14.0), Color32::from_gray(235));
                                        let folder = cfg.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                                        ui.painter().text(egui::pos2(pic.right() + 10.0, rect.top() + 37.0), egui::Align2::LEFT_CENTER, format!("{kind} · {folder}"), egui::FontId::proportional(11.0), TEXT_DIM);
                                        resp.clone().on_hover_text(cfg.display().to_string());
                                        if resp.clicked() {
                                            st.welcome_pick = Some(cfg.clone());
                                        }
                                        if resp.double_clicked() {
                                            actions.push(Action::Open(cfg.clone()));
                                        }
                                        let in_content = st.maps.iter().any(|m| m.cfg == *cfg && m.in_content);
                                        resp.context_menu(|ui| {
                                            if ui.button(icons::labelled(Icon::Open, tr("Open", "Открыть"))).clicked() {
                                                actions.push(Action::Open(cfg.clone()));
                                                ui.close();
                                            }
                                            if ui.button(icons::labelled(Icon::Copy, tr("Make a copy", "Сделать копию"))).on_hover_text(tr("A copy of the map in the content folder under a new name, to try something out", "Копия карты в папке контента под новым именем, чтобы что-то попробовать")).clicked() {
                                                actions.push(Action::CopyMap(cfg.clone()));
                                                ui.close();
                                            }
                                            if ui.button(icons::labelled(Icon::Search, tr("Show in the file manager", "Показать в файловом менеджере"))).clicked() {
                                                actions.push(Action::RevealMap(cfg.clone()));
                                                ui.close();
                                            }
                                            if in_content {
                                                ui.separator();
                                                if ui.button(icons::labelled(Icon::Delete, tr("Move to openMapper's bin", "Убрать в корзину openMapper"))).on_hover_text(tr("Its folder goes into openMapper\\bin in the content folder, from where it can be put back by hand", "Её папка переносится в openMapper\\bin в папке контента, откуда её можно вернуть вручную")).clicked() {
                                                    actions.push(Action::BinMap(cfg.clone()));
                                                    ui.close();
                                                }
                                            }
                                        });
                                    }
                                });
                            });
                        });
                        // the actions
                        ui.vertical(|ui| {
                            ui.add_space(19.0);
                            let w = Vec2::new(180.0, 26.0);
                            if ui.add_enabled(st.welcome_pick.is_some(), egui::Button::new(icons::labelled(Icon::Open, tr("Open", "Открыть"))).min_size(w)).clicked() {
                                if let Some(p) = st.welcome_pick.clone() {
                                    actions.push(Action::Open(p));
                                }
                            }
                            if ui.add(egui::Button::new(icons::labelled(Icon::NewFile, tr("New map…", "Новая карта…"))).min_size(w)).clicked() {
                                st.new_map = Some(Default::default());
                            }
                            if ui.add(egui::Button::new(icons::labelled(Icon::Open, tr("Browse…", "Обзор…"))).min_size(w)).clicked() {
                                actions.push(Action::OpenDialog);
                            }
                            ui.separator();
                            if ui.add(egui::Button::new(icons::labelled(Icon::Import, tr("Import assets…", "Импорт ассетов…"))).min_size(w)).clicked() {
                                actions.push(Action::Import);
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new(tr("Double-click a map to open it. Maps of OMSI 2 are saved as a copy into openOMSI's content folder; the original stays untouched.", "Двойной клик: открыть карту. Карты OMSI 2 сохраняются копией в папку контента openOMSI; оригинал не меняется.")).small().color(TEXT_DIM));
                });
            });
        });
    });
    about(root.ctx(), st);
}

fn about(ctx: &egui::Context, st: &mut UiState) {
    let mut open = st.about_open;
    egui::Window::new(tr("About openMapper", "О программе")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
        ui.label(RichText::new(format!("openMapper {}", env!("CARGO_PKG_VERSION"))).strong());
        ui.label(tr("A simple map editor for openOMSI.", "Простой редактор карт для openOMSI."));
        ui.hyperlink_to("openomsi.org", "https://openomsi.org/");
        ui.label(RichText::new("Icons: Material Symbols (Google, Apache 2.0)").small().color(TEXT_DIM));
    });
    st.about_open = open;
}

// ---------------------------------------------------------------------------- editor

pub struct EditorUi<'a> {
    pub ed: &'a mut Editor,
    pub lib: &'a Library,
    pub thumbs: &'a HashMap<String, Option<Thumb>>,
    pub wanted: &'a mut Vec<(String, bool)>,
    pub swatches: &'a HashMap<String, egui::TextureHandle>,
    pub wanted_swatches: &'a mut Vec<String>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub game_found: bool,
    pub texture_files: &'a [String],
    pub prefs: &'a Prefs,
    pub status: Status,
}

/// The editor's interface; returns the 3D view's rectangle and whether the mouse is over
/// it (and not over a window on top of it).
pub fn editor(root: &mut egui::Ui, st: &mut UiState, u: EditorUi, actions: &mut Vec<Action>) -> (egui::Rect, bool) {
    let EditorUi { ed, lib, thumbs, wanted, swatches, wanted_swatches, can_undo, can_redo, game_found, texture_files, prefs, status } = u;
    menu_bar(root, st, ed, can_undo, can_redo, game_found, prefs, actions);
    toolbar(root, st, ed, can_undo, can_redo, game_found, actions);
    // (the panels below fill in the help; the status bar shows last frame's)
    let help_line = st.status_help.take();
    status_bar(root, help_line, ed, &status);
    egui::Panel::left("tool panel").resizable(true).default_size(290.0).size_range(240.0..=480.0).frame(panel_frame()).show(root, |ui| {
        tool_panel(ui, st, ed, lib, thumbs, wanted, swatches, wanted_swatches, actions);
    });
    egui::Panel::right("properties").resizable(true).default_size(270.0).size_range(220.0..=420.0).frame(panel_frame()).show(root, |ui| {
        // (a road's rows of objects can make it long)
        egui::ScrollArea::vertical().id_salt("properties scroll").auto_shrink([false, false]).show(ui, |ui| {
            properties(ui, st, ed, lib, actions);
        });
    });
    let favourites = st.favourites.clone();
    st.assets.panel(root, ed, lib, thumbs, wanted, &favourites, actions);
    let mut view = (egui::Rect::NOTHING, false);
    egui::CentralPanel::default().frame(Frame::NONE).show(root, |ui| {
        let rect = ui.max_rect();
        let resp = ui.interact(rect, Id::new("viewport"), Sense::hover());
        view = (rect, resp.contains_pointer());
        ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
        if let (Some(doc), Some(cam)) = (&ed.doc, status.cam) {
            if let Some(p) = st.minimap.show(ui, rect, doc, lib, cam) {
                actions.push(Action::LookAt(p));
            }
        }
    });
    let ctx = root.ctx().clone();
    // the round 2D / 3D switch at the top right of the view
    if view.0.is_positive() {
        let at = egui::Pos2::new(view.0.right() - 46.0, view.0.top() + 12.0);
        egui::Area::new(Id::new("view 2d 3d")).fixed_pos(at).order(egui::Order::Foreground).show(&ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
            let p = ui.painter();
            p.circle_filled(rect.center(), 17.0, if resp.hovered() { Color32::from_rgb(70, 74, 82) } else { Color32::from_rgb(46, 49, 55) });
            p.circle_stroke(rect.center(), 17.0, Stroke::new(1.5, if st.view_2d { ACCENT } else { Color32::from_gray(150) }));
            p.text(rect.center(), egui::Align2::CENTER_CENTER, if st.view_2d { "3D" } else { "2D" }, egui::FontId::proportional(13.0), Color32::WHITE);
            if resp.on_hover_text(if st.view_2d { tr("Back to the 3D view (0)", "Вернуться в 3D (0)") } else { tr("The plan view from above, north up (0)", "Вид сверху, север вверху (0)") }).clicked() {
                st.view_2d = !st.view_2d;
            }
        });
    }
    texture_picker(&ctx, st, texture_files, swatches, wanted_swatches, actions);
    controls(&ctx, st, prefs, actions);
    about(&ctx, st);
    st.check.window(&ctx, ed, lib, actions);
    st.find.window(&ctx, ed, lib, actions);
    st.groups.window(&ctx, ed, lib);
    history_window(&ctx, st, ed, actions);
    st.merge.window(&ctx, ed, lib);
    crate::ui_palette::window(&ctx, st, ed, actions);
    if let Some(doc) = &ed.doc {
        crate::doc::safe::versions_window(&ctx, &mut st.versions_open, &doc.dir, actions);
    }
    view
}

#[allow(clippy::too_many_arguments)]
fn menu_bar(root: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, can_undo: bool, can_redo: bool, game_found: bool, prefs: &Prefs, actions: &mut Vec<Action>) {
    egui::Panel::top("menu").frame(Frame::new().fill(BG_DARK).inner_margin(Margin::symmetric(6, 2))).show(root, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("File", "Файл"), |ui| {
                if menu_item(ui, Some(Icon::NewFile), tr("New map…", "Новая карта…"), "", true) {
                    st.new_map = Some(Default::default());
                }
                if menu_item(ui, Some(Icon::Open), tr("Open map…", "Открыть карту…"), "", true) {
                    actions.push(Action::OpenDialog);
                }
                ui.menu_button(icons::labelled(Icon::Map, tr("Recent maps", "Недавние карты")), |ui| {
                    let recent: Vec<PathBuf> = prefs.recent.iter().map(PathBuf::from).filter(|p| p.is_file()).collect();
                    if recent.is_empty() {
                        ui.label(RichText::new(tr("(none)", "(нет)")).color(TEXT_DIM));
                    }
                    for p in recent {
                        let name = p.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        if ui.button(name).clicked() {
                            actions.push(Action::Open(p));
                        }
                    }
                });
                ui.separator();
                if menu_item(ui, Some(Icon::Save), tr("Save", "Сохранить"), "Ctrl+S", true) {
                    actions.push(Action::Save);
                }
                if menu_item(ui, Some(Icon::Copy), tr("Export map for sharing…", "Экспорт карты для обмена…"), "", true) {
                    actions.push(Action::Export);
                }
                if menu_item(ui, Some(Icon::Map), tr("Export a plan of the map…", "Экспорт плана карты…"), "", true) {
                    actions.push(Action::ExportPlan);
                }
                if menu_item(ui, Some(Icon::Import), tr("Import assets…", "Импорт ассетов…"), "", true) {
                    actions.push(Action::Import);
                }
                if menu_item(ui, Some(Icon::History), tr("Earlier versions…", "Прошлые версии…"), "", true) {
                    st.versions_open = true;
                }
                if menu_item(ui, Some(Icon::Walk), tr("Multiplayer…", "Совместная правка…"), "", true) {
                    actions.push(Action::Multiplayer);
                }
                ui.separator();
                if menu_item(ui, None, tr("Close map", "Закрыть карту"), "", true) {
                    actions.push(Action::Close);
                }
                if menu_item(ui, None, tr("Quit", "Выход"), "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button(tr("Edit", "Правка"), |ui| {
                let undo = match ed.history.undo_label() {
                    Some(l) => format!("{} «{l}»", tr("Undo", "Отменить")),
                    None => tr("Undo", "Отменить").to_string(),
                };
                if menu_item(ui, Some(Icon::Undo), &undo, "Ctrl+Z", can_undo) {
                    actions.push(Action::Undo);
                }
                if menu_item(ui, Some(Icon::Redo), tr("Redo", "Повторить"), "Ctrl+Shift+Z", can_redo) {
                    actions.push(Action::Redo);
                }
                ui.separator();
                let any = ed.selected.is_some();
                if menu_item(ui, None, tr("Cut", "Вырезать"), "Ctrl+X", any) {
                    actions.push(Action::Cut);
                }
                if menu_item(ui, Some(Icon::Copy), tr("Copy", "Копировать"), "Ctrl+C", any) {
                    actions.push(Action::Copy);
                }
                if menu_item(ui, Some(Icon::Paste), tr("Paste", "Вставить"), "Ctrl+V", ed.clip.is_some()) {
                    actions.push(Action::Paste);
                }
                if menu_item(ui, None, tr("Duplicate", "Дублировать"), "Ctrl+D", any) {
                    actions.push(Action::Duplicate);
                }
                if menu_item(ui, Some(Icon::History), tr("Edit history…", "История правок…"), "", true) {
                    st.history_open = true;
                }
                if menu_item(ui, Some(Icon::Objects), tr("Groups for other maps…", "Группы для других карт…"), "", true) {
                    st.groups.open = true;
                }
                if menu_item(ui, Some(Icon::Search), tr("Find in map…", "Поиск по карте…"), "Ctrl+F", true) {
                    st.find.show();
                }
                if menu_item(ui, Some(Icon::Search), tr("Search for a command…", "Поиск команды…"), "Ctrl+K", true) {
                    st.palette.show();
                }
                if menu_item(ui, Some(Icon::SelectAll), tr("Select all", "Выбрать всё"), "Ctrl+A", true) {
                    actions.push(Action::SelectAll);
                }
                ui.separator();
                if menu_item(ui, Some(Icon::Delete), tr("Delete", "Удалить"), "Del", ed.selected.is_some()) {
                    ed.delete_selected();
                }
                if menu_item(ui, None, tr("Deselect", "Снять выделение"), "Esc", ed.selected.is_some()) {
                    ed.select(None);
                }
            });
            ui.menu_button(tr("View", "Вид"), |ui| {
                check(ui, &mut ed.show_grid, tr("Tile grid", "Сетка тайлов"));
                let icon = if ed.show_technical { Icon::Visible } else { Icon::Hidden };
                check(ui, &mut ed.show_technical, icons::labelled(icon, tr("What the game does not show", "То, чего не видно в игре")));
                check(ui, &mut ed.show_lanes, icons::labelled(Icon::Route, tr("AI lanes", "Полосы ИИ")));
                check(ui, &mut ed.show_names, icons::labelled(Icon::Bus, tr("Names of stops and start points", "Названия остановок и точек старта")));
                ui.menu_button(icons::labelled(Icon::Layers, tr("Show", "Показывать")), |ui| {
                    // (what is hidden cannot be clicked either: handy in a dense town)
                    for (i, name, icon) in [(0, tr("Trees", "Деревья"), Icon::Tree), (1, tr("Objects", "Объекты"), Icon::Building), (2, tr("Roads and splines", "Дороги и сплайны"), Icon::Road)] {
                        let mut on = !ed.hide[i];
                        if check(ui, &mut on, icons::labelled(icon, name)).changed() {
                            ed.hide[i] = !on;
                        }
                    }
                });
                check(ui, &mut st.minimap.open, icons::labelled(Icon::Map, tr("Overview map (M)", "Обзорная карта (M)")));
                check(ui, &mut st.assets.open, icons::labelled(Icon::Objects, tr("Assets (B)", "Ассеты (B)")));
                crate::ui_geo::view_menu(ui, &mut st.geo);
                if menu_item(ui, Some(Icon::Terrain), tr("Flatten the terrain (roads follow)", "Убрать рельеф (дороги подстроятся)"), "", true) {
                    actions.push(Action::FlattenTerrain);
                }
                if menu_item(ui, Some(Icon::Flag), tr("Saved views…", "Сохранённые виды…"), "Ctrl+1…9", true) {
                    st.views.open = !st.views.open;
                }
                if menu_item(ui, Some(Icon::Focus), tr("Go to selection", "К выделенному"), "F", ed.selected.is_some()) {
                    actions.push(Action::FocusSelection);
                }
                if menu_item(ui, Some(Icon::Save), tr("Save a picture of the view", "Сохранить картинку вида"), "F12", true) {
                    actions.push(Action::ViewPicture);
                }
                ui.separator();
                lang_menu(ui, actions);
                ui.menu_button(tr("Camera", "Камера"), |ui| {
                    let unity = prefs.camera != "map";
                    if radio(ui, unity, tr("Unity: right button looks round and W A S D fly, middle pans, Alt orbits", "Unity: правая кнопка оглядывает и W A S D летят, средняя двигает, Alt вращает")).clicked() {
                        actions.push(Action::SetCamera("fly".into()));
                    }
                    if radio(ui, !unity, tr("Map editor: right button looks round and W A S D fly (Q down, E up), middle pans, the wheel zooms to the cursor, drag moves the map", "Редактор карт: правая кнопка оглядывает и W A S D летят (Q вниз, E вверх), средняя двигает, колесо приближает к курсору, перетаскивание двигает карту")).clicked() {
                        actions.push(Action::SetCamera("map".into()));
                    }
                });
                let mut streaming = prefs.tile_streaming == Some(true);
                if check(ui, &mut streaming, tr("Load 100 tiles at a time (like OMSI 2's editor)", "Загружать по 100 тайлов (как в редакторе OMSI 2)")).clicked() {
                    actions.push(Action::SetTileStreaming(streaming));
                }
                ui.menu_button(tr("Time of day", "Время суток"), |ui| {
                    if radio(ui, st.daytime.is_none(), tr("Plain day (as the game by day)", "Обычный день (как в игре днём)")).clicked() {
                        st.daytime = None;
                    }
                    if radio(ui, st.daytime.is_some(), tr("At an hour", "В заданный час")).clicked() && st.daytime.is_none() {
                        st.daytime = Some(18.0);
                    }
                    if let Some(h) = &mut st.daytime {
                        let r = ui.add(egui::Slider::new(h, 0.0..=24.0).step_by(0.25).custom_formatter(|v, _| format!("{:02}:{:02}", v as u32 % 24, ((v.fract() * 60.0).round() as u32) % 60)));
                        r.on_hover_text(tr("Where the sun stands then, today, at the map's place (Map > Calendar and place); by night the moon", "Где тогда стоит солнце, сегодня, в месте карты (Карта > Календарь и место); ночью луна"));
                    }
                });
                ui.menu_button(tr("Season", "Время года"), |ui| {
                    // (the texture folders OMSI keeps for them)
                    let seasons: [(Option<&'static str>, &str); 6] = [
                        (None, tr("Summer", "Лето")),
                        (Some("SummerDry"), tr("Dry summer", "Сухое лето")),
                        (Some("Spring"), tr("Spring", "Весна")),
                        (Some("Fall"), tr("Autumn", "Осень")),
                        (Some("Winter"), tr("Winter", "Зима")),
                        (Some("WinterSnow"), tr("Snow", "Снег")),
                    ];
                    for (f, name) in seasons {
                        if radio(ui, st.season == f, name).clicked() && st.season != f {
                            actions.push(Action::SetSeason(f));
                        }
                    }
                });
                ui.menu_button(tr("Graphics quality", "Качество графики"), |ui| {
                    let now = crate::render::quality::Quality::from_code(&prefs.graphics);
                    for q in crate::render::quality::Quality::ALL {
                        if radio(ui, now == q, q.label()).clicked() {
                            actions.push(Action::SetGraphics(q));
                        }
                    }
                });
            });
            ui.menu_button(tr("Tools", "Инструменты"), |ui| {
                for t in Tool::ALL {
                    if ui.add(egui::Button::new(icons::labelled(t.icon(), t.label())).shortcut_text(t.key()).selected(ed.tool == t)).clicked() {
                        ed.set_tool(t);
                    }
                }
                ui.separator();
                if menu_item(ui, Some(Icon::Road), tr("Road types and junctions…", "Типы дорог и перекрёстки…"), "", true) {
                    actions.push(Action::RoadTypes);
                }
                if menu_item(ui, Some(Icon::Layers), tr("PBR maps for a picture…", "PBR-карты для картинки…"), "", true) {
                    actions.push(Action::PbrMaker);
                }
            });
            ui.menu_button(tr("Map", "Карта"), |ui| {
                if menu_item(ui, Some(Icon::Settings), tr("Map settings…", "Настройки карты…"), "", true) {
                    actions.push(Action::MapSettings);
                }
                if menu_item(ui, Some(Icon::Schedule), tr("Calendar and place…", "Календарь и место…"), "", true) {
                    actions.push(Action::Calendar);
                }
                if menu_item(ui, Some(Icon::Traffic), tr("Signal routes…", "Маршруты сигналов…"), "", true) {
                    actions.push(Action::SignalRoutes);
                }
                if menu_item(ui, Some(Icon::History), tr("Chrono scenarios…", "Хроно-сценарии…"), "", true) {
                    actions.push(Action::Chrono);
                }
                if menu_item(ui, Some(Icon::Layers), tr("Ground textures…", "Текстуры земли…"), "", true) {
                    ed.set_tool(Tool::Paint);
                }
                if menu_item(ui, Some(Icon::CheckCircle), tr("Check map…", "Проверить карту…"), "", true) {
                    st.check.open = true;
                    st.check.issues = None;
                }
                if menu_item(ui, Some(Icon::Junction), tr("Make the junctions again", "Перестроить перекрёстки"), "", true) {
                    let n = ed.remake_all_junctions();
                    ed.toast(format!("{} {n}", tr("Junctions made again:", "Перекрёстков перестроено:")));
                }
                if menu_item(ui, Some(Icon::Bus), tr("Lines and timetable…", "Линии и расписание…"), "L", true) {
                    actions.push(Action::Timetable);
                }
                if menu_item(ui, Some(Icon::Traffic), tr("Traffic and AI…", "Трафик и ИИ…"), "T", true) {
                    actions.push(Action::Traffic);
                }
                if menu_item(ui, Some(Icon::Flag), tr("Depot files (destinations)…", "Файлы депо (конечные)…"), "", true) {
                    actions.push(Action::Depots);
                }
                if menu_item(ui, Some(Icon::Layers), tr("Satellite and terrain…", "Спутник и рельеф…"), "I", true) {
                    actions.push(Action::Geo);
                }
                if menu_item(ui, Some(Icon::Road), tr("From OpenStreetMap…", "Из OpenStreetMap…"), "", true) {
                    actions.push(Action::Osm);
                }
                if menu_item(ui, Some(Icon::Route), tr("GPS tracks to trace…", "GPS-треки для обводки…"), "", true) {
                    actions.push(Action::Trace);
                }
                if menu_item(ui, Some(Icon::Tiles), tr("Tiles from another map…", "Тайлы из другой карты…"), "", true) {
                    actions.push(Action::MergeTiles);
                }
                ui.separator();
                if menu_item(ui, Some(Icon::Play), tr("Test drive in openOMSI", "Тест-драйв в openOMSI"), "F5", game_found) {
                    actions.push(Action::Play);
                }
                play_here_items(ui, game_found, actions);
                if menu_item(ui, Some(Icon::Play), tr("Test drive in OMSI 2 (the original)…", "Тест-драйв в оригинальном OMSI 2…"), "", true) {
                    actions.push(Action::PlayOriginal);
                }
                if menu_item(ui, Some(Icon::CheckCircle), tr("Check in openOMSI…", "Проверка в openOMSI…"), "", game_found) {
                    actions.push(Action::GameCheck);
                }
            });
            ui.menu_button(tr("Help", "Справка"), |ui| {
                if menu_item(ui, Some(Icon::CheckCircle), tr("Getting started", "С чего начать"), "", true) {
                    actions.push(Action::Guide);
                }
                if menu_item(ui, Some(Icon::Import), tr("Updates…", "Обновления…"), "", true) {
                    actions.push(Action::Updates);
                }
                if menu_item(ui, Some(Icon::Info), tr("About openMapper", "О программе"), "", true) {
                    st.about_open = true;
                }
            });
        });
    });
}

fn toolbar(root: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, can_undo: bool, can_redo: bool, game_found: bool, actions: &mut Vec<Action>) {
    egui::Panel::top("toolbar").frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(6, 3)).stroke(Stroke::new(1.0, BORDER))).show(root, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let r = tool_button(ui, Icon::NewFile, false, true);
            help(st, &r, tr("New map", "Новая карта"));
            if r.clicked() {
                st.new_map = Some(Default::default());
            }
            let r = tool_button(ui, Icon::Open, false, true);
            help(st, &r, tr("Open a map", "Открыть карту"));
            if r.clicked() {
                actions.push(Action::OpenDialog);
            }
            let dirty = ed.doc.as_ref().map(|d| d.is_dirty()).unwrap_or(false);
            let r = tool_button(ui, Icon::Save, false, true);
            if dirty {
                ui.painter().circle_filled(r.rect.right_top() + Vec2::new(-6.0, 6.0), 3.0, ACCENT);
            }
            help(st, &r, if dirty { tr("Save (Ctrl+S): there are unsaved changes", "Сохранить (Ctrl+S): есть несохранённые изменения") } else { tr("Save (Ctrl+S)", "Сохранить (Ctrl+S)") });
            if r.clicked() {
                actions.push(Action::Save);
            }
            let r = tool_button(ui, Icon::Import, false, true);
            help(st, &r, tr("Import assets: a mod folder or .zip, a .obj / .x / .o3d model (or drop files on the window)", "Импорт ассетов: папка мода или .zip, модель .obj / .x / .o3d (или перетащите файлы в окно)"));
            if r.clicked() {
                actions.push(Action::Import);
            }
            ui.separator();
            let r = tool_button(ui, Icon::Undo, false, can_undo);
            help(st, &r, format!("{} (Ctrl+Z){}", tr("Undo", "Отменить"), ed.history.undo_label().map(|l| format!(": {l}")).unwrap_or_default()));
            if r.clicked() {
                actions.push(Action::Undo);
            }
            let r = tool_button(ui, Icon::Redo, false, can_redo);
            help(st, &r, tr("Redo (Ctrl+Shift+Z)", "Повторить (Ctrl+Shift+Z)"));
            if r.clicked() {
                actions.push(Action::Redo);
            }
            ui.separator();
            for t in Tool::ALL {
                let on = ed.tool == t && (t != Tool::Select || ed.xform == crate::editor::Xform::Select);
                let r = tool_button(ui, t.icon(), on, true);
                help(st, &r, format!("{} ({})", t.label(), t.key()));
                if r.clicked() {
                    ed.set_tool(t);
                    if t == Tool::Select {
                        ed.xform = crate::editor::Xform::Select;
                    }
                }
                // (the three that work on what is selected stand by the Select tool)
                if t == Tool::Select {
                    for x in crate::editor::Xform::ALL.into_iter().skip(1) {
                        let r = tool_button(ui, x.icon(), ed.tool == Tool::Select && ed.xform == x, true);
                        help(st, &r, x.label());
                        if r.clicked() {
                            ed.set_tool(Tool::Select);
                            ed.xform = if ed.xform == x { crate::editor::Xform::Select } else { x };
                        }
                    }
                }
            }
            ui.separator();
            let r = tool_button(ui, Icon::Tiles, ed.show_grid, true);
            help(st, &r, tr("Tile grid (G)", "Сетка тайлов (G)"));
            if r.clicked() {
                ed.show_grid = !ed.show_grid;
            }
            let r = tool_button(ui, Icon::Objects, st.assets.open, true);
            help(st, &r, tr("Assets: every object and road type, with pictures (B)", "Ассеты: все объекты и типы дорог с картинками (B)"));
            if r.clicked() {
                st.assets.open = !st.assets.open;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = ui.add_enabled(game_found, egui::Button::new(icons::labelled(Icon::Play, tr("Test drive", "Тест-драйв"))));
                help(st, &r, if game_found { tr("Save the map and drive it in openOMSI (F5)", "Сохранить карту и проехать её в openOMSI (F5)") } else { tr("openOMSI was not found: install it to test maps", "openOMSI не найден: установите его, чтобы проверять карты") });
                if r.clicked() {
                    actions.push(Action::Play);
                }
                r.context_menu(|ui| play_here_items(ui, game_found, actions));
                if let Some(d) = &ed.doc {
                    ui.label(RichText::new(d.display_name()).color(TEXT_DIM));
                }
            });
        });
    });
}

/// Test drives from somewhere else than the first start point (Map menu, and a right-click
/// on Test drive).
fn play_here_items(ui: &mut egui::Ui, game_found: bool, actions: &mut Vec<Action>) {
    if menu_item(ui, Some(Icon::Flag), tr("Test drive from the start point nearest the view", "Тест-драйв с ближайшей к виду точки старта"), "Shift+F5", game_found) {
        actions.push(Action::PlayHere(false));
    }
    if menu_item(ui, Some(Icon::Visible), tr("See this view in openOMSI", "Посмотреть этот вид в openOMSI"), "", game_found) {
        actions.push(Action::PlayHere(true));
    }
}

fn status_bar(root: &mut egui::Ui, help_line: Option<String>, ed: &mut Editor, status: &Status) {
    ed.toasts.retain(|t| t.at.elapsed().as_secs_f32() < 8.0);
    egui::Panel::bottom("status").exact_size(22.0).frame(Frame::new().fill(BG_DARK).inner_margin(Margin::symmetric(8, 3))).show(root, |ui| {
        ui.horizontal(|ui| {
            // the help of what the mouse is over, else the newest message, else the tool's
            let (text, color) = match (ed.toasts.last(), help_line) {
                (_, Some(h)) => (h, Color32::from_gray(210)),
                (Some(t), None) => (t.text.clone(), if t.error { Color32::from_rgb(240, 120, 100) } else { Color32::from_rgb(150, 210, 140) }),
                (None, None) => (String::new(), TEXT_DIM),
            };
            // (the facts on the right first; the help gets what is left, cut short)
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&status.gpu).small().color(Color32::from_gray(95)));
                ui.separator();
                if status.loading > 0 {
                    ui.label(RichText::new(format!("{} {}", tr("loading", "загрузка"), status.loading)).small().color(TEXT_DIM));
                    ui.spinner();
                    ui.separator();
                }
                if let Some(doc) = &ed.doc {
                    let objects: usize = doc.tiles.values().map(|t| t.tile.objects.len()).sum();
                    let splines: usize = doc.tiles.values().map(|t| t.tile.splines.len()).sum();
                    ui.label(RichText::new(format!("{} {} · {} {} · {} {}", doc.tiles.len(), tr("tiles", "тайлов"), splines, tr("splines", "сплайнов"), objects, tr("objects", "объектов"))).small().color(TEXT_DIM));
                    ui.separator();
                }
                if let Some(r) = ed.drag_readout() {
                    ui.label(RichText::new(r).small().strong().color(ACCENT));
                    ui.separator();
                }
                if let Some(c) = status.cursor {
                    let t = crate::doc::tile_of(c.truncate());
                    ui.label(RichText::new(format!("X {:>8.1}   Y {:>8.1}   Z {:>6.1}   {} {},{}", c.x, c.y, c.z, tr("tile", "тайл"), t.0, t.1)).small().monospace().color(TEXT_DIM));
                }
                if ed.doc.is_some() {
                    ui.separator();
                    crate::ui_edit::snap_controls(ui, ed);
                }
                ui.separator();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let r = ui.add(egui::Label::new(RichText::new(&text).small().color(color)).truncate());
                    r.on_hover_text(&text);
                });
            });
        });
    });
}

// ---------------------------------------------------------------------------- left panel

#[allow(clippy::too_many_arguments)]
fn tool_panel(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ed: &mut Editor,
    lib: &Library,
    thumbs: &HashMap<String, Option<Thumb>>,
    wanted: &mut Vec<(String, bool)>,
    swatches: &HashMap<String, egui::TextureHandle>,
    wanted_swatches: &mut Vec<String>,
    actions: &mut Vec<Action>,
) {
    if ed.mesh.as_ref().map(|m| m.editing).unwrap_or(false) {
        section(ui, tr("Edit mesh", "Редактирование меша"));
        Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| crate::ui_mesh::edit_panel(ui, ed, &mut st.uv_open, actions));
        return;
    }
    section(ui, ed.tool.label());
    Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| match ed.tool {
        Tool::Roads => catalogue(ui, st, ed, lib, thumbs, wanted, Kind::Spline, actions),
        Tool::Objects => {
            egui::CollapsingHeader::new(RichText::new(tr("Shapes and lamps", "Фигуры и светильники")).strong()).id_salt("shapes").default_open(false).show(ui, |ui| {
                crate::ui_mesh::primitives_grid(ui, actions);
                crate::ui_mesh::lamp_makers(ui, &mut st.signal_lamps, actions);
                crate::ui_mesh::extra_makers(ui, &mut st.makers, actions);
            });
            ui.add_space(4.0);
            catalogue(ui, st, ed, lib, thumbs, wanted, Kind::Object, actions)
        }
        Tool::Logic => logic_panel(ui, st, ed, lib, thumbs, wanted, actions),
        Tool::Terrain => terrain_panel(ui, st, ed, actions),
        Tool::Paint => paint_panel(ui, st, ed, swatches, wanted_swatches, actions),
        Tool::Tiles => {
            let n = ed.doc.as_ref().map(|d| d.tiles.len()).unwrap_or(0);
            ui.label(tr("Click beside the map to add a 300 × 300 m tile. Right-click a tile to remove it.", "Клик рядом с картой: добавить тайл 300 × 300 м. Правый клик по тайлу: удалить."));
            ui.add_space(6.0);
            egui::Grid::new("tiles").num_columns(2).show(ui, |ui| {
                ui.label(RichText::new(tr("Tiles", "Тайлов")).color(TEXT_DIM));
                ui.label(n.to_string());
                ui.end_row();
                ui.label(RichText::new(tr("Area", "Площадь")).color(TEXT_DIM));
                ui.label(format!("{:.2} km²", n as f64 * crate::doc::tile_size().powi(2) / 1e6));
                ui.end_row();
            });
            ui.add_space(4.0);
            if ui.button(icons::labelled(Icon::Add, tr("A ring of tiles round the map", "Кольцо тайлов вокруг карты"))).on_hover_text(tr("Every free place beside the map, corners too, gets a tile at the height of its neighbours", "Каждое свободное место рядом с картой, и углы тоже, получает тайл на высоте соседей")).clicked() {
                let added = ed.grow_map();
                ed.toast(format!("{} {added}", tr("Tiles added:", "Тайлов добавлено:")));
            }
            water_panel(ui, ed);
            crate::ui_geo::tiles_panel(ui, &mut st.geo);
        }
        Tool::Select => {
            // (two tabs: the tools, and everything on the map by folder)
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(icons::labelled(Icon::Select, tr("Tools", "Инструменты"))).selected(!st.outliner.open).min_size(Vec2::new(0.0, 26.0))).clicked() {
                    st.outliner.open = false;
                }
                if ui.add(egui::Button::new(icons::labelled(Icon::Layers, tr("Objects", "Объекты"))).selected(st.outliner.open).min_size(Vec2::new(0.0, 26.0))).on_hover_text(tr("Everything on the map, grouped by the folders its files lie in", "Всё, что есть на карте, по папкам файлов")).clicked() {
                    st.outliner.open = true;
                }
            });
            ui.add_space(6.0);
            if st.outliner.open {
                st.outliner.panel(ui, ed, lib, actions);
                return;
            }
            ui.label(RichText::new(tr("Click an object or a road in the view to see and change it on the right.", "Кликните объект или дорогу в окне вида: свойства появятся справа.")).color(TEXT_DIM));
            ui.add_space(8.0);
            for t in Tool::ALL.iter().skip(1) {
                if ui.add(egui::Button::new(icons::labelled(t.icon(), t.label())).shortcut_text(t.key()).min_size(Vec2::new(ui.available_width(), 24.0))).clicked() {
                    ed.set_tool(*t);
                }
            }
        }
    });
}

fn catalogue(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, lib: &Library, thumbs: &HashMap<String, Option<Thumb>>, wanted: &mut Vec<(String, bool)>, kind: Kind, actions: &mut Vec<Action>) {
    catalogue_of(ui, st, ed, lib, thumbs, wanted, kind, None, actions)
}

/// The Logic tool: what the game uses but does not show, a tab for each kind.
fn logic_panel(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, lib: &Library, thumbs: &HashMap<String, Option<Thumb>>, wanted: &mut Vec<(String, bool)>, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for m in LogicMode::ALL {
            let on = ed.logic == m;
            let b = egui::Button::new(icons::labelled(m.icon(), m.label())).selected(on).min_size(Vec2::new(0.0, 26.0));
            if ui.add(b).clicked() {
                ed.set_logic(m);
            }
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new(ed.logic.hint()).small().color(TEXT_DIM));
    // (stops, start points and car parks go to the kerb or the lane by themselves: switchable)
    if matches!(ed.logic, LogicMode::Stops | LogicMode::Start | LogicMode::Parking) {
        check(ui, &mut ed.align_to_road, tr("Line up with the nearest road (Shift: put it freely)", "Выравнивать по ближайшей дороге (Shift: ставить свободно)"));
    }
    ui.add_space(4.0);
    match ed.logic {
        LogicMode::Stops => {
            check(ui, &mut ed.stop_both_sides, tr("On both sides of the road", "С обеих сторон дороги")).on_hover_text(tr("A stop of the same name also goes at the kerb across the road, facing the other way", "Остановка с тем же названием ставится и у обочины напротив, в другую сторону"));
        }
        LogicMode::Parking => {
            check(ui, &mut ed.parking_row, tr("In a row along a line", "Рядом вдоль линии"));
            if ed.parking_row {
                ui.horizontal(|ui| {
                    ui.label(tr("Every", "Через каждые"));
                    ui.add(egui::DragValue::new(&mut ed.parking_step).range(2.0..=30.0).speed(0.1).suffix(" m"));
                });
            }
        }
        LogicMode::Paths => {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(icons::labelled(Icon::Add, tr("Draw new", "Рисовать новые"))).selected(!ed.path_edit)).clicked() && ed.path_edit {
                    ed.cancel();
                    ed.path_edit = false;
                }
                if ui.add(egui::Button::new(icons::labelled(Icon::Select, tr("Edit existing", "Править готовые"))).selected(ed.path_edit)).clicked() && !ed.path_edit {
                    ed.cancel();
                    ed.path_edit = true;
                    ed.select(None);
                }
            });
            if ed.path_edit {
                ui.label(RichText::new(tr("Click a path to pick it: every point is a dot to drag (the small ones in the middle of a piece bend it). Double-click the path to put a point in. Click a dot, then Del, to take that point out; Del with no dot picked deletes the whole path (Ctrl+Z brings it back). Lanes of roads and junctions belong to their road type: use Edit its AI lanes on the road.", "Клик по пути выбирает его: каждая точка это кружок, который можно тянуть (маленькие в середине куска изгибают его). Двойной клик по пути ставит точку. Клик по кружку, затем Del: убрать эту точку; Del без выбранной точки удаляет весь путь (Ctrl+Z вернёт). Полосы дорог и перекрёстков принадлежат типу дороги: используйте «Править полосы ИИ» у дороги.")).small().color(TEXT_DIM));
            }
            // the two everyone needs, at the top
            ui.horizontal(|ui| {
                for (icon, label, file) in [(Icon::Traffic, tr("Cars", "Машины"), "Splines\\Marcel\\invis_street.sli"), (Icon::Walk, tr("People", "Люди"), "Splines\\invis_sidewalk.sli")] {
                    let Some(a) = lib.find(file) else { continue };
                    let on = ed.path_type.as_deref().map(|p| p.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
                    if ui.add(egui::Button::new(icons::labelled(icon, label)).selected(on).min_size(Vec2::new(90.0, 28.0))).clicked() {
                        ed.set_path_type(a.rel.clone());
                    }
                }
            });
            if ed.path_type.is_none() {
                if let Some(a) = lib.find("Splines\\Marcel\\invis_street.sli") {
                    ed.set_path_type(a.rel.clone());
                }
            }
        }
        _ => {}
    }
    check(ui, &mut ed.show_technical, tr("Show these in the other tools too", "Показывать это и в других инструментах"));
    check(ui, &mut ed.show_lanes, tr("Show the AI's lanes in the other tools too", "Показывать полосы ИИ и в других инструментах"));
    ui.add_space(4.0);
    let kind = if ed.logic == LogicMode::Paths { Kind::Spline } else { Kind::Object };
    catalogue_of(ui, st, ed, lib, thumbs, wanted, kind, Some(ed.logic), actions);
}

/// The asset list (with a Logic tab: only what belongs to it, and no category choice).
#[allow(clippy::too_many_arguments)]
fn catalogue_of(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, lib: &Library, thumbs: &HashMap<String, Option<Thumb>>, wanted: &mut Vec<(String, bool)>, kind: Kind, logic: Option<LogicMode>, actions: &mut Vec<Action>) {
    let belongs = |a: &crate::library::Asset| -> bool {
        match logic {
            None => true,
            Some(LogicMode::Start) => a.is_entry_point,
            Some(LogicMode::Stops) => a.is_bus_stop,
            Some(LogicMode::Parking) => a.is_car_park,
            Some(LogicMode::Paths) => a.category == Category::Technical && (a.lanes.0 > 0 || a.lanes.1 > 0),
            Some(LogicMode::Other) => a.category == Category::Technical && !a.is_car_park,
        }
    };
    let cats: &[Category] = if kind == Kind::Spline { &Category::SPLINES } else { &Category::OBJECTS };
    let cat = if kind == Kind::Spline { &mut st.spline_cat } else { &mut st.object_cat };
    let mut none: Option<Category> = None;
    let cat = if logic.is_some() { &mut none } else { cat };
    egui::Grid::new("filter").num_columns(2).spacing(Vec2::new(6.0, 4.0)).show(ui, |ui| {
        if logic.is_none() {
        ui.label(tr("Show", "Показать"));
        let current = cat.map(|c| c.label()).unwrap_or(tr("Everything", "Всё"));
        egui::ComboBox::from_id_salt(("cat", kind == Kind::Spline)).icon(combo_icon).width(ui.available_width()).selected_text(current).show_ui(ui, |ui| {
            ui.selectable_value(cat, None, tr("Everything", "Всё"));
            for c in cats {
                let n = lib.count(kind, *c);
                if n > 0 {
                    ui.selectable_value(cat, Some(*c), icons::labelled(c.icon(), &format!("{}  ({n})", c.label())));
                }
            }
        });
        ui.end_row();
        }
        ui.label(tr("Search", "Поиск"));
        ui.add(egui::TextEdit::singleline(&mut st.search).desired_width(f32::INFINITY));
        ui.end_row();
    });
    if logic.is_some() {
    } else if kind == Kind::Spline {
        if ui.button(icons::labelled(Icon::Settings, tr("Road types and junctions…", "Типы дорог и перекрёстки…"))).clicked() {
            actions.push(Action::RoadTypes);
        }
        check(ui, &mut ed.follow_ground, tr("Shape the ground under the road", "Подгонять землю под дорогу"));
        crate::ui_edit::road_options(ui, ed);
        check(ui, &mut ed.keep_drawing, tr("Go on drawing from the end of the road just built", "Продолжать рисовать с конца построенной дороги"));
        check(ui, &mut ed.snap_roads, tr("Join road ends, roads and junction pieces near the cursor", "Пристыковывать к концам дорог, дорогам и деталям перекрёстков у курсора"));
        check(ui, &mut ed.auto_junctions, tr("Make junctions where roads meet", "Делать перекрёстки, где дороги встречаются"));
        check(ui, &mut ed.inherit_type, tr("Go on with the type of the road I start from", "Продолжать типом дороги, от которой начинаю"));
        let current = ed.road_type.as_deref().map(|r| lib.find(r).map(|a| a.name.clone()).unwrap_or_else(|| r.to_string()));
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Building with:", "Строится:")).color(TEXT_DIM));
            ui.label(RichText::new(current.unwrap_or_else(|| tr("nothing chosen", "ничего не выбрано").to_string())).strong().color(Color32::WHITE));
        });
    } else {
        check(ui, &mut ed.align_to_road, tr("Line up with the nearest road (Shift: put it freely)", "Выравнивать по ближайшей дороге (Shift: ставить свободно)"));
        if ed.align_to_road {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                check(ui, &mut ed.align_front, tr("Front at the roadside, back", "Фасадом к дороге, отступ"));
                ui.add_enabled(ed.align_front, egui::DragValue::new(&mut ed.road_setback).range(0.0..=30.0).speed(0.1).suffix(" m"));
            });
        }
        // the plant brush
        check(ui, &mut ed.plant_brush, icons::labelled(Icon::Tree, tr("Plant brush: sow trees by painting", "Кисть растений: сажать деревья кистью")));
        if ed.plant_brush {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                if radio(ui, !ed.plant_erase, tr("Sow", "Сажать")).clicked() {
                    ed.plant_erase = false;
                }
                if radio(ui, ed.plant_erase, tr("Clear", "Убирать")).clicked() {
                    ed.plant_erase = true;
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                if radio(ui, !ed.plant_area, tr("With the brush", "Кистью")).clicked() {
                    ed.plant_area = false;
                    ed.plant_outline.clear();
                }
                if radio(ui, ed.plant_area, tr("In an outline", "По контуру")).on_hover_text(tr("Click round an area (a wood, a park, a garden) and it is filled at once, or cleared", "Обведите участок (лес, парк, сад) кликами: он засаживается сразу целиком или очищается")).clicked() {
                    ed.plant_area = true;
                }
            });
            if let Some(n) = ed.plant_outline_estimate() {
                ui.label(RichText::new(format!("{} {n} {}", tr("About", "Около"), tr("plants (Enter: plant)", "растений (Enter: посадить)"))).small().color(TEXT_DIM));
            }
            egui::Grid::new("plants").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
                if !ed.plant_area {
                    ui.label(tr("Size", "Размер"));
                    ui.add(egui::Slider::new(&mut ed.brush_radius, 3.0..=150.0).suffix(" m"));
                    ui.end_row();
                }
                ui.label(tr("Apart", "Расстояние"));
                ui.add(egui::Slider::new(&mut ed.plant_spacing, 1.0..=40.0).suffix(" m"));
                ui.end_row();
            });
            if ed.plant_mix.is_empty() {
                ui.label(RichText::new(tr("Sows the object chosen below. Right-click entries: \"Add to the plant mix\" to sow several at random. Clearing takes trees and the mix's objects.", "Сажает объект, выбранный ниже. Правый клик по пунктам: «Добавить в смесь», чтобы сажать несколько вперемешку. Кисть «Убирать» убирает деревья и объекты смеси.")).small().color(TEXT_DIM));
            } else {
                ui.label(RichText::new(tr("Mix (picked at random):", "Смесь (вперемешку):")).small().color(TEXT_DIM));
                let mut drop = None;
                for (i, rel) in ed.plant_mix.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(lib.find(rel).map(|a| a.name.clone()).unwrap_or_else(|| rel.clone())).small());
                        if ui.small_button("×").clicked() {
                            drop = Some(i);
                        }
                    });
                }
                if let Some(i) = drop {
                    ed.plant_mix.remove(i);
                }
            }
        }
    }
    let selected = if logic == Some(LogicMode::Paths) { ed.path_type.clone() } else if kind == Kind::Spline { ed.road_type.clone() } else { ed.object_type.clone() };
    let items: Vec<&crate::library::Asset> = lib.filter(kind, *cat, &st.search).filter(|a| belongs(a)).collect();
    ui.label(RichText::new(if lib.scanning { tr("reading OMSI 2…", "чтение OMSI 2…").to_string() } else { format!("{} {}", items.len(), tr("items", "шт.")) }).small().color(TEXT_DIM));
    let avail = ui.available_height();
    let searching = !st.search.trim().is_empty();
    Frame::new().fill(BG_FIELD).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
        ui.set_min_height(avail - 6.0);
        ui.set_width(ui.available_width());
        if items.is_empty() {
            ui.label(RichText::new(tr("Nothing found", "Ничего не найдено")).color(TEXT_DIM));
            return;
        }
        let mut row = |ui: &mut egui::Ui, a: &crate::library::Asset, i: usize, st: &mut UiState, ed: &mut Editor| {
            let h = 40.0;
            let on = selected.as_deref().map(|s| s.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
            let fill = if on {
                SELECT
            } else if resp.hovered() {
                Color32::from_gray(40)
            } else if i % 2 == 1 {
                Color32::from_rgb(26, 27, 30)
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
            let img = egui::Rect::from_min_size(rect.min + Vec2::new(3.0, 2.0), Vec2::splat(h - 4.0));
            match thumbs.get(&a.rel.to_lowercase()) {
                Some(Some(t)) => {
                    egui::Image::new((t.id, img.size())).paint_at(ui, img);
                }
                Some(None) => {
                    icons::paint(ui.painter(), a.category.icon(), img.center(), 20.0, Color32::from_gray(90));
                }
                None => {
                    wanted.push((a.rel.clone(), kind == Kind::Spline));
                    ui.painter().rect_filled(img.shrink(11.0), CornerRadius::same(2), Color32::from_gray(40));
                }
            }
            ui.painter().text(rect.min + Vec2::new(h + 6.0, 12.0), egui::Align2::LEFT_CENTER, &a.name, egui::FontId::proportional(13.0), Color32::from_gray(230));
            let starred = st.favourites.iter().any(|f| f.eq_ignore_ascii_case(&a.rel));
            if starred {
                icons::paint(ui.painter(), Icon::Star, rect.right_center() - Vec2::new(14.0, 0.0), 15.0, ACCENT);
            }
            let mut sub = a.category.label().to_string();
            if searching && !a.dir.is_empty() {
                sub = format!("{} · {}", a.dir, sub);
            }
            if a.from_content {
                sub.push_str(&format!(" · {}", tr("mod", "мод")));
            }
            ui.painter().text(rect.min + Vec2::new(h + 6.0, 28.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(11.0), TEXT_DIM);
            if resp.hovered() {
                st.status_help = Some(format!("{} · {}", a.rel, tr("right-click: star it", "правый клик: в избранное")));
            }
            resp.context_menu(|ui| {
                if kind == Kind::Object && logic.is_none() {
                    let in_mix = ed.plant_mix.iter().any(|m| m.eq_ignore_ascii_case(&a.rel));
                    let text = if in_mix { tr("Take out of the plant mix", "Убрать из смеси растений") } else { tr("Add to the plant mix", "Добавить в смесь растений") };
                    if ui.button(icons::labelled(Icon::Tree, text)).clicked() {
                        if in_mix {
                            ed.plant_mix.retain(|m| !m.eq_ignore_ascii_case(&a.rel));
                        } else {
                            ed.plant_mix.push(a.rel.clone());
                            ed.plant_brush = true;
                        }
                        ui.close();
                    }
                }
                let text = if starred { tr("Remove from favourites", "Убрать из избранного") } else { tr("Add to favourites", "Добавить в избранное") };
                if ui.button(icons::labelled(Icon::Star, text)).clicked() {
                    actions.push(Action::ToggleFavourite(a.rel.clone()));
                    ui.close();
                }
            });
            if resp.clicked() {
                actions.push(Action::UsedAsset(a.rel.clone()));
                if logic == Some(LogicMode::Paths) {
                    ed.set_path_type(a.rel.clone());
                } else if kind == Kind::Spline {
                    ed.road_type = Some(a.rel.clone());
                } else {
                    ed.object_type = Some(a.rel.clone());
                    ed.place_heading = 0.0;
                    let hint = crate::editor::asset_hint(a);
                    if !hint.is_empty() {
                        ed.toast(hint);
                    }
                }
            }
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if searching {
                // a search lists what it finds, wherever it lies
                for (i, a) in items.iter().take(400).enumerate() {
                    row(ui, a, i, st, ed);
                }
                return;
            }
            // starred and lately used ones first, among what the filter shows
            for (title, icon, list) in [(tr("Favourites", "Избранное"), Icon::Star, &st.favourites.clone()), (tr("Recently used", "Недавние"), Icon::History, &st.recent_assets.clone())] {
                let picked: Vec<&crate::library::Asset> = list.iter().filter_map(|r| items.iter().find(|a| a.rel.eq_ignore_ascii_case(r)).copied()).take(12).collect();
                if picked.is_empty() {
                    continue;
                }
                let head = icons::labelled(icon, &format!("{title}   ({})", picked.len()));
                egui::CollapsingHeader::new(head).icon(group_icon).id_salt(("cat-pinned", kind == Kind::Spline, title)).default_open(true).show(ui, |ui| {
                    for (i, a) in picked.iter().enumerate() {
                        row(ui, a, i, st, ed);
                    }
                });
            }
            // the folders as they are on disk: the first level, then what lies below it
            let mut tree: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<&crate::library::Asset>>> = std::collections::BTreeMap::new();
            for a in &items {
                let (top, rest) = match a.dir.split_once('\\') {
                    Some((t, r)) => (t.to_string(), r.to_string()),
                    None => (if a.dir.is_empty() { tr("(not in a folder)", "(вне папок)").to_string() } else { a.dir.clone() }, String::new()),
                };
                tree.entry(top).or_default().entry(rest).or_default().push(a);
            }
            let open_all = tree.len() <= 2;
            for (top, subs) in &tree {
                let n: usize = subs.values().map(|v| v.len()).sum();
                egui::CollapsingHeader::new(RichText::new(format!("{top}   ({n})")).strong()).icon(group_icon).id_salt(("cat-dir", kind == Kind::Spline, top)).default_open(open_all).show(ui, |ui| {
                    for (sub, list) in subs {
                        let mut draw = |ui: &mut egui::Ui| {
                            for (i, a) in list.iter().enumerate() {
                                row(ui, a, i, st, ed);
                            }
                        };
                        if sub.is_empty() {
                            draw(ui);
                        } else {
                            egui::CollapsingHeader::new(format!("{sub}   ({})", list.len())).icon(group_icon).id_salt(("cat-sub", kind == Kind::Spline, top, sub)).show(ui, |ui| draw(ui));
                        }
                    }
                });
            }
        });
    });
}

fn brush_grid(ui: &mut egui::Ui, ed: &mut Editor) {
    egui::Grid::new("brush").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
        ui.label(tr("Size", "Размер"));
        ui.add(egui::Slider::new(&mut ed.brush_radius, 3.0..=150.0).suffix(" m").logarithmic(true));
        ui.end_row();
        ui.label(tr("Strength", "Сила"));
        ui.add(egui::Slider::new(&mut ed.brush_strength, 0.05..=1.0).fixed_decimals(2));
        ui.end_row();
    });
}

fn terrain_panel(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, actions: &mut Vec<Action>) {
    ui.label(RichText::new(tr("Mode", "Режим")).strong());
    for m in TerrainMode::ALL {
        let r = ui.add(egui::Button::new(icons::labelled(m.icon(), m.label())).selected(ed.terrain_mode == m).min_size(Vec2::new(ui.available_width(), 24.0)));
        if r.clicked() {
            ed.terrain_mode = m;
        }
        let h = match m {
            TerrainMode::Flatten => tr("Levels the ground to the height where you start dragging", "Выравнивает землю до высоты точки начала"),
            TerrainMode::Smooth => tr("Softens bumps and edges", "Смягчает неровности и края"),
            TerrainMode::Raise => tr("Raises the ground under the brush", "Поднимает землю под кистью"),
            TerrainMode::Lower => tr("Lowers the ground under the brush", "Опускает землю под кистью"),
            TerrainMode::Roughen => tr("Gives flat ground the rolls and dips of a natural field; hold longer for more", "Придаёт ровной земле бугры и впадины, как в поле; держите дольше: сильнее"),
            TerrainMode::Water => tr("Paint a lake or a river: the ground is dug to a bed under the water's level (where you start the stroke, or the tile's water) and the tile gets water at that level. Water in OMSI is one level for a whole tile: another low place on the same tile shows it too", "Рисуйте озеро или реку: земля копается до дна под уровнем воды (там, где начали мазок, или водой тайла), и тайл получает воду на этом уровне. Вода в OMSI одного уровня на весь тайл: другое низкое место на том же тайле тоже её покажет"),
            TerrainMode::Ramp => tr("Click where it starts, then where it ends: the ground between becomes one even slope as wide as the brush (an embankment, a ramp, a level bed)", "Клик в начале, затем в конце: земля между ними станет ровным склоном шириной с кисть (насыпь, пандус, ровное полотно)"),
        };
        help(st, &r, h);
    }
    if ed.terrain_mode == TerrainMode::Flatten {
        ui.horizontal(|ui| {
            let mut fixed = ed.flatten_fixed.is_some();
            let r = check(ui, &mut fixed, tr("To a height", "До высоты"));
            help(st, &r, tr("Flatten to the height typed instead of the one where the drag starts (a level square, a platform)", "Выравнивать до введённой высоты, а не до высоты начала (ровная площадь, платформа)"));
            if r.changed() {
                ed.flatten_fixed = fixed.then(|| ed.cursor.map(|c| c.z as f32).unwrap_or(0.0));
            }
            if let Some(h) = &mut ed.flatten_fixed {
                ui.add(egui::DragValue::new(h).speed(0.1).fixed_decimals(2).suffix(" m"));
            }
        });
    }
    if ed.terrain_mode == TerrainMode::Water {
        ui.horizontal(|ui| {
            ui.label(tr("Depth", "Глубина"));
            ui.add(egui::DragValue::new(&mut ed.water_depth).speed(0.05).range(0.2..=20.0).fixed_decimals(1).suffix(" m"));
        });
        ui.label(RichText::new(tr("The water's level is set where a stroke starts: start at the bank.", "Уровень воды берётся там, где начат мазок: начинайте у берега.")).small().color(TEXT_DIM));
    }
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Brush", "Кисть")).strong());
    brush_grid(ui, ed);
    ui.add_space(6.0);
    if ui.button(icons::labelled(Icon::Terrain, tr("Flatten the whole terrain", "Убрать рельеф со всей карты"))).on_hover_text(tr("Every tile's ground levelled to the map's mean height; the roads, the AI paths and the junctions follow it. One undo step brings the relief back. (The satellite picture: View > Satellite overlay.)", "Земля всех тайлов выравнивается до средней высоты карты; дороги, пути ИИ и перекрёстки подстраиваются. Один шаг отмены вернёт рельеф. (Спутниковая картинка: Вид > Спутниковая подложка.)")).clicked() {
        actions.push(Action::FlattenTerrain);
    }
    st.heightmap.panel(ui, ed);
}

fn paint_panel(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, swatches: &HashMap<String, egui::TextureHandle>, wanted: &mut Vec<String>, actions: &mut Vec<Action>) {
    let Some(doc) = ed.doc.as_mut() else { return };
    ui.label(RichText::new(tr("Ground layers", "Слои земли")).strong());
    let mut remove: Option<usize> = None;
    let mut move_layer: Option<bool> = None;
    let n = doc.global.ground_textures.len();
    Frame::new().fill(BG_FIELD).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in 0..n {
            let g = &doc.global.ground_textures[i];
            let on = ed.paint_layer == i;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, if on { SELECT } else if resp.hovered() { Color32::from_gray(40) } else { Color32::TRANSPARENT });
            let sw = egui::Rect::from_min_size(rect.min + Vec2::new(3.0, 3.0), Vec2::splat(32.0));
            match swatches.get(&g.texture.to_lowercase()) {
                Some(h) => {
                    egui::Image::new((h.id(), sw.size())).paint_at(ui, sw);
                }
                None => wanted.push(g.texture.clone()),
            }
            let name = std::path::Path::new(&g.texture.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            ui.painter().text(rect.min + Vec2::new(42.0, 12.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), Color32::from_gray(230));
            let sub = if i == 0 { tr("base ground", "основа").to_string() } else { format!("{} {}", tr("layer", "слой"), i) };
            ui.painter().text(rect.min + Vec2::new(42.0, 27.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(11.0), TEXT_DIM);
            if resp.hovered() {
                st.status_help = Some(g.texture.clone());
            }
            if resp.clicked() {
                ed.paint_layer = i;
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button(icons::labelled(Icon::Add, tr("Add…", "Добавить…"))).clicked() {
            st.texture_picker = Some(String::new());
            st.texture_pick_for = None;
        }
        if ui.add_enabled(ed.paint_layer > 0, egui::Button::new(icons::labelled(Icon::Delete, tr("Remove", "Убрать")))).clicked() {
            remove = Some(ed.paint_layer);
        }
        // (the order they are drawn in: a later layer over an earlier one)
        let l = ed.paint_layer;
        if ui.add_enabled(l > 1, egui::Button::new(Icon::Raise.text(14.0))).on_hover_text(tr("Up the list: drawn under the layers after it", "Выше по списку: рисуется под следующими слоями")).clicked() {
            move_layer = Some(false);
        }
        if ui.add_enabled(l > 0 && l + 1 < n, egui::Button::new(Icon::Lower.text(14.0))).on_hover_text(tr("Down the list: drawn over the layers before it", "Ниже по списку: рисуется поверх предыдущих слоёв")).clicked() {
            move_layer = Some(true);
        }
    });
    let mut detail: Option<f32> = None;
    if let Some(g) = doc.global.ground_textures.get_mut(ed.paint_layer) {
        let before = g.params;
        let size = crate::doc::mask_size(g);
        let file_name = |t: &str| t.rsplit('\\').next().filter(|n| !n.is_empty()).unwrap_or("-").to_string();
        egui::Grid::new("layer").num_columns(2).show(ui, |ui| {
            for (detail, name, hint) in [
                (false, tr("Texture", "Текстура"), tr("The layer's picture; its painting stays", "Картинка слоя; его покраска остаётся")),
                (true, tr("Detail texture", "Детальная текстура"), tr("A fine grain laid over the texture, seen close up (OMSI's detail texture)", "Мелкая фактура поверх текстуры, видна вблизи (детальная текстура OMSI)")),
            ] {
                ui.label(name).on_hover_text(hint);
                let now = if detail { &g.detail_texture } else { &g.texture };
                if ui.add(egui::Button::new(file_name(now)).truncate()).on_hover_text(format!("{now}\n{}", tr("Click to choose another", "Щёлкните, чтобы выбрать другую"))).clicked() {
                    st.texture_picker = Some(String::new());
                    st.texture_pick_for = Some((ed.paint_layer, detail));
                }
                ui.end_row();
            }
            ui.label(tr("Repeats per tile", "Повторов на тайл"));
            ui.add(egui::DragValue::new(&mut g.params[1]).range(1.0..=200.0).speed(0.5));
            ui.end_row();
            ui.label(tr("Detail repeats", "Повторов детальной"));
            ui.add(egui::DragValue::new(&mut g.params[2]).range(1.0..=400.0).speed(0.5));
            ui.end_row();
            if ed.paint_layer > 0 {
                ui.label(tr("Paint detail", "Детальность покраски")).on_hover_text(tr("How finely this layer can be painted: texels across a tile (its masks are made again at the new size)", "Насколько тонко можно красить этот слой: текселей поперёк тайла (маски пересчитываются под новый размер)"));
                egui::ComboBox::from_id_salt("paint detail").selected_text(format!("{size} ({:.1} m)", crate::doc::tile_size() / size as f64)).show_ui(ui, |ui| {
                    for p in [7.0f32, 8.0, 9.0, 10.0] {
                        let n = 1usize << p as u32;
                        if ui.selectable_label(n == size, format!("{n} ({:.1} m)", crate::doc::tile_size() / n as f64)).clicked() && n != size {
                            detail = Some(p);
                        }
                    }
                });
                ui.end_row();
            }
        });
        if g.params != before {
            let now = g.params;
            g.params = before;
            let was = (doc.global.clone(), doc.order.clone());
            doc.global.ground_textures[ed.paint_layer].params = now;
            ed.history.push_snapshot(tr("Edit ground texture", "Правка текстуры земли"), crate::history::Snapshot { global: Some(was), ..Default::default() });
            doc.global_dirty = true;
            doc.global_rev = crate::doc::next_rev();
        }
    }
    if let Some(p) = detail {
        ed.set_paint_detail(ed.paint_layer, p);
    }
    if let Some(over) = move_layer {
        ed.move_ground_layer(ed.paint_layer, over);
    }
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Brush", "Кисть")).strong());
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.paint_erase, false, icons::labelled(Icon::Brush, tr("Paint", "Красить")));
        ui.selectable_value(&mut ed.paint_erase, true, icons::labelled(Icon::Eraser, tr("Erase", "Стирать")));
    });
    brush_grid(ui, ed);
    crate::autopaint::panel(ui, &mut st.autopaint, ed);
    let _ = actions;
    if let Some(i) = remove {
        let Some(doc) = ed.doc.as_mut() else { return };
        ed.history.begin(tr("Remove ground texture", "Удаление текстуры земли"));
        let keys: Vec<_> = doc.tiles.keys().copied().collect();
        for k in keys {
            ed.history.touch_tile(doc, k);
        }
        ed.history.touch_global(doc);
        doc.global.ground_textures.remove(i);
        for t in doc.tiles.values_mut() {
            if i < t.masks.len() {
                t.masks.remove(i);
            }
            t.touch_masks();
        }
        doc.global_dirty = true;
        doc.global_rev = crate::doc::next_rev();
        ed.history.commit();
        ed.paint_layer = ed.paint_layer.min(doc.global.ground_textures.len().saturating_sub(1));
    }
}

// ---------------------------------------------------------------------------- right panel

pub(crate) fn prop_row(ui: &mut egui::Ui, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(name).color(TEXT_DIM));
    add(ui);
    ui.end_row();
}

/// The tile clicked last (Tiles tool): its water on or off and its height, and whether its
/// lamps light the ground at night.
fn water_panel(ui: &mut egui::Ui, ed: &mut Editor) {
    let Some((k, water, lowest)) = ed.picked_tile.and_then(|k| ed.doc.as_ref()?.tiles.get(&k).map(|t| (k, t.water, t.terrain.heights.iter().copied().fold(f32::MAX, f32::min)))) else { return };
    ui.add_space(10.0);
    ui.label(RichText::new(format!("{} {}, {}", tr("Tile", "Тайл"), k.0, k.1)).strong());
    let mut on = water.is_some();
    let mut level = water.map(|w| (w[0] + w[1] + w[2] + w[3]) / 4.0).unwrap_or(lowest + 1.0);
    let mut change = false;
    ui.horizontal(|ui| {
        change |= check(ui, &mut on, tr("Water", "Вода")).on_hover_text(tr("A water surface over the whole tile: it shows where the ground lies below it (lower the ground with the Terrain tool for a lake or a river)", "Водная гладь на весь тайл: видна там, где земля ниже неё (опустите землю инструментом «Рельеф» для озера или реки)")).changed();
        if on {
            change |= ui.add(egui::DragValue::new(&mut level).speed(0.05).max_decimals(2).prefix(tr("at ", "на ")).suffix(" m")).changed();
        }
    });
    if change {
        ed.set_water(k, on.then_some(level));
    } else {
        ed.end_panel_step_if_idle(ui.ctx());
    }
    let mut lamps = ed.doc.as_ref().and_then(|d| d.tiles.get(&k)).map(|t| t.tile.variable_terrain_lightmap).unwrap_or(false);
    if check(ui, &mut lamps, tr("Lamps light the ground at night", "Фонари освещают землю ночью")).on_hover_text(tr("The game works out the light pools of the street lamps on and round this tile when it loads it ([variable_terrainlightmap]); off, it shows the tile's light map file as it is", "Игра рассчитывает пятна света фонарей на этом тайле и вокруг при загрузке ([variable_terrainlightmap]); выключено — показывает файл карты освещения тайла как есть")).changed() {
        ed.set_tile_lamplight(k, lamps);
    }
}

/// The edits made this session: a click on one goes back (or forward) to just after it.
fn history_window(ctx: &egui::Context, st: &mut UiState, ed: &Editor, actions: &mut Vec<Action>) {
    if !st.history_open {
        return;
    }
    let (undo, redo) = ed.history.steps();
    let mut open = st.history_open;
    egui::Window::new(icons::labelled(Icon::History, tr("Edit history", "История правок"))).id(egui::Id::new("edit history")).open(&mut open).default_pos([340.0, 120.0]).default_size([300.0, 380.0]).resizable(true).show(ctx, |ui| {
        ui.label(RichText::new(tr("Click a step to go back to just after it", "Клик по шагу: вернуться к состоянию сразу после него")).small().color(TEXT_DIM));
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if ui.selectable_label(undo.is_empty(), RichText::new(tr("Opened", "Открыто")).italics()).clicked() {
                actions.extend((0..undo.len()).map(|_| Action::Undo));
            }
            for (i, l) in undo.iter().enumerate() {
                let last = i + 1 == undo.len();
                if ui.selectable_label(last, *l).clicked() {
                    actions.extend((0..undo.len() - i - 1).map(|_| Action::Undo));
                }
            }
            for (i, l) in redo.iter().enumerate() {
                if ui.selectable_label(false, RichText::new(*l).color(TEXT_DIM)).clicked() {
                    actions.extend((0..i + 1).map(|_| Action::Redo));
                }
            }
        });
    });
    st.history_open = open;
}

fn properties(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, lib: &Library, actions: &mut Vec<Action>) {
    let Some(sel) = ed.selected else {
        map_properties(ui, st, ed, lib, actions);
        return;
    };
    if ed.selection().len() > 1 {
        crate::ui_edit::group_properties(ui, ed, lib, actions);
        let objects = ed.selection().iter().filter(|s| matches!(s, Sel::Object(..))).count();
        let splines = ed.selection().len() - objects;
        Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| crate::ui_mesh::many_panel(ui, &mut st.keep_cutters, objects, splines, actions));
        return;
    }
    section(ui, tr("Properties", "Свойства"));
    let mut delete = false;
    let mut changed_labels: Option<Vec<String>> = None;
    let mut new_rot: Option<[f64; 3]> = None;
    let mut new_lift: Option<f64> = None;
    let mut new_scale: Option<(bool, glam::DVec3)> = None;
    let mut new_name: Option<String> = None;
    let mut new_pos: Option<(f64, f64)> = None;
    let mut ends = crate::ui_edit::EndsEdit::default();
    let mut spline_change: Option<(Option<String>, Option<bool>)> = None;
    // (skew at the start and the end, the ground's mode: 0 none, 1 cut under it, 2–4 the
    // ends held)
    let mut spline_extra: Option<(f64, f64, u8)> = None;
    let mut whole_road = false;
    let mut knife = false;
    let mut join_next = false;
    let mut reverse = false;
    let mut lay = false;
    let mut align_road = false;
    let mut lean = false;
    let mut alike = false;
    let mut replace_obj: Option<String> = None;
    let Some(doc) = ed.doc.as_ref() else { return };
    Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
        match sel {
            Sel::Object(k, id) => {
                let Some(o) = crate::selection::any_object(doc, k, id) else { return };
                let a = lib.find(&o.file);
                let name = a.map(|a| a.name.clone()).unwrap_or_else(|| o.file.rsplit('\\').next().unwrap_or("").to_string());
                ui.label(RichText::new(name).strong().color(Color32::WHITE));
                ui.label(RichText::new(&o.file).small().color(TEXT_DIM));
                // its textures' PBR maps: made here (Configure) or not yet (Create)
                let made = st.pbr_made.as_ref().filter(|(rel, _)| rel.eq_ignore_ascii_case(&o.file)).map(|(_, m)| *m).unwrap_or(false);
                let label = if made { tr("Configure PBR", "Настроить PBR") } else { tr("Create PBR", "Создать PBR") };
                let r = ui.button(icons::labelled(Icon::Layers, label));
                help(st, &r, tr("Normal, roughness, metal and shadow maps made from the object's textures, tuned and written as openOMSI reads them", "Карты нормалей, шероховатости, металла и теней из текстур объекта: настроить и записать так, как их читает openOMSI"));
                if r.clicked() {
                    actions.push(Action::PbrForObject(o.file.clone()));
                }
                ui.add_space(4.0);
                egui::Grid::new("obj").num_columns(2).spacing(Vec2::new(8.0, 5.0)).show(ui, |ui| {
                    prop_row(ui, tr("Type", "Тип"), |ui| {
                        ui.label(a.map(|a| a.category.label()).unwrap_or("-"));
                    });
                    prop_row(ui, "ID", |ui| {
                        ui.label(id.to_string());
                    });
                    prop_row(ui, tr("Tile", "Тайл"), |ui| {
                        ui.label(format!("{}, {}", k.0, k.1));
                    });
                    if let Some(p) = o.parent_id {
                        prop_row(ui, tr("Hangs on", "Висит на"), |ui| {
                            ui.label(format!("ID {p} · {} {}", tr("point", "точка"), o.attach_index));
                        });
                    }
                    if o.parent_id.is_none() {
                        // (on the map, in metres; past a tile's edge it goes over to the next one;
                        // the height is the world's, whatever the object keeps it as)
                        let org = crate::doc::tile_origin(k);
                        let abs = a.map(|a| a.abs_height).unwrap_or(false);
                        let ground = doc.ground(glam::DVec2::new(o.pos[0] + org.x, o.pos[1] + org.y));
                        let mut v = [o.pos[0] + org.x, o.pos[1] + org.y, if abs { o.pos[2] } else { ground + o.pos[2] }];
                        prop_row(ui, tr("Location", "Положение"), |ui| {
                            let ch = crate::ui_xform::xyz(ui, &mut v, 0.05, " m", 3, None, [tr("East", "Восток"), tr("North", "Север"), tr("Height above sea level", "Высота над уровнем моря")]);
                            if ch[0] || ch[1] {
                                new_pos = Some((v[0] - org.x, v[1] - org.y));
                            }
                            if ch[2] {
                                new_lift = Some(if abs { v[2] } else { v[2] - ground });
                            }
                        });
                        if !abs {
                            prop_row(ui, "", |ui| {
                                ui.label(RichText::new(format!("{} {:.2} m", tr("above the ground:", "над землёй:"), o.pos[2])).small().color(TEXT_DIM)).on_hover_text(tr("OMSI keeps this object's height above the ground under it: moved elsewhere it stays as high above the ground there", "OMSI хранит высоту этого объекта над землёй под ним: перенесённый в другое место, он остаётся на той же высоте над землёй"));
                            });
                        }
                    }
                    prop_row(ui, tr("Rotation", "Поворот"), |ui| {
                        // (X: tilt forward, Y: bank to the side, Z: heading, as the gizmo's rings)
                        let r = o.rot;
                        let mut v = [r[1], r[2], r[0]];
                        ui.vertical(|ui| {
                            let ch = crate::ui_xform::xyz(ui, &mut v, 0.5, "°", 2, Some((-360.0, 720.0)), [tr("Tilt forward (pitch)", "Наклон вперёд (тангаж)"), tr("Bank to the side (roll)", "Крен вбок"), tr("Heading", "Направление (курс)")]);
                            if ch.iter().any(|c| *c) {
                                new_rot = Some([v[2].rem_euclid(360.0), v[0], v[1]]);
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button(Icon::Road.text(13.0)).on_hover_text(tr("Turn it the way the nearest road runs beside it", "Повернуть по ходу ближайшей дороги рядом")).clicked() {
                                    align_road = true;
                                }
                                if ui.small_button(Icon::Terrain.text(13.0)).on_hover_text(crate::ui_edit::lean_help()).clicked() {
                                    lean = true;
                                }
                                if (r[1] != 0.0 || r[2] != 0.0) && ui.small_button("↺").on_hover_text(tr("Stand it upright again", "Поставить ровно")).clicked() {
                                    new_rot = Some([r[0].rem_euclid(360.0), 0.0, 0.0]);
                                }
                            });
                        });
                    });
                    if o.parent_id.is_none() {
                        prop_row(ui, tr("Scale", "Масштаб"), |ui| {
                            let now = crate::scaled::base_of(&o.file).1;
                            if let Some((done, v)) = crate::ui_xform::scale(ui, egui::Id::new(("scale", id)), [now.x, now.y, now.z], &mut st.scale_uniform) {
                                let f = glam::DVec3::new(v[0] / now.x.max(1e-6), v[1] / now.y.max(1e-6), v[2] / now.z.max(1e-6));
                                new_scale = Some((done, f));
                            }
                        });
                    }
                });
                let is_stop = a.map(|a| a.is_bus_stop).unwrap_or(false);
                let is_entry = a.map(|a| a.is_entry_point).unwrap_or(false);
                if is_entry {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("Start point", "Точка старта")).strong());
                    if let Some(e) = doc.global.entry_points.iter().find(|e| e.object_id == id) {
                        let mut n = e.name.clone();
                        if ui.add(egui::TextEdit::singleline(&mut n).desired_width(f32::INFINITY)).changed() {
                            new_name = Some(n);
                        }
                    }
                }
                let mut labels = o.extra.clone();
                // (the model's names for its labels, once it is read)
                // (a tree keeps its own picture, height and width in them)
                let tree = lib.find(&o.file).map(|a| a.category == crate::library::Category::Trees).unwrap_or(false);
                let tree_names = [tr("Picture", "Картинка").to_string(), tr("Height", "Высота").to_string(), tr("Ratio", "Пропорции").to_string()];
                let names: &[String] = if tree { &tree_names } else { st.obj_info.as_ref().filter(|i| i.file.eq_ignore_ascii_case(&o.file)).map(|i| i.labels.as_slice()).unwrap_or(&[]) };
                if is_stop || !labels.is_empty() || (!names.is_empty() && !tree) {
                    ui.add_space(6.0);
                    ui.label(RichText::new(if is_stop { tr("Stop name", "Название остановки") } else if tree { tr("This tree", "Это дерево") } else { tr("Texts", "Тексты") }).strong());
                    if !is_stop && !tree && !names.is_empty() {
                        ui.label(RichText::new(tr("Shown on the object (@ starts a new line)", "Видны на объекте (@ — новая строка)")).small().color(TEXT_DIM));
                    }
                    let n = if is_stop { 1 } else { labels.len().max(names.len()).max(1) };
                    while labels.len() < n {
                        labels.push(String::new());
                    }
                    let mut any = false;
                    egui::Grid::new("labels").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
                        for (i, l) in labels.iter_mut().take(n).enumerate() {
                            let name = names.get(i).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("{} {}", tr("Text", "Текст"), i + 1));
                            ui.label(RichText::new(name).color(TEXT_DIM));
                            if ui.add(egui::TextEdit::singleline(l).desired_width(f32::INFINITY)).changed() {
                                any = true;
                            }
                            ui.end_row();
                        }
                    });
                    if any {
                        // (trailing empty labels are not written)
                        while labels.len() > 1 && labels.last().map(|l| l.is_empty()).unwrap_or(false) && labels.len() > o.extra.len() {
                            labels.pop();
                        }
                        changed_labels = Some(labels);
                    }
                }
                // the model: its size, and its textures with their PBR maps
                if let Some(info) = st.obj_info.as_ref().filter(|i| i.file.eq_ignore_ascii_case(&o.file)) {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(RichText::new(tr("Model", "Модель")).strong()).icon(group_icon).default_open(true).show(ui, |ui| {
                        egui::Grid::new("model info").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
                            prop_row(ui, tr("Size", "Размер"), |ui| {
                                ui.label(format!("{:.1} × {:.1} × {:.1} m", info.size[0], info.size[1], info.size[2]));
                            });
                            prop_row(ui, tr("Triangles", "Треугольники"), |ui| {
                                ui.label(info.triangles.to_string());
                            });
                        });
                        // the pieces it was made of, each with the transparency of its materials (an
                        // imported model's can be changed)
                        // (an openMapper object's materials are set in its Mesh section: its files are
                        // written from there)
                        let imported = o.file.to_lowercase().contains("\\imported\\") && !o.file.to_lowercase().contains("\\openmapper\\");
                        if !info.pieces.is_empty() {
                            ui.label(RichText::new(format!("{} ({})", tr("Parts", "Части"), info.pieces.len())).color(TEXT_DIM));
                        }
                        for (pi, piece) in info.pieces.iter().enumerate() {
                            egui::CollapsingHeader::new(RichText::new(format!("{} {} · {} {}", tr("Part", "Часть"), pi + 1, piece.triangles, tr("triangles", "треугольников"))).small()).id_salt(("piece", pi)).show(ui, |ui| {
                                for (mi, (tex, index, alpha)) in piece.materials.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(if tex.is_empty() { tr("colour only", "только цвет") } else { tex }).small());
                                        let names = [tr("opaque", "непрозрачный"), tr("cut out", "вырезной"), tr("blended", "полупрозрачный")];
                                        let mut v = (*alpha).min(2);
                                        ui.add_enabled_ui(imported && !tex.is_empty(), |ui| {
                                            egui::ComboBox::from_id_salt(("alpha", pi, mi)).selected_text(names[v as usize]).width(110.0).show_ui(ui, |ui| {
                                                for (n, name) in names.iter().enumerate() {
                                                    ui.selectable_value(&mut v, n as u8, *name);
                                                }
                                            }).response.on_hover_text(tr("How the texture's transparency is used: cut out for leaves and fences, blended for glass", "Как используется прозрачность текстуры: вырезной — листья и заборы, полупрозрачный — стекло"));
                                        });
                                        if v != *alpha {
                                            actions.push(Action::SetAlpha(o.file.clone(), pi, tex.clone(), *index, v));
                                        }
                                    });
                                }
                            });
                        }
                        ui.label(RichText::new(tr("Textures", "Текстуры")).color(TEXT_DIM));
                        for (name, pbr) in &info.textures {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(name).small());
                                if *pbr {
                                    ui.label(RichText::new("PBR").small().color(ACCENT));
                                }
                            });
                        }
                    });
                }
            }
            Sel::Spline(k, id) => {
                let Some(s) = doc.spline(k, id) else { return };
                let a = lib.find(&s.file);
                ui.label(RichText::new(a.map(|a| a.name.clone()).unwrap_or_else(|| s.file.clone())).strong().color(Color32::WHITE));
                ui.label(RichText::new(&s.file).small().color(TEXT_DIM));
                let made = st.pbr_made.as_ref().filter(|(rel, _)| rel.eq_ignore_ascii_case(&s.file)).map(|(_, m)| *m).unwrap_or(false);
                let r = ui.button(icons::labelled(Icon::Layers, if made { tr("Configure PBR", "Настроить PBR") } else { tr("Create PBR", "Создать PBR") }));
                help(st, &r, tr("Normal, roughness, metal and shadow maps made from the road type's textures", "Карты нормалей, шероховатости, металла и теней из текстур типа дороги"));
                if r.clicked() {
                    actions.push(Action::PbrForSpline(s.file.clone()));
                }
                ui.add_space(4.0);
                egui::Grid::new("spl").num_columns(2).spacing(Vec2::new(8.0, 5.0)).show(ui, |ui| {
                    prop_row(ui, "ID", |ui| {
                        ui.label(id.to_string());
                    });
                    prop_row(ui, tr("Length", "Длина"), |ui| {
                        let mut l = s.length;
                        // (the range takes in what is there: a value pulled into it would count as typed)
                        if ui.add(egui::DragValue::new(&mut l).speed(0.1).range(s.length.min(0.5)..=s.length.max(5000.0)).max_decimals(2).suffix(" m")).changed() && l != s.length {
                            ed.shape_request = Some((k, id, l, s.radius));
                        }
                    });
                    prop_row(ui, tr("Radius", "Радиус"), |ui| {
                        ui.horizontal(|ui| {
                            let mut r = s.radius.abs();
                            // (f64::signum gives 1 for 0: a straight is no side)
                            let mut side = if s.radius == 0.0 { 0.0 } else { s.radius.signum() };
                            let mut changed = false;
                            if side != 0.0 {
                                changed = ui.add(egui::DragValue::new(&mut r).speed(0.5).range(s.radius.abs().min(3.0)..=s.radius.abs().max(20000.0)).max_decimals(1).suffix(" m")).changed() && r != s.radius.abs();
                            }
                            egui::ComboBox::from_id_salt("bend").selected_text(if side > 0.0 { tr("right", "вправо") } else if side < 0.0 { tr("left", "влево") } else { tr("straight", "прямая") }).width(80.0).show_ui(ui, |ui| {
                                for (v, name) in [(0.0, tr("straight", "прямая")), (-1.0, tr("left", "влево")), (1.0, tr("right", "вправо"))] {
                                    if ui.selectable_value(&mut side, v, name).changed() {
                                        changed = true;
                                    }
                                }
                            });
                            if changed {
                                // (a straight made a bend starts gently: ten times its length)
                                let r = if r == 0.0 { (s.length * 10.0).max(50.0) } else { r };
                                ed.shape_request = Some((k, id, s.length, side * r));
                            }
                        });
                    });
                    prop_row(ui, tr("Linked", "Связи"), |ui| {
                        ui.label(format!("{} / {}", if s.prev_id != 0 { tr("start", "начало") } else { "-" }, if s.next_id != 0 { tr("end", "конец") } else { "-" }));
                    });
                    prop_row(ui, tr("Mirrored", "Зеркально"), |ui| {
                        let mut m = s.mirror;
                        if check(ui, &mut m, "").changed() {
                            spline_change = Some((None, Some(m)));
                        }
                    });
                    let mut skew = (s.skew_start, s.skew_end);
                    let mut mode = s.terrain_align.map(|v| v as u8).unwrap_or(s.terrain_align_flag as u8);
                    prop_row(ui, tr("Skew", "Скос"), |ui| {
                        ui.horizontal(|ui| {
                            let a = ui.add(egui::DragValue::new(&mut skew.0).speed(0.01).range(-4.0..=4.0).max_decimals(3)).on_hover_text(tr("The start cut on a slant: 1 is 45° (two fences meeting at a corner: 1 and -1)", "Скошенный срез в начале: 1 — это 45° (два забора в углу: 1 и -1)"));
                            let b = ui.add(egui::DragValue::new(&mut skew.1).speed(0.01).range(-4.0..=4.0).max_decimals(3)).on_hover_text(tr("The end cut on a slant", "Скошенный срез в конце"));
                            if a.changed() || b.changed() {
                                spline_extra = Some((skew.0, skew.1, mode));
                            }
                        });
                    });
                    prop_row(ui, tr("Ground", "Земля"), |ui| {
                        let names = [tr("as it is", "как есть"), tr("cut away under it", "вырезать под ним"), tr("cut, held at the end", "вырезать, до конца"), tr("cut, held at the start", "вырезать, от начала"), tr("cut, held at both ends", "вырезать, оба конца")];
                        let before = mode;
                        egui::ComboBox::from_id_salt("terrain align").selected_text(names[(mode as usize).min(4)]).width(170.0).show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(&mut mode, i as u8, *n);
                            }
                        })
                        .response
                        .on_hover_text(tr("A hole in the ground under it, as for a tunnel, an underpass or a pit (OMSI's terrain align)", "Дыра в земле под ним, как для тоннеля, подземного перехода или ямы (terrain align в OMSI)"));
                        if mode != before {
                            spline_extra = Some((skew.0, skew.1, mode));
                        }
                    });
                });
                if ui.small_button(tr("Lay on the ground", "Положить на землю")).on_hover_text(crate::ui_edit::lay_help()).clicked() {
                    lay = true;
                }
                if ui.small_button(tr("Reverse direction", "Развернуть направление")).on_hover_text(crate::ui_edit::reverse_help()).clicked() {
                    reverse = true;
                }
                if s.prev_id != 0 || s.next_id != 0 {
                    if ui.small_button(tr("Select the whole road", "Выбрать всю дорогу")).on_hover_text(tr("Every piece linked on from this one (or double-click the road)", "Все куски, связанные с этим (или двойной клик по дороге)")).clicked() {
                        whole_road = true;
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    let b = egui::Button::new(tr("Cut", "Разрезать")).small().selected(ed.cutting);
                    if ui.add(b).on_hover_text(crate::ui_edit::knife_help()).clicked() {
                        knife = true;
                    }
                    if s.next_id != 0 && ui.small_button(tr("Join with the next piece", "Соединить со следующим куском")).on_hover_text(tr("One piece again with the one after it, where the two make one curve", "Снова один кусок со следующим, если вместе они образуют одну кривую")).clicked() {
                        join_next = true;
                    }
                });
                ends = crate::ui_edit::spline_ends(ui, doc, k, id);
                if let Some(rt) = &ed.road_type {
                    if !rt.eq_ignore_ascii_case(&s.file) {
                        let n = lib.find(rt).map(|a| a.name.clone()).unwrap_or_else(|| rt.clone());
                        ui.add_space(4.0);
                        if ui.button(icons::labelled(Icon::Swap, &format!("{} «{}»", tr("Replace with", "Заменить на"), n))).clicked() {
                            spline_change = Some((Some(rt.clone()), None));
                        }
                    }
                }
            }
        }
        // an object becomes the one chosen in the Objects catalogue (where it stands, turned so)
        if let (Sel::Object(k, id), Some(ot)) = (sel, ed.object_type.clone()) {
            let now = ed.doc.as_ref().and_then(|d| crate::selection::any_object(d, k, id)).map(|o| o.file.clone()).unwrap_or_default();
            if !ot.eq_ignore_ascii_case(&now) {
                let n = lib.find(&ot).map(|a| a.name.clone()).unwrap_or_else(|| ot.clone());
                ui.add_space(4.0);
                if ui.button(icons::labelled(Icon::Swap, &format!("{} «{}»", tr("Replace with", "Заменить на"), n))).on_hover_text(tr("The object chosen in the Objects tool's catalogue takes this one's place", "Объект, выбранный в каталоге инструмента «Объекты», встаёт на место этого")).clicked() {
                    replace_obj = Some(ot);
                }
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button(icons::labelled(Icon::Focus, tr("Go to", "Перейти"))).clicked() {
                actions.push(Action::FocusSelection);
            }
            let is_tree = match sel {
                Sel::Object(k, id) => ed.doc.as_ref().and_then(|d| crate::selection::any_object(d, k, id)).and_then(|o| lib.find(&o.file)).map(|a| a.category == crate::library::Category::Trees).unwrap_or(false),
                _ => false,
            };
            if matches!(sel, Sel::Object(..)) && !is_tree && ui.button(icons::labelled(Icon::Flatten, tr("Level ground", "Выровнять землю"))).on_hover_text(tr("The ground under it made level with its foot, blending in 3 m round it (a house on a slope)", "Земля под ним — ровно на уровне основания, с плавным переходом 3 м вокруг (дом на склоне)")).clicked() {
                ed.level_request = true;
            }
            if ui.button(icons::labelled(Icon::SelectAll, tr("All like it", "Все такие"))).on_hover_text(tr("Select every one of this type on the map", "Выбрать все такие на карте")).clicked() {
                alike = true;
            }
            if ui.button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked() {
                delete = true;
            }
        });
    });
    // (an object's rules only where it has paths, or rules already)
    let has_paths = match sel {
        Sel::Object(k, id) => {
            let file = ed.doc.as_ref().and_then(|d| crate::selection::any_object(d, k, id)).map(|o| (o.file.clone(), !o.rules.is_empty()));
            file.map(|(f, r)| r || st.obj_info.as_ref().filter(|i| i.file.eq_ignore_ascii_case(&f)).map(|i| i.paths > 0).unwrap_or(false)).unwrap_or(false)
        }
        Sel::Spline(..) => true,
    };
    let attach_points = match sel {
        Sel::Object(k, id) => ed.doc.as_ref().and_then(|d| crate::selection::any_object(d, k, id)).and_then(|o| st.obj_info.as_ref().filter(|i| i.file.eq_ignore_ascii_case(&o.file))).map(|i| i.attach_points).unwrap_or(0),
        _ => 0,
    };
    let obj_info = st.obj_info.clone();
    let parent_lights = st.parent_lights.clone();
    if alike {
        let n = ed.select_alike(sel);
        ed.toast(format!("{} {n}", tr("Selected:", "Выбрано:")));
        return;
    }
    if delete {
        ed.delete_selected();
        return;
    }
    if ed.doc.is_none() {
        return;
    }
    match sel {
        Sel::Object(k, id) => {
            crate::ui_edit::junction_section(ui, ed, k, id);
            crate::ui_edit::array_section(ui, ed, lib);
            crate::ui_edit::link_section(ui, ed, lib, k, id, obj_info.as_ref(), parent_lights.as_ref());
            if has_paths {
                crate::ui_edit::rules_section(ui, ed, sel);
            }
            if attach_points > 0 {
                crate::ui_edit::attach_section(ui, ed, lib, k, id, attach_points);
            }
            if let Some(f) = replace_obj {
                ed.replace_selected_objects(&f);
                return;
            }
            if lean {
                ed.lean_objects(true);
                return;
            }
            if align_road && ed.align_to_roads() == 0 {
                ed.toast(tr("No road within 60 m", "В пределах 60 м нет дороги"));
            }
            if let Some((x, y)) = new_pos {
                let o = crate::doc::tile_origin(k);
                ed.put_object(k, id, glam::DVec2::new(o.x + x, o.y + y));
            }
            match new_scale {
                // (while the number is dragged the object is drawn at its new size; its files are
                // made once it is let go)
                Some((false, f)) => ed.scale_preview = vec![(Sel::Object(k, id), f)],
                Some((true, f)) => {
                    ed.scale_preview.clear();
                    ed.history.begin(tr("Scale", "Масштаб"));
                    ed.scale_requests.push((Sel::Object(k, id), f));
                }
                None => {}
            }
            if new_rot.is_some() || new_lift.is_some() || changed_labels.is_some() || new_name.is_some() {
                ed.begin_panel_step(tr("Edit object", "Изменение объекта"));
                let Some(doc) = ed.doc.as_mut() else { return };
                ed.history.touch_tile(doc, k);
                ed.history.touch_global(doc);
                // (a stop renamed: its name before, and where it stands)
                let mut renamed: Option<(String, String, glam::DVec2)> = None;
                if let Some(o) = crate::selection::any_object_mut(doc, k, id) {
                    if let Some(r) = new_rot {
                        o.rot = r;
                    }
                    if let Some(z) = new_lift {
                        o.pos[2] = z;
                    }
                    if let Some(l) = changed_labels {
                        if o.flag == 7 && o.extra.len() >= 4 {
                            if let (Some(a), Some(b)) = (o.extra.first(), l.first()) {
                                renamed = Some((a.clone(), b.clone(), crate::doc::tile_origin(k) + glam::DVec2::new(o.pos[0], o.pos[1])));
                            }
                        }
                        if o.flag < l.len() as i32 {
                            o.flag = l.len() as i32;
                        }
                        o.extra = l;
                    }
                }
                // its twin across the road (a stop of the same name within 40 m) follows
                if let Some((was, now, at)) = renamed.filter(|r| r.0 != r.1) {
                    let was = &was;
                    let twins: Vec<(crate::doc::TileKey, i64)> = doc
                        .tiles
                        .iter()
                        .flat_map(|(tk, t)| t.tile.objects.iter().filter(move |o| o.id != id && o.flag == 7 && o.extra.first() == Some(was) && (crate::doc::tile_origin(*tk) + glam::DVec2::new(o.pos[0], o.pos[1]) - at).length() < 40.0).map(move |o| (*tk, o.id)))
                        .collect();
                    for (tk, tid) in twins {
                        ed.history.touch_tile(doc, tk);
                        if let Some(t) = doc.tiles.get_mut(&tk) {
                            if let Some(o) = t.tile.objects.iter_mut().find(|o| o.id == tid) {
                                o.extra[0] = now.clone();
                            }
                            t.touch_content();
                        }
                    }
                }
                if let Some(n) = new_name {
                    if let Some(e) = doc.global.entry_points.iter_mut().find(|e| e.object_id == id) {
                        e.name = n;
                    }
                }
                if let Some(t) = doc.tiles.get_mut(&k) {
                    t.touch_content();
                }
                crate::editor::sync_entry_points(doc);
            } else {
                ed.end_panel_step_if_idle(ui.ctx());
            }
            // as a mesh: openMapper's own object's parts, modifiers, materials and lamps, or
            // what can be done to any other
            egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
                if ed.mesh.as_ref().map(|m| m.inst == Sel::Object(k, id)).unwrap_or(false) {
                    crate::ui_mesh::object_panel(ui, ed, &mut st.bake, actions);
                } else {
                    let parts = st.obj_info.as_ref().map(|i| i.pieces.len()).unwrap_or(0);
                    crate::ui_mesh::plain_object_panel(ui, parts, actions);
                }
            });
        }
        Sel::Spline(k, id) => {
            if whole_road {
                ed.select_chain(k, id);
                return;
            }
            if knife {
                ed.toggle_cutting();
                return;
            }
            if join_next {
                ed.join_with_next(k, id);
                return;
            }
            if reverse {
                ed.reverse_selected();
                return;
            }
            if lay {
                ed.lay_selected_on_ground();
                return;
            }
            crate::ui_edit::apply_ends(ed, ui.ctx(), k, id, ends);
            crate::ui_edit::parallel_section(ui, ed, lib);
            if let Some((a, b, mode)) = spline_extra {
                ed.set_spline_extra(k, id, a, b, mode);
            }
            crate::ui_edit::rules_section(ui, ed, sel);
            crate::ui_edit::rows_section(ui, ed, lib);
            crate::ui_edit::paint_along_section(ui, ed, lib);
            crate::ui_profile::profile_section(ui, ed, actions);
            let Some(doc) = ed.doc.as_mut() else { return };
            if let Some((file, mirror)) = spline_change {
                ed.history.begin(tr("Edit road", "Изменение дороги"));
                ed.history.touch_tile(doc, k);
                if let Some(s) = doc.tiles.get_mut(&k).and_then(|t| t.tile.splines.iter_mut().find(|s| s.id == id)) {
                    if let Some(f) = file {
                        s.file = f;
                    }
                    if let Some(m) = mirror {
                        s.mirror = m;
                    }
                }
                if let Some(t) = doc.tiles.get_mut(&k) {
                    t.touch_content();
                }
                ed.history.commit();
            }
            Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| crate::ui_mesh::spline_panel(ui, actions));
        }
    }
}

/// With nothing selected the right panel shows the map's own settings.
/// Area (km²), roads and tracks (km), objects and bus stops of a map.
fn map_stats(doc: &crate::doc::Document, lib: &Library) -> [f64; 5] {
    use crate::library::Category;
    let mut v = [doc.tiles.len() as f64 * crate::doc::tile_size().powi(2) / 1e6, 0.0, 0.0, 0.0, 0.0];
    for t in doc.tiles.values() {
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            match lib.find(&s.file).map(|a| a.category) {
                Some(Category::Roads) => v[1] += s.length / 1000.0,
                Some(Category::Tracks) => v[2] += s.length / 1000.0,
                _ => {}
            }
        }
        v[3] += (t.tile.objects.len() + t.tile.attach_objects.len()) as f64;
        v[4] += t.tile.objects.iter().filter(|o| lib.find(&o.file).map(|a| a.is_bus_stop).unwrap_or(false)).count() as f64;
    }
    v
}

fn map_properties(ui: &mut egui::Ui, st: &mut UiState, ed: &mut Editor, lib: &Library, actions: &mut Vec<Action>) {
    section(ui, tr("Map", "Карта"));
    let Some(doc) = ed.doc.as_mut() else { return };
    let before = (doc.global.friendly_name.clone(), doc.global.description.clone(), doc.global.left_hand_traffic);
    Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
        egui::Grid::new("map").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
            prop_row(ui, tr("Name", "Название"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut doc.global.friendly_name).desired_width(f32::INFINITY));
            });
            prop_row(ui, tr("Folder", "Папка"), |ui| {
                ui.label(doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
            });
            prop_row(ui, tr("Traffic", "Движение"), |ui| {
                check(ui, &mut doc.global.left_hand_traffic, tr("left-hand", "левостороннее"));
            });
            prop_row(ui, tr("Start points", "Точки старта"), |ui| {
                ui.label(doc.global.entry_points.len().to_string());
            });
            // what the map holds (worked out again only when a tile changed)
            let key = doc.tiles.values().fold(lib.assets.len() as u64, |h, t| h.rotate_left(5) ^ t.content_rev);
            let id = egui::Id::new("map stats");
            let stats: Option<(u64, [f64; 5])> = ui.data(|d| d.get_temp(id));
            let v = match stats {
                Some((k, v)) if k == key => v,
                _ => {
                    let v = map_stats(doc, lib);
                    ui.data_mut(|d| d.insert_temp(id, (key, v)));
                    v
                }
            };
            prop_row(ui, tr("Area", "Площадь"), |ui| {
                ui.label(format!("{:.2} km² · {} {}", v[0], doc.tiles.len(), tr("tiles", "тайлов")));
            });
            prop_row(ui, tr("Roads", "Дороги"), |ui| {
                ui.label(format!("{:.1} km", v[1]));
            });
            if v[2] > 0.0 {
                prop_row(ui, tr("Tracks", "Пути"), |ui| {
                    ui.label(format!("{:.1} km", v[2]));
                });
            }
            prop_row(ui, tr("Objects", "Объекты"), |ui| {
                ui.label(format!("{} · {} {}", v[3] as u64, v[4] as u64, tr("stops", "остановок")));
            });
        });
        ui.add_space(4.0);
        crate::descriptions::editor(ui, doc, &mut ed.history);
        if doc.global.entry_points.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(tr("No start point yet: place one (Game logic > Start points) so the bus has somewhere to appear in openOMSI.", "Нет точки старта: поставьте её (Служебное > Точки старта), чтобы автобусу было где появиться в openOMSI.")).small().color(Color32::from_rgb(230, 170, 100)));
        }
        ui.add_space(8.0);
        ui.label(RichText::new(tr("Picture", "Картинка")).color(TEXT_DIM));
        let path = doc.dir.join("picture.jpg");
        let path = if path.is_file() { path } else { doc.source.as_ref().map(|s| s.join("picture.jpg")).unwrap_or(path) };
        let time = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if st.map_picture.as_ref().map(|(p, t, _)| *p != path || *t != time).unwrap_or(true) {
            let tex = image::open(&path).ok().map(|img| {
                let img = img.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                ui.ctx().load_texture("map picture", ci, egui::TextureOptions::LINEAR)
            });
            st.map_picture = Some((path, time, tex));
        }
        match st.map_picture.as_ref().and_then(|(_, _, t)| t.as_ref()) {
            Some(t) => {
                let w = ui.available_width();
                ui.add(egui::Image::new((t.id(), Vec2::new(w, w * 280.0 / 370.0))));
            }
            None => {
                ui.label(RichText::new(tr("None yet: openOMSI shows it where maps are chosen.", "Пока нет: openOMSI показывает её при выборе карты.")).small().color(TEXT_DIM));
            }
        }
        let r = ui.button(icons::labelled(Icon::Import, tr("Import your thumbnail", "Импортировать свою картинку")));
        help(st, &r, tr("Choose a picture: it is cut to 370 × 280 and becomes the map's picture (picture.jpg).", "Выберите картинку: она будет обрезана до 370 × 280 и станет картинкой карты (picture.jpg)."));
        if r.clicked() {
            actions.push(Action::ImportMapPicture);
        }
        let r = ui.button(icons::labelled(Icon::Flag, tr("Open the map with this view", "Открывать карту с этого вида")));
        help(st, &r, tr("The camera as it is now is where the map opens (here and in OMSI 2's editor).", "Карта будет открываться с камерой, как сейчас (здесь и в редакторе OMSI 2)."));
        if r.clicked() {
            actions.push(Action::SetMapCamera);
        }
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{}:\n{}", tr("Saved to", "Сохраняется в"), doc.dir.display())).small().color(TEXT_DIM));
    });
    let after = (doc.global.friendly_name.clone(), doc.global.description.clone(), doc.global.left_hand_traffic);
    if before != after {
        doc.global_dirty = true;
        // (a step of its own: undoing an earlier edit of the global settings would else take
        // this back too)
        let mut g = doc.global.clone();
        (g.friendly_name, g.description, g.left_hand_traffic) = before;
        ed.history.push_snapshot(tr("Edit map", "Правка карты"), crate::history::Snapshot { global: Some((g, doc.order.clone())), ..Default::default() });
    }
}

// ---------------------------------------------------------------------------- dialogs

fn texture_picker(ctx: &egui::Context, st: &mut UiState, files: &[String], swatches: &HashMap<String, egui::TextureHandle>, wanted: &mut Vec<String>, actions: &mut Vec<Action>) {
    let Some(search) = &mut st.texture_picker else { return };
    let mut open = true;
    let mut chosen = None;
    let title = match st.texture_pick_for {
        None => tr("Add ground texture", "Добавить текстуру земли"),
        Some((_, false)) => tr("Change the layer's texture", "Сменить текстуру слоя"),
        Some((_, true)) => tr("Change the layer's detail texture", "Сменить детальную текстуру слоя"),
    };
    egui::Window::new(title).id(egui::Id::new("texture picker")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
        ui.set_width(470.0);
        ui.horizontal(|ui| {
            ui.label(tr("Search", "Поиск"));
            ui.add(egui::TextEdit::singleline(search).desired_width(f32::INFINITY));
        });
        let q = search.to_lowercase();
        let list: Vec<&String> = files.iter().filter(|f| f.to_lowercase().contains(&q)).collect();
        let cols = 5;
        Frame::new().fill(BG_FIELD).stroke(Stroke::new(1.0, BORDER)).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, false]).show_rows(ui, 94.0, list.len().div_ceil(cols), |ui, range| {
                for r in range {
                    ui.horizontal(|ui| {
                        for f in list.iter().skip(r * cols).take(cols) {
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(86.0, 90.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(2), SELECT);
                            }
                            let img = egui::Rect::from_min_size(rect.min + Vec2::new(5.0, 3.0), Vec2::splat(76.0));
                            match swatches.get(&f.to_lowercase()) {
                                Some(h) => {
                                    egui::Image::new((h.id(), img.size())).paint_at(ui, img);
                                }
                                None => wanted.push((*f).clone()),
                            }
                            let name = f.rsplit('\\').next().unwrap_or(f);
                            ui.painter().text(egui::pos2(rect.center().x, rect.max.y - 5.0), egui::Align2::CENTER_CENTER, name.chars().take(14).collect::<String>(), egui::FontId::proportional(10.0), Color32::from_gray(200));
                            if resp.clicked() {
                                chosen = Some((*f).clone());
                            }
                        }
                    });
                }
            });
        });
    });
    if let Some(c) = chosen {
        actions.push(match st.texture_pick_for {
            Some((layer, detail)) => Action::SetGroundTexture(layer, detail, c),
            None => Action::AddGroundTexture(c),
        });
        open = false;
    }
    if !open {
        st.texture_picker = None;
        st.texture_pick_for = None;
    }
}

fn controls(ctx: &egui::Context, st: &mut UiState, prefs: &Prefs, actions: &mut Vec<Action>) {
    let mut open = st.help_open;
    egui::Window::new(tr("Controls", "Управление")).open(&mut open).collapsible(false).default_height(560.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().auto_shrink([true, false]).show(ui, |ui| {
        ui.label(RichText::new(tr("Keyboard", "Клавиатура")).strong().color(ACCENT));
        ui.label(RichText::new(tr("Click a key, then press the new one (Esc: leave it). A key already in use is taken from its command.", "Нажмите на клавишу, затем новую (Esc: оставить). Занятая клавиша отбирается у своей команды.")).small().color(TEXT_DIM));
        let map = crate::controls::Keymap::of(&prefs.keys);
        egui::Grid::new("key bindings").striped(true).num_columns(3).show(ui, |ui| {
            for cmd in crate::controls::ALL {
                ui.label(cmd.label());
                let waiting = st.key_capture == Some(*cmd);
                let text = if waiting { tr("press a key…", "нажмите клавишу…").to_string() } else { map.chord(*cmd).map(|k| k.text()).unwrap_or_else(|| "—".into()) };
                let also = cmd.also();
                let mut b = ui.add_sized([130.0, 20.0], egui::Button::new(RichText::new(text).monospace()).selected(waiting));
                if !also.is_empty() {
                    b = b.on_hover_text(format!("{} {also}", tr("Also:", "Также:")));
                }
                if b.clicked() {
                    st.key_capture = if waiting { None } else { Some(*cmd) };
                }
                if ui.small_button(icons::labelled(Icon::Delete, "")).on_hover_text(tr("Back to the default key", "Вернуть клавишу по умолчанию")).clicked() {
                    actions.push(Action::BindKey(*cmd, Some(cmd.default())));
                }
                ui.end_row();
            }
        });
        if ui.button(tr("Reset all keys", "Сбросить все клавиши")).clicked() {
            actions.push(Action::ResetKeys);
        }
        ui.add_space(8.0);
        let rows: &[(&str, &str)] = &[
            (tr("Mouse", "Мышь"), ""),
            (tr("Right button held", "Правая кнопка зажата"), tr("look round; W A S D fly, Q / E down / up, Shift faster, wheel: speed", "оглядываться; W A S D: полёт, Q / E: вниз / вверх, Shift: быстрее, колесо: скорость")),
            (tr("Middle drag", "Средняя кнопка"), tr("pan", "сдвиг вида")),
            (tr("Alt + left / right / middle drag", "Alt + левая / правая / средняя"), tr("orbit / zoom / pan (as in Unity)", "орбита / зум / сдвиг (как в Unity)")),
            (tr("Left drag on nothing", "Левая по пустому месту"), tr("select with a box", "выделить рамкой")),
            (tr("Alt + left drag", "Alt + левая кнопка"), tr("orbit round the point under the cursor", "орбита вокруг точки под курсором")),
            (tr("Wheel", "Колесо"), tr("move towards the cursor", "приближение к курсору")),
            (tr("Trackpad", "Трекпад"), ""),
            (tr("Two fingers", "Два пальца"), tr("pan", "сдвиг вида")),
            (tr("Pinch / twist", "Щипок / поворот"), tr("zoom / turn", "зум / поворот")),
            (tr("Option + two fingers or drag", "Option + два пальца или drag"), tr("orbit round the point under the cursor", "орбита вокруг точки под курсором")),
            (tr("Fixed keys", "Постоянные клавиши"), ""),
            (tr("Arrow keys", "Стрелки"), tr("move over the map (Shift: faster)", "двигаться по карте (Shift: быстрее)")),
            ("Ctrl+1…9", tr("go to a saved view (Shift: keep the view there)", "к сохранённому виду (Shift: запомнить вид туда)")),
            (tr("Alt + arrows", "Alt + стрелки"), tr("move the selection a grid step (Shift: 10 cm)", "сдвинуть выделение на шаг сетки (Shift: 10 см)")),
            (tr("Alt + Page Up / Down", "Alt + Page Up / Down"), tr("selection up / down 10 cm (Shift: 1 m)", "выделение выше / ниже на 10 см (Shift: 1 м)")),
            ("Shift+click", tr("add to / take from the selection", "добавить к выделению / убрать из него")),
            (tr("Shift / Ctrl + drag on empty ground", "Shift / Ctrl + тянуть по пустой земле"), tr("select with a box (Shift adds)", "выделить рамкой (Shift: добавить)")),
            (tr("Double-click a road", "Двойной клик по дороге"), tr("select all of it", "выбрать её целиком")),
            ("Ctrl+wheel", tr("turn the object", "повернуть объект")),
            ("Shift+wheel", tr("brush size", "размер кисти")),
            ("Esc", tr("stop / deselect", "отмена / снять выбор")),
        ];
        egui::Grid::new("keys").striped(true).num_columns(2).show(ui, |ui| {
            for (k, v) in rows {
                if v.is_empty() {
                    ui.label(RichText::new(*k).strong().color(ACCENT));
                } else {
                    ui.label(RichText::new(*k).color(Color32::from_gray(230)));
                }
                ui.label(*v);
                ui.end_row();
            }
        });
        });
    });
    st.help_open = open;
}

/// A row of buttons at the right of a window, one line high (a right-to-left layout opened
/// straight in a window takes all the height there is and stretches the window).
pub fn button_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), Layout::right_to_left(Align::Center), add);
}

/// Ask before something unsaved is lost.
pub fn confirm_unsaved(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    if st.confirm.is_none() {
        return;
    }
    let mut decided: Option<bool> = None;
    let mut save = false;
    egui::Window::new(tr("Unsaved changes", "Несохранённые изменения")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
        ui.set_width(330.0);
        ui.horizontal(|ui| {
            ui.label(Icon::Info.text(28.0).color(ACCENT));
            ui.label(tr("The map has changes that are not saved. Save them first?", "На карте есть несохранённые изменения. Сохранить их?"));
        });
        ui.separator();
        button_row(ui, |ui| {
            if ui.button(tr("Cancel", "Отмена")).clicked() {
                decided = Some(false);
            }
            if ui.button(tr("Don't save", "Не сохранять")).clicked() {
                decided = Some(true);
            }
            if ui.button(icons::labelled(Icon::Save, tr("Save", "Сохранить"))).clicked() {
                save = true;
                decided = Some(true);
            }
        });
    });
    match decided {
        Some(true) => {
            if let Some(a) = st.confirm.take() {
                // (a save that fails keeps the map open with its changes)
                actions.push(if save { Action::SaveThen(Box::new(a)) } else { Action::Force(Box::new(a)) });
            }
        }
        Some(false) => st.confirm = None,
        None => {}
    }
}
