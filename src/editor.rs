//! The editor: the open map, the tools and what the mouse does with them.

use std::path::Path;
use std::time::Instant;

use glam::{DVec2, DVec3, Vec3};
use omsi_map::{EntryPoint, MapObject, MapSpline, TERRAIN_SAMPLES};

use crate::assets::Assets;
use crate::doc::{tile_of, tile_origin, tile_size, Document, Sel, TileKey};
use crate::history::History;
use crate::i18n::tr;
use crate::library::{Asset, Library};
use crate::render::{object_matrix, Brush, FrameExtras, Ghost, OverlayPoint};
use crate::reshape::{self, Handle};
use crate::roads::{self, Anchor, Seg};
use crate::selection::{self, Origin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Terrain,
    Paint,
    Roads,
    Objects,
    /// What the game uses but does not show: start points, stops, car parks, AI paths,
    /// sound sources and other helpers.
    Logic,
    Tiles,
}

impl Tool {
    pub const ALL: [Tool; 7] = [Tool::Select, Tool::Roads, Tool::Objects, Tool::Logic, Tool::Terrain, Tool::Paint, Tool::Tiles];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => tr("Select", "Выбор"),
            Tool::Terrain => tr("Terrain", "Рельеф"),
            Tool::Paint => tr("Paint", "Покраска"),
            Tool::Roads => tr("Roads", "Дороги"),
            Tool::Objects => tr("Objects", "Объекты"),
            Tool::Logic => tr("Game logic", "Служебное"),
            Tool::Tiles => tr("Tiles", "Тайлы"),
        }
    }

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Tool::Select => Icon::Select,
            Tool::Terrain => Icon::Terrain,
            Tool::Paint => Icon::Paint,
            Tool::Roads => Icon::Road,
            Tool::Objects => Icon::Objects,
            Tool::Logic => Icon::Sensors,
            Tool::Tiles => Icon::Tiles,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "1",
            Tool::Roads => "2",
            Tool::Objects => "3",
            Tool::Logic => "4",
            Tool::Terrain => "5",
            Tool::Paint => "6",
            Tool::Tiles => "7",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainMode {
    Raise,
    Lower,
    Smooth,
    Flatten,
    /// Rolls and dips of the natural ground.
    Roughen,
    /// An even slope from where it starts to where it ends, as wide as the brush.
    Ramp,
    /// A lake or a river bed: the ground under the brush dug to `water_depth` under the water's
    /// level (the level of the stroke's start, or the tile's water), water laid on the tiles
    /// it touches.
    Water,
}

/// How a lake or a river is drawn (the Lake / river mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterShape {
    /// Click the shore round, close it on the first point: the ground inside is dug to a bed.
    Lake,
    /// Click the river's middle line from its source on, finish on the last point (or Enter).
    River,
    /// The brush: dig by hand.
    Brush,
}

impl TerrainMode {
    pub const ALL: [TerrainMode; 7] = [TerrainMode::Raise, TerrainMode::Lower, TerrainMode::Smooth, TerrainMode::Flatten, TerrainMode::Roughen, TerrainMode::Ramp, TerrainMode::Water];
    pub fn label(self) -> &'static str {
        match self {
            TerrainMode::Raise => tr("Raise", "Поднять"),
            TerrainMode::Lower => tr("Lower", "Опустить"),
            TerrainMode::Smooth => tr("Smooth", "Сгладить"),
            TerrainMode::Flatten => tr("Flatten", "Выровнять"),
            TerrainMode::Roughen => tr("Roughen", "Неровности"),
            TerrainMode::Ramp => tr("Ramp", "Пандус"),
            TerrainMode::Water => tr("Lake / river", "Озеро / река"),
        }
    }
    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            TerrainMode::Raise => Icon::Raise,
            TerrainMode::Lower => Icon::Lower,
            TerrainMode::Smooth => Icon::Smooth,
            TerrainMode::Flatten => Icon::Flatten,
            TerrainMode::Roughen => Icon::Terrain,
            TerrainMode::Ramp => Icon::Route,
            TerrainMode::Water => Icon::Lower,
        }
    }
}

/// Where the knife would cut: the road (tile, id), how far along it and how long it is, the
/// point and the road's way there; `fits` while both pieces would be a metre or longer.
#[derive(Debug, Clone, Copy)]
pub struct CutPreview {
    pub key: TileKey,
    pub id: i64,
    pub at: f64,
    pub length: f64,
    pub pos: DVec3,
    pub dir: DVec2,
    pub fits: bool,
}

/// A short message at the bottom of the screen.
pub struct Toast {
    pub text: String,
    pub at: Instant,
    pub error: bool,
}

/// What the Logic tool places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicMode {
    /// `[entrypoint]`: where the bus appears.
    Start,
    /// `[busstop]` objects.
    Stops,
    /// `[carpark_p]`: where parked cars stand, one by one or in a row along a road.
    Parking,
    /// Invisible splines with lanes for the AI's cars or people.
    Paths,
    /// Sound sources, help arrows and other `[onlyeditor]` helpers.
    Other,
}

impl LogicMode {
    pub const ALL: [LogicMode; 5] = [LogicMode::Start, LogicMode::Stops, LogicMode::Parking, LogicMode::Paths, LogicMode::Other];

    pub fn label(self) -> &'static str {
        match self {
            LogicMode::Start => tr("Start points", "Точки старта"),
            LogicMode::Stops => tr("Bus stops", "Остановки"),
            LogicMode::Parking => tr("Car parks", "Парковки"),
            LogicMode::Paths => tr("AI paths", "Пути ИИ"),
            LogicMode::Other => tr("Other", "Прочее"),
        }
    }

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            LogicMode::Start => Icon::Flag,
            LogicMode::Stops => Icon::Bus,
            LogicMode::Parking => Icon::Parking,
            LogicMode::Paths => Icon::Route,
            LogicMode::Other => Icon::Sound,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            LogicMode::Start => tr("Where the bus appears when a drive starts. Put it on a road or in the depot, facing the way to drive.", "Где появляется автобус в начале поездки. Ставьте на дорогу или в депо, лицом по ходу движения."),
            LogicMode::Stops => tr("Where passengers wait. Put it beside the road at the kerb; name it on the right.", "Где ждут пассажиры. Ставьте у бордюра; имя задаётся справа."),
            LogicMode::Parking => tr("Where parked cars stand. Click for one; with a row on, click the start and end of a row along the kerb.", "Где стоят припаркованные машины. Клик: одно место; с рядом: клик в начале и в конце ряда вдоль бордюра."),
            LogicMode::Paths => tr("Lanes the AI's cars or people follow. Draw like a road: it snaps to lane ends (green: joined, red: dead end) and along lanes. A car path runs the way you draw it.", "Полосы, по которым едут машины ИИ или идут люди. Рисуется как дорога: прилипает к концам полос (зелёные: соединены, красные: тупик) и вдоль полос. Путь машин идёт в ту сторону, куда вы его рисуете."),
            LogicMode::Other => tr("Sound sources, help arrows and other helpers the game does not show.", "Источники звука, стрелки-подсказки и прочие помощники, которых не видно в игре."),
        }
    }
}

/// What the Select tool's drag does to the selection: any of it as it comes (Select), or
/// one thing only, by handles drawn on the selection (Move, Rotate, Scale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xform {
    Select,
    Move,
    Rotate,
    Scale,
}

impl Xform {
    pub const ALL: [Xform; 4] = [Xform::Select, Xform::Move, Xform::Rotate, Xform::Scale];

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Xform::Select => Icon::Select,
            Xform::Move => Icon::Move,
            Xform::Rotate => Icon::Rotate,
            Xform::Scale => Icon::Scale,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Xform::Select => tr("Select", "Выбор"),
            Xform::Move => tr("Move (V): drag an arrow along X, Y or Z, a square in its plane, the circle over the ground", "Перемещение (V): тяните стрелку вдоль X, Y или Z, квадрат — в его плоскости, круг — по земле"),
            Xform::Rotate => tr("Rotate (C): drag a ring to turn about X, Y or Z", "Поворот (C): тяните кольцо, чтобы вращать вокруг X, Y или Z"),
            Xform::Scale => tr("Scale (X): drag an arrow to scale along that axis, a square for two, the circle for all three", "Масштаб (X): тяните стрелку для масштаба по оси, квадрат — по двум, круг — по всем трём"),
        }
    }
}

/// What the left mouse button is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    None,
    Brush,
    /// Moving the selection: where it was grabbed; started once the mouse has gone far
    /// enough that it is not a click.
    Move { grab: DVec2, started: bool },
    /// A handle of the Move / Rotate / Scale gizmo held.
    Gizmo { grab: crate::gizmo::Grab, started: bool },
    /// A handle of the gizmo on the vertices, edges or faces of a mesh being edited.
    MeshGizmo { grab: crate::gizmo::Grab, started: bool },
    /// Dragging a grip of the selected spline.
    Reshape(TileKey, i64, Handle),
}

/// How far the mouse must go before a press on the selection moves it (m).
const MOVE_THRESHOLD: f64 = 0.3;

/// Blender's grab (G, in the Blender and OMSI key sets): the selection is taken hold of and
/// follows the pointer until Enter or a click lets it go (Esc or a right click calls it off).
/// X, Y or Z lock it to that axis (the same one again lets it go free); numbers typed say how
/// far it goes.
pub struct Grab {
    /// Where the selection stood when it was taken.
    from: Vec<Origin>,
    /// The height each thing had, for the Z lock.
    z: Vec<(Sel, f64)>,
    /// The road ends whose junctions follow it.
    arms: Vec<(Sel, Vec<selection::Arm>)>,
    /// The point the pointer was at when the grab began, and its place on the screen.
    start: DVec3,
    start_mouse: (f32, f32),
    /// Where the pointer is now (world, and its place on the screen).
    at: DVec3,
    mouse: (f32, f32),
    /// The axis locked to (0, 1, 2; None: the ground plane).
    pub axis: Option<u8>,
    /// A distance typed in: it says where it goes, the pointer no more, until it is cleared.
    typed: String,
    /// How far it has gone from where it was taken (m).
    pub delta: DVec3,
    /// What it was last asked to go (before the grid had its say): a frame that asks for the
    /// same again, as every frame does while the pointer is still, has nothing to move.
    asked: DVec3,
    /// Where what it carries is linked (read once, when it takes hold: reading it again every
    /// frame is walking the whole map for every piece moved).
    links: Option<selection::SplineWhere>,
}

impl Grab {
    /// The axis as it is written: "X", "Y", "Z", or nothing (free on the ground).
    pub fn axis_name(&self) -> &'static str {
        match self.axis {
            Some(0) => "X",
            Some(1) => "Y",
            Some(2) => "Z",
            _ => "",
        }
    }
}

pub struct Editor {
    pub doc: Option<Document>,
    pub history: History,
    pub tool: Tool,
    /// The selection: the one shown in the properties (None: nothing selected, whatever
    /// `also` holds) and the others selected with it.
    pub selected: Option<Sel>,
    pub also: Vec<Sel>,
    pub hover: Option<Sel>,
    /// The grip of the selected spline under the cursor.
    pub hover_handle: Option<Handle>,
    /// The box being dragged out on the screen (window points: where it began, where it is).
    pub select_box: Option<[(f32, f32); 2]>,
    /// What was copied.
    pub clip: Option<crate::clipboard::Clip>,
    /// Where the selection stood when a move began.
    move_from: Vec<Origin>,
    /// A reshape being dragged: its plan, or why it cannot be.
    reshape: Option<Result<reshape::Plan, &'static str>>,
    /// What the editor has learnt of the types it edits (models load in the background):
    /// objects' absolute height and spline helpers, splines' half widths (0: not a road one
    /// sees, the ground is left as it is).
    known_helpers: std::collections::HashMap<String, (bool, Vec<[f32; 4]>)>,
    known_half: std::collections::HashMap<String, f64>,
    /// The road ends the junctions being moved hold, and how far the move has gone.
    move_arms: Vec<(Sel, Vec<selection::Arm>)>,
    move_delta: DVec2,
    /// The Select tool's mode: plain selecting, or the Move / Rotate / Scale handles.
    pub xform: Xform,
    /// The view the handles are seen in this frame (camera, its size, the cursor).
    gizmo_view: Option<crate::gizmo::View>,
    /// The selected things when a handle was taken: height (above their ground or
    /// absolute), heading / pitch / bank and world position.
    move_z_from: Vec<f64>,
    move_rot_from: Vec<[f64; 3]>,
    move_world_from: Vec<DVec3>,
    /// The size each object being scaled has come to (its own axes), drawn so until it is let
    /// go; then the app gives them scaled copies of their files.
    pub scale_preview: Vec<(Sel, DVec3)>,
    pub scale_requests: Vec<(Sel, DVec3)>,
    /// What takes its state from an object and what that one takes its from (`[varparent]`),
    /// read from the whole map once and kept until the map or that object changes: the
    /// properties panel asks on every frame of a drag, and walking a map of a hundred
    /// thousand things for it was most of the frame.
    links_cache: Option<(u64, i64, (Option<(TileKey, i64)>, Vec<(TileKey, i64)>))>,
    /// The selected openMapper object's source, and its mesh being edited (Tab).
    pub mesh: Option<crate::meshedit::Session>,
    /// What a drag has come to, for the status bar: metres, degrees or a factor.
    pub xform_value: f64,
    /// How those ends would follow, shown while dragging.
    carry_preview: Vec<reshape::Plan>,
    /// A length and radius typed for a road, put in at the next tick (where the junctions
    /// its neighbour may end at are known).
    pub shape_request: Option<(TileKey, i64, f64, f64)>,
    /// GPS tracks shown to trace over (world metres; Map > GPS tracks to trace).
    pub trace_lines: Vec<Vec<DVec2>>,
    /// The camera asked to go over this point (a window's "go there").
    pub look_request: Option<DVec2>,
    /// The ground under the selected objects is to be levelled at the next tick.
    pub level_request: bool,
    /// The last click on something with Select, and when (a second soon on a road: its chain).
    last_click: Option<(Sel, Instant)>,
    last_mesh_click: Option<(crate::meshedit::Hit, Instant)>,
    /// openMapper's junctions whose arms changed: their plates are made again before the
    /// open history step is closed (the next tick, which has the models).
    pending_rebuild: Vec<i64>,
    /// New options for a junction being made again, and roads to add to it as arms.
    pending_options: std::collections::HashMap<i64, crate::junction::Options>,
    pending_arms: std::collections::HashMap<i64, Vec<crate::junction::Arm>>,
    /// A road being drawn from or into a junction plate (its IDCode, centre, radius).
    road_plate_start: Option<(i64, DVec3, f64)>,
    road_plate_end: Option<(i64, DVec3, f64)>,
    /// Switches the track being drawn lays where it leaves or joins another track.
    switch_plans: Vec<crate::switches::Plan>,
    /// The turnouts the installation and the mods offer (filled when the library is read).
    pub switch_catalogue: Vec<crate::switches::Candidate>,
    /// A road end being dragged onto a plate.
    reshape_plate: Option<(i64, DVec3, f64)>,
    /// The options of junctions shown in the properties (read from their files).
    pub junction_options: std::collections::HashMap<i64, crate::junction::Options>,
    /// The first IDCode of this session: junction files numbered from it on were made now,
    /// and undo may still need them.
    session_first_id: i64,
    /// How objects are put along roads.
    pub row_params: crate::rows::RowParams,
    /// Numbers typed while a road is drawn: length, radius, heading, rise (m) at its end.
    pub typed_text: [String; 4],
    /// Shift held while drawing: a straight.
    pub straight_lock: bool,
    /// The Logic tool's AI paths are being edited (picked, reshaped, split) rather than drawn.
    pub path_edit: bool,
    /// Ctrl / Cmd is held (a click on a selected AI path puts a point into it).
    pub ctrl_held: bool,
    /// The road drawn is planned again this frame.
    pub replan: bool,
    /// The Roads tool builds roundabouts: a click on the centre, then on the ring (or a
    /// drag out to it).
    pub roundabout: bool,
    /// The Roads tool's bus pocket tool: a click on a road lays a pocket there, until it is
    /// switched off. The road type of the pocket (one made with a pocket lane) and the length
    /// of the bay where it is full width (m).
    pub bus_pocket: bool,
    /// The custom junctions saved (read when first needed) and whether automatic junctions
    /// use those that fit.
    pub custom_layouts: Option<Vec<crate::cjunction::Layout>>,
    pub use_custom: bool,
    pub pocket_type: Option<String>,
    pub pocket_len: f64,
    /// The taper into and out of a bay: metres of road per metre the width changes, and
    /// whether the two ends are kept alike. Both ends are always tapered (never a step).
    pub pocket_taper_in: f64,
    pub pocket_taper_out: f64,
    pub pocket_alike: bool,
    /// The tapers given as their length in metres (else as a ratio), and those lengths.
    pub pocket_metres: bool,
    pub pocket_in_m: f64,
    pub pocket_out_m: f64,
    /// The width of the bay (m, 0: as the road type has it): a road type made in the road
    /// maker is written again with this pocket lane.
    pub pocket_width: f32,
    /// Shift the bay along the road to where it fits, if the click is too near a road's end
    /// for the tapers.
    pub pocket_fit: bool,
    ring_centre: Option<DVec3>,
    /// Roads keep their height (bridges, embankments): an even gradient from end to end,
    /// the ground left as it is.
    pub keep_height: bool,
    /// The ruler, when on: its first point and, once clicked, its second.
    pub ruler: Option<(Option<DVec3>, Option<DVec3>)>,
    /// The knife (K): a click on a road cuts it there, until right-click or Esc.
    pub cutting: bool,
    // snapping
    pub snap_grid: bool,
    pub grid_step: f64,
    pub snap_angle: bool,
    /// The angle snapping's step, and what R turns by (degrees).
    pub angle_step: f64,
    /// The open history step is the properties panel's.
    panel_step: bool,
    pub cursor: Option<DVec3>,
    /// The cursor is over the 3D view (not over a panel).
    pub cursor_in_view: bool,
    // terrain
    pub terrain_mode: TerrainMode,
    pub brush_radius: f32,
    /// How long the move handles may be (metres): the size of the selection, noted while drawing.
    gizmo_cap: std::cell::Cell<f64>,
    pub brush_strength: f32,
    flatten_to: Option<f32>,
    /// The water's level of the stroke going on (Lake / river), and how deep the bed is dug under it (m).
    water_level: Option<f32>,
    /// An AI path's pieces picked together for a move: the piece to pick alone again after it.
    /// The spline whose grip the pointer is on (with `hover_handle`), and the point of an AI path picked
    /// by a press on its grip (Del takes that point out, not the whole path).
    hover_handle_of: Option<(TileKey, i64)>,
    /// A press on a road's own lane while AI paths are edited: the road's pieces and the place, for
    /// the app to make the lanes editable (it has the folders) and pick the one clicked.
    pub lane_detach_request: Option<(Vec<(TileKey, i64)>, DVec2)>,
    path_point: Option<((TileKey, i64), Handle)>,
    /// "Connect to" is waiting for the other end of a path (Esc calls it off).
    path_connect: Option<((TileKey, i64), Handle)>,
    pub water_depth: f32,
    pub water_shape: WaterShape,
    /// A river's width and the banks' reach out from the shore (m).
    pub river_width: f32,
    pub water_bank: f32,
    /// The shore or the middle line clicked so far.
    pub water_pts: Vec<DVec3>,
    /// Flatten to this height rather than to where the drag starts.
    pub flatten_fixed: Option<f32>,
    // paint
    pub paint_layer: usize,
    pub paint_erase: bool,
    // roads
    pub road_type: Option<String>,
    road_start: Option<(Anchor, bool)>,
    road_plan: Option<(Vec<Seg>, f64, f64, Option<Anchor>)>,
    pub follow_ground: bool,
    /// What is built follows the ground: new roads, paths and tracks take its height along their
    /// way (off: they run level from where they start). Only what is built from now on; what is
    /// there stays as it is.
    pub auto_terrain: bool,
    /// Where a new road meets or crosses another one, cut both and put a junction there.
    pub auto_junctions: bool,
    /// Roads of different width are joined by a taper (Auto-Adapt).
    pub auto_adapt: bool,
    /// Where a track branches off another, put a switch there (off: the tracks just meet).
    pub auto_switches: bool,
    /// A road drawn near another's end or middle, or a junction piece's helper, joins it
    /// (off: roads are drawn exactly where clicked).
    pub snap_roads: bool,
    /// After a road is built the next one starts at its end (off: each road is drawn with
    /// two fresh clicks).
    pub keep_drawing: bool,
    /// How junctions treat each road type (corners, lights, crossings by themselves).
    pub road_styles: crate::road_maker::Styles,
    /// openOMSI's content folder (junction objects are written there) and the asphalt
    /// texture of their plates.
    pub content_dir: Option<std::path::PathBuf>,
    pub asphalt: Option<std::path::PathBuf>,
    pub road_snap: Option<Anchor>,
    road_snap_file: Option<String>,
    /// A road started from another one takes its type.
    pub inherit_type: bool,
    // objects
    pub object_type: Option<String>,
    pub place_heading: f64,
    pub align_to_road: bool,
    /// Aligned to a road, the object's front is put at the roadside, this far (m) from the
    /// road's edge (else it is only turned).
    pub align_front: bool,
    pub road_setback: f64,
    /// The road point an aligned object was set against (for the guide line).
    align_guide: Option<(DVec3, DVec3)>,
    /// Shift is held (places an object freely, not aligned).
    pub shift_held: bool,
    /// A press on what is selected and a drag of it moves it. False in OMSI 2's and Blender's
    /// key sets, where the move is G alone: a press there only picks (a drag of one piece
    /// never carries the whole selection off, which is what a hand used to those editors
    /// does not expect of a click).
    pub press_drags: bool,
    /// The Objects tool sows plants with a brush (or clears them).
    pub plant_brush: bool,
    pub plant_erase: bool,
    /// Plants no nearer to one another (m).
    pub plant_spacing: f64,
    /// What the brush sows, picked at random (empty: the catalogue's choice).
    pub plant_mix: Vec<String>,
    /// How the ground is painted along the selected roads (their properties).
    pub paint_along: crate::roadpaint::Along,
    /// Where a ramp of the Terrain tool starts (its first click).
    pub ramp_start: Option<DVec3>,
    /// The plant brush fills (or clears) an outline clicked round instead of painting.
    pub plant_area: bool,
    /// The outline's corners so far (world).
    pub plant_outline: Vec<DVec2>,
    /// The camera's distance at the last frame (how near a click must come to a corner).
    view_distance: std::cell::Cell<f64>,
    plant_rng: crate::plants::Rng,
    ghost_pos: Option<(DVec3, f64)>,
    /// A bus stop placed with "both sides": the one across the road (where, heading).
    stop_twin: Option<(DVec3, f64)>,
    /// A bus stop goes on both sides of the road at once.
    pub stop_both_sides: bool,
    /// The junction piece picked in a road's properties, and the search in its list.
    pub junction_piece: Option<String>,
    pub junction_search: String,
    /// The kind of bus stop the Game logic tool places.
    pub stop_kind: crate::stops::StopKind,
    // tiles
    hover_tile: Option<TileKey>,
    /// The tile clicked last with the Tiles tool (its overlay shown in the panel).
    pub picked_tile: Option<TileKey>,
    drag: Drag,
    /// The selection taken hold of by G (Blender's grab): it follows the pointer until Enter,
    /// a click, or Esc.
    pub grab: Option<Grab>,
    pub toasts: Vec<Toast>,
    pub show_grid: bool,
    // game logic
    pub logic: LogicMode,
    /// The spline type AI paths are drawn with.
    pub path_type: Option<String>,
    /// The road type while the Logic tool draws AI paths with the road machinery.
    stashed_road: Option<Option<String>>,
    /// Car parks go in a row from one click to the next.
    pub parking_row: bool,
    /// Between car parks in a row (m).
    pub parking_step: f64,
    parking_start: Option<DVec3>,
    /// Things the game does not show are drawn in the view.
    pub show_technical: bool,
    /// Layers hidden in the view, and so not picked either: trees, other objects, splines.
    pub hide: [bool; 3],
    /// The AI's lanes are drawn as arrows (always with the Logic tool).
    pub show_lanes: bool,
    /// The names of bus stops and start points float over them in the view.
    pub show_names: bool,
    /// A window picks things on the map (the Lines window its stops): a click only notes
    /// what is under the cursor and where, nothing is selected or moved.
    pub pick_mode: bool,
    pub picked: Option<(Option<Sel>, Option<DVec3>)>,
    /// An object waiting for the one it takes its state from (`[varparent]`) to be clicked.
    pub link_pick: Option<(TileKey, i64)>,
    /// The paths of what is selected are numbered in the view (its path rules are open).
    pub path_numbers: Option<Sel>,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            doc: None,
            history: History::default(),
            tool: Tool::Select,
            selected: None,
            also: Vec::new(),
            hover: None,
            hover_handle: None,
            select_box: None,
            clip: None,
            move_from: Vec::new(),
            press_drags: true,
            reshape: None,
            known_helpers: Default::default(),
            known_half: Default::default(),
            move_arms: Vec::new(),
            move_delta: DVec2::ZERO,
            xform: Xform::Select,
            gizmo_view: None,
            move_z_from: Vec::new(),
            move_rot_from: Vec::new(),
            move_world_from: Vec::new(),
            scale_preview: Vec::new(),
            links_cache: None,
            scale_requests: Vec::new(),
            mesh: None,
            xform_value: 0.0,
            carry_preview: Vec::new(),
            shape_request: None,
            trace_lines: Vec::new(),
            look_request: None,
            level_request: false,
            last_click: None,
            last_mesh_click: None,
            pending_rebuild: Vec::new(),
            pending_options: Default::default(),
            pending_arms: Default::default(),
            road_plate_start: None,
            road_plate_end: None,
            switch_plans: Vec::new(),
            switch_catalogue: Vec::new(),
            reshape_plate: None,
            junction_options: Default::default(),
            session_first_id: i64::MAX,
            row_params: Default::default(),
            ruler: None,
            cutting: false,
            typed_text: Default::default(),
            straight_lock: false,
            path_edit: false,
            ctrl_held: false,
            replan: false,
            roundabout: false,
            bus_pocket: false,
            custom_layouts: None,
            use_custom: true,
            pocket_type: None,
            pocket_len: 20.0,
            pocket_taper_in: POCKET_TAPER,
            pocket_taper_out: POCKET_TAPER,
            pocket_alike: true,
            pocket_metres: false,
            pocket_in_m: 18.0,
            pocket_out_m: 18.0,
            pocket_width: 0.0,
            pocket_fit: true,
            ring_centre: None,
            keep_height: false,
            snap_grid: false,
            grid_step: 1.0,
            snap_angle: false,
            angle_step: 15.0,
            panel_step: false,
            cursor: None,
            cursor_in_view: false,
            terrain_mode: TerrainMode::Raise,
            brush_radius: 25.0,
            gizmo_cap: std::cell::Cell::new(f64::INFINITY),
            brush_strength: 0.5,
            flatten_to: None,
            water_level: None,
            hover_handle_of: None,
            lane_detach_request: None,
            path_point: None,
            path_connect: None,
            water_depth: 1.5,
            water_shape: WaterShape::Lake,
            river_width: 14.0,
            water_bank: 8.0,
            water_pts: Vec::new(),
            flatten_fixed: None,
            paint_layer: 1,
            paint_erase: false,
            road_type: None,
            road_start: None,
            road_plan: None,
            follow_ground: true,
            auto_junctions: true,
            auto_adapt: true,
            auto_terrain: true,
            auto_switches: true,
            snap_roads: true,
            keep_drawing: false,
            road_styles: Default::default(),
            content_dir: None,
            asphalt: None,
            road_snap: None,
            road_snap_file: None,
            inherit_type: true,
            object_type: None,
            place_heading: 0.0,
            align_to_road: true,
            align_front: true,
            road_setback: 2.0,
            align_guide: None,
            shift_held: false,
            plant_brush: false,
            plant_erase: false,
            plant_spacing: 6.0,
            plant_mix: Vec::new(),
            paint_along: Default::default(),
            ramp_start: None,
            plant_area: false,
            plant_outline: Vec::new(),
            view_distance: std::cell::Cell::new(100.0),
            plant_rng: crate::plants::Rng::new(0x5eed),
            ghost_pos: None,
            stop_twin: None,
            stop_both_sides: true,
            stop_kind: Default::default(),
            junction_piece: None,
            junction_search: String::new(),
            hover_tile: None,
            picked_tile: None,
            drag: Drag::None,
            grab: None,
            toasts: Vec::new(),
            show_grid: true,
            logic: LogicMode::Start,
            path_type: None,
            stashed_road: None,
            parking_row: true,
            parking_step: 5.5,
            parking_start: None,
            show_technical: true,
            hide: [false; 3],
            show_lanes: false,
            show_names: true,
            pick_mode: false,
            picked: None,
            link_pick: None,
            path_numbers: None,
        }
    }
}

const ORANGE: [f32; 4] = [0.96, 0.50, 0.19, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
const GREEN: [f32; 4] = [0.35, 0.85, 0.45, 1.0];
/// Links between objects (`[varparent]`).
const LINK: [f32; 4] = [0.3, 0.85, 0.95, 0.9];
/// How near a road's own place a hit has to be to be that thing standing on the road: an
/// object stands *on* the road, so its foot and the road's surface are the same place, though
/// the aim meets the road's surface a few metres further along than the object's own face.
const ON_THE_ROAD: f64 = 1.5;

/// Where a ray enters a model's own box, seen through `m` (None: it does not pass through it).
///
/// Only a hint, and the crown of a tree: the box is far bigger than what a model draws (a
/// building's courtyard, the space between two wings), so a hit on it is not a hit on the
/// object - see `ray_model`.
fn ray_object_box(origin: glam::Vec3, dir: glam::Vec3, m: &glam::Mat4, lo: glam::Vec3, hi: glam::Vec3) -> Option<f32> {
    let inv = m.inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let mut t0 = 0.0f32;
    let mut t1 = f32::MAX;
    for axis in 0..3 {
        let (o, d) = (o[axis], d[axis]);
        let (lo, hi) = (lo[axis].min(hi[axis]), lo[axis].max(hi[axis]));
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let (mut a, mut b) = ((lo - o) / d, (hi - o) / d);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    (t1 >= 0.0).then_some(t0.max(0.0))
}

/// How far along a ray a model's own drawn faces are met (None: not met at all).
///
/// Three ways, in this order:
/// 1. its meshes, as they are;
/// 2. a tree's crown: a tree is drawn as two crossed cards of a texture, not as a mesh, so the
///    cards themselves are met - not the box round them (a box is a square as wide as the
///    crown, reaching the ground, so it stands in front of the pavement, the kerb, the road and
///    the lamp *round and under* every tree on the map: nothing near a tree could be clicked);
/// 3. a few centimetres to either side of the aim, as wide as a pixel is: a sign's board or a
///    bay's picture seen almost edge-on is met by neither of the two above.
/// Where a ray meets a tree, as it is drawn: two crossed cards `h` tall and `h * r` wide,
/// standing on the object's origin (`assets::tree_mesh`, the mesh the view draws them with).
///
/// A tree's box is a square that wide and reaches the ground: it covered the pavement, the
/// kerb and the road for metres round every trunk, and being nearer than the ground it took
/// the click from them - so on a map lined with trees, "most of it cannot be selected under a
/// tree". The cards are what a person sees, so they are what the aim is met against.
fn ray_tree(ov: glam::Vec3, dv: glam::Vec3, m: &glam::Mat4, h: f32, r: f32, mask: Option<&crate::assets::TreeMask>) -> Option<f64> {
    let w = (h * r * 0.5).max(0.05);
    let corner = |x: f32, y: f32, z: f32| m.transform_point3(glam::Vec3::new(x, y, z));
    let mut best: Option<f32> = None;
    for (ax, ay) in [(-w, 0.0f32), (0.0, -w)] {
        let (bx, by) = (-ax, -ay);
        // (the two triangles as `assets::tree_mesh` lays them out, each with its picture
        // coordinates: the card's left end is u 0, its foot v 1)
        let (a, b, c, d) = (corner(ax, ay, 0.0), corner(bx, by, 0.0), corner(bx, by, h), corner(ax, ay, h));
        let uvs = [(0.0f32, 1.0f32), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)];
        for (tri, uv) in [([0usize, 1, 2], [uvs[0], uvs[1], uvs[2]]), ([0, 2, 3], [uvs[0], uvs[2], uvs[3]])] {
            let corners = [a, b, c, d];
            let Some((t, bu, bv)) = omsi_geometry::ray_triangle_bary(ov, dv, corners[tri[0]], corners[tri[1]], corners[tri[2]]) else { continue };
            if let Some(mask) = mask {
                // (the picture's own shape: a gap between two branches is not a click on the tree)
                let u = uv[0].0 + (uv[1].0 - uv[0].0) * bu + (uv[2].0 - uv[0].0) * bv;
                let v = uv[0].1 + (uv[1].1 - uv[0].1) * bu + (uv[2].1 - uv[0].1) * bv;
                if !mask.solid(u, v) {
                    continue;
                }
            }
            best = Some(best.map(|q| q.min(t)).unwrap_or(t));
        }
    }
    best.map(|t| t as f64)
}

fn ray_model(ov: glam::Vec3, dv: glam::Vec3, m: &glam::Mat4, model: &crate::assets::ObjectModel) -> Option<f64> {
    ray_model_masked(ov, dv, m, model, None)
}

fn ray_model_masked(ov: glam::Vec3, dv: glam::Vec3, m: &glam::Mat4, model: &crate::assets::ObjectModel, mask: Option<&crate::assets::TreeMask>) -> Option<f64> {
    let meshes = |o: glam::Vec3| model.parts.iter().filter_map(|p| omsi_geometry::ray_mesh(o, dv, &p.mesh, m)).reduce(f32::min);
    if let Some(t) = meshes(ov) {
        return Some(t as f64);
    }
    if model.parts.is_empty() {
        return None;
    }
    if let Some((_, _, h, r)) = &model.tree {
        return ray_tree(ov, dv, m, *h, *r, mask);
    }
    if model.parts.is_empty() {
        return None;
    }
    // (the box only says how far away the aim is, so the aim's width there is known)
    let t0 = ray_object_box(ov, dv, m, model.lo, model.hi)?;
    let step = (t0 * 0.0025).clamp(0.01, 0.35);
    // (two directions across the ray)
    let up = if dv.z.abs() > 0.9 { glam::Vec3::X } else { glam::Vec3::Z };
    let a = dv.cross(up).normalize_or_zero() * step;
    let b = dv.cross(a).normalize_or_zero() * step;
    [(a, 1.0), (a, -1.0), (b, 1.0), (b, -1.0), (a, 3.0), (a, -3.0), (b, 3.0), (b, -3.0)]
        .into_iter()
        .filter_map(|(n, k)| meshes(ov + n * k))
        .reduce(f32::min)
        .map(|t| t as f64)
}

/// Where a ray passes a piece of a road: `(how far to the side, how far along)`.
///
/// The piece's own closest point to the ray is taken (the ray is a line, so this is a small
/// closed form), and then its height against the ray's, as the pick measures a road's surface:
/// a slanted aim meets a road's height at a different place than it passes closest to it.
/// Where a ray meets the vertical face a spline standing up from `a` to `b` is drawn as, over the
/// height `top`: `(lat, along)`, the first how far it passes to the side of that face.
///
/// A wall, a fence, a hedge or a railing is a *plane standing on its line*, and a click has to
/// meet that plane between the line and the top - not merely pass near the line. Measured as a
/// distance to the line itself, the tolerance reaches straight up as well as sideways: a
/// railing 1.2 m tall then took every click within a metre and a half *above* it, and the road,
/// the crossing, the pavement and the lamps behind it could not be picked at all. What is to the
/// side is a width; what is above is sky.
fn ray_wall_piece(o: DVec3, d: DVec3, a: DVec3, b: DVec3, top: f64) -> Option<(f64, f64)> {
    let u = b - a;
    let flat_len = (u.truncate()).length();
    if flat_len < 1e-9 {
        return None;
    }
    // (the face's own normal, level: the plane holds the segment and the vertical)
    let nu = DVec3::new(-u.y, u.x, 0.0) / flat_len;
    let den = d.dot(nu);
    let (lat, along) = if den.abs() < 1e-9 {
        // (the aim runs along the face: wherever it comes within a hair of it is the same place)
        ray_near_piece(o, d, a, b)?
    } else {
        let t = (a - o).dot(nu) / den;
        if t <= 0.0 {
            return None;
        }
        let p = o + d * t;
        // (the side: the horizontal distance from the face, with the segment's own direction
        // taken out - a slanted piece is a slanted plane, not a wider one)
        let off = p - a;
        let s = off.dot(u) / u.dot(u);
        if !(-0.05..=1.05).contains(&s) {
            return None;
        }
        let side = off - u * s.clamp(0.0, 1.0);
        (DVec3::new(side.x, side.y, 0.0).length(), t)
    };
    // (how high above its own line the aim meets the face; below the line is the ground under it)
    let p = o + d * along;
    let base = a.z + (b.z - a.z) * (((p - a).dot(u)) / u.dot(u)).clamp(0.0, 1.0);
    let h = p.z - base;
    if h < -0.30 || h > top + 0.05 {
        return None;
    }
    Some((lat, along))
}

fn ray_near_piece(o: DVec3, d: DVec3, a: DVec3, b: DVec3) -> Option<(f64, f64)> {
    let u = b - a;
    let au = u.length_squared();
    let w = a - o;
    let (bu, du, eu) = (u.dot(d), u.dot(w), d.dot(w));
    let mut s = 0.0;
    if au > 1e-12 {
        // (t, the place along the ray, is not needed: only the piece's own place on it)
        let den = bu * bu - au;
        if den.abs() > 1e-12 {
            let t = (bu * du - eu * au) / den;
            s = ((bu * t - du) / au).clamp(0.0, 1.0);
        }
    }
    let p = a + u * s;
    let rel = p - o;
    let along = rel.dot(d);
    if along <= 0.0 {
        return None;
    }
    let lat = (rel - d * along).length();
    let cross = if d.z.abs() > 0.1 { rel.z / d.z } else { along };
    Some((lat, cross))
}

impl Editor {
    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), at: Instant::now(), error: false });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), at: Instant::now(), error: true });
    }

    pub fn set_tool(&mut self, t: Tool) {
        if self.tool != t {
            self.cancel();
            self.tool = t;
            self.sync_path_type();
            self.leave_selection_for_paths();
        }
    }

    /// The selected roads' own AI lanes made editable: each road gets the copy of its type
    /// without lanes, and each lane becomes an AI path spline of its own (see `lanes_edit`).
    pub fn detach_lanes(&mut self, assets: &mut Assets, roots: &crate::install::Roots) {
        let pieces: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let Some(doc) = self.doc.as_ref() else { return };
        let mut free = std::collections::HashMap::new();
        for (k, id) in &pieces {
            let Some(file) = doc.spline(*k, *id).map(|s| s.file.clone()) else { continue };
            if free.contains_key(&file.to_lowercase()) {
                continue;
            }
            let Some(m) = assets.spline(&file) else { continue };
            if m.def.paths.is_empty() || m.is_ai_path() || m.def.paths.iter().any(|p| p.kind == 2) {
                continue;
            }
            match crate::lanes_edit::lane_free_copy(roots, &file) {
                Ok(copy) => {
                    free.insert(file.to_lowercase(), copy);
                }
                Err(e) => {
                    self.error(format!("{e}"));
                    return;
                }
            }
        }
        if free.is_empty() {
            self.toast(tr("These roads have no lanes of their own to take apart (rails stay as they are)", "У этих дорог нет полос, которые можно разобрать (рельсы остаются как есть)"));
            return;
        }
        self.history.begin(tr("Make the lanes editable", "Полосы стали редактируемыми"));
        let doc = self.doc.as_mut().unwrap();
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        let n = crate::lanes_edit::detach(doc, assets, &pieces, &free, "Splines\\Marcel\\invis_street.sli", "Splines\\invis_sidewalk.sli");
        self.history.commit();
        self.show_lanes = true;
        self.toast(format!("{} {n}", tr("Roads whose lanes are now editable:", "Дорог с редактируемыми полосами:")));
    }

    /// The whole map's relief taken away: every tile's ground levelled to the map's mean height
    /// (the picture of a satellite map stays; a map from OpenStreetMap needs none), the roads
    /// and the AI paths laid on it again and the junction plates brought to it; one undo step.
    /// How many roads were laid.
    pub fn flatten_terrain(&mut self) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let (sum, n) = doc.tiles.values().flat_map(|t| t.terrain.heights.iter()).fold((0.0f64, 0usize), |(s, n), h| (s + *h as f64, n + 1));
        if n == 0 {
            return 0;
        }
        let level = (sum / n as f64) as f32;
        let all: Vec<Sel> = doc.tiles.iter().flat_map(|(k, t)| t.tile.splines.iter().filter(|s| !s.deleted).map(move |s| Sel::Spline(*k, s.id))).collect();
        self.history.begin(tr("Flatten the terrain", "Выровнять рельеф"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in &keys {
            self.history.touch_tile(doc, *k);
        }
        self.history.touch_global(doc);
        let Some(doc) = self.doc.as_mut() else { return 0 };
        for t in doc.tiles.values_mut() {
            t.terrain.heights.iter_mut().for_each(|h| *h = level);
            t.touch_terrain();
        }
        // (the junction plates stand at the ground's height now)
        for t in doc.tiles.values_mut() {
            let mut touched = false;
            for o in t.tile.objects.iter_mut().filter(|o| is_own_junction(&o.file)) {
                o.pos[2] = level as f64;
                touched = true;
            }
            if touched {
                t.touch_content();
            }
        }
        crate::onground::lay(doc, &all);
        self.history.commit();
        all.len()
    }

    /// The AI paths are worked on: what was selected (a building, say) is let go, so the
    /// properties and the dots are the paths'.
    fn leave_selection_for_paths(&mut self) {
        if self.tool == Tool::Logic && self.logic == LogicMode::Paths {
            self.select(None);
        }
    }

    pub fn set_logic(&mut self, m: LogicMode) {
        if self.logic != m {
            self.cancel();
            self.logic = m;
            self.leave_selection_for_paths();
            self.object_type = None;
            self.ghost_pos = None;
            self.sync_path_type();
        }
    }

    /// AI paths are drawn as roads are: while they are, the road machinery's type is the
    /// path's, and the road type waits.
    fn sync_path_type(&mut self) {
        let paths = self.tool == Tool::Logic && self.logic == LogicMode::Paths;
        match (paths, self.stashed_road.is_some()) {
            (true, false) => self.stashed_road = Some(std::mem::replace(&mut self.road_type, self.path_type.clone())),
            (false, true) => self.road_type = self.stashed_road.take().unwrap_or(None),
            _ => {}
        }
    }

    /// Choose the AI path type (while drawing paths, the road machinery takes it at once).
    pub fn set_path_type(&mut self, rel: String) {
        self.path_type = Some(rel.clone());
        if self.stashed_road.is_some() {
            self.road_type = Some(rel);
        }
    }

    /// Drawing AI paths with the Logic tool.
    pub fn drawing_paths(&self) -> bool {
        self.tool == Tool::Logic && self.logic == LogicMode::Paths && !self.path_edit
    }

    /// AI paths are edited: only they can be picked.
    pub fn editing_paths(&self) -> bool {
        self.tool == Tool::Logic && self.logic == LogicMode::Paths && self.path_edit
    }

    /// The tool the mouse works as: the Logic tool draws paths as roads are drawn and
    /// places everything else as objects are placed.
    fn acts_as(&self) -> Tool {
        match self.tool {
            Tool::Logic if self.logic == LogicMode::Paths && self.path_edit => Tool::Select,
            Tool::Logic if self.logic == LogicMode::Paths => Tool::Roads,
            Tool::Logic => Tool::Objects,
            t => t,
        }
    }

    /// Stop what the tool is in the middle of.
    pub fn cancel(&mut self) {
        // (a grab is called off: nothing of it stands)
        if self.grab.take().is_some() {
            if let Some(doc) = &mut self.doc {
                self.history.rollback(doc);
            }
        }
        self.road_start = None;
        self.road_plan = None;
        self.parking_start = None;
        self.drag = Drag::None;
        self.reshape = None;
        self.select_box = None;
        self.ring_centre = None;
        self.plant_outline.clear();
        self.ramp_start = None;
        self.water_pts.clear();
        self.link_pick = None;
        self.path_connect = None;
        self.cutting = false;
        if let Some(doc) = &mut self.doc {
            self.history.commit();
            let _ = doc;
        }
    }

    /// Undo the last step. What the tool was in the middle of stops first: it may stand on
    /// what the undo takes away (a road drawn on from the end of the one undone).
    pub fn undo(&mut self) {
        if let Some(m) = self.mesh.as_mut().filter(|m| m.editing) {
            m.undo();
            return;
        }
        self.cancel();
        let groups = self.groups().to_vec();
        if let Some(d) = self.doc.as_mut() {
            self.history.undo(d);
        }
        if self.groups() != groups.as_slice() {
            self.save_groups();
        }
        self.select(None);
    }

    pub fn redo(&mut self) {
        if let Some(m) = self.mesh.as_mut().filter(|m| m.editing) {
            m.redo();
            return;
        }
        self.cancel();
        let groups = self.groups().to_vec();
        if let Some(d) = self.doc.as_mut() {
            self.history.redo(d);
        }
        if self.groups() != groups.as_slice() {
            self.save_groups();
        }
        self.select(None);
    }

    pub fn open(&mut self, doc: Document) {
        self.history.clear();
        self.select(None);
        self.hover = None;
        self.cancel();
        // (what was kept by IDCode belongs to the map that was open)
        self.junction_options.clear();
        self.pending_rebuild.clear();
        self.pending_options.clear();
        self.pending_arms.clear();
        self.paint_layer = if doc.global.ground_textures.len() > 1 { 1 } else { 0 };
        self.session_first_id = doc.global.next_id_code;
        let mut doc = doc;
        doc.groups = crate::mapgroups::load(&doc.dir);
        self.doc = Some(doc);
    }

    /// The map's groups (things a click selects and a drag moves as one).
    pub fn groups(&self) -> &[crate::mapgroups::MapGroup] {
        self.doc.as_ref().map(|d| d.groups.as_slice()).unwrap_or(&[])
    }

    /// A selection grown to the whole of the groups it touches.
    fn with_groups(&self, v: Vec<Sel>) -> Vec<Sel> {
        match self.doc.as_ref() {
            Some(doc) if !doc.groups.is_empty() => crate::mapgroups::expand(doc, &doc.groups, &v),
            _ => v,
        }
    }

    /// The groups changed (one undo step), and written beside the map.
    fn change_groups(&mut self, label: &str, f: impl FnOnce(&mut Vec<crate::mapgroups::MapGroup>)) {
        let Some(doc) = self.doc.as_mut() else { return };
        let before = doc.groups.clone();
        f(&mut doc.groups);
        if doc.groups != before {
            self.history.push_snapshot(label, crate::history::Snapshot { groups: Some(before), ..Default::default() });
            self.save_groups();
        }
    }

    /// The group (its place among the map's) the selection is all of, if it is one.
    pub fn selected_group(&self) -> Option<usize> {
        let sels = self.selection();
        let first = sels.first()?;
        let doc = self.doc.as_ref()?;
        let g = crate::mapgroups::group_of(&doc.groups, crate::mapgroups::id_of(*first))?;
        let all = crate::mapgroups::expand(doc, &doc.groups, &[*first]);
        (all.len() == sels.len() && all.iter().all(|s| sels.contains(s))).then_some(g)
    }

    /// Whether anything selected belongs to a group.
    pub fn selection_grouped(&self) -> bool {
        self.selection().iter().any(|s| crate::mapgroups::group_of(self.groups(), crate::mapgroups::id_of(*s)).is_some())
    }

    /// The selection made a group of its own (taken out of any it was in).
    pub fn group_selection(&mut self) {
        let ids: Vec<i64> = self.selection().iter().map(|s| crate::mapgroups::id_of(*s)).collect();
        if ids.len() < 2 {
            self.toast(tr("Select two things or more to group them", "Выделите два объекта или больше, чтобы сгруппировать"));
            return;
        }
        let name = crate::mapgroups::free_name(self.groups(), tr("Group", "Сгруппировать"));
        let n2 = name.clone();
        self.change_groups(tr("Group", "Сгруппировать"), move |groups| {
            for g in groups.iter_mut() {
                g.ids.retain(|id| !ids.contains(id));
            }
            groups.retain(|g| g.ids.len() >= 2);
            groups.push(crate::mapgroups::MapGroup { name: n2, ids });
        });
        self.toast(format!("{} «{name}»", tr("Grouped:", "Сгруппировано:")));
    }

    /// The groups the selection touches undone (their things stay where they are).
    pub fn ungroup_selection(&mut self) {
        let ids: Vec<i64> = self.selection().iter().map(|s| crate::mapgroups::id_of(*s)).collect();
        let before = self.groups().len();
        self.change_groups(tr("Ungroup", "Разгруппировать"), |groups| groups.retain(|g| !g.ids.iter().any(|id| ids.contains(id))));
        if self.groups().len() != before {
            self.toast(tr("Ungrouped", "Разгруппировано"));
        }
    }

    pub fn rename_group(&mut self, g: usize, name: &str) {
        let name = name.to_string();
        self.change_groups(tr("Rename group", "Имя группы"), move |groups| {
            if let Some(x) = groups.get_mut(g) {
                x.name = name;
            }
        });
    }

    /// Select every thing of a group.
    pub fn select_group(&mut self, g: usize) {
        let Some(doc) = self.doc.as_ref() else { return };
        let v: Vec<Sel> = doc.groups.get(g).map(|x| x.ids.iter().filter_map(|id| crate::mapgroups::sel_of(doc, *id)).collect()).unwrap_or_default();
        self.set_selection(v);
    }

    /// The groups written beside the map at once (they are not part of OMSI 2's files).
    fn save_groups(&mut self) {
        let Some(doc) = self.doc.as_ref() else { return };
        if let Err(e) = crate::mapgroups::save(&doc.dir, &doc.groups) {
            self.error(format!("groups.json: {e}"));
        }
    }

    // ---------------------------------------------------------------- picking

    /// Where a ray meets the ground (outside the map: the plane at height 0).
    pub fn ground_hit(&self, o: DVec3, d: DVec3) -> Option<DVec3> {
        let doc = self.doc.as_ref()?;
        let mut t = 0.0;
        let max = 20000.0;
        let mut prev = o;
        let above = |p: DVec3| -> Option<bool> { doc.height_at(p.truncate()).map(|h| p.z > h as f64) };
        let mut step = 2.0f64;
        while t < max {
            let p = o + d * t;
            match above(p) {
                Some(false) => {
                    // refine between prev and p
                    let (mut a, mut b) = (prev, p);
                    for _ in 0..24 {
                        let m = (a + b) * 0.5;
                        if above(m).unwrap_or(true) {
                            a = m;
                        } else {
                            b = m;
                        }
                    }
                    let q = (a + b) * 0.5;
                    return Some(DVec3::new(q.x, q.y, doc.ground(q.truncate())));
                }
                _ => {}
            }
            prev = p;
            step = (t * 0.01).clamp(1.0, 25.0);
            t += step;
        }
        let _ = step;
        // the plane at 0
        if d.z < -1e-6 {
            let t = -o.z / d.z;
            return Some(o + d * t);
        }
        None
    }

    /// The object or spline under a ray.
    pub fn pick(&self, assets: &mut Assets, o: DVec3, d: DVec3, ground: Option<DVec3>) -> Option<Sel> {
        let doc = self.doc.as_ref()?;
        let mut best: Option<(f64, Sel)> = None;
        // (relative to the ray's origin: f32 world positions far from the map's origin are
        // half a metre coarse)
        let ov = glam::Vec3::ZERO;
        let dv = d.as_vec3();
        let gdist = ground.map(|g| (g - o).length()).unwrap_or(f64::MAX);
        let paths_only = self.editing_paths();
        // (what is not drawn is not picked)
        let extras = FrameExtras { show_technical: self.show_technical || self.tool == Tool::Logic, hide: self.hide, ..Default::default() };
        let unseen = |m: &crate::assets::ObjectModel| m.technical && !extras.show_technical || crate::render::hidden_layer(m, &extras);
        for (k, t) in doc.tiles.iter().filter(|_| !paths_only) {
            for ob in &t.tile.objects {
                let Some(model) = assets.object(&ob.file) else { continue };
                if unseen(&model) {
                    continue;
                }
                let world = doc.object_world(*k, ob, model.abs_height);
                let m = object_matrix(world - o, ob.rot);
                let c = m.transform_point3(model.centre());
                let r = model.radius().max(1.0);
                if !omsi_geometry::ray_near_sphere(ov, dv, c, r) {
                    continue;
                }
                let hit = if let Some((dirs, tex, h, r)) = model.tree.clone() {
                    // (a tree is its cards, cut by its picture, meshes and all)
                    let mask = assets.tree_mask(&dirs, &tex);
                    let cards = ray_tree(ov, dv, &m, h, r, mask.as_deref());
                    let meshes = model.parts.iter().filter_map(|p| omsi_geometry::ray_mesh(ov, dv, &p.mesh, &m)).reduce(f32::min).map(|t| t as f64);
                    match (cards, meshes) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    }
                } else if model.parts.is_empty() {
                    // a marker: its sphere
                    let oc = c.as_dvec3();
                    let tca = oc.dot(d);
                    (tca > 0.0).then_some(tca)
                } else {
                    ray_model(ov, dv, &m, &model)
                };
                if let Some(t) = hit {
                    if t < gdist + 2.0 && best.map(|b| t < b.0).unwrap_or(true) {
                        best = Some((t, Sel::Object(*k, ob.id)));
                    }
                }
            }
        }
        // what hangs on objects and rows (signs on posts, lamps on masts), placed as they are drawn
        // (only near where the ray meets the ground: rows are worked out along their roads)
        let near = |k: &TileKey| ground.map(|g| (tile_origin(*k) + DVec2::splat(tile_size() * 0.5) - g.truncate()).length() < tile_size() * 1.5).unwrap_or(false);
        for (k, t) in doc.tiles.iter().filter(|_| !paths_only) {
            if t.tile.attach_objects.is_empty() || !near(k) {
                continue;
            }
            let mut parents: std::collections::HashMap<i64, (crate::place::Pose, std::sync::Arc<crate::assets::ObjectModel>)> = std::collections::HashMap::new();
            for ob in &t.tile.objects {
                let Some(model) = assets.object(&ob.file) else { continue };
                parents.insert(ob.id, (crate::place::Pose::of_object(doc.object_world(*k, ob, model.abs_height), ob.rot), model));
            }
            for a in t.tile.spline_attachments.iter().filter(|a| a.repeater.is_none()) {
                let Some(model) = assets.object(&a.file) else { continue };
                let objs = crate::place::row_objects(a, &t.tile.splines, tile_origin(*k));
                if let Some(first) = objs.iter().min_by_key(|o| o.index) {
                    parents.entry(a.id).or_insert((first.pose, model));
                }
            }
            for ob in &t.tile.attach_objects {
                let Some(model) = assets.object(&ob.file) else { continue };
                let Some((parent, pmodel)) = ob.parent_id.and_then(|p| parents.get(&p)).cloned() else { continue };
                let attach = pmodel.attachments.get(ob.attach_index).copied().unwrap_or(glam::Mat4::IDENTITY);
                let pose = parent.attached(&attach, omsi_geometry::map_rotation(ob.rot));
                parents.insert(ob.id, (pose, model.clone()));
                if unseen(&model) {
                    continue;
                }
                let m = pose.matrix_from(o);
                let c = m.transform_point3(model.centre());
                if !omsi_geometry::ray_near_sphere(ov, dv, c, model.radius().max(0.5)) {
                    continue;
                }
                let hit = if model.parts.is_empty() {
                    let tca = c.as_dvec3().dot(d);
                    (tca > 0.0).then_some(tca)
                } else {
                    ray_model(ov, dv, &m, &model)
                };
                if let Some(t) = hit {
                    if t < gdist + 2.0 && best.map(|b| t < b.0).unwrap_or(true) {
                        best = Some((t, Sel::Object(*k, ob.id)));
                    }
                }
            }
        }
        // the lamps, posts and bays that stand along a spline (`[splineAtt]` rows): each record
        // is a thing of its own (its copies are one object), so a click on one of its copies
        // picks that record and its properties.
        if !paths_only && !self.hide[1] {
            for (k, t) in doc.tiles.iter() {
                if t.tile.spline_attachments.is_empty() || !near(k) {
                    continue;
                }
                for a in &t.tile.spline_attachments {
                    let Some(model) = assets.object(&a.file) else { continue };
                    for ro in crate::selection::attachment_copies(doc, *k, a.id) {
                        let m = ro.pose.matrix_from(o);
                        let c = m.transform_point3(model.centre());
                        if !omsi_geometry::ray_near_sphere(ov, dv, c, model.radius().max(0.5)) {
                            continue;
                        }
                        let hit = if let Some((dirs, tex, h, r)) = model.tree.clone() {
                            let mask = assets.tree_mask(&dirs, &tex);
                            ray_tree(ov, dv, &m, h, r, mask.as_deref())
                        } else if model.parts.is_empty() {
                            let tca = c.as_dvec3().dot(d);
                            (tca > 0.0).then_some(tca)
                        } else {
                            ray_model(ov, dv, &m, &model)
                        };
                        if let Some(t) = hit {
                            if t < gdist + 2.0 && best.map(|b| t < b.0).unwrap_or(true) {
                                best = Some((t, Sel::Attached(*k, a.id)));
                            }
                        }
                    }
                }
            }
        }
        // splines: the one whose surface the ray meets first, in three dimensions. Ranked by
        // their place on the ground alone, a bridge's deck and the road under it, a fence and
        // the railing beside it, a marking and the road under that are one place - so the one
        // that came first in the map's order was always picked, and the other could not be
        // chosen at all, however plainly the camera was looking at it.
        //
        // `limit` is as far as an object is taken: what the ray meets after it has met the
        // ground is behind the hillside, not under the cursor.
        let limit = gdist + 2.0;
        let mut sbest: Option<(f64, f64, Sel)> = None;
        for (k, t) in doc.tiles.iter().filter(|_| !self.hide[2] || paths_only) {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let model = assets.spline(&s.file);
                let ai = model.as_ref().map(|m| m.is_ai_path()).unwrap_or(false);
                if paths_only && !ai {
                    continue;
                }
                // (an AI path is seen only as lines, with the lanes shown: otherwise not picked)
                if ai && !paths_only && !(self.show_lanes || self.show_technical) {
                    continue;
                }
                // (an AI path has no width of its own: a metre and a half either side)
                //
                // (something standing up from its line is a *face*, and a face has no
                // thickness: a railing drawn on a line of no width is met by an aim that meets
                // it, not by one that passes a quarter of a metre to the side of it. Something
                // lying on its line - a road, a marking, a kerb - keeps a hand's width of
                // tolerance, so that a line a hand's width wide can be clicked at all)
                let top = model.as_ref().map(|m| m.top() as f64).unwrap_or(0.0).max(0.0);
                let standing = top > 0.05;
                let slack = if standing { 0.06 } else { 0.25 };
                let hw = if ai { 1.5 } else { model.as_ref().map(|m| m.half_width as f64).unwrap_or(3.0).max(slack) };
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                // (the box round the whole road first: a map of this size has tens of thousands
                // of splines, and every mouse move looks at all of them)
                let centre = (c.point_at(s.length * 0.5) - o).as_vec3();
                let reach = (s.length * 0.5 + hw + 1.0) as f32;
                if !omsi_geometry::ray_near_sphere(ov, dv, centre, reach) {
                    continue;
                }
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                // (the road's own surface is where the ray meets *its height there*, not where
                // the ray first comes within the road's width. A ray at a slant comes within a
                // lane's width of a road several metres before it reaches it, so an object stood
                // on the road - a lamp, a bench, a low tree, a parking bay - was always behind
                // the road and could not be clicked; the road under a bridge could not be picked
                // either. The piece the ray passes nearest is the one whose height it meets.)
                //
                // (the pieces between the samples, not the samples themselves: a wall, a fence,
                // a hedge or a painted line is a tenth of a metre wide, and samples a metre
                // apart lie up to half a metre either side of the aim - so the thinnest splines,
                // which are exactly the ones that need aiming at, could not be clicked at all)
                //
                // (a wall, a fence, a hedge stands *up* from its line and is drawn from it to
                // `top`: clicking the middle of the face a person sees, two metres up, was a
                // click on the air above the line, so the tallest and most visible things on
                // the map - London's 4 m retaining walls - could not be picked at all. The aim
                // is met against the whole drawn height, sampled as finely as the road is wide)
                // (a thing standing up from its line is met as the face it is drawn as, over
                // the height it is drawn to; a road, a marking, a kerb lies on the line and is
                // met where the aim crosses its height)
                let mut nearest: Option<(f64, f64, Sel)> = None;
                let mut prev = c.point_at(0.0);
                for i in 1..=n {
                    let next = c.point_at(s.length * i as f64 / n as f64);
                    let met = if standing { ray_wall_piece(o, d, prev, next, top) } else { ray_near_piece(o, d, prev, next) };
                    if let Some((lat, cross)) = met {
                        if lat <= hw && cross > 0.0 && cross <= limit && nearest.map(|b| lat < b.0 || (lat == b.0 && cross < b.1)).unwrap_or(true) {
                            nearest = Some((lat, cross, Sel::Spline(*k, s.id)));
                        }
                    }
                    prev = next;
                }
                if let Some((lat, t, sel)) = nearest {
                    // (the nearest of two roads at the same place, not the first in the map: a
                    // painted line, a kerb and a pavement run side by side at one height, and a
                    // vertical aim meets all of them at the same distance - so the one the aim
                    // is really over is the one it passes closest to)
                    let better = match sbest {
                        Some((bt, blat, _)) => t < bt - 1e-3 || ((t - bt).abs() <= 1e-3 && lat < blat),
                        None => true,
                    };
                    if better {
                        sbest = Some((t, lat, sel));
                    }
                }
            }
        }
        // (whichever the ray reaches first: an object used to win over every spline, however far
        // behind it, so the road inside a tunnel of objects or under a canopy could not be picked)
        // (an object standing on a road is its own thing: the road's surface lies *under* its
        // foot, so the aim meets the road that much further along than the object's own face -
        // an object clicked over the road it stands on keeps the click)
        match (best, sbest) {
            (Some((a, sel)), Some((b, _, ssel))) => {
                let foot = (o + d * a).truncate();
                let under = (o + d * b).truncate();
                Some(if a <= b + 0.4 || (foot - under).length() <= ON_THE_ROAD { sel } else { ssel })
            }
            (Some((_, sel)), None) => Some(sel),
            (None, s) => s.map(|b| b.2),
        }
    }

    // ---------------------------------------------------------------- per frame

    /// Mouse moved or the view changed: update the cursor and what the tool shows.
    pub fn hover_ray(&mut self, assets: &mut Assets, _lib: &Library, o: DVec3, d: DVec3) {
        if self.doc.is_none() {
            return;
        }
        let g = self.ground_hit(o, d);
        self.cursor = g;
        self.hover = None;
        self.hover_tile = None;
        match self.acts_as() {
            Tool::Select => {
                if self.drag == Drag::None {
                    let found = self.handle_at_ray(assets, o, d);
                    self.hover_handle = found.map(|f| f.1);
                    self.hover_handle_of = found.map(|f| f.0);
                    self.hover = if self.hover_handle.is_some() { self.selected } else { self.pick(assets, o, d, g) };
                }
            }
            Tool::Tiles => {
                self.hover_tile = g.map(|p| tile_of(p.truncate()));
            }
            Tool::Roads => self.plan_road(assets),
            // (Game logic: a stop, start point or other helper already there is picked to
            // select and move, not put again on top of it)
            Tool::Objects if self.tool == Tool::Logic && self.drag == Drag::None => {
                self.hover = self.pick(assets, o, d, g).filter(|s| self.is_logic_object(assets, *s));
                if self.hover.is_some() {
                    self.ghost_pos = None;
                    self.stop_twin = None;
                } else {
                    self.plan_object(assets, _lib);
                }
            }
            Tool::Objects => self.plan_object(assets, _lib),
            _ => {}
        }
    }

    /// What the Game logic tool picks rather than places over: a bus stop, a start point or
    /// any helper the game does not show (car parks, sound sources, …).
    fn is_logic_object(&self, assets: &mut Assets, s: Sel) -> bool {
        let Sel::Object(k, id) = s else { return false };
        let Some(o) = self.doc.as_ref().and_then(|d| selection::any_object(d, k, id)) else { return false };
        let file = o.file.clone();
        o.flag == 7 || self.doc.as_ref().map(|d| d.global.entry_points.iter().any(|e| e.object_id == id)).unwrap_or(false) || assets.object(&file).map(|m| m.technical).unwrap_or(false)
    }

    /// The cursor at a ground point (scripted use: the demo and tests).
    pub fn hover_point(&mut self, assets: &mut Assets, _lib: &Library, p: DVec3) {
        self.cursor = Some(p);
        self.cursor_in_view = true;
        match self.acts_as() {
            Tool::Roads => self.plan_road(assets),
            Tool::Objects => self.plan_object(assets, _lib),
            Tool::Tiles => self.hover_tile = Some(tile_of(p.truncate())),
            _ => {}
        }
    }

    /// Continuous work while a button is held (brushes, dragging).
    pub fn tick(&mut self, assets: &mut Assets, dt: f64, held: bool) {
        self.learn(assets);
        if let Some((k, id, length, radius)) = self.shape_request.take() {
            self.set_spline_shape(assets, k, id, length, radius);
        }
        // (once the objects' models are there: their outlines are needed)
        if self.level_request {
            let files: Vec<String> = self.doc.as_ref().map(|d| self.selection().iter().filter_map(|s| if let Sel::Object(k, id) = s { d.object(*k, *id).map(|o| o.file.clone()) } else { None }).collect()).unwrap_or_default();
            if files.iter().all(|f| assets.object(f).is_some() || assets.object_failed(f)) {
                self.level_request = false;
                let n = self.level_under_objects(assets);
                self.toast(format!("{} {n}", tr("Ground levelled under objects:", "Земля выровнена под объектами:")));
            }
        }
        // (an undo in between closed the step: the plates are as they were)
        if !self.pending_rebuild.is_empty() && self.drag == Drag::None {
            if self.history.is_open() {
                self.rebuild_pending(assets);
                self.history.commit();
            }
            self.pending_rebuild.clear();
            self.pending_options.clear();
            self.pending_arms.clear();
        }
        // (numbers typed or Shift changed: the road drawn follows at once)
        if self.replan && self.acts_as() == Tool::Roads {
            self.plan_road(assets);
        }
        self.replan = false;
        if !held {
            match std::mem::replace(&mut self.drag, Drag::None) {
                Drag::None => {}
                Drag::Move { started, .. } => {
                    if started {
                        let sels = self.selection();
                        if let Some(doc) = self.doc.as_mut() {
                            selection::unlink_outside(doc, &mut self.history, &sels);
                        }
                        let arms = std::mem::take(&mut self.move_arms);
                        self.carry(&arms, DVec2::ZERO, 0.0, self.move_delta, 0.0);
                        self.ground_to_selection();
                        self.rebuild_pending(assets);
                    }
                    self.carry_preview.clear();
                    self.history.commit();
                }
                Drag::MeshGizmo { started, .. } => {
                    if let (true, Some(m)) = (started, self.mesh.as_mut()) {
                        m.end_drag();
                    }
                }
                Drag::Gizmo { grab, started } => {
                    if started {
                        self.finish_gizmo(assets, grab);
                    }
                    self.carry_preview.clear();
                    // (a scale is finished by the app, which makes the scaled files: in the same step)
                    if self.scale_requests.is_empty() {
                        self.history.commit();
                    }
                }
                Drag::Reshape(k, id, _) => self.finish_reshape(k, id),
                Drag::Brush => self.history.commit(),
            }
            return;
        }
        let Some(c) = self.cursor else { return };
        match self.drag {
            Drag::Brush => match self.tool {
                Tool::Terrain => self.brush_terrain(assets, c.truncate(), dt),
                Tool::Paint => self.brush_paint(c.truncate(), dt),
                Tool::Objects => self.brush_plants(assets, c.truncate(), dt),
                _ => {}
            },
            Drag::Move { grab, started } => {
                let raw = c.truncate() - grab;
                if !started {
                    if raw.length() < MOVE_THRESHOLD || self.move_from.is_empty() {
                        return;
                    }
                    self.history.begin(tr("Move", "Перемещение"));
                    self.drag = Drag::Move { grab, started: true };
                    self.move_arms = self.arms_for(&self.selection());
                }
                // (the grid holds the first one's place; the rest keep their distances)
                let first = self.move_from[0].pos;
                let delta = self.snap_point(first + raw) - first;
                let Some(doc) = self.doc.as_mut() else { return };
                let now = selection::transform(doc, &mut self.history, &self.move_from, DVec2::ZERO, 0.0, delta);
                // (the roads a junction holds follow when it is let go; meanwhile their new shape
                // is shown)
                let all: Vec<selection::Arm> = self.move_arms.iter().flat_map(|g| g.1.iter().copied()).collect();
                self.carry_preview = selection::carry_plans(doc, &all, DVec2::ZERO, 0.0, delta, 0.0, &|_| false).into_iter().flatten().collect();
                self.move_delta = delta;
                self.set_selection_unique(now);
            }
            Drag::Gizmo { grab, started } => self.drag_gizmo(grab, started),
            Drag::MeshGizmo { grab, started } => {
                let (Some(view), Some(m)) = (self.gizmo_view.clone(), self.mesh.as_mut()) else { return };
                if !started {
                    if !grab.moved(&view) {
                        return;
                    }
                    m.begin_drag();
                    self.drag = Drag::MeshGizmo { grab, started: true };
                }
                let change = grab.change(&view, self.snap_angle.then_some(self.angle_step));
                m.drag(change, grab.gizmo.pivot);
                self.xform_value = match change {
                    crate::gizmo::Change::Move(d) => d.length(),
                    crate::gizmo::Change::Turn(_, deg) => deg,
                    crate::gizmo::Change::Scale(f) => f.max_element().max(f.min_element()),
                };
            }
            Drag::Reshape(k, id, h) => {
                let mut p = self.snap_point(c.truncate());
                // an end dropped on a junction plate goes to its centre and becomes an arm
                let ai = self.editing_paths();
                self.reshape_plate = if h == Handle::Mid || ai { None } else { self.plate_at(assets, c.truncate()) };
                if let Some((_, centre, _)) = self.reshape_plate {
                    p = centre.truncate();
                }
                // (an AI path's end dragged to the end of a lane: it goes there, tangent to it, to join it)
                let kind = self.doc.as_ref().and_then(|d| d.spline(k, id)).and_then(|s| assets.spline(&s.file)).and_then(|m| m.def.paths.first().map(|q| q.kind));
                let lane = match (ai, h, kind) {
                    (true, Handle::Start | Handle::End, Some(kind)) => self.snap_lane_kind(assets, c.truncate(), h == Handle::Start, kind).filter(|(a, _)| a.mid.is_none()).map(|(a, _)| a),
                    _ => None,
                };
                if let Some(a) = &lane {
                    p = a.pos.truncate();
                }
                let joined = self.helper_anchors(assets);
                let locked = |q: DVec2| joined.iter().any(|j| (j.truncate() - q).length() < 0.6);
                let Some(doc) = self.doc.as_ref() else { return };
                let ground = |q: DVec2| doc.ground(q);
                self.reshape = Some(match h {
                    Handle::Start | Handle::End => match &lane {
                        Some(a) => reshape::drag_end_aligned(doc, k, id, h == Handle::End, p, roads::norm_deg(a.heading + 180.0), &ground, &|_| false).or_else(|_| reshape::drag_end(doc, k, id, h == Handle::End, p, &ground, &locked)),
                        None => reshape::drag_end(doc, k, id, h == Handle::End, p, &ground, &locked),
                    },
                    // (an AI path bent in the middle keeps the way it leaves and meets the lanes it is joined to)
                    Handle::Mid if ai => reshape::drag_mid(doc, k, id, p, &|_| true).or_else(|_| reshape::drag_mid(doc, k, id, p, &locked)),
                    Handle::Mid => reshape::drag_mid(doc, k, id, p, &locked),
                });
            }
            Drag::None => {}
        }
    }

    // ---------------------------------------------------------------- clicks

    pub fn left_press(&mut self, assets: &mut Assets, lib: &Library, shift: bool) {
        if self.doc.is_none() {
            return;
        }
        // "Connect to": the next click is the other end of the path, not a new selection
        if self.path_connect.is_some() && self.editing_paths() {
            match (self.hover_handle, self.hover_handle_of) {
                (Some(h), Some(to)) => {
                    let from = self.path_connect.take().unwrap();
                    self.connect_path_points(from, (to, h));
                }
                _ => self.toast(tr("Click an end point of a path", "Щёлкните конечную точку пути")),
            }
            return;
        }
        if let (Some((k, id)), Tool::Select) = (self.link_pick, self.tool) {
            self.link_pick = None;
            match self.hover {
                Some(Sel::Object(_, p)) if p != id => self.link_to(k, id, p),
                _ => self.toast(tr("No object clicked: nothing linked", "Объект не выбран: ничего не связано")),
            }
            return;
        }
        if self.pick_mode && self.tool == Tool::Select {
            self.picked = Some((self.hover, self.cursor));
            return;
        }
        // the knife takes the clicks while it is on: each cuts the road under the cursor
        if self.cutting {
            match self.cut_preview() {
                Some(c) if c.fits => self.cut_spline(c.key, c.id, c.at),
                Some(_) => self.error(tr("Too near the road's end to cut it there", "Слишком близко к концу дороги, чтобы разрезать её здесь")),
                None => self.toast(tr("No road under the cursor", "Под курсором нет дороги")),
            }
            return;
        }
        // the ruler takes the clicks while it is on: from one point to the next
        if let (Some(r), Some(c)) = (&mut self.ruler, self.cursor) {
            *r = match *r {
                (Some(a), None) => (Some(a), Some(c)),
                _ => (Some(c), None),
            };
            return;
        }
        // (editing AI paths: a press on a road's own lane makes the road's lanes editable and picks it)
        if self.editing_paths() && self.hover.is_none() && self.hover_handle.is_none() {
            if let Some(req) = self.road_lane_under_cursor(assets) {
                self.lane_detach_request = Some(req);
                return;
            }
        }
        // (Game logic: a press on one already there selects it, and a drag moves it)
        if self.tool == Tool::Logic && self.acts_as() == Tool::Objects && self.hover.is_some() {
            self.select_press(shift);
            return;
        }
        match self.acts_as() {
            Tool::Select => self.select_press(shift),
            Tool::Terrain if self.tool == Tool::Terrain && self.terrain_mode == TerrainMode::Ramp => {
                let Some(c) = self.cursor else { return };
                match self.ramp_start.take() {
                    None => self.ramp_start = Some(c),
                    Some(a) => {
                        let grade = self.ramp(a, c);
                        self.toast(format!("{} {:+.1} %", tr("Ramp:", "Пандус:"), grade));
                    }
                }
            }
            Tool::Terrain if self.tool == Tool::Terrain && self.terrain_mode == TerrainMode::Water && self.water_shape != WaterShape::Brush => {
                let Some(c) = self.cursor else { return };
                let near = |p: Option<&DVec3>| p.map(|p| (p.truncate() - c.truncate()).length() < self.close_radius()).unwrap_or(false);
                let done = match self.water_shape {
                    WaterShape::Lake => self.water_pts.len() >= 3 && near(self.water_pts.first()),
                    _ => self.water_pts.len() >= 2 && near(self.water_pts.last()),
                };
                if done {
                    self.make_water();
                } else {
                    self.water_pts.push(c);
                }
            }
            Tool::Terrain | Tool::Paint => {
                self.history.begin(if self.tool == Tool::Terrain { tr("Shape terrain", "Рельеф") } else { tr("Paint ground", "Покраска") });
                self.flatten_to = self.flatten_fixed.or(self.cursor.map(|c| c.z as f32));
                // (a lake's surface: the tile's water if it has some, else a little under where the stroke starts)
                self.water_level = self.cursor.map(|c| {
                    let tile = self.doc.as_ref().and_then(|d| d.tiles.get(&tile_of(c.truncate())).and_then(|t| t.water));
                    tile.map(|w| w.iter().sum::<f32>() / 4.0).unwrap_or(c.z as f32 - 0.3)
                });
                self.drag = Drag::Brush;
            }
            Tool::Roads if self.bus_pocket && !self.drawing_paths() => self.pocket_click(assets),
            Tool::Roads if self.roundabout && !self.drawing_paths() => self.ring_click(assets),
            Tool::Roads => self.road_click(assets, lib),
            Tool::Objects if self.tool == Tool::Logic && self.logic == LogicMode::Parking && self.parking_row => self.parking_click(assets, lib),
            Tool::Objects if self.tool == Tool::Objects && self.plant_brush && self.plant_area => {
                let Some(c) = self.cursor else { return };
                // (a click on the first corner closes the outline)
                let close = self.plant_outline.len() >= 3 && (self.plant_outline[0] - c.truncate()).length() < self.close_radius();
                if close {
                    self.finish_plant_outline(assets);
                } else {
                    self.plant_outline.push(c.truncate());
                }
            }
            Tool::Objects if self.tool == Tool::Objects && self.plant_brush => {
                self.history.begin(if self.plant_erase { tr("Clear plants", "Убрать растения") } else { tr("Plant", "Посадка") });
                self.drag = Drag::Brush;
            }
            Tool::Objects => {
                self.place_object(lib, shift);
            }
            Tool::Tiles => self.tile_click(),
            Tool::Logic => {}
        }
    }

    /// The left button let go; `dragged`: the mouse moved since it went down (a road is
    /// built by dragging from its start to its end, as well as by two clicks).
    pub fn left_release(&mut self, assets: &mut Assets, lib: &Library, dragged: bool) {
        if self.acts_as() == Tool::Roads && self.roundabout && dragged && self.ring_centre.is_some() {
            self.ring_click(assets);
            return;
        }
        if self.acts_as() == Tool::Roads && dragged && self.road_start.is_some() && self.road_plan.is_some() {
            self.road_click(assets, lib);
        }
    }

    pub fn road_start_active(&self) -> bool {
        self.road_start.is_some()
    }

    /// A right click without a drag: what the tool was doing is called off; true when the
    /// view's menu is to open instead.
    pub fn right_click(&mut self) -> bool {
        if let Some(m) = self.mesh.as_mut().filter(|m| m.modal_active()) {
            m.cancel();
            return false;
        }
        if self.cutting {
            self.cutting = false;
            return false;
        }
        if self.ruler.is_some() {
            self.ruler = None;
            return false;
        }
        match self.acts_as() {
            Tool::Roads if self.ring_centre.is_some() => {
                self.ring_centre = None;
            }
            Tool::Roads if self.road_start.is_some() => {
                self.road_start = None;
                self.road_plan = None;
            }
            Tool::Terrain if self.ramp_start.is_some() => {
                self.ramp_start = None;
            }
            Tool::Terrain if !self.water_pts.is_empty() => {
                self.water_pts.pop();
            }
            Tool::Objects if !self.plant_outline.is_empty() => {
                self.plant_outline.pop();
            }
            Tool::Objects if self.parking_start.is_some() => {
                self.parking_start = None;
            }
            Tool::Objects if self.object_type.is_some() => {
                self.object_type = None;
                self.ghost_pos = None;
            }
            Tool::Tiles => {
                if let Some(k) = self.hover_tile {
                    self.remove_tile(k);
                }
            }
            // (Select, and Game logic on a thing of its own: what is under the cursor is
            // selected, its group with it, and the view's menu opens)
            Tool::Select => {
                if let Some(h) = self.hover.filter(|h| !self.is_selected(*h)) {
                    let v = if self.ctrl_held { vec![h] } else { self.with_groups(vec![h]) };
                    self.set_selection(v);
                }
                return true;
            }
            Tool::Objects if self.tool == Tool::Logic && self.hover.is_some() => {
                if let Some(h) = self.hover.filter(|h| !self.is_selected(*h)) {
                    let v = self.with_groups(vec![h]);
                    self.set_selection(v);
                }
                return true;
            }
            _ => {
                self.select(None);
            }
        }
        false
    }

    /// Turn what is placed or selected (degrees): a selection turns round its middle.
    pub fn rotate(&mut self, deg: f64) {
        match self.acts_as() {
            Tool::Objects => self.place_heading = self.snap_heading(roads::norm_deg(self.place_heading + deg)),
            _ => {
                let sels = self.selection();
                let arms = self.arms_for(&sels);
                let Some(doc) = self.doc.as_mut() else { return };
                let Some(pivot) = selection::centre(doc, &sels) else { return };
                let from: Vec<Origin> = sels.iter().filter_map(|s| selection::origin_of(doc, *s)).collect();
                // (one thing alone, with angle snapping on, comes to a whole step)
                let deg = match (&from[..], self.snap_angle) {
                    ([one], true) => snap_deg(one.heading + deg, self.angle_step) - one.heading,
                    _ => deg,
                };
                self.history.begin(if sels.len() > 1 { tr("Rotate selection", "Поворот выделенного") } else { tr("Rotate", "Поворот") });
                let now = selection::transform(doc, &mut self.history, &from, pivot, deg, DVec2::ZERO);
                selection::unlink_outside(doc, &mut self.history, &now);
                self.set_selection_unique(now);
                self.carry(&arms, pivot, deg, DVec2::ZERO, 0.0);
                self.ground_to_selection();
                self.finish_step();
            }
        }
    }

    pub fn delete_selected(&mut self) {
        // (an AI path's point picked: that point only)
        if self.editing_paths() && self.path_point.is_some() {
            self.remove_path_point();
            return;
        }
        let sels = self.selection();
        if self.doc.is_none() || sels.is_empty() {
            return;
        }
        self.history.begin(match sels[..] {
            [Sel::Object(..)] => tr("Delete object", "Удаление объекта"),
            [Sel::Spline(..)] => tr("Delete road", "Удаление дороги"),
            [Sel::Attached(..)] => tr("Delete object along a road", "Удаление объекта вдоль дороги"),
            _ => tr("Delete selection", "Удаление выделенного"),
        });
        // a junction's signal heads go with it
        let mut sels = sels;
        if let Some(doc) = self.doc.as_ref() {
            let kids: Vec<Sel> = sels.iter().filter_map(|s| if let Sel::Object(_, id) = s { Some(*id) } else { None }).flat_map(|id| crate::junction::children(doc, id)).map(|(k, c)| Sel::Object(k, c)).collect();
            sels.extend(kids.into_iter().filter(|k| !sels.contains(k)).collect::<Vec<_>>());
        }
        // the junctions that lose an arm are made again with the rest (two left: a joint)
        let hit = self.junctions_at_ends(&sels);
        let Some(doc) = self.doc.as_mut() else { return };
        selection::delete(doc, &mut self.history, &sels);
        self.pending_rebuild.extend(hit);
        self.finish_step();
        self.select(None);
    }

    // ---------------------------------------------------------------- objects

    fn plan_object(&mut self, assets: &mut Assets, lib: &Library) {
        self.ghost_pos = None;
        self.stop_twin = None;
        self.align_guide = None;
        if self.tool == Tool::Objects && self.plant_brush {
            return;
        }
        let (Some(c), Some(rel)) = (self.cursor, self.object_type.clone()) else { return };
        let mut heading = self.place_heading;
        let mut pos = c;
        if self.snap_grid {
            let q = self.snap_point(c.truncate());
            pos = DVec3::new(q.x, q.y, self.doc.as_ref().map(|d| d.ground(q)).unwrap_or(c.z));
        }
        // a start point, a car park or a bus stop stands in the traffic's way on its side of
        // the road: a start point on the lane itself, a car park and a stop at the kerb
        let (entry, park, stop) = lib.find(&rel).map(|a| (a.is_entry_point, a.is_car_park, a.is_bus_stop)).unwrap_or((false, false, false));
        if self.align_to_road && !self.shift_held && (entry || park || stop) {
            if let Some(doc) = self.doc.as_ref() {
                let lht = doc.global.left_hand_traffic;
                let (lanes, _) = crate::lanes::near(doc, assets, c.truncate(), 25.0);
                let best = lanes.iter().filter(|l| l.kind == 0).filter_map(|l| l.nearest(c.truncate()).map(|n| (n, l.direction))).filter(|(n, _)| n.2 < 12.0).min_by(|a, b| a.0 .2.total_cmp(&b.0 .2));
                if let Some(((q, h, _, _), dir)) = best {
                    let flow = if dir == 1 { roads::norm_deg(h + 180.0) } else { h };
                    // (the kerb: right of the traffic where it keeps right)
                    let kerb = roads::right(flow) * if lht { -1.0 } else { 1.0 };
                    let at = if entry { q.truncate() } else if park { q.truncate() + kerb * 2.6 } else { q.truncate() + kerb * 3.2 };
                    let z = doc.ground(at);
                    let pos = DVec3::new(at.x, at.y, z);
                    self.align_guide = Some((DVec3::new(q.x, q.y, z), pos));
                    self.ghost_pos = Some((pos, roads::norm_deg(flow + self.place_heading)));
                    // the stop across the road: at the kerb of the nearest lane going the
                    // other way
                    if stop && self.stop_both_sides {
                        let back = |h: f64| ((h - flow + 540.0).rem_euclid(360.0) - 180.0).abs() > 135.0;
                        let twin = lanes
                            .iter()
                            .filter(|l| l.kind == 0 && l.direction != 2)
                            .filter_map(|l| l.nearest(q.truncate()).map(|n| (n, if l.direction == 1 { roads::norm_deg(n.1 + 180.0) } else { n.1 })))
                            .filter(|((_, _, d, _), f)| *d > 0.5 && *d < 15.0 && back(*f))
                            .min_by(|a, b| a.0 .2.total_cmp(&b.0 .2));
                        if let Some(((q2, _, _, _), flow2)) = twin {
                            let kerb2 = roads::right(flow2) * if lht { -1.0 } else { 1.0 };
                            let at2 = q2.truncate() + kerb2 * 3.2;
                            self.stop_twin = Some((DVec3::new(at2.x, at2.y, doc.ground(at2)), roads::norm_deg(flow2 + self.place_heading)));
                        }
                    }
                    return;
                }
            }
        }
        // (Shift held: put it down freely)
        let model = assets.object(&rel);
        // (a model brought in from a file stands where it is put: it was not made for a road)
        let imported = rel.to_lowercase().contains("\\imported\\");
        let free = self.shift_held || imported || model.as_ref().map(|m| m.tree.is_some() || m.technical).unwrap_or(false);
        // a junction or crossing piece (spline helpers) clicks onto the road ends near it:
        // turned and moved so one of its helpers meets an end, facing along it
        if let (true, false, Some(m)) = (self.align_to_road, free, model.as_ref().filter(|m| !m.helpers.is_empty())) {
            let reach = m.radius() as f64 + 12.0;
            let mut best: Option<(f64, DVec3, f64, DVec3)> = None;
            for e in self.free_road_ends(assets, c.truncate(), reach) {
                for (h, _) in &m.helpers {
                    let turn = roads::norm_deg(e.heading + 180.0 - h[3] as f64);
                    let off = object_matrix(DVec3::ZERO, [turn, 0.0, 0.0]).transform_point3(Vec3::new(h[0], h[1], h[2])).as_dvec3();
                    let at = e.pos - DVec3::new(off.x, off.y, 0.0);
                    let d = (at.truncate() - c.truncate()).length();
                    if d < reach && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, at, turn, e.pos));
                    }
                }
            }
            if let Some((_, at, turn, end)) = best {
                let z = self.doc.as_ref().map(|d| d.ground(at.truncate())).unwrap_or(c.z);
                let pos = DVec3::new(at.x, at.y, z);
                self.align_guide = Some((end, pos));
                self.ghost_pos = Some((pos, roads::norm_deg(turn + self.place_heading)));
                return;
            }
        }
        // on the road itself (the cursor within a road's width): it stands there, at the
        // road's height, and aligned it runs along the road; no setback pushes it off the road
        let on_road = if free && !imported { None } else { self.road_under(assets, c.truncate()) };
        if let (true, Some((h, _))) = (self.align_to_road && !free, on_road) {
            heading = roads::norm_deg(h + self.place_heading);
        }
        if self.align_to_road && !free && on_road.is_none() {
            // its depth decides how far from a road it still belongs to it
            let depth = model.as_ref().map(|m| (m.hi.y - m.lo.y) as f64).unwrap_or(10.0);
            if let Some((q, h, n, hw, _)) = self.nearest_road(assets, c.truncate(), depth + 25.0) {
                // stand facing the road; the user's turns are added
                heading = roads::norm_deg(h + self.place_heading);
                if self.align_front && self.place_heading.rem_euclid(360.0) < 1e-6 {
                    // the front (+y) at the setback from the road's edge
                    let front = model.as_ref().map(|m| m.hi.y as f64).unwrap_or(0.0);
                    let at = q + n * (hw + self.road_setback + front);
                    // (along the road the cursor decides; across it the setback)
                    pos = DVec3::new(at.x, at.y, self.doc.as_ref().map(|d| d.ground(at)).unwrap_or(c.z));
                    let edge = q + n * hw;
                    let z = self.doc.as_ref().map(|d| d.ground(edge)).unwrap_or(c.z);
                    self.align_guide = Some((DVec3::new(edge.x, edge.y, z), pos));
                }
            }
        }
        if let Some((_, z)) = on_road {
            // (the road's own height: a bridge or an embankment is not the ground under it)
            pos.z = z;
        } else if assets.object(&rel).map(|m| m.abs_height).unwrap_or(false) {
            pos.z = c.z;
        }
        self.ghost_pos = Some((pos, self.snap_heading(heading)));
    }

    /// When `p` lies on a road's surface: the way the road runs there and its height.
    fn road_under(&self, assets: &mut Assets, p: DVec2) -> Option<(f64, f64)> {
        let doc = self.doc.as_ref()?;
        let mut best: Option<(f64, f64, f64)> = None;
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                if (DVec2::new(org.x + s.pos[0], org.y + s.pos[1]) - p).length() > s.length + 20.0 {
                    continue;
                }
                let Some(m) = assets.spline(&s.file) else { continue };
                if !m.joinable() {
                    continue;
                }
                let hw = m.half_width as f64;
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let n = (s.length / 0.5).ceil().max(2.0) as usize;
                for i in 0..=n {
                    let at = s.length * i as f64 / n as f64;
                    let q = c.point_at(at);
                    let d = (q.truncate() - p).length();
                    if d < hw && best.map(|b| d < b.2).unwrap_or(true) {
                        best = Some((c.heading_at(at), q.z, d));
                    }
                }
            }
        }
        best.map(|b| (b.0, b.1))
    }

    /// The free ends of visible roads within `within` m of `p` (as anchors: heading out of
    /// the road).
    fn free_road_ends(&self, assets: &mut Assets, p: DVec2, within: f64) -> Vec<Anchor> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        // (every spline end near: a helper one of them is at is already used)
        let mut taken: Vec<DVec3> = Vec::new();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                taken.push(c.point_at(0.0));
                taken.push(c.end_point());
            }
        }
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted && (s.prev_id == 0 || s.next_id == 0)) {
                if (DVec2::new(org.x + s.pos[0], org.y + s.pos[1]) - p).length() > s.length + within {
                    continue;
                }
                if !assets.spline(&s.file).map(|m| m.joinable()).unwrap_or(false) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                if s.prev_id == 0 && (c.point_at(0.0).truncate() - p).length() < within {
                    out.push(Anchor { pos: c.point_at(0.0), heading: roads::norm_deg(s.heading + 180.0), spline: Some((*k, s.id, false)), mid: None });
                }
                if s.next_id == 0 && (c.end_point().truncate() - p).length() < within {
                    out.push(Anchor { pos: c.end_point(), heading: roads::norm_deg(c.heading_at(s.length)), spline: Some((*k, s.id, true)), mid: None });
                }
            }
            // the helper points of junction pieces already on the map (not those a road
            // already ends at): a piece can be set against another piece too
            for o in &t.tile.objects {
                let Some(model) = assets.object(&o.file) else { continue };
                if model.helpers.is_empty() {
                    continue;
                }
                let world = doc.object_world(*k, o, model.abs_height);
                let m = object_matrix(DVec3::ZERO, o.rot);
                for (h, _) in &model.helpers {
                    let q = world + m.transform_point3(Vec3::new(h[0], h[1], h[2])).as_dvec3();
                    if (q.truncate() - p).length() < within && !taken.iter().any(|t| (t.truncate() - q.truncate()).length() < 1.0) {
                        out.push(Anchor { pos: q, heading: roads::norm_deg(h[3] as f64 + o.rot[0]), spline: None, mid: None });
                    }
                }
            }
        }
        out
    }

    /// The visible road nearest to `p` within `within` m: the point on its centre line, the
    /// heading that faces it, the unit way from the road towards `p`, its half width, and how
    /// far along the road that point is.
    fn nearest_road(&self, assets: &mut Assets, p: DVec2, within: f64) -> Option<(DVec2, f64, DVec2, f64, f64)> {
        let doc = self.doc.as_ref()?;
        let mut best: Option<(DVec2, f64, DVec2, f64, f64, f64)> = None;
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
                if (start - p).length() > s.length + within {
                    continue;
                }
                let Some(m) = assets.spline(&s.file) else { continue };
                if !m.joinable() {
                    continue;
                }
                let hw = m.half_width as f64;
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                for i in 0..=n {
                    let at = s.length * i as f64 / n as f64;
                    let q = c.point_at(at).truncate();
                    let d = (q - p).length();
                    if d < within + hw && best.map(|b| d < b.4).unwrap_or(true) {
                        let h = c.heading_at(at);
                        let right = roads::right(h);
                        let side = if (p - q).dot(right) > 0.0 { 1.0 } else { -1.0 };
                        // face the road: the object's front (+y) towards the centre line
                        let face = roads::norm_deg(h + if side > 0.0 { 270.0 } else { 90.0 });
                        best = Some((q, face, right * side, hw, d, at));
                    }
                }
            }
        }
        best.map(|b| (b.0, b.1, b.2, b.3, b.5))
    }


    /// Put the junction piece `piece` onto an end of road `id` (`at_end`: its far end, else
    /// its start): turned and moved so the piece's helper that takes this road type (else its
    /// first) meets the end, facing along it. `Err` says why not.
    pub fn junction_at_end(&mut self, assets: &mut Assets, lib: &Library, k: TileKey, id: i64, at_end: bool, piece: &str) -> Result<(), &'static str> {
        let Some(doc) = self.doc.as_ref() else { return Err("") };
        let Some(s) = doc.tiles.get(&k).and_then(|t| t.tile.splines.iter().find(|s| s.id == id && !s.deleted)) else { return Err(tr("The road is gone", "Дороги уже нет")) };
        if (at_end && s.next_id != 0) || (!at_end && s.prev_id != 0) {
            return Err(tr("This end is already joined to something", "Этот конец уже к чему-то присоединён"));
        }
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
        let (end, heading) = if at_end { (c.end_point(), roads::norm_deg(c.heading_at(s.length))) } else { (c.point_at(0.0), roads::norm_deg(s.heading + 180.0)) };
        let road_file = s.file.replace('/', "\\").to_lowercase();
        let Some(model) = assets.object_wait(piece, 3000) else { return Err(tr("The junction piece could not be read", "Не удалось прочитать перекрёсток")) };
        if model.helpers.is_empty() {
            return Err(tr("This object has no places for roads to join", "У этого объекта нет мест для присоединения дорог"));
        }
        let h = model.helpers.iter().find(|h| h.1.replace('/', "\\").to_lowercase() == road_file).unwrap_or(&model.helpers[0]).0;
        let abs = model.abs_height;
        let turn = roads::norm_deg(heading + 180.0 - h[3] as f64);
        let off = object_matrix(DVec3::ZERO, [turn, 0.0, 0.0]).transform_point3(Vec3::new(h[0], h[1], h[2])).as_dvec3();
        let at = end - off;
        let z = if abs { at.z } else { at.z.max(doc.ground(at.truncate())) };
        self.object_type = Some(piece.to_string());
        self.ghost_pos = Some((DVec3::new(at.x, at.y, z), turn));
        let _ = self.place_object(lib, false);
        Ok(())
    }

    /// Place the object chosen at the cursor now (dropped from the asset browser).
    pub fn place_now(&mut self, assets: &mut Assets, lib: &Library) {
        self.plan_object(assets, lib);
        let _ = self.place_object(lib, false);
    }

    /// OMSI 2's editor's N: put the object chosen in the catalogue at the cursor and go into
    /// move mode for it, as that editor does.
    pub fn new_object(&mut self, lib: &Library) {
        if self.object_type.is_none() {
            self.toast(tr("Pick an object in the catalogue first", "Сначала выберите объект в каталоге"));
            return;
        }
        match self.cursor {
            // (the object goes where the cursor is, turned the way the last one was)
            Some(c) => {
                let heading = self.ghost_pos.map(|g| g.1).unwrap_or(0.0);
                self.ghost_pos = Some((c, heading));
            }
            None => {
                self.toast(tr("Point at the ground where the object is to stand", "Укажите на землю, где должен стоять объект"));
                return;
            }
        }
        if let Some(sel) = self.place_object(lib, false) {
            self.set_tool(Tool::Select);
            self.xform = Xform::Move;
            self.select(Some(sel));
        }
    }

    fn place_object(&mut self, lib: &Library, keep: bool) -> Option<Sel> {
        let _ = keep;
        let (Some((pos, heading)), Some(rel)) = (self.ghost_pos, self.object_type.clone()) else {
            if self.object_type.is_none() {
                self.toast(tr("Pick an object in the catalogue first", "Сначала выберите объект в каталоге"));
            }
            return None;
        };
        let asset = lib.find(&rel).cloned();
        let Some(doc) = self.doc.as_mut() else { return None };
        let k = tile_of(pos.truncate());
        if !doc.tiles.contains_key(&k) {
            self.error(tr("There is no tile here: add one with the Tiles tool", "Здесь нет тайла: добавьте его инструментом «Тайлы»"));
            return None;
        }
        self.history.begin(tr("Place object", "Установка объекта"));
        self.history.touch_tile(doc, k);
        self.history.touch_global(doc);
        let id = doc.new_id();
        let org = tile_origin(k);
        let abs = asset.as_ref().map(|a| a.abs_height).unwrap_or(false);
        let mut o = MapObject {
            file: rel.clone(),
            id,
            // (height above the ground, which an object on a raised road needs)
            pos: [pos.x - org.x, pos.y - org.y, if abs { pos.z } else { let up = pos.z - doc.ground(pos.truncate()); if up.abs() > 0.05 { up } else { 0.0 } }],
            rot: [heading, 0.0, 0.0],
            ..Default::default()
        };
        let is_stop = asset.as_ref().map(|a| a.is_bus_stop).unwrap_or(false);
        if is_stop {
            // (a name no stop far from here has: stops put down one after another are not
            // all "New stop")
            let name = crate::stops::unique_name(&crate::stops::all(doc), None, tr("New stop", "Новая остановка"), pos.truncate());
            o.extra = crate::stops::StopSettings::new_strings(&name, self.stop_kind);
            o.flag = 7;
        }
        let is_entry = asset.as_ref().map(|a| a.is_entry_point).unwrap_or(false);
        // a signal head takes its light from the nearest crossing with a light program
        let mut linked_to: Option<String> = None;
        if asset.as_ref().map(|a| a.light_head).unwrap_or(false) {
            let here = pos.truncate();
            let near = doc.tiles.iter().flat_map(|(tk, t)| t.tile.objects.iter().map(move |x| (tile_origin(*tk) + DVec2::new(x.pos[0], x.pos[1]), x))).filter_map(|(p, x)| lib.find(&x.file).filter(|a| a.light_program).map(|a| ((p - here).length(), x.id, a.name.clone()))).filter(|c| c.0 < 40.0).min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, pid, name)) = near {
                o.var_parent = Some(pid);
                if o.extra.is_empty() {
                    o.extra = vec!["0".into()];
                    o.flag = 1;
                }
                linked_to = Some(name);
            }
        }
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.objects.push(o.clone());
        t.touch_content();
        // its twin across the road, of the same name
        if let (true, Some((p2, h2))) = (is_stop, self.stop_twin) {
            let k2 = tile_of(p2.truncate());
            if doc.tiles.contains_key(&k2) {
                self.history.touch_tile(doc, k2);
                let org2 = tile_origin(k2);
                let twin = MapObject { id: doc.new_id(), pos: [p2.x - org2.x, p2.y - org2.y, if abs { p2.z } else { 0.0 }], rot: [h2, 0.0, 0.0], ..o };
                let t2 = doc.tiles.get_mut(&k2).unwrap();
                t2.tile.objects.push(twin);
                t2.touch_content();
            }
        }
        if is_entry {
            let name = format!("{} {}", tr("Start", "Старт"), doc.global.entry_points.len() + 1);
            doc.global.entry_points.push(EntryPoint { object_id: id, name, ..Default::default() });
            sync_entry_points(doc);
        }
        self.history.commit();
        if let Some(n) = linked_to {
            self.toast(format!("{} «{n}»: {}", tr("Linked to", "Связан с"), tr("choose its light in its properties", "сигнал выберите в его свойствах")));
        }
        // (the tool goes on placing: the caller chooses whether to take what was put)
        Some(Sel::Object(k, id))
    }

    /// Plants of `mix` put at `spots` (OpenStreetMap's trees and woods), each a random one
    /// turned a random way, none on a road nor outside the map; one undo step. How many.
    pub fn plant_osm(&mut self, spots: &[DVec2], mix: &[String]) -> usize {
        self.plant_spots(spots, mix, tr("Trees from OpenStreetMap", "Деревья из OpenStreetMap"))
    }

    /// Plants put at `spots` (drawn from `mix`, turned at random), none on a road nor off
    /// the map; one undo step. How many were put.
    fn plant_spots(&mut self, spots: &[DVec2], mix: &[String], label: &'static str) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        if mix.is_empty() {
            return 0;
        }
        // the roads, every 3 m with how far from them is clear, in 20 m cells
        let cell = |p: DVec2| ((p.x / 20.0).floor() as i64, (p.y / 20.0).floor() as i64);
        let mut roads: std::collections::HashMap<(i64, i64), Vec<(DVec2, f64)>> = Default::default();
        for (k, t) in &doc.tiles {
            for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
                let clear = self.known_half.get(&sp.file.to_lowercase()).copied().unwrap_or(4.0) + 2.0;
                let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
                let n = (sp.length / 3.0).ceil().max(1.0) as usize;
                for i in 0..=n {
                    let q = c.point_at(sp.length * i as f64 / n as f64).truncate();
                    roads.entry(cell(q)).or_default().push((q, clear));
                }
            }
        }
        let on_road = |p: DVec2| {
            let (cx, cy) = cell(p);
            (cx - 1..=cx + 1).any(|x| (cy - 1..=cy + 1).any(|y| roads.get(&(x, y)).map(|v| v.iter().any(|(q, r)| (*q - p).length() < *r)).unwrap_or(false)))
        };
        self.history.begin(label);
        let mut n = 0;
        for p in spots {
            let k = tile_of(*p);
            let Some(doc) = self.doc.as_mut() else { break };
            if !doc.tiles.contains_key(&k) || on_road(*p) {
                continue;
            }
            self.history.touch_tile(doc, k);
            let id = doc.new_id();
            let org = tile_origin(k);
            let file = mix[(self.plant_rng.next() * mix.len() as f64) as usize % mix.len()].clone();
            let o = MapObject { file, id, pos: [p.x - org.x, p.y - org.y, 0.0], rot: [self.plant_rng.next() * 360.0, 0.0, 0.0], ..Default::default() };
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.objects.push(o);
            t.touch_content();
            n += 1;
        }
        if n == 0 {
            self.history.cancel();
        } else {
            self.history.touch_global(self.doc.as_ref().unwrap());
            self.history.commit();
        }
        n
    }

    /// The plant outline clicked so far: closed and filled with the mix (or cleared of
    /// plants with the brush on Clear), one undo step. How many were planted or taken away.
    pub fn finish_plant_outline(&mut self, assets: &mut Assets) -> usize {
        let poly = std::mem::take(&mut self.plant_outline);
        if poly.len() < 3 {
            return 0;
        }
        let mix: Vec<String> = if self.plant_mix.is_empty() { self.object_type.iter().cloned().collect() } else { self.plant_mix.clone() };
        let Some(doc) = self.doc.as_mut() else { return 0 };
        if self.plant_erase {
            self.history.begin(tr("Clear plants", "Убрать растения"));
            let n = crate::plants::clear_inside(doc, &mut self.history, assets, &poly, &mix);
            if n == 0 {
                self.history.cancel();
            } else {
                self.history.commit();
            }
            return n;
        }
        if mix.is_empty() {
            return 0;
        }
        let taken = crate::plants::plants_round(doc, assets, &poly, self.plant_spacing, &mix);
        let spots = crate::plants::fill_spots(&poly, &taken, self.plant_spacing, &mut self.plant_rng);
        self.plant_spots(&spots, &mix, tr("Plant an area", "Посадка на участке"))
    }

    /// How near (m) a click must come to an outline's first corner to close it: some
    /// pixels on the screen.
    fn close_radius(&self) -> f64 {
        (self.view_distance.get() * 0.015).clamp(1.0, 40.0)
    }

    /// How many plants the outline being clicked would take (for the hint).
    pub fn plant_outline_estimate(&self) -> Option<usize> {
        let poly = &self.plant_outline;
        if poly.len() < 3 || self.plant_erase {
            return None;
        }
        let area = (0..poly.len()).map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()])).sum::<f64>().abs() * 0.5;
        Some((area * 0.6 / (self.plant_spacing * self.plant_spacing)).round() as usize)
    }

    /// Buildings from OpenStreetMap: each outline filled with the model of `pool` whose
    /// ground plan comes nearest (a random one of those about as near), its front to the
    /// nearest road within 40 m; an outline no model fits, or one on a road, is left out.
    /// One undo step; how many were put.
    pub fn place_osm_buildings(&mut self, assets: &mut Assets, outlines: &[(Vec<DVec2>, f64, bool)], pool: &[String]) -> usize {
        // the pool's models (their ground plans)
        let t0 = Instant::now();
        let mut sizes: Vec<(String, f64, f64, DVec2, f64)> = Vec::new();
        let mut waiting: Vec<&String> = pool.iter().collect();
        while !waiting.is_empty() && t0.elapsed().as_secs() < 60 {
            assets.poll();
            waiting.retain(|rel| match assets.object(rel) {
                Some(m) => {
                    let (w, d) = ((m.hi.x - m.lo.x) as f64, (m.hi.y - m.lo.y) as f64);
                    // (trees, technical helpers and flat things are no buildings)
                    if m.tree.is_none() && !m.technical && w > 2.0 && d > 2.0 && m.hi.z - m.lo.z > 2.5 {
                        sizes.push(((*rel).clone(), w, d, DVec2::new(((m.lo.x + m.hi.x) * 0.5) as f64, ((m.lo.y + m.hi.y) * 0.5) as f64), (m.hi.z - m.lo.z) as f64));
                    }
                    false
                }
                None => !assets.object_failed(rel),
            });
            if !waiting.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let Some(doc) = self.doc.as_ref() else { return 0 };
        if sizes.is_empty() {
            return 0;
        }
        // the roads, every 2 m with their heading and half width, in 20 m cells
        let cell = |p: DVec2| ((p.x / 20.0).floor() as i64, (p.y / 20.0).floor() as i64);
        let mut roads_at: std::collections::HashMap<(i64, i64), Vec<(DVec2, f64, f64)>> = Default::default();
        for (k, t) in &doc.tiles {
            for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
                let Some(m) = assets.spline(&sp.file) else { continue };
                if !m.joinable() {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
                let n = (sp.length / 2.0).ceil().max(1.0) as usize;
                for i in 0..=n {
                    let at = sp.length * i as f64 / n as f64;
                    let q = c.point_at(at).truncate();
                    roads_at.entry(cell(q)).or_default().push((q, c.heading_at(at), m.half_width as f64));
                }
            }
        }
        let nearest = |p: DVec2, within: f64| -> Option<(DVec2, f64, f64)> {
            let (cx, cy) = cell(p);
            let r = (within / 20.0).ceil() as i64;
            let mut best: Option<((DVec2, f64, f64), f64)> = None;
            for x in cx - r..=cx + r {
                for y in cy - r..=cy + r {
                    for q in roads_at.get(&(x, y)).into_iter().flatten() {
                        let d = (q.0 - p).length();
                        if d < within && best.map(|b| d < b.1).unwrap_or(true) {
                            best = Some((*q, d));
                        }
                    }
                }
            }
            best.map(|b| b.0)
        };
        self.history.begin(tr("Buildings from OpenStreetMap", "Здания из OpenStreetMap"));
        let mut n = 0;
        for (poly, height, known) in outlines {
            if poly.len() < 4 {
                continue;
            }
            let c0 = poly.iter().copied().sum::<DVec2>() / poly.len() as f64;
            // front to the nearest road (none: along its longest side); on a road: none
            let heading = match nearest(c0, 40.0) {
                Some((q, h, hw)) => {
                    if (q - c0).length() < hw + 1.0 {
                        continue;
                    }
                    let side = if (c0 - q).dot(roads::right(h)) > 0.0 { 270.0 } else { 90.0 };
                    roads::norm_deg(h + side)
                }
                None => crate::osm_buildings::own_heading(poly),
            };
            let (c, w, d) = crate::osm_buildings::footprint(poly, heading);
            let fits: Vec<(f64, &(String, f64, f64, DVec2, f64))> = sizes.iter().filter_map(|m| crate::osm_buildings::misfit((m.1, m.2, m.4), (w, d, *height), *known).map(|e| (e, m))).collect();
            let Some(best) = fits.iter().map(|f| f.0).min_by(|a, b| a.total_cmp(b)) else { continue };
            let near: Vec<_> = fits.iter().filter(|f| f.0 <= best + 0.12).collect();
            let pick = near[(self.plant_rng.next() * near.len() as f64) as usize % near.len()].1;
            // (the model's own middle on the outline's)
            let at = c - roads::right(heading) * pick.3.x - roads::dir(heading) * pick.3.y;
            let k = tile_of(at);
            let Some(doc) = self.doc.as_mut() else { break };
            if !doc.tiles.contains_key(&k) {
                continue;
            }
            self.history.touch_tile(doc, k);
            let id = doc.new_id();
            let org = tile_origin(k);
            let o = MapObject { file: pick.0.clone(), id, pos: [at.x - org.x, at.y - org.y, 0.0], rot: [heading, 0.0, 0.0], ..Default::default() };
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.objects.push(o);
            t.touch_content();
            n += 1;
        }
        if n == 0 {
            self.history.cancel();
        } else {
            self.history.touch_global(self.doc.as_ref().unwrap());
            self.history.commit();
        }
        n
    }

    /// Stops put down where OpenStreetMap has them (`(where, name)`), each at the kerb of
    /// the lane nearest to it as a stop placed by hand stands; one with no lane within 25 m,
    /// or a stop the map has within 10 m, is left out. One undo step; how many were put.
    pub fn place_osm_stops(&mut self, assets: &mut Assets, stops: &[(DVec2, String)], rel: &str) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let lht = doc.global.left_hand_traffic;
        // where the map's stops stand (the ones put here count too)
        let mut stood: Vec<DVec2> = doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.extra.len() >= 4 && o.flag == 7).map(move |o| tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1]))).collect();
        self.history.begin(tr("Stops from OpenStreetMap", "Остановки из OpenStreetMap"));
        let mut n = 0;
        for (p, name) in stops {
            let Some(doc) = self.doc.as_ref() else { break };
            let (lanes, _) = crate::lanes::near(doc, assets, *p, 25.0);
            let best = lanes.iter().filter(|l| l.kind == 0).filter_map(|l| l.nearest(*p).map(|q| (q, l.direction))).filter(|(q, _)| q.2 < 25.0).min_by(|a, b| a.0 .2.total_cmp(&b.0 .2));
            let Some(((q, h, _, _), dir)) = best else {
                log::debug!("osm stop {name} at {p:?}: no lane near ({} lanes)", lanes.len());
                continue;
            };
            let flow = if dir == 1 { roads::norm_deg(h + 180.0) } else { h };
            let kerb = roads::right(flow) * if lht { -1.0 } else { 1.0 };
            let at = q.truncate() + kerb * 3.2;
            if stood.iter().any(|s| (*s - at).length() < 10.0) {
                continue;
            }
            let k = tile_of(at);
            let Some(doc) = self.doc.as_mut() else { break };
            if !doc.tiles.contains_key(&k) {
                continue;
            }
            self.history.touch_tile(doc, k);
            let id = doc.new_id();
            let org = tile_origin(k);
            let label = if name.trim().is_empty() { tr("Stop", "Остановка").to_string() } else { name.trim().to_string() };
            let o = MapObject { file: rel.to_string(), id, pos: [at.x - org.x, at.y - org.y, 0.0], rot: [flow, 0.0, 0.0], extra: crate::stops::StopSettings::new_strings(&label, Default::default()), flag: 7, ..Default::default() };
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.objects.push(o);
            t.touch_content();
            stood.push(at);
            n += 1;
        }
        if n == 0 {
            self.history.cancel();
        } else {
            self.history.touch_global(self.doc.as_ref().unwrap());
            self.history.commit();
        }
        n
    }

    /// A car park click with rows on: the first click starts a row, the second fills it to
    /// there with car parks every `parking_step` metres, standing along it.
    fn parking_click(&mut self, assets: &mut Assets, lib: &Library) {
        let Some(c) = self.cursor else { return };
        if self.object_type.is_none() {
            self.toast(tr("Pick a car park in the list first", "Сначала выберите парковку в списке"));
            return;
        }
        let Some(a) = self.parking_start.take() else {
            self.parking_start = Some(c);
            return;
        };
        let d = (c - a).truncate();
        let len = d.length();
        if len < 0.5 {
            self.parking_start = Some(c);
            return;
        }
        // (a car's front along the row; the user's turns are added)
        let heading = roads::norm_deg(roads::heading_of(d) + self.place_heading);
        let n = (len / self.parking_step).floor() as usize + 1;
        let step = if n > 1 { len / (n - 1) as f64 } else { 0.0 };
        let dir = d / len;
        let rel = self.object_type.clone().unwrap();
        let abs = assets.object(&rel).map(|m| m.abs_height).unwrap_or(false);
        self.history.begin(tr("Place car parks", "Установка парковок"));
        let auto_terrain = self.auto_terrain;
        let mut placed = 0;
        for i in 0..n {
            let p = a.truncate() + dir * step * i as f64;
            let Some(doc) = self.doc.as_mut() else { return };
            let k = tile_of(p);
            if !doc.tiles.contains_key(&k) {
                continue;
            }
            self.history.touch_tile(doc, k);
            let org = tile_origin(k);
            let id = doc.new_id();
            let z = if abs && auto_terrain { doc.ground(p) } else { 0.0 };
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.objects.push(MapObject { file: rel.clone(), id, pos: [p.x - org.x, p.y - org.y, z], rot: [heading, 0.0, 0.0], ..Default::default() });
            t.touch_content();
            placed += 1;
        }
        self.history.commit();
        let _ = lib;
        self.toast(format!("{} {placed}", tr("Car parks placed:", "Парковок поставлено:")));
    }

    // ---------------------------------------------------------------- roads

    /// Free spline ends and junction connectors near a point.
    /// (and the spline type that is there, which a road started from it goes on with)
    fn snap_anchor(&self, assets: &mut Assets, p: DVec2, within: f64) -> Option<(Anchor, Option<String>)> {
        let doc = self.doc.as_ref()?;
        // over a junction's plate: the plate itself is the place (a road starts from its
        // middle, as a new arm, or runs into it), not the road ends and middles at its edge
        if !self.drawing_paths() && !self.drawing_track(assets) && self.plate_at(assets, p).is_some() {
            return None;
        }
        let mut best: Option<(f64, Anchor, Option<String>)> = None;
        let mut consider = |d: f64, a: Anchor, file: &str| {
            if d < within && best.as_ref().map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, a, (!file.trim().is_empty()).then(|| file.to_string())));
            }
        };
        // (a track joins tracks, a road roads: rails and asphalt only cross)
        let track = self.drawing_track(assets);
        // the connectors of openMapper's own junctions: a road's end at one is an arm of the
        // junction, not a loose end to go on from (a new road from there is drawn out of the
        // plate itself, as a new arm)
        let mut plate_edges: Vec<DVec2> = Vec::new();
        for (k, t) in &doc.tiles {
            for o in t.tile.objects.iter().filter(|o| is_own_junction(&o.file)) {
                let Some(model) = assets.object(&o.file) else { continue };
                let world = doc.object_world(*k, o, model.abs_height);
                let m = object_matrix(DVec3::ZERO, o.rot);
                plate_edges.extend(model.helpers.iter().map(|(h, _)| (world + m.transform_point3(Vec3::new(h[0], h[1], h[2])).as_dvec3()).truncate()));
            }
        }
        let at_plate = |q: DVec3| !track && plate_edges.iter().any(|e| (*e - q.truncate()).length() < 3.0);
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                if assets.spline(&s.file).map(|m| !m.joinable() || m.is_track() != track).unwrap_or(true) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                if s.prev_id == 0 && !at_plate(c.point_at(0.0)) {
                    let a = c.point_at(0.0);
                    consider((a.truncate() - p).length(), Anchor { pos: a, heading: roads::norm_deg(s.heading + 180.0), spline: Some((*k, s.id, false)), mid: None }, &s.file);
                }
                if s.next_id == 0 && !at_plate(c.end_point()) {
                    let e = c.end_point();
                    consider((e.truncate() - p).length(), Anchor { pos: e, heading: roads::norm_deg(c.heading_at(s.length)), spline: Some((*k, s.id, true)), mid: None }, &s.file);
                }
            }
            for o in &t.tile.objects {
                let Some(model) = assets.object(&o.file) else { continue };
                if model.helpers.is_empty() {
                    continue;
                }
                let world = doc.object_world(*k, o, model.abs_height);
                let m = object_matrix(DVec3::ZERO, o.rot);
                for (h, file) in &model.helpers {
                    // (h[3]: the way out of the junction, in the object's frame)
                    let q = world + m.transform_point3(Vec3::new(h[0], h[1], h[2])).as_dvec3();
                    let d = (q.truncate() - p).length();
                    // (a connector a road already ends at is taken: a new road does not start
                    // there, on top of that road)
                    if d < within && self.spline_end_at(assets, q.truncate()) {
                        continue;
                    }
                    consider(d, Anchor { pos: q, heading: roads::norm_deg(h[3] as f64 + o.rot[0]), spline: None, mid: None }, file);
                }
            }
        }
        if best.is_some() {
            return best.map(|b| (b.1, b.2));
        }
        // no end near: a point in the middle of a road (the road is cut there). A track's
        // middle is joined only by a switch: with Auto-Switch off, tracks meet at ends only.
        if track && !self.auto_switches {
            return None;
        }
        let mut mid: Option<(f64, Anchor, String)> = None;
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
                if (start - p).length() > s.length + 40.0 {
                    continue;
                }
                let Some(m) = assets.spline(&s.file) else { continue };
                // (only roads one sees: not a fence, a footpath, or the invisible AI splines
                // under a map's roads; and of the family drawn)
                if !m.joinable() || m.is_track() != track {
                    continue;
                }
                let hw = m.half_width as f64;
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                for i in 1..n {
                    let at = s.length * i as f64 / n as f64;
                    // (right at a joint of two splines too: the cut goes just inside one)
                    if at < 0.6 || at > s.length - 0.6 {
                        continue;
                    }
                    let q = c.point_at(at);
                    let d = (q.truncate() - p).length();
                    if d < hw + 2.0 && mid.as_ref().map(|b| d < b.0).unwrap_or(true) {
                        mid = Some((d, Anchor { pos: q, heading: roads::norm_deg(c.heading_at(at) + 90.0), spline: None, mid: Some((*k, s.id, at, hw)) }, s.file.clone()));
                    }
                }
            }
        }
        if let Some(m) = mid {
            return Some((m.1, Some(m.2)));
        }
        // nothing a road joins: the ends of the other splines one sees (a kerb, a wall, a
        // footpath) are still places to start from, the point only, nothing joined to it
        let mut end: Option<(f64, Anchor, String)> = None;
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
                if (start - p).length() > s.length + within {
                    continue;
                }
                let Some(m) = assets.spline(&s.file) else { continue };
                if m.joinable() || m.def.only_editor || m.is_track() != track {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                for (q, heading) in [(c.point_at(0.0), roads::norm_deg(s.heading + 180.0)), (c.end_point(), roads::norm_deg(c.heading_at(s.length)))] {
                    let d = (q.truncate() - p).length();
                    if d < within && end.as_ref().map(|b| d < b.0).unwrap_or(true) {
                        end = Some((d, Anchor { pos: q, heading, spline: None, mid: None }, s.file.clone()));
                    }
                }
            }
        }
        end.map(|m| (m.1, Some(m.2)))
    }

    /// A road's end (either) lies within a metre of `p`.
    fn spline_end_at(&self, assets: &mut Assets, p: DVec2) -> bool {
        let Some(doc) = self.doc.as_ref() else { return false };
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
                if (start - p).length() > s.length + 2.0 || assets.spline(&s.file).map(|m| !m.joinable()).unwrap_or(true) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                if (c.point_at(0.0).truncate() - p).length() < 1.0 || (c.end_point().truncate() - p).length() < 1.0 {
                    return true;
                }
            }
        }
        false
    }

    /// The type being drawn is a track (rails meet in switches, not junctions).
    /// The road drawn is joined to what is near: its ends take the ends of roads and the places
    /// of junction pieces, a junction plate takes it as an arm, junctions are made where it meets
    /// roads. Not for a cable, a fence or a footpath (nothing drives on them: they only lie where
    /// they are drawn), not with the option off and not while Ctrl is held.
    fn joins(&self, assets: &mut Assets) -> bool {
        if !self.snap_roads || self.ctrl_held {
            return false;
        }
        self.drawing_paths() || self.road_type.as_deref().and_then(|r| assets.spline(r)).map(|m| m.joinable()).unwrap_or(true)
    }

    fn drawing_track(&self, assets: &mut Assets) -> bool {
        !self.drawing_paths() && self.road_type.as_deref().and_then(|r| assets.spline(r)).map(|m| m.is_track()).unwrap_or(false)
    }

    /// A switch with its toe at `toe`, opening towards `heading`, its branch to the left or
    /// right: the plan and its diverging heel (place, heading out of the switch). None while
    /// the switch's model loads, or where the installation has none.
    fn switch_plan(&self, assets: &mut Assets, toe: DVec3, heading: f64, left: bool) -> Option<(crate::switches::Plan, (DVec3, f64))> {
        if !self.auto_switches {
            return None;
        }
        let track = self.road_type.as_deref()?;
        let rel = self.best_switch(assets, track, left);
        let model = assets.object(&rel)?;
        let shape = crate::switches::Shape::of(&model)?;
        let (mut pos, out) = shape.world(shape.branch, toe, heading);
        pos.z = self.doc.as_ref().map(|d| d.ground(pos.truncate())).unwrap_or(toe.z);
        let plan = crate::switches::Plan { rel, toe, heading, length: shape.length(), origin: shape.origin(toe, heading) };
        Some((plan, (pos, out)))
    }

    /// The switch that looks most like the track (the one whose model shares the most of
    /// its textures; while the models load, the stock one for its kind).
    fn best_switch(&self, assets: &mut Assets, track: &str, left: bool) -> String {
        let name = |p: &std::path::PathBuf| p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        let track_tex: Vec<String> = assets.spline(track).map(|m| m.materials.iter().filter_map(|t| t.texture.as_ref().map(name)).collect()).unwrap_or_default();
        let mut best: Option<(i32, String)> = None;
        for c in self.switch_catalogue.iter().filter(|c| c.left == left) {
            let Some(m) = assets.object(&c.rel) else { continue };
            let tex: Vec<String> = m.parts.iter().flat_map(|p| p.materials.iter()).filter_map(|t| t.texture.as_ref().map(name)).collect();
            let score = crate::switches::fit(track, &track_tex, &c.rel, &tex);
            if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                best = Some((score, c.rel.clone()));
            }
        }
        best.map(|b| b.1).unwrap_or_else(|| crate::switches::object_for(track, left))
    }

    /// The way a spline runs at a point in its middle (degrees).
    fn heading_in(&self, mid: ((i32, i32), i64, f64, f64)) -> Option<f64> {
        let (k, id, at, _) = mid;
        let s = self.doc.as_ref()?.spline(k, id)?;
        Some(omsi_geometry::SplineCurve::from_map(s, tile_origin(k)).heading_at(at))
    }

    /// The kind of lane the AI path type carries (0 cars, 1 people).
    fn path_kind(&self, assets: &mut Assets) -> i32 {
        self.path_type.as_deref().and_then(|r| assets.spline(r)).and_then(|m| m.def.paths.first().map(|p| p.kind)).unwrap_or(0)
    }

    /// Where an AI path may start (`starting`) or end near a point: a lane end of its kind
    /// that traffic leaves by (to start) or comes in by (to end), else a point along a lane,
    /// heading the way its traffic goes.
    fn snap_lane(&self, assets: &mut Assets, p: DVec2, starting: bool) -> Option<(Anchor, Option<String>)> {
        let kind = self.path_kind(assets);
        self.snap_lane_kind(assets, p, starting, kind)
    }

    /// The lane of `kind` near a point to join: its end, else a place along it.
    fn snap_lane_kind(&self, assets: &mut Assets, p: DVec2, starting: bool, kind: i32) -> Option<(Anchor, Option<String>)> {
        let doc = self.doc.as_ref()?;
        let (lanes, ends) = crate::lanes::near(doc, assets, p, 80.0);
        let file = self.path_type.clone();
        let end = ends
            .iter()
            .filter(|e| e.kind == kind && if starting { e.exit } else { e.entry })
            .map(|e| ((e.pos.truncate() - p).length(), e))
            .filter(|(d, _)| *d < 4.0)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, e)) = end {
            return Some((Anchor { pos: e.pos, heading: e.heading, spline: None, mid: None }, file));
        }
        let along = lanes
            .iter()
            .filter(|l| l.kind == kind)
            .filter_map(|l| l.nearest(p).map(|n| (n, l.direction, l.spline)))
            .filter(|((_, _, d, _), _, _)| *d < 2.0)
            .min_by(|a, b| a.0 .2.total_cmp(&b.0 .2));
        along.map(|((q, h, _, at), dir, sp)| {
            let flow = if dir == 1 { h + 180.0 } else { h };
            // (on a spline's lane: the spline is cut there when the path is built, so the lane
            // has an end there for the path to join - openOMSI joins lanes only end to end)
            let mid = sp.zip(at).filter(|((_, _, len), at)| *at > 0.6 && *at < len - 0.6).map(|((k, id, _), at)| (k, id, at, 0.0));
            (Anchor { pos: q, heading: roads::norm_deg(if starting { flow } else { flow + 180.0 }), spline: None, mid }, file)
        })
    }

    /// A path starts at a lane end of its own kind: near the end of a footway lane the path type
    /// becomes the footpath, near a street lane's the street.
    fn follow_lane_kind(&mut self, assets: &mut Assets, p: DVec2) {
        let Some(doc) = self.doc.as_ref() else { return };
        let (_, ends) = crate::lanes::near(doc, assets, p, 12.0);
        let near = ends.iter().filter(|e| e.kind < 2 && (e.pos.truncate() - p).length() < 3.0).min_by(|a, b| (a.pos.truncate() - p).length().total_cmp(&(b.pos.truncate() - p).length()));
        let Some(kind) = near.map(|e| e.kind) else { return };
        if kind != self.path_kind(assets) {
            self.set_path_type((if kind == 1 { "Splines\\invis_sidewalk.sli" } else { "Splines\\Marcel\\invis_street.sli" }).to_string());
        }
    }

    /// The road lane (not an AI path of its own) under the cursor while AI paths are edited: the
    /// road's pieces to make editable, and where it was clicked.
    fn road_lane_under_cursor(&self, assets: &mut Assets) -> Option<(Vec<(TileKey, i64)>, DVec2)> {
        let doc = self.doc.as_ref()?;
        let p = self.cursor?.truncate();
        let (lanes, _) = crate::lanes::near(doc, assets, p, 12.0);
        let hit = lanes.iter().filter(|l| l.kind < 2 && l.spline.is_some()).filter_map(|l| l.nearest(p).map(|n| (n.2, l.spline.unwrap()))).filter(|(d, _)| *d < 1.2).min_by(|a, b| a.0.total_cmp(&b.0))?;
        let (k, id, _) = hit.1;
        let file = doc.spline(k, id)?.file.clone();
        if assets.spline(&file).map(|m| m.is_ai_path()).unwrap_or(true) {
            return None;
        }
        // (the road's pieces along it, up to 40 each way: a lane goes on through them)
        let mut pieces = vec![(k, id)];
        for dir in [false, true] {
            let mut cur = doc.spline(k, id)?.clone();
            for _ in 0..40 {
                let n = if dir { cur.next_id } else { cur.prev_id };
                let Some((nk, next)) = (n != 0).then(|| doc.find_spline(n)).flatten() else { break };
                if !next.file.eq_ignore_ascii_case(&file) || pieces.contains(&(nk, n)) {
                    break;
                }
                pieces.push((nk, n));
                cur = next.clone();
            }
        }
        Some((pieces, p))
    }

    /// The AI path nearest to a point picked, with the pieces round it.
    pub fn pick_ai_path_near(&mut self, assets: &mut Assets, p: DVec2) {
        let Some(doc) = self.doc.as_ref() else { return };
        let mut best: Option<(f64, TileKey, i64)> = None;
        for (k, t) in &doc.tiles {
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                if !assets.spline(&s.file).map(|m| m.is_ai_path()).unwrap_or(false) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(*k));
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                for i in 0..=n {
                    let d = (c.point_at(s.length * i as f64 / n as f64).truncate() - p).length();
                    if d < 2.0 && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, *k, s.id));
                    }
                }
            }
        }
        if let Some((_, k, id)) = best {
            self.select_path_pieces(k, id);
        }
    }

    fn plan_road(&mut self, assets: &mut Assets) {
        self.road_plan = None;
        let Some(c) = self.cursor else { return };
        // (an end pulls the cursor only from close by: aimed at a road a little way from its
        // end, the road is joined there, not bent round to the end)
        if self.drawing_paths() && self.road_start.is_none() {
            self.follow_lane_kind(assets, c.truncate());
        }
        let joins = self.joins(assets);
        let found = if self.drawing_paths() { self.snap_lane(assets, c.truncate(), self.road_start.is_none()) } else if joins { self.snap_anchor(assets, c.truncate(), 5.0) } else { None };
        self.road_snap_file = found.as_ref().and_then(|f| f.1.clone());
        let snap = found.map(|f| f.0);
        // (the start end itself is not a target)
        let snap = snap.filter(|a| self.road_start.as_ref().map(|(s, _)| (s.pos - a.pos).length() > 1.0).unwrap_or(true));
        self.road_snap = snap;
        let Some((mut start, fixed)) = self.road_start else { return };
        let mut target = if snap.is_some() { c.truncate() } else { self.snap_point(c.truncate()) };
        // a straight from a free start: its heading to whole steps
        if !fixed && snap.is_none() && self.snap_angle {
            let d = target - start.pos.truncate();
            target = start.pos.truncate() + roads::dir(snap_deg(roads::heading_of(d), self.angle_step)) * d.length();
        }
        let mut end_anchor = snap;
        // into a junction plate: to its centre (it is cut back to the edge and becomes an arm)
        self.road_plate_end = None;
        if snap.is_none() && !self.drawing_paths() && joins {
            if let Some(p) = self.plate_at(assets, c.truncate()) {
                target = p.1.truncate();
                self.road_plate_end = Some(p);
            }
        }
        // tracks branch off and run into one another as switches: along the track, not
        // across it
        let track = self.drawing_track(assets);
        let mut fixed = fixed;
        self.switch_plans.clear();
        if track {
            if let Some(h) = start.mid.and_then(|m| self.heading_in(m)) {
                // (the way along the track that leans towards where the new one goes, and the
                // side it goes off to)
                let towards = target - start.pos.truncate();
                let travel = if roads::dir(h).dot(towards) >= 0.0 { h } else { roads::norm_deg(h + 180.0) };
                let left = towards.dot(roads::right(travel)) < 0.0;
                match self.switch_plan(assets, start.pos, travel, left) {
                    // a switch: the new track leaves from its diverging heel
                    Some((plan, (pos, heading))) => {
                        start.pos = pos;
                        start.heading = heading;
                        self.switch_plans.push(plan);
                    }
                    None => start.heading = travel,
                }
                fixed = true;
            }
            if let Some(a) = snap.filter(|a| a.mid.is_some()) {
                if let Some(h) = self.heading_in(a.mid.unwrap()) {
                    // a switch whose heels face the new track: it arrives at the diverging one
                    let from = start.pos.truncate() - a.pos.truncate();
                    let open = if roads::dir(h).dot(from) >= 0.0 { h } else { roads::norm_deg(h + 180.0) };
                    let left = from.dot(roads::right(open)) < 0.0;
                    match self.switch_plan(assets, a.pos, open, left) {
                        Some((plan, (pos, heading))) => {
                            target = pos.truncate();
                            end_anchor = Some(Anchor { pos, heading, spline: None, mid: None });
                            self.switch_plans.push(plan);
                        }
                        None => {
                            target = a.pos.truncate();
                            end_anchor = Some(Anchor { pos: a.pos, heading: open, spline: None, mid: None });
                        }
                    }
                }
            }
        }
        // joining a road in its middle: the new road meets its edge, coming straight at it
        if let Some(a) = snap.filter(|a| a.mid.is_some() && !self.drawing_paths() && !track) {
            let (_, _, _, hw) = a.mid.unwrap();
            let from = start.pos.truncate();
            let d = (a.pos.truncate() - from).normalize_or_zero();
            // (a junction is made at the centre line; without one the road stops at the edge)
            target = if self.auto_junctions { a.pos.truncate() } else { a.pos.truncate() - d * hw };
            end_anchor = None;
        }
        // leaving a road from its middle: from its centre line (a junction) or its edge
        if let (Some((_, _, _, hw)), false, false, false) = (start.mid, self.auto_junctions, self.drawing_paths(), track) {
            let d = (target - start.pos.truncate()).normalize_or_zero();
            start.pos += DVec3::new(d.x, d.y, 0.0) * hw;
        }
        let typed = self.typed();
        let z0 = start.pos.z;
        // numbers typed or the straight lock: the road is drawn by them, not to what the
        // cursor is near
        if (typed.any() || self.straight_lock) && !self.drawing_paths() {
            let Some(segs) = crate::draw::plan(start.pos.truncate(), fixed.then_some(start.heading), self.snap_point(c.truncate()), &typed, self.straight_lock) else { return };
            self.road_snap = None;
            let end = segs.last().unwrap().end();
            let z1 = if self.keep_height { z0 + self.typed_rise() } else if !self.auto_terrain { z0 } else { self.doc.as_ref().map(|d| d.ground(end)).unwrap_or(c.z) };
            self.road_plan = Some((segs, z0, z1, None));
            return;
        }
        // (a cursor on the plane far beyond the map, looking at the horizon: no road there)
        let Some(segs) = roads::plan(&start, fixed, target, end_anchor.as_ref()).filter(|s| roads::total_length(s) <= crate::draw::MAX_LENGTH) else { return };
        let z1 = match snap {
            _ if self.road_plate_end.is_some() => self.road_plate_end.unwrap().1.z,
            Some(a) => a.pos.z,
            None if self.keep_height => z0 + self.typed_rise(),
            None if !self.auto_terrain => z0,
            None => c.z,
        };
        self.road_plan = Some((segs, z0, z1, snap));
    }

    fn road_click(&mut self, assets: &mut Assets, lib: &Library) {
        let Some(rel) = self.road_type.clone() else {
            self.toast(tr("Pick a road type in the catalogue first", "Сначала выберите тип дороги в каталоге"));
            return;
        };
        let Some(c) = self.cursor else { return };
        if self.road_start.is_none() {
            // a road started from another one goes on with its type
            if self.inherit_type && !self.drawing_paths() {
                if let (Some(_), Some(f)) = (self.road_snap, self.road_snap_file.clone()) {
                    if lib.find(&f).is_some() && assets.spline(&f).is_some() {
                        self.road_type = Some(f);
                    }
                }
            }
            match self.road_snap {
                // (from the middle of a road the heading is free)
                Some(a) if a.mid.is_some() && !self.drawing_paths() => self.road_start = Some((a, false)),
                Some(a) => self.road_start = Some((a, true)),
                None => {
                    // from inside a junction plate: from its centre, as a new arm of it
                    self.road_plate_start = if self.joins(assets) { self.plate_at(assets, c.truncate()) } else { None };
                    let q = self.snap_point(c.truncate());
                    let z = self.doc.as_ref().map(|d| d.ground(q)).unwrap_or(c.z);
                    let pos = self.road_plate_start.map(|p| p.1).unwrap_or(DVec3::new(q.x, q.y, z));
                    self.road_start = Some((Anchor { pos, ..Default::default() }, false));
                }
            }
            return;
        }
        let Some((segs, z0, z1, end)) = self.road_plan.clone() else {
            self.error(tr("That bend is too sharp", "Слишком крутой поворот"));
            return;
        };
        let (start, _) = self.road_start.unwrap();
        let half = assets.spline(&rel).map(|m| m.half_width).unwrap_or(4.0);
        // a road met in its middle is cut in two there: the joint is where a junction goes
        let mids: Vec<_> = [start.mid, end.and_then(|e| e.mid)].into_iter().flatten().collect();
        let end_link = end.filter(|e| e.mid.is_none());
        let start_link = if start.mid.is_some() { Anchor { spline: None, ..start } } else { start };
        let made = self.build_road(&rel, &segs, z0, z1, start_link, end_link, half as f64);
        let mut last = made.as_ref().and_then(|m| m.last().copied());
        if let Some(made) = &made {
            if self.drawing_track(assets) {
                // a switch: the track met is cut where the new one leaves or joins it, so the
                // rails' lanes have ends there for trains to choose between
                let plans = std::mem::take(&mut self.switch_plans);
                let switched = !plans.is_empty();
                if let Some(doc) = self.doc.as_mut() {
                    if plans.is_empty() {
                        for (k, id, at, _) in mids {
                            doc.split_spline(k, id, at);
                        }
                    }
                    // the switch objects, the main track taken out under them; where the main
                    // track bends, it is brought round to the switch's straight heel again
                    for p in &plans {
                        let Some(laid) = crate::switches::lay(doc, p) else { continue };
                        if let (Some(r), Some(shape)) = (laid.rejoin, assets.object(&p.rel).and_then(|m| crate::switches::Shape::of(&m))) {
                            let heel = shape.world(shape.straight, p.toe, p.heading);
                            rejoin_track(doc, r, heel);
                        }
                    }
                }
                if switched {
                    self.toast(tr("Switch made", "Стрелка создана"));
                }
            } else if self.auto_junctions && !self.drawing_paths() && self.joins(assets) {
                let start_mid = start.mid.map(|m| (m, start.pos));
                let end_mid = end.and_then(|e| e.mid.map(|m| (m, e.pos)));
                let n = self.make_junctions(assets, made, half as f64, start_mid, end_mid);
                if n > 0 {
                    self.toast(format!("{} {n}", tr("Junctions made:", "Перекрёстков создано:")));
                }
                if self.auto_adapt {
                    self.adapt_joints(assets, [start.spline, end.and_then(|e| e.spline)].into_iter().flatten().collect());
                }
                // drawing goes on from the road's end, if a junction has not taken it
                last = last.filter(|l| self.doc.as_ref().map(|d| d.spline(l.0, l.1).is_some()).unwrap_or(false));
            } else if let Some(doc) = self.doc.as_mut() {
                for (k, id, at, _) in mids {
                    doc.split_spline(k, id, at);
                }
            }
            // a road from or into a junction plate: cut back to its edge, an arm of it
            let plates = [(self.road_plate_start.take(), made.first().copied(), false), (self.road_plate_end.take(), made.last().copied(), true)];
            for (plate, end, at_end) in plates {
                let (Some((jid, _, r)), Some((k, sid))) = (plate, end) else { continue };
                let Some(doc) = self.doc.as_mut() else { break };
                if let Some(arm) = doc.trim(k, sid, at_end, r) {
                    self.pending_arms.entry(jid).or_default().push(crate::junction::Arm { key: arm.0, id: arm.1, at_end: arm.2 });
                    self.pending_rebuild.push(jid);
                    if at_end {
                        last = None;
                    }
                }
            }
            self.finish_step();
        }
        let made = if end.is_none() { last } else { made.and_then(|m| m.last().copied()) };
        let _ = lib;
        match (made, end) {
            (Some(last), None) if self.keep_drawing => {
                // go on from the end just made
                let (k, id) = last;
                let doc = self.doc.as_ref().unwrap();
                if let Some(s) = doc.spline(k, id) {
                    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
                    self.road_start = Some((Anchor { pos: c.end_point(), heading: roads::norm_deg(c.heading_at(s.length)), spline: Some((k, id, true)), mid: None }, true));
                }
            }
            _ => {
                self.road_start = None;
            }
        }
        self.road_plan = None;
    }

    /// Where a road was drawn on from the end of another of a different width (`ends`: the
    /// old road's ends, as (tile, id, at its end)), cut both back and lay a plate between
    /// them that tapers from one width to the other.
    fn adapt_joints(&mut self, assets: &mut Assets, ends: Vec<(TileKey, i64, bool)>) {
        self.adapt_joints_by(assets, ends, 5.0);
    }

    /// [`Editor::adapt_joints`] with the taper `ratio` as metres cut back per metre of
    /// difference in half width (5: 1:10 over both sides; a bus pocket's is steeper).
    fn adapt_joints_by(&mut self, assets: &mut Assets, ends: Vec<(TileKey, i64, bool)>, ratio: f64) -> usize {
        let (Some(content), asphalt) = (self.content_dir.clone(), self.asphalt.clone()) else { return 0 };
        let follow = self.follow_ground;
        let Some(doc) = self.doc.as_mut() else { return 0 };
        let mut made = 0;
        for (k, id, at_end) in ends {
            let Some(s) = doc.spline(k, id) else { continue };
            let next = if at_end { s.next_id } else { s.prev_id };
            let Some((nk, n)) = doc.find_spline(next).map(|(nk, n)| (nk, n.clone())) else { continue };
            let n_end = n.next_id == id && n.prev_id != id;
            let (a, b) = (crate::junction::Arm { key: k, id, at_end }, crate::junction::Arm { key: nk, id: n.id, at_end: n_end });
            let Some(step) = width_step(doc, assets, &a, &b) else { continue };
            let back = taper_back_by(step, ratio);
            // (a road too short for the taper stays joined as it is)
            if reach_along(doc, k, id, at_end) < back + 1.0 || reach_along(doc, nk, n.id, n_end) < back + 1.0 {
                continue;
            }
            self.history.touch_tile(doc, k);
            self.history.touch_tile(doc, nk);
            self.history.touch_global(doc);
            let arms: Vec<crate::junction::Arm> = [(k, id, at_end), (nk, n.id, n_end)].into_iter().filter_map(|(k, id, e)| doc.trim(k, id, e, back)).map(|x| crate::junction::Arm { key: x.0, id: x.1, at_end: x.2 }).collect();
            // (counted only where the plate was laid: a bare bend is no taper)
            if arms.len() == 2 && join_two(doc, assets, true, follow, &content, asphalt.as_deref(), &arms[0], &arms[1]) {
                made += 1;
            }
        }
        made
    }

    /// Make the splines of a planned road. Returns the last one.
    fn build_road(&mut self, rel: &str, segs: &[Seg], z0: f64, z1: f64, start: Anchor, end: Option<Anchor>, half: f64) -> Option<Vec<(TileKey, i64)>> {
        let doc = self.doc.as_mut()?;
        let pieces = roads::split(segs, 50.0);
        let total = roads::total_length(&pieces).max(0.1);
        for p in &pieces {
            if !doc.tiles.contains_key(&tile_of(p.start)) {
                self.error(tr("The road leaves the map: add tiles there first", "Дорога выходит за карту: сначала добавьте тайлы"));
                return None;
            }
        }
        self.history.begin(tr("Build road", "Строительство дороги"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        let _ = total;
        self.lay_road(rel, &pieces, z0, z1, start, end, half)
    }

    /// The splines of a road, cut into pieces (`roads::split`), put into the map, linked to
    /// what its ends meet; inside an open step that keeps every tile.
    fn lay_road(&mut self, rel: &str, pieces: &[Seg], z0: f64, z1: f64, start: Anchor, end: Option<Anchor>, half: f64) -> Option<Vec<(TileKey, i64)>> {
        let doc = self.doc.as_mut()?;
        let pieces = pieces.to_vec();
        let profile = if self.keep_height || !self.auto_terrain { even_profile(&pieces, z0, z1) } else { road_profile(doc, &pieces, z0, z1) };
        let mut made: Vec<(TileKey, i64)> = Vec::new();
        let ids: Vec<i64> = pieces.iter().map(|_| doc.new_id()).collect();
        let mut curves = Vec::new();
        let mut chain = 0.0;
        for (i, p) in pieces.iter().enumerate() {
            let k = tile_of(p.start);
            let org = tile_origin(k);
            let sp = piece_spline(rel, p, &profile, i, org);
            let sp = MapSpline {
                id: ids[i],
                prev_id: if i > 0 { ids[i - 1] } else { 0 },
                next_id: if i + 1 < ids.len() { ids[i + 1] } else { 0 },
                // (textures run on across the joints)
                tex_offset: chain,
                map_chain_offset: Some(chain),
                ..sp
            };
            chain += p.length;
            curves.push(omsi_geometry::SplineCurve::from_map(&sp, org));
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.splines.push(sp);
            t.touch_content();
            made.push((k, ids[i]));
        }
        // link the ends to what they meet (whether it is still there with that end free: a
        // one-sided link would point at nothing)
        let link = |doc: &mut Document, at: Option<(TileKey, i64, bool)>, mine: i64| -> bool {
            let mut linked = false;
            if let Some((k, id, at_end)) = at {
                if let Some(t) = doc.tiles.get_mut(&k) {
                    if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted) {
                        if at_end && s.next_id == 0 {
                            s.next_id = mine;
                            linked = true;
                        } else if !at_end && s.prev_id == 0 {
                            s.prev_id = mine;
                            linked = true;
                        }
                    }
                    t.touch_content();
                }
            }
            linked
        };
        if let Some((k, id, _)) = start.spline {
            let first = made[0];
            if link(doc, start.spline, first.1) {
                if let Some(t) = doc.tiles.get_mut(&first.0) {
                    if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == first.1) {
                        s.prev_id = id;
                    }
                }
            }
            let _ = k;
        }
        if let Some(e) = end {
            if let Some((_, id, _)) = e.spline {
                let last = *made.last().unwrap();
                if link(doc, e.spline, last.1) {
                    if let Some(t) = doc.tiles.get_mut(&last.0) {
                        if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == last.1) {
                            s.next_id = id;
                        }
                    }
                }
            }
        }
        if self.follow_ground && self.auto_terrain && !self.keep_height && !(self.tool == Tool::Logic) {
            conform_ground(doc, &curves, half);
        }
        // (the step stays open: the caller may cut the road it joined, in the same step)
        Some(made)
    }

    /// Roads from OpenStreetMap built as if drawn one after the other (in `osm::roads`'
    /// order; a one-way road with `oneway`'s type when there is one): each joins the roads it meets with junctions, or goes on from a road end in
    /// line with it; one undo step. A road lying mostly on a road the map had is left out
    /// (built before). Gives back how many roads and junctions were made.
    pub fn build_osm(&mut self, assets: &mut Assets, list: &[crate::osm::Road], types: &[(crate::osm::Class, String)], oneway: Option<&str>) -> (usize, usize) {
        // (the types' widths and kinds first, and the map's own roads': junctions are made
        // only with roads whose kind is known)
        let t0 = Instant::now();
        let mut files: Vec<String> = types.iter().map(|t| t.1.clone()).chain(oneway.map(str::to_string)).collect();
        if let Some(doc) = self.doc.as_ref() {
            let mut seen = std::collections::HashSet::new();
            files.extend(doc.tiles.values().flat_map(|t| t.tile.splines.iter().filter(|s| !s.deleted).map(|s| s.file.clone())).filter(|f| seen.insert(f.to_lowercase())));
        }
        for t in &files {
            while assets.spline(t).is_none() && !assets.spline_failed(t) && t0.elapsed().as_secs() < 20 {
                assets.poll();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        // (and the switches' models, where tracks branch: their shape places the new track)
        if self.auto_switches && list.iter().any(|r| r.class.is_track()) {
            let rels: Vec<String> = self.switch_catalogue.iter().map(|c| c.rel.clone()).collect();
            for r in &rels {
                while assets.object(r).is_none() && !assets.object_failed(r) && t0.elapsed().as_secs() < 25 {
                    assets.poll();
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        let Some(doc) = self.doc.as_ref() else { return (0, 0) };
        // where the map's roads run, every 3 m in 10 m cells
        let cell = |p: DVec2| ((p.x / 10.0).floor() as i64, (p.y / 10.0).floor() as i64);
        let mut had: std::collections::HashMap<(i64, i64), Vec<DVec2>> = Default::default();
        for (k, t) in &doc.tiles {
            for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
                let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
                let n = (sp.length / 3.0).ceil().max(1.0) as usize;
                for i in 0..=n {
                    let q = c.point_at(sp.length * i as f64 / n as f64).truncate();
                    had.entry(cell(q)).or_default().push(q);
                }
            }
        }
        let near_old = |p: DVec2| {
            let (cx, cy) = cell(p);
            (cx - 1..=cx + 1).any(|x| (cy - 1..=cy + 1).any(|y| had.get(&(x, y)).map(|v| v.iter().any(|q| (*q - p).length() < 4.0)).unwrap_or(false)))
        };
        self.history.begin(tr("Roads from OpenStreetMap", "Дороги из OpenStreetMap"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        let was = self.road_type.clone();
        let (mut built, mut junctions, mut switches, mut joined_plates) = (0, 0, 0, 0);
        let mut ends_made: Vec<Option<(TileKey, i64, bool)>> = Vec::new();
        for r in list {
            let Some(rel) = types.iter().find(|t| t.0 == r.class).map(|t| t.1.clone()) else { continue };
            let track = r.class.is_track();
            let rel = match oneway {
                Some(o) if r.oneway && !track => o.to_string(),
                _ => rel,
            };
            let Some(model) = assets.spline(&rel) else { continue };
            let half = model.half_width as f64;
            let mut segs = crate::osm::segments(&r.points, r.class);
            if segs.is_empty() {
                continue;
            }
            let samples: Vec<DVec2> = segs.iter().flat_map(|s| {
                let c = s.curve();
                let n = (s.length / 5.0).ceil().max(1.0) as usize;
                (0..=n).map(move |i| c.point_at(s.length * i as f64 / n as f64).truncate())
            }).collect();
            // (a tram's track lies on its street: only roads are checked so)
            if !track && samples.iter().filter(|p| near_old(**p)).count() * 2 > samples.len() {
                continue;
            }
            self.road_type = Some(rel.clone());
            let (p0, p1) = (segs[0].start, segs.last().unwrap().end());
            // (a track may meet another's middle only where a switch is made)
            let mid_ok = |a: &Anchor| !track || a.mid.is_none() || self.auto_switches;
            // an end in a junction plate already there: run to its centre, an arm of it
            let plate0 = if track || !self.auto_junctions { None } else { self.plate_at(assets, p0) };
            let plate1 = if track || !self.auto_junctions { None } else { self.plate_at(assets, p1) };
            let a0 = if plate0.is_some() { None } else { self.snap_anchor(assets, p0, 3.0).map(|a| a.0).filter(mid_ok) };
            let a1 = if plate1.is_some() { None } else { self.snap_anchor(assets, p1, 3.0).map(|a| a.0).filter(|a| a.spline.map(|s| Some(s) != a0.and_then(|b| b.spline)).unwrap_or(true)).filter(mid_ok) };
            if let Some(p) = plate0 {
                if let Some(sg) = roads::straight(p.1.truncate(), p0) {
                    segs.insert(0, sg);
                }
            }
            if let Some(p) = plate1 {
                segs.extend(roads::straight(p1, p.1.truncate()));
            }
            // a track branching off another one or joining it: a switch there (Auto-Switch),
            // the new track starting from (or running into) its diverging heel
            let mut plans: Vec<crate::switches::Plan> = Vec::new();
            let mut track_mids: Vec<((i32, i32), i64, f64, f64)> = Vec::new();
            let mut start_heel: Option<Anchor> = None;
            let mut end_heel: Option<Anchor> = None;
            if track {
                if let Some(a) = a0.filter(|a| a.mid.is_some()) {
                    let m = a.mid.unwrap();
                    if let Some(h) = self.heading_in(m) {
                        let towards = segs[0].curve().point_at(segs[0].length.min(10.0)).truncate() - a.pos.truncate();
                        let travel = if roads::dir(h).dot(towards) >= 0.0 { h } else { roads::norm_deg(h + 180.0) };
                        let left = towards.dot(roads::right(travel)) < 0.0;
                        match self.switch_plan(assets, a.pos, travel, left) {
                            Some((plan, (pos, heading))) => {
                                start_heel = Some(Anchor { pos, heading, spline: None, mid: None });
                                plans.push(plan);
                            }
                            None => track_mids.push(m),
                        }
                    }
                }
                if let Some(a) = a1.filter(|a| a.mid.is_some()) {
                    let m = a.mid.unwrap();
                    if let Some(h) = self.heading_in(m) {
                        let last = segs.last().unwrap();
                        let from = last.curve().point_at((last.length - 10.0).max(0.0)).truncate() - a.pos.truncate();
                        let open = if roads::dir(h).dot(from) >= 0.0 { h } else { roads::norm_deg(h + 180.0) };
                        let left = from.dot(roads::right(open)) < 0.0;
                        match self.switch_plan(assets, a.pos, open, left) {
                            Some((plan, (pos, heading))) => {
                                end_heel = Some(Anchor { pos, heading, spline: None, mid: None });
                                plans.push(plan);
                            }
                            None => track_mids.push(m),
                        }
                    }
                }
                // the OSM line between the switch and where it parts from the old track gives
                // way to a curve from the heel
                if start_heel.is_some() || end_heel.is_some() {
                    let mut pts: Vec<DVec2> = r.points.clone();
                    if let (Some(h), Some(a)) = (start_heel, a0) {
                        let reach = (h.pos.truncate() - a.pos.truncate()).length() + 4.0;
                        while pts.len() > 2 && (pts[0] - a.pos.truncate()).length() < reach {
                            pts.remove(0);
                        }
                    }
                    if let (Some(h), Some(a)) = (end_heel, a1) {
                        let reach = (h.pos.truncate() - a.pos.truncate()).length() + 4.0;
                        while pts.len() > 2 && (pts[pts.len() - 1] - a.pos.truncate()).length() < reach {
                            pts.pop();
                        }
                    }
                    let mut mid_segs = crate::osm::segments(&pts, r.class);
                    if mid_segs.is_empty() {
                        mid_segs = roads::straight(pts[0], pts[pts.len() - 1]).into_iter().collect();
                    }
                    let mut all = Vec::new();
                    if let Some(h) = start_heel {
                        all.extend(roads::plan(&h, true, mid_segs[0].start, None).unwrap_or_default());
                    }
                    all.extend(mid_segs);
                    if let Some(h) = end_heel {
                        let last = *all.last().unwrap();
                        let from = Anchor { pos: DVec3::new(last.end().x, last.end().y, 0.0), heading: last.curve().heading_at(last.length), spline: None, mid: None };
                        all.extend(roads::plan(&from, true, h.pos.truncate(), Some(&h)).unwrap_or_default());
                    }
                    if !all.is_empty() {
                        segs = all;
                    }
                }
            }
            // (into a road's middle: from its centre line exactly)
            if let Some(a) = a0.filter(|a| a.mid.is_some() && !(track && start_heel.is_some())) {
                if let Some(s) = roads::straight(a.pos.truncate(), segs[0].start) {
                    segs.insert(0, s);
                }
            }
            if let Some(a) = a1.filter(|a| a.mid.is_some() && !(track && end_heel.is_some())) {
                let e = segs.last().unwrap().end();
                segs.extend(roads::straight(e, a.pos.truncate()));
            }
            let h0 = segs[0].heading;
            let h1 = segs.last().map(|s| s.curve().heading_at(s.length)).unwrap_or(0.0);
            // a road end met in line with it: going on from it (else beside it, unjoined)
            let in_line = |a: f64, b: f64| (roads::norm_deg(a - b + 180.0) - 180.0).abs() < 25.0;
            let start_link = match a0 {
                Some(a) if a.mid.is_none() && in_line(a.heading, h0) => a,
                _ => Anchor { pos: DVec3::new(segs[0].start.x, segs[0].start.y, 0.0), heading: h0, ..Default::default() },
            };
            let end_link = a1.filter(|a| a.mid.is_none() && in_line(a.heading + 180.0, h1));
            let auto_terrain = self.auto_terrain;
            let Some(doc) = self.doc.as_ref() else { break };
            let q0 = segs[0].start;
            let q1 = segs.last().unwrap().end();
            let z0 = start_heel.map(|h| h.pos.z).or(plate0.map(|p| p.1.z)).or(a0.map(|a| a.pos.z)).unwrap_or_else(|| doc.ground(q0));
            let z1 = end_heel.map(|h| h.pos.z).or(plate1.map(|p| p.1.z)).or(a1.map(|a| a.pos.z)).unwrap_or_else(|| if auto_terrain { doc.ground(q1) } else { z0 });
            if segs.iter().any(|s| !doc.tiles.contains_key(&tile_of(s.start)) || !doc.tiles.contains_key(&tile_of(s.end()))) {
                continue;
            }
            let pieces = roads::split(&segs, 50.0);
            let Some(made) = self.lay_road(&rel, &pieces, z0, z1, start_link, end_link, half) else { continue };
            built += 1;
            ends_made.push(made.first().map(|m| (m.0, m.1, false)));
            ends_made.push(made.last().map(|m| (m.0, m.1, true)));
            if track {
                // the switches laid, the old track taken out under them; or the old track cut
                // where the new one meets it
                if let Some(doc) = self.doc.as_mut() {
                    for (k, id, at, _) in track_mids {
                        doc.split_spline(k, id, at);
                    }
                    for p in &plans {
                        let Some(laid) = crate::switches::lay(doc, p) else { continue };
                        if let (Some(r), Some(shape)) = (laid.rejoin, assets.object(&p.rel).and_then(|m| crate::switches::Shape::of(&m))) {
                            let heel = shape.world(shape.straight, p.toe, p.heading);
                            rejoin_track(doc, r, heel);
                        }
                        switches += 1;
                    }
                }
            } else if self.auto_junctions {
                let start_mid = a0.and_then(|a| a.mid.map(|m| (m, a.pos)));
                let end_mid = a1.and_then(|a| a.mid.map(|m| (m, a.pos)));
                junctions += self.make_junctions(assets, &made, half, start_mid, end_mid);
                // into a plate there before: cut back to its edge and joined to it
                for (plate, end, at_end) in [(plate0, made.first().copied(), false), (plate1, made.last().copied(), true)] {
                    let (Some((jid, _, r)), Some((k, sid))) = (plate, end) else { continue };
                    let Some(doc) = self.doc.as_mut() else { break };
                    if let Some(arm) = doc.trim(k, sid, at_end, r) {
                        self.pending_arms.entry(jid).or_default().push(crate::junction::Arm { key: arm.0, id: arm.1, at_end: arm.2 });
                        self.rebuild_junction(assets, jid);
                        joined_plates += 1;
                    }
                }
            }
        }
        // where streets end at one node (OSM splits its ways at crossings): a junction there
        if self.auto_junctions {
            junctions += self.junctions_at_nodes(assets, &ends_made.into_iter().flatten().collect::<Vec<_>>());
        }
        if switches > 0 || joined_plates > 0 {
            log::info!("osm: {switches} switches, {joined_plates} roads joined to junctions there before");
        }
        self.road_type = was;
        self.finish_step();
        log::info!("osm: {built} of {} roads built, {junctions} junctions", list.len());
        (built, junctions)
    }

    /// Junctions where three or more road ends meet (or two at an angle), as OpenStreetMap's
    /// ways end at the crossings it splits them at: each end cut back and the plate made in
    /// the middle. `ends` are the ends of the roads just built; the free ends of others near
    /// them count too. How many junctions were made.
    fn junctions_at_nodes(&mut self, assets: &mut Assets, ends: &[(TileKey, i64, bool)]) -> usize {
        let (Some(content), asphalt) = (self.content_dir.clone(), self.asphalt.clone()) else { return 0 };
        let styles = self.road_styles.clone();
        let follow = self.follow_ground;
        let adapt = self.auto_adapt;
        let mut made = 0;
        // every free end of a road one sees and joins (tracks meet in switches)
        let free = |doc: &Document, assets: &mut Assets| -> Vec<(TileKey, i64, bool, DVec3, f64, f64)> {
            let mut v = Vec::new();
            for (k, t) in &doc.tiles {
                for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
                    let Some(m) = assets.spline(&sp.file) else { continue };
                    if !m.joinable() || m.is_track() {
                        continue;
                    }
                    let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
                    if sp.prev_id == 0 {
                        v.push((*k, sp.id, false, c.point_at(0.0), c.heading_at(0.0), m.half_width as f64));
                    }
                    if sp.next_id == 0 {
                        v.push((*k, sp.id, true, c.end_point(), roads::norm_deg(c.heading_at(sp.length) + 180.0), m.half_width as f64));
                    }
                }
            }
            v
        };
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let all = free(doc, assets);
        let seeds: Vec<DVec2> = ends.iter().filter_map(|(k, id, at_end)| all.iter().find(|e| e.0 == *k && e.1 == *id && e.2 == *at_end).map(|e| e.3.truncate())).collect();
        let mut used = vec![false; all.len()];
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for s in seeds {
            let near: Vec<usize> = (0..all.len()).filter(|&i| !used[i] && (all[i].3.truncate() - s).length() < 3.0).collect();
            if near.len() < 2 {
                continue;
            }
            for &i in &near {
                used[i] = true;
            }
            clusters.push(near);
        }
        for c in clusters {
            // (two ends in line are one road going on: they are joined, not made a junction)
            let headings: Vec<f64> = c.iter().map(|&i| all[i].4).collect();
            let sharpest = headings.iter().enumerate().flat_map(|(i, a)| headings.iter().skip(i + 1).map(move |b| (roads::norm_deg(a - b + 180.0) - 180.0).abs())).fold(180.0f64, f64::min);
            let centre = c.iter().map(|&i| all[i].3).sum::<DVec3>() / c.len() as f64;
            if self.plate_at(assets, centre.truncate()).is_some() {
                continue;
            }
            let halves: Vec<f64> = c.iter().map(|&i| all[i].5).collect();
            let corner = c.iter().filter_map(|&i| self.doc.as_ref().and_then(|d| d.spline(all[i].0, all[i].1)).map(|s| styles.get(&s.file).corner)).fold(1.0f64, f64::max).clamp(0.5, 3.0);
            let r = crate::junction::radius_at(&halves, sharpest.max(20.0)) * corner;
            let Some(doc) = self.doc.as_mut() else { break };
            if c.len() == 2 {
                // two streets at a corner: both cut back and joined by a bend (in line they go
                // on as they are; folded back on each other they stay apart)
                // (roads of different width are joined by a taper, in line or not)
                let step = (halves[0] - halves[1]).abs();
                let tapered = adapt && step >= 0.25 && sharpest >= 30.0;
                if tapered || (30.0..165.0).contains(&sharpest) {
                    let back = if tapered { taper_back(step) } else { halves.iter().cloned().fold(3.0, f64::max) + 2.0 };
                    let arms: Vec<crate::junction::Arm> = c.iter().filter_map(|&i| doc.trim(all[i].0, all[i].1, all[i].2, back)).map(|x| crate::junction::Arm { key: x.0, id: x.1, at_end: x.2 }).collect();
                    if arms.len() == 2 {
                        join_two(doc, assets, adapt, follow, &content, asphalt.as_deref(), &arms[0], &arms[1]);
                    }
                }
                continue;
            }
            let arms: Vec<crate::junction::Arm> = c.iter().filter_map(|&i| doc.trim(all[i].0, all[i].1, all[i].2, r)).map(|x| crate::junction::Arm { key: x.0, id: x.1, at_end: x.2 }).collect();
            if arms.len() < 3 {
                continue;
            }
            let opts = style_options(doc, &styles, &arms, centre);
            match crate::junction::make_with(doc, assets, &content, asphalt.as_deref(), centre, &arms, &opts) {
                Ok(_) => {
                    made += 1;
                    if follow {
                        let e = arm_ends_world(doc, &arms);
                        let reach = crate::junction::reach(doc, assets, centre, &arms).max(r * 1.1);
                        conform_disc(doc, centre, reach, &e);
                    }
                }
                Err(e) => log::warn!("junction at a node: {e}"),
            }
        }
        log::info!("osm: {made} junctions where streets end at one node");
        made
    }

    /// Cut the roads where a newly built road `made` crosses one or meets one's middle, and
    /// fill each meeting point with a junction object.
    fn make_junctions(&mut self, assets: &mut Assets, made: &[(TileKey, i64)], half: f64, start_mid: Option<(((i32, i32), i64, f64, f64), DVec3)>, end_mid: Option<(((i32, i32), i64, f64, f64), DVec3)>) -> usize {
        let (Some(content), asphalt) = (self.content_dir.clone(), self.asphalt.clone()) else { return 0 };
        let follow = self.follow_ground;
        let adapt = self.auto_adapt;
        let customs: Vec<crate::cjunction::Layout> = if self.use_custom { self.custom_layouts().to_vec() } else { Vec::new() };
        let styles = self.road_styles.clone();
        let mine_style = self.road_type.as_deref().map(|r| styles.get(r)).unwrap_or_default();
        let Some(doc) = self.doc.as_mut() else { return 0 };
        let mut count = 0;
        // the new road's splines (their IDCodes change as they are cut)
        let mut mine: Vec<i64> = made.iter().map(|m| m.1).collect();
        let first = made.first().copied();
        let last = made.last().copied();
        // the ends that meet a road's middle (T junctions), first: their roads are cut there
        for (mid, end, pos) in [(end_mid, last, true), (start_mid, first, false)].into_iter().filter_map(|(m, e, at_end)| Some((m?, e?, at_end))) {
            let (((k, id, at, hw), centre), (nk, nid), at_end) = (mid, end, pos);
            // (a road run in at a slant needs more room for its sharp corner)
            let angle = {
                let h1 = doc.spline(k, id).map(|s| omsi_geometry::SplineCurve::from_map(s, tile_origin(k)).heading_at(at)).unwrap_or(0.0);
                let h2 = doc.spline(nk, nid).map(|s| { let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(nk)); c.heading_at(if at_end { s.length } else { 0.0 }) }).unwrap_or(h1 + 90.0);
                h1 - h2
            };
            let other = doc.spline(k, id).map(|s| styles.get(&s.file)).unwrap_or_default();
            let r = crate::junction::radius_at(&[hw, half], angle) * mine_style.corner.max(other.corner).clamp(0.5, 3.0);
            let (p, q) = doc.cut_at(k, id, at);
            // a custom junction drawn for roads meeting like these takes the place of the plate
            if let Some(used) = crate::ed_cjunction::apply_custom(doc, assets, &customs, &[p, q, Some((nk, nid, at_end))], centre, follow) {
                if let Some(c) = used[2] {
                    mine.push(c.1);
                }
                count += 1;
                continue;
            }
            let arms = [p.and_then(|p| doc.trim(p.0, p.1, p.2, r)), q.and_then(|q| doc.trim(q.0, q.1, q.2, r)), doc.trim(nk, nid, at_end, r)];
            log::debug!("T junction at {at:.1} on {id} (r {r:.1}): arms {:?}", arms);
            for x in arms.iter().flatten() {
                if let Some(sp) = doc.spline(x.0, x.1) {
                    let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(x.0));
                    let e = if x.2 { c.end_point() } else { c.point_at(0.0) };
                    log::debug!("  arm spline {} ({}): junction end at ({:.1}, {:.1}) = {:.1} m from the centre", x.1, sp.file, e.x, e.y, (e.truncate() - centre.truncate()).length());
                }
            }
            if let Some(c) = arms[2] {
                mine.push(c.1);
            }
            let arms: Vec<crate::junction::Arm> = arms.iter().flatten().map(|x| crate::junction::Arm { key: x.0, id: x.1, at_end: x.2 }).collect();
            if arms.len() >= 3 {
                let opts = style_options(doc, &styles, &arms, centre);
                match crate::junction::make_with(doc, assets, &content, asphalt.as_deref(), centre, &arms, &opts) {
                    Ok(_) => {
                        count += 1;
                        if follow {
                            let ends = arm_ends_world(doc, &arms);
                            let reach = crate::junction::reach(doc, assets, centre, &arms).max(r * 1.1);
                            conform_disc(doc, centre, reach, &ends);
                        }
                    }
                    Err(e) => log::warn!("junction: {e}"),
                }
            } else if arms.len() == 2 {
                join_two(doc, assets, adapt, follow, &content, asphalt.as_deref(), &arms[0], &arms[1]);
            }
        }
        // where the new road crosses another one (X junctions), one after the other
        let mut skip: Vec<(i64, i64)> = Vec::new();
        for _ in 0..24 {
            let ours: Vec<(TileKey, MapSpline)> = doc.tiles.iter().flat_map(|(k, t)| t.tile.splines.iter().filter(|s| !s.deleted && mine.contains(&s.id)).map(move |s| (*k, s.clone()))).collect();
            let mut found = None;
            'search: for (k1, s1) in &ours {
                let c1 = omsi_geometry::SplineCurve::from_map(s1, tile_origin(*k1));
                let p1 = crate::junction::polyline(&c1);
                for (k2, t) in &doc.tiles {
                    for s2 in t.tile.splines.iter().filter(|s| !s.deleted && !ours.iter().any(|o| o.1.id == s.id)) {
                        if skip.contains(&(s1.id, s2.id)) {
                            continue;
                        }
                        let org = tile_origin(*k2);
                        let start = DVec2::new(org.x + s2.pos[0], org.y + s2.pos[1]);
                        if (start - c1.start.truncate()).length() > s1.length + s2.length + 10.0 {
                            continue;
                        }
                        let Some(m) = assets.spline(&s2.file) else { continue };
                        // (a road over a track is a level crossing: both go on as they are; so is a
                        // road whose style makes no junctions)
                        if !m.joinable() || m.is_track() || !mine_style.junctions || !styles.get(&s2.file).junctions {
                            continue;
                        }
                        let c2 = omsi_geometry::SplineCurve::from_map(s2, org);
                        let linked = s1.prev_id == s2.id || s1.next_id == s2.id || s2.prev_id == s1.id || s2.next_id == s1.id;
                        let joints: Vec<DVec2> = [c1.point_at(0.0), c1.end_point()].into_iter().map(|p| p.truncate()).filter(|p| [c2.point_at(0.0), c2.end_point()].iter().any(|q| (q.truncate() - *p).length() < 1.0)).collect();
                        log::trace!("candidate {} ({:.0},{:.0}) x {} ({:.0},{:.0})", s1.id, c1.start.x, c1.start.y, s2.id, c2.start.x, c2.start.y);
                        for (pt, a1, a2) in crate::junction::crossings(&p1, &crate::junction::polyline(&c2)) {
                            let (z1, z2) = (c1.point_at(a1).z, c2.point_at(a2).z);
                            log::debug!("crossing {} x {} at ({:.1}, {:.1}): z {z1:.2} / {z2:.2}, {a2:.1} of {:.1}", s1.id, s2.id, pt.x, pt.y, s2.length);
                            // (well above or below: a bridge, not a junction; and not at a
                            // spline's very end, where roads are only linked)
                            if (z1 - z2).abs() > 2.5 {
                                skip.push((s1.id, s2.id));
                                continue;
                            }
                            if linked && joints.iter().any(|j| (*j - pt).length() < 1.5) {
                                continue;
                            }
                            found = Some((*k1, s1.id, a1, *k2, s2.id, a2, DVec3::new(pt.x, pt.y, z2), m.half_width as f64));
                            break 'search;
                        }
                    }
                }
            }
            log::debug!("junctions: {} splines of the new road, found {:?}", ours.len(), found.map(|f| (f.1, f.4)));
            let Some((k1, id1, a1, k2, id2, a2, centre, hw2)) = found else { break };
            // (a sharp crossing needs more room for its corners)
            let angle = {
                let h1 = doc.spline(k1, id1).map(|s| omsi_geometry::SplineCurve::from_map(s, tile_origin(k1)).heading_at(a1)).unwrap_or(0.0);
                let h2 = doc.spline(k2, id2).map(|s| omsi_geometry::SplineCurve::from_map(s, tile_origin(k2)).heading_at(a2)).unwrap_or(90.0);
                h1 - h2
            };
            let other = doc.spline(k2, id2).map(|s| styles.get(&s.file)).unwrap_or_default();
            let r = crate::junction::radius_at(&[half, hw2], angle) * mine_style.corner.max(other.corner).clamp(0.5, 3.0);
            // both roads cut there (at a joint, the splines meeting there are the sides)
            let (p1, q1) = doc.cut_at(k1, id1, a1);
            for x in [p1, q1].iter().flatten() {
                mine.push(x.1);
            }
            let (p2, q2) = doc.cut_at(k2, id2, a2);
            if let Some(used) = crate::ed_cjunction::apply_custom(doc, assets, &customs, &[p1, q1, p2, q2], centre, follow) {
                for a in used.iter().take(2).flatten() {
                    mine.push(a.1);
                }
                skip.push((id1, id2));
                count += 1;
                continue;
            }
            let arms = [p1.and_then(|x| doc.trim(x.0, x.1, x.2, r)), q1.and_then(|x| doc.trim(x.0, x.1, x.2, r)), p2.and_then(|x| doc.trim(x.0, x.1, x.2, r)), q2.and_then(|x| doc.trim(x.0, x.1, x.2, r))];
            for a in arms.iter().take(2).flatten() {
                mine.push(a.1);
            }
            skip.push((id1, id2));
            let arms: Vec<crate::junction::Arm> = arms.iter().flatten().map(|x| crate::junction::Arm { key: x.0, id: x.1, at_end: x.2 }).collect();
            if arms.len() >= 3 {
                let opts = style_options(doc, &styles, &arms, centre);
                match crate::junction::make_with(doc, assets, &content, asphalt.as_deref(), centre, &arms, &opts) {
                    Ok(_) => {
                        count += 1;
                        if follow {
                            let ends = arm_ends_world(doc, &arms);
                            let reach = crate::junction::reach(doc, assets, centre, &arms).max(r * 1.1);
                            conform_disc(doc, centre, reach, &ends);
                        }
                    }
                    Err(e) => log::warn!("junction: {e}"),
                }
            } else if arms.len() == 2 {
                // (the rest were stubs too short to keep: a bend, not a junction)
                join_two(doc, assets, adapt, follow, &content, asphalt.as_deref(), &arms[0], &arms[1]);
            }
        }
        // a junction made before whose arm road this one trimmed away (a short road between
        // the two): made again for the roads it still holds, or a joint of two
        for id in self.orphaned_junctions(assets) {
            log::debug!("junction {id} lost an arm: made again");
            self.rebuild_junction(assets, id);
        }
        count
    }

    /// openMapper's junctions with an arm no road ends at any more.
    fn orphaned_junctions(&self, assets: &mut Assets) -> Vec<i64> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for (k, t) in &doc.tiles {
            for o in t.tile.objects.iter().filter(|o| is_own_junction(&o.file)) {
                let Some((abs, helpers)) = self.content_dir.as_deref().and_then(|c| plate_helpers(c, assets, &o.file)) else { continue };
                let pts = selection::helper_points(doc, *k, o.id, abs, &helpers);
                // (a helper for each way along an arm: one place per arm)
                let mut places: Vec<DVec2> = Vec::new();
                for (p, _) in &pts {
                    if !places.iter().any(|q| (*q - p.truncate()).length() < 1.5) {
                        places.push(p.truncate());
                    }
                }
                let held = places.iter().filter(|q| pts.iter().filter(|(p, _)| (p.truncate() - **q).length() < 1.5).any(|(p, _)| selection::free_end_at(doc, p.truncate()).is_some())).count();
                if held < places.len() {
                    out.push(o.id);
                }
            }
        }
        out
    }

    // ---------------------------------------------------------------- terrain and paint

    fn brush_terrain(&mut self, assets: &mut Assets, c: DVec2, dt: f64) {
        let Some(doc) = self.doc.as_mut() else { return };
        let r = self.brush_radius as f64;
        // the roads near the brush: the ground is not raised up through them
        let roads = road_points_near(doc, assets, c, r + 15.0);
        let strength = self.brush_strength as f64;
        let mode = self.terrain_mode;
        let flat = self.flatten_to.unwrap_or(0.0) as f64;
        let cell = tile_size() / (TERRAIN_SAMPLES - 1) as f64;
        let n = (TERRAIN_SAMPLES - 1) as i64;
        let (gx0, gx1) = (((c.x - r) / cell).floor() as i64, ((c.x + r) / cell).ceil() as i64);
        let (gy0, gy1) = (((c.y - r) / cell).floor() as i64, ((c.y + r) / cell).ceil() as i64);
        // read first (smoothing needs the neighbours as they were)
        let mut changes: Vec<(i64, i64, f32)> = Vec::new();
        for gy in gy0..=gy1 {
            for gx in gx0..=gx1 {
                let p = DVec2::new(gx as f64 * cell, gy as f64 * cell);
                let d = (p - c).length();
                if d > r {
                    continue;
                }
                let w = {
                    let x = 1.0 - (d / r) * (d / r);
                    x * x
                };
                let Some(h) = world_sample(doc, gx, gy, n) else { continue };
                let h = h as f64;
                let nh = match mode {
                    TerrainMode::Raise => h + strength * 12.0 * dt * w,
                    TerrainMode::Lower => h - strength * 12.0 * dt * w,
                    TerrainMode::Smooth => {
                        let mut s = 0.0;
                        let mut k = 0.0;
                        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                            if let Some(v) = world_sample(doc, gx + dx, gy + dy, n) {
                                s += v as f64;
                                k += 1.0;
                            }
                        }
                        if k > 0.0 { h + (s / k - h) * (strength * 6.0 * dt * w).min(1.0) } else { h }
                    }
                    TerrainMode::Flatten => h + (flat - h) * (strength * 8.0 * dt * w).min(1.0),
                    // (towards a fixed pattern of rolls, the same wherever the brush goes, so
                    // holding it longer shapes them more and does not make noise of noise)
                    TerrainMode::Roughen => h + rolls(p) * strength * 3.0 * dt * w,
                    // (clicked from end to end, not brushed)
                    TerrainMode::Ramp => h,
                    // (dug towards the bed, never filled; the banks ease out with the brush's rim)
                    TerrainMode::Water => {
                        let bed = self.water_level.unwrap_or(0.0) as f64 - self.water_depth as f64;
                        if h > bed { h + (bed - h) * (strength * 8.0 * dt * w).min(1.0) } else { h }
                    }
                };
                changes.push((gx, gy, nh as f32));
            }
        }
        for (gx, gy, h) in changes {
            let p = DVec2::new(gx as f64 * cell, gy as f64 * cell);
            let h = match ground_cap(&roads, p, cell) {
                // (never above the road; ground that already was stays as it is)
                Some(cap) => (h as f64).min(cap.max(world_sample(doc, gx, gy, n).unwrap_or(h) as f64)) as f32,
                None => h,
            };
            set_world_sample(doc, &mut self.history, gx, gy, n, h);
        }
        if mode == TerrainMode::Water {
            if let Some(level) = self.water_level {
                // (the tiles under the brush that have no water get it, at the lake's level)
                let mut keys = std::collections::BTreeSet::new();
                for p in [c, c + DVec2::new(r, 0.0), c - DVec2::new(r, 0.0), c + DVec2::new(0.0, r), c - DVec2::new(0.0, r)] {
                    keys.insert(tile_of(p));
                }
                for k in keys {
                    if doc.tiles.get(&k).map(|t| t.water.is_none()).unwrap_or(false) {
                        self.history.touch_tile(doc, k);
                        if let Some(t) = doc.tiles.get_mut(&k) {
                            t.water = Some([level; 4]);
                            t.touch_content();
                        }
                    }
                }
            }
        }
    }

    /// The lake or river clicked out (`water_pts`) dug into the ground: a bed `water_depth` under the water's
    /// level with banks that ease out over `water_bank` metres, and water laid on the tiles it
    /// dug into. The level is a little under the lowest point clicked (or the water the tile has).
    /// One undo step.
    pub fn make_water(&mut self) -> bool {
        let pts = std::mem::take(&mut self.water_pts);
        let lake = self.water_shape == WaterShape::Lake;
        if pts.len() < if lake { 3 } else { 2 } {
            return false;
        }
        let (depth, bank, half) = (self.water_depth as f64, (self.water_bank as f64).max(1.0), self.river_width as f64 / 2.0);
        let Some(doc) = self.doc.as_mut() else { return false };
        let poly: Vec<DVec2> = pts.iter().map(|p| p.truncate()).collect();
        let existing = doc.tiles.get(&tile_of(poly[0])).and_then(|t| t.water).map(|w| w.iter().sum::<f32>() / 4.0);
        let level = existing.map(|l| l as f64).unwrap_or_else(|| pts.iter().map(|p| p.z).fold(f64::MAX, f64::min) - 0.3);
        let edges = if lake { poly.len() } else { poly.len() - 1 };
        let dist = |p: DVec2| -> f64 {
            (0..edges)
                .map(|i| {
                    let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                    let ab = b - a;
                    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                    (p - (a + ab * t)).length()
                })
                .fold(f64::MAX, f64::min)
        };
        // (negative inside the water)
        let signed = |p: DVec2| -> f64 {
            if !lake {
                return dist(p) - half;
            }
            let mut inside = false;
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                    inside = !inside;
                }
            }
            if inside { -dist(p) } else { dist(p) }
        };
        let ease = |x: f64| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let edge = if lake { bank } else { bank.min(half.max(1.0)) };
        let cell = tile_size() / (TERRAIN_SAMPLES - 1) as f64;
        let n = (TERRAIN_SAMPLES - 1) as i64;
        let reach = if lake { 0.0 } else { half } + bank + cell;
        let lo = poly.iter().fold(DVec2::splat(f64::MAX), |a, p| a.min(*p)) - DVec2::splat(reach);
        let hi = poly.iter().fold(DVec2::splat(f64::MIN), |a, p| a.max(*p)) + DVec2::splat(reach);
        self.history.begin(if lake { tr("Lake", "Озеро") } else { tr("River", "Река") });
        let mut changes: Vec<(i64, i64, f32)> = Vec::new();
        let mut wet = std::collections::BTreeSet::new();
        for gy in (lo.y / cell).floor() as i64..=(hi.y / cell).ceil() as i64 {
            for gx in (lo.x / cell).floor() as i64..=(hi.x / cell).ceil() as i64 {
                let p = DVec2::new(gx as f64 * cell, gy as f64 * cell);
                let sd = signed(p);
                if sd > bank {
                    continue;
                }
                let Some(h) = world_sample(doc, gx, gy, n) else { continue };
                let h = h as f64;
                // (ground is only taken down: a bed under the water, a bank sloping to the shore)
                let target = if sd <= 0.0 {
                    wet.insert(tile_of(p));
                    level - depth * ease(-sd / edge)
                } else if h > level {
                    level + (h - level) * ease(sd / bank)
                } else {
                    h
                };
                if target < h {
                    changes.push((gx, gy, target as f32));
                }
            }
        }
        for (gx, gy, h) in changes {
            set_world_sample(doc, &mut self.history, gx, gy, n, h);
        }
        for k in wet {
            if doc.tiles.get(&k).map(|t| t.water.is_none()).unwrap_or(false) {
                self.history.touch_tile(doc, k);
                if let Some(t) = doc.tiles.get_mut(&k) {
                    t.water = Some([level as f32; 4]);
                    t.touch_content();
                }
            }
        }
        self.history.commit();
        self.toast(format!("{} {:.1} m", tr("Water level:", "Уровень воды:"), level));
        true
    }

    /// The ground from `a` to `b` made one even slope from the height at `a` to that at
    /// `b`, as wide as the brush (fully within 60 % of its radius, easing out to its rim);
    /// one undo step. Its gradient (%).
    pub fn ramp(&mut self, a: DVec3, b: DVec3) -> f64 {
        let Some(doc) = self.doc.as_mut() else { return 0.0 };
        let r = self.brush_radius as f64;
        let core = r * 0.6;
        let (a2, b2) = (a.truncate(), b.truncate());
        let len = (b2 - a2).length();
        let cell = tile_size() / (TERRAIN_SAMPLES - 1) as f64;
        let n = (TERRAIN_SAMPLES - 1) as i64;
        let (lo, hi) = (a2.min(b2) - DVec2::splat(r), a2.max(b2) + DVec2::splat(r));
        self.history.begin(tr("Ramp", "Пандус"));
        let mut changes: Vec<(i64, i64, f32)> = Vec::new();
        for gy in (lo.y / cell).floor() as i64..=(hi.y / cell).ceil() as i64 {
            for gx in (lo.x / cell).floor() as i64..=(hi.x / cell).ceil() as i64 {
                let p = DVec2::new(gx as f64 * cell, gy as f64 * cell);
                let t = if len > 1e-6 { ((p - a2).dot(b2 - a2) / (len * len)).clamp(0.0, 1.0) } else { 0.0 };
                let d = (p - (a2 + (b2 - a2) * t)).length();
                if d > r {
                    continue;
                }
                let Some(h) = world_sample(doc, gx, gy, n) else { continue };
                let target = a.z + (b.z - a.z) * t;
                let w = if d <= core {
                    1.0
                } else {
                    let x = 1.0 - (d - core) / (r - core);
                    x * x * (3.0 - 2.0 * x)
                };
                changes.push((gx, gy, (h as f64 + (target - h as f64) * w) as f32));
            }
        }
        for (gx, gy, h) in changes {
            set_world_sample(doc, &mut self.history, gx, gy, n, h);
        }
        self.history.commit();
        if len > 1e-6 { (b.z - a.z) / len * 100.0 } else { 0.0 }
    }

    /// The plant brush held over `c` for `dt` seconds.
    fn brush_plants(&mut self, assets: &mut Assets, c: DVec2, dt: f64) {
        let mix: Vec<String> = if self.plant_mix.is_empty() { self.object_type.iter().cloned().collect() } else { self.plant_mix.clone() };
        let r = self.brush_radius as f64;
        let Some(doc) = self.doc.as_mut() else { return };
        if self.plant_erase {
            crate::plants::clear(doc, &mut self.history, assets, c, r, &mix);
            return;
        }
        if mix.is_empty() {
            self.toast(tr("Pick a tree in the catalogue (or add some to the mix) first", "Сначала выберите дерево в каталоге (или добавьте в смесь)"));
            return;
        }
        // (about as many tries a second as the circle holds plants: it fills in a moment)
        let room = std::f64::consts::PI * r * r / (self.plant_spacing * self.plant_spacing);
        let tries = (room * dt * 3.0).ceil().clamp(1.0, 200.0) as usize;
        crate::plants::sow(doc, &mut self.history, assets, c, r, self.plant_spacing, &mix, &mut self.plant_rng, tries);
    }

    fn brush_paint(&mut self, c: DVec2, dt: f64) {
        let Some(doc) = self.doc.as_mut() else { return };
        doc.sync_mask_slots();
        let layers = doc.global.ground_textures.len();
        if layers < 2 && self.paint_layer > 0 {
            return;
        }
        let r = self.brush_radius as f64;
        let k = (self.brush_strength as f64 * 6.0 * dt).min(1.0);
        let layer = self.paint_layer.min(layers.saturating_sub(1));
        let erase = self.paint_erase;
        let ts = tile_size();
        let t0 = tile_of(c - DVec2::splat(r));
        let t1 = tile_of(c + DVec2::splat(r));
        for ty in t0.1..=t1.1 {
            for tx in t0.0..=t1.0 {
                let key = (tx, ty);
                if !doc.tiles.contains_key(&key) {
                    continue;
                }
                self.history.touch_tile(doc, key);
                let org = tile_origin(key);
                let sizes: Vec<usize> = doc.global.ground_textures.iter().map(crate::doc::mask_size).collect();
                let t = doc.tiles.get_mut(&key).unwrap();
                let mut touched = false;
                for li in 1..layers {
                    // the layer painted goes up; the ones above it go down so it shows
                    let up = li == layer && !erase;
                    let down = (li == layer && erase) || (li > layer && !erase);
                    if !up && !down {
                        continue;
                    }
                    let size = sizes[li];
                    let texel = ts / size as f64;
                    let (x0, x1) = ((((c.x - r - org.x) / texel).floor()).max(0.0) as usize, (((c.x + r - org.x) / texel).ceil()).min(size as f64) as usize);
                    let (y0w, y1w) = ((c.y - r - org.y) / texel, (c.y + r - org.y) / texel);
                    if x1 <= x0 {
                        continue;
                    }
                    let mask = t.masks[li].get_or_insert_with(|| vec![0u8; size * size]);
                    if mask.len() != size * size {
                        *mask = vec![0u8; size * size];
                    }
                    for yi in (y0w.floor().max(0.0) as usize)..(y1w.ceil().min(size as f64) as usize) {
                        for xi in x0..x1 {
                            let p = DVec2::new(org.x + (xi as f64 + 0.5) * texel, org.y + (yi as f64 + 0.5) * texel);
                            let d = (p - c).length();
                            if d > r {
                                continue;
                            }
                            let w = (1.0 - d / r).clamp(0.0, 1.0).powf(0.7) * k;
                            // (row 0 is the north edge)
                            let row = size - 1 - yi;
                            let a = &mut mask[row * size + xi];
                            let v = *a as f64;
                            let nv = if up { v + (255.0 - v) * w } else { v - v * w };
                            *a = nv.round().clamp(0.0, 255.0) as u8;
                            touched = true;
                        }
                    }
                }
                // painting the base ground: everything above it goes down (done above when
                // layer == 0: every layer counts as "above")
                if touched {
                    t.touch_masks();
                }
            }
        }
    }

    // ---------------------------------------------------------------- tiles

    fn tile_click(&mut self) {
        let Some(k) = self.hover_tile else { return };
        let Some(doc) = self.doc.as_mut() else { return };
        if doc.tiles.contains_key(&k) {
            self.picked_tile = Some(k);
            self.toast(tr("Right-click a tile to remove it", "Правый клик по тайлу: удалить его"));
            return;
        }
        let near = doc.tiles.is_empty() || [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| doc.tiles.contains_key(&(k.0 + dx, k.1 + dy)));
        if !near {
            self.error(tr("New tiles go next to the map", "Новые тайлы добавляются рядом с картой"));
            return;
        }
        self.history.begin(tr("Add tile", "Добавление тайла"));
        self.history.touch_tile(doc, k);
        self.history.touch_global(doc);
        doc.add_tile(k);
        self.history.commit();
        self.picked_tile = Some(k);
    }

    /// A tile's water: on at a height (all four corners), or off. Kept open as one step
    /// while the number is dragged.
    pub fn set_water(&mut self, k: TileKey, level: Option<f32>) {
        self.begin_panel_step(tr("Water", "Вода"));
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.touch_tile(doc, k);
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.water = level.map(|h| [h; 4]);
            t.touch_content();
        }
    }

    /// A ground layer's paint detail (its masks `2^p` texels a side), every tile's mask of it
    /// made again at that size; one undo step.
    pub fn set_paint_detail(&mut self, layer: usize, p: f32) {
        let Some(doc) = self.doc.as_mut() else { return };
        if layer == 0 || layer >= doc.global.ground_textures.len() {
            return;
        }
        let from = crate::doc::mask_size(&doc.global.ground_textures[layer]);
        self.history.begin(tr("Paint detail", "Детальность покраски"));
        self.history.touch_global(doc);
        doc.global.ground_textures[layer].params[0] = p;
        let to = crate::doc::mask_size(&doc.global.ground_textures[layer]);
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            if doc.tiles[&k].masks.get(layer).map(|m| m.is_some()).unwrap_or(false) {
                self.history.touch_tile(doc, k);
                let t = doc.tiles.get_mut(&k).unwrap();
                if let Some(m) = t.masks[layer].as_mut() {
                    *m = crate::doc::resample_mask(m, from, to);
                }
                t.touch_masks();
            }
        }
        doc.global_dirty = true;
        doc.global_rev = crate::doc::next_rev();
        self.history.commit();
    }

    /// A painted ground layer moved one place on (`over`: after the next, drawn over it) or
    /// back, its painting on every tile going with it; the base ground stays first. One undo
    /// step.
    pub fn move_ground_layer(&mut self, layer: usize, over: bool) {
        let Some(doc) = self.doc.as_mut() else { return };
        let other = if over { layer + 1 } else { layer.wrapping_sub(1) };
        let n = doc.global.ground_textures.len();
        if layer == 0 || other == 0 || layer >= n || other >= n {
            return;
        }
        self.history.begin(tr("Ground layer order", "Порядок слоёв земли"));
        self.history.touch_global(doc);
        doc.global.ground_textures.swap(layer, other);
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            // (every tile is written again: its mask files are named by the layers' places)
            self.history.touch_tile(doc, k);
            let t = doc.tiles.get_mut(&k).unwrap();
            t.masks.resize(n, None);
            t.masks.swap(layer, other);
            t.touch_masks();
        }
        doc.global_dirty = true;
        doc.global_rev = crate::doc::next_rev();
        self.history.commit();
        self.paint_layer = other;
    }

    /// A ground layer's texture, or its detail texture (the fine grain over it, seen close
    /// up), changed; its painting stays. One undo step.
    pub fn set_ground_texture(&mut self, layer: usize, detail: bool, rel: &str) {
        let Some(doc) = self.doc.as_mut() else { return };
        if layer >= doc.global.ground_textures.len() {
            return;
        }
        self.history.begin(if detail { tr("Detail texture", "Детальная текстура") } else { tr("Ground texture", "Текстура земли") });
        self.history.touch_global(doc);
        let g = &mut doc.global.ground_textures[layer];
        if detail {
            g.detail_texture = rel.to_string();
        } else {
            g.texture = rel.to_string();
        }
        doc.global_dirty = true;
        doc.global_rev = crate::doc::next_rev();
        self.history.commit();
    }

    /// Whether the game lights a tile's ground from the lamps on and round it at night
    /// (`[variable_terrainlightmap]`: it bakes the tile's light map when it loads it).
    pub fn set_tile_lamplight(&mut self, k: TileKey, on: bool) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Lamp light on the ground", "Свет фонарей на земле"));
        self.history.touch_tile(doc, k);
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.tile.variable_terrain_lightmap = on;
            t.touch_content();
        }
        self.history.commit();
    }

    /// A ring of new tiles round the whole map (corners too), in one step. How many.
    pub fn grow_map(&mut self) -> usize {
        let Some(doc) = self.doc.as_mut() else { return 0 };
        let mut new: Vec<TileKey> = Vec::new();
        for k in doc.tiles.keys() {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let n = (k.0 + dx, k.1 + dy);
                    if !doc.tiles.contains_key(&n) && !new.contains(&n) {
                        new.push(n);
                    }
                }
            }
        }
        new.sort();
        self.history.begin(tr("Add tiles", "Добавление тайлов"));
        self.history.touch_global(doc);
        // (side neighbours first: a corner tile takes its edges' heights from them)
        new.sort_by_key(|k| std::cmp::Reverse([(-1, 0), (1, 0), (0, -1), (0, 1)].iter().filter(|(dx, dy)| doc.tiles.contains_key(&(k.0 + dx, k.1 + dy))).count()));
        for k in &new {
            self.history.touch_tile(doc, *k);
            doc.add_tile(*k);
        }
        self.history.commit();
        new.len()
    }

    fn remove_tile(&mut self, k: TileKey) {
        let Some(doc) = self.doc.as_mut() else { return };
        if !doc.tiles.contains_key(&k) {
            return;
        }
        if doc.tiles.len() == 1 {
            self.error(tr("A map needs at least one tile", "На карте должен остаться хотя бы один тайл"));
            return;
        }
        self.history.begin(tr("Remove tile", "Удаление тайла"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for t in keys {
            self.history.touch_tile(doc, t);
        }
        self.history.touch_global(doc);
        doc.remove_tile(k);
        self.history.commit();
        if self.selection().iter().any(|s| matches!(s, Sel::Object(t, _) | Sel::Spline(t, _) | Sel::Attached(t, _) if *t == k)) {
            self.select(None);
        }
    }

    // ---------------------------------------------------------------- drawing

    /// The brush, ghosts and lines this frame shows.
    /// The points junction objects offer roads to join them (their `[splinehelper]`s).
    fn helper_anchors(&self, assets: &mut Assets) -> Vec<DVec3> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for (k, t) in &doc.tiles {
            for o in &t.tile.objects {
                let Some(model) = assets.object(&o.file) else { continue };
                if model.helpers.is_empty() {
                    continue;
                }
                let world = doc.object_world(*k, o, model.abs_height);
                let m = object_matrix(DVec3::ZERO, o.rot);
                for (h, _) in &model.helpers {
                    out.push(world + m.transform_point3(Vec3::new(h[0], h[1], h[2])).as_dvec3());
                }
            }
        }
        out
    }

    pub fn extras(&self, cam: &crate::camera::Camera, assets: &mut Assets) -> FrameExtras {
        self.view_distance.set(cam.distance);
        let mut x = FrameExtras { grid: self.show_grid, selected: self.selected, selected_more: self.selection().into_iter().skip(1).collect(), hover: self.hover, ..Default::default() };
        let Some(doc) = self.doc.as_ref() else { return x };
        let w = (cam.distance * 0.004).clamp(0.08, 6.0) as f32;
        // (the dots that mark places on the ground are small: they are there to be seen, not to cover)
        let dw = w * 0.4;
        x.show_technical = self.show_technical || self.tool == Tool::Logic;
        x.hide = self.hide;
        // the AI's lanes: arrows the way traffic goes, ends green where joined, red where not
        // (thin and small: a lane is a line on the road, not a road of its own)
        if self.tool == Tool::Logic || self.show_lanes {
            let r = (cam.distance * 1.3).clamp(40.0, 500.0);
            let (lanes, ends) = crate::lanes::near(doc, assets, cam.target.truncate(), r);
            let lw = (cam.distance * 0.0012).clamp(0.03, 1.2) as f32;
            for l in lanes.iter().filter(|l| l.kind != 2) {
                let color = if l.kind == 0 { [1.0, 0.78, 0.2, 0.9] } else { [0.35, 0.8, 1.0, 0.9] };
                let lift = DVec3::Z * 0.15;
                for s in l.pts.windows(2) {
                    line(&mut x.overlay, cam, s[0] + lift, s[1] + lift, lw, color);
                }
                // a chevron every 8 m (both ways for a two-way lane)
                let mut acc = 0.0;
                for s in l.pts.windows(2) {
                    let seg = (s[1] - s[0]).length();
                    acc += seg;
                    // (further apart when seen from far, so they do not merge into a band)
                    if acc < (cam.distance * 0.06).clamp(6.0, 40.0) || seg < 1e-3 {
                        continue;
                    }
                    acc = 0.0;
                    let mid = (s[0] + s[1]) * 0.5 + lift;
                    let d = (s[1] - s[0]).truncate() / seg;
                    for back in [false, true] {
                        if (l.direction == 1) != back && l.direction != 2 {
                            continue;
                        }
                        let f = if back { -d } else { d };
                        let sz = (lw as f64 * 6.0).clamp(0.3, 4.0);
                        let tip = mid + DVec3::new(f.x, f.y, 0.0) * sz;
                        let side = DVec3::new(f.y, -f.x, 0.0) * sz * 0.6;
                        line(&mut x.overlay, cam, tip, mid - side, lw, color);
                        line(&mut x.overlay, cam, tip, mid + side, lw, color);
                    }
                }
            }
            for e in ends.iter().filter(|e| e.kind != 2) {
                disc(&mut x.overlay, cam, e.pos + DVec3::Z * 0.2, lw * 1.2, if e.linked { [0.35, 0.85, 0.45, 0.95] } else { [0.95, 0.25, 0.2, 0.95] });
            }
        }
        // an AI path has nothing to see: the picked and the hovered one drawn as lines
        // (a selection of thousands has no lines to show - the tint says what is picked, and
        // every one of them would be looked up in the assets for a path that is not drawn)
        {
            let editing = self.editing_paths();
            let sels = self.selection();
            let shown: Vec<Sel> = if editing || sels.len() <= 512 { sels } else { sels.into_iter().take(512).collect() };
            for (sel, color) in shown.into_iter().map(|s| (s, ORANGE)).chain(self.hover.map(|h| (h, WHITE))) {
                let Sel::Spline(k, id) = sel else { continue };
                let Some(s) = doc.spline(k, id) else { continue };
                if !editing && !assets.spline(&s.file).map(|m| m.is_ai_path()).unwrap_or(false) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                let pts: Vec<DVec3> = (0..=n).map(|i| c.point_at(s.length * i as f64 / n as f64) + DVec3::Z * 0.25).collect();
                for p in pts.windows(2) {
                    line(&mut x.overlay, cam, p[0], p[1], w * 0.6, color);
                }
            }
        }
        // a row of car parks being laid out
        if let (Some(a), Some(c), Tool::Logic) = (self.parking_start, self.cursor, self.tool) {
            line(&mut x.overlay, cam, a + DVec3::Z * 0.3, c + DVec3::Z * 0.3, w, ORANGE);
            let d = (c - a).truncate();
            let len = d.length();
            if len > 0.5 {
                let n = (len / self.parking_step).floor() as usize + 1;
                for i in 0..n {
                    let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                    disc(&mut x.overlay, cam, a + (c - a) * t + DVec3::Z * 0.3, dw * 2.5, ORANGE);
                }
            }
        }
        // a lake's shore or a river's middle line clicked so far, on to the cursor
        if self.tool == Tool::Terrain && self.terrain_mode == TerrainMode::Water && self.water_shape != WaterShape::Brush && !self.water_pts.is_empty() {
            let blue = [0.35, 0.65, 1.0, 1.0];
            let lift = DVec3::Z * 0.4;
            let mut pts: Vec<DVec3> = self.water_pts.clone();
            if let (Some(c), true) = (self.cursor, self.cursor_in_view) {
                pts.push(c);
            }
            for p in pts.windows(2) {
                line(&mut x.overlay, cam, p[0] + lift, p[1] + lift, w, blue);
                if self.water_shape == WaterShape::River {
                    let d = (p[1] - p[0]).truncate();
                    let side = DVec2::new(-d.y, d.x).normalize_or_zero() * self.river_width as f64 * 0.5;
                    let o = DVec3::new(side.x, side.y, 0.0);
                    for s in [o, -o] {
                        line(&mut x.overlay, cam, p[0] + s + lift, p[1] + s + lift, w * 0.5, [1.0, 1.0, 1.0, 0.6]);
                    }
                }
            }
            if self.water_shape == WaterShape::Lake && pts.len() >= 3 {
                line(&mut x.overlay, cam, pts[pts.len() - 1] + lift, pts[0] + lift, w * 0.5, [0.35, 0.65, 1.0, 0.5]);
            }
            for p in &self.water_pts {
                disc(&mut x.overlay, cam, *p + lift, dw * 3.0, blue);
            }
        }
        // a ramp being laid out: its middle and its edges, from its start to the cursor
        if let (Some(a), Some(c), true) = (self.ramp_start, self.cursor, self.tool == Tool::Terrain && self.terrain_mode == TerrainMode::Ramp) {
            let d = (c - a).truncate();
            let side = DVec2::new(-d.y, d.x).normalize_or_zero() * self.brush_radius as f64 * 0.6;
            let lift = DVec3::Z * 0.4;
            line(&mut x.overlay, cam, a + lift, c + lift, w, ORANGE);
            for s in [side, -side] {
                let o = DVec3::new(s.x, s.y, 0.0);
                line(&mut x.overlay, cam, a + o + lift, c + o + lift, w * 0.5, [1.0, 1.0, 1.0, 0.6]);
            }
            disc(&mut x.overlay, cam, a + lift, dw * 3.0, ORANGE);
        }
        // GPS tracks to trace, in magenta on the ground
        for t in &self.trace_lines {
            let on = |p: DVec2| DVec3::new(p.x, p.y, doc.ground(p) + 0.4);
            for seg in t.windows(2) {
                let n = ((seg[1] - seg[0]).length() / 6.0).ceil().clamp(1.0, 50.0) as usize;
                for i in 0..n {
                    line(&mut x.overlay, cam, on(seg[0].lerp(seg[1], i as f64 / n as f64)), on(seg[0].lerp(seg[1], (i + 1) as f64 / n as f64)), w * 0.8, [0.95, 0.3, 0.85, 0.9]);
                }
            }
        }
        // a plant outline being clicked round, closed to the cursor
        if self.tool == Tool::Objects && self.plant_brush && self.plant_area {
            let color = if self.plant_erase { [1.0, 0.4, 0.4, 1.0] } else { [0.45, 0.9, 0.45, 1.0] };
            let on = |p: DVec2| DVec3::new(p.x, p.y, doc.ground(p) + 0.3);
            let mut pts: Vec<DVec2> = self.plant_outline.clone();
            if let (Some(c), true) = (self.cursor, self.cursor_in_view) {
                pts.push(c.truncate());
            }
            // (every 4 m on the ground, so the line lies over hills)
            let edge = |x: &mut FrameExtras, a: DVec2, b: DVec2, width: f32, color: [f32; 4]| {
                let n = ((b - a).length() / 4.0).ceil().max(1.0) as usize;
                for i in 0..n {
                    line(&mut x.overlay, cam, on(a.lerp(b, i as f64 / n as f64)), on(a.lerp(b, (i + 1) as f64 / n as f64)), width, color);
                }
            };
            for seg in pts.windows(2) {
                edge(&mut x, seg[0], seg[1], w, color);
            }
            if pts.len() >= 3 {
                edge(&mut x, pts[pts.len() - 1], pts[0], w * 0.6, [color[0], color[1], color[2], 0.5]);
            }
            for (i, p) in self.plant_outline.iter().enumerate() {
                disc(&mut x.overlay, cam, on(*p), dw * if i == 0 { 4.0 } else { 2.5 }, color);
            }
        }
        match self.acts_as() {
            Tool::Objects if self.tool == Tool::Objects && self.plant_brush && self.plant_area => {}
            Tool::Objects if self.tool == Tool::Objects && self.plant_brush && self.cursor_in_view => {
                if let Some(c) = self.cursor {
                    let color = if self.plant_erase { [1.0, 0.4, 0.4, 1.0] } else { [0.45, 0.9, 0.45, 1.0] };
                    x.brush = Some(Brush { centre: c.truncate(), radius: self.brush_radius, color });
                }
            }
            Tool::Terrain | Tool::Paint if self.cursor_in_view => {
                if let Some(c) = self.cursor {
                    let color = match (self.tool, self.terrain_mode, self.paint_erase) {
                        (Tool::Terrain, TerrainMode::Lower | TerrainMode::Water, _) => [0.35, 0.65, 1.0, 1.0],
                        (Tool::Paint, _, true) => [1.0, 0.4, 0.4, 1.0],
                        _ => [1.0, 1.0, 1.0, 1.0],
                    };
                    x.brush = Some(Brush { centre: c.truncate(), radius: self.brush_radius, color });
                }
            }
            Tool::Roads => {
                // the switches the track being drawn lays
                for p in &self.switch_plans {
                    let o = DVec3::new(p.origin.x, p.origin.y, doc.ground(p.origin.truncate()));
                    x.ghosts.push(Ghost { rel: p.rel.clone(), world: o, rot: [p.heading, 0.0, 0.0], tint: [0.35, 0.85, 0.45, 0.4] });
                }
                // a roundabout being laid out: its ring
                if let (Some((c, r)), Some(rel)) = (self.ring_radius(), &self.road_type) {
                    for sg in crate::roundabout::ring(c.truncate(), r, doc.global.left_hand_traffic) {
                        let mut s = piece_spline(rel, &sg, &[(c.z, 0.0), (c.z, 0.0)], 0, DVec2::ZERO);
                        s.pos[2] += 0.05;
                        x.ghost_splines.push((rel.clone(), s, [0.35, 0.85, 0.45, 0.3]));
                    }
                    disc(&mut x.overlay, cam, c, dw * 3.0, GREEN);
                }
                if let Some(rel) = &self.road_type {
                    if let Some((segs, z0, z1, _)) = &self.road_plan {
                        let pieces = roads::split(segs, 50.0);
                        let total = roads::total_length(&pieces).max(0.1);
                        let _ = total;
                        let profile = if self.keep_height || !self.auto_terrain { even_profile(&pieces, *z0, *z1) } else { road_profile(doc, &pieces, *z0, *z1) };
                        for (i, p) in pieces.iter().enumerate() {
                            let mut s = piece_spline(rel, p, &profile, i, DVec2::ZERO);
                            s.pos[2] += 0.02;
                            x.ghost_splines.push((rel.clone(), s, [0.35, 0.85, 0.45, 0.25]));
                        }
                        for p in &pieces {
                            let c = p.curve();
                            let n = (p.length / 2.0).ceil().max(1.0) as usize;
                            for i in 0..n {
                                let a = c.point_at(p.length * i as f64 / n as f64);
                                let b = c.point_at(p.length * (i + 1) as f64 / n as f64);
                                let za = doc.ground(a.truncate()) + 0.3;
                                let zb = doc.ground(b.truncate()) + 0.3;
                                line(&mut x.overlay, cam, DVec3::new(a.x, a.y, za), DVec3::new(b.x, b.y, zb), w * 0.6, [1.0, 1.0, 1.0, 0.5]);
                            }
                        }
                        if total > 1.0 {
                            let _ = total;
                        }
                    }
                }
                if let Some((s, _)) = &self.road_start {
                    disc(&mut x.overlay, cam, s.pos, dw * 3.0, GREEN);
                }
                // every free end, so the user sees where roads connect (not those that
                // already join a junction object)
                let joined = if self.drawing_paths() { Vec::new() } else { self.helper_anchors(assets) };
                let free = |p: DVec3| !joined.iter().any(|j| (j.truncate() - p.truncate()).length() < 0.6);
                for (k, t) in doc.tiles.iter().filter(|_| !self.drawing_paths()) {
                    let org = tile_origin(*k);
                    for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                        let c = omsi_geometry::SplineCurve::from_map(s, org);
                        if s.prev_id == 0 && free(c.point_at(0.0)) {
                            disc(&mut x.overlay, cam, c.point_at(0.0), dw * 2.0, [1.0, 1.0, 1.0, 0.6]);
                        }
                        if s.next_id == 0 && free(c.end_point()) {
                            disc(&mut x.overlay, cam, c.end_point(), dw * 2.0, [1.0, 1.0, 1.0, 0.6]);
                        }
                    }
                }
                if let Some(a) = &self.road_snap {
                    disc(&mut x.overlay, cam, a.pos, dw * 2.5, ORANGE);
                    let d = roads::dir(a.heading);
                    line(&mut x.overlay, cam, a.pos, a.pos + DVec3::new(d.x, d.y, 0.0) * (w as f64 * 25.0), w, ORANGE);
                } else if let (Some(c), true) = (self.cursor, self.cursor_in_view) {
                    disc(&mut x.overlay, cam, c, dw * 2.0, WHITE);
                }
            }
            Tool::Objects => {
                // the points a junction piece or a road can be joined at, near the cursor
                if let (Some(c), true) = (self.cursor, self.cursor_in_view) {
                    for a in self.free_road_ends(&mut *assets, c.truncate(), (cam.distance * 0.6).clamp(30.0, 200.0)) {
                        disc(&mut x.overlay, cam, a.pos + DVec3::Z * 0.2, dw * 2.0, [1.0, 1.0, 1.0, 0.7]);
                    }
                }
                if let (Some((p, h)), Some(rel)) = (self.ghost_pos, &self.object_type) {
                    if self.cursor_in_view {
                        let ok = doc.tiles.contains_key(&tile_of(p.truncate()));
                        x.ghosts.push(Ghost { rel: rel.clone(), world: p, rot: [h, 0.0, 0.0], tint: if ok { [0.35, 0.85, 0.45, 0.35] } else { [1.0, 0.3, 0.3, 0.5] } });
                        if let Some((p2, h2)) = self.stop_twin {
                            x.ghosts.push(Ghost { rel: rel.clone(), world: p2, rot: [h2, 0.0, 0.0], tint: [0.35, 0.85, 0.45, 0.25] });
                        }
                        // an arrow for the front
                        let d = roads::dir(h);
                        line(&mut x.overlay, cam, p + DVec3::Z * 0.3, p + DVec3::new(d.x * 6.0, d.y * 6.0, 0.3), w, GREEN);
                        // set against a road: the road's edge it keeps its distance from
                        if let Some((edge, at)) = self.align_guide {
                            line(&mut x.overlay, cam, edge + DVec3::Z * 0.3, at + DVec3::Z * 0.3, w * 0.5, WHITE);
                            disc(&mut x.overlay, cam, edge + DVec3::Z * 0.3, w * 1.5, ORANGE);
                        }
                    }
                }
            }
            Tool::Tiles => {
                x.highlight_tile = self.hover_tile;
                if let Some(k) = self.hover_tile {
                    if !doc.tiles.contains_key(&k) {
                        let o = tile_origin(k);
                        let ts = tile_size();
                        let pts = [DVec3::new(o.x, o.y, 0.0), DVec3::new(o.x + ts, o.y, 0.0), DVec3::new(o.x + ts, o.y + ts, 0.0), DVec3::new(o.x, o.y + ts, 0.0)];
                        for i in 0..4 {
                            line(&mut x.overlay, cam, pts[i], pts[(i + 1) % 4], w * 2.0, GREEN);
                        }
                    }
                }
                // the outline of every tile that may be added
                for k in doc.tiles.keys() {
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let n = (k.0 + dx, k.1 + dy);
                        if doc.tiles.contains_key(&n) {
                            continue;
                        }
                        let o = tile_origin(n);
                        let ts = tile_size();
                        let h = doc.ground(tile_origin(*k) + DVec2::splat(ts * 0.5));
                        let c = DVec3::new(o.x + ts * 0.5, o.y + ts * 0.5, h);
                        disc(&mut x.overlay, cam, c, w * 6.0, [1.0, 1.0, 1.0, 0.35]);
                    }
                }
            }
            _ => {}
        }
        self.selection_extras(&mut x, cam, w, assets);
        x.resized = self.scale_preview.clone();
        if let (Some(m), Some(view)) = (self.mesh.as_ref().filter(|m| m.editing), self.gizmo_view.as_ref()) {
            m.extras(view, &mut x);
            if !m.tex_preview || m.xray {
                x.resized.push((m.inst, DVec3::ZERO));
            }
            x.hover = None;
        }
        self.gizmo_extras(&mut x);
        // the grid things snap to, round the cursor (fading out), while snapping is on
        if let (true, Some(c)) = (self.snap_grid, self.cursor) {
            let step = self.grid_step.max(0.1);
            // (no more lines than a few dozen on the screen: a fine step seen from afar shows
            // every so many)
            let every = ((cam.distance * 0.02) / step).ceil().max(1.0);
            let cell = step * every;
            let n = 12i32;
            let base = DVec2::new((c.x / cell).round() * cell, (c.y / cell).round() * cell);
            let reach = cell * n as f64;
            for i in -n..=n {
                for (along_x, off) in [(true, i as f64 * cell), (false, i as f64 * cell)] {
                    // (each line in pieces, so it lies on the ground)
                    for j in -n..n {
                        let (s0, s1) = (j as f64 * cell, (j + 1) as f64 * cell);
                        let (p0, p1) = if along_x { (base + DVec2::new(s0, off), base + DVec2::new(s1, off)) } else { (base + DVec2::new(off, s0), base + DVec2::new(off, s1)) };
                        let fade = 1.0 - ((p0 + p1) * 0.5 - c.truncate()).length() / reach;
                        if fade <= 0.0 {
                            continue;
                        }
                        let z0 = doc.ground(p0) + 0.15;
                        let z1 = doc.ground(p1) + 0.15;
                        line(&mut x.overlay, cam, DVec3::new(p0.x, p0.y, z0), DVec3::new(p1.x, p1.y, z1), w * 0.35, [1.0, 1.0, 1.0, 0.35 * fade as f32]);
                    }
                }
            }
        }
        // the knife: a line across the road where a click cuts it
        if let Some(c) = self.cut_preview() {
            let half = doc.spline(c.key, c.id).and_then(|s| assets.spline(&s.file)).map(|m| m.half_width as f64).unwrap_or(4.0).max(1.5) + 0.5;
            let side = DVec3::new(c.dir.y, -c.dir.x, 0.0) * half;
            let lift = DVec3::Z * 0.4;
            let col = if c.fits { [1.0, 0.25, 0.2, 1.0] } else { [0.6, 0.6, 0.6, 1.0] };
            line(&mut x.overlay, cam, c.pos - side + lift, c.pos + side + lift, w * 1.5, col);
            disc(&mut x.overlay, cam, c.pos + lift, w * 2.5, col);
        }
        if let Some((a, b)) = self.ruler_line() {
            let lift = DVec3::Z * 0.5;
            line(&mut x.overlay, cam, a + lift, b + lift, w, [1.0, 0.85, 0.2, 1.0]);
            for p in [a, b] {
                disc(&mut x.overlay, cam, p + lift, w * 2.5, [1.0, 0.85, 0.2, 1.0]);
            }
        }
        x
    }
}

/// The free road ends within `reach` of `centre` that face it (the arms a junction plate
/// there had), nearest first.
fn ends_facing(doc: &Document, centre: DVec2, reach: f64) -> Vec<crate::junction::Arm> {
    let mut found: Vec<(f64, crate::junction::Arm)> = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            let c = omsi_geometry::SplineCurve::from_map(s, org);
            for at_end in [false, true] {
                if (if at_end { s.next_id } else { s.prev_id }) != 0 {
                    continue;
                }
                let (p, h) = if at_end { (c.end_point(), c.heading_at(s.length)) } else { (c.point_at(0.0), c.heading_at(0.0) + 180.0) };
                let to = centre - p.truncate();
                let d = to.length();
                let off = ((roads::heading_of(to) - h + 540.0).rem_euclid(360.0) - 180.0).abs();
                if d < reach && (d < 1.0 || off < 35.0) {
                    found.push((d, crate::junction::Arm { key: *k, id: s.id, at_end }));
                }
            }
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|f| f.1).collect()
}

/// A junction openMapper made (and can make again): `Sceneryobjects\openMapper\<map>\junction_<id>.sco`.
/// A junction plate's helper points (x, forward, height, heading out) and whether it stands
/// at a height of its own: from its model, or, for a plate made in this very build and not
/// loaded yet, read from its file.
fn plate_helpers(content: &std::path::Path, assets: &mut Assets, file: &str) -> Option<(bool, Vec<[f32; 4]>)> {
    if let Some(m) = assets.object(file) {
        return Some((m.abs_height, m.helpers.iter().map(|h| h.0).collect()));
    }
    let sco = omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(content, file)).ok()?;
    // (OMSI's order: x, height, forward)
    Some((sco.absolute_height(), sco.spline_helpers.iter().map(|h| [h.x, h.z, h.y, h.heading + 180.0]).collect()))
}

pub fn is_own_junction(file: &str) -> bool {
    let f = file.to_lowercase();
    f.contains("\\openmapper\\") && f.contains("junction_")
}

/// A heading to the nearest whole 15 degrees.
pub fn snap_deg(h: f64, step: f64) -> f64 {
    let step = if step > 0.0 { step } else { 15.0 };
    roads::norm_deg((h / step).round() * step)
}

// -------------------------------------------------------------------- selection and editing

impl Editor {
    /// The parent an object takes its state from, and the objects that take theirs from it
    /// (`[varparent]`), both by tile and IDCode. Read from the whole map the first time it is
    /// asked for and kept while the map does not change.
    pub fn links_of(&mut self, id: i64, parent: Option<i64>) -> (Option<(TileKey, i64)>, Vec<(TileKey, i64)>) {
        let Some(doc) = self.doc.as_ref() else { return (None, Vec::new()) };
        let rev = doc.content_rev();
        if let Some((cached_rev, cached_id, got)) = &self.links_cache {
            if *cached_rev == rev && *cached_id == id {
                return got.clone();
            }
        }
        let parent = parent.and_then(|p| doc.tiles.iter().find_map(|(k, t)| (t.tile.objects.iter().chain(&t.tile.attach_objects)).any(|o| o.id == p).then_some((*k, p))));
        let mut children = Vec::new();
        for (k, t) in &doc.tiles {
            for o in t.tile.objects.iter().chain(&t.tile.attach_objects) {
                if o.var_parent == Some(id) {
                    children.push((*k, o.id));
                }
            }
        }
        let got = (parent, children);
        self.links_cache = Some((rev, id, got.clone()));
        got
    }

    /// Everything selected, the one shown in the properties first.
    ///
    /// `also` is kept free of duplicates and of the one shown first (see
    /// [`set_selection`](Self::set_selection)), so this is a copy and nothing more: the windows
    /// and the view read the selection several times a frame, and a selection of a hundred
    /// thousand deduplicated on each read was most of a frame.
    /// The selection grows and the map's picture with it: the minimap can wait.
    pub fn moving(&self) -> bool {
        self.grab.is_some() || !matches!(self.drag, Drag::None)
    }

    pub fn selection(&self) -> Vec<Sel> {
        let Some(p) = self.selected else { return Vec::new() };
        let mut v: Vec<Sel> = Vec::with_capacity(1 + self.also.len());
        v.push(p);
        v.extend_from_slice(&self.also);
        v
    }

    pub fn is_selected(&self, s: Sel) -> bool {
        self.selected == Some(s) || (self.selected.is_some() && self.also.contains(&s))
    }

    /// Select one thing (or nothing).
    pub fn select(&mut self, s: Option<Sel>) {
        self.selected = s;
        self.path_point = None;
        self.path_connect = None;
        self.also.clear();
    }

    pub fn set_selection(&mut self, v: Vec<Sel>) {
        let mut v = v.into_iter();
        self.selected = v.next();
        self.path_point = None;
        self.path_connect = None;
        let rest: Vec<Sel> = v.collect();
        // (deduplicated here once, so every read of the selection is a copy and nothing more:
        // the windows and the view read it several times a frame, and a selection of a
        // hundred thousand deduplicated on each read was most of a frame)
        self.also.clear();
        if rest.len() <= 1 {
            self.also.extend(rest.into_iter().filter(|s| Some(*s) != self.selected));
            return;
        }
        let mut seen: std::collections::HashSet<Sel> = std::collections::HashSet::with_capacity(rest.len());
        if let Some(p) = self.selected {
            seen.insert(p);
        }
        self.also.extend(rest.into_iter().filter(|s| seen.insert(*s)));
    }

    /// As [`set_selection`](Self::set_selection), for a list already free of duplicates and of
    /// the one that goes first (what a drag or a grab carries: it was read from a selection
    /// and moved, so nothing of it is twice).
    pub fn set_selection_unique(&mut self, v: Vec<Sel>) {
        let mut v = v.into_iter();
        self.selected = v.next();
        self.path_point = None;
        self.path_connect = None;
        self.also.clear();
        self.also.extend(v);
    }

    /// Shift-click: in the selection if it was not, out if it was.
    pub fn toggle_selected(&mut self, s: Sel) {
        let mut v = self.selection();
        // (a group comes and goes as one)
        let whole = if self.ctrl_held { vec![s] } else { self.with_groups(vec![s]) };
        if v.contains(&s) {
            v.retain(|x| !whole.contains(x));
        } else {
            for (i, w) in whole.into_iter().enumerate() {
                if !v.contains(&w) {
                    v.insert(i, w);
                }
            }
        }
        self.set_selection(v);
    }

    /// The spline selected alone (its grips are shown).
    pub fn single_spline(&self) -> Option<(TileKey, i64)> {
        match (self.selected, self.also.iter().any(|s| Some(*s) != self.selected)) {
            (Some(Sel::Spline(k, id)), false) => Some((k, id)),
            _ => None,
        }
    }

    /// Where the grips are: a road's three (its two ends and its middle); on an AI path, a grip
    /// at each of its points (the ends and the joints of its pieces) and one in the middle of
    /// each piece, to bend it.
    fn handles(&self) -> Vec<((TileKey, i64), Handle, DVec3)> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let pieces: Vec<(TileKey, i64)> = if self.editing_paths() {
            self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect()
        } else {
            self.single_spline().into_iter().collect()
        };
        let mut out = Vec::new();
        for (k, id) in pieces.iter().copied() {
            let Some(s) = doc.spline(k, id) else { continue };
            let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
            // (a joint is the grip of the piece that ends there)
            let joined_before = self.editing_paths() && s.prev_id != 0 && pieces.iter().any(|p| p.1 == s.prev_id);
            if !joined_before {
                out.push(((k, id), Handle::Start, c.point_at(0.0)));
            }
            out.push(((k, id), Handle::End, c.end_point()));
            out.push(((k, id), Handle::Mid, c.point_at(s.length * 0.5)));
        }
        out
    }

    /// A grip as a ball: its centre and radius. The radius follows the size of the spline
    /// (a kerb's grips are small, a street's larger), but never drops below a few pixels
    /// on the screen, however far the camera is.
    fn grip(&self, assets: &mut Assets, sp: (TileKey, i64), h: Handle, at: DVec3, dist: f64) -> (DVec3, f64) {
        let half = self.doc.as_ref().and_then(|d| d.spline(sp.0, sp.1)).and_then(|s| assets.spline(&s.file)).map(|m| m.half_width as f64).unwrap_or(1.5);
        let r = (half * 0.05).max(dist * 0.0025).clamp(0.05, 1.0);
        let r = if h == Handle::Mid { r * 0.75 } else { r };
        (at + DVec3::Z * (r * 0.5), r)
    }

    /// The grip the ray from `o` along `d` passes through (or just by: a grip takes clicks
    /// well beyond its edge). A point beats the middle of a piece when they lie close.
    fn handle_at_ray(&self, assets: &mut Assets, o: DVec3, d: DVec3) -> Option<((TileKey, i64), Handle)> {
        let dir = d.normalize_or_zero();
        if dir == DVec3::ZERO {
            return None;
        }
        self.handles()
            .into_iter()
            .filter_map(|(sp, h, q)| {
                let t = (q - o).dot(dir);
                if t <= 0.0 {
                    return None;
                }
                let (c, r) = self.grip(assets, sp, h, q, t);
                let off = (c - (o + dir * (c - o).dot(dir))).length();
                (off < r * 1.8).then_some(((sp, h), off / r + if h == Handle::Mid { 0.3 } else { 0.0 }))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(x, _)| x)
    }

    /// Take the picked point out of an AI path: the two pieces that meet there become one (its
    /// shape fitted again between their far ends), or, at an end, the last piece goes.
    fn remove_path_point(&mut self) {
        let Some(((k, id), h)) = self.path_point.take() else { return };
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(s) = doc.spline(k, id).cloned() else { return };
        // (the piece before the point and the one after it)
        let (a, b) = match h {
            Handle::End if s.next_id != 0 => (Some((k, id)), doc.find_spline(s.next_id).map(|(nk, n)| (nk, n.id))),
            Handle::Start if s.prev_id != 0 => (doc.find_spline(s.prev_id).map(|(pk, p)| (pk, p.id)), Some((k, id))),
            Handle::Mid => {
                self.toast(tr("Pick a point (a dot at an end or where pieces meet) to take it out", "Выберите точку (кружок на конце или на стыке кусков), чтобы убрать её"));
                return;
            }
            // an end of the path: its last piece goes, if there is another
            _ => (None, None),
        };
        match (a, b) {
            (Some(a), Some(b)) => {
                let (Some(sa), Some(sb)) = (doc.spline(a.0, a.1).cloned(), doc.spline(b.0, b.1).cloned()) else { return };
                let (ca, cb) = (omsi_geometry::SplineCurve::from_map(&sa, tile_origin(a.0)), omsi_geometry::SplineCurve::from_map(&sb, tile_origin(b.0)));
                let (p0, p1) = (ca.point_at(0.0), cb.end_point());
                let keep0 = sa.prev_id != 0;
                let keep1 = sb.next_id != 0;
                let segs = reshape::fit(
                    reshape::End { pos: p0.truncate(), heading: keep0.then(|| roads::norm_deg(sa.heading)) },
                    reshape::End { pos: p1.truncate(), heading: keep1.then(|| roads::norm_deg(cb.heading_at(sb.length))) },
                )
                .or_else(|| reshape::fit(reshape::End { pos: p0.truncate(), heading: None }, reshape::End { pos: p1.truncate(), heading: None }));
                let Some(segs) = segs else {
                    self.error(tr("That bend is too sharp", "Слишком крутой поворот"));
                    return;
                };
                self.history.begin(tr("Remove a point of the path", "Убрать точку пути"));
                let plan = reshape::Plan { pieces: vec![reshape::Piece { key: a.0, id: a.1, segs, z0: p0.z, z1: p1.z, g0: None, g1: None }] };
                self.apply_plan(&plan);
                if let Some(doc) = self.doc.as_mut() {
                    // (the second piece goes; what followed it follows the first)
                    for t in [a.0, b.0] {
                        self.history.touch_tile(doc, t);
                    }
                    let after = sb.next_id;
                    doc.delete_spline(b.0, b.1);
                    if let Some((nk, _)) = doc.find_spline(after) {
                        self.history.touch_tile(doc, nk);
                    }
                    if let Some(t) = doc.tiles.get_mut(&a.0) {
                        if let Some(x) = t.tile.splines.iter_mut().find(|x| x.id == a.1) {
                            x.next_id = after;
                        }
                        t.touch_content();
                    }
                    if after != 0 {
                        if let Some((nk, _)) = doc.find_spline(after) {
                            if let Some(t) = doc.tiles.get_mut(&nk) {
                                for x in t.tile.splines.iter_mut().filter(|x| x.id == after) {
                                    x.prev_id = a.1;
                                }
                                t.touch_content();
                            }
                        }
                    }
                }
                self.history.commit();
                self.select_path_pieces(a.0, a.1);
                self.toast(tr("Point taken out", "Точка убрана"));
            }
            _ => {
                // an end: the piece that ends the path goes, if the path has more than that one
                let linked = if h == Handle::Start { s.next_id != 0 } else { s.prev_id != 0 };
                if !linked {
                    self.toast(tr("A path of one piece: Del with nothing picked deletes it", "Путь из одного куска: Del без выбранной точки удалит его целиком"));
                    return;
                }
                self.history.begin(tr("Remove a point of the path", "Убрать точку пути"));
                if let Some(doc) = self.doc.as_mut() {
                    let (other, other_end) = if h == Handle::Start { (s.next_id, false) } else { (s.prev_id, true) };
                    self.history.touch_tile(doc, k);
                    if let Some((ok, _)) = doc.find_spline(other) {
                        self.history.touch_tile(doc, ok);
                        if let Some(t) = doc.tiles.get_mut(&ok) {
                            for x in t.tile.splines.iter_mut().filter(|x| x.id == other) {
                                if other_end { x.next_id = 0 } else { x.prev_id = 0 }
                            }
                            t.touch_content();
                        }
                        doc.delete_spline(k, id);
                        self.history.commit();
                        self.select_path_pieces(ok, other);
                        return;
                    }
                }
                self.history.commit();
            }
        }
    }

    /// The Move / Rotate / Scale handles of the selection (None: the Select tool's plain
    /// mode, nothing selected, or a single road, which has grips of its own).
    pub fn gizmo(&self) -> Option<crate::gizmo::Gizmo> {
        use crate::gizmo::{Gizmo, Mode};
        if let Some(m) = self.mesh.as_ref().filter(|m| m.editing) {
            // (in the edit mode the handles move what is selected, even in plain Select)
            return m.gizmo(match self.xform {
                Xform::Rotate => Mode::Rotate,
                Xform::Scale => Mode::Scale,
                _ => Mode::Move,
            });
        }
        if self.tool != Tool::Select || self.xform == Xform::Select || self.cutting || self.ruler.is_some() {
            return None;
        }
        // (a road selected alone has no handles: read from the selection kept rather than from a
        // copy of it made again - copies of a selection of thousands were most of a frame)
        if matches!(self.selected, Some(Sel::Spline(..))) && self.also.iter().all(|s| Some(*s) == self.selected) {
            return None;
        }
        let sels = self.selection();
        if sels.is_empty() {
            return None;
        }
        let doc = self.doc.as_ref()?;
        // (a selection of thousands: its middle read from every so many of them - a reading of
        // each, the whole map walked each frame, was most of a frame)
        let step = (sels.len() / 1024).max(1);
        let pts: Vec<DVec3> = sels.iter().step_by(step).filter_map(|s| self.world_of(doc, *s)).collect();
        if pts.is_empty() {
            return None;
        }
        let pivot = pts.iter().sum::<DVec3>() / pts.len() as f64;
        let mode = match self.xform {
            Xform::Rotate => Mode::Rotate,
            Xform::Scale => Mode::Scale,
            _ => Mode::Move,
        };
        // (one object scales along its own axes; the rest use the world's)
        let axes = match (mode, sels.as_slice()) {
            (Mode::Scale, [Sel::Object(k, id)]) => doc.object(*k, *id).map(|o| glam::DMat3::from_quat(crate::gizmo::object_quat(o.rot))).unwrap_or(glam::DMat3::IDENTITY),
            _ => glam::DMat3::IDENTITY,
        };
        // (a road turns only about the vertical: tilting it is its climb and banking)
        let roads = sels.iter().any(|s| matches!(s, Sel::Spline(..)));
        let allowed = if mode == Mode::Rotate && roads { [false, false, true] } else { [true; 3] };
        Some(Gizmo { mode, pivot, axes, allowed, cap: self.gizmo_cap.get() })
    }

    /// Where a selected thing stands in the world: an object's origin, a road's middle.
    fn world_of(&self, doc: &Document, s: Sel) -> Option<DVec3> {
        match s {
            Sel::Object(k, id) => {
                let o = doc.object(k, id)?;
                let abs = self.known_helpers.get(&o.file.to_lowercase()).map(|h| h.0).unwrap_or(false);
                Some(doc.object_world(k, o, abs))
            }
            Sel::Spline(k, id) => doc.spline(k, id).map(|sp| omsi_geometry::SplineCurve::from_map(sp, tile_origin(k)).point_at(sp.length * 0.5)),
            Sel::Attached(k, id) => selection::attachment_first(doc, k, id).map(|f| f.pose.pos),
        }
    }

    /// The view the handles are picked and dragged in, each frame.
    pub fn set_gizmo_view(&mut self, cam: &crate::camera::Camera, w: f32, h: f32, x: f32, y: f32) {
        self.gizmo_view = Some(crate::gizmo::View { cam: cam.clone(), w: w.max(1.0), h: h.max(1.0), pointer: DVec2::new(x as f64, y as f64) });
    }

    /// A drag let go of without its end (cancelled with Esc).
    pub fn stop_drag(&mut self) {
        self.drag = Drag::None;
    }

    /// What a drag of the handles or a sized operation has come to, for the status bar.
    pub fn drag_readout(&self) -> Option<String> {
        use crate::gizmo::Mode;
        // (the grab: how far it has gone, and the keys that steer it)
        if let Some(g) = &self.grab {
            let axis = match g.axis_name() {
                "" => String::new(),
                a => format!(" {a}"),
            };
            let d = g.delta;
            return Some(format!(
                "{} {axis} {:.3} m ({:.3}, {:.3}, {:.3}) · {}",
                tr("Grab", "Захват"),
                d.length(),
                d.x,
                d.y,
                d.z,
                tr("X / Y / Z: lock the axis, numbers: how far, Enter or a click: done, Esc: cancel", "X / Y / Z: закрепить ось, цифры: как далеко, Enter или клик: готово, Esc: отмена")
            ));
        }
        if let Some((kind, v)) = self.mesh.as_ref().and_then(|m| m.modal_value()) {
            let what = match kind {
                crate::meshedit::Sizing::Extrude => tr("Extrude", "Выдавливание"),
                crate::meshedit::Sizing::Inset => tr("Inset", "Вставка"),
                crate::meshedit::Sizing::Bevel => tr("Bevel", "Фаска"),
                crate::meshedit::Sizing::LoopCut(_) => tr("Loop cut", "Разрез петлёй"),
            };
            return Some(format!("{what} {v:.3} · {}", tr("click: done, right click / Esc: cancel, wheel: segments", "клик: готово, правый клик / Esc: отмена, колесо: сегменты")));
        }
        let mode = match self.drag {
            Drag::Gizmo { grab, started: true } | Drag::MeshGizmo { grab, started: true } => grab.gizmo.mode,
            _ => return None,
        };
        Some(match mode {
            Mode::Move => format!("{} {:.3} m", tr("Move", "Перемещение"), self.xform_value),
            Mode::Rotate => format!("{} {:.1}°", tr("Rotate", "Поворот"), self.xform_value),
            Mode::Scale => format!("{} ×{:.3}", tr("Scale", "Масштаб"), self.xform_value),
        })
    }

    pub fn gizmo_view(&self) -> Option<crate::gizmo::View> {
        self.gizmo_view.clone()
    }

    /// Whether the cursor is on a handle (a press there takes it instead of the map).
    pub fn gizmo_under_cursor(&self) -> bool {
        matches!(self.drag, Drag::Gizmo { .. } | Drag::MeshGizmo { .. }) || matches!((self.gizmo(), &self.gizmo_view), (Some(g), Some(v)) if g.hit(v).is_some())
    }

    fn start_gizmo(&mut self, gz: crate::gizmo::Gizmo, part: crate::gizmo::Part, view: &crate::gizmo::View) {
        let Some(doc) = self.doc.as_ref() else { return };
        let sels = self.selection();
        let mut from: Vec<Origin> = sels.iter().filter_map(|s| selection::origin_of(doc, *s)).collect();
        let mut partial = false;
        if gz.mode == crate::gizmo::Mode::Scale {
            let had = from.len();
            from.retain(|o| matches!(o.sel, Sel::Object(..)));
            if from.is_empty() {
                self.toast(tr("Roads are not scaled: change their length and width in their properties", "Дороги не масштабируются: длина и ширина меняются в их свойствах"));
                return;
            }
            partial = from.len() < had;
        }
        self.move_z_from = from.iter().map(|o| match o.sel {
            Sel::Object(k, id) => doc.object(k, id).map(|o| o.pos[2]).unwrap_or(0.0),
            Sel::Spline(k, id) => doc.spline(k, id).map(|s| s.pos[2]).unwrap_or(0.0),
            Sel::Attached(k, id) => doc.attachment(k, id).map(|a| a.offset[1]).unwrap_or(0.0),
        })
        .collect();
        self.move_rot_from = from.iter().map(|o| match o.sel {
            Sel::Object(k, id) => doc.object(k, id).map(|o| o.rot).unwrap_or([0.0; 3]),
            Sel::Attached(k, id) => doc.attachment(k, id).map(|a| a.rot).unwrap_or([0.0; 3]),
            Sel::Spline(..) => [0.0; 3],
        }).collect();
        self.move_world_from = from.iter().map(|o| self.world_of(doc, o.sel).unwrap_or(DVec3::new(o.pos.x, o.pos.y, 0.0))).collect();
        self.move_from = from;
        self.xform_value = 0.0;
        self.drag = Drag::Gizmo { grab: crate::gizmo::Grab::new(gz, part, view), started: false };
        if partial {
            self.toast(tr("Only the objects are scaled, the roads stay", "Масштабируются только объекты, дороги остаются"));
        }
    }

    /// A handle dragged: the selection follows from where it stood when it was taken.
    fn drag_gizmo(&mut self, grab: crate::gizmo::Grab, started: bool) {
        use crate::gizmo::Change;
        let Some(view) = self.gizmo_view.clone() else { return };
        let change = grab.change(&view, self.snap_angle.then_some(self.angle_step));
        if !started {
            if !grab.moved(&view) || self.move_from.is_empty() {
                return;
            }
            self.history.begin(match change {
                Change::Move(_) => tr("Move", "Перемещение"),
                Change::Turn(..) => tr("Rotate", "Поворот"),
                Change::Scale(_) => tr("Scale", "Масштаб"),
            });
            self.drag = Drag::Gizmo { grab, started: true };
            self.move_arms = self.arms_for(&self.selection());
        }
        match change {
            Change::Move(d) => self.gizmo_move(grab.part, d),
            Change::Turn(axis, deg) => self.gizmo_turn(grab.gizmo.pivot, axis, deg),
            Change::Scale(s) => self.gizmo_scale(&grab.gizmo, s),
        }
    }

    fn gizmo_move(&mut self, part: crate::gizmo::Part, d: DVec3) {
        let first = self.move_from[0].pos;
        let mut delta = d.truncate();
        if delta.length() > 1e-9 {
            delta = self.snap_point(first + delta) - first;
            // (the grid may not pull it off the arrow it is held to)
            match part {
                crate::gizmo::Part::Axis(0) => delta.y = 0.0,
                crate::gizmo::Part::Axis(1) => delta.x = 0.0,
                _ => {}
            }
        }
        let dz = if self.snap_grid { (d.z / 0.05).round() * 0.05 } else { d.z };
        let Some(doc) = self.doc.as_mut() else { return };
        let now = selection::transform(doc, &mut self.history, &self.move_from, DVec2::ZERO, 0.0, delta);
        for (i, o) in self.move_from.iter().enumerate() {
            set_height(doc, &mut self.history, o.sel, self.move_z_from[i] + dz);
        }
        let all: Vec<selection::Arm> = self.move_arms.iter().flat_map(|g| g.1.iter().copied()).collect();
        self.carry_preview = selection::carry_plans(doc, &all, DVec2::ZERO, 0.0, delta, 0.0, &|_| false).into_iter().flatten().collect();
        self.move_delta = delta;
        self.xform_value = DVec3::new(delta.x, delta.y, dz).length();
        self.set_selection_unique(now);
    }

    fn gizmo_turn(&mut self, pivot: DVec3, axis: DVec3, deg: f64) {
        let Some(doc) = self.doc.as_mut() else { return };
        // about the vertical: as the Select tool always turned things (roads too)
        if axis.z.abs() > 0.999 {
            let cw = -deg * axis.z.signum();
            let now = selection::transform(doc, &mut self.history, &self.move_from, pivot.truncate(), cw, DVec2::ZERO);
            let all: Vec<selection::Arm> = self.move_arms.iter().flat_map(|g| g.1.iter().copied()).collect();
            self.carry_preview = selection::carry_plans(doc, &all, pivot.truncate(), cw, DVec2::ZERO, 0.0, &|_| false).into_iter().flatten().collect();
            self.xform_value = cw;
            self.set_selection_unique(now);
            return;
        }
        let q = glam::DQuat::from_axis_angle(axis, deg.to_radians());
        let mut now = Vec::new();
        for (i, o) in self.move_from.iter().enumerate() {
            let Sel::Object(_, id) = o.sel else { continue };
            let p = pivot + q * (self.move_world_from[i] - pivot);
            let rot = crate::gizmo::object_rot(q * crate::gizmo::object_quat(self.move_rot_from[i]));
            if let Some(s) = put_object_3d(doc, &mut self.history, id, p, rot, &self.known_helpers) {
                now.push(s);
            }
        }
        self.xform_value = deg;
        self.set_selection_unique(now);
    }

    fn gizmo_scale(&mut self, gz: &crate::gizmo::Gizmo, s: DVec3) {
        let Some(doc) = self.doc.as_mut() else { return };
        let mut now = Vec::new();
        let mut preview = Vec::new();
        for (i, o) in self.move_from.iter().enumerate() {
            let Sel::Object(_, id) = o.sel else { continue };
            // where it stands spreads from the middle along the handles' axes
            let local = gz.axes.transpose() * (self.move_world_from[i] - gz.pivot);
            let p = gz.pivot + gz.axes * (local * s);
            if let Some(sel) = put_object_3d(doc, &mut self.history, id, p, self.move_rot_from[i], &self.known_helpers) {
                now.push(sel);
                // and its own size, along its own axes
                let own = glam::DMat3::from_quat(crate::gizmo::object_quat(self.move_rot_from[i]));
                let world_scale = gz.axes * glam::DMat3::from_diagonal(s) * gz.axes.transpose();
                let f = DVec3::new((world_scale * own.col(0)).length(), (world_scale * own.col(1)).length(), (world_scale * own.col(2)).length());
                preview.push((sel, f));
            }
        }
        self.xform_value = if (s.x - s.y).abs() < 1e-9 && (s.y - s.z).abs() < 1e-9 { s.x } else { s.max_element().max(s.min_element()) };
        self.scale_preview = preview;
        self.set_selection_unique(now);
    }

    /// A handle let go: the roads the selection holds follow it, the ground under roads is
    /// made again, and scaled objects are handed to the app for their files.
    fn finish_gizmo(&mut self, assets: &mut Assets, grab: crate::gizmo::Grab) {
        let sels = self.selection();
        if let Some(doc) = self.doc.as_mut() {
            selection::unlink_outside(doc, &mut self.history, &sels);
        }
        let arms = std::mem::take(&mut self.move_arms);
        match grab.gizmo.mode {
            crate::gizmo::Mode::Move => self.carry(&arms, DVec2::ZERO, 0.0, self.move_delta, 0.0),
            crate::gizmo::Mode::Rotate if matches!(grab.part, crate::gizmo::Part::Ring(2)) => self.carry(&arms, grab.gizmo.pivot.truncate(), self.xform_value, DVec2::ZERO, 0.0),
            _ => {}
        }
        self.ground_to_selection();
        self.rebuild_pending(assets);
        self.scale_requests = std::mem::take(&mut self.scale_preview).into_iter().filter(|(_, f)| (*f - DVec3::ONE).abs().max_element() > 1e-4).collect();
    }

    /// Give the objects a scale dragged out to scaled copies of their types (the step the drag
    /// opened is closed here).
    pub fn apply_scales(&mut self, roots: &crate::install::Roots) {
        if self.scale_requests.is_empty() {
            return;
        }
        let reqs = std::mem::take(&mut self.scale_requests);
        let mut failed = None;
        for (sel, f) in reqs {
            let Sel::Object(_, id) = sel else { continue };
            let Some(doc) = self.doc.as_mut() else { break };
            let Some((k, o)) = selection::object_by_id(doc, id) else { continue };
            // (openMapper's own objects take the size into their meshes, a copy of their own)
            let made = if crate::editable::is_editable(roots, &o.file) { crate::editable::scaled_copy(roots, &o.file, f) } else { crate::scaled::variant(roots, &o.file, f) };
            match made {
                Ok(rel) => {
                    self.history.touch_tile(doc, k);
                    // (the copy has the same height and helpers as what it was made from)
                    if let Some(h) = self.known_helpers.get(&o.file.to_lowercase()).cloned() {
                        self.known_helpers.insert(rel.to_lowercase(), h);
                    }
                    if let Some(ob) = doc.object_mut(k, id) {
                        ob.file = rel;
                    }
                    if let Some(t) = doc.tiles.get_mut(&k) {
                        t.touch_content();
                    }
                }
                Err(e) => failed = Some(e.to_string()),
            }
        }
        self.history.commit();
        if let Some(e) = failed {
            self.error(format!("{}: {e}", tr("Could not scale", "Не удалось масштабировать")));
        }
    }

    /// The Move / Rotate / Scale handles drawn on the selection.
    fn gizmo_extras(&self, x: &mut FrameExtras) {
        let (Some(gz), Some(view)) = (self.gizmo(), self.gizmo_view.as_ref()) else { return };
        let (gz, active) = match self.drag {
            // (while dragging, the handles stay where they were taken)
            Drag::Gizmo { grab, started: true } => (crate::gizmo::Gizmo { pivot: grab.gizmo.pivot + self.drag_offset(&grab), ..grab.gizmo }, Some(grab.part)),
            Drag::Gizmo { grab, .. } => (gz, Some(grab.part)),
            Drag::MeshGizmo { grab, started: true } => (crate::gizmo::Gizmo { pivot: grab.gizmo.pivot, ..gz }, Some(grab.part)),
            Drag::MeshGizmo { grab, .. } => (gz, Some(grab.part)),
            _ => (gz, None),
        };
        let hot = if active.is_none() { gz.hit(view) } else { None };
        gz.draw(view, hot, active, &mut x.overlay);
    }

    /// How far the handles have gone with a move being dragged.
    fn drag_offset(&self, grab: &crate::gizmo::Grab) -> DVec3 {
        match (grab.gizmo.mode, &self.gizmo_view) {
            (crate::gizmo::Mode::Move, Some(view)) => match grab.change(view, None) {
                crate::gizmo::Change::Move(d) => d,
                _ => DVec3::ZERO,
            },
            _ => DVec3::ZERO,
        }
    }

    /// The left button down with the Select tool.
    fn select_press(&mut self, shift: bool) {
        // (whatever the properties had open is done: the view's edit is a step of its own)
        if self.panel_step {
            self.panel_step = false;
            self.history.commit();
        }
        if let (Some(gz), Some(view)) = (self.gizmo(), self.gizmo_view.clone()) {
            if let Some(part) = gz.hit(&view) {
                if self.mesh.as_ref().map(|m| m.editing).unwrap_or(false) {
                    self.drag = Drag::MeshGizmo { grab: crate::gizmo::Grab::new(gz, part, &view), started: false };
                } else {
                    self.start_gizmo(gz, part, &view);
                }
                return;
            }
        }
        if let (Some(m), Some(view)) = (self.mesh.as_mut().filter(|m| m.editing), self.gizmo_view.clone()) {
            // (a press in the edit mode: an operation going on is done, or vertices, edges or
            // faces are picked; a second click soon picks the loop)
            if m.modal_active() {
                m.confirm();
                return;
            }
            let hit = m.pick(&view);
            let double = hit.is_some() && self.last_mesh_click.map(|(h, t)| Some(h) == hit && t.elapsed().as_millis() < 450).unwrap_or(false);
            m.click(hit, shift, double);
            self.last_mesh_click = if double { None } else { hit.map(|h| (h, Instant::now())) };
            return;
        }
        if let (Some(h), Some((k, id))) = (self.hover_handle, self.hover_handle_of) {
            // (on an AI path the grip pressed is the point picked: Del takes it out)
            self.path_point = self.editing_paths().then_some(((k, id), h));
            self.drag = Drag::Reshape(k, id, h);
            self.reshape = None;
            return;
        }
        let Some(hit) = self.hover else {
            if !shift {
                self.select(None);
            }
            return;
        };
        if shift {
            self.toggle_selected(hit);
            return;
        }
        // Ctrl + click on the AI path being edited: a new point there, to drag
        if let (true, true, Sel::Spline(k, id), Some(c)) = (self.editing_paths(), self.ctrl_held, hit, self.cursor) {
            self.split_path_at(k, id, c.truncate());
            return;
        }
        // a double click on a road: the whole road, all its pieces up to where it branches or ends
        let double = self.last_click.map(|(s, t)| s == hit && t.elapsed().as_millis() < 450).unwrap_or(false);
        self.last_click = Some((hit, Instant::now()));
        if let (true, Sel::Spline(k, id)) = (double, hit) {
            self.last_click = None;
            // (an AI path: a double click puts a point in it, to drag; a road: the whole road)
            if let (true, Some(c)) = (self.editing_paths(), self.cursor) {
                self.split_path_at(k, id, c.truncate());
            } else {
                self.select_chain(k, id);
            }
            return;
        }
        let was_selected = self.is_selected(hit);
        if !self.is_selected(hit) {
            // (a group's thing brings the whole group; Ctrl + click takes it alone)
            let v = if self.ctrl_held { vec![hit] } else { self.with_groups(vec![hit]) };
            self.set_selection(v);
        }
        // an AI path is picked as a whole, each point of it a grip of its own
        if let (true, false, Sel::Spline(k, id)) = (self.editing_paths(), was_selected, hit) {
            self.select_path_pieces(k, id);
        }
        // a press on the selection may become a move of all of it (not in OMSI 2's and
        // Blender's key sets: there the move is G, and a click only picks)
        if let (true, Some(c), Some(doc)) = (self.press_drags, self.cursor, self.doc.as_ref()) {
            let mut sels = self.selection();
            // (the one grabbed leads: the grid holds its place)
            if let Some(i) = sels.iter().position(|s| *s == hit) {
                sels.swap(0, i);
            }
            self.move_from = sels.iter().filter_map(|s| selection::origin_of(doc, *s)).collect();
            self.drag = Drag::Move { grab: c.truncate(), started: false };
        }
    }

    /// The spline and every one linked on from it either way: the road it is a piece of.
    /// An AI path's pieces round `id`: up to 8 each way along it (a long route through a whole
    /// town is not all picked at once, each piece with grips of its own).
    pub fn select_path_pieces(&mut self, k: TileKey, id: i64) {
        let Some(doc) = self.doc.as_ref() else { return };
        let mut v = vec![Sel::Spline(k, id)];
        let Some((_, first)) = doc.find_spline(id) else { return };
        for dir in [false, true] {
            let mut cur = first.clone();
            for _ in 0..8 {
                let n = if dir { cur.next_id } else { cur.prev_id };
                let Some((nk, next)) = (n != 0).then(|| doc.find_spline(n)).flatten() else { break };
                if v.contains(&Sel::Spline(nk, n)) {
                    break;
                }
                v.push(Sel::Spline(nk, n));
                cur = next.clone();
            }
        }
        self.set_selection(v);
    }

    pub fn select_chain(&mut self, k: TileKey, id: i64) {
        let Some(doc) = self.doc.as_ref() else { return };
        let mut v = vec![Sel::Spline(k, id)];
        let mut seen = std::collections::HashSet::from([id]);
        let mut todo = vec![id];
        while let Some(cur) = todo.pop() {
            let Some((_, s)) = doc.find_spline(cur) else { continue };
            for n in [s.prev_id, s.next_id] {
                if n != 0 && seen.len() < 5000 && seen.insert(n) {
                    if let Some((nk, _)) = doc.find_spline(n) {
                        v.push(Sel::Spline(nk, n));
                        todo.push(n);
                    }
                }
            }
        }
        self.set_selection(v);
    }

    /// A point put into an AI path where it is nearest to `p`: the spline is cut there, so
    /// the joint becomes a grip that drags both pieces.
    fn split_path_at(&mut self, k: TileKey, id: i64, p: DVec2) {
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(s) = doc.spline(k, id) else { return };
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
        let n = (s.length / 0.5).ceil().max(2.0) as usize;
        let at = (0..=n).map(|i| s.length * i as f64 / n as f64).min_by(|a, b| (c.point_at(*a).truncate() - p).length().total_cmp(&(c.point_at(*b).truncate() - p).length())).unwrap_or(s.length * 0.5);
        // (a point right at an end would leave a piece too short to grab)
        if at < 1.0 || at > s.length - 1.0 {
            self.toast(tr("Too near the end: drag the end's dot instead", "Слишком близко к концу: тяните точку конца"));
            return;
        }
        let next = doc.find_spline(s.next_id).map(|(nk, _)| nk);
        self.history.begin(tr("Add a point to the path", "Точка в пути"));
        let doc = self.doc.as_ref().unwrap();
        self.history.touch_global(doc);
        self.history.touch_tile(doc, k);
        // (the path after it is linked to the new piece instead)
        if let Some(nk) = next {
            self.history.touch_tile(doc, nk);
        }
        self.doc.as_mut().unwrap().split_spline(k, id, at);
        self.history.commit();
        self.select_path_pieces(k, id);
        self.toast(tr("Point added: drag the dot where the pieces meet", "Точка добавлена: тяните кружок на стыке"));
    }

    /// "Connect to" is waiting for the second point.
    pub fn path_connect_on(&self) -> bool {
        self.path_connect.is_some()
    }

    /// Esc: only the connect is called off.
    pub fn take_path_connect(&mut self) -> bool {
        self.path_connect.take().is_some()
    }

    /// The picked path point is one end. The next click on another end joins the two paths
    /// (a gap between them gets a piece of the same type). Clicking the button again calls it off.
    pub fn arm_path_connect(&mut self) {
        if self.path_connect.take().is_some() {
            return;
        }
        let Some((sp, h)) = self.path_point else {
            self.toast(tr("Pick an end point of a path first", "Сначала выберите конечную точку пути"));
            return;
        };
        if h == Handle::Mid || !self.path_end_free(sp, h) {
            self.toast(if h == Handle::Mid {
                tr("Pick an end point of a path first", "Сначала выберите конечную точку пути")
            } else {
                tr("That point is already joined", "Эта точка уже соединена")
            });
            return;
        }
        self.path_connect = Some((sp, h));
    }

    /// Where a path's end is, and the heading of travel there (degrees, the way the piece runs).
    fn path_end_at(&self, sp: (TileKey, i64), h: Handle) -> Option<(DVec3, f64)> {
        let doc = self.doc.as_ref()?;
        let s = doc.spline(sp.0, sp.1)?;
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(sp.0));
        Some(match h {
            Handle::Start => (c.point_at(0.0), c.heading_at(0.0)),
            Handle::End => (c.end_point(), c.heading_at(s.length)),
            Handle::Mid => return None,
        })
    }

    fn path_end_free(&self, sp: (TileKey, i64), h: Handle) -> bool {
        let Some(s) = self.doc.as_ref().and_then(|d| d.spline(sp.0, sp.1)) else { return false };
        match h {
            Handle::Start => s.prev_id == 0,
            Handle::End => s.next_id == 0,
            Handle::Mid => false,
        }
    }

    /// Join two free ends of AI paths so one runs on into the other. A gap gets a piece of the
    /// first path's type. An end that faces the wrong way is turned round first, the whole
    /// path it belongs to with it, so traffic keeps one direction.
    fn connect_path_points(&mut self, a: ((TileKey, i64), Handle), b: ((TileKey, i64), Handle)) {
        if a.1 == Handle::Mid || b.1 == Handle::Mid {
            self.toast(tr("Click an end point of a path", "Щёлкните конечную точку пути"));
            return;
        }
        if a.0 == b.0 && a.1 == b.1 {
            self.toast(tr("Click the other point", "Щёлкните другую точку"));
            return;
        }
        if !self.path_end_free(a.0, a.1) || !self.path_end_free(b.0, b.1) {
            self.toast(tr("That point is already joined", "Эта точка уже соединена"));
            return;
        }
        let Some((pa, ha)) = self.path_end_at(a.0, a.1) else { return };
        let Some((pb, hb)) = self.path_end_at(b.0, b.1) else { return };
        let (chain_a, same, chain_b) = {
            let Some(doc) = self.doc.as_ref() else { return };
            let chain_a = path_chain(doc, a.0.1);
            let same = chain_a.iter().any(|p| p.1 == b.0.1);
            let chain_b = if !same && b.1 == Handle::End { path_chain(doc, b.0.1) } else { Vec::new() };
            (chain_a, same, chain_b)
        };
        // (the piece the new stretch leaves, and the one it arrives at)
        let (from, to, from_pos, to_pos, leave, arrive, turn) = if same {
            let (hk, hid) = chain_a[0];
            let (tk, tid) = *chain_a.last().unwrap();
            let (Some((fp, fl)), Some((tp, ta))) = (self.path_end_at((tk, tid), Handle::End), self.path_end_at((hk, hid), Handle::Start)) else { return };
            ((tk, tid), (hk, hid), fp, tp, fl, ta, Vec::new())
        } else {
            let leave = if a.1 == Handle::End { ha } else { roads::norm_deg(ha + 180.0) };
            let arrive = if b.1 == Handle::Start { hb } else { roads::norm_deg(hb + 180.0) };
            let mut turn = Vec::new();
            if a.1 == Handle::Start {
                turn.extend(chain_a);
            }
            if b.1 == Handle::End {
                turn.extend(chain_b);
            }
            (a.0, b.0, pa, pb, leave, arrive, turn)
        };
        let Some(doc) = self.doc.as_ref() else { return };
        let file = doc.spline(from.0, from.1).map(|s| s.file.clone()).unwrap_or_default();
        let mirror = doc.spline(from.0, from.1).map(|s| s.mirror).unwrap_or(false);
        let gap = (from_pos.truncate() - to_pos.truncate()).length();
        let segs = if gap <= 1.5 {
            Vec::new()
        } else {
            match reshape::fit(reshape::End { pos: from_pos.truncate(), heading: Some(leave) }, reshape::End { pos: to_pos.truncate(), heading: Some(arrive) })
                .or_else(|| reshape::fit(reshape::End { pos: from_pos.truncate(), heading: None }, reshape::End { pos: to_pos.truncate(), heading: None }))
            {
                Some(s) => s,
                None => {
                    self.error(tr("That bend is too sharp", "Слишком крутой поворот"));
                    return;
                }
            }
        };
        for p in &segs {
            if !doc.tiles.contains_key(&tile_of(p.start)) {
                self.toast(tr("There is no tile there", "Там нет клетки"));
                return;
            }
        }
        if turn.iter().any(|(k, id)| chain_has_rows(doc, &[(*k, *id)])) {
            self.error(tr("Remove the rows of objects along it first (they are measured from its start)", "Сначала уберите ряды объектов вдоль неё (они отмеряются от её начала)"));
            return;
        }
        self.history.begin(tr("Connect the path", "Соединить путь"));
        let doc = self.doc.as_ref().unwrap();
        self.history.touch_global(doc);
        let mut tiles: Vec<TileKey> = turn.iter().map(|p| p.0).chain([from.0, to.0]).chain(segs.iter().map(|p| tile_of(p.start))).collect();
        tiles.sort();
        tiles.dedup();
        for k in &tiles {
            self.history.touch_tile(doc, *k);
        }
        let doc = self.doc.as_mut().unwrap();
        if !turn.is_empty() {
            reverse_path(doc, &turn);
        }
        let ids: Vec<i64> = segs.iter().map(|_| doc.new_id()).collect();
        if !segs.is_empty() {
            let profile = even_profile(&segs, from_pos.z, to_pos.z);
            for (i, p) in segs.iter().enumerate() {
                let k = tile_of(p.start);
                let mut sp = piece_spline(&file, p, &profile, i, tile_origin(k));
                sp.id = ids[i];
                sp.prev_id = if i == 0 { from.1 } else { ids[i - 1] };
                sp.next_id = if i + 1 == ids.len() { to.1 } else { ids[i + 1] };
                sp.mirror = mirror;
                let t = doc.tiles.get_mut(&k).unwrap();
                t.tile.splines.push(sp);
                t.touch_content();
            }
        }
        let (next, prev) = if ids.is_empty() { (to.1, from.1) } else { (ids[0], *ids.last().unwrap()) };
        if from.1 == to.1 {
            set_path_link(doc, from.0, from.1, Some(next), Some(prev));
        } else {
            set_path_link(doc, from.0, from.1, Some(next), None);
            set_path_link(doc, to.0, to.1, None, Some(prev));
        }
        self.history.commit();
        self.path_point = None;
        self.select_path_pieces(from.0, from.1);
        self.toast(tr("Path connected", "Путь соединён"));
    }

    /// The button let go after dragging a grip: the new shape goes in.
    fn finish_reshape(&mut self, k: TileKey, id: i64) {
        match self.reshape.take() {
            Some(Ok(plan)) => {
                self.history.begin(tr("Reshape road", "Изменение формы дороги"));
                self.apply_plan(&plan);
                if let Some((jid, centre, r)) = self.reshape_plate.take() {
                    let doc = self.doc.as_mut().unwrap();
                    if let Some(end) = selection::free_end_at(doc, centre.truncate()) {
                        if let Some(arm) = doc.trim(end.0, end.1, end.2, r) {
                            self.pending_arms.entry(jid).or_default().push(crate::junction::Arm { key: arm.0, id: arm.1, at_end: arm.2 });
                            self.pending_rebuild.push(jid);
                        }
                    }
                }
                self.finish_step();
                if self.editing_paths() {
                    self.select_path_pieces(k, id);
                } else {
                    self.select(Some(Sel::Spline(k, id)));
                }
            }
            Some(Err(e)) => self.error(e),
            None => {}
        }
    }

    /// The box dragged on the screen is let go: what is inside it is selected (added to the
    /// selection with `add`). `view`: the 3D view's corner and size in window points.
    pub fn finish_box(&mut self, assets: &mut Assets, cam: &crate::camera::Camera, view: (f32, f32, f32, f32), add: bool) {
        let Some([a, b]) = self.select_box.take() else { return };
        if let (Some(m), Some(gv)) = (self.mesh.as_mut().filter(|m| m.editing), self.gizmo_view.as_ref()) {
            let (vx, vy, _, _) = view;
            if (a.0 - b.0).abs() < 4.0 && (a.1 - b.1).abs() < 4.0 {
                if !add {
                    m.click(None, false, false);
                }
                return;
            }
            m.box_select(gv, DVec2::new((a.0 - vx) as f64, (a.1 - vy) as f64), DVec2::new((b.0 - vx) as f64, (b.1 - vy) as f64), add);
            return;
        }
        let Some(doc) = self.doc.as_ref() else { return };
        if (a.0 - b.0).abs() < 4.0 && (a.1 - b.1).abs() < 4.0 {
            return;
        }
        let (vx, vy, w, h) = view;
        let extras = FrameExtras { show_technical: self.show_technical || self.tool == Tool::Logic, hide: self.hide, ..Default::default() };
        let found = selection::in_box(doc, cam, (a.0 - vx, a.1 - vy), (b.0 - vx, b.1 - vy), w, h, |f| assets.object(f).map(|m| m.technical && !extras.show_technical || crate::render::hidden_layer(&m, &extras)).unwrap_or(false), !self.hide[2]);
        let mut v = if add { self.selection() } else { Vec::new() };
        for s in found {
            if !v.contains(&s) {
                v.push(s);
            }
        }
        let v = self.with_groups(v);
        let n = v.len();
        self.set_selection(v);
        if n > 0 {
            self.toast(format!("{} {n}", tr("Selected:", "Выбрано:")));
        }
    }

    /// Everything on the map (Ctrl+A).
    pub fn select_all(&mut self) {
        let Some(doc) = self.doc.as_ref() else { return };
        let v: Vec<Sel> = doc
            .tiles
            .iter()
            .flat_map(|(k, t)| {
                t.tile
                    .objects
                    .iter()
                    .map(move |o| Sel::Object(*k, o.id))
                    .chain(t.tile.spline_attachments.iter().map(move |a| Sel::Attached(*k, a.id)))
                    .chain(t.tile.splines.iter().filter(|s| !s.deleted).map(move |s| Sel::Spline(*k, s.id)))
            })
            .collect();
        self.set_selection(v);
    }

    // ------------------------------------------------------------ clipboard

    /// Copy the selection; returns how many things were copied.
    pub fn copy(&mut self, lib: &Library) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let clip = crate::clipboard::copy(doc, &self.selection(), |f| lib.find(f).map(|a| a.abs_height).unwrap_or(false));
        let n = clip.len();
        if n > 0 {
            self.clip = Some(clip);
            self.toast(format!("{} {n}", tr("Copied:", "Скопировано:")));
        }
        n
    }

    /// Something was copied (to paste).
    pub fn has_clip(&self) -> bool {
        self.clip.is_some()
    }

    pub fn cut(&mut self, lib: &Library) {
        if self.copy(lib) > 0 {
            self.delete_selected();
        }
    }

    /// Paste what was copied round the cursor (without one, a little beside where it was).
    pub fn paste(&mut self, lib: &Library) {
        let Some(clip) = self.clip.clone() else {
            self.toast(tr("Nothing copied yet", "Ещё ничего не скопировано"));
            return;
        };
        let at = match self.cursor {
            Some(c) => self.snap_point(c.truncate()),
            None => clip.at.truncate() + DVec2::splat(5.0),
        };
        self.paste_clip(&clip, at, tr("Paste", "Вставка"), lib);
    }

    /// Copy the selection and put the copy down beside it (the clipboard stays as it was).
    pub fn duplicate(&mut self, lib: &Library) {
        let Some(doc) = self.doc.as_ref() else { return };
        let clip = crate::clipboard::copy(doc, &self.selection(), |f| lib.find(f).map(|a| a.abs_height).unwrap_or(false));
        if clip.is_empty() {
            return;
        }
        let step = if self.snap_grid { self.grid_step.max(1.0) * (4.0 / self.grid_step.max(1.0)).ceil() } else { 4.0 };
        let at = clip.at.truncate() + DVec2::splat(step);
        self.paste_clip(&clip, at, tr("Duplicate", "Дублирование"), lib);
    }

    /// The ground under each selected object made level with its foot (where it stands at its
    /// middle), its outline and 3 m round it blending into the ground about. How many.
    pub fn level_under_objects(&mut self, assets: &mut Assets) -> usize {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return 0 };
        let mut done = 0;
        self.history.begin(tr("Level the ground", "Выравнивание земли"));
        for s in &sels {
            let Sel::Object(k, id) = *s else { continue };
            let Some(o) = doc.object(k, id).cloned() else { continue };
            let Some(model) = assets.object(&o.file) else { continue };
            if model.tree.is_some() {
                continue;
            }
            let centre = tile_origin(k) + DVec2::new(o.pos[0], o.pos[1]);
            let level = if model.abs_height { o.pos[2] } else { doc.ground(centre) as f64 };
            level_ground(doc, &mut self.history, centre, o.rot[0], (model.lo, model.hi), level);
            done += 1;
        }
        self.history.commit();
        done
    }

    /// The selected objects turned the way the nearest road runs where it passes them (within
    /// 60 m; the heading a row along that road would give them). How many were turned.
    pub fn align_to_roads(&mut self) -> usize {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return 0 };
        let mut turns: Vec<(TileKey, i64, f64)> = Vec::new();
        for s in &sels {
            let Sel::Object(k, id) = *s else { continue };
            let Some(o) = doc.object(k, id) else { continue };
            let p = tile_origin(k) + DVec2::new(o.pos[0], o.pos[1]);
            let mut best: Option<(f64, f64)> = None;
            for (sk, t) in &doc.tiles {
                let org = tile_origin(*sk);
                for sp in t.tile.splines.iter().filter(|x| !x.deleted) {
                    let start = org + DVec2::new(sp.pos[0], sp.pos[1]);
                    if (start - p).length() > sp.length + 60.0 {
                        continue;
                    }
                    let c = omsi_geometry::SplineCurve::from_map(sp, org);
                    let n = (sp.length.ceil() as usize).clamp(2, 2000);
                    for i in 0..=n {
                        let at = sp.length * i as f64 / n as f64;
                        let d = (c.point_at(at).truncate() - p).length();
                        if d < 60.0 && best.map(|b| d < b.0).unwrap_or(true) {
                            best = Some((d, c.heading_at(at)));
                        }
                    }
                }
            }
            if let Some((_, h)) = best {
                turns.push((k, id, roads::norm_deg(h)));
            }
        }
        if turns.is_empty() {
            return 0;
        }
        self.history.begin(tr("Turn objects", "Поворот объектов"));
        for (k, id, h) in &turns {
            self.history.touch_tile(doc, *k);
            if let Some(o) = doc.object_mut(*k, *id) {
                o.rot[0] = *h;
            }
            doc.tiles.get_mut(k).map(|t| t.touch_content());
        }
        self.history.commit();
        turns.len()
    }

    /// `n` copies of the selection in a row, each `step` (m) on from the one before and its
    /// objects turned `turn` degrees more (a fence, a row of houses, an avenue), in one step.
    /// All of them are selected.
    /// The selection copied `n` times a `step` on (each turned `turn` more), in `rows`
    /// rows a `row_step` apart (a grid: an orchard, a car park), one undo step.
    pub fn array(&mut self, lib: &Library, n: usize, step: DVec2, turn: f64, rows: usize, row_step: DVec2) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        let clip = crate::clipboard::copy(doc, &sels, |f| lib.find(f).map(|a| a.abs_height).unwrap_or(false));
        let rows = rows.max(1);
        if clip.is_empty() || (n == 0 && rows < 2) || (step.length() < 0.01 && turn.abs() < 1e-6 && rows < 2) {
            return;
        }
        self.history.begin(tr("Copies in a row", "Копии в ряд"));
        self.history.touch_global(doc);
        let mut all = sels.clone();
        let mut lost = 0;
        // (the original is the first of the first row)
        let places: Vec<(usize, DVec2)> = (0..rows).flat_map(|r| (0..=n).map(move |i| (i, row_step * r as f64 + step * i as f64))).filter(|(i, _)| *i > 0 || rows > 1).filter(|p| p.1.length() > 1e-9 || p.0 > 0).take(2000).collect();
        for (i, off) in places {
            let at2 = clip.at.truncate() + off;
            for k in crate::clipboard::tiles_touched(doc, &clip, at2) {
                self.history.touch_tile(doc, k);
            }
            let at = DVec3::new(at2.x, at2.y, doc.ground(at2));
            let got = crate::clipboard::paste(doc, &clip, at);
            lost += clip.len() - got.len();
            for s in &got {
                if let Sel::Object(k, id) = *s {
                    if let Some(o) = doc.object_mut(k, id) {
                        o.rot[0] = crate::roads::norm_deg(o.rot[0] + turn * i as f64);
                    }
                }
            }
            all.extend(got);
        }
        self.history.commit();
        if lost > 0 {
            self.toast(format!("{} {lost}", tr("Left out (off the map):", "Пропущено (за картой):")));
        }
        self.set_selection(all);
    }

    fn paste_clip(&mut self, clip: &crate::clipboard::Clip, at: DVec2, label: &str, lib: &Library) {
        let Some(doc) = self.doc.as_mut() else { return };
        let tiles = crate::clipboard::tiles_touched(doc, clip, at);
        if tiles.is_empty() {
            self.error(tr("There is no tile here: add one with the Tiles tool", "Здесь нет тайла: добавьте его инструментом «Тайлы»"));
            return;
        }
        self.history.begin(label);
        for k in tiles {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        let at = DVec3::new(at.x, at.y, doc.ground(at));
        let got = crate::clipboard::paste(doc, clip, at);
        // the ground comes to the roads pasted (the catalogue knows their widths)
        let roads: Vec<(TileKey, i64)> = got.iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((*k, *id)) } else { None }).collect();
        self.ground_to(&roads, &|f| lib.find(f).filter(|a| matches!(a.category, crate::library::Category::Roads | crate::library::Category::Tracks)).map(|a| a.half_width as f64).unwrap_or(0.0));
        self.history.commit();
        let n = got.len();
        if n < clip.len() {
            self.toast(format!("{} {n} / {}", tr("Pasted (the rest is off the map):", "Вставлено (остальное за картой):"), clip.len()));
        }
        self.set_selection(got);
    }

    // ------------------------------------------------------------ numbers

    /// A number in the properties changes: one step for as long as it is dragged or typed.
    pub fn begin_panel_step(&mut self, label: &str) {
        if !(self.panel_step && self.history.is_open()) {
            self.history.begin(label);
            self.panel_step = true;
        }
    }

    /// The properties' step ends once no number is being dragged or typed (a step the view
    /// opened, such as a move, is left alone).
    pub fn end_panel_step_if_idle(&mut self, ctx: &egui::Context) {
        if self.panel_step && self.drag == Drag::None && !ctx.egui_is_using_pointer() && !ctx.egui_wants_keyboard_input() {
            self.panel_step = false;
            self.history.commit();
        }
    }


    // ------------------------------------------------------------ the grab (G)

    /// Take hold of the selection, as Blender's G does: it follows the pointer from where it
    /// is until [`grab_confirm`](Self::grab_confirm) (Enter or a click) or
    /// [`grab_cancel`](Self::grab_cancel) (Esc or a right click). False: there is nothing
    /// selected to take hold of, or the pointer is not over the map.
    pub fn grab_begin(&mut self, mouse: (f32, f32)) -> bool {
        let Some(at) = self.cursor else { return false };
        let sels = self.selection();
        if sels.is_empty() {
            return false;
        }
        let Some(doc) = self.doc.as_ref() else { return false };
        let from: Vec<Origin> = sels.iter().filter_map(|s| selection::origin_of(doc, *s)).collect();
        if from.is_empty() {
            return false;
        }
        // (the heights: the Z lock puts them back where they were and lifts them from there)
        let z: Vec<(Sel, f64)> = sels
            .iter()
            .filter_map(|s| match *s {
                Sel::Object(k, id) => doc.object(k, id).map(|o| (*s, o.pos[2])),
                Sel::Spline(k, id) => doc.spline(k, id).map(|sp| (*s, sp.pos[2])),
                Sel::Attached(k, id) => doc.attachment(k, id).map(|a| (*s, a.offset[1])),
            })
            .collect();
        // (a chain of thousands: its links read now, not on every frame of the drag)
        let links = (from.len() > 16).then(|| selection::SplineWhere::of(doc));
        let arms = self.arms_for(&sels);
        // (a move of a great many is worth a line: what a big map answers slowly with)
        if from.len() > 100 {
            log::info!("grab of {} things", from.len());
        }
        if self.grab.is_some() {
            self.grab_cancel();
        }
        self.history.begin(tr("Move", "Перемещение"));
        self.grab = Some(Grab { from, z, arms, start: at, start_mouse: mouse, at, mouse, axis: None, typed: String::new(), delta: DVec3::ZERO, asked: DVec3::ZERO, links });
        self.xform_value = 0.0;
        true
    }

    /// The pointer has gone to `at` (world) / `mouse` (window points): the selection follows
    /// it, as the axis locked allows.
    pub fn grab_follow(&mut self, at: DVec3, mouse: (f32, f32)) {
        let Some(g) = self.grab.as_mut() else { return };
        g.at = at;
        g.mouse = mouse;
        let mpp = self.gizmo_view.as_ref().map(|v| v.metres_per_point(g.start)).unwrap_or(1.0);
        let mut d = DVec3::new(at.x - g.start.x, at.y - g.start.y, 0.0);
        match g.axis {
            // (Blender's Z: the pointer up the screen lifts it)
            Some(2) => {
                d.x = 0.0;
                d.y = 0.0;
                d.z = -(mouse.1 - g.start_mouse.1) as f64 * mpp;
            }
            Some(0) => d.y = 0.0,
            Some(1) => d.x = 0.0,
            _ => {}
        }
        // a distance typed in says how far it goes, the pointer no more
        if let Ok(v) = g.typed.parse::<f64>() {
            d = match g.axis {
                Some(2) => DVec3::new(0.0, 0.0, v),
                Some(0) => DVec3::new(v, 0.0, 0.0),
                Some(1) => DVec3::new(0.0, v, 0.0),
                // (no axis locked: the way the pointer has gone)
                _ => (d.truncate().normalize_or(DVec2::X) * v).extend(0.0),
            };
        }
        // (nothing asked for that is not already done: the selection is not moved again, which
        // on a map of many tiles and a road of many pieces is the frame's whole cost)
        if (d - g.asked).length() < 1e-12 {
            return;
        }
        g.asked = d;
        self.grab_to(d);
    }

    /// Lock the grab to an axis (0: X, 1: Y, 2: Z): pressing that axis again lets it go free.
    pub fn grab_axis(&mut self, axis: u8) {
        let Some(g) = self.grab.as_mut() else { return };
        g.axis = if g.axis == Some(axis) { None } else { Some(axis) };
        g.typed.clear();
        let (at, mouse) = (g.at, g.mouse);
        // (it goes there at once: the pointer itself may not have moved)
        self.grab_follow(at, mouse);
    }

    /// A character typed while grabbing: a distance (m) it goes along the axis locked.
    pub fn grab_type(&mut self, ch: char) {
        let Some(g) = self.grab.as_mut() else { return };
        match ch {
            '0'..='9' => g.typed.push(ch),
            '.' | ',' => {
                if !g.typed.contains('.') {
                    g.typed.push('.');
                }
            }
            '-' => {
                if g.typed.starts_with('-') {
                    g.typed.remove(0);
                } else {
                    g.typed.insert(0, '-');
                }
            }
            _ => return,
        }
        let (at, mouse) = (g.at, g.mouse);
        self.grab_follow(at, mouse);
    }

    /// A digit rubbed out of the distance typed.
    pub fn grab_backspace(&mut self) {
        let Some(g) = self.grab.as_mut() else { return };
        g.typed.pop();
        let (at, mouse) = (g.at, g.mouse);
        self.grab_follow(at, mouse);
    }

    /// Let the selection go where it is: the move stands.
    pub fn grab_confirm(&mut self) -> bool {
        if self.grab.take().is_none() {
            return false;
        }
        self.carry_preview.clear();
        self.finish_step();
        true
    }

    /// Call the grab off: everything back where it was taken.
    pub fn grab_cancel(&mut self) -> bool {
        let Some(g) = self.grab.take() else { return false };
        self.carry_preview.clear();
        if let Some(doc) = self.doc.as_mut() {
            self.history.rollback(doc);
        }
        self.set_selection(g.from.iter().map(|o| o.sel).collect());
        self.xform_value = 0.0;
        true
    }

    /// Put the selection where `d` (m) says, from where it was taken.
    fn grab_to(&mut self, d: DVec3) {
        let Some(g) = self.grab.as_ref() else { return };
        // (nothing has moved yet: nothing is put anywhere, the grid's snapping included, so
        // taking hold of something and locking an axis leaves it where it stands)
        if d.length() < 1e-9 {
            self.xform_value = 0.0;
            return;
        }
        // (the grid holds the first one's place, as a drag does)
        let first = g.from[0].pos;
        let snapped = self.snap_point(first + d.truncate()) - first;
        let delta = DVec2::new(snapped.x, snapped.y);
        let dz = if self.snap_grid && self.grid_step > 0.0 { (d.z / self.grid_step).round() * self.grid_step } else { d.z };
        // (what it carries is moved out and back: a selection of a hundred thousand pieces
        // copied on every frame of a drag is a frame's work for nothing)
        let arms: Vec<selection::Arm> = g.arms.iter().flat_map(|(_, a)| a.iter().copied()).collect();
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(carried) = self.grab.as_mut() else { return };
        let from = std::mem::take(&mut carried.from);
        let z = std::mem::take(&mut carried.z);
        let n = from.len();
        let links = self.grab.as_ref().and_then(|g| g.links.as_ref());
        let t0 = std::time::Instant::now();
        let now = selection::transform_known(doc, &mut self.history, &from, DVec2::ZERO, 0.0, delta, links);
        let t_move = t0.elapsed();
        // (the roads the selected junctions hold follow, shown as they would be)
        self.carry_preview = selection::carry_plans(doc, &arms, DVec2::ZERO, 0.0, delta, dz, &|_| false).into_iter().flatten().collect();
        let t_preview = t0.elapsed() - t_move;
        self.set_selection_unique(now);
        if dz.abs() > 1e-9 {
            self.set_height_from(&z, dz);
        }
        if let Some(g) = self.grab.as_mut() {
            g.from = from;
            g.z = z;
            g.delta = delta.extend(dz);
        }
        self.xform_value = delta.extend(dz).length();
        let total = t0.elapsed();
        if total.as_millis() > 200 {
            log::warn!("the grab of {n} things took {:.0} ms (the move {:.0}, the preview {:.0}, the heights {:.0})", total.as_secs_f64() * 1000.0, t_move.as_secs_f64() * 1000.0, t_preview.as_secs_f64() * 1000.0, (total - t_move - t_preview).as_secs_f64() * 1000.0);
        }
    }

    /// Put the height of what is being grabbed at its own plus `dz` (the Z lock).
    fn set_height_from(&mut self, z: &[(Sel, f64)], dz: f64) {
        let Some(doc) = self.doc.as_mut() else { return };
        for (sel, z0) in z {
            match *sel {
                Sel::Object(k, id) => {
                    // (it may have gone over into another tile: it is found by its IDCode)
                    let k = if doc.object(k, id).is_some() { k } else { match doc.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == id)) { Some((k, _)) => *k, None => continue } };
                    self.history.touch_tile(doc, k);
                    if let Some(o) = doc.object_mut(k, id) {
                        o.pos[2] = z0 + dz;
                    }
                    doc.tiles.get_mut(&k).map(|t| t.touch_content());
                }
                Sel::Spline(k, id) => {
                    self.history.touch_tile(doc, k);
                    if let Some(sp) = doc.tiles.get_mut(&k).and_then(|t| t.tile.splines.iter_mut().find(|x| x.id == id)) {
                        sp.pos[2] = z0 + dz;
                    }
                    doc.tiles.get_mut(&k).map(|t| t.touch_content());
                }
                Sel::Attached(k, id) => {
                    self.history.touch_tile(doc, k);
                    if let Some(a) = doc.attachment_mut(k, id) {
                        a.offset[1] = z0 + dz;
                    }
                    doc.tiles.get_mut(&k).map(|t| t.touch_content());
                }
            }
        }
    }

    /// Move the selection by `delta` (m), as one step.
    pub fn move_selection_by(&mut self, delta: DVec2) {
        let sels = self.selection();
        let arms = self.arms_for(&sels);
        let Some(doc) = self.doc.as_mut() else { return };
        let from: Vec<Origin> = sels.iter().filter_map(|s| selection::origin_of(doc, *s)).collect();
        if from.is_empty() || delta.length() < 1e-6 {
            return;
        }
        self.history.begin(tr("Move", "Перемещение"));
        let now = selection::transform(doc, &mut self.history, &from, DVec2::ZERO, 0.0, delta);
        selection::unlink_outside(doc, &mut self.history, &now);
        self.set_selection_unique(now);
        self.carry(&arms, DVec2::ZERO, 0.0, delta, 0.0);
        self.ground_to_selection();
        self.finish_step();
    }

    /// Put the object selected at a world position (the numbers in the properties).
    pub fn put_object(&mut self, k: TileKey, id: i64, to: DVec2) {
        let Some(o) = self.doc.as_ref().and_then(|d| selection::origin_of(d, Sel::Object(k, id))) else { return };
        self.begin_panel_step(tr("Edit object", "Изменение объекта"));
        let arms = self.arms_for(&[Sel::Object(k, id)]);
        let Some(doc) = self.doc.as_mut() else { return };
        let now = selection::transform(doc, &mut self.history, &[o], DVec2::ZERO, 0.0, to - o.pos);
        if let Some(s) = now.first() {
            if self.selected == Some(Sel::Object(k, id)) {
                self.selected = Some(*s);
            }
        }
        self.carry(&arms, DVec2::ZERO, 0.0, to - o.pos, 0.0);
        // (a number dragged moves the plate as it is: it is not made again every frame)
        self.pending_rebuild.clear();
    }

    /// Raise (or lower) the selection by `dz` metres: objects above their ground, splines'
    /// heights.
    /// The selected objects' headings: each a random one (`random`), or that of the one shown
    /// in the properties.
    /// The selected objects tilted to the ground under them (a fence or a shed on a slope),
    /// or stood upright again (`lean` false); things hanging on others are left alone.
    pub fn lean_objects(&mut self, lean: bool) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(if lean { tr("Lean with the ground", "Наклон по земле") } else { tr("Stand upright", "Поставить ровно") });
        for s in &sels {
            let Sel::Object(k, id) = *s else { continue };
            let Some(o) = doc.object(k, id) else { continue };
            let at = tile_origin(k) + DVec2::new(o.pos[0], o.pos[1]);
            let rot = if lean { lean_rotation(o.rot[0], &|p| doc.ground(p), at) } else { [o.rot[0], 0.0, 0.0] };
            self.history.touch_tile(doc, k);
            if let Some(t) = doc.tiles.get_mut(&k) {
                if let Some(o) = t.tile.objects.iter_mut().find(|o| o.id == id) {
                    o.rot = rot;
                }
                t.touch_content();
            }
        }
        self.history.commit();
    }

    pub fn turn_objects(&mut self, random: bool) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        let first = match self.selected {
            Some(Sel::Object(k, id)) => selection::any_object(doc, k, id).map(|o| o.rot[0]),
            _ => None,
        };
        if !random && first.is_none() {
            return;
        }
        self.history.begin(tr("Turn objects", "Поворот объектов"));
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0) ^ 0x9e37_79b9_7f4a_7c15;
        for s in &sels {
            let Sel::Object(k, id) = *s else { continue };
            self.history.touch_tile(doc, k);
            if let Some(o) = selection::any_object_mut(doc, k, id) {
                o.rot[0] = if random {
                    // (a hash of the IDCode: every object its own way)
                    let mut x = (id as u64).wrapping_mul(0x2545_f491_4f6c_dd1d) ^ seed;
                    x ^= x >> 33;
                    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
                    x ^= x >> 33;
                    (x % 36000) as f64 / 100.0
                } else {
                    first.unwrap_or(0.0)
                };
            }
            doc.tiles.get_mut(&k).map(|t| t.touch_content());
        }
        self.history.commit();
    }

    pub fn raise_selection(&mut self, dz: f64) {
        let sels = self.selection();
        let arms = self.arms_for(&sels);
        let Some(doc) = self.doc.as_mut() else { return };
        if sels.is_empty() || dz.abs() < 1e-9 {
            return;
        }
        self.history.begin(tr("Change height", "Изменение высоты"));
        for s in &sels {
            match *s {
                Sel::Object(k, id) => {
                    self.history.touch_tile(doc, k);
                    if let Some(o) = doc.object_mut(k, id) {
                        o.pos[2] += dz;
                    }
                    doc.tiles.get_mut(&k).map(|t| t.touch_content());
                }
                Sel::Spline(k, id) => {
                    self.history.touch_tile(doc, k);
                    if let Some(sp) = doc.tiles.get_mut(&k).and_then(|t| t.tile.splines.iter_mut().find(|x| x.id == id)) {
                        sp.pos[2] += dz;
                    }
                    doc.tiles.get_mut(&k).map(|t| t.touch_content());
                }
                Sel::Attached(k, id) => {
                    self.history.touch_tile(doc, k);
                    if let Some(a) = doc.attachment_mut(k, id) {
                        a.offset[1] += dz;
                    }
                    doc.tiles.get_mut(&k).map(|t| t.touch_content());
                }
            }
        }
        // a road joined from outside the selection meets it again: its end there comes up too
        let ids: Vec<i64> = sels.iter().filter_map(|s| if let Sel::Spline(_, id) = s { Some(*id) } else { None }).collect();
        for s in &sels {
            let Sel::Spline(k, id) = *s else { continue };
            let Some(sp) = doc.spline(k, id).cloned() else { continue };
            let c = omsi_geometry::SplineCurve::from_map(&sp, tile_origin(k));
            for (at_end, link, z) in [(false, sp.prev_id, c.point_at(0.0).z), (true, sp.next_id, c.end_point().z)] {
                if link != 0 && !ids.contains(&link) {
                    selection::touch_spline(&mut self.history, doc, k, id);
                    reshape::set_end(doc, k, id, at_end, Some(z), None);
                }
            }
        }
        self.history.touch_global(doc);
        sync_entry_points(doc);
        self.carry(&arms, DVec2::ZERO, 0.0, DVec2::ZERO, dz);
        self.finish_step();
    }

    /// Give every selected spline another type.
    pub fn retype_splines(&mut self, file: &str) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Change road type", "Смена типа дороги"));
        for s in &sels {
            if let Sel::Spline(k, id) = *s {
                self.history.touch_tile(doc, k);
                if let Some(t) = doc.tiles.get_mut(&k) {
                    if let Some(sp) = t.tile.splines.iter_mut().find(|x| x.id == id) {
                        sp.file = file.to_string();
                    }
                    t.touch_content();
                }
            }
        }
        self.history.commit();
    }

    /// The selected splines, one chain, given one even slope from the height at one end to
    /// that at the other (a bridge, a ramp).
    pub fn even_selected_chain(&mut self) {
        let splines: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(chain) = reshape::chain_order(doc, &splines) else {
            self.error(tr("The selected roads are not one chain (they branch or leave gaps)", "Выбранные дороги не одна цепочка (ветвятся или с разрывами)"));
            return;
        };
        self.history.begin(tr("Even slope", "Ровный уклон"));
        for (k, _, _) in &chain {
            self.history.touch_tile(doc, *k);
        }
        reshape::even_chain(doc, &chain);
        self.history.commit();
    }

    /// The ground painted along the selected roads as `paint_along` says; how many texels.
    pub fn paint_along_selected(&mut self, lib: &Library) -> usize {
        let splines: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let a = self.paint_along;
        let Some(doc) = self.doc.as_mut() else { return 0 };
        crate::roadpaint::paint_along(doc, &mut self.history, &splines, |f| lib.find(f).map(|x| x.half_width as f64).unwrap_or(3.0), &a)
    }

    /// The selected roads laid on the ground again, the ground then shaped to them (with
    /// "Shape the ground" on).
    pub fn lay_selected_on_ground(&mut self) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_ref() else { return };
        if !sels.iter().any(|s| matches!(s, Sel::Spline(..))) {
            return;
        }
        self.history.begin(tr("Lay on the ground", "Положить на землю"));
        for s in &sels {
            if let Sel::Spline(k, id) = s {
                self.history.touch_tile(doc, *k);
                if let Some(sp) = doc.spline(*k, *id) {
                    for l in [sp.prev_id, sp.next_id] {
                        if let Some((nk, _)) = doc.find_spline(l) {
                            self.history.touch_tile(doc, nk);
                        }
                    }
                }
            }
        }
        let Some(doc) = self.doc.as_mut() else { return };
        crate::onground::lay(doc, &sels);
        self.ground_to_selection();
        self.history.commit();
    }

    /// The selected splines turned round (each starts where it ended), the traffic on them
    /// going the other way: a one-way street, a track's direction.
    pub fn reverse_selected(&mut self) {
        let splines: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let Some(doc) = self.doc.as_mut() else { return };
        if splines.is_empty() {
            return;
        }
        // (a row of objects is measured along its spline from the start: it would move)
        let with_rows = splines.iter().any(|(k, id)| {
            let Some(t) = doc.tiles.get(k) else { return false };
            let idx = t.tile.splines.iter().position(|s| s.id == *id && !s.deleted);
            idx.map(|i| t.tile.spline_attachments.iter().any(|a| a.spline_index == i as i32)).unwrap_or(false)
        });
        if with_rows {
            self.error(tr("Remove the rows of objects along it first (they are measured from its start)", "Сначала уберите ряды объектов вдоль неё (они отмеряются от её начала)"));
            return;
        }
        // the textures run on along a chain the new way
        let mut tex: std::collections::HashMap<i64, f64> = Default::default();
        if let Some(chain) = reshape::chain_order(doc, &splines) {
            let mut at = 0.0;
            for (k, id, _) in chain.iter().rev() {
                tex.insert(*id, at);
                at += doc.spline(*k, *id).map(|s| s.length).unwrap_or(0.0);
            }
        }
        self.history.begin(tr("Reverse direction", "Разворот направления"));
        for (k, id) in &splines {
            self.history.touch_tile(doc, *k);
            let org = tile_origin(*k);
            let t = doc.tiles.get_mut(k).unwrap();
            if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == *id && !s.deleted) {
                let at = tex.get(id).copied().unwrap_or(s.tex_offset);
                *s = reshape::reversed(s, org, at);
                t.touch_content();
            }
        }
        self.history.commit();
    }

    /// The knife on or off (K); on, the Select tool takes over and the ruler goes.
    pub fn toggle_cutting(&mut self) {
        let on = !self.cutting;
        self.set_tool(Tool::Select);
        self.ruler = None;
        self.cutting = on;
    }

    /// Where a click of the knife would cut: the road under the cursor, at the point nearest
    /// it (on whole grid steps along the road with grid snapping on).
    pub fn cut_preview(&self) -> Option<CutPreview> {
        if !self.cutting || !self.cursor_in_view {
            return None;
        }
        let (k, id, at) = self.road_at(self.cursor?.truncate())?;
        let s = self.doc.as_ref()?.spline(k, id)?;
        let at = if self.snap_grid && self.grid_step > 0.0 { ((at / self.grid_step).round() * self.grid_step).clamp(0.0, s.length) } else { at };
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
        let h = c.heading_at(at).to_radians();
        Some(CutPreview { key: k, id, at, length: s.length, pos: c.point_at(at), dir: DVec2::new(h.sin(), h.cos()), fits: at >= 1.0 && at <= s.length - 1.0 })
    }

    /// The road or track nearest to `p` (within a few metres of its middle): tile, id and the
    /// distance along it.
    pub fn road_at(&self, p: DVec2) -> Option<(TileKey, i64, f64)> {
        crate::switches::spline_at(self.doc.as_ref()?, p, 8.0)
    }

    /// The road at `p` cut in two there (to give a stretch of it another type, a bridge, a
    /// junction later); both pieces selected.
    pub fn cut_road_at(&mut self, p: DVec2) {
        let Some((k, id, at)) = self.road_at(p) else {
            self.error(tr("No road there to cut", "Здесь нет дороги, которую можно разрезать"));
            return;
        };
        self.cut_spline(k, id, at);
    }

    /// Spline `id` cut in two `at` m along it; both pieces selected.
    pub fn cut_spline(&mut self, k: TileKey, id: i64, at: f64) {
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(s) = doc.spline(k, id) else { return };
        if at < 1.0 || at > s.length - 1.0 {
            self.error(tr("Too near the road's end to cut it there", "Слишком близко к концу дороги, чтобы разрезать её здесь"));
            return;
        }
        self.history.begin(tr("Cut the road", "Разрезать дорогу"));
        self.history.touch_tile(doc, k);
        if let Some((nk, _)) = doc.find_spline(s.next_id) {
            self.history.touch_tile(doc, nk);
        }
        let Some(new) = self.doc.as_mut().unwrap().split_spline(k, id, at) else {
            self.history.cancel();
            return;
        };
        self.history.commit();
        self.set_selection(vec![Sel::Spline(k, id), Sel::Spline(k, new)]);
        self.toast(tr("Cut in two: each piece can now be changed on its own", "Разрезана надвое: каждый кусок теперь можно менять отдельно"));
    }

    /// Spline `id` and the one after it made one spline again, where they make one curve.
    pub fn join_with_next(&mut self, k: TileKey, id: i64) {
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(s) = doc.spline(k, id) else { return };
        self.history.begin(tr("Join the pieces", "Соединить куски"));
        self.history.touch_tile(doc, k);
        if let Some((nk, n)) = doc.find_spline(s.next_id) {
            self.history.touch_tile(doc, nk);
            if let Some((ck, _)) = doc.find_spline(n.next_id) {
                self.history.touch_tile(doc, ck);
            }
        }
        match self.doc.as_mut().unwrap().join_splines(k, id) {
            Ok(()) => {
                self.history.commit();
                self.set_selection(vec![Sel::Spline(k, id)]);
                self.toast(tr("Joined into one piece", "Соединены в один кусок"));
            }
            Err(why) => {
                self.history.cancel();
                self.error(why);
            }
        }
    }

    /// The selected splines copied alongside, `d` m to the right of the first's way
    /// (negative: left); the copies are selected.
    pub fn parallel_copy(&mut self, d: f64) {
        let splines: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let Some(doc) = self.doc.as_mut() else { return };
        if splines.is_empty() {
            return;
        }
        self.history.begin(tr("Parallel copy", "Параллельная копия"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        match crate::parallel::copy(doc, &splines, d) {
            Ok(made) => {
                self.history.commit();
                self.set_selection(made.into_iter().map(|(k, id)| Sel::Spline(k, id)).collect());
            }
            Err(e) => {
                self.history.cancel();
                self.error(e);
            }
        }
    }

    /// A spline of type `wire` laid on each selected one (a track's overhead wire: its height
    /// is in the wire's own profile), the pieces linked as the track's are; one undo step.
    /// How many.
    pub fn wire_along(&mut self, wire: &str) -> usize {
        let splines: Vec<(TileKey, i64)> = self.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
        let Some(doc) = self.doc.as_mut() else { return 0 };
        if splines.is_empty() {
            return 0;
        }
        self.history.begin(tr("Overhead wire", "Контактный провод"));
        self.history.touch_global(doc);
        let ids: std::collections::HashMap<i64, i64> = splines.iter().map(|(_, id)| (*id, doc.new_id())).collect();
        let link = |id: i64| ids.get(&id).copied().unwrap_or(0);
        for (k, id) in &splines {
            let Some(s) = doc.spline(*k, *id).cloned() else { continue };
            self.history.touch_tile(doc, *k);
            // (a wire cuts no hole in the ground under it)
            let w = MapSpline { file: wire.to_string(), id: ids[id], prev_id: link(s.prev_id), next_id: link(s.next_id), mirror: false, terrain_align: None, ..s };
            let t = doc.tiles.get_mut(k).unwrap();
            t.tile.splines.push(w);
            t.touch_content();
        }
        self.history.commit();
        ids.len()
    }

    /// Every object (or spline) of the same type as `s` on the map, selected; `s` stays the
    /// one in the properties. How many.
    pub fn select_alike(&mut self, s: Sel) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let file = match s {
            Sel::Object(k, id) => selection::any_object(doc, k, id).map(|o| o.file.clone()),
            Sel::Spline(k, id) => doc.spline(k, id).map(|x| x.file.clone()),
            Sel::Attached(k, id) => doc.attachment(k, id).map(|a| a.file.clone()),
        };
        let Some(file) = file else { return 0 };
        let mut v = vec![s];
        for (k, t) in &doc.tiles {
            match s {
                Sel::Object(..) => v.extend(t.tile.objects.iter().chain(&t.tile.attach_objects).filter(|o| o.file.eq_ignore_ascii_case(&file)).map(|o| Sel::Object(*k, o.id)).filter(|x| *x != s)),
                Sel::Spline(..) => v.extend(t.tile.splines.iter().filter(|x| !x.deleted && x.file.eq_ignore_ascii_case(&file)).map(|x| Sel::Spline(*k, x.id)).filter(|x| *x != s)),
                Sel::Attached(..) => v.extend(t.tile.spline_attachments.iter().filter(|a| a.file.eq_ignore_ascii_case(&file)).map(|a| Sel::Attached(*k, a.id)).filter(|x| *x != s)),
            }
        }
        let n = v.len();
        self.set_selection(v);
        n
    }

    /// Objects and splines a chrono scenario had, put into the map's tile `k` (they are the
    /// map's again, from the first day on). Inside one step.
    pub fn put_back(&mut self, k: TileKey, objects: Vec<MapObject>, hanging: Vec<MapObject>, splines: Vec<MapSpline>) {
        let Some(doc) = self.doc.as_mut() else { return };
        if !doc.tiles.contains_key(&k) {
            return;
        }
        self.history.begin(tr("Back into the map", "Обратно в карту"));
        self.history.touch_tile(doc, k);
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.objects.extend(objects);
        t.tile.attach_objects.extend(hanging);
        t.tile.splines.extend(splines);
        t.touch_content();
        self.history.commit();
    }

    /// The object `file` hung on point `at` of object `id` (`[attachObj]`); it is selected.
    pub fn attach_object(&mut self, k: TileKey, parent: i64, at: usize, file: &str) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Hang an object", "Подвесить объект"));
        self.history.touch_tile(doc, k);
        self.history.touch_global(doc);
        let id = doc.new_id();
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.tile.attach_objects.push(MapObject { file: file.to_string(), id, parent_id: Some(parent), attach_index: at, ..Default::default() });
            t.touch_content();
        }
        self.history.commit();
        self.select(Some(Sel::Object(k, id)));
    }

    /// Put the free object `id` onto the road nearest it as a `[splineAttachement]` record:
    /// from then on it stands on that road and follows it, keeping its IDCode. `Err` says why
    /// not.
    pub fn attach_object_to_spline(&mut self, lib: &Library, k: TileKey, id: i64) -> Result<(), &'static str> {
        let Some(doc) = self.doc.as_ref() else { return Err("") };
        let Some(o) = doc.object(k, id).cloned() else {
            return Err(tr("Only a free object can be put on a road", "На дорогу можно поставить только свободный объект"));
        };
        if o.parent_id.is_some() {
            return Err(tr("An object hanging on another cannot go onto a road", "Объект, висящий на другом, нельзя поставить на дорогу"));
        }
        let org = tile_origin(k);
        let p = org + DVec2::new(o.pos[0], o.pos[1]);
        // the nearest road's own spline, and where the object lies on it
        let mut best: Option<(f64, TileKey, i64, f64, f64, f64, f64)> = None;
        for (tk, t) in &doc.tiles {
            let torg = tile_origin(*tk);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                if (torg + DVec2::new(s.pos[0], s.pos[1]) - p).length() > s.length + 30.0 {
                    continue;
                }
                let Some((u, beside)) = selection::on_spline(s, torg, p) else { continue };
                let c = omsi_geometry::SplineCurve::from_map(s, torg);
                let d = beside.abs();
                if d < 30.0 && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, *tk, s.id, u, beside, c.heading_at(u), c.point_at(u).z));
                }
            }
        }
        let Some((_, sk, sid, u, beside, heading, road_z)) = best else {
            return Err(tr("There is no road within 30 m", "В пределах 30 м нет дороги"));
        };
        let abs = lib.find(&o.file).map(|a| a.abs_height).unwrap_or(false);
        let world_z = if abs { o.pos[2] } else { doc.ground(p) + o.pos[2] };
        let rec = omsi_map::SplineAttachment {
            file: o.file.clone(),
            id: o.id,
            spline_index: -1,
            offset: [beside, world_z - road_z, 0.0],
            rot: [roads::norm_deg(o.rot[0] - heading), o.rot[1], o.rot[2]],
            interval: 0.0,
            range: 0.0,
            tilt: false,
            strings: o.extra.clone(),
            var_parent: o.var_parent,
            rules: o.rules.clone(),
            ..Default::default()
        };
        let Some(doc) = self.doc.as_mut() else { return Err("") };
        self.history.begin(tr("Put on a road", "Поставить на дорогу"));
        self.history.touch_tile(doc, k);
        self.history.touch_tile(doc, sk);
        let Some(t) = doc.tiles.get_mut(&sk) else { return Err("") };
        let Some(idx) = t.tile.splines.iter().position(|s| s.id == sid && !s.deleted) else {
            return Err(tr("The road is gone", "Дороги уже нет"));
        };
        let mut rec = rec;
        rec.spline_index = idx as i32;
        rec.offset[2] = t.tile.splines[idx].map_chain_offset.unwrap_or(0.0) + u;
        t.tile.spline_attachments.push(rec);
        t.touch_content();
        doc.delete_object(k, id);
        self.history.commit();
        self.select(Some(Sel::Attached(sk, id)));
        Ok(())
    }

    /// Take a `[splineAttachement]` record off its road: it becomes a free object where its
    /// first copy stood, keeping its IDCode. A record of several copies becomes one object.
    pub fn detach_attachment(&mut self, k: TileKey, id: i64) -> Result<(), &'static str> {
        let Some(doc) = self.doc.as_ref() else { return Err("") };
        let Some(a) = doc.attachment(k, id).cloned() else {
            return Err(tr("The object is gone", "Объекта уже нет"));
        };
        let Some(first) = selection::attachment_first(doc, k, id) else {
            return Err(tr("It stands on no road", "Он не стоит на дороге"));
        };
        let world = first.pose.pos;
        let tk = crate::doc::tile_of(world.truncate());
        if !doc.tiles.contains_key(&tk) {
            return Err(tr("There is no tile there", "Там нет тайла"));
        }
        let torg = tile_origin(tk);
        let z = world.z - doc.ground(world.truncate());
        let o = MapObject {
            file: a.file.clone(),
            id: a.id,
            pos: [world.x - torg.x, world.y - torg.y, z],
            rot: [crate::selection::attachment_world_rot(doc, k, &a), a.rot[1], a.rot[2]],
            flag: a.strings.len() as i32,
            extra: a.strings.clone(),
            var_parent: a.var_parent,
            rules: a.rules.clone(),
            ..Default::default()
        };
        let Some(doc) = self.doc.as_mut() else { return Err("") };
        self.history.begin(tr("Take off the spline", "Снять со сплайна"));
        self.history.touch_tile(doc, k);
        self.history.touch_tile(doc, tk);
        // (what hangs on its IDCode keeps hanging on it: the free object has the same IDCode)
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.tile.spline_attachments.retain(|x| x.id != id);
            t.touch_content();
        }
        if let Some(t) = doc.tiles.get_mut(&tk) {
            t.tile.objects.push(o);
            t.touch_content();
        }
        self.history.commit();
        self.select(Some(Sel::Object(tk, id)));
        Ok(())
    }

    /// A record's place on its road and the shape of its row, as the properties panel changes
    /// them (one step while a number is dragged).
    pub fn edit_attachment(&mut self, k: TileKey, id: i64, f: impl FnOnce(&mut omsi_map::SplineAttachment)) {
        self.begin_panel_step(tr("Edit object", "Изменение объекта"));
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.touch_tile(doc, k);
        if let Some(a) = doc.attachment_mut(k, id) {
            f(a);
        }
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.touch_content();
        }
    }

    /// Object `id` takes its state from object `parent` (`[varparent]`: a signal head from
    /// its crossing's light program, a stop's display from its stop), or from none.
    pub fn set_var_parent(&mut self, k: TileKey, id: i64, parent: Option<i64>) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(if parent.is_some() { tr("Link to an object", "Связать с объектом") } else { tr("Unlink", "Отвязать") });
        self.history.touch_tile(doc, k);
        if let Some(o) = selection::any_object_mut(doc, k, id) {
            o.var_parent = parent;
        }
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.touch_content();
        }
        self.history.commit();
    }

    /// The object clicked while linking: `id` takes its state from it, unless that would
    /// close a ring (it takes its own state, through others, from `id`).
    fn link_to(&mut self, k: TileKey, id: i64, parent: i64) {
        let Some(doc) = self.doc.as_ref() else { return };
        let mut at = Some(parent);
        for _ in 0..64 {
            match at {
                Some(p) if p == id => {
                    self.toast(tr("That object already takes its state from this one", "Тот объект сам берёт состояние от этого"));
                    return;
                }
                Some(p) => at = selection::var_parent_of(doc, p),
                None => break,
            }
        }
        self.set_var_parent(k, id, Some(parent));
        self.toast(tr("Linked", "Связано"));
    }

    /// Which light of its parent's program a signal head shows (its first label).
    pub fn set_light_index(&mut self, k: TileKey, id: i64, light: usize) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Signal head's light", "Сигнал светофора"));
        self.history.touch_tile(doc, k);
        if let Some(o) = selection::any_object_mut(doc, k, id) {
            match o.extra.first_mut() {
                Some(l) => *l = light.to_string(),
                None => o.extra.push(light.to_string()),
            }
            o.flag = o.flag.max(o.extra.len() as i32);
        }
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.touch_content();
        }
        self.history.commit();
    }

    /// The rules of an object's or a spline's paths (one step while a number is dragged).
    pub fn set_rules(&mut self, sel: Sel, rules: Vec<omsi_map::MapRule>) {
        self.begin_panel_step(tr("Path rules", "Правила путей"));
        let Some(doc) = self.doc.as_mut() else { return };
        let k = match sel {
            Sel::Object(k, _) | Sel::Spline(k, _) | Sel::Attached(k, _) => k,
        };
        self.history.touch_tile(doc, k);
        match sel {
            Sel::Object(k, id) => {
                if let Some(o) = selection::any_object_mut(doc, k, id) {
                    o.rules = rules;
                }
            }
            Sel::Spline(k, id) => {
                if let Some(s) = doc.tiles.get_mut(&k).and_then(|t| t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted)) {
                    s.rules = rules;
                }
            }
            Sel::Attached(k, id) => {
                if let Some(a) = doc.attachment_mut(k, id) {
                    a.rules = rules;
                }
            }
        }
        doc.tiles.get_mut(&k).map(|t| t.touch_content());
    }

    /// The selected objects made ones of the type `file`, where they stand.
    pub fn replace_selected_objects(&mut self, file: &str) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Replace objects", "Замена объектов"));
        for s in &sels {
            if let Sel::Attached(k, id) = *s {
                self.history.touch_tile(doc, k);
                if let Some(a) = doc.attachment_mut(k, id) {
                    a.file = file.to_string();
                }
                doc.tiles.get_mut(&k).map(|t| t.touch_content());
                continue;
            }
            let Sel::Object(k, id) = *s else { continue };
            self.history.touch_tile(doc, k);
            if let Some(o) = selection::any_object_mut(doc, k, id) {
                o.file = file.to_string();
            }
            doc.tiles.get_mut(&k).map(|t| t.touch_content());
        }
        self.history.commit();
    }

    /// Every object (or spline) of the type `from` made one of `to`, in one step. How many.
    pub fn replace_everywhere(&mut self, from: &str, to: &str, splines: bool) -> usize {
        let Some(doc) = self.doc.as_mut() else { return 0 };
        self.history.begin(if splines { tr("Change road type", "Смена типа дороги") } else { tr("Replace objects", "Замена объектов") });
        let mut n = 0;
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            let t = &doc.tiles[&k];
            let here = if splines { t.tile.splines.iter().filter(|s| !s.deleted && s.file.eq_ignore_ascii_case(from)).count() } else { t.tile.objects.iter().chain(t.tile.attach_objects.iter()).filter(|o| o.file.eq_ignore_ascii_case(from)).count() };
            if here == 0 {
                continue;
            }
            self.history.touch_tile(doc, k);
            let t = doc.tiles.get_mut(&k).unwrap();
            if splines {
                for s in t.tile.splines.iter_mut().filter(|s| !s.deleted && s.file.eq_ignore_ascii_case(from)) {
                    s.file = to.to_string();
                }
            } else {
                for o in t.tile.objects.iter_mut().chain(t.tile.attach_objects.iter_mut()).filter(|o| o.file.eq_ignore_ascii_case(from)) {
                    o.file = to.to_string();
                }
            }
            t.touch_content();
            n += here;
        }
        self.history.commit();
        n
    }

    /// Height and gradient at one end of a spline (the properties); the neighbour linked
    /// there follows. Kept open as one step while a number is dragged.
    pub fn set_spline_end(&mut self, k: TileKey, id: i64, at_end: bool, z: Option<f64>, g: Option<f64>) {
        self.begin_panel_step(tr("Edit road", "Изменение дороги"));
        let Some(doc) = self.doc.as_mut() else { return };
        selection::touch_spline(&mut self.history, doc, k, id);
        reshape::set_end(doc, k, id, at_end, z, g);
    }

    /// A road's length and radius by numbers: its start stays, the road joined at its end
    /// follows, tangent, its far end kept.
    fn set_spline_shape(&mut self, assets: &mut Assets, k: TileKey, id: i64, length: f64, radius: f64) {
        let joined = self.helper_anchors(assets);
        let locked = |q: DVec2| joined.iter().any(|j| (j.truncate() - q).length() < 0.6);
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(s) = doc.spline(k, id).cloned() else { return };
        let org = tile_origin(k);
        let old = omsi_geometry::SplineCurve::from_map(&s, org);
        let mut new = s.clone();
        new.length = length;
        new.radius = radius;
        let c = omsi_geometry::SplineCurve::from_map(&new, org);
        let p = c.end_point();
        let mut plan = None;
        if let Some((nk, n)) = doc.find_spline(s.next_id) {
            let at_end = if n.prev_id == id { false } else if n.next_id == id { true } else { return };
            match reshape::carry_end(doc, nk, n.id, at_end, p.truncate(), crate::roads::norm_deg(c.heading_at(length) + 180.0), p.z - old.end_point().z, &locked) {
                Ok(pl) => plan = Some(pl),
                Err(e) => {
                    self.toast(e);
                    return;
                }
            }
        }
        self.begin_panel_step(tr("Edit road", "Изменение дороги"));
        let Some(doc) = self.doc.as_mut() else { return };
        selection::touch_spline(&mut self.history, doc, k, id);
        if let Some(t) = doc.tiles.get_mut(&k) {
            if let Some(x) = t.tile.splines.iter_mut().find(|x| x.id == id && !x.deleted) {
                x.length = length;
                x.radius = radius;
            }
            t.touch_content();
        }
        if let Some(plan) = plan {
            self.apply_plan(&plan);
        }
    }

    /// A spline's skew at its ends and how it cuts the ground (0: not, 1: under it, 2–4: the
    /// hole held at the end, the start, both: `[spline_terrain_align_2]`).
    pub fn set_spline_extra(&mut self, k: TileKey, id: i64, skew_start: f64, skew_end: f64, ground: u8) {
        self.begin_panel_step(tr("Edit road", "Изменение дороги"));
        let Some(doc) = self.doc.as_mut() else { return };
        selection::touch_spline(&mut self.history, doc, k, id);
        let Some(t) = doc.tiles.get_mut(&k) else { return };
        if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted) {
            s.skew_start = skew_start;
            s.skew_end = skew_end;
            s.terrain_align_flag = ground == 1;
            s.terrain_align = (ground >= 2).then_some(ground as f64);
        }
        t.touch_content();
    }

    /// A road's cant (%) at one end; the road joined there follows.
    pub fn set_spline_cant(&mut self, k: TileKey, id: i64, at_end: bool, cant: f64) {
        self.begin_panel_step(tr("Edit road", "Изменение дороги"));
        let Some(doc) = self.doc.as_mut() else { return };
        selection::touch_spline(&mut self.history, doc, k, id);
        reshape::set_cant(doc, k, id, at_end, cant);
    }

    // ------------------------------------------------------------ junctions and ground

    /// Learn the types of what is selected, as their models come in.
    fn learn(&mut self, assets: &mut Assets) {
        let Some(doc) = self.doc.as_ref() else { return };
        let sels = self.selection();
        if sels.len() > 5000 {
            return;
        }
        let mut splines: Vec<String> = Vec::new();
        for s in &sels {
            match *s {
                Sel::Object(k, id) => {
                    let Some(o) = doc.object(k, id) else { continue };
                    let key = o.file.to_lowercase();
                    if !self.known_helpers.contains_key(&key) {
                        if let Some(m) = assets.object(&o.file) {
                            self.known_helpers.insert(key.clone(), (m.abs_height, m.helpers.iter().map(|h| h.0).collect()));
                        }
                    }
                    // (and the roads it holds)
                    if let Some((abs, h)) = self.known_helpers.get(&key).filter(|h| !h.1.is_empty()) {
                        for a in selection::arms_of(doc, &selection::helper_points(doc, k, id, *abs, h)) {
                            if let Some((ak, aid, _)) = selection::free_end_at(doc, a.pos.truncate()) {
                                if let Some(sp) = doc.spline(ak, aid) {
                                    splines.push(sp.file.clone());
                                }
                            }
                        }
                    }
                }
                Sel::Attached(k, id) => {
                    let Some(a) = doc.attachment(k, id) else { continue };
                    let key = a.file.to_lowercase();
                    if !self.known_helpers.contains_key(&key) {
                        if let Some(m) = assets.object(&a.file) {
                            self.known_helpers.insert(key.clone(), (m.abs_height, m.helpers.iter().map(|h| h.0).collect()));
                        }
                    }
                    // (the road it stands on)
                    if let Some(t) = doc.tiles.get(&k) {
                        if let Some(sp) = t.tile.splines.get(a.spline_index.max(0) as usize) {
                            splines.push(sp.file.clone());
                        }
                    }
                }
                Sel::Spline(k, id) => {
                    if let Some(sp) = doc.spline(k, id) {
                        splines.push(sp.file.clone());
                        // (and the junctions at its ends, which a delete makes again)
                        let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(k));
                        let ends = [c.point_at(0.0).truncate(), c.end_point().truncate()];
                        for (jk, t) in &doc.tiles {
                            for o in t.tile.objects.iter().filter(|o| is_own_junction(&o.file)) {
                                let key = o.file.to_lowercase();
                                if self.known_helpers.contains_key(&key) {
                                    continue;
                                }
                                let at = tile_origin(*jk) + DVec2::new(o.pos[0], o.pos[1]);
                                if ends.iter().any(|e| (*e - at).length() < 80.0) {
                                    if let Some(m) = assets.object(&o.file) {
                                        self.known_helpers.insert(key, (m.abs_height, m.helpers.iter().map(|h| h.0).collect()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for f in splines {
            let key = f.to_lowercase();
            if !self.known_half.contains_key(&key) {
                if let Some(m) = assets.spline(&f) {
                    self.known_half.insert(key, if m.joinable() { m.half_width as f64 } else { 0.0 });
                }
            }
        }
    }

    /// Half the width of a spline type the ground is brought to (0: left alone).
    fn half_of(&self, file: &str) -> f64 {
        self.known_half.get(&file.to_lowercase()).copied().unwrap_or(0.0)
    }

    /// The road ends the selected junctions hold (not those of roads selected with them,
    /// which move along).
    fn arms_for(&self, sels: &[Sel]) -> Vec<(Sel, Vec<selection::Arm>)> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let ids: Vec<i64> = sels.iter().filter_map(|s| if let Sel::Spline(_, id) = s { Some(*id) } else { None }).collect();
        let mut out = Vec::new();
        for s in sels {
            let Sel::Object(k, id) = *s else { continue };
            let Some(o) = doc.object(k, id) else { continue };
            let Some((abs, h)) = self.known_helpers.get(&o.file.to_lowercase()).filter(|h| !h.1.is_empty()) else { continue };
            let arms: Vec<selection::Arm> = selection::arms_of(doc, &selection::helper_points(doc, k, id, *abs, h))
                .into_iter()
                .filter(|a| selection::free_end_at(doc, a.pos.truncate()).map(|e| !ids.contains(&e.1)).unwrap_or(false))
                .collect();
            if !arms.is_empty() {
                out.push((*s, arms));
            }
        }
        out
    }

    /// The roads junctions hold follow them (turned by `deg` round `pivot`, shifted by
    /// `delta`, raised by `dz`), and the ground comes to the junctions' plates. Inside an open
    /// history step.
    fn carry(&mut self, groups: &[(Sel, Vec<selection::Arm>)], pivot: DVec2, deg: f64, delta: DVec2, dz: f64) {
        let mut failed = 0;
        for (_, arms) in groups {
            for a in arms {
                // (one at a time against the map as it now is: two arms may be one road)
                let plan = self.doc.as_ref().and_then(|d| selection::carry_plans(d, &[*a], pivot, deg, delta, dz, &|_| false).pop());
                match plan {
                    Some(Ok(p)) => self.apply_plan(&p),
                    Some(Err(_)) => failed += 1,
                    None => {}
                }
            }
        }
        if self.follow_ground && (delta.length() > 1e-6 || deg.abs() > 1e-9 || dz.abs() > 1e-9) {
            for (sel, arms) in groups {
                let Sel::Object(_, id) = *sel else { continue };
                let Some(doc) = self.doc.as_mut() else { return };
                let Some((k, o)) = doc.tiles.iter().find_map(|(k, t)| t.tile.objects.iter().find(|o| o.id == id).map(|o| (*k, o.clone()))) else { continue };
                let abs = self.known_helpers.get(&o.file.to_lowercase()).map(|h| h.0).unwrap_or(false);
                let centre = doc.object_world(k, &o, abs);
                let ends: Vec<DVec3> = arms.iter().map(|a| (selection::turn_point(a.pos.truncate(), pivot, deg) + delta).extend(a.pos.z + dz)).collect();
                let r = ends.iter().map(|e| (e.truncate() - centre.truncate()).length()).fold(3.0, f64::max) * 1.05;
                self.touch_around(&[centre.truncate()], r + 9.0);
                let Some(doc) = self.doc.as_mut() else { return };
                conform_disc(doc, centre, r, &ends);
            }
        }
        if delta.length() > 1e-6 || deg.abs() > 1e-9 || dz.abs() > 1e-9 {
            for (sel, _) in groups {
                if let Sel::Object(_, id) = *sel {
                    if self.doc.as_ref().and_then(|d| d.tiles.values().find_map(|t| t.tile.objects.iter().find(|o| o.id == id))).map(|o| is_own_junction(&o.file)).unwrap_or(false) {
                        self.pending_rebuild.push(id);
                    }
                }
            }
        }
        if failed > 0 {
            self.error(format!("{} {failed}", tr("Roads that could not follow the junction (too sharp a bend):", "Дороги, которые не смогли последовать за перекрёстком (слишком крутой изгиб):")));
        }
    }

    /// Close the open step, unless junction plates are still to be made again in it (the
    /// next tick does that and closes it).
    fn finish_step(&mut self) {
        if self.pending_rebuild.is_empty() {
            self.history.commit();
        }
    }

    /// openMapper's junctions (known ones) holding an end of a spline in `sels`, not
    /// themselves among them.
    fn junctions_at_ends(&self, sels: &[Sel]) -> Vec<i64> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let mut ends: Vec<DVec2> = Vec::new();
        for s in sels {
            if let Sel::Spline(k, id) = *s {
                if let Some(sp) = doc.spline(k, id) {
                    let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(k));
                    ends.extend([c.point_at(0.0).truncate(), c.end_point().truncate()]);
                }
            }
        }
        let mut out = Vec::new();
        for (k, t) in &doc.tiles {
            for o in t.tile.objects.iter().filter(|o| is_own_junction(&o.file) && !sels.contains(&Sel::Object(*k, o.id))) {
                let Some((abs, h)) = self.known_helpers.get(&o.file.to_lowercase()) else { continue };
                let pts = selection::helper_points(doc, *k, o.id, *abs, h);
                if pts.iter().any(|(p, _)| ends.iter().any(|e| (*e - p.truncate()).length() < 0.6)) {
                    out.push(o.id);
                }
            }
        }
        out
    }

    /// openMapper's junction plate a point lies on: (its IDCode, centre, radius out to its
    /// arms' ends).
    fn plate_at(&self, assets: &mut Assets, p: DVec2) -> Option<(i64, DVec3, f64)> {
        let doc = self.doc.as_ref()?;
        for (k, t) in &doc.tiles {
            for o in t.tile.objects.iter().filter(|o| is_own_junction(&o.file)) {
                let at = tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1]);
                if (at - p).length() > 80.0 {
                    continue;
                }
                let Some(m) = assets.object(&o.file) else { continue };
                let r = m.helpers.iter().map(|h| DVec2::new(h.0[0] as f64, h.0[1] as f64).length()).fold(0.0, f64::max);
                // (its middle, or anywhere on the plate itself: a corner reaches past the
                // arms' ends)
                let local = object_matrix(DVec3::ZERO, o.rot).inverse().transform_point3(Vec3::new((p.x - at.x) as f32, (p.y - at.y) as f32, 0.0));
                let on_plate = local.x >= m.lo.x && local.x <= m.hi.x && local.y >= m.lo.y && local.y <= m.hi.y;
                if r > 0.0 && ((at - p).length() < r * 0.85 || on_plate) {
                    return Some((o.id, doc.object_world(*k, o, m.abs_height), r));
                }
            }
        }
        None
    }

    /// The headings out of a junction of the roads it holds (its known helpers).
    pub fn junction_arms(&self, k: TileKey, id: i64) -> Vec<f64> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let Some(o) = doc.object(k, id) else { return Vec::new() };
        let Some((abs, h)) = self.known_helpers.get(&o.file.to_lowercase()) else { return Vec::new() };
        selection::arms_of(doc, &selection::helper_points(doc, k, id, *abs, h)).iter().map(|a| a.out).collect()
    }

    /// Change a junction's lights and crossings: it is made again with them.
    pub fn set_junction_options(&mut self, id: i64, opts: crate::junction::Options) {
        if self.doc.is_none() {
            return;
        }
        self.history.begin(tr("Edit junction", "Изменение перекрёстка"));
        self.junction_options.insert(id, opts.clone());
        self.pending_options.insert(id, opts);
        self.pending_rebuild.push(id);
    }

    /// After a save: the junction files of this map that nothing uses any more (not a tile,
    /// not a tile undo or redo may bring back, not an earlier version kept by a save, and not
    /// one made in this session) are deleted.
    /// Returns how many went.
    pub fn clean_junction_files(&self) -> usize {
        let (Some(doc), Some(content)) = (self.doc.as_ref(), self.content_dir.as_ref()) else { return 0 };
        let mut used: std::collections::HashSet<String> = doc
            .tiles
            .values()
            .flat_map(|t| t.tile.objects.iter().chain(t.chrono.iter().flat_map(|c| c.objects.iter())))
            .map(|o| o.file.as_str())
            // (and those undo or redo can bring back)
            .chain(self.history.object_files())
            .filter(|f| is_own_junction(f))
            .map(|f| f.to_lowercase())
            .collect();
        // (and those the earlier versions kept by saves name: going back to one brings them back)
        for b in crate::doc::safe::backups(&doc.dir) {
            for e in std::fs::read_dir(&b).into_iter().flatten().flatten() {
                if !e.file_name().to_string_lossy().to_lowercase().ends_with(".map") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(e.path()) else { continue };
                used.extend(omsi_cfg::decode_text(&bytes).lines().map(str::trim).filter(|l| is_own_junction(l)).map(str::to_lowercase));
            }
        }
        let folder: String = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "map".into()).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let dir = crate::doc::safe::own_path(content, &format!("Sceneryobjects\\openMapper\\{folder}"));
        crate::junction::remove_unused(&dir, &folder, &used, self.session_first_id)
    }

    /// One of openMapper's junction plates made again at the next tick (its file lost: the
    /// map moved without it); false if it is not on the map.
    pub fn remake_junction(&mut self, id: i64) -> bool {
        let Some(doc) = self.doc.as_ref() else { return false };
        if !doc.tiles.values().any(|t| t.tile.objects.iter().any(|o| o.id == id && is_own_junction(&o.file))) {
            return false;
        }
        self.history.begin(tr("Make the junction again", "Перекрёсток заново"));
        self.pending_rebuild.push(id);
        true
    }

    /// Every junction plate of openMapper's made again at the next tick, for the roads it has
    /// now (with the newest asphalt and kerbs); one undo step. How many.
    pub fn remake_all_junctions(&mut self) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        let ids: Vec<i64> = doc.tiles.values().flat_map(|t| t.tile.objects.iter().filter(|o| is_own_junction(&o.file)).map(|o| o.id)).collect();
        if !ids.is_empty() {
            self.history.begin(tr("Make the junctions again", "Перестроить перекрёстки"));
            self.pending_rebuild.extend(ids.iter().copied());
        }
        ids.len()
    }

    /// Make the queued junctions' plates again for the arms they have now.
    fn rebuild_pending(&mut self, assets: &mut Assets) {
        let ids = std::mem::take(&mut self.pending_rebuild);
        for id in ids {
            self.rebuild_junction(assets, id);
        }
    }

    /// One of openMapper's junctions made again in place for the roads now ending at its
    /// helper points: three or more, a new plate; two, a smooth joint between them; fewer,
    /// nothing. Inside an open history step.
    fn rebuild_junction(&mut self, assets: &mut Assets, id: i64) {
        let Some(content) = self.content_dir.clone() else { return };
        let asphalt = self.asphalt.clone();
        let adapt = self.auto_adapt;
        let follow = self.follow_ground;
        let was_selected = self.selection().iter().any(|s| matches!(s, Sel::Object(_, x) if *x == id));
        let Some(doc) = self.doc.as_mut() else { return };
        let Some((jk, o)) = doc.tiles.iter().find_map(|(k, t)| t.tile.objects.iter().find(|o| o.id == id).map(|o| (*k, o.clone()))) else { return };
        let mut arms: Vec<crate::junction::Arm> = Vec::new();
        // (a plate whose file is gone, a map moved without them: the roads that end facing
        // its middle are its arms; it stood at its own height)
        let abs = match plate_helpers(&content, assets, &o.file) {
            Some((abs, helpers)) => {
                let pts = selection::helper_points(doc, jk, id, abs, &helpers);
                for a in selection::arms_of(doc, &pts) {
                    if let Some((k, sid, at_end)) = selection::free_end_at(doc, a.pos.truncate()) {
                        if !arms.iter().any(|x| x.id == sid && x.at_end == at_end) {
                            arms.push(crate::junction::Arm { key: k, id: sid, at_end });
                        }
                    }
                }
                abs
            }
            None if is_own_junction(&o.file) => {
                arms = ends_facing(doc, tile_origin(jk) + DVec2::new(o.pos[0], o.pos[1]), 30.0);
                true
            }
            None => return,
        };
        for a in self.pending_arms.remove(&id).unwrap_or_default() {
            if doc.spline(a.key, a.id).is_some() && !arms.iter().any(|x| x.id == a.id && x.at_end == a.at_end) {
                arms.push(a);
            }
        }
        let opts = self.pending_options.remove(&id).unwrap_or_else(|| crate::junction::Options::read(&omsi_cfg::resolve_path(&content, &o.file)));
        let centre = doc.object_world(jk, &o, abs);
        self.history.touch_tile(doc, jk);
        for a in &arms {
            self.history.touch_tile(doc, a.key);
        }
        self.history.touch_global(doc);
        // its signal heads go with it (new ones are put up for the new plate)
        for (k, c) in crate::junction::children(doc, id) {
            self.history.touch_tile(doc, k);
            doc.delete_object(k, c);
        }
        doc.delete_object(jk, id);
        self.junction_options.remove(&id);
        match arms.len() {
            n if n >= 3 => match crate::junction::make_with(doc, assets, &content, asphalt.as_deref(), centre, &arms, &opts) {
                Ok(new) => {
                    if was_selected {
                        let k = doc.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == new)).map(|(k, _)| *k).unwrap_or(jk);
                        let v: Vec<Sel> = self.selection().into_iter().map(|s| if s == Sel::Object(jk, id) { Sel::Object(k, new) } else { s }).collect();
                        self.set_selection(v);
                    }
                }
                Err(e) => {
                    // (the plate as it was, rather than none)
                    log::warn!("junction: {e}");
                    if let Some(t) = doc.tiles.get_mut(&jk) {
                        t.tile.objects.push(o);
                        t.touch_content();
                    }
                }
            },
            2 => {
                join_two(doc, assets, adapt, follow, &content, asphalt.as_deref(), &arms[0], &arms[1]);
                if was_selected {
                    self.select(None);
                }
            }
            _ => {
                if was_selected {
                    self.select(None);
                }
            }
        }
    }

    /// Keep every tile within `reach` of the points before the ground there changes.
    fn touch_around(&mut self, pts: &[DVec2], reach: f64) {
        let Some(doc) = self.doc.as_ref() else { return };
        let mut keys: Vec<TileKey> = Vec::new();
        for p in pts {
            let (a, b) = (tile_of(*p - DVec2::splat(reach)), tile_of(*p + DVec2::splat(reach)));
            for x in a.0..=b.0 {
                for y in a.1..=b.1 {
                    if doc.tiles.contains_key(&(x, y)) && !keys.contains(&(x, y)) {
                        keys.push((x, y));
                    }
                }
            }
        }
        for k in keys {
            self.history.touch_tile(doc, k);
        }
    }

    /// Put a reshape into the map, the ground following it. Inside an open history step.
    fn apply_plan(&mut self, plan: &reshape::Plan) {
        let Some(doc) = self.doc.as_ref() else { return };
        for t in reshape::tiles_of(doc, plan) {
            self.history.touch_tile(doc, t);
        }
        self.history.touch_global(doc);
        let half = plan.pieces.first().and_then(|p| doc.spline(p.key, p.id)).map(|s| self.half_of(&s.file)).unwrap_or(0.0);
        let curves: Vec<omsi_geometry::SplineCurve> = reshape::preview(doc, plan).iter().map(|s| omsi_geometry::SplineCurve::from_map(s, DVec2::ZERO)).collect();
        let follow = self.follow_ground && half > 0.0;
        if follow {
            let pts: Vec<DVec2> = curves.iter().flat_map(|c| (0..=8).map(move |i| c.point_at(c.length * i as f64 / 8.0).truncate())).collect();
            self.touch_around(&pts, half + 12.0);
        }
        let Some(doc) = self.doc.as_mut() else { return };
        reshape::apply(doc, plan);
        if follow {
            conform_ground(doc, &curves, half);
        }
    }

    /// Bring the ground to roads ("Shape the ground" on), each type with its own width
    /// (`half`: 0 leaves one alone). Inside an open history step.
    fn ground_to(&mut self, splines: &[(TileKey, i64)], half: &dyn Fn(&str) -> f64) {
        if !self.follow_ground {
            return;
        }
        let Some(doc) = self.doc.as_ref() else { return };
        let mut groups: Vec<(f64, Vec<omsi_geometry::SplineCurve>)> = Vec::new();
        for (k, id) in splines {
            let Some(s) = doc.spline(*k, *id) else { continue };
            let h = half(&s.file);
            if h <= 0.0 {
                continue;
            }
            let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(*k));
            match groups.iter_mut().find(|g| (g.0 - h).abs() < 1e-6) {
                Some(g) => g.1.push(c),
                None => groups.push((h, vec![c])),
            }
        }
        for (h, curves) in groups {
            let pts: Vec<DVec2> = curves.iter().flat_map(|c| (0..=8).map(move |i| c.point_at(c.length * i as f64 / 8.0).truncate())).collect();
            self.touch_around(&pts, h + 12.0);
            let Some(doc) = self.doc.as_mut() else { return };
            conform_ground(doc, &curves, h);
        }
    }

    /// The ground to the selected roads (after they moved or turned).
    fn ground_to_selection(&mut self) {
        let roads: Vec<(TileKey, i64)> = self.selection().iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((*k, *id)) } else { None }).collect();
        let known = self.known_half.clone();
        self.ground_to(&roads, &|f| known.get(&f.to_lowercase()).copied().unwrap_or(0.0));
    }

    // ------------------------------------------------------------ objects along roads

    /// Put the object type chosen in the catalogue along the selected roads.
    pub fn place_rows(&mut self) {
        let Some(file) = self.object_type.clone() else {
            self.toast(tr("Pick an object in the Objects tool's catalogue first", "Сначала выберите объект в каталоге инструмента «Объекты»"));
            return;
        };
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        let chains = crate::rows::chains(doc, &sels);
        let rows: Vec<_> = chains.iter().flat_map(|c| crate::rows::plan(doc, c, &file, &self.row_params)).collect();
        if rows.is_empty() {
            self.toast(tr("The road is too short for that", "Дорога для этого слишком короткая"));
            return;
        }
        self.history.begin(tr("Objects along the road", "Объекты вдоль дороги"));
        for (k, _, _) in &rows {
            self.history.touch_tile(doc, *k);
        }
        self.history.touch_global(doc);
        let n = crate::rows::add(doc, rows);
        self.history.commit();
        self.toast(format!("{} {n}", tr("Rows of objects added:", "Добавлено рядов объектов:")));
    }

    /// Change a row (the road's properties): one step while a number is dragged.
    pub fn edit_row(&mut self, k: TileKey, row: usize, f: impl FnOnce(&mut omsi_map::SplineAttachment)) {
        self.begin_panel_step(tr("Edit row of objects", "Изменение ряда объектов"));
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.touch_tile(doc, k);
        if let Some(t) = doc.tiles.get_mut(&k) {
            if let Some(a) = t.tile.spline_attachments.get_mut(row) {
                f(a);
                t.touch_content();
            }
        }
    }

    pub fn remove_row(&mut self, k: TileKey, row: usize) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Remove row of objects", "Удаление ряда объектов"));
        self.history.touch_tile(doc, k);
        crate::rows::remove(doc, k, row);
        self.history.commit();
    }

    /// Take every row off the selected roads.
    pub fn remove_rows_of_selection(&mut self) {
        let sels = self.selection();
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Remove rows of objects", "Удаление рядов объектов"));
        for s in &sels {
            let Sel::Spline(k, id) = *s else { continue };
            self.history.touch_tile(doc, k);
            // (from the last: the places of the others stay)
            for r in crate::rows::rows_on(doc, k, id).into_iter().rev() {
                crate::rows::remove(doc, k, r);
            }
        }
        self.history.commit();
    }

    // ------------------------------------------------------------ drawing by numbers

    /// The numbers typed for the road being drawn.
    pub fn typed(&self) -> crate::draw::Typed {
        let t = &self.typed_text;
        crate::draw::Typed { length: crate::draw::parse(&t[0]).filter(|v| *v > 0.0), radius: crate::draw::parse(&t[1]), heading: crate::draw::parse(&t[2]).map(roads::norm_deg) }
    }

    /// How much a road that keeps its height climbs to its end (m).
    pub fn typed_rise(&self) -> f64 {
        crate::draw::parse(&self.typed_text[3]).unwrap_or(0.0)
    }

    /// The road being drawn, as it would be built: length, radius of its last arc (0:
    /// straight), heading at its end, rise.
    pub fn road_plan_facts(&self) -> Option<(f64, f64, f64, f64)> {
        let (segs, z0, z1, _) = self.road_plan.as_ref()?;
        let last = segs.last()?;
        Some((roads::total_length(segs), last.radius, roads::norm_deg(last.end_heading()), z1 - z0))
    }

    /// Build the road drawn so far (Enter).
    pub fn confirm_road(&mut self, assets: &mut Assets, lib: &Library) {
        if self.tool == Tool::Objects && !self.plant_outline.is_empty() {
            self.finish_plant_outline(assets);
            return;
        }
        if self.tool == Tool::Terrain && !self.water_pts.is_empty() {
            self.make_water();
            return;
        }
        if self.acts_as() == Tool::Roads && self.road_start.is_some() {
            self.plan_road(assets);
            self.road_click(assets, lib);
        }
    }

    // ------------------------------------------------------------ roundabouts

    /// The ring's radius for the cursor now (None: no centre yet).
    fn ring_radius(&self) -> Option<(DVec3, f64)> {
        let c = self.ring_centre?;
        let r = (self.cursor?.truncate() - c.truncate()).length();
        // (nor a ring round half the world, the cursor looking at the horizon)
        (r * std::f64::consts::TAU <= crate::draw::MAX_LENGTH).then_some((c, r.max(crate::roundabout::MIN_RADIUS)))
    }

    /// The ring goes the way traffic goes round: anticlockwise where it keeps right, unless
    /// the road type's lanes run backwards.
    fn ring_clockwise(&self, assets: &mut Assets, rel: &str) -> bool {
        let left = self.doc.as_ref().map(|d| d.global.left_hand_traffic).unwrap_or(false);
        let backwards = assets.spline(rel).map(|m| {
            let streets: Vec<i32> = m.def.paths.iter().filter(|p| p.kind == 0).map(|p| p.direction).collect();
            !streets.is_empty() && streets.iter().all(|d| *d == 1)
        });
        left != backwards.unwrap_or(false)
    }

    fn ring_click(&mut self, assets: &mut Assets) {
        let Some(c) = self.cursor else { return };
        if self.ring_centre.is_none() {
            self.ring_centre = Some(c);
            return;
        }
        let Some((centre, r)) = self.ring_radius() else { return };
        self.ring_centre = None;
        self.build_roundabout(assets, centre, r);
    }

    /// Build a roundabout of the chosen road type: the ring, the roads inside it cut back
    /// to its edge, junctions where they meet it.
    pub fn build_roundabout(&mut self, assets: &mut Assets, centre: DVec3, r: f64) {
        let Some(rel) = self.road_type.clone() else {
            self.toast(tr("Pick a road type in the catalogue first", "Сначала выберите тип дороги в каталоге"));
            return;
        };
        let Some(model) = assets.spline(&rel) else { return };
        let two_way = {
            let dirs: Vec<i32> = model.def.paths.iter().filter(|p| p.kind == 0).map(|p| p.direction).collect();
            dirs.contains(&2) || (dirs.contains(&0) && dirs.contains(&1))
        };
        let half = model.half_width as f64;
        let clockwise = self.ring_clockwise(assets, &rel);
        let segs = crate::roundabout::ring(centre.truncate(), r, clockwise);
        let Some(doc) = self.doc.as_ref() else { return };
        let z = doc.ground(segs[0].start);
        // (the roads one sees that come into the circle: they are cut back to it)
        let mut inside: Vec<(TileKey, i64)> = Vec::new();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                if !assets.spline(&s.file).map(|m| m.joinable()).unwrap_or(false) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let n = (s.length / 2.0).ceil().max(2.0) as usize;
                if (0..=n).any(|i| (c.point_at(s.length * i as f64 / n as f64).truncate() - centre.truncate()).length() < r) {
                    inside.push((*k, s.id));
                }
            }
        }
        let Some(made) = self.build_road(&rel, &segs, z, z, Anchor::default(), None, half) else { return };
        let Some(doc) = self.doc.as_mut() else { return };
        // the ring closes on itself
        let (first, last) = (made[0], *made.last().unwrap());
        if let Some(s) = doc.tiles.get_mut(&first.0).and_then(|t| t.tile.splines.iter_mut().find(|s| s.id == first.1)) {
            s.prev_id = last.1;
        }
        if let Some(s) = doc.tiles.get_mut(&last.0).and_then(|t| t.tile.splines.iter_mut().find(|s| s.id == last.1)) {
            s.next_id = first.1;
        }
        // (cut a metre inside the ring's centre line, so each road still crosses it there)
        crate::roundabout::clear_inside(doc, &inside, centre.truncate(), r - 1.0);
        // openMapper's junctions inside go with the roads they joined
        crate::roundabout::remove_junctions_inside(doc, centre.truncate(), r - 1.0);
        for k in made.iter().map(|m| m.0).collect::<Vec<_>>() {
            doc.tiles.get_mut(&k).map(|t| t.touch_content());
        }
        let n = if self.auto_junctions { self.make_junctions(assets, &made, half, None, None) } else { 0 };
        self.history.commit();
        self.toast(format!("{} {n}", tr("Roundabout built; roads joined to it:", "Кольцо построено; дорог присоединено:")));
        if two_way {
            self.error(tr("This road type carries traffic both ways: pick a one-way road for a proper roundabout", "У этого типа дороги движение в обе стороны: для настоящего кольца выберите одностороннюю дорогу"));
        }
    }

    // ------------------------------------------------------------ bus pockets

    /// A bus pocket where the cursor is on a road: the stretch of the road, `pocket_len` m of
    /// full width and a taper each end, takes the pocket's road type; the width adapter lays
    /// the plates that taper between the types.
    pub fn pocket_click(&mut self, assets: &mut Assets) {
        let Some(c) = self.cursor else { return };
        let Some(doc) = self.doc.as_ref() else { return };
        let mut seen: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let mut wanted = |f: &str| *seen.entry(f.to_lowercase()).or_insert_with(|| assets.spline_wait(f, 1000).map(|m| m.joinable() && !m.is_track()).unwrap_or(false));
        let Some((k, id, at)) = crate::pocket::road_near(doc, c.truncate(), 8.0, &mut wanted) else {
            self.toast(tr("Press on a road", "Нажмите на дорогу"));
            return;
        };
        let Some((main_file, mirrored)) = doc.spline(k, id).map(|s| (s.file.clone(), s.mirror)) else { return };
        let left_hand = doc.global.left_hand_traffic;
        let Some(content) = self.content_dir.clone() else { return };
        if main_file.to_lowercase().contains("_bay_") {
            self.toast(tr("This stretch is already of the pocket's road type", "Этот участок уже того типа, что карман"));
            return;
        }
        // (the bay's road type: the road's own, widened along the kerb its traffic keeps to; the
        // other side is left as it is)
        let width = if self.pocket_width > 0.0 { self.pocket_width } else { 3.0 };
        let (cfg, _) = match crate::pocket::bay_type(assets, &content, &main_file, mirrored, left_hand, width) {
            Ok(v) => v,
            Err(e) => {
                self.error(format!("{}: {e}", tr("The bus pocket could not be made", "Не удалось сделать автобусный карман")));
                return;
            }
        };
        crate::hot::own_change();
        if assets.spline_wait(&cfg, 3000).is_none() {
            self.error(tr("The pocket's road type could not be read", "Не удалось прочитать тип дороги кармана"));
            return;
        }
        let Some(doc) = self.doc.as_ref() else { return };
        let step = width as f64;
        let ratio_in = self.pocket_taper_in.clamp(1.0, 12.0);
        let ratio_out = if self.pocket_alike { ratio_in } else { self.pocket_taper_out.clamp(1.0, 12.0) };
        // (a taper in metres is a plate that long: half of it is cut from each road)
        let (m_in, m_out) = (self.pocket_in_m.clamp(12.0, 60.0), if self.pocket_alike { self.pocket_in_m.clamp(12.0, 60.0) } else { self.pocket_out_m.clamp(12.0, 60.0) });
        let (back_in, back_out) = if self.pocket_metres { (m_in / 2.0, m_out / 2.0) } else { (taper_back_by(step.max(0.25), ratio_in), taper_back_by(step.max(0.25), ratio_out)) };
        // (the ratio the plates are laid with: the length asked for over the width changed)
        let (ratio_in, ratio_out) = if self.pocket_metres { (back_in / step.max(0.25), back_out / step.max(0.25)) } else { (ratio_in, ratio_out) };
        let len = self.pocket_len.clamp(4.0, 80.0);
        let stretch = len + back_in + back_out;
        let Some((start, total)) = crate::pocket::chain_position(doc, k, id) else { return };
        // (the road must go on for the taper on each side: a bay is never ended by a step)
        let (lo, hi) = (back_in + 1.0, total - back_out - 1.0 - stretch);
        let mut from = start + at - len / 2.0 - back_in;
        if hi < lo - 1e-6 {
            self.error(tr("The road is too short here for a bay with a taper at both ends", "Дорога здесь слишком короткая для кармана с переходом с обеих сторон"));
            return;
        }
        if from < lo || from > hi {
            if !self.pocket_fit {
                self.error(tr("Too near the end of the road for the taper at both ends of the bay", "Слишком близко к концу дороги для перехода с обеих сторон кармана"));
                return;
            }
            from = from.clamp(lo, hi);
        }
        let Some(doc) = self.doc.as_mut() else { return };
        self.history.begin(tr("Bus pocket", "Автобусный карман"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for key in keys {
            self.history.touch_tile(doc, key);
        }
        self.history.touch_global(doc);
        let result = crate::pocket::cut_out(doc, k, id, from, from + stretch).and_then(|cut| {
            let mut inner = crate::pocket::retype(doc, &cut, &cfg, 0.0)?;
            // (one piece where the stretch's pieces make one curve: the taper needs length)
            let mut i = 0;
            while i + 1 < inner.len() {
                if doc.join_splines(inner[i].0, inner[i].1).is_ok() {
                    inner.remove(i + 1);
                } else {
                    i += 1;
                }
            }
            Ok((cut.before, cut.after, inner))
        });
        match result {
            Ok((before, after, inner)) => {
                let (mut laid, mut wanted_plates) = (0, 0);
                if let (Some((bk, bid)), true) = (before, !inner.is_empty()) {
                    wanted_plates += 1;
                    laid += self.adapt_joints_by(assets, vec![(bk, bid, true)], ratio_in);
                }
                if let (Some((ak, aid)), true) = (after, !inner.is_empty()) {
                    wanted_plates += 1;
                    laid += self.adapt_joints_by(assets, vec![(ak, aid, false)], ratio_out);
                }
                if wanted_plates < 2 || laid < wanted_plates {
                    if let Some(d) = self.doc.as_mut() {
                        self.history.rollback(d);
                    }
                    self.error(tr("The bay needs a taper at both ends and one could not be laid there", "Карману нужен переход с обеих сторон, а один здесь уложить не удалось"));
                    return;
                }
                self.history.commit();
                self.set_selection(inner.into_iter().filter(|(k, id)| self.doc.as_ref().map(|d| d.spline(*k, *id).is_some()).unwrap_or(false)).map(|(k, id)| Sel::Spline(k, id)).collect());
                self.toast(tr("Bus pocket laid", "Автобусный карман создан"));
            }
            Err(e) => {
                if let Some(d) = self.doc.as_mut() {
                    self.history.rollback(d);
                }
                self.error(e);
            }
        }
    }

    // ------------------------------------------------------------ ruler

    /// The ruler on or off.
    pub fn toggle_ruler(&mut self) {
        self.ruler = if self.ruler.is_some() { None } else { Some((None, None)) };
    }

    /// What the ruler measures: from its first point to its second (or the cursor).
    pub fn ruler_line(&self) -> Option<(DVec3, DVec3)> {
        let (Some(a), b) = self.ruler? else { return None };
        Some((a, b.or(self.cursor)?))
    }

    // ------------------------------------------------------------ snapping

    /// A point to the grid, when the grid snaps.
    pub fn snap_point(&self, p: DVec2) -> DVec2 {
        if self.snap_grid && self.grid_step > 0.0 {
            (p / self.grid_step).round() * self.grid_step
        } else {
            p
        }
    }

    /// A heading to whole 15 degrees, when angles snap.
    pub fn snap_heading(&self, h: f64) -> f64 {
        if self.snap_angle { snap_deg(h, self.angle_step) } else { h }
    }

    // ------------------------------------------------------------ drawing

    /// The selection drawn over the view: splines' centre lines, a ring round each object
    /// selected with others, the grips of a spline selected alone, a reshape's new shape.
    fn selection_extras(&self, x: &mut FrameExtras, cam: &crate::camera::Camera, w: f32, assets: &mut Assets) {
        let Some(doc) = self.doc.as_ref() else { return };
        let sels = self.selection();
        // (the handles are about as long as the selection is big: read for the one thing it is
        // about, never for a selection of thousands, whose every record would be looked up
        // and its model found for a number then thrown away)
        let size = if sels.len() != 1 { f64::INFINITY } else { sels.iter().map(|s| match *s {
            Sel::Object(k, id) => doc.object(k, id).and_then(|o| assets.object(&o.file)).map(|m| m.radius() as f64 * 1.3).unwrap_or(f64::INFINITY),
            Sel::Attached(k, id) => doc.attachment(k, id).and_then(|a| assets.object(&a.file)).map(|m| m.radius() as f64 * 1.3).unwrap_or(f64::INFINITY),
            Sel::Spline(..) => f64::INFINITY,
        }).fold(0.0, f64::max) };
        self.gizmo_cap.set(if sels.len() == 1 { size.max(2.0) } else { f64::INFINITY });
        // (what is selected glows in the scene itself: no outline or ball is drawn over it)
        // what one object takes its state from (cyan) and what takes it from it
        if let (Some(Sel::Object(k, id)), 1) = (self.selected, sels.len()) {
            let at = |k: TileKey, o: &MapObject| (o.parent_id.is_none()).then(|| doc.object_world(k, o, false) + DVec3::Z * 0.5);
            if let Some(me) = doc.object(k, id).and_then(|o| at(k, o)) {
                let parent = doc.object(k, id).and_then(|o| o.var_parent).and_then(|p| selection::object_by_id(doc, p));
                let others = parent.into_iter().map(|p| (p, true)).chain(selection::var_children(doc, id).into_iter().map(|c| (c, false)));
                for ((pk, o), up) in others {
                    let Some(there) = at(pk, o) else { continue };
                    // (thin, and a dot as big as the linked thing, not as the view is wide)
                    let r = assets.object(&o.file).map(|m| m.radius() as f64 * 0.2).unwrap_or(0.5).clamp(0.15, 1.5) as f32;
                    line(&mut x.overlay, cam, me, there, (w * 0.3).min(r), LINK);
                    disc(&mut x.overlay, cam, there, r * if up { 1.4 } else { 1.0 }, LINK);
                }
            }
        }
        if self.acts_as() == Tool::Select {
            let eye = cam.eye();
            for (sp, h, p) in self.handles() {
                let on = (self.hover_handle == Some(h) && self.hover_handle_of == Some(sp)) || matches!(self.drag, Drag::Reshape(k, i, d) if d == h && (k, i) == sp);
                let picked = self.path_point == Some((sp, h));
                let (c, r) = self.grip(assets, sp, h, p, (p - eye).length());
                ball(&mut x.overlay_depth, c, r, if picked { ORANGE } else if on { GREEN } else { WHITE });
            }
            if let Some((sp, h)) = self.path_connect {
                if let Some(s) = doc.spline(sp.0, sp.1) {
                    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(sp.0));
                    let p = match h {
                        Handle::Start => c.point_at(0.0),
                        Handle::End => c.end_point(),
                        Handle::Mid => c.point_at(s.length * 0.5),
                    };
                    if let Some(cur) = self.cursor {
                        line(&mut x.overlay, cam, p + DVec3::Z * 0.3, cur + DVec3::Z * 0.3, w, ORANGE);
                    }
                }
            }
        }
        for plan in &self.carry_preview {
            for mut s in reshape::preview(doc, plan) {
                s.pos[2] += 0.03;
                let file = s.file.clone();
                x.ghost_splines.push((file, s, [0.35, 0.85, 0.45, 0.35]));
            }
        }
        match &self.reshape {
            Some(Ok(plan)) => {
                for s in reshape::preview(doc, plan) {
                    let mut s = s;
                    s.pos[2] += 0.03;
                    let file = s.file.clone();
                    x.ghost_splines.push((file, s, [0.35, 0.85, 0.45, 0.35]));
                }
            }
            Some(Err(_)) => {
                if let Some(c) = self.cursor {
                    disc(&mut x.overlay, cam, c, w * 3.0, [1.0, 0.3, 0.3, 0.9]);
                }
            }
            None => {}
        }
    }
}

/// A shaded ball (the light from above and a little aside), for the grips.
fn ball(out: &mut Vec<OverlayPoint>, c: DVec3, r: f64, color: [f32; 4]) {
    let (rows, cols) = (8, 14);
    let light = DVec3::new(0.35, -0.45, 0.82).normalize();
    let at = |i: usize, j: usize| {
        let (lat, lon) = (std::f64::consts::PI * i as f64 / rows as f64, std::f64::consts::TAU * j as f64 / cols as f64);
        let n = DVec3::new(lat.sin() * lon.cos(), lat.sin() * lon.sin(), lat.cos());
        let k = (0.5 + 0.5 * n.dot(light).max(0.0)) as f32;
        OverlayPoint { pos: c + n * r, color: [color[0] * k, color[1] * k, color[2] * k, 1.0] }
    };
    for i in 0..rows {
        for j in 0..cols {
            let (a, b, c2, d) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            out.extend([a, b, c2, b, d, c2]);
        }
    }
}

/// Join two road ends that were cut back for a junction that turned out to have only these
/// two (the others too short to keep): a smooth piece of the first one's road between them.
/// A new junction's options from its roads' styles: lights if any road asks for them, a
/// zebra on every arm if any road does.
fn style_options(doc: &Document, styles: &crate::road_maker::Styles, arms: &[crate::junction::Arm], centre: DVec3) -> crate::junction::Options {
    let st: Vec<crate::road_maker::JunctionStyle> = arms.iter().filter_map(|a| doc.spline(a.key, a.id)).map(|s| styles.get(&s.file)).collect();
    let mut o = crate::junction::Options { lights: st.iter().any(|s| s.lights), ..Default::default() };
    if st.iter().any(|s| s.crossings) {
        o.crossings = arm_ends_world(doc, arms).iter().map(|e| roads::heading_of(e.truncate() - centre.truncate())).collect();
    }
    o
}

/// A bus pocket's taper: metres cut back each side per metre the width changes (1:6 over both).
const POCKET_TAPER: f64 = 3.0;

/// How much wider (half width, m) the road of one arm is than the other's, if the roads
/// differ enough to need a taper between them.
fn width_step(doc: &Document, assets: &mut Assets, a: &crate::junction::Arm, b: &crate::junction::Arm) -> Option<f64> {
    // (the drawn extent either side of the road's middle, as the map sees it: a mirrored
    // spline is turned round)
    let extent = |assets: &mut Assets, arm: &crate::junction::Arm| -> Option<(f64, f64)> {
        let s = doc.spline(arm.key, arm.id)?;
        let m = assets.spline(&s.file)?;
        if !(m.joinable() && !m.is_track()) {
            return None;
        }
        let (lo, hi) = crate::pocket::extent(&m).unwrap_or((-m.half_width, m.half_width));
        let (lo, hi) = (lo as f64, hi as f64);
        Some(if s.mirror { (-hi, -lo) } else { (lo, hi) })
    };
    let (ea, eb) = (extent(assets, a)?, extent(assets, b)?);
    let step = (ea.0 - eb.0).abs().max((ea.1 - eb.1).abs());
    (step >= 0.25).then_some(step)
}

/// How far each of two roads of different width is cut back for the plate that tapers from
/// one to the other (a taper of 1:10 on each side).
fn taper_back(step: f64) -> f64 {
    taper_back_by(step, 5.0)
}

fn taper_back_by(step: f64, ratio: f64) -> f64 {
    (step * ratio).clamp(6.0, 30.0)
}

/// How far the road runs from its end `at_end` of spline `id` along its chain, up to a limit
/// far past any taper (m).
pub(crate) fn reach_along(doc: &Document, k: TileKey, id: i64, at_end: bool) -> f64 {
    let (mut cur, mut total) = ((k, id, at_end), 0.0);
    for _ in 0..8 {
        let Some(s) = doc.spline(cur.0, cur.1) else { break };
        total += s.length;
        if total > 60.0 {
            break;
        }
        // (on from the other end of this piece)
        let next = if cur.2 { s.prev_id } else { s.next_id };
        match doc.find_spline(next) {
            Some((nk, n)) => cur = (nk, n.id, n.next_id == cur.1),
            None => break,
        }
    }
    total
}

/// Join the two arms left of a junction: roads of different width get a plate that tapers
/// from the one to the other (as a junction of two roads), else a bend of the first's type.
/// Gives back whether the plate was laid (false: the ends are only joined by a bend).
fn join_two(doc: &mut Document, assets: &mut Assets, adapt: bool, follow: bool, content: &Path, asphalt: Option<&Path>, a: &crate::junction::Arm, b: &crate::junction::Arm) -> bool {
    if adapt && width_step(doc, assets, a, b).is_some() {
        let ends = arm_ends_world(doc, &[*a, *b]);
        if ends.len() == 2 {
            let centre = (ends[0] + ends[1]) * 0.5;
            match crate::junction::make_with(doc, assets, content, asphalt, centre, &[*a, *b], &crate::junction::Options::default()) {
                Ok(_) => {
                    if follow {
                        let reach = crate::junction::reach(doc, assets, centre, &[*a, *b]).max(4.0);
                        conform_disc(doc, centre, reach, &ends);
                    }
                    return true;
                }
                Err(e) => log::warn!("road adapter: {e}"),
            }
        }
    }
    join_arms(doc, a, b);
    false
}

fn join_arms(doc: &mut Document, a: &crate::junction::Arm, b: &crate::junction::Arm) {
    let end = |arm: &crate::junction::Arm| -> Option<(DVec3, f64, String, f64)> {
        let s = doc.spline(arm.key, arm.id)?;
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(arm.key));
        let at = if arm.at_end { s.length } else { 0.0 };
        // heading towards the gap, and the gradient going that way
        let h = if arm.at_end { c.heading_at(at) } else { c.heading_at(at) + 180.0 };
        let g = c.slope_at(at) * 100.0 * if arm.at_end { 1.0 } else { -1.0 };
        Some((c.point_at(at), roads::norm_deg(h), s.file.clone(), g))
    };
    let (Some((pa, ha, file, ga)), Some((pb, hb, _, gb))) = (end(a), end(b)) else { return };
    let Some(segs) = roads::biarc(pa.truncate(), ha, pb.truncate(), roads::norm_deg(hb + 180.0)).or_else(|| roads::straight(pa.truncate(), pb.truncate()).map(|s| vec![s])) else { return };
    let total: f64 = segs.iter().map(|s| s.length).sum::<f64>().max(0.01);
    let mut ids = Vec::new();
    let mut acc = 0.0;
    for sg in &segs {
        let z0 = pa.z + (pb.z - pa.z) * acc / total;
        acc += sg.length;
        let z1 = pa.z + (pb.z - pa.z) * acc / total;
        let k = tile_of(sg.start);
        if !doc.tiles.contains_key(&k) {
            return;
        }
        let id = doc.new_id();
        let org = tile_origin(k);
        let t0 = if ids.is_empty() { ga } else { (z1 - z0) / sg.length.max(0.01) * 100.0 };
        let t1 = if acc >= total - 1e-6 { -gb } else { (z1 - z0) / sg.length.max(0.01) * 100.0 };
        let sp = MapSpline {
            file: file.clone(),
            id,
            pos: [sg.start.x - org.x, sg.start.y - org.y, z0],
            heading: sg.heading,
            length: sg.length,
            radius: sg.radius,
            grad_start: t0,
            grad_end: t1,
            delta_h: Some(z1 - z0),
            is_h: true,
            ..Default::default()
        };
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.splines.push(sp);
        t.touch_content();
        ids.push((k, id));
    }
    // link the chain: a's end -> new pieces -> b's end
    let set = |doc: &mut Document, k: TileKey, id: i64, at_end: bool, other: i64| {
        if let Some(t) = doc.tiles.get_mut(&k) {
            if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id) {
                if at_end { s.next_id = other } else { s.prev_id = other }
            }
            t.touch_content();
        }
    };
    for (i, (k, id)) in ids.iter().enumerate() {
        let prev = if i == 0 { a.id } else { ids[i - 1].1 };
        let next = if i + 1 == ids.len() { b.id } else { ids[i + 1].1 };
        set(doc, *k, *id, false, prev);
        set(doc, *k, *id, true, next);
    }
    set(doc, a.key, a.id, a.at_end, ids[0].1);
    set(doc, b.key, b.id, b.at_end, ids[ids.len() - 1].1);
}

/// Keep the entry points' records in step with their objects.
/// The angles that tilt an object facing `heading` at `at` to the ground under it: its
/// forward and right axes, as the view turns them, climbing as the ground climbs along them
/// (sampled 2 m either side; the signs found by trying, so they follow OMSI's convention).
pub fn lean_rotation(heading: f64, ground: &dyn Fn(DVec2) -> f64, at: DVec2) -> [f64; 3] {
    let axes = |rot: [f64; 3]| {
        let m = object_matrix(DVec3::ZERO, rot);
        (m.transform_vector3(glam::Vec3::Y).as_dvec3(), m.transform_vector3(glam::Vec3::X).as_dvec3())
    };
    let (f0, r0) = axes([heading, 0.0, 0.0]);
    let grade = |d: DVec3| {
        let d = d.truncate().normalize_or_zero();
        (ground(at + d * 2.0) - ground(at - d * 2.0)) / 4.0
    };
    let (gf, gr) = (grade(f0), grade(r0));
    let (a, b) = (gf.atan().to_degrees(), gr.atan().to_degrees());
    let rise = |v: DVec3| v.z / v.truncate().length().max(1e-9);
    [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)]
        .into_iter()
        .map(|(s1, s2)| [heading, s1 * a, s2 * b])
        .min_by(|x, y| {
            let err = |rot: [f64; 3]| {
                let (f, r) = axes(rot);
                (rise(f) - gf).abs() + (rise(r) - gr).abs()
            };
            err(*x).total_cmp(&err(*y))
        })
        .unwrap()
}

pub fn sync_entry_points(doc: &mut Document) {
    // (nothing names an entry point: the walk of the whole map is not worth making, and a move
    // of a big selection makes it every frame)
    if doc.global.entry_points.is_empty() {
        doc.global_dirty = true;
        return;
    }
    // (each IDCode's place and tile read once: a map of a hundred thousand objects walked for
    // every entry point is what a grab of them answered with)
    let mut at: std::collections::HashMap<i64, (TileKey, usize)> = std::collections::HashMap::new();
    for (k, t) in &doc.tiles {
        for (i, o) in t.tile.objects.iter().enumerate() {
            at.insert(o.id, (*k, i));
        }
    }
    let groups: std::collections::HashMap<TileKey, i32> = doc.order.iter().enumerate().map(|(i, k)| (*k, i as i32)).collect();
    let mut eps = std::mem::take(&mut doc.global.entry_points);
    for e in &mut eps {
        let Some((k, i)) = at.get(&e.object_id).copied() else { continue };
        let Some(o) = doc.tiles.get(&k).and_then(|t| t.tile.objects.get(i)) else { continue };
        e.index = i as i32;
        e.pos = [o.pos[0], o.pos[1], o.pos[2]];
        let h = (o.rot[0] / 2.0).to_radians();
        e.quat = [0.0, h.sin(), 0.0, h.cos()];
        e.group = groups.get(&k).copied().unwrap_or(0);
    }
    doc.global.entry_points = eps;
    doc.global_dirty = true;
}

/// The tiles a sample of the map-wide grid (tile × 60 + i) belongs to along one axis.
fn owners(g: i64, n: i64) -> Vec<i64> {
    if g.rem_euclid(n) == 0 { vec![g.div_euclid(n), g.div_euclid(n) - 1] } else { vec![g.div_euclid(n)] }
}

/// A terrain sample on the map-wide grid.
pub(crate) fn world_sample(doc: &Document, gx: i64, gy: i64, n: i64) -> Option<f32> {
    for tx in owners(gx, n) {
        for ty in owners(gy, n) {
            if let Some(t) = doc.tiles.get(&(tx as i32, ty as i32)) {
                let (i, j) = ((gx - tx * n) as usize, (gy - ty * n) as usize);
                return Some(t.terrain.height_at(i, j));
            }
        }
    }
    None
}

/// The ground under an outline (`lo`..`hi` in the frame of something at `centre` turned
/// `heading`) made `level`, blending into the ground about over 3 m.
fn level_ground(doc: &mut Document, hist: &mut History, centre: DVec2, heading: f64, (lo, hi): (Vec3, Vec3), level: f64) {
    const MARGIN: f64 = 3.0;
    let cell = tile_size() / (TERRAIN_SAMPLES - 1) as f64;
    let n = (TERRAIN_SAMPLES - 1) as i64;
    let inv = object_matrix(DVec3::ZERO, [heading, 0.0, 0.0]).inverse();
    let r = ((hi.x - lo.x).hypot(hi.y - lo.y) as f64) * 0.5 + MARGIN + DVec2::new(((lo.x + hi.x) * 0.5) as f64, ((lo.y + hi.y) * 0.5) as f64).length();
    let (gx0, gx1) = (((centre.x - r) / cell).floor() as i64, ((centre.x + r) / cell).ceil() as i64);
    let (gy0, gy1) = (((centre.y - r) / cell).floor() as i64, ((centre.y + r) / cell).ceil() as i64);
    for gy in gy0..=gy1 {
        for gx in gx0..=gx1 {
            let p = DVec2::new(gx as f64 * cell, gy as f64 * cell) - centre;
            let l = inv.transform_point3(Vec3::new(p.x as f32, p.y as f32, 0.0));
            // how far outside the outline (0 inside)
            let dx = (lo.x - l.x).max(l.x - hi.x).max(0.0) as f64;
            let dy = (lo.y - l.y).max(l.y - hi.y).max(0.0) as f64;
            let out = dx.hypot(dy);
            if out >= MARGIN {
                continue;
            }
            let Some(h) = world_sample(doc, gx, gy, n) else { continue };
            let t = out / MARGIN;
            let w = 1.0 - t * t * (3.0 - 2.0 * t);
            set_world_sample(doc, hist, gx, gy, n, (h as f64 + (level - h as f64) * w) as f32);
        }
    }
}

/// Set a sample of the map-wide grid in every tile that has it (edges are shared).
pub(crate) fn set_world_sample(doc: &mut Document, hist: &mut History, gx: i64, gy: i64, n: i64, h: f32) {
    let txs = owners(gx, n);
    let tys = owners(gy, n);
    for &tx in &txs {
        for &ty in &tys {
            let key = (tx as i32, ty as i32);
            if !doc.tiles.contains_key(&key) {
                continue;
            }
            hist.touch_tile(doc, key);
            let t = doc.tiles.get_mut(&key).unwrap();
            let i = (gx - tx * n) as usize;
            let j = (gy - ty * n) as usize;
            let s = TERRAIN_SAMPLES;
            if i < s && j < s {
                t.terrain.heights[j * s + i] = h;
                t.touch_terrain();
            }
        }
    }
}

/// The heights of a planned road at the joints of its pieces, (height, gradient %): the
/// ground's, smoothed along the road (so it climbs hills instead of cutting through them),
/// from `z0` at the start to `z1` at the end, no steeper than 10 %.
/// An even climb from `z0` to `z1` (a road that keeps its height: a bridge, an embankment).
fn even_profile(pieces: &[Seg], z0: f64, z1: f64) -> Vec<(f64, f64)> {
    let total = roads::total_length(pieces).max(0.1);
    let g = (z1 - z0) / total * 100.0;
    let mut acc = 0.0;
    let mut out = vec![(z0, g)];
    for p in pieces {
        acc += p.length;
        out.push((z0 + (z1 - z0) * acc / total, g));
    }
    out
}

fn road_profile(doc: &Document, pieces: &[Seg], z0: f64, z1: f64) -> Vec<(f64, f64)> {
    let mut nodes = vec![0.0];
    for p in pieces {
        nodes.push(nodes.last().unwrap() + p.length);
    }
    let total = *nodes.last().unwrap();
    // the ground every 4 m along the road
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let mut acc = 0.0;
    for p in pieces {
        let c = p.curve();
        let n = (p.length / 4.0).ceil().max(1.0) as usize;
        for i in 0..n {
            let s = p.length * i as f64 / n as f64;
            samples.push((acc + s, doc.ground(c.point_at(s).truncate())));
        }
        acc += p.length;
    }
    let window = 20.0;
    let mut z: Vec<f64> = nodes
        .iter()
        .map(|&s| {
            let near: Vec<f64> = samples.iter().filter(|q| (q.0 - s).abs() <= window).map(|q| q.1).collect();
            if near.is_empty() { doc.ground(DVec2::ZERO) } else { near.iter().sum::<f64>() / near.len() as f64 }
        })
        .collect();
    let last = z.len() - 1;
    z[0] = z0;
    z[last] = z1;
    // no steeper than 10 % (a few passes both ways)
    for _ in 0..4 {
        for i in 1..=last {
            let d = (nodes[i] - nodes[i - 1]).max(0.1) * 0.10;
            if i != last {
                z[i] = z[i].clamp(z[i - 1] - d, z[i - 1] + d);
            }
        }
        for i in (0..last).rev() {
            let d = (nodes[i + 1] - nodes[i]).max(0.1) * 0.10;
            if i != 0 {
                z[i] = z[i].clamp(z[i + 1] - d, z[i + 1] + d);
            }
        }
    }
    let _ = total;
    (0..=last)
        .map(|i| {
            let (a, b) = (i.saturating_sub(1), (i + 1).min(last));
            let g = if b > a { (z[b] - z[a]) / (nodes[b] - nodes[a]).max(0.1) * 100.0 } else { 0.0 };
            (z[i], g)
        })
        .collect()
}

/// Piece `i` of a planned road as a `[spline_h]` (its height a smooth cubic between the
/// joints' heights and gradients), tile-local to `org`.
/// Bring a track that goes on beyond a switch round to the switch's straight heel: where
/// it bent away under the (straight) switch it is cut back a little more and joined to the
/// heel by a pair of arcs, tangent at both ends. `end`: the track's spline and whether its
/// end (else its start) is where it stops; `heel`: the heel's place and heading out.
fn rejoin_track(doc: &mut Document, end: (TileKey, i64, bool), heel: (DVec3, f64)) {
    let at = |doc: &Document, e: (TileKey, i64, bool)| -> Option<(DVec3, f64, String)> {
        let s = doc.spline(e.0, e.1)?;
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(e.0));
        // (the heading on along the track, away from the switch)
        Some(if e.2 { (c.end_point(), roads::norm_deg(c.heading_at(s.length) + 180.0), s.file.clone()) } else { (c.point_at(0.0), s.heading, s.file.clone()) })
    };
    let Some((p, h, _)) = at(doc, end) else { return };
    let off = (p.truncate() - heel.0.truncate()).length();
    let turn = (roads::norm_deg(h - heel.1 + 180.0) - 180.0).abs();
    if off < 0.05 && turn < 0.2 {
        return;
    }
    // (room for the arcs to bring it round as gently as a track bends, some 300 m: an S of
    // that radius moves sideways L² / 4R over its length L, and turns L / R)
    const R: f64 = 300.0;
    let room = (4.0 * R * off).sqrt() + R * turn.to_radians();
    let Some(end) = doc.trim(end.0, end.1, end.2, room.clamp(10.0, 150.0)) else { return };
    let Some((q, hq, file)) = at(doc, end) else { return };
    let Some(segs) = roads::biarc(heel.0.truncate(), heel.1, q.truncate(), hq) else { return };
    let pieces = roads::split(&segs, 50.0);
    let profile = even_profile(&pieces, heel.0.z.max(q.z.min(heel.0.z + 5.0)), q.z);
    let ids: Vec<i64> = pieces.iter().map(|_| doc.new_id()).collect();
    for (i, piece) in pieces.iter().enumerate() {
        let k = tile_of(piece.start);
        let Some(t) = doc.tiles.get_mut(&k) else { continue };
        let mut sp = piece_spline(&file, piece, &profile, i, tile_origin(k));
        sp.id = ids[i];
        sp.prev_id = if i > 0 { ids[i - 1] } else { 0 };
        sp.next_id = if i + 1 < ids.len() { ids[i + 1] } else { 0 };
        t.tile.splines.push(sp);
        t.touch_content();
    }
    // the last piece runs on into the track
    let last = *ids.last().unwrap();
    if let Some((k, s)) = doc.find_spline(end.1).map(|(k, s)| (k, s.id)) {
        if let Some(t) = doc.tiles.get_mut(&k) {
            if let Some(sp) = t.tile.splines.iter_mut().find(|x| x.id == s) {
                if end.2 { sp.next_id = last } else { sp.prev_id = last }
            }
            t.touch_content();
        }
        if let Some(pk) = doc.find_spline(last).map(|(k, _)| k) {
            if let Some(sp) = doc.tiles.get_mut(&pk).and_then(|t| t.tile.splines.iter_mut().find(|x| x.id == last)) {
                sp.next_id = s;
            }
        }
    }
}

/// Every piece of the path that holds `id`, from its free start along to its free end.
fn path_chain(doc: &Document, id: i64) -> Vec<(TileKey, i64)> {
    let mut head = id;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..5000 {
        if !seen.insert(head) {
            break;
        }
        let Some((_, s)) = doc.find_spline(head) else { break };
        if s.prev_id == 0 || s.prev_id == head {
            break;
        }
        let Some((_, p)) = doc.find_spline(s.prev_id) else { break };
        if p.next_id != head && p.prev_id != head {
            break;
        }
        head = s.prev_id;
    }
    let mut out = Vec::new();
    let mut id = head;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..5000 {
        if !seen.insert(id) {
            break;
        }
        let Some((k, s)) = doc.find_spline(id) else { break };
        let next = s.next_id;
        out.push((k, id));
        if next == 0 || next == id {
            break;
        }
        id = next;
    }
    out
}

fn chain_has_rows(doc: &Document, chain: &[(TileKey, i64)]) -> bool {
    chain.iter().any(|(k, id)| {
        let Some(t) = doc.tiles.get(k) else { return false };
        let Some(idx) = t.tile.splines.iter().position(|s| s.id == *id && !s.deleted) else { return false };
        t.tile.spline_attachments.iter().any(|a| a.spline_index == idx as i32)
    })
}

/// Each piece turned round, so the path runs the other way and its links stay a chain.
fn reverse_path(doc: &mut Document, chain: &[(TileKey, i64)]) {
    let snaps: Vec<(TileKey, MapSpline)> = chain
        .iter()
        .filter_map(|(k, id)| {
            let s = doc.spline(*k, *id)?;
            Some((*k, reshape::reversed(s, tile_origin(*k), s.tex_offset)))
        })
        .collect();
    for (k, s) in snaps {
        if let Some(t) = doc.tiles.get_mut(&k) {
            if let Some(x) = t.tile.splines.iter_mut().find(|x| x.id == s.id && !x.deleted) {
                *x = s;
            }
            t.touch_content();
        }
    }
}

fn set_path_link(doc: &mut Document, k: TileKey, id: i64, next: Option<i64>, prev: Option<i64>) {
    if let Some(t) = doc.tiles.get_mut(&k) {
        if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == id && !s.deleted) {
            if let Some(n) = next {
                s.next_id = n;
            }
            if let Some(p) = prev {
                s.prev_id = p;
            }
        }
        t.touch_content();
    }
}

fn piece_spline(rel: &str, p: &Seg, profile: &[(f64, f64)], i: usize, org: DVec2) -> MapSpline {
    let (za, ga) = profile[i];
    let (zb, gb) = profile[i + 1];
    MapSpline {
        file: rel.to_string(),
        pos: [p.start.x - org.x, p.start.y - org.y, za],
        heading: p.heading,
        length: p.length,
        radius: p.radius,
        grad_start: ga,
        grad_end: gb,
        delta_h: Some(zb - za),
        is_h: true,
        ..Default::default()
    }
}

/// Bring the ground to a road: flat under it, blending back to the old ground beside it.
/// The whole road at once (each ground point follows the road's nearest point), so the
/// blend beside one piece never lifts the ground back over the piece next to it.
fn conform_ground(doc: &mut Document, curves: &[omsi_geometry::SplineCurve], half: f64) {
    let cell = tile_size() / (TERRAIN_SAMPLES - 1) as f64;
    let n = (TERRAIN_SAMPLES - 1) as i64;
    let core = half + 0.5;
    let blend = 8.0;
    // (the ground is drawn as triangles between its samples: a sample one cell beside the
    // road, left high on a slope, lifts the triangle it shares with one under the road up
    // through the road's edge. Within a cell's diagonal of the edge the ground is only ever
    // taken down to the road)
    let below = half + cell * std::f64::consts::SQRT_2 + 0.1;
    let reach = (core + blend).max(below);
    let mut pts: Vec<(DVec2, f64)> = Vec::new();
    for c in curves {
        let steps = (c.length / 1.0).ceil().max(2.0) as usize;
        for i in 0..=steps {
            let q = c.point_at(c.length * i as f64 / steps as f64);
            pts.push((q.truncate(), q.z));
        }
    }
    if pts.is_empty() {
        return;
    }
    let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for (q, _) in &pts {
        lo = lo.min(*q);
        hi = hi.max(*q);
    }
    lo -= DVec2::splat(reach);
    hi += DVec2::splat(reach);
    let mut dummy = History::default();
    for gy in (lo.y / cell).floor() as i64..=(hi.y / cell).ceil() as i64 {
        for gx in (lo.x / cell).floor() as i64..=(hi.x / cell).ceil() as i64 {
            let q = DVec2::new(gx as f64 * cell, gy as f64 * cell);
            let (d, z) = pts.iter().map(|(a, z)| ((*a - q).length(), *z)).fold((f64::MAX, 0.0), |b, x| if x.0 < b.0 { x } else { b });
            if d > reach {
                continue;
            }
            let Some(old) = world_sample(doc, gx, gy, n) else { continue };
            let target = z - 0.12;
            let w = if d <= core { 1.0 } else { 1.0 - ((d - core) / blend).clamp(0.0, 1.0) };
            let w = w * w * (3.0 - 2.0 * w);
            let h = old as f64 + (target - old as f64) * w;
            let h = if d <= below { h.min(target) } else { h };
            set_world_sample(doc, &mut dummy, gx, gy, n, h as f32);
        }
    }
}

/// Points along the visible roads within `within` of `c`, every metre: (point, height,
/// half width).
fn road_points_near(doc: &Document, assets: &mut Assets, c: DVec2, within: f64) -> Vec<(DVec2, f64, f64)> {
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            let start = DVec2::new(org.x + s.pos[0], org.y + s.pos[1]);
            if (start - c).length() > s.length + within + 20.0 {
                continue;
            }
            let Some(m) = assets.spline(&s.file) else { continue };
            if !m.joinable() {
                continue;
            }
            let hw = m.half_width as f64;
            let curve = omsi_geometry::SplineCurve::from_map(s, org);
            let steps = s.length.ceil().max(1.0) as usize;
            for i in 0..=steps {
                let q = curve.point_at(s.length * i as f64 / steps as f64);
                if (q.truncate() - c).length() <= within + hw {
                    out.push((q.truncate(), q.z, hw));
                }
            }
        }
    }
    out
}

/// The highest the ground sample at `p` may lie without its triangles showing through a
/// road beside it (see `conform_ground`), if a road is that near.
fn ground_cap(roads: &[(DVec2, f64, f64)], p: DVec2, cell: f64) -> Option<f64> {
    roads
        .iter()
        .filter(|(q, _, hw)| (*q - p).length() <= hw + cell * std::f64::consts::SQRT_2 + 0.1)
        .map(|(_, z, _)| z - 0.12)
        .min_by(|a, b| a.total_cmp(b))
}

/// Where the arms of a junction end (world, with their heights).
fn arm_ends_world(doc: &Document, arms: &[crate::junction::Arm]) -> Vec<DVec3> {
    arms.iter()
        .filter_map(|a| {
            let s = doc.spline(a.key, a.id)?;
            let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(a.key));
            Some(if a.at_end { c.end_point() } else { c.point_at(0.0) })
        })
        .collect()
}

/// Bring the ground under a junction to its plate: within `r` of the centre onto the plane
/// through its arms' ends (a junction on a hill slopes with its roads), blending back to
/// the old ground over the next 8 m.
pub(crate) fn conform_disc(doc: &mut Document, centre: DVec3, r: f64, ends: &[DVec3]) {
    // the plane z = a + b x + c y through the ends (least squares; level at the centre's
    // height if they say nothing)
    let plane = {
        let n = ends.len() as f64;
        let (mx, my, mz) = (ends.iter().map(|p| p.x - centre.x).sum::<f64>() / n.max(1.0), ends.iter().map(|p| p.y - centre.y).sum::<f64>() / n.max(1.0), ends.iter().map(|p| p.z).sum::<f64>() / n.max(1.0));
        let (mut sxx, mut sxy, mut syy, mut sxz, mut syz) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for p in ends {
            let (x, y, z) = (p.x - centre.x - mx, p.y - centre.y - my, p.z - mz);
            sxx += x * x;
            sxy += x * y;
            syy += y * y;
            sxz += x * z;
            syz += y * z;
        }
        let det = sxx * syy - sxy * sxy;
        let (b, c) = if ends.len() >= 3 && det.abs() > 1e-6 { ((sxz * syy - syz * sxy) / det, (syz * sxx - sxz * sxy) / det) } else { (0.0, 0.0) };
        let mz = if ends.is_empty() { centre.z } else { mz };
        move |q: DVec2| mz + b * (q.x - centre.x - mx) + c * (q.y - centre.y - my)
    };
    let cell = tile_size() / (TERRAIN_SAMPLES - 1) as f64;
    let n = (TERRAIN_SAMPLES - 1) as i64;
    let blend = 8.0;
    let reach = r + blend;
    let c = centre.truncate();
    let mut dummy = History::default();
    for gy in ((c.y - reach) / cell).floor() as i64..=((c.y + reach) / cell).ceil() as i64 {
        for gx in ((c.x - reach) / cell).floor() as i64..=((c.x + reach) / cell).ceil() as i64 {
            let q = DVec2::new(gx as f64 * cell, gy as f64 * cell);
            let d = (q - c).length();
            if d > reach {
                continue;
            }
            let Some(old) = world_sample(doc, gx, gy, n) else { continue };
            let w = if d <= r { 1.0 } else { 1.0 - ((d - r) / blend).clamp(0.0, 1.0) };
            let w = w * w * (3.0 - 2.0 * w);
            let z = plane(q) - 0.12;
            let h = old as f64 + (z - old as f64) * w;
            set_world_sample(doc, &mut dummy, gx, gy, n, h as f32);
        }
    }
}

/// Set a selected thing's height (an object's above its ground, or absolute; a road's
/// start), found by IDCode wherever it is now.
fn set_height(doc: &mut Document, hist: &mut History, s: Sel, z: f64) {
    match s {
        Sel::Object(_, id) => {
            let Some((k, _)) = selection::object_by_id(doc, id) else { return };
            hist.touch_tile(doc, k);
            if let Some(o) = doc.object_mut(k, id) {
                o.pos[2] = z;
            }
            if let Some(t) = doc.tiles.get_mut(&k) {
                t.touch_content();
            }
        }
        Sel::Spline(_, id) => {
            let Some((k, _)) = doc.find_spline(id) else { return };
            selection::touch_spline(hist, doc, k, id);
            if let Some(t) = doc.tiles.get_mut(&k) {
                if let Some(sp) = t.tile.splines.iter_mut().find(|s| s.id == id) {
                    sp.pos[2] = z;
                }
                t.touch_content();
            }
        }
        Sel::Attached(k, id) => {
            hist.touch_tile(doc, k);
            if let Some(a) = doc.attachment_mut(k, id) {
                a.offset[1] = z;
            }
            if let Some(t) = doc.tiles.get_mut(&k) {
                t.touch_content();
            }
        }
    }
}

/// Put an object (by IDCode) at a world point, turned so: its height is kept above the
/// ground there unless its type has an absolute one.
fn put_object_3d(doc: &mut Document, hist: &mut History, id: i64, p: DVec3, rot: [f64; 3], known: &std::collections::HashMap<String, (bool, Vec<[f32; 4]>)>) -> Option<Sel> {
    let (cur, o) = selection::object_by_id(doc, id)?;
    let abs = known.get(&o.file.to_lowercase()).map(|h| h.0).unwrap_or(false);
    hist.touch_tile(doc, cur);
    let target = tile_of(p.truncate());
    if doc.tiles.contains_key(&target) {
        hist.touch_tile(doc, target);
    }
    let placed = selection::place_object_at(doc, cur, id, p.truncate()).unwrap_or(Sel::Object(cur, id));
    let Sel::Object(k, _) = placed else { return None };
    let z = if abs { p.z } else { p.z - doc.ground(p.truncate()) };
    if let Some(ob) = doc.object_mut(k, id) {
        ob.pos[2] = z;
        ob.rot = rot;
    }
    if let Some(t) = doc.tiles.get_mut(&k) {
        t.touch_content();
    }
    Some(placed)
}

/// A line drawn as a flat ribbon facing the camera.
pub fn line(out: &mut Vec<OverlayPoint>, cam: &crate::camera::Camera, a: DVec3, b: DVec3, width: f32, color: [f32; 4]) {
    let eye = cam.eye();
    let dir = (b - a).normalize_or_zero();
    let to_eye = (eye - a).normalize_or_zero();
    let side = dir.cross(to_eye).normalize_or_zero() * (width as f64 * 0.5);
    let v = |p: DVec3| OverlayPoint { pos: p, color };
    out.extend([v(a - side), v(a + side), v(b + side), v(a - side), v(b + side), v(b - side)]);
}

/// A flat disc on the ground.
pub fn disc(out: &mut Vec<OverlayPoint>, _cam: &crate::camera::Camera, c: DVec3, r: f32, color: [f32; 4]) {
    let n = 20;
    let c = c + DVec3::Z * 0.15;
    let v = |p: DVec3| OverlayPoint { pos: p, color };
    for i in 0..n {
        let a0 = i as f64 / n as f64 * std::f64::consts::TAU;
        let a1 = (i + 1) as f64 / n as f64 * std::f64::consts::TAU;
        let p0 = c + DVec3::new(a0.cos(), a0.sin(), 0.0) * r as f64;
        let p1 = c + DVec3::new(a1.cos(), a1.sin(), 0.0) * r as f64;
        out.extend([v(c), v(p0), v(p1)]);
    }
}

/// An asset's default things to place when it is chosen.
pub fn asset_hint(a: &Asset) -> &'static str {
    if a.is_entry_point {
        tr("Start point: where the bus appears in openOMSI", "Точка старта: здесь автобус появляется в openOMSI")
    } else if a.is_bus_stop {
        tr("Bus stop: name it in the panel on the right", "Остановка: задайте имя в панели справа")
    } else {
        ""
    }
}

/// Rolls of the natural ground at `p`: smooth value noise of two sizes (about 40 m and
/// 12 m across), between -1 and 1.
fn rolls(p: DVec2) -> f64 {
    let hash = |x: i64, y: i64| {
        let mut h = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        h ^= h >> 31;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        (h % 20_001) as f64 / 10_000.0 - 1.0
    };
    let noise = |q: DVec2| {
        let (x0, y0) = (q.x.floor(), q.y.floor());
        let (fx, fy) = (q.x - x0, q.y - y0);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let (ix, iy) = (x0 as i64, y0 as i64);
        let a = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * sx;
        let b = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * sx;
        a + (b - a) * sy
    };
    noise(p / 40.0) * 0.7 + noise(p / 12.0 + DVec2::splat(17.3)) * 0.3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_object, add_spline, flat_map};

    #[test]
    fn two_path_ends_join_and_a_gap_gets_a_piece() {
        let spline = |ed: &Editor, id: i64| ed.doc.as_ref().unwrap().find_spline(id).unwrap().1.clone();
        let open = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
            let mut doc = flat_map(1);
            add_spline(&mut doc, (0, 0), 1, a.0, a.1, a.2, a.3, 0, 0);
            add_spline(&mut doc, (0, 0), 2, b.0, b.1, b.2, b.3, 0, 0);
            let mut ed = Editor::default();
            ed.open(doc);
            ed
        };
        // ends a hand's width apart: joined, and no extra piece
        let mut ed = open((20.0, 20.0, 0.0, 20.0), (20.0, 40.4, 0.0, 20.0));
        ed.connect_path_points((((0, 0), 1), Handle::End), (((0, 0), 2), Handle::Start));
        assert_eq!(spline(&ed, 1).next_id, 2, "the end runs on into the other path");
        assert_eq!(spline(&ed, 2).prev_id, 1);
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).count(), 2);
        ed.undo();
        assert_eq!(spline(&ed, 1).next_id, 0, "undo takes the join away");
        // twenty metres of gap: a piece is laid between them
        let mut ed = open((20.0, 20.0, 0.0, 20.0), (20.0, 60.0, 0.0, 20.0));
        ed.connect_path_points((((0, 0), 1), Handle::End), (((0, 0), 2), Handle::Start));
        let n = ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).count();
        assert!(n >= 3, "the gap is a piece");
        let mut id = spline(&ed, 1).next_id;
        let mut steps = 0;
        while id != 2 && id != 0 && steps < 6 {
            id = spline(&ed, id).next_id;
            steps += 1;
        }
        assert_eq!(id, 2, "the new piece runs on into the other path");
        assert!(steps >= 1);
        // the other path arrives at this one's end, facing the other way: it is turned round
        let mut ed = open((20.0, 20.0, 0.0, 20.0), (20.0, 60.0, 180.0, 20.0));
        ed.connect_path_points((((0, 0), 1), Handle::End), (((0, 0), 2), Handle::End));
        assert_eq!(spline(&ed, 1).next_id, 2);
        assert_eq!(spline(&ed, 2).prev_id, 1);
        let b = spline(&ed, 2);
        assert!((b.pos[1] - 40.0).abs() < 0.5, "it now starts where the first ends, at y {:.1}", b.pos[1]);
    }

    /// A click gives what the camera is looking at.
    ///
    /// Splines used to be ranked by their place on the ground alone: a bridge's deck and the
    /// road under it, a fence and the railing beside it, a line and the road under that share a
    /// place, so whichever came first in the map's order was picked and the other could not be
    /// chosen at all. On a map of 72,000 splines that was most of a city's bridges and railings.
    ///
    /// The ray now takes the surface it meets first. Checked by looking straight down at every
    /// fiftieth spline of a real map's two hundred tiles: nothing comes back at all for none of
    /// them, and nothing comes back that lies under the spline aimed at.
    #[test]
    fn a_click_takes_the_surface_the_ray_meets_first() {
        // (the map's grid is global: this holds it while it reads a map, as the other tests do)
        let _map = crate::doc::one_map();
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2 installation: skipped");
            return;
        };
        // (the map the plan measured is kept beside the sources, in `world`, when the
        // installation has none of that name)
        let mut cfg = omsi.join("maps").join("NT_merged_v2").join("global.cfg");
        if !cfg.is_file() {
            cfg = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("world").join("NT_merged_v2").join("global.cfg");
        }
        if !cfg.is_file() {
            eprintln!("no NT_merged_v2: skipped");
            return;
        }
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        // every record of every attachment must answer to a selection of its own and have a first
        // copy to stand at (the record is the object, not the spline it hangs on)
        // every record that places a copy must be an object of its own, with a place and a
        // selection (a record whose row hangs on another tile, or whose road is gone, places
        // nothing here: its copies are the neighbouring record's, or it is invisible)
        let (mut records, mut placed) = (0, 0);
        let mut lost: Vec<String> = Vec::new();
        for (k, t) in &doc.tiles {
            for a in &t.tile.spline_attachments {
                records += 1;
                if selection::attachment_copies(&doc, *k, a.id).is_empty() {
                    continue;
                }
                placed += 1;
                match selection::attachment_view(&doc, *k, a.id) {
                    Some((..)) if selection::origin_of(&doc, Sel::Attached(*k, a.id)).is_some() => {}
                    _ => lost.push(format!("{} on {k:?}", a.id)),
                }
            }
        }
        eprintln!("{placed} of {records} records stand in the map and can be picked");
        assert!(records > 1000, "the map has {records} records");
        assert!(placed * 100 > records * 90, "most records stand in the map: {placed} of {records}");
        assert!(lost.is_empty(), "{} of {placed} cannot be picked: {:?}", lost.len(), lost);
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        // (an AI path shows only as lines, and is picked only while those are shown: here they all are)
        ed.show_lanes = true;
        let doc = ed.doc.as_ref().unwrap();

        let (mut seen, mut nothing, mut behind) = (0, 0, 0);
        let mut report: Vec<String> = Vec::new();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                // (a sample: each pick walks every tile's splines, so all 72,000 would take it too long)
                let sample = seen % 50 == 0;
                seen += 1;
                if !sample {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let mid = c.point_at(s.length * 0.5);
                let from = DVec3::new(mid.x, mid.y, mid.z + 60.0);
                // (straight down: the nearest surface is the highest one there)
                let got = ed.pick(&mut assets, from, -DVec3::Z, Some(DVec3::new(mid.x, mid.y, mid.z)));
                match got {
                    None => {
                        nothing += 1;
                        if report.len() < 5 {
                            report.push(format!("{k:?} {} z{:.2} len {:.1}: nothing at all", s.file, mid.z, s.length));
                        }
                    }
                    Some(Sel::Spline(a, b)) => {
                        let win = doc.spline(a, b).unwrap();
                        let wc = omsi_geometry::SplineCurve::from_map(win, tile_origin(a));
                        let wm = assets.spline(&win.file);
                        let hw = wm.as_ref().map(|m| m.half_width as f64).unwrap_or(3.0).max(1.5);
                        // (the same walk pick does: the piece of it the ray comes nearest, over
                        // the height it is drawn to - a wall stands up from its line)
                        let top = wm.as_ref().map(|m| m.top() as f64).unwrap_or(0.0).max(0.0);
                        let steps = if top <= 0.3 { 1 } else { ((top / (2.0 * hw)).ceil() as usize + 1).min(6) };
                        let n = (win.length / 1.0).ceil().max(2.0) as usize;
                        let mut along = f64::MAX;
                        let mut z = mid.z;
                        let mut prev = wc.point_at(0.0);
                        for i in 1..=n {
                            let next = wc.point_at(win.length * i as f64 / n as f64);
                            for si in 0..steps {
                                let lift = if steps == 1 { 0.0 } else { top * (si as f64 / (steps - 1) as f64) };
                                let up = if lift == 0.0 { DVec3::ZERO } else { DVec3::new(0.0, 0.0, lift) };
                                if let Some((lat, cross)) = ray_near_piece(from, -DVec3::Z, prev + up, next + up) {
                                    if lat <= hw && cross > 0.0 && cross < along {
                                        along = cross;
                                        z = (prev.z + next.z) * 0.5 + lift;
                                    }
                                }
                            }
                            prev = next;
                        }
                        // (the ray goes straight down, so nearer means higher)
                        if along > 60.0 + 0.5 {
                            behind += 1;
                            if report.len() < 5 {
                                report.push(format!("{:?} {} z{:.2} -> {} z{:.2} ({:.1} m further)", k, s.file, mid.z, win.file, z, along - 60.0));
                            }
                        }
                    }
                    // (an object may be the nearest thing there too: it must not be one behind)
                    Some(Sel::Object(a, b)) => {
                        let Some(ob) = doc.object(a, b) else { continue };
                        let Some(model) = assets.object(&ob.file) else { continue };
                        let world = doc.object_world(a, ob, model.abs_height);
                        let m = crate::render::object_matrix(world - from, ob.rot);
                        let c = m.transform_point3(model.centre());
                        let t = if model.parts.is_empty() {
                            c.as_dvec3().dot(-DVec3::Z)
                        } else {
                            model.parts.iter().filter_map(|p| omsi_geometry::ray_mesh(glam::Vec3::ZERO, -glam::Vec3::Z, &p.mesh, &m)).map(|t| t as f64).reduce(f64::min).unwrap_or(f64::MAX)
                        };
                        if t > 60.0 + 0.5 {
                            behind += 1;
                            if report.len() < 5 {
                                report.push(format!("{:?} {} z{:.2} -> object {} {:.1} m further", k, s.file, mid.z, ob.file, t - 60.0));
                            }
                        }
                    }
                    // (a record's copy may stand on the road itself - a parking bay, an arrow -
                    // and be met first: it lies at the place aimed at, so nothing is wrong)
                    Some(Sel::Attached(..)) => {}
                }
            }
        }
        for r in &report {
            eprintln!("picked under the one aimed at: {r}");
        }
        assert_eq!(nothing, 0, "every spline aimed at is picked");
        assert_eq!(behind, 0, "nothing under the spline aimed at is picked");
    }


    /// A road over another road is the one a click gives: two splines in the same place on the
    /// ground are told apart by the height the camera meets first, not by the map's order.
    #[test]
    fn a_click_takes_the_road_the_camera_meets_first_not_the_one_under_it() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 60.0, 0, 0);
        add_spline(&mut doc, (0, 0), 2, 100.0, 100.0, 0.0, 60.0, 0, 0);
        // (the second is a flyover eight metres up, straight over the first)
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[1].pos[2] = 8.0;
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        // (where the flyover's deck actually is: its heading decides which way it runs)
        let (x, y, z) = {
            let d = ed.doc.as_ref().unwrap();
            let s = &d.tiles[&(0, 0)].tile.splines[1];
            let c = omsi_geometry::SplineCurve::from_map(s, tile_origin((0, 0)));
            let p = c.point_at(s.length * 0.5);
            (p.x, p.y, p.z)
        };
        // from above the flyover's deck
        let got = ed.pick(&mut assets, DVec3::new(x, y, z + 50.0), -DVec3::Z, Some(DVec3::new(x, y, 0.0)));
        assert_eq!(got, Some(Sel::Spline((0, 0), 2)), "the flyover, not the road under it");
        // and from under it, looking up at it
        let got = ed.pick(&mut assets, DVec3::new(x, y, z - 2.0), DVec3::Z, None);
        assert_eq!(got, Some(Sel::Spline((0, 0), 2)), "the flyover from below");
    }


    /// A click on an object standing along a road picks that road: the lamps, rails and bays a
    /// map lays along a spline (`[spline_att]`) are drawn with no selection of their own, so a
    /// click on one used to give nothing at all and its row could only be reached by finding the
    /// road under it. They belong to the road they stand on, and are picked, tinted and edited
    /// as it ("Objects along the road" in the properties).
    ///
    /// Each `[splineAttachement]` record is a thing of its own, so a click on one of its copies
    /// picks that record (not the road it stands on).
    #[test]
    fn a_click_on_an_object_along_a_road_picks_that_object() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 60.0, 0, 0);
        // (a row of them every ten metres, a metre above the road, so its copies are the first
        // thing the ray meets - as a lamp or a sign standing on the road would be)
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(omsi_map::SplineAttachment {
            file: "Sceneryobjects\\lamp.sco".into(),
            id: 5,
            spline_index: 0,
            offset: [0.0, 1.0, 0.0],
            interval: 10.0,
            range: 40.0,
            ..Default::default()
        });
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        // (a model of its own: the record's file is not on this machine, and the pick needs its
        // size; one metre up, its place is nearer the ray than the road under it)
        assets.objects.insert("sceneryobjects\\lamp.sco".into(), crate::assets::Slot::Ready(std::sync::Arc::new(crate::assets::ObjectModel::default())));
        // where the row's objects stand, as the renderer puts them there
        let spots = {
            let d = ed.doc.as_ref().unwrap();
            let t = &d.tiles[&(0, 0)].tile;
            let row = &t.spline_attachments[0];
            crate::place::row_objects(row, &t.splines, tile_origin((0, 0)))
        };
        assert!(spots.len() > 1, "the row has objects to click on: {}", spots.len());
        for ro in spots.iter().take(3) {
            let p = ro.pose.pos;
            let got = ed.pick(&mut assets, DVec3::new(p.x, p.y, p.z + 30.0), -DVec3::Z, Some(DVec3::new(p.x, p.y, 0.0)));
            assert_eq!(got, Some(Sel::Attached((0, 0), 5)), "clicking the row's object picks that record, at {:?}", p);
        }
        // (and the road under it is still there to be clicked, a little to the side)
        let p = spots[1].pose.pos;
        let got = ed.pick(&mut assets, DVec3::new(p.x + 2.0, p.y, 30.0), -DVec3::Z, Some(DVec3::new(p.x + 2.0, p.y, 0.0)));
        assert_eq!(got, Some(Sel::Spline((0, 0), 1)));
    }

    /// A tree is drawn as two crossed cards of a texture, not as a mesh: aimed at its crown, no
    /// triangle is there to be met, and the road under it was picked instead. The box the model
    /// was measured with is the crown's own size, so it answers for the tree - and only for a
    /// tree: a building's box holds its courtyard, and taking it for the building took clicks
    /// from whatever really stood under the cursor.
    #[test]
    fn a_tree_drawn_as_crossed_cards_is_picked_by_its_crown() {
        use glam::Vec3;
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 60.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(omsi_map::SplineAttachment {
            file: "Sceneryobjects\\tree.sco".into(),
            id: 5,
            spline_index: 0,
            offset: [0.0, 0.0, 20.0],
            interval: 10.0,
            range: 20.0,
            ..Default::default()
        });
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        // (a trunk of one small mesh and a crown of two crossed cards, as a tree is)
        let trunk = omsi_geometry::MeshData {
            positions: vec![Vec3::new(-0.2, -0.2, 0.0), Vec3::new(0.2, -0.2, 0.0), Vec3::new(0.2, 0.2, 0.0), Vec3::new(-0.2, 0.2, 0.0)],
            normals: Vec::new(),
            uvs: vec![],
            ranges: Vec::new(),
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: false,
        };
        let model = crate::assets::ObjectModel {
            parts: vec![crate::assets::Part { mesh: trunk, materials: Vec::new(), texts: Vec::new(), freetex: Vec::new(), visible: None, faded: None }],
            lo: Vec3::new(-2.5, -2.5, 0.0),
            hi: Vec3::new(2.5, 2.5, 10.0),
            tree: Some((vec![], "tree.tga".into(), 10.0, 1.0)),
            ..Default::default()
        };
        assets.objects.insert("sceneryobjects\\tree.sco".into(), crate::assets::Slot::Ready(std::sync::Arc::new(model)));
        let spots = {
            let d = ed.doc.as_ref().unwrap();
            let t = &d.tiles[&(0, 0)].tile;
            crate::place::row_objects(&t.spline_attachments[0], &t.splines, tile_origin((0, 0)))
        };
        let p = spots[0].pose.pos;
        // (aimed at the middle of the crown, where only the cards are drawn)
        let crown = p + DVec3::new(0.0, 0.0, 6.0);
        let from = crown + DVec3::new(18.0, -18.0, 18.0);
        let got = ed.pick(&mut assets, from, (crown - from).normalize(), Some(DVec3::new(p.x, p.y, 0.0)));
        assert_eq!(got, Some(Sel::Attached((0, 0), 5)));
    }

    /// A building is what it draws, not the box it was measured with: its box holds its
    /// courtyard, and the gap between two wings, and it is met before the faces inside it - so
    /// taking it for the building made a building take the click from the lamp that really
    /// stands under the cursor ("every building gives the wrong thing").
    #[test]
    fn a_building_is_what_it_draws_not_the_box_it_was_measured_with() {
        use glam::Vec3;
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 200.0, 0, 0);
        // (the lamp at (102, 100), inside the building's box, which reaches from x 70 to 110
        // and y 80 to 120)
        add_object(&mut doc, (0, 0), 9, 102.0, 100.0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: "Sceneryobjects\\house.sco".into(), id: 10, pos: [90.0, 100.0, 0.0], ..Default::default() });
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        // (a lamp: a thin post of two triangles)
        let post = omsi_geometry::MeshData {
            positions: vec![Vec3::new(-0.1, -0.1, 0.0), Vec3::new(0.1, -0.1, 0.0), Vec3::new(0.1, 0.1, 4.0), Vec3::new(-0.1, 0.1, 4.0)],
            normals: Vec::new(),
            uvs: Vec::new(),
            ranges: Vec::new(),
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: false,
        };
        let lamp = crate::assets::ObjectModel {
            parts: vec![crate::assets::Part { mesh: post, materials: Vec::new(), texts: Vec::new(), freetex: Vec::new(), visible: None, faded: None }],
            lo: Vec3::new(-0.1, -0.1, 0.0),
            hi: Vec3::new(0.1, 0.1, 4.0),
            ..Default::default()
        };
        // (a building: one roof at the top of a 40 m box, the rest of the box empty air)
        let wall = omsi_geometry::MeshData {
            positions: vec![Vec3::new(-20.0, -20.0, 12.0), Vec3::new(20.0, -20.0, 12.0), Vec3::new(20.0, 20.0, 12.0), Vec3::new(-20.0, 20.0, 12.0)],
            normals: Vec::new(),
            uvs: Vec::new(),
            ranges: Vec::new(),
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: false,
        };
        let house = crate::assets::ObjectModel {
            parts: vec![crate::assets::Part { mesh: wall, materials: Vec::new(), texts: Vec::new(), freetex: Vec::new(), visible: None, faded: None }],
            lo: Vec3::new(-20.0, -20.0, 0.0),
            hi: Vec3::new(20.0, 20.0, 12.0),
            ..Default::default()
        };
        assets.objects.insert("sceneryobjects\\x.sco".into(), crate::assets::Slot::Ready(std::sync::Arc::new(lamp)));
        assets.objects.insert("sceneryobjects\\house.sco".into(), crate::assets::Slot::Ready(std::sync::Arc::new(house)));
        // (looking along the box, from in front of it: the lamp is the only thing drawn there.
        // The building's box starts five metres nearer, and it used to take the click.)
        let got = ed.pick(&mut assets, DVec3::new(102.0, 85.0, 2.0), DVec3::Y, Some(DVec3::new(102.0, 100.0, 0.0)));
        assert_eq!(got, Some(Sel::Object((0, 0), 9)), "the building took the click from what is drawn in front of it");
    }

    /// A record standing on its own road is its own thing, clicked from any side: the road was
    /// met where the ray first came within a lane's width of it - several metres before the
    /// road itself when the eye is high - so a parking bay, a low shrub or a bench along a road
    /// was always behind the road and could not be clicked at all.
    #[test]
    fn a_record_lying_on_its_road_is_picked_from_a_slant() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 60.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.spline_attachments.push(omsi_map::SplineAttachment {
            file: "Sceneryobjects\\bay.sco".into(),
            id: 5,
            spline_index: 0,
            // (a parking bay: a flat picture laid on the road itself)
            offset: [0.0, 0.02, 10.0],
            interval: 10.0,
            range: 40.0,
            ..Default::default()
        });
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        let model = crate::assets::ObjectModel { lo: glam::Vec3::new(-1.0, -2.4, 0.0), hi: glam::Vec3::new(1.0, 2.4, 0.1), ..Default::default() };
        assets.objects.insert("sceneryobjects\\bay.sco".into(), crate::assets::Slot::Ready(std::sync::Arc::new(model)));
        let spots = {
            let d = ed.doc.as_ref().unwrap();
            let t = &d.tiles[&(0, 0)].tile;
            crate::place::row_objects(&t.spline_attachments[0], &t.splines, tile_origin((0, 0)))
        };
        let p = spots[0].pose.pos;
        // (the eye 25 m up and 25 m to the side, as a person looks at a bay across the road)
        let from = p + DVec3::new(25.0, -25.0, 25.0);
        let got = ed.pick(&mut assets, from, (p - from).normalize(), Some(DVec3::new(p.x, p.y, 0.0)));
        assert_eq!(got, Some(Sel::Attached((0, 0), 5)));
    }

    /// A click names the object the ray meets first. Checked on a real city map: aiming at the
    /// middle of a building from above and to the side, what the pick names has to be what a
    /// walk over every object of every tile says is met first - not a nearer one in a tile the
    /// aim's ground point does not fall on, and not a farther one.
    #[test]
    fn a_click_names_the_object_whose_faces_it_meets() {
        let _map = crate::doc::one_map();
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2 installation: skipped");
            return;
        };
        let cfg = omsi.join("maps").join("Berlin-Spandau").join("global.cfg");
        if !cfg.is_file() {
            eprintln!("no Berlin-Spandau: skipped");
            return;
        }
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        // (the objects big enough to be buildings, and the tiles they stand on)
        let spots: Vec<(TileKey, i64)> = {
            let d = ed.doc.as_ref().unwrap();
            let mut v: Vec<(TileKey, i64)> = d.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().map(move |o| (*k, o.id))).collect();
            v.sort();
            v
        };
        let mut aims = 0;
        let mut tested = 0;
        let mut invented: Vec<String> = Vec::new();
        for (k, id) in spots.iter().step_by(9) {
            let ob = match ed.doc.as_ref().unwrap().object(*k, *id) {
                Some(o) => o.clone(),
                None => continue,
            };
            let Some(model) = assets.object_wait(&ob.file, 2000) else { continue };
            if (model.hi - model.lo).truncate().length() < 8.0 || model.parts.is_empty() {
                continue;
            }
            aims += 1;
            // (from above and to the side, at the middle of what it draws)
            let world = ed.doc.as_ref().unwrap().object_world(*k, &ob, model.abs_height);
            let rel = crate::render::object_matrix(DVec3::ZERO, ob.rot);
            let centre = world.as_vec3() + rel.transform_point3(model.centre());
            let from = centre.as_dvec3() + DVec3::new(30.0, -30.0, 22.0);
            let dir = (centre.as_dvec3() - from).normalize();
            let g = ed.doc.as_ref().unwrap().ground(centre.truncate().as_dvec2());
            let got = ed.pick(&mut assets, from, dir, Some(DVec3::new(centre.x as f64, centre.y as f64, g)));
            let Some(Sel::Object(gk, gid)) = got else { continue };
            // what the ray really meets first, over every object's own faces - a tree by the
            // cards it is drawn as and the shape of its picture, as the pick meets it
            let mut truth = f64::MAX;
            let mut named = f64::INFINITY;
            let aim = dir.as_vec3();
            let mut dist = |assets: &mut Assets, mat: &glam::Mat4, m: &crate::assets::ObjectModel| -> Option<f64> {
                if let Some((dirs, tex, h, r)) = m.tree.clone() {
                    let mask = assets.tree_mask(&dirs, &tex);
                    let cards = ray_tree(glam::Vec3::ZERO, aim, mat, h, r, mask.as_deref());
                    let meshes = m.parts.iter().filter_map(|p| omsi_geometry::ray_mesh(glam::Vec3::ZERO, aim, &p.mesh, mat)).reduce(f32::min).map(|t| t as f64);
                    return match (cards, meshes) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
                (!m.parts.is_empty()).then(|| ray_model(glam::Vec3::ZERO, aim, mat, m)).flatten()
            };
            for (tk, t) in ed.doc.as_ref().unwrap().tiles.iter() {
                for o in &t.tile.objects {
                    let Some(m) = assets.object(&o.file) else { continue };
                    let w = ed.doc.as_ref().unwrap().object_world(*tk, o, m.abs_height);
                    let mat = crate::render::object_matrix(w - from, o.rot);
                    let Some(d) = dist(&mut assets, &mat, &m) else { continue };
                    if d < truth {
                        truth = d;
                    }
                    if *tk == gk && o.id == gid {
                        named = d;
                    }
                }
            }
            // (the ones hanging on others are a list of their own: an aim that ends on one is
            // not what this checks)
            if named.is_finite() && named > truth + 0.01 && invented.len() < 6 {
                invented.push(format!("{} {gk:?}#{gid}: met at {named:.2} m, but {truth:.2} m is the first face on the ray", ob.file));
            }
            if named.is_finite() {
                tested += 1;
            }
        }
        for i in &invented {
            eprintln!("{i}");
        }
        assert!(tested > 10, "aims answered by an object of the map: {tested} of {aims}");
        assert!(invented.is_empty(), "an object answered from where nothing of it is drawn");
    }

    /// DIAGNOSIS (temporary): one click, in full.
    ///
    /// `OPENMAPPER_POINT` is the eye (`x,y,z`), `OPENMAPPER_AIM` the place aimed at; both in
    /// the map's own coordinates, as the status bar shows them. Prints what the pick answers,
    /// and everything the ray comes near on the way - each with how far along, how far to the
    /// side, and whether it is inside its own width - so a click that answers wrongly says why.
    #[test]
    #[ignore = "one click on a whole map: run by hand with OPENMAPPER_POINT and OPENMAPPER_AIM"]
    fn diag_one_click() {
        let _map = crate::doc::one_map();
        let map = std::env::var("OPENMAPPER_SWEEP_MAP").unwrap_or_else(|_| "London".into());
        let parse = |v: &str| -> Option<DVec3> {
            let n: Vec<f64> = v.split(',').filter_map(|q| q.trim().parse().ok()).collect();
            (n.len() == 3).then(|| DVec3::new(n[0], n[1], n[2]))
        };
        let Some(eye) = std::env::var("OPENMAPPER_POINT").ok().and_then(|v| parse(&v)) else {
            eprintln!("OPENMAPPER_POINT=x,y,z and OPENMAPPER_AIM=x,y,z: skipped");
            return;
        };
        let Some(aim) = std::env::var("OPENMAPPER_AIM").ok().and_then(|v| parse(&v)) else {
            eprintln!("OPENMAPPER_AIM=x,y,z: skipped");
            return;
        };
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2: skipped");
            return;
        };
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let cfg = omsi.join("maps").join(&map).join("global.cfg");
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);

        let dir = (aim - eye).normalize();
        let gh = ed.ground_hit(eye, dir);
        eprintln!("EYE {eye:?} AIM {aim:?}");
        eprintln!("GROUND HIT {gh:?} at {:?} m along", gh.map(|q| (q - eye).length()));
        eprintln!("PICK {:?}", ed.pick(&mut assets, eye, dir, gh));

        let doc = ed.doc.as_ref().unwrap();
        let radius = 30.0;
        eprintln!("--- roads whose line passes within {radius} m of the aim");
        let mut near: Vec<(f64, TileKey, i64, String, f64, f64, f64, f64)> = Vec::new();
        for (k, t) in doc.tiles.iter() {
            let org = tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let n = (s.length / 2.0).ceil().max(2.0) as usize;
                let mut closest = f64::MAX;
                let mut at = 0.0;
                let mut prev = c.point_at(0.0);
                for i in 1..=n {
                    let next = c.point_at(s.length * i as f64 / n as f64);
                    let q = prev + (next - prev) * 0.5;
                    let d = (q.truncate() - aim.truncate()).length();
                    if d < closest {
                        closest = d;
                        at = (i as f64 - 0.5) / n as f64 * s.length;
                    }
                    prev = next;
                }
                if closest > radius {
                    continue;
                }
                let hw = assets.spline_wait(&s.file, 3000).map(|m| (m.half_width as f64).max(1.5)).unwrap_or(3.0);
                let top = assets.spline(&s.file).map(|m| m.top() as f64).unwrap_or(0.0);
                // (how this road is met by the aim, as the pick meets it)
                let standing = top > 0.05;
                let mut best: Option<(f64, f64)> = None;
                let mut prev = c.point_at(0.0);
                for i in 1..=n {
                    let next = c.point_at(s.length * i as f64 / n as f64);
                    let met = if standing { ray_wall_piece(eye, dir, prev, next, top) } else { ray_near_piece(eye, dir, prev, next) };
                    if let Some((lat, cross)) = met {
                        if best.map(|b| lat < b.0).unwrap_or(true) {
                            best = Some((lat, cross));
                        }
                    }
                    prev = next;
                }
                let (lat, cross) = best.unwrap_or((f64::MAX, f64::MAX));
                near.push((cross, *k, s.id, s.file.clone(), hw, top, lat, closest));
                let _ = at;
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (cross, k, id, file, hw, top, lat, closest) in near.iter() {
            if *lat > *hw {
                continue;
            }
            let real = assets.spline(file).map(|m| m.half_width as f64).unwrap_or(-1.0);
            eprintln!("ROAD {file} {k:?}#{id}: {cross:.2} m along, {lat:.3} m to the side (picked as {hw:.2} m wide, drawn {real:.2} m, top {top:.2}); its line is {closest:.1} m from the aim");
        }

        eprintln!("--- objects whose box the ray meets");
        let ov = glam::Vec3::ZERO;
        let dv = dir.as_vec3();
        let mut hits: Vec<(f64, TileKey, i64, String, f64)> = Vec::new();
        for (k, t) in doc.tiles.iter() {
            for ob in &t.tile.objects {
                let Some(model) = assets.object_wait(&ob.file, 3000) else { continue };
                let world = doc.object_world(*k, ob, model.abs_height);
                let m = object_matrix(world - eye, ob.rot);
                let Some(t0) = ray_object_box(ov, dv, &m, model.lo, model.hi) else { continue };
                if (t0 as f64) > (gh.map(|q| (q - eye).length()).unwrap_or(f64::MAX)) + 2.0 {
                    continue;
                }
                let hit = if let Some((dirs, tex, h, r)) = model.tree.clone() {
                    let mask = assets.tree_mask(&dirs, &tex);
                    ray_tree(ov, dv, &m, h, r, mask.as_deref())
                } else {
                    ray_model(ov, dv, &m, &model)
                };
                hits.push((t0 as f64, *k, ob.id, ob.file.clone(), hit.unwrap_or(f64::MAX)));
                if let Some(h) = hit {
                    let _ = h;
                }
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (t0, k, id, file, drawn) in hits.iter().take(10) {
            eprintln!("OBJECT {file} {k:?}#{id}: box met at {t0:.2} m, drawn face at {drawn:.2} m");
        }
    }

    /// DIAGNOSIS (temporary): the road under a tree.
    ///
    /// A tree is drawn as two crossed cards; its box is a square as wide as its crown, reaching
    /// the ground, so it stood in front of the pavement, the kerb, the road and the lamps round
    /// and under every trunk - on a map lined with trees, "most of it cannot be selected under
    /// a tree". Every road running under a crown is aimed at from four sides: what answers must
    /// be the road.
    #[test]
    #[ignore = "sweeps a whole map: run by hand with OPENMAPPER_SWEEP_MAP"]
    fn diag_under_trees() {
        let _map = crate::doc::one_map();
        let map = std::env::var("OPENMAPPER_SWEEP_MAP").unwrap_or_else(|_| "London".into());
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2: skipped");
            return;
        };
        let cfg = omsi.join("maps").join(&map).join("global.cfg");
        if !cfg.is_file() {
            eprintln!("no {map}: skipped");
            return;
        }
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let off_of = |el: f64, az: f64| {
            let (e, a) = (el.to_radians(), az.to_radians());
            DVec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())
        };

        // (a tree is drawn as two crossed cards; its box is a square as wide as its crown
        // reaching the ground, so it stood in front of everything round and under it. Aimed
        // at from four sides, the road under a tree must answer as the road.)
        let trees: Vec<(TileKey, i64, f64, f64)> = {
            let d = ed.doc.as_ref().unwrap();
            let mut v: Vec<(TileKey, i64, f64, f64)> = Vec::new();
            for (k, t) in &d.tiles {
                for ob in &t.tile.objects {
                    let Some(m) = assets.object_wait(&ob.file, 3000) else { continue };
                    let Some((_, _, h, r)) = &m.tree else { continue };
                    let w = (h * r * 0.5) as f64;
                    let p = d.object_world(*k, ob, m.abs_height);
                    v.push((*k, ob.id, w, p.z));
                }
            }
            v.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            v
        };
        ed.hide = [false, false, false];
        let (mut diag_sampled, mut diag_near) = (0usize, 0usize);
        let (mut u_seen, mut u_road, mut u_tree, mut u_other, mut u_none) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let mut u_bad: Vec<String> = Vec::new();
        for (ti, (k, id, w, _)) in trees.iter().enumerate() {
            if ti % 37 != 0 {
                continue;
            }
            diag_sampled += 1;
            let Some(ob) = ed.doc.as_ref().unwrap().object(*k, *id).cloned() else { continue };
            let Some(model) = assets.object(&ob.file) else { continue };
            let origin = ed.doc.as_ref().unwrap().object_world(*k, &ob, model.abs_height);
            // (the roads through its footprint, in this tile and the eight round it)
            let mut near: Vec<(TileKey, i64)> = Vec::new();
            let (tx, ty) = (k.0, k.1);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let tk = (tx + dx, ty + dy);
                    let Some(t) = ed.doc.as_ref().unwrap().tiles.get(&tk) else { continue };
                    for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(tk));
                        let n = (s.length / 2.0).ceil().max(2.0) as usize;
                        let hit = (0..=n).any(|i| (c.point_at(s.length * i as f64 / n as f64).truncate() - origin.truncate()).length() < *w + 1.5);
                        if hit {
                            near.push((tk, s.id));
                        }
                    }
                }
            }
            diag_near += near.len();
            if diag_sampled <= 5 {
                eprintln!("TREE-AT {:?}#{id} w {w:.2} at {:?}: {} roads near", k, origin.truncate(), near.len());
            }
            for (sk, sid) in near {
                let Some(s) = ed.doc.as_ref().unwrap().spline(sk, sid).cloned() else { continue };
                let Some(sm) = assets.spline_wait(&s.file, 3000) else { continue };
                if sm.is_ai_path() || s.length < 1.0 {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(&s, tile_origin(sk));
                // (the piece of it that runs under the crown)
                let n = (s.length / 1.0).ceil().max(2.0) as usize;
                let mut under: Vec<DVec3> = Vec::new();
                for i in 0..=n {
                    let q = c.point_at(s.length * i as f64 / n as f64);
                    if (q.truncate() - origin.truncate()).length() < *w * 0.9 {
                        under.push(q);
                    }
                }
                if under.is_empty() {
                    continue;
                }
                u_seen += 1;
                let mut mine = false;
                let mut by_tree = false;
                let mut other = false;
                for q in under.iter().step_by((under.len() / 3).max(1)) {
                    for (el, az) in [(40.0, 30.0), (40.0, 210.0), (20.0, 120.0), (20.0, 300.0)] {
                        let from = *q + off_of(el, az) * 3.0;
                        let dir = (*q - from).normalize();
                        let gh = ed.ground_hit(from, dir);
                        match ed.pick(&mut assets, from, dir, gh) {
                            Some(Sel::Spline(a, b)) if a == sk && b == sid => mine = true,
                            Some(Sel::Object(a, b)) => {
                                let is_tree = ed.doc.as_ref().unwrap().object(a, b).and_then(|o| assets.object_wait(&o.file, 3000)).map(|m| m.tree.is_some()).unwrap_or(false);
                                if is_tree {
                                    by_tree = true
                                } else {
                                    other = true
                                }
                            }
                            Some(_) => other = true,
                            None => {}
                        }
                    }
                }
                if mine {
                    u_road += 1;
                } else if by_tree {
                    u_tree += 1;
                    if u_bad.len() < 10 {
                        u_bad.push(format!("{} {sk:?}#{sid} at {:?} under a {:?}#{id} crown", s.file, under[0].truncate(), k));
                    }
                } else if other {
                    u_other += 1;
                } else {
                    u_none += 1;
                }
            }
        }
        for b in &u_bad {
            eprintln!("UNDER-TREE {b}: the tree itself was picked");
        }
        // (and the tree itself must still answer when its leaves are what the aim meets)
        let (mut c_seen, mut c_ok) = (0usize, 0usize);
        let mut c_bad: Vec<String> = Vec::new();
        for (ti, (k, id, w, _)) in trees.iter().enumerate() {
            if ti % 41 != 0 {
                continue;
            }
            let Some(ob) = ed.doc.as_ref().unwrap().object(*k, *id).cloned() else { continue };
            let Some(model) = assets.object_wait(&ob.file, 3000) else { continue };
            let Some((_, _, h, r)) = model.tree.clone() else { continue };
            let _ = (h, r);
            let base = ed.doc.as_ref().unwrap().object_world(*k, &ob, model.abs_height);
            let Some((_, _, hh, _)) = model.tree.clone() else { continue };
            // (the middle of the crown, straight on and from the side)
            let aim = base + DVec3::new(0.0, 0.0, hh as f64 * 0.6);
            c_seen += 1;
            let mut hit = false;
            for (el, az) in [(5.0, 0.0), (5.0, 90.0), (25.0, 45.0)] {
                let from = aim + off_of(el, az) * (2.0 + *w);
                let dir = (aim - from).normalize();
                let gh = ed.ground_hit(from, dir);
                if matches!(ed.pick(&mut assets, from, dir, gh), Some(Sel::Object(a, b)) if a == *k && b == *id) {
                    hit = true;
                }
            }
            if hit {
                c_ok += 1;
            } else if c_bad.len() < 6 {
                // (what answered instead, and whether the picture is solid where the aim met it)
                let from = aim + off_of(5.0, 0.0) * (2.0 + *w);
                let dir = (aim - from).normalize();
                let gh = ed.ground_hit(from, dir);
                let what = match ed.pick(&mut assets, from, dir, gh) {
                    Some(Sel::Object(a, b)) => format!("object {:?}", ed.doc.as_ref().unwrap().object(a, b).map(|o| o.file.clone())),
                    Some(s) => format!("{s:?}"),
                    None => "nothing".into(),
                };
                let solid = assets.tree_mask(&model.tree.clone().unwrap().0, &model.tree.clone().unwrap().1).map(|m| m.solid(0.5, 0.4));
                c_bad.push(format!("{:?}#{id} crown {} m: {what}, picture solid at its middle {solid:?}", k, hh));
            }
        }
        for b in &c_bad {
            eprintln!("CROWN-MISS {b}: the tree itself was not picked at its leaves");
        }
        eprintln!("CROWNS {map}: {c_ok} of {c_seen} trees picked when aimed at their leaves");
        eprintln!("TREES {map}: {} with a [tree] block of their own; {diag_sampled} sampled, {diag_near} roads near a sampled trunk", trees.len());
        {
            let mut seen: std::collections::HashMap<String, (usize, bool, [f32; 3], [f32; 3])> = std::collections::HashMap::new();
            let d = ed.doc.as_ref().unwrap();
            for (k, t) in d.tiles.iter() {
                for ob in &t.tile.objects {
                    let f = ob.file.clone();
                    let low = f.to_lowercase();
                    if !(low.contains("tree") || low.contains("shrub") || low.contains("bush") || low.contains("plant")) {
                        continue;
                    }
                    if seen.contains_key(&f) || seen.len() > 12 {
                        continue;
                    }
                    let Some(m) = assets.object_wait(&f, 3000) else { continue };
                    let tris: usize = m.parts.iter().map(|p| p.mesh.indices.len() / 3).sum();
                    seen.insert(f.clone(), (tris, m.tree.is_some(), m.lo.to_array(), m.hi.to_array()));
                    let _ = k;
                }
            }
            for (f, (tris, tree, lo, hi)) in seen {
                eprintln!("TREE-AS {f}: {tris} triangles, [tree] {tree}, lo {lo:?} hi {hi:?}");
            }
        }
        eprintln!("UNDER TREES {map}: of {u_seen} roads running under a crown, {u_road} selectable through it, {u_tree} taken by the tree, {u_other} another thing, {u_none} nothing");
    }

    /// DIAGNOSIS (temporary): sweeping a whole map from several angles.
    ///
    /// "All of it is selectable" is only true of a view: a marking seen edge-on is a line, a
    /// wall seen from straight above is its own plane. So every object and every road is aimed
    /// at from six camera angles (high, eye level, low, from both sides), and what is reported is
    /// per angle, together with the things that answer nothing from any of them.
    #[test]
    #[ignore = "sweeps a whole map: run by hand with OPENMAPPER_SWEEP_MAP"]
    fn diag_map_sweep() {
        let _map = crate::doc::one_map();
        let map = std::env::var("OPENMAPPER_SWEEP_MAP").unwrap_or_else(|_| "London".into());
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2: skipped");
            return;
        };
        let cfg = omsi.join("maps").join(&map).join("global.cfg");
        if !cfg.is_file() {
            eprintln!("no {map}: skipped");
            return;
        }
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);

        // (elevation above the ground, azimuth: where the camera stands relative to the aim)
        let angles: [(f64, f64, &str); 6] = [
            (85.0, 0.0, "from above"),
            (55.0, 30.0, "high"),
            (35.0, 90.0, "eye level, from +y"),
            (35.0, 200.0, "eye level, from -x/-y"),
            (18.0, 45.0, "low, from +x/+y"),
            (18.0, 300.0, "low, from -x/+y"),
        ];
        let off_of = |el: f64, az: f64| {
            let (e, a) = (el.to_radians(), az.to_radians());
            DVec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())
        };

        // ---------------- objects
        let objs: Vec<(TileKey, i64)> = {
            let d = ed.doc.as_ref().unwrap();
            let mut v: Vec<(TileKey, i64)> = d.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().map(move |o| (*k, o.id))).collect();
            v.sort();
            v
        };
        let mut o_angle = vec![(0usize, 0usize); angles.len()];
        let (mut o_total, mut o_any) = (0usize, 0usize);
        let mut o_dead: Vec<String> = Vec::new();
        for (k, id) in objs.iter().step_by(52) {
            let ob = match ed.doc.as_ref().unwrap().object(*k, *id) {
                Some(o) => o.clone(),
                None => continue,
            };
            let Some(model) = assets.object_wait(&ob.file, 3000) else { continue };
            let world = ed.doc.as_ref().unwrap().object_world(*k, &ob, model.abs_height);
            let mat = crate::render::object_matrix(world - world, ob.rot);
            let mid = world + mat.transform_point3(model.centre()).as_dvec3();
            let Some(face) = model
                .parts
                .iter()
                .flat_map(|p| {
                    let pos = &p.mesh.positions;
                    p.mesh
                        .indices
                        .chunks_exact(3)
                        .filter(|t| t.iter().all(|i| (*i as usize) < pos.len()))
                        .map(|t| (pos[t[0] as usize] + pos[t[1] as usize] + pos[t[2] as usize]) / 3.0)
                        .collect::<Vec<_>>()
                })
                .map(|f| world + mat.transform_point3(f).as_dvec3())
                .max_by(|a, b| (*a - mid).length_squared().total_cmp(&(*b - mid).length_squared()))
            else {
                continue;
            };
            o_total += 1;
            let g = ed.doc.as_ref().unwrap().ground(face.truncate());
            let mut any = false;
            for (ai, (el, az, _)) in angles.iter().enumerate() {
                let from = face + off_of(*el, *az) * 2.5;
                let dir = (face - from).normalize();
                let aim = dir.as_vec3();
                let named = ray_model(glam::Vec3::ZERO, aim, &crate::render::object_matrix(world - from, ob.rot), &model);
                let got = ed.pick(&mut assets, from, dir, Some(DVec3::new(face.x, face.y, g)));
                let mut near = |sel: Sel| -> Option<f64> {
                    let d = ed.doc.as_ref().unwrap();
                    match sel {
                        Sel::Object(gk, gid) => {
                            let o = d.object(gk, gid)?;
                            let m = assets.object(&o.file)?;
                            let w = d.object_world(gk, o, m.abs_height);
                            ray_model(glam::Vec3::ZERO, aim, &crate::render::object_matrix(w - from, o.rot), &m)
                        }
                        Sel::Attached(gk, gid) => {
                            let a = d.tiles.get(&gk)?.tile.spline_attachments.iter().find(|a| a.id == gid)?;
                            let m = assets.object(&a.file)?;
                            let ro = crate::selection::attachment_copies(d, gk, gid).into_iter().min_by_key(|r| r.index)?;
                            ray_model(glam::Vec3::ZERO, aim, &ro.pose.matrix_from(from), &m)
                        }
                        _ => None,
                    }
                };
                let good = match got {
                    Some(Sel::Object(gk, gid)) if gk == *k && gid == *id => true,
                    Some(other) => match (near(other), named) {
                        (Some(a), Some(b)) => a <= b + 0.05,
                        _ => false,
                    },
                    None => false,
                };
                o_angle[ai].0 += 1;
                if good {
                    o_angle[ai].1 += 1;
                    any = true;
                }
            }
            if any {
                o_any += 1;
            } else if o_dead.len() < 12 {
                o_dead.push(format!("{} {:?}#{}", ob.file, k, id));
            }
        }
        for a in &o_dead {
            eprintln!("OBJ-DEAD {a} (nothing from any angle)");
        }
        for (i, (seen, ok)) in o_angle.iter().enumerate() {
            eprintln!("OBJECTS {map} [{} {} deg]: {ok} of {seen} picked as themselves", angles[i].2, angles[i].0 as usize);
        }
        eprintln!("OBJECTS {map}: {o_any} of {o_total} selectable from at least one angle, {} never", o_total - o_any);

        // ---------------- roads, with the objects put away
        ed.hide = [true, true, false];
        let spl: Vec<(TileKey, i64, f64, String)> = {
            let d = ed.doc.as_ref().unwrap();
            let mut v: Vec<(TileKey, i64, f64, String)> = d.tiles.iter().flat_map(|(k, t)| t.tile.splines.iter().filter(|s| !s.deleted).map(move |s| (*k, s.id, s.length, s.file.clone()))).collect();
            v.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            v
        };
        let mut s_angle = vec![(0usize, 0usize); angles.len()];
        // (from above: a road over it, a road under it, an object, nothing)
        let mut s_over = (0usize, 0usize, 0usize, 0usize);
        let (mut s_total, mut s_any, mut s_buried) = (0usize, 0usize, 0usize);
        let mut s_dead: Vec<String> = Vec::new();
        let mut thin = (0usize, 0usize);
        for (k, id, len, file) in spl.iter().step_by(54) {
            if *len < 1.0 {
                continue;
            }
            let Some(s) = ed.doc.as_ref().unwrap().spline(*k, *id).cloned() else { continue };
            let Some(m) = assets.spline(file) else { continue };
            if m.is_ai_path() {
                continue;
            }
            let c = omsi_geometry::SplineCurve::from_map(&s, tile_origin(*k));
            let p = c.point_at(len * 0.5);
            s_total += 1;
            let is_thin = m.half_width < 0.5;
            if is_thin {
                thin.0 += 1;
            }
            // (a road whose surface is under the ground where it stands cannot be clicked, and
            // rightly so: a wall buried in a hillside)
            let g = ed.doc.as_ref().unwrap().ground(p.truncate());
            if g > p.z + 2.0 {
                s_buried += 1;
            }
            let walk = |c: &omsi_geometry::SplineCurve, len: f64, from: DVec3, dir: DVec3| -> Option<f64> {
                let n = (len / 1.0).ceil().max(2.0) as usize;
                let mut best: Option<f64> = None;
                let mut prev = c.point_at(0.0);
                for i in 1..=n {
                    let next = c.point_at(len * i as f64 / n as f64);
                    if let Some((_, cross)) = ray_near_piece(from, dir, prev, next) {
                        if cross > 0.0 && best.map(|b| cross < b).unwrap_or(true) {
                            best = Some(cross);
                        }
                    }
                    prev = next;
                }
                best
            };
            let mut any = false;
            // (a user clicks somewhere along the road, not only at its middle: a fence sharing
            // its place with a hedge for part of its length is still clickable elsewhere)
            for frac in [0.5f64, 0.25, 0.75] {
                let p = c.point_at(len * frac);
                let g = ed.doc.as_ref().unwrap().ground(p.truncate());
                for (ai, (el, az, _)) in angles.iter().enumerate() {
                    let from = p + off_of(*el, *az) * 4.0;
                    let dir = (p - from).normalize();
                    // (the ground as the app finds it: marching the height map along the ray,
                    // which is what a hover does - a point put under the aim would hide the
                    // ground's own part in the answer)
                    let gh = ed.ground_hit(from, dir);
                    let got = ed.pick(&mut assets, from, dir, gh);
                    let good = match got {
                        Some(Sel::Spline(sk, sid)) if sk == *k && sid == *id => true,
                        // (another road at the same place - the second line of a double line,
                        // the joint between two pieces - is as good)
                        Some(Sel::Spline(sk, sid)) => match (ed.doc.as_ref().unwrap().spline(sk, sid).cloned(), walk(&c, *len, from, dir)) {
                            (Some(o), Some(b)) => walk(&omsi_geometry::SplineCurve::from_map(&o, tile_origin(sk)), o.length, from, dir).map(|a| a <= b + 0.05).unwrap_or(false),
                            _ => false,
                        },
                        _ => false,
                    };
                    // (from straight above, a road under a bridge or a roof is not what is
                    // seen: what answers is the surface in front of it)
                    if frac == 0.5 && ai == 0 && !good {
                        match got {
                            Some(Sel::Spline(sk, sid)) => {
                                let z = ed.doc.as_ref().unwrap().spline(sk, sid).map(|o| omsi_geometry::SplineCurve::from_map(o, tile_origin(sk)).point_at(o.length * 0.5).z).unwrap_or(p.z);
                                if z > p.z + 0.3 {
                                    s_over.0 += 1;
                                } else {
                                    s_over.1 += 1;
                                }
                            }
                            Some(_) => s_over.2 += 1,
                            None => s_over.3 += 1,
                        }
                    }
                    // (the table is of the middle of the road: the places a person aims at first)
                    if frac == 0.5 {
                        s_angle[ai].0 += 1;
                        if good {
                            s_angle[ai].1 += 1;
                        }
                    }
                    if good {
                        any = true;
                    }
                }
            }
            if any {
                s_any += 1;
                if is_thin {
                    thin.1 += 1;
                }
            } else if s_dead.len() < 12 {
                // (what it does answer, from the high angle)
                let from = p + off_of(55.0, 30.0) * 4.0;
                let dir = (p - from).normalize();
                let what = match ed.pick(&mut assets, from, dir, Some(DVec3::new(p.x, p.y, g))) {
                    Some(Sel::Spline(sk, sid)) => {
                        let f = ed.doc.as_ref().unwrap().spline(sk, sid).map(|s| (s.file.clone(), s.pos)).unwrap_or_default();
                        let hw = assets.spline(&f.0).map(|m| m.half_width).unwrap_or(-1.0);
                        format!("road {} at {:?} half width {hw}", f.0, f.1)
                    }
                    Some(Sel::Object(gk, gid)) => format!("object {:?}", ed.doc.as_ref().unwrap().object(gk, gid).map(|o| o.file.clone())),
                    Some(Sel::Attached(..)) => "a record".into(),
                    None => "nothing".into(),
                };
                let gd = ed.ground_hit(from, dir).map(|q| (q - from).length()).unwrap_or(f64::MAX);
                let cross = {
                    let n = (len / 1.0).ceil().max(2.0) as usize;
                    let mut best = f64::MAX;
                    let mut prev = c.point_at(0.0);
                    for i in 1..=n {
                        let next = c.point_at(len * i as f64 / n as f64);
                        if let Some((lat, x)) = ray_near_piece(from, dir, prev, next) {
                            if lat <= 1.5 && x > 0.0 && x < best {
                                best = x;
                            }
                        }
                        prev = next;
                    }
                    best
                };
                s_dead.push(format!("{file} {k:?}#{id} at {p:?} len {len:.1} hw {} (height {:.2}, ground under it {g:.2}, ray {:.2} to it, ground hit at {gd:.2}) -> {what}", m.half_width, p.z, cross));
            }
        }
        for a in &s_dead {
            eprintln!("ROAD-DEAD {a} (nothing from any angle)");
        }
        for (i, (seen, ok)) in s_angle.iter().enumerate() {
            eprintln!("ROADS {map} [{} {} deg]: {ok} of {seen} picked as themselves", angles[i].2, angles[i].0 as usize);
        }
        eprintln!("ROADS {map}: {s_any} of {s_total} selectable from at least one angle, {} never ({s_buried} of all are under the ground where they stand)", s_total - s_any);
        eprintln!("THIN ROADS {map}: {} of {} selectable", thin.1, thin.0);


        // (a wall is drawn upward from its line: a person clicks the middle of the face they
        // see, two metres up, not the line at its foot)
        let (mut tall, mut tall_ok) = (0usize, 0usize);
        let mut tall_bad: Vec<String> = Vec::new();
        for (k, id, len, file) in spl.iter().step_by(54) {
            if *len < 1.0 {
                continue;
            }
            let Some(m) = assets.spline(file) else { continue };
            if m.is_ai_path() || m.top() < 0.5 {
                continue;
            }
            let Some(s) = ed.doc.as_ref().unwrap().spline(*k, *id).cloned() else { continue };
            let c = omsi_geometry::SplineCurve::from_map(&s, tile_origin(*k));
            tall += 1;
            let mut any = false;
            for frac in [0.5f64, 0.2, 0.8] {
                let p = c.point_at(len * frac);
                for (el, az) in [(35.0, 90.0), (35.0, 200.0), (18.0, 45.0), (18.0, 300.0), (55.0, 30.0), (-15.0, 90.0), (-15.0, 200.0), (-25.0, 45.0)] {
                    // (the middle of the face: half its height up)
                    let aim_at = p + DVec3::new(0.0, 0.0, m.top() as f64 * 0.5);
                    let from = aim_at + off_of(el, az) * 4.0;
                    let dir = (aim_at - from).normalize();
                    let g = ed.doc.as_ref().unwrap().ground(p.truncate());
                    let good = matches!(ed.pick(&mut assets, from, dir, Some(DVec3::new(p.x, p.y, g))), Some(Sel::Spline(sk, sid)) if sk == *k && sid == *id);
                    if good {
                        any = true;
                    }
                }
            }
            if any {
                tall_ok += 1;
            } else if tall_bad.len() < 10 {
                tall_bad.push(format!("{file} top {}", m.top()));
            }
        }
        for b in &tall_bad {
            eprintln!("TALL-DEAD {b} (the face itself, from any angle)");
        }
        eprintln!("TALL ROADS {map} (drawn 0.5 m up or more): {tall_ok} of {tall} selectable by their face");
        eprintln!("ROADS {map} from above, the {} not answered by themselves: {} under a road overhead, {} another road lower down, {} an object, {} nothing", s_over.0 + s_over.1 + s_over.2 + s_over.3, s_over.0, s_over.1, s_over.2, s_over.3);
    }

    /// A record's copies are picked as the record itself, never as the road they stand on.
    ///
    /// Aimed at the middle of a copy of every fortieth record of a real map (12,000 rows of
    /// lamps, signs, trees and parking bays), with the models loaded as the view loads them:
    /// what came back was the road the record hangs on, for 27 of 52 copies before the road was
    /// met at its own surface, and for none of them after.
    #[test]
    fn every_copy_of_a_record_on_a_real_map_is_picked_as_that_record() {
        let _map = crate::doc::one_map();
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2 installation: skipped");
            return;
        };
        // (the map the plan measured is kept beside the sources, in `world`, when the
        // installation has none of that name)
        let mut cfg = omsi.join("maps").join("NT_merged_v2").join("global.cfg");
        if !cfg.is_file() {
            cfg = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("world").join("NT_merged_v2").join("global.cfg");
        }
        if !cfg.is_file() {
            eprintln!("no NT_merged_v2: skipped");
            return;
        }
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        ed.show_lanes = true;
        // (a sample spread over the map: every fortieth record that places copies, and the road
        // each of them hangs on)
        let sample: Vec<(TileKey, i64, String, i64)> = {
            let doc = ed.doc.as_ref().unwrap();
            let mut v: Vec<(TileKey, i64, String, i64)> = doc
                .tiles
                .iter()
                .flat_map(|(k, t)| {
                    let splines = &t.tile.splines;
                    t.tile.spline_attachments.iter().filter_map(move |a| {
                        // (the record, its file, and its own road's IDCode)
                        let road = splines.get(a.spline_index.max(0) as usize)?.id;
                        Some((*k, a.id, a.file.clone(), road))
                    })
                })
                .filter(|(k, id, _, _)| !crate::selection::attachment_copies(doc, *k, *id).is_empty())
                .collect();
            v.sort();
            v.into_iter().step_by(40).collect()
        };
        let (mut copies, mut own_road, mut nothing) = (0, 0, 0);
        let mut elsewhere = 0;
        let mut report: Vec<String> = Vec::new();
        for (k, id, file, road) in &sample {
            // (the view loads what it draws; the pick can only know a model it has)
            if assets.object_wait(file, 2000).is_none() {
                continue;
            }
            let spots = {
                let doc = ed.doc.as_ref().unwrap();
                crate::selection::attachment_copies(doc, *k, *id)
            };
            for ro in spots.iter().take(2) {
                let p = ro.pose.pos;
                let g = ed.doc.as_ref().unwrap().ground(p.truncate());
                copies += 1;
                // (where a person would click: the middle of a face the object draws, from 25 m
                // up and 25 m to the side - not its foot on the road, and not the middle of the
                // box it was measured with, which for a lamp post or an L-shaped shed is air)
                // (built from the record's own place, as the pick builds it: a world coordinate
                // is 11,000 km out and f32 there is a metre coarse)
                let centre = {
                    let model = assets.object(file).unwrap();
                    let c = model.centre();
                    let face = model
                        .parts
                        .iter()
                        .flat_map(|p| {
                            let pos = &p.mesh.positions;
                            p.mesh.indices.chunks_exact(3).filter(|t| t.iter().all(|i| (*i as usize) < pos.len())).map(|t| (pos[t[0] as usize] + pos[t[1] as usize] + pos[t[2] as usize]) / 3.0)
                        })
                        .min_by(|a, b| (*a - c).length_squared().total_cmp(&(*b - c).length_squared()));
                    let aim = face.unwrap_or(c);
                    ro.pose.pos + ro.pose.matrix_from(ro.pose.pos).transform_point3(aim).as_dvec3()
                };
                let from = centre + DVec3::new(25.0, -25.0, 25.0);
                let got = ed.pick(&mut assets, from, (centre - from).normalize(), Some(DVec3::new(p.x, p.y, g as f64)));
                match got {
                    // (the record's own road: exactly what a click on the record must never give)
                    Some(Sel::Spline(sk, sid)) if sk == *k && sid == *road => {
                        own_road += 1;
                        if report.len() < 6 {
                            report.push(format!("{file} {k:?}#{id} on road {road}: its own road was picked"));
                        }
                    }
                    None => {
                        nothing += 1;
                        if report.len() < 6 {
                            report.push(format!("{file} {k:?}#{id}: nothing was picked"));
                        }
                    }
                    // (a bus stop beside a road, a lamp under a tree of the same place: another
                    // map's content, merged into this one - what stands in front is in front)
                    Some(other) if other != Sel::Attached(*k, *id) => elsewhere += 1,
                    _ => {}
                }
            }
        }
        for r in &report {
            eprintln!("{r}");
        }
        eprintln!("{copies} copies: {own_road} gave the road they stand on, {nothing} nothing, {elsewhere} something in front");
        assert!(copies > 30, "the sample has copies to aim at: {copies}");
        assert_eq!((own_road, nothing), (0, 0), "a copy of a record is never the road it stands on, nor nothing");
    }

    /// The box a model is measured with answers for a ray that lies in its drawn faces.
    #[test]
    fn a_ray_meets_the_box_of_an_object_it_aims_at() {
        let lo = glam::Vec3::new(-0.07, -0.07, 0.0);
        let hi = glam::Vec3::new(0.07, 0.07, 3.5);
        for rot in [0.0f32, 90.0, 137.0] {
            let m = glam::Mat4::from_translation(glam::Vec3::new(30.0, -40.0, 3.0)) * glam::Mat4::from_rotation_z(rot.to_radians());
            let centre = m.transform_point3((lo + hi) * 0.5);
            let from = centre + glam::Vec3::new(25.0, -25.0, 25.0);
            let dir = (centre - from).normalize();
            let t = ray_object_box(glam::Vec3::ZERO, dir, &(glam::Mat4::from_translation(-from) * m), lo, hi);
            assert!(t.is_some(), "rot {rot}");
            // (the middle of the mast is where the ray was aimed, so it is at `from`'s distance)
            assert!((t.unwrap() - (centre - from).length()).abs() < 1.0, "rot {rot}: {t:?}");
        }
    }

    /// An object put onto a road becomes a `[splineAttachement]` record that keeps its IDCode,
    /// and taken off again it is the object it was, where it stood (to the millimetre).
    #[test]
    fn an_object_put_on_a_road_and_taken_off_is_where_it_was() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 100.0, 0.0, 200.0, 0, 0);
        add_object(&mut doc, (0, 0), 2, 102.0, 120.0);
        let mut ed = Editor::default();
        ed.open(doc);
        let lib = crate::library::Library::default();
        ed.attach_object_to_spline(&lib, (0, 0), 2).unwrap();
        {
            let d = ed.doc.as_ref().unwrap();
            assert!(d.object((0, 0), 2).is_none(), "it is no longer a free object");
            let a = d.attachment((0, 0), 2).expect("it is a record now");
            assert_eq!(a.id, 2, "its IDCode stays");
            assert!((a.offset[0] - 2.0).abs() < 1e-6, "2 m right of the road: {:?}", a.offset);
            assert!((a.offset[2] - 20.0).abs() < 1e-3, "20 m along it: {:?}", a.offset);
            assert_eq!(a.interval, 0.0, "one object");
        }
        assert_eq!(ed.selected, Some(Sel::Attached((0, 0), 2)));
        ed.detach_attachment((0, 0), 2).unwrap();
        let d = ed.doc.as_ref().unwrap();
        assert!(d.attachment((0, 0), 2).is_none());
        let o = d.object((0, 0), 2).expect("a free object again");
        // (within a millimetre of where it stood)
        assert!((o.pos[0] - 102.0).abs() < 1e-3 && (o.pos[1] - 120.0).abs() < 1e-3, "{:?}", o.pos);
        assert!(o.rot[0].abs() < 1e-6, "{:?}", o.rot);
        assert_eq!(ed.selected, Some(Sel::Object((0, 0), 2)));
    }

    #[test]
    fn a_ramp_makes_one_even_slope_as_wide_as_the_brush() {
        let mut ed = Editor::default();
        ed.open(flat_map(1));
        ed.brush_radius = 20.0;
        let grade = ed.ramp(DVec3::new(50.0, 150.0, 0.0), DVec3::new(250.0, 150.0, 20.0));
        assert!((grade - 10.0).abs() < 1e-9);
        let d = ed.doc.as_ref().unwrap();
        assert!((d.ground(DVec2::new(150.0, 150.0)) - 10.0).abs() < 0.3);
        assert!((d.ground(DVec2::new(200.0, 155.0)) - 15.0).abs() < 0.3);
        // past its edge and its ends the ground is as it was
        assert!(d.ground(DVec2::new(150.0, 180.0)).abs() < 1e-6);
        assert!(d.ground(DVec2::new(10.0, 150.0)).abs() < 1e-6);
        ed.undo();
        assert!(ed.doc.as_ref().unwrap().ground(DVec2::new(150.0, 150.0)).abs() < 1e-6);
    }

    #[test]
    fn a_layer_painted_finer_keeps_its_paint_and_undoes() {
        let mut doc = flat_map(2);
        let g = |p: f32| omsi_map::GroundTex { texture: "x.bmp".into(), detail_texture: String::new(), params: [p, 16.0, 4.0] };
        doc.global.ground_textures = vec![g(0.0), g(8.0)];
        doc.sync_mask_slots();
        let mut m = vec![0u8; 256 * 256];
        m[..256 * 128].iter_mut().for_each(|a| *a = 255);
        doc.tiles.get_mut(&(0, 0)).unwrap().masks[1] = Some(m.clone());
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_paint_detail(1, 10.0);
        let d = ed.doc.as_ref().unwrap();
        let big = d.tiles[&(0, 0)].masks[1].as_ref().unwrap();
        assert_eq!(big.len(), 1024 * 1024);
        assert_eq!((big[0], big[1024 * 1023]), (255, 0));
        assert!(d.tiles[&(1, 0)].masks[1].is_none());
        ed.undo();
        let d = ed.doc.as_ref().unwrap();
        assert_eq!(d.global.ground_textures[1].params[0], 8.0);
        assert_eq!(d.tiles[&(0, 0)].masks[1].as_ref(), Some(&m));
    }

    #[test]
    fn a_signal_head_put_by_a_crossing_takes_its_light_from_it() {
        let Some(root) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
        let roots = crate::install::Roots { omsi: root, content: std::env::temp_dir().join(format!("om-head-{}", std::process::id())) };
        let mut lib = Library::default();
        lib.start_scan(&roots);
        while lib.scanning {
            lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects[0].file = "Sceneryobjects\\Kreuz_MC\\Kreuz_See_Elsflether.sco".into();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.object_type = Some("Sceneryobjects\\Verkehrszeichen_MC\\Ampel_Kfz_1.sco".into());
        ed.ghost_pos = Some((DVec3::new(112.0, 100.0, 0.0), 0.0));
        let _ = ed.place_object(&lib, false);
        let d = ed.doc.as_ref().unwrap();
        let head = d.tiles[&(0, 0)].tile.objects.iter().find(|o| o.file.contains("Ampel")).unwrap();
        assert_eq!((head.var_parent, head.extra.clone()), (Some(1), vec!["0".to_string()]));
        // (far from any crossing: linked to nothing)
        ed.ghost_pos = Some((DVec3::new(250.0, 250.0, 0.0), 0.0));
        let _ = ed.place_object(&lib, false);
        assert!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.iter().filter(|o| o.file.contains("Ampel")).any(|o| o.var_parent.is_none()));
    }

    #[test]
    fn omsis_n_puts_the_chosen_object_at_the_cursor_and_takes_it() {
        let lib = Library::default();
        let mut ed = Editor::default();
        ed.open(flat_map(1));
        ed.object_type = Some("Sceneryobjects\\Test\\Test.sco".into());
        ed.cursor = Some(DVec3::new(120.0, 130.0, 0.0));
        ed.new_object(&lib);
        let d = ed.doc.as_ref().unwrap();
        let o = d.tiles[&(0, 0)].tile.objects.iter().find(|o| o.file.ends_with("Test.sco")).expect("put down");
        assert_eq!((o.pos[0].round(), o.pos[1].round()), (120.0, 130.0));
        let (id, n) = (o.id, d.tiles[&(0, 0)].tile.objects.len());
        // (as in OMSI 2's editor: the new object is taken and can be moved at once)
        assert_eq!(ed.selected, Some(Sel::Object((0, 0), id)));
        assert_eq!(ed.xform, Xform::Move);
        // (nothing chosen in the catalogue: nothing put down)
        ed.object_type = None;
        ed.new_object(&lib);
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len(), n);
    }

    #[test]
    fn nothing_is_joined_with_the_option_off_or_ctrl_held() {
        let mut ed = Editor::default();
        let mut assets = Assets::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        // (a road type not read yet counts as a road)
        assert!(ed.joins(&mut assets));
        ed.ctrl_held = true;
        assert!(!ed.joins(&mut assets));
        ed.ctrl_held = false;
        ed.snap_roads = false;
        assert!(!ed.joins(&mut assets));
    }

    #[test]
    fn a_group_is_selected_and_moved_as_one() {
        let dir = std::env::temp_dir().join(format!("om-groups-{}", std::process::id()));
        let mut doc = flat_map(1);
        doc.dir = dir.join("maps").join("G");
        for (id, x) in [(1, 100.0), (2, 110.0), (3, 200.0)] {
            add_object(&mut doc, (0, 0), id, x, 100.0);
        }
        let mut ed = Editor::default();
        ed.open(doc);
        let mut assets = Assets::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        let lib = Library::default();
        ed.set_selection(vec![Sel::Object((0, 0), 1), Sel::Object((0, 0), 2)]);
        ed.group_selection();
        assert_eq!(ed.groups().len(), 1);
        assert_eq!(crate::mapgroups::load(&ed.doc.as_ref().unwrap().dir), ed.groups(), "kept beside the map");
        // a click on one: both, and a drag moves both
        ed.select(None);
        ed.hover = Some(Sel::Object((0, 0), 2));
        ed.cursor = Some(DVec3::new(110.0, 100.0, 0.0));
        ed.left_press(&mut assets, &lib, false);
        assert_eq!(ed.selection().len(), 2);
        assert_eq!(ed.selected_group(), Some(0));
        ed.cursor = Some(DVec3::new(110.0, 120.0, 0.0));
        ed.tick(&mut assets, 0.0, true);
        ed.tick(&mut assets, 0.0, false);
        let d = ed.doc.as_ref().unwrap();
        assert!((d.object((0, 0), 1).unwrap().pos[1] - 120.0).abs() < 0.6 && (d.object((0, 0), 3).unwrap().pos[1] - 100.0).abs() < 1e-9);
        // Ctrl + click: one alone; a right click on one selects its group and asks for the menu
        ed.select(None);
        ed.ctrl_held = true;
        ed.left_press(&mut assets, &lib, false);
        ed.tick(&mut assets, 0.0, false);
        assert_eq!(ed.selection(), [Sel::Object((0, 0), 2)]);
        ed.ctrl_held = false;
        ed.select(None);
        assert!(ed.right_click());
        assert_eq!(ed.selection().len(), 2);
        ed.ungroup_selection();
        assert!(ed.groups().is_empty() && crate::mapgroups::load(&ed.doc.as_ref().unwrap().dir).is_empty());
        // (grouping and ungrouping are undone like any edit, the file following)
        ed.undo();
        assert_eq!(ed.groups().len(), 1);
        assert_eq!(crate::mapgroups::load(&ed.doc.as_ref().unwrap().dir).len(), 1);
        ed.redo();
        assert!(ed.groups().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_ruler_takes_two_clicks_wherever_they_land() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        let mut ed = Editor::default();
        ed.open(doc);
        let mut assets = Assets::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        let lib = Library::default();
        ed.toggle_ruler();
        // (the first on an object: it is measured from, not selected)
        ed.hover = Some(Sel::Object((0, 0), 1));
        ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
        ed.left_press(&mut assets, &lib, false);
        ed.hover = None;
        ed.cursor = Some(DVec3::new(130.0, 140.0, 0.0));
        ed.left_press(&mut assets, &lib, false);
        let (a, b) = ed.ruler_line().unwrap();
        assert!(((b - a).truncate().length() - 50.0).abs() < 1e-9);
        assert_eq!(ed.selected, None);
        // the angle step is what R turns a selection by and what snapping comes to
        assert_eq!(snap_deg(37.0, 10.0), 40.0);
        assert_eq!(snap_deg(37.0, 45.0), 45.0);
    }

    #[test]
    fn the_knife_cuts_a_road_wherever_it_is_clicked_until_turned_off() {
        let mut doc = flat_map(1);
        crate::selection::tests::add_spline(&mut doc, (0, 0), 1, 50.0, 100.0, 90.0, 100.0, 0, 0);
        let mut ed = Editor::default();
        ed.open(doc);
        let mut assets = Assets::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        let lib = Library::default();
        ed.toggle_cutting();
        ed.cursor_in_view = true;
        // beside the road: cut at the point of it nearest the cursor
        ed.cursor = Some(DVec3::new(80.3, 103.0, 0.0));
        let c = ed.cut_preview().unwrap();
        assert!((c.at - 30.3).abs() < 0.2 && c.fits, "{c:?}");
        // whole metres along it with grid snapping on
        ed.snap_grid = true;
        ed.grid_step = 1.0;
        assert_eq!(ed.cut_preview().unwrap().at, 30.0);
        ed.left_press(&mut assets, &lib, false);
        ed.cursor = Some(DVec3::new(120.0, 99.0, 0.0));
        ed.left_press(&mut assets, &lib, false);
        let lengths: Vec<f64> = ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().map(|s| s.length).collect();
        assert_eq!(lengths, vec![30.0, 40.0, 30.0]);
        // a right click puts the knife away; a click then cuts nothing
        ed.right_click();
        assert!(!ed.cutting && ed.cut_preview().is_none());
        ed.history.undo(ed.doc.as_mut().unwrap());
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.len(), 2);
    }

    #[test]
    fn game_logic_selects_and_moves_a_stop_already_there() {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        let o = &mut doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects[0];
        o.flag = 7;
        o.extra = vec!["Market".into(), "0".into(), "0".into(), "10".into()];
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Logic);
        ed.set_logic(LogicMode::Stops);
        ed.object_type = Some("stop.sco".into());
        let mut assets = Assets::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        let lib = Library::default();
        ed.hover = Some(Sel::Object((0, 0), 1));
        ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
        ed.left_press(&mut assets, &lib, false);
        assert_eq!(ed.selected, Some(Sel::Object((0, 0), 1)));
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len(), 1, "nothing new put");
        // (dragged: it moves)
        ed.cursor = Some(DVec3::new(110.0, 100.0, 0.0));
        ed.tick(&mut assets, 0.0, true);
        ed.tick(&mut assets, 0.0, false);
        assert!((ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().pos[0] - 110.0).abs() < 0.6);
    }

    #[test]
    fn ground_layers_change_places_with_their_paint_and_undo() {
        let mut doc = flat_map(2);
        let g = |t: &str| omsi_map::GroundTex { texture: t.into(), detail_texture: String::new(), params: [8.0, 16.0, 4.0] };
        doc.global.ground_textures = vec![g("base.bmp"), g("sand.bmp"), g("rock.bmp")];
        doc.sync_mask_slots();
        let sand = vec![7u8; 256 * 256];
        doc.tiles.get_mut(&(0, 0)).unwrap().masks[1] = Some(sand.clone());
        let mut ed = Editor::default();
        ed.open(doc);
        ed.paint_layer = 1;
        // sand after rock: drawn over it, its paint along; the base stays first
        ed.move_ground_layer(1, true);
        let d = ed.doc.as_ref().unwrap();
        assert_eq!(d.global.ground_textures.iter().map(|g| g.texture.as_str()).collect::<Vec<_>>(), ["base.bmp", "rock.bmp", "sand.bmp"]);
        assert_eq!((d.tiles[&(0, 0)].masks[1].as_ref(), d.tiles[&(0, 0)].masks[2].as_ref()), (None, Some(&sand)));
        assert!(d.tiles[&(1, 0)].dirty);
        assert_eq!(ed.paint_layer, 2);
        ed.move_ground_layer(2, true);
        ed.move_ground_layer(1, false);
        assert_eq!(ed.doc.as_ref().unwrap().global.ground_textures[0].texture, "base.bmp");
        ed.set_ground_texture(2, true, "Texture\\noise_low.bmp");
        assert_eq!(ed.doc.as_ref().unwrap().global.ground_textures[2].detail_texture, "Texture\\noise_low.bmp");
        ed.undo();
        ed.undo();
        let d = ed.doc.as_ref().unwrap();
        assert_eq!(d.global.ground_textures[1].texture, "sand.bmp");
        assert_eq!(d.tiles[&(0, 0)].masks[1].as_ref(), Some(&sand));
    }

    #[test]
    fn an_object_leans_as_the_ground_slopes_under_it() {
        // the ground climbs 10 % to the east and 5 % to the north
        let ground = |p: DVec2| p.x * 0.10 + p.y * 0.05;
        for heading in [0.0, 37.0, 90.0, 200.0, 315.0] {
            let rot = lean_rotation(heading, &ground, DVec2::new(100.0, 100.0));
            assert_eq!(rot[0], heading);
            let m = object_matrix(DVec3::ZERO, rot);
            for v in [glam::Vec3::Y, glam::Vec3::X] {
                let w = m.transform_vector3(v).as_dvec3();
                let flat = w.truncate().normalize();
                let want = flat.x * 0.10 + flat.y * 0.05;
                let got = w.z / w.truncate().length();
                assert!((got - want).abs() < 0.01, "heading {heading}: {got} {want}");
            }
        }
        // flat ground: upright
        assert_eq!(lean_rotation(45.0, &|_| 3.0, DVec2::ZERO), [45.0, 0.0, 0.0]);
    }

    fn editor() -> Editor {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        add_object(&mut doc, (0, 0), 2, 110.0, 100.0);
        add_spline(&mut doc, (0, 0), 3, 50.0, 50.0, 0.0, 30.0, 0, 0);
        let mut ed = Editor::default();
        ed.open(doc);
        ed
    }

    /// A road made longer by numbers: the road joined at its end starts where it now ends,
    /// tangent, and still ends where it did.
    #[test]
    fn a_length_typed_carries_the_next_road() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 50.0, 50.0, 0.0, 30.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 50.0, 80.0, 0.0, 60.0, 1, 0);
        let mut ed = Editor::default();
        ed.open(doc);
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        ed.shape_request = Some(((0, 0), 1, 40.0, 0.0));
        ed.tick(&mut assets, 0.016, false);
        let doc = ed.doc.as_ref().unwrap();
        assert_eq!(doc.spline((0, 0), 1).unwrap().length, 40.0);
        let c = omsi_geometry::SplineCurve::from_map(doc.spline((0, 0), 2).unwrap(), DVec2::ZERO);
        assert!((c.point_at(0.0).truncate() - DVec2::new(50.0, 90.0)).length() < 1e-6, "{:?}", c.point_at(0.0));
        assert!((c.end_point().truncate() - DVec2::new(50.0, 140.0)).length() < 1e-3, "{:?}", c.end_point());
        // and back with one undo
        ed.undo();
        assert_eq!(ed.doc.as_ref().unwrap().spline((0, 0), 1).unwrap().length, 30.0);
    }

    #[test]
    fn a_grab_follows_the_pointer() {
        let mut ed = editor();
        ed.snap_grid = false;
        ed.set_selection(vec![Sel::Object((0, 0), 1)]);
        // (the pointer over the object it takes hold of)
        ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_follow(DVec3::new(103.0, 104.0, 0.0), (13.0, 14.0));
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 103.0).abs() < 1e-6 && (o.pos[1] - 104.0).abs() < 1e-6, "{:?}", o.pos);
        // (let go: it stands, and one undo puts it back)
        assert!(ed.grab_confirm());
        assert!(ed.grab.is_none());
        ed.undo();
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 100.0).abs() < 1e-6 && (o.pos[1] - 100.0).abs() < 1e-6, "{:?}", o.pos);
    }

    /// In OMSI 2's and Blender's key sets a press on something and a drag of it picks it and
    /// nothing more: the move there is G, as it is in those editors (a hand used to them does
    /// not expect a click to carry a whole selection off). The map's own key set drags it as
    /// it always did, and G moves it in every set.
    #[test]
    fn a_press_drags_only_where_the_key_set_says_so() {
        let mut assets = Assets::new(std::env::temp_dir().join("openmapper-no-omsi"), std::env::temp_dir().join("openmapper-no-content"));
        let lib = Library::default();
        for press_drags in [true, false] {
            let mut ed = editor();
            ed.snap_grid = false;
            ed.press_drags = press_drags;
            ed.set_selection(vec![Sel::Object((0, 0), 1)]);
            // (the pointer over the object, pressed, then dragged 5 m east)
            ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
            ed.hover = Some(Sel::Object((0, 0), 1));
            ed.left_press(&mut assets, &lib, false);
            ed.cursor = Some(DVec3::new(105.0, 100.0, 0.0));
            ed.tick(&mut assets, 1.0 / 60.0, true);
            ed.left_release(&mut assets, &lib, true);
            let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
            let moved = (o.pos[0] - 100.0).abs() > 1e-6;
            assert_eq!(moved, press_drags, "press_drags {press_drags}: the object is at {:?}", o.pos);
            // (G moves it either way)
            ed.press_drags = false;
            let (x, y) = (o.pos[0], o.pos[1]);
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            assert!(ed.grab_begin((10.0, 10.0)), "the grab did not take hold");
            ed.grab_follow(DVec3::new(x, y + 7.0, 0.0), (10.0, 40.0));
            assert!(ed.grab_confirm());
            let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
            assert!((o.pos[1] - 107.0).abs() < 1e-6, "{:?}", o.pos);
        }
    }

    /// A whole chain taken hold of moves as one: every piece comes along, its links looked up
    /// once for the move, and one undo puts every one of them back.
    #[test]
    fn a_grab_of_a_whole_chain_moves_every_piece() {
        let mut doc = flat_map(2);
        let mut id = 1;
        let mut y = 20.0;
        for i in 0..20 {
            let next = if i == 19 { 0 } else { id + 1 };
            let prev = if i == 0 { 0 } else { id - 1 };
            add_spline(&mut doc, (0, 0), id, 20.0, y, 0.0, 10.0, prev, next);
            id += 1;
            y += 10.0;
        }
        let mut ed = Editor::default();
        ed.open(doc);
        ed.snap_grid = false;
        ed.set_selection((1..=20).map(|i| Sel::Spline((0, 0), i)).collect());
        ed.cursor = Some(DVec3::new(20.0, 20.0, 0.0));
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_follow(DVec3::new(23.0, 24.0, 0.0), (13.0, 14.0));
        assert!(ed.grab_confirm());
        let doc = ed.doc.as_ref().unwrap();
        for i in 1..=20 {
            let s = doc.spline((0, 0), i).unwrap();
            assert!((s.pos[0] - 23.0).abs() < 1e-6, "piece {i} east: {:?}", s.pos);
            assert!((s.pos[1] - (24.0 + (i - 1) as f64 * 10.0)).abs() < 1e-6, "piece {i} north: {:?}", s.pos);
        }
        ed.undo();
        let doc = ed.doc.as_ref().unwrap();
        for i in 1..=20 {
            let s = doc.spline((0, 0), i).unwrap();
            assert!((s.pos[0] - 20.0).abs() < 1e-6 && (s.pos[1] - (20.0 + (i - 1) as f64 * 10.0)).abs() < 1e-6, "piece {i} back: {:?}", s.pos);
        }
    }

    /// X, Y and Z lock the grab to that axis: the pointer's other ways are not taken.
    #[test]
    fn a_grab_locks_to_one_axis() {
        let mut ed = editor();
        ed.snap_grid = false;
        ed.set_selection(vec![Sel::Object((0, 0), 1)]);
        ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_axis(1);
        ed.grab_follow(DVec3::new(105.0, 106.0, 0.0), (15.0, 16.0));
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 100.0).abs() < 1e-6, "x stays: {:?}", o.pos);
        assert!((o.pos[1] - 106.0).abs() < 1e-6, "y follows: {:?}", o.pos);
        // (the same axis again lets it go free)
        ed.grab_axis(1);
        ed.grab_follow(DVec3::new(105.0, 106.0, 0.0), (15.0, 16.0));
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 105.0).abs() < 1e-6, "{:?}", o.pos);
    }

    /// Z lifts it: the pointer up the screen, and a distance typed.
    #[test]
    fn a_grab_lifts_with_z() {
        let mut ed = editor();
        ed.snap_grid = false;
        ed.set_selection(vec![Sel::Object((0, 0), 1)]);
        ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_axis(2);
        // (20 points up the screen, a metre a point)
        ed.grab_follow(DVec3::new(140.0, 140.0, 0.0), (30.0, -10.0));
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[2] - 20.0).abs() < 1e-6, "lifted: {:?}", o.pos);
        assert!((o.pos[0] - 100.0).abs() < 1e-6 && (o.pos[1] - 100.0).abs() < 1e-6, "and not moved: {:?}", o.pos);
        // a distance typed says how far, not the pointer
        ed.grab_type('2');
        ed.grab_type('.');
        ed.grab_type('5');
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[2] - 2.5).abs() < 1e-6, "typed: {:?}", o.pos);
    }

    /// Taking hold of something and locking an axis moves nothing: the grid's rounding is not
    /// let loose until the pointer (or a number typed) says how far.
    #[test]
    fn a_grab_that_is_not_moved_stands_still() {
        let mut ed = editor();
        ed.snap_grid = true;
        ed.grid_step = 1.0;
        ed.doc.as_mut().unwrap().object_mut((0, 0), 1).unwrap().pos = [100.4, 100.4, 0.0];
        ed.set_selection(vec![Sel::Object((0, 0), 1)]);
        ed.cursor = Some(DVec3::new(100.4, 100.4, 0.0));
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_axis(1);
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 100.4).abs() < 1e-9 && (o.pos[1] - 100.4).abs() < 1e-9, "not moved: {:?}", o.pos);
        // (a distance typed does move it, to the grid's whole metres along the axis)
        ed.grab_type('2');
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 100.0).abs() < 1e-6, "the grid's x: {:?}", o.pos);
        assert!((o.pos[1] - 102.0).abs() < 1e-6, "two along y: {:?}", o.pos);
    }

    /// Calling a grab off puts everything back where it was, with nothing left to undo.
    #[test]
    fn a_grab_called_off_leaves_no_trace() {
        let mut ed = editor();
        ed.snap_grid = false;
        ed.set_selection(vec![Sel::Object((0, 0), 1)]);
        ed.cursor = Some(DVec3::new(100.0, 100.0, 0.0));
        let before = ed.history.undo_label().map(|s| s.to_string());
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_follow(DVec3::new(130.0, 140.0, 0.0), (40.0, 50.0));
        assert!(ed.grab_cancel());
        let o = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap();
        assert!((o.pos[0] - 100.0).abs() < 1e-6 && (o.pos[1] - 100.0).abs() < 1e-6, "{:?}", o.pos);
        assert_eq!(ed.history.undo_label().map(|s| s.to_string()), before, "nothing to undo");
        assert_eq!(ed.selection(), vec![Sel::Object((0, 0), 1)]);
    }

    /// A road is grabbed as a thing, its height as well.
    #[test]
    fn a_grab_moves_a_road_and_its_height() {
        let mut ed = editor();
        ed.snap_grid = false;
        ed.set_selection(vec![Sel::Spline((0, 0), 3)]);
        ed.cursor = Some(DVec3::new(50.0, 50.0, 0.0));
        assert!(ed.grab_begin((10.0, 10.0)));
        ed.grab_axis(2);
        ed.grab_type('3');
        ed.grab_axis(0);
        ed.grab_follow(DVec3::new(56.0, 50.0, 0.0), (16.0, 10.0));
        let sp = ed.doc.as_ref().unwrap().spline((0, 0), 3).unwrap();
        assert!((sp.pos[0] - 56.0).abs() < 1e-6, "{:?}", sp.pos);
        assert!((sp.pos[2] - 3.0).abs() < 1e-6, "{:?}", sp.pos);
        assert!(ed.grab_confirm());
    }

    #[test]
    fn objects_turn_at_random_or_alike() {
        let mut ed = editor();
        ed.set_selection(vec![Sel::Object((0, 0), 1), Sel::Object((0, 0), 2)]);
        ed.doc.as_mut().unwrap().object_mut((0, 0), 1).unwrap().rot[0] = 33.0;
        ed.turn_objects(false);
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 2).unwrap().rot[0], 33.0);
        ed.turn_objects(true);
        let (a, b) = (ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().rot[0], ed.doc.as_ref().unwrap().object((0, 0), 2).unwrap().rot[0]);
        assert!(a != b && (0.0..360.0).contains(&a) && (0.0..360.0).contains(&b));
    }

    #[test]
    fn rolls_are_smooth_and_bounded() {
        let mut last = rolls(DVec2::ZERO);
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 1..2000 {
            let v = rolls(DVec2::new(i as f64 * 0.5, i as f64 * 0.21));
            assert!((-1.0..=1.0).contains(&v));
            assert!((v - last).abs() < 0.2, "a step of half a metre stays small");
            (lo, hi, last) = (lo.min(v), hi.max(v), v);
        }
        assert!(hi - lo > 0.6, "it does roll: {lo}..{hi}");
    }

    /// A slope under a 20 × 10 m house is level with its foot, inside its outline and not
    /// far off it.
    #[test]
    fn the_ground_is_levelled_under_an_outline() {
        let mut doc = flat_map(1);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        let s = TERRAIN_SAMPLES;
        let cell = tile_size() / (s - 1) as f64;
        for j in 0..s {
            for i in 0..s {
                t.terrain.heights[j * s + i] = (i as f64 * cell * 0.1) as f32;
            }
        }
        let mut hist = History::default();
        let centre = DVec2::new(150.0, 150.0);
        level_ground(&mut doc, &mut hist, centre, 0.0, (Vec3::new(-10.0, -5.0, 0.0), Vec3::new(10.0, 5.0, 8.0)), 15.0);
        for p in [DVec2::new(141.0, 146.0), DVec2::new(159.0, 154.0), centre] {
            assert!((doc.ground(p) as f64 - 15.0).abs() < 0.3, "{p}: {}", doc.ground(p));
        }
        // well outside, the slope as it was
        assert!((doc.ground(DVec2::new(180.0, 150.0)) as f64 - 18.0).abs() < 0.1);
    }

    #[test]
    fn an_object_hangs_on_another() {
        let mut ed = editor();
        ed.attach_object((0, 0), 1, 2, "Sceneryobjects\\sign.sco");
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        let a = t.attach_objects.last().unwrap();
        assert_eq!((a.parent_id, a.attach_index), (Some(1), 2));
        assert_eq!(ed.selected, Some(Sel::Object((0, 0), a.id)));
        ed.undo();
        assert!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.attach_objects.is_empty());
    }

    #[test]
    fn path_rules_are_set_in_one_step() {
        let mut ed = editor();
        let r = omsi_map::MapRule { path_index: 1, kind: "speedlimit".into(), value: 30.0, extra: 0.0, kill: false };
        ed.set_rules(Sel::Spline((0, 0), 3), vec![r.clone()]);
        ed.set_rules(Sel::Spline((0, 0), 3), vec![omsi_map::MapRule { value: 40.0, ..r }]);
        assert_eq!(ed.doc.as_ref().unwrap().spline((0, 0), 3).unwrap().rules[0].value, 40.0);
        ed.undo();
        assert!(ed.doc.as_ref().unwrap().spline((0, 0), 3).unwrap().rules.is_empty());
    }

    #[test]
    fn all_alike_are_selected() {
        let mut ed = editor();
        assert_eq!(ed.select_alike(Sel::Object((0, 0), 2)), 2);
        assert_eq!(ed.selected, Some(Sel::Object((0, 0), 2)));
        assert_eq!(ed.select_alike(Sel::Spline((0, 0), 3)), 1);
    }

    #[test]
    fn objects_align_to_the_road_beside_them() {
        let mut ed = editor();
        // (the road from (50, 50) runs north; turn it east)
        ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].heading = 90.0;
        ed.set_selection(vec![Sel::Object((0, 0), 1)]);
        ed.doc.as_mut().unwrap().object_mut((0, 0), 1).unwrap().pos = [70.0, 60.0, 0.0];
        assert_eq!(ed.align_to_roads(), 1);
        assert!((ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().rot[0] - 90.0).abs() < 1e-6);
    }

    #[test]
    fn copies_in_a_row_stand_a_step_apart() {
        let mut ed = editor();
        ed.select(Some(Sel::Object((0, 0), 1)));
        ed.array(&Library::default(), 3, DVec2::new(10.0, 0.0), 90.0, 1, DVec2::ZERO);
        let doc = ed.doc.as_ref().unwrap();
        let xs: Vec<(f64, f64)> = doc.tiles[&(0, 0)].tile.objects.iter().skip(2).map(|o| (o.pos[0], o.rot[0])).collect();
        assert_eq!(xs, vec![(110.0, 90.0), (120.0, 180.0), (130.0, 270.0)]);
        assert_eq!(ed.selection().len(), 4);
        ed.undo();
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len(), 2);
    }

    #[test]
    fn a_wire_is_laid_on_a_track_piece_for_piece() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 10, 50.0, 50.0, 0.0, 30.0, 0, 11);
        add_spline(&mut doc, (0, 0), 11, 50.0, 80.0, 0.0, 30.0, 10, 0);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_selection(vec![Sel::Spline((0, 0), 10), Sel::Spline((0, 0), 11)]);
        assert_eq!(ed.wire_along("Splines\\wire.sli"), 2);
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        let w: Vec<&MapSpline> = t.splines.iter().filter(|s| s.file.ends_with("wire.sli")).collect();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].next_id, w[1].prev_id), (w[1].id, w[0].id));
        assert_eq!((w[0].pos, w[0].length), ([50.0, 50.0, 0.0], 30.0));
        // (the track itself as it was)
        assert_eq!(t.splines.iter().find(|s| s.id == 10).unwrap().next_id, 11);
    }

    #[test]
    fn copies_in_rows_make_a_grid() {
        let mut ed = editor();
        ed.select(Some(Sel::Object((0, 0), 1)));
        ed.array(&Library::default(), 2, DVec2::new(10.0, 0.0), 0.0, 3, DVec2::new(0.0, 20.0));
        let doc = ed.doc.as_ref().unwrap();
        let mut at: Vec<(i64, i64)> = doc.tiles[&(0, 0)].tile.objects.iter().skip(2).map(|o| (o.pos[0] as i64, o.pos[1] as i64)).collect();
        at.sort();
        assert_eq!(at, vec![(100, 120), (100, 140), (110, 100), (110, 120), (110, 140), (120, 100), (120, 120), (120, 140)]);
    }

    #[test]
    fn a_ring_of_tiles_is_one_step() {
        let mut ed = editor();
        assert_eq!(ed.grow_map(), 8);
        assert_eq!(ed.doc.as_ref().unwrap().tiles.len(), 9);
        assert_eq!(ed.grow_map(), 16);
        ed.undo();
        assert_eq!(ed.doc.as_ref().unwrap().tiles.len(), 9);
    }

    #[test]
    fn replacing_a_type_everywhere_is_one_step() {
        let mut ed = editor();
        let from = ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().file.clone();
        let n = ed.replace_everywhere(&from.to_uppercase(), "Sceneryobjects\\x\\new.sco", false);
        assert_eq!(n, 2);
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 2).unwrap().file, "Sceneryobjects\\x\\new.sco");
        ed.undo();
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 2).unwrap().file, from);
    }

    #[test]
    fn shift_click_adds_and_takes_away() {
        let mut ed = editor();
        ed.select(Some(Sel::Object((0, 0), 1)));
        ed.toggle_selected(Sel::Spline((0, 0), 3));
        assert_eq!(ed.selection().len(), 2);
        assert_eq!(ed.selected, Some(Sel::Spline((0, 0), 3)), "the one added shows in the properties");
        ed.toggle_selected(Sel::Spline((0, 0), 3));
        assert_eq!(ed.selection(), vec![Sel::Object((0, 0), 1)]);
        // nothing selected: what `also` held does not count
        ed.toggle_selected(Sel::Object((0, 0), 2));
        ed.selected = None;
        assert!(ed.selection().is_empty());
        ed.toggle_selected(Sel::Object((0, 0), 1));
        assert_eq!(ed.selection(), vec![Sel::Object((0, 0), 1)]);
    }

    #[test]
    fn a_selection_turns_deletes_and_comes_back_in_single_steps() {
        let mut ed = editor();
        ed.select_all();
        assert_eq!(ed.selection().len(), 3);
        ed.rotate(90.0);
        let doc = ed.doc.as_ref().unwrap();
        assert!((doc.spline((0, 0), 3).unwrap().heading - 90.0).abs() < 1e-9);
        assert!((doc.object((0, 0), 1).unwrap().rot[0] - 90.0).abs() < 1e-9);
        ed.delete_selected();
        let doc = ed.doc.as_ref().unwrap();
        assert!(doc.tiles[&(0, 0)].tile.objects.is_empty() && doc.tiles[&(0, 0)].tile.splines.is_empty());
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len(), 2);
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        let doc = ed.doc.as_ref().unwrap();
        assert_eq!(doc.spline((0, 0), 3).unwrap().heading, 0.0);
    }

    #[test]
    fn copy_paste_and_duplicate_make_new_things_round_the_cursor() {
        let mut ed = editor();
        let lib = Library::default();
        ed.set_selection(vec![Sel::Object((0, 0), 1), Sel::Object((0, 0), 2)]);
        assert_eq!(ed.copy(&lib), 2);
        ed.cursor = Some(DVec3::new(200.0, 200.0, 0.0));
        ed.paste(&lib);
        let pasted = ed.selection();
        assert_eq!(pasted.len(), 2);
        let doc = ed.doc.as_ref().unwrap();
        let xs: Vec<f64> = pasted.iter().map(|s| match *s { Sel::Object(k, id) => doc.object(k, id).unwrap().pos[0], _ => 0.0 }).collect();
        assert_eq!(xs, vec![195.0, 205.0], "kept 10 m apart round the cursor");
        ed.duplicate(&lib);
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len(), 6);
        // one undo takes the duplicate away again
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len(), 4);
    }

    /// A junction at (150, 150) holding, 10 m north of it, the start of a road running on north.
    fn junction_editor() -> Editor {
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 150.0, 150.0);
        add_spline(&mut doc, (0, 0), 3, 150.0, 160.0, 0.0, 40.0, 0, 0);
        let mut ed = Editor::default();
        ed.known_helpers.insert("sceneryobjects\\x.sco".into(), (false, vec![[0.0, 10.0, 0.0, 0.0]]));
        ed.open(doc);
        ed.select(Some(Sel::Object((0, 0), 1)));
        ed
    }

    fn start_of(ed: &Editor, id: i64) -> (DVec2, f64) {
        let (k, s) = ed.doc.as_ref().unwrap().find_spline(id).unwrap();
        let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
        (c.point_at(0.0).truncate(), roads::norm_deg(c.heading_at(0.0)))
    }

    #[test]
    fn a_moved_junction_carries_its_roads_ends_along() {
        let mut ed = junction_editor();
        ed.follow_ground = false;
        ed.move_selection_by(DVec2::new(5.0, 0.0));
        let (p, h) = start_of(&ed, 3);
        assert!((p - DVec2::new(155.0, 160.0)).length() < 1e-6, "{p:?}");
        assert!(h < 1e-6 || h > 360.0 - 1e-6, "leaves the junction northwards: {h}");
        // the far end stays
        let s = ed.doc.as_ref().unwrap().spline((0, 0), 3).unwrap().clone();
        let mut cur = s;
        while cur.next_id != 0 {
            cur = ed.doc.as_ref().unwrap().find_spline(cur.next_id).unwrap().1.clone();
        }
        let c = omsi_geometry::SplineCurve::from_map(&cur, DVec2::ZERO);
        assert!((c.end_point().truncate() - DVec2::new(150.0, 200.0)).length() < 1e-6);
        // and one undo puts it all back
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        assert!((start_of(&ed, 3).0 - DVec2::new(150.0, 160.0)).length() < 1e-9);
    }

    #[test]
    fn a_turned_junction_turns_its_roads_ends() {
        let mut ed = junction_editor();
        ed.follow_ground = false;
        ed.rotate(90.0);
        let (p, h) = start_of(&ed, 3);
        assert!((p - DVec2::new(160.0, 150.0)).length() < 1e-6, "{p:?}");
        assert!((h - 90.0).abs() < 1e-6, "{h}");
    }

    #[test]
    fn raising_a_road_brings_the_neighbour_joined_outside_with_it() {
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 1, 100.0, 50.0, 0.0, 50.0, 0, 2);
        add_spline(&mut doc, (0, 0), 2, 100.0, 100.0, 0.0, 50.0, 1, 0);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.select(Some(Sel::Spline((0, 0), 2)));
        ed.raise_selection(1.5);
        let doc = ed.doc.as_ref().unwrap();
        let c1 = omsi_geometry::SplineCurve::from_map(doc.spline((0, 0), 1).unwrap(), DVec2::ZERO);
        assert!((c1.end_point().z - 1.5).abs() < 1e-9 && c1.point_at(0.0).z.abs() < 1e-9);
    }

    #[test]
    fn deleting_an_arm_queues_its_junction_to_be_made_again() {
        let mut doc = flat_map(1);
        let file = "Sceneryobjects\\openMapper\\T\\junction_1.sco";
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: file.into(), id: 1, pos: [150.0, 150.0, 0.0], ..Default::default() });
        add_spline(&mut doc, (0, 0), 3, 150.0, 160.0, 0.0, 40.0, 0, 0);
        let mut ed = Editor::default();
        ed.known_helpers.insert(file.to_lowercase(), (true, vec![[0.0, 10.0, 0.0, 0.0]]));
        ed.open(doc);
        assert!(is_own_junction(file) && !is_own_junction("Sceneryobjects\\Kreuz_MC\\x.sco"));
        assert_eq!(ed.junctions_at_ends(&[Sel::Spline((0, 0), 3)]), vec![1]);
        // (the junction deleted with it is not made again)
        assert!(ed.junctions_at_ends(&[Sel::Spline((0, 0), 3), Sel::Object((0, 0), 1)]).is_empty());
        ed.select(Some(Sel::Spline((0, 0), 3)));
        ed.delete_selected();
        assert_eq!(ed.pending_rebuild, vec![1]);
        assert!(ed.history.is_open(), "the step stays open for the plate");
    }

    #[test]
    fn snapping_rounds_to_the_grid_and_to_whole_steps() {
        let mut ed = Editor::default();
        assert_eq!(ed.snap_point(DVec2::new(1.26, 3.74)), DVec2::new(1.26, 3.74));
        ed.snap_grid = true;
        ed.grid_step = 0.5;
        assert_eq!(ed.snap_point(DVec2::new(1.26, 3.74)), DVec2::new(1.5, 3.5));
        assert_eq!(ed.snap_heading(52.0), 52.0);
        ed.snap_angle = true;
        assert_eq!(ed.snap_heading(52.0), 45.0);
        assert_eq!(ed.snap_heading(358.0), 0.0);
    }

    #[test]
    fn numbers_in_the_properties_move_an_object_over_into_the_next_tile() {
        let mut doc = flat_map(2);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.select(Some(Sel::Object((0, 0), 1)));
        ed.put_object((0, 0), 1, DVec2::new(350.0, 20.0));
        assert_eq!(ed.selected, Some(Sel::Object((1, 0), 1)));
        let o = ed.doc.as_ref().unwrap().object((1, 0), 1).unwrap();
        assert!((o.pos[0] - 50.0).abs() < 1e-9 && (o.pos[1] - 20.0).abs() < 1e-9);
    }

    #[test]
    fn a_point_put_into_an_ai_path_splits_it_and_undoes() {
        let mut ed = editor();
        ed.tool = Tool::Logic;
        ed.logic = LogicMode::Paths;
        ed.path_edit = true;
        // the spline runs north from (50, 50) for 30 m
        ed.split_path_at((0, 0), 3, DVec2::new(51.0, 62.0));
        let doc = ed.doc.as_ref().unwrap();
        let a = doc.spline((0, 0), 3).unwrap();
        assert!((a.length - 12.0).abs() < 0.3, "{}", a.length);
        let b = doc.spline((0, 0), a.next_id).unwrap();
        assert_eq!(b.prev_id, 3);
        assert!((a.length + b.length - 30.0).abs() < 1e-6);
        // too near an end: nothing happens
        ed.split_path_at((0, 0), 3, DVec2::new(50.0, 50.3));
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.len(), 2);
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        let doc = ed.doc.as_ref().unwrap();
        assert_eq!(doc.tiles[&(0, 0)].tile.splines.len(), 1);
        assert!((doc.spline((0, 0), 3).unwrap().length - 30.0).abs() < 1e-9);
    }

    /// A junction of an earlier session deleted, then the map saved: its files stay while
    /// undo can still bring it back.
    #[test]
    fn junction_files_undo_can_bring_back_are_kept() {
        let content = std::env::temp_dir().join(format!("openmapper-test-jfiles-{}", std::process::id()));
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("J");
        let file = "Sceneryobjects\\openMapper\\J\\junction_5.sco";
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: file.into(), id: 5, pos: [150.0, 150.0, 0.0], ..Default::default() });
        let dir = content.join("Sceneryobjects").join("openMapper").join("J");
        std::fs::create_dir_all(dir.join("model")).unwrap();
        std::fs::write(dir.join("junction_5.sco"), "x").unwrap();
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.select(Some(Sel::Object((0, 0), 5)));
        ed.delete_selected();
        ed.history.commit();
        assert_eq!(ed.clean_junction_files(), 0);
        assert!(dir.join("junction_5.sco").exists());
        // (nothing can bring it back any more: it goes)
        ed.history.clear();
        assert_eq!(ed.clean_junction_files(), 1);
        let _ = std::fs::remove_dir_all(&content);
    }

    /// A junction file an earlier version of the map (kept by a save) names stays: going
    /// back to that version brings the junction back.
    #[test]
    fn junction_files_an_earlier_version_names_are_kept() {
        let content = std::env::temp_dir().join(format!("openmapper-test-jbackup-{}", std::process::id()));
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("J");
        let dir = content.join("Sceneryobjects").join("openMapper").join("J");
        std::fs::create_dir_all(dir.join("model")).unwrap();
        std::fs::write(dir.join("junction_5.sco"), "x").unwrap();
        std::fs::create_dir_all(&doc.dir).unwrap();
        std::fs::write(doc.dir.join("tile_0_0.map"), "[object]\n0\nSceneryobjects\\openMapper\\J\\junction_5.sco\n5\n").unwrap();
        // a save that replaced that tile keeps it in a backup
        crate::doc::safe::backup(&doc.dir, &[crate::doc::safe::Op::Write(doc.dir.join("tile_0_0.map"), Vec::new())], &[]).unwrap();
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        assert_eq!(ed.clean_junction_files(), 0);
        assert!(dir.join("junction_5.sco").exists());
        let _ = std::fs::remove_dir_all(&content);
    }

    /// Undo while drawing on from a road just built: the drawing stops (it went on from an
    /// end the undo took away, and the next road would have been linked to a spline that
    /// is not there).
    #[test]
    fn undo_stops_drawing_from_what_it_takes_away() {
        let mut ed = editor();
        ed.set_tool(Tool::Roads);
        ed.history.begin("Build road");
        let d = ed.doc.as_mut().unwrap();
        ed.history.touch_tile(d, (0, 0));
        add_spline(d, (0, 0), 7, 50.0, 100.0, 0.0, 30.0, 0, 0);
        ed.history.commit();
        ed.road_start = Some((Anchor { pos: DVec3::new(50.0, 130.0, 0.0), heading: 0.0, spline: Some(((0, 0), 7, true)), mid: None }, true));
        ed.undo();
        assert!(ed.doc.as_ref().unwrap().spline((0, 0), 7).is_none());
        assert!(ed.road_start.is_none());
        ed.redo();
        assert!(ed.doc.as_ref().unwrap().spline((0, 0), 7).is_some());
    }

    /// With Auto-Terrain off a new road runs level from where it starts, over a slope too (on,
    /// it takes the ground's height along its way).
    #[test]
    fn a_road_runs_level_over_a_slope_with_auto_terrain_off() {
        let rise = |auto: bool| {
            let mut ed = editor();
            ed.follow_ground = false;
            ed.auto_terrain = auto;
            {
                let t = ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap();
                let n = (t.terrain.heights.len() as f64).sqrt() as usize;
                for (i, h) in t.terrain.heights.iter_mut().enumerate() {
                    *h = ((i % n) + (i / n)) as f32 * 0.3;
                }
            }
            let seg = roads::straight(DVec2::new(60.0, 120.0), DVec2::new(60.0, 220.0)).unwrap();
            let start = Anchor { pos: DVec3::new(60.0, 120.0, 4.0), heading: 0.0, ..Default::default() };
            let made = ed.build_road("Splines\\road.sli", &[seg], 4.0, 4.0, start, None, 4.0).unwrap();
            ed.history.commit();
            let doc = ed.doc.as_ref().unwrap();
            let zs: Vec<f64> = made.iter().map(|(k, id)| doc.spline(*k, *id).unwrap().pos[2]).collect();
            zs.iter().cloned().fold(f64::MIN, f64::max) - zs.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!(rise(false) < 1e-6, "{}", rise(false));
        assert!(rise(true) > 0.5, "{}", rise(true));
    }

    /// A road drawn from an end that is gone meanwhile (deleted while drawing) starts free:
    /// it is not linked to a spline that is not there.
    #[test]
    fn a_road_from_an_end_that_went_is_not_linked_to_it() {
        let mut ed = editor();
        ed.follow_ground = false;
        let seg = roads::straight(DVec2::new(50.0, 130.0), DVec2::new(50.0, 160.0)).unwrap();
        let start = Anchor { pos: DVec3::new(50.0, 130.0, 0.0), heading: 0.0, spline: Some(((0, 0), 99, true)), mid: None };
        let made = ed.build_road("Splines\\road.sli", &[seg], 0.0, 0.0, start, None, 4.0).unwrap();
        ed.history.commit();
        let (k, id) = made[0];
        assert_eq!(ed.doc.as_ref().unwrap().spline(k, id).unwrap().prev_id, 0);
    }

    /// The junction options shown in the properties are those of the map open: another
    /// map's junction of the same IDCode does not show (and get made again with) them.
    #[test]
    fn junction_options_are_not_carried_over_to_another_map() {
        let mut ed = editor();
        ed.junction_options.insert(5, crate::junction::Options { lights: true, ..Default::default() });
        ed.open(flat_map(1));
        assert!(ed.junction_options.is_empty());
    }

    /// The installation's spline, loaded (None without an OMSI 2 here).
    fn installed_spline(rel: &str) -> Option<Assets> {
        let omsi = crate::install::find_omsi_root(&Default::default())?;
        let mut a = Assets::new(omsi, std::env::temp_dir().join("openmapper-test-empty-content"));
        for _ in 0..500 {
            if a.spline(rel).is_some() {
                return Some(a);
            }
            if a.spline_failed(rel) {
                return None;
            }
            a.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn a_track_drawn_from_a_track_lays_a_switch() {
        let rail = "Splines\\Ruede\\rail_concrete_01.sli";
        let Some(mut assets) = installed_spline(rail) else {
            eprintln!("no OMSI 2 here: skipped");
            return;
        };
        // (the switch's model loaded too: the plan waits for it)
        let sw = crate::switches::object_for(rail, false);
        for _ in 0..500 {
            if assets.object(&sw).is_some() || assets.object_failed(&sw) {
                break;
            }
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(assets.object(&sw).is_some(), "{sw} is in the installation");
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 50.0, 50.0, 0.0, 150.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].file = rail.into();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(rail.into());
        ed.inherit_type = false;
        let lib = Library::default();
        // start on the track's middle, go off to the north-east (to the right)
        ed.cursor = Some(DVec3::new(50.3, 100.0, 0.0));
        ed.plan_road(&mut assets);
        assert!(ed.road_snap.map(|a| a.mid.is_some()).unwrap_or(false), "the track's middle is a place to start");
        ed.road_click(&mut assets, &lib);
        ed.cursor = Some(DVec3::new(80.0, 180.0, 0.0));
        ed.plan_road(&mut assets);
        assert_eq!(ed.switch_plans.len(), 1);
        ed.road_click(&mut assets, &lib);
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        // a right-hand switch at the toe, facing north
        let o = t.objects.iter().find(|o| o.file.contains("EW190_R")).expect("a switch object");
        assert!(o.rot[0].abs() < 1.0 || (o.rot[0] - 360.0).abs() < 1.0, "{}", o.rot[0]);
        // the main track: up to the toe, then on again from the heels, 27 m on
        let old = doc.spline((0, 0), 3).unwrap();
        assert!((old.length - 50.0).abs() < 0.5, "{}", old.length);
        assert!(t.splines.iter().any(|s| (s.pos[1] - 127.0).abs() < 0.6 && s.pos[0].abs() - 50.0 < 0.1), "the track goes on from the straight heel");
        // the branch leaves the diverging heel, a little right and turned a little right
        let branch = t.splines.iter().find(|s| s.pos[0] > 51.0).expect("the new track");
        assert!((branch.pos[0] - 51.82).abs() < 0.2 && (branch.pos[1] - 126.9).abs() < 0.3, "{:?}", branch.pos);
        assert!((branch.heading - 6.34).abs() < 0.5, "{}", branch.heading);
    }

    #[test]
    fn a_track_drawn_off_a_track_to_the_left_lays_a_left_switch() {
        let rail = "Splines\\Ruede\\rail_concrete_01.sli";
        let Some(mut assets) = installed_spline(rail) else {
            eprintln!("no OMSI 2 here: skipped");
            return;
        };
        for left in [false, true] {
            let sw = crate::switches::object_for(rail, left);
            for _ in 0..500 {
                if assets.object(&sw).is_some() || assets.object_failed(&sw) {
                    break;
                }
                assets.poll();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 50.0, 50.0, 0.0, 150.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].file = rail.into();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(rail.into());
        ed.inherit_type = false;
        let lib = Library::default();
        ed.cursor = Some(DVec3::new(50.3, 100.0, 0.0));
        ed.plan_road(&mut assets);
        ed.road_click(&mut assets, &lib);
        // off to the north-west (to the left)
        ed.cursor = Some(DVec3::new(20.0, 180.0, 0.0));
        ed.plan_road(&mut assets);
        eprintln!("plans {} plan {:?}", ed.switch_plans.len(), ed.road_plan.as_ref().map(|p| p.0.len()));
        assert_eq!(ed.switch_plans.len(), 1);
        ed.road_click(&mut assets, &lib);
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        for s in &t.splines {
            eprintln!("spline {} ({:.1},{:.1}) h {:.1} len {:.1} prev {} next {}", s.id, s.pos[0], s.pos[1], s.heading, s.length, s.prev_id, s.next_id);
        }
        let o = t.objects.iter().find(|o| o.file.contains("EW190_L")).expect("a left switch object");
        assert!(o.rot[0].abs() < 1.0 || (o.rot[0] - 360.0).abs() < 1.0, "{}", o.rot[0]);
        let branch = t.splines.iter().find(|s| s.pos[0] < 49.0).expect("the new track");
        assert!((branch.pos[0] - 48.18).abs() < 0.3, "{:?}", branch.pos);
    }

    #[test]
    fn a_roads_lanes_become_paths_of_their_own() {
        let road = "Splines\\Marcel\\str_2spur_8m_borkumer1.sli";
        let Some(mut assets) = installed_spline(road) else { return };
        let omsi = crate::install::find_omsi_root(&Default::default()).unwrap();
        let content = std::env::temp_dir().join(format!("om-detach-{}", std::process::id()));
        std::fs::create_dir_all(&content).unwrap();
        let roots = crate::install::Roots { omsi, content: content.clone() };
        for f in ["Splines\\Marcel\\invis_street.sli", "Splines\\invis_sidewalk.sli"] {
            while assets.spline(f).is_none() && !assets.spline_failed(f) {
                assets.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 80.0, 80.0, 30.0, 60.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].file = road.into();
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].radius = -150.0;
        let mut ed = Editor::default();
        ed.open(doc);
        // the lanes as they are, as (where traffic starts, where it ends)
        let ways = |ed: &Editor, assets: &mut Assets| -> Vec<(i64, i64, i64, i64)> {
            let doc = ed.doc.as_ref().unwrap();
            let r = |v: f64| (v * 10.0).round() as i64;
            let mut out = Vec::new();
            for l in crate::lanes::near(doc, assets, DVec2::new(100.0, 100.0), 100.0).0 {
                let (a, b) = (l.pts[0], *l.pts.last().unwrap());
                let fwd = (r(a.x), r(a.y), r(b.x), r(b.y));
                let back = (r(b.x), r(b.y), r(a.x), r(a.y));
                match l.direction {
                    0 => out.push(fwd),
                    1 => out.push(back),
                    _ => {
                        out.push(fwd);
                        out.push(back);
                    }
                }
            }
            out.sort();
            out
        };
        let before = ways(&ed, &mut assets);
        assert!(before.len() >= 4, "{before:?}");
        ed.select(Some(Sel::Spline((0, 0), 3)));
        ed.detach_lanes(&mut assets, &roots);
        for _ in 0..200 {
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let after = ways(&ed, &mut assets);
        assert_eq!(before.len(), after.len(), "{before:?}\n{after:?}");
        for (b, a) in before.iter().zip(&after) {
            // (within a decimetre or two: the lanes' ends are the same)
            assert!((b.0 - a.0).abs() <= 2 && (b.1 - a.1).abs() <= 2 && (b.2 - a.2).abs() <= 2 && (b.3 - a.3).abs() <= 2, "{b:?} vs {a:?}");
        }
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        assert!(t.splines.iter().find(|s| s.id == 3).unwrap().file.contains("no_lanes"));
        assert!(t.splines.len() >= 5);
        assert!(content.join("Splines/openMapper/str_2spur_8m_borkumer1_no_lanes.sli").is_file());
        // one undo gives the road its lanes back
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        assert_eq!(ways(&ed, &mut assets), before);
        let _ = std::fs::remove_dir_all(content);
    }

    #[test]
    fn the_water_brush_digs_a_bed_and_lays_water_on_the_tile() {
        let mut assets = Assets::new(std::path::PathBuf::new(), std::env::temp_dir().join("openmapper-test-empty-content"));
        let mut ed = editor();
        ed.set_tool(Tool::Terrain);
        ed.terrain_mode = TerrainMode::Water;
        ed.brush_radius = 12.0;
        ed.water_depth = 2.0;
        ed.water_level = Some(-0.3);
        ed.history.begin("water");
        for _ in 0..40 {
            ed.brush_terrain(&mut assets, DVec2::new(100.0, 100.0), 0.05);
        }
        ed.history.commit();
        let doc = ed.doc.as_ref().unwrap();
        assert!(doc.ground(DVec2::new(100.0, 100.0)) < -2.0, "{}", doc.ground(DVec2::new(100.0, 100.0)));
        // (beyond the brush the ground is as it was)
        assert!(doc.ground(DVec2::new(130.0, 100.0)).abs() < 1e-3);
        assert_eq!(doc.tiles[&(0, 0)].water, Some([-0.3; 4]));
    }

    #[test]
    fn a_lake_clicked_round_is_dug_with_sloping_banks_and_a_river_along_its_line() {
        let mut ed = editor();
        ed.set_tool(Tool::Terrain);
        ed.terrain_mode = TerrainMode::Water;
        ed.water_depth = 2.0;
        ed.water_bank = 10.0;
        ed.water_shape = WaterShape::Lake;
        ed.water_pts = vec![DVec3::new(60.0, 60.0, 0.0), DVec3::new(140.0, 60.0, 0.0), DVec3::new(140.0, 140.0, 0.0), DVec3::new(60.0, 140.0, 0.0)];
        assert!(ed.make_water());
        let doc = ed.doc.as_ref().unwrap();
        // the middle is the bed under the level (0 - 0.3), the shore is at the level, far out is as it was
        assert!((doc.ground(DVec2::new(100.0, 100.0)) - -2.3).abs() < 0.2, "{}", doc.ground(DVec2::new(100.0, 100.0)));
        assert!(doc.ground(DVec2::new(200.0, 100.0)).abs() < 1e-3);
        assert_eq!(doc.tiles[&(0, 0)].water, Some([-0.3; 4]));
        // a river: dug along its line, not beside it
        ed.water_shape = WaterShape::River;
        ed.river_width = 12.0;
        ed.water_pts = vec![DVec3::new(20.0, 220.0, 0.0), DVec3::new(280.0, 220.0, 0.0)];
        assert!(ed.make_water());
        let doc = ed.doc.as_ref().unwrap();
        assert!(doc.ground(DVec2::new(150.0, 220.0)) < -1.5);
        assert!(doc.ground(DVec2::new(150.0, 260.0)).abs() < 1e-3);
        // one undo takes the river away
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        assert!(ed.doc.as_ref().unwrap().ground(DVec2::new(150.0, 220.0)).abs() < 1e-3);
    }

    #[test]
    fn an_ai_path_is_bent_and_moved_not_lost() {
        let file = "Splines\\Marcel\\invis_street.sli";
        let Some(mut assets) = installed_spline(file) else { return };
        let lib = Library::default();
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 50.0, 50.0, 0.0, 40.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].file = file.into();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Logic);
        ed.set_logic(LogicMode::Paths);
        ed.path_edit = true;
        let n = |ed: &Editor| ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).count();
        let report = |ed: &Editor, what: &str| {
            let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
            let v: Vec<String> = t.splines.iter().filter(|s| !s.deleted).map(|s| format!("[{} ({:.1},{:.1}) h{:.0} l{:.1} r{:.0} {}>{}]", s.id, s.pos[0], s.pos[1], s.heading, s.length, s.radius, s.prev_id, s.next_id)).collect();
            eprintln!("{what}: {}", v.join(" "));
        };
        // a click on the path picks it (as the app does: pick under the cursor, then the press)
        ed.cursor = Some(DVec3::new(50.5, 70.0, 0.0));
        ed.hover = ed.pick(&mut assets, DVec3::new(50.5, 70.0, 80.0), DVec3::new(0.0, 0.0, -1.0), Some(DVec3::new(50.5, 70.0, 0.0)));
        eprintln!("hover {:?}", ed.hover);
        ed.left_press(&mut assets, &lib, false);
        ed.tick(&mut assets, 0.0, false);
        eprintln!("selected {:?}", ed.selected);
        // drag its far end sideways
        ed.cursor = Some(DVec3::new(50.0, 90.0, 0.0));
        let found = ed.handle_at_ray(&mut assets, DVec3::new(50.0, 90.0, 40.0), -DVec3::Z);
        ed.hover_handle = found.map(|f| f.1);
        ed.hover_handle_of = found.map(|f| f.0);
        eprintln!("handle {:?}", ed.hover_handle);
        ed.left_press(&mut assets, &lib, false);
        for x in [52.0, 56.0, 62.0] {
            ed.hover_point(&mut assets, &lib, DVec3::new(x, 88.0, 0.0));
            ed.tick(&mut assets, 0.05, true);
        }
        ed.tick(&mut assets, 0.0, false);
        report(&ed, "after the end drag");
        assert_eq!(n(&ed), 1, "still one path");
        // drag its middle
        ed.select(Some(Sel::Spline((0, 0), 3)));
        let mid = ed.handles().iter().find(|h| h.1 == Handle::Mid).map(|h| h.2).unwrap();
        ed.cursor = Some(mid);
        let found = ed.handle_at_ray(&mut assets, mid + DVec3::new(0.0, 0.0, 40.0), -DVec3::Z);
        ed.hover_handle = found.map(|f| f.1);
        ed.hover_handle_of = found.map(|f| f.0);
        ed.left_press(&mut assets, &lib, false);
        for d in [3.0, 8.0, 12.0] {
            ed.hover_point(&mut assets, &lib, DVec3::new(mid.x - d, mid.y, 0.0));
            ed.tick(&mut assets, 0.05, true);
        }
        ed.tick(&mut assets, 0.0, false);
        report(&ed, "after the middle drag");
        assert!(n(&ed) >= 1);
        // the whole path moved by a drag of its body (the path is picked whole, as a click picks it)
        let before = ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).map(|s| s.pos[0]).fold(f64::MAX, f64::min);
        ed.select_chain((0, 0), 3);
        let at = ed.doc.as_ref().map(|d| omsi_geometry::SplineCurve::from_map(d.spline((0, 0), 3).unwrap(), tile_origin((0, 0))).point_at(5.0)).unwrap();
        ed.cursor = Some(at);
        ed.hover_handle = None;
        ed.hover = Some(Sel::Spline((0, 0), 3));
        ed.last_click = None;
        ed.left_press(&mut assets, &lib, false);
        for d in [2.0, 5.0, 9.0] {
            ed.hover_point(&mut assets, &lib, DVec3::new(at.x + d, at.y, 0.0));
            ed.tick(&mut assets, 0.05, true);
        }
        ed.tick(&mut assets, 0.0, false);
        report(&ed, "after the body drag");
        let after = ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).map(|s| s.pos[0]).fold(f64::MAX, f64::min);
        assert!(after > before + 5.0, "{before} -> {after}");
        // a point put in with Ctrl + click, then that point dragged
        ed.split_path_at((0, 0), 3, DVec2::new(55.0, 70.0));
        report(&ed, "after a point put in");
        assert_eq!(n(&ed), 3);
    }

    #[test]
    fn an_ai_path_has_a_grip_at_every_point_and_a_point_can_be_taken_out() {
        let file = "Splines\\Marcel\\invis_street.sli";
        let Some(mut assets) = installed_spline(file) else { return };
        let lib = Library::default();
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 50.0, 50.0, 0.0, 60.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].file = file.into();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Logic);
        ed.set_logic(LogicMode::Paths);
        ed.path_edit = true;
        // two points put in: three pieces
        ed.split_path_at((0, 0), 3, DVec2::new(50.0, 70.0));
        let second = ed.doc.as_ref().unwrap().spline((0, 0), 3).unwrap().next_id;
        ed.split_path_at((0, 0), second, DVec2::new(50.0, 90.0));
        let pieces = ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).count();
        assert_eq!(pieces, 3);
        // a click on the path picks all of it, a grip for each point (4) and each piece's middle (3)
        ed.hover = Some(Sel::Spline((0, 0), 3));
        ed.last_click = None;
        ed.left_press(&mut assets, &lib, false);
        ed.tick(&mut assets, 0.0, false);
        assert_eq!(ed.selection().len(), 3);
        assert_eq!(ed.handles().iter().filter(|h| h.1 != Handle::Mid).count(), 4);
        assert_eq!(ed.handles().iter().filter(|h| h.1 == Handle::Mid).count(), 3);
        // the first joint picked (a press on its grip) and taken out with Del: two pieces, still one path
        let joint = ed.handles().iter().find(|h| h.1 == Handle::End).unwrap().clone();
        ed.cursor = Some(joint.2);
        ed.hover_handle = Some(joint.1);
        ed.hover_handle_of = Some(joint.0);
        ed.left_press(&mut assets, &lib, false);
        ed.tick(&mut assets, 0.0, false);
        assert!(ed.path_point.is_some());
        ed.delete_selected();
        let doc = ed.doc.as_ref().unwrap();
        let left: Vec<_> = doc.tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).collect();
        assert_eq!(left.len(), 2, "{:?}", left.iter().map(|s| (s.id, s.prev_id, s.next_id, s.length)).collect::<Vec<_>>());
        assert!((left.iter().map(|s| s.length).sum::<f64>() - 60.0).abs() < 1.0);
        assert!(left.iter().any(|s| s.next_id != 0) && left.iter().any(|s| s.prev_id != 0));
        // undone, the path has its three pieces
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        assert_eq!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.iter().filter(|s| !s.deleted).count(), 3);
    }

    #[test]
    fn flattening_the_terrain_levels_every_tile_and_lays_the_roads_on_it() {
        let mut ed = editor();
        {
            let t = ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap();
            for (i, h) in t.terrain.heights.iter_mut().enumerate() {
                *h = 5.0 + (i % 7) as f32;
            }
            t.tile.splines[0].pos[2] = 9.0;
        }
        let n = ed.flatten_terrain();
        assert!(n >= 1);
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)];
        let first = t.terrain.heights[0];
        assert!(t.terrain.heights.iter().all(|h| (*h - first).abs() < 1e-6));
        assert!((first - 8.0).abs() < 0.2, "{first}");
        assert!((t.tile.splines[0].pos[2] - first as f64).abs() < 0.2, "{}", t.tile.splines[0].pos[2]);
        // one undo brings the relief back
        let d = ed.doc.as_mut().unwrap();
        ed.history.undo(d);
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)];
        assert!(t.terrain.heights.iter().any(|h| (*h - 5.0).abs() < 1e-6));
    }

    #[test]
    fn a_press_on_a_roads_own_lane_asks_for_its_lanes_to_be_made_editable() {
        let road = "Splines\\Marcel\\str_2spur_8m_borkumer1.sli";
        let Some(mut assets) = installed_spline(road) else { return };
        let lib = Library::default();
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 50.0, 50.0, 0.0, 60.0, 0, 0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0].file = road.into();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Logic);
        ed.set_logic(LogicMode::Paths);
        ed.path_edit = true;
        // (a car lane of this road lies 2 m right of its centre line: east of x 50)
        ed.cursor = Some(DVec3::new(52.0, 80.0, 0.0));
        ed.hover = None;
        ed.hover_handle = None;
        ed.left_press(&mut assets, &lib, false);
        let (pieces, at) = ed.lane_detach_request.clone().expect("a request");
        assert_eq!(pieces, vec![((0, 0), 3)]);
        assert!((at - DVec2::new(52.0, 80.0)).length() < 1e-9);
    }

    #[test]
    fn each_track_gets_the_switch_that_looks_like_it() {
        let Some(mut assets) = installed_spline("Splines\\Ruede\\rail_wood_S49.sli") else {
            eprintln!("no OMSI 2 here: skipped");
            return;
        };
        let tracks = ["Splines\\Ruede\\rail_wood_S49.sli", "Splines\\Ruede\\rail_concrete_S49.sli", "Splines\\Ruede\\rail_wood_S41_kleinprofil_0.sli", "Splines\\Ruede\\rail_concrete_01.sli"];
        let names = ["Beton", "Klp_0_0", "Klp_0_1", "Klp_1_0", "Klp_1_1", "S49_0_0", "S49_0_1", "S49_1_0", "S49_1_1"];
        let mut ed = Editor::default();
        ed.switch_catalogue = names.iter().map(|n| crate::switches::Candidate { rel: format!("Sceneryobjects\\Railroad_RUE\\EW190_L_{n}.sco"), left: true }).collect();
        // (everything loaded first)
        for _ in 0..800 {
            let mut all = true;
            for c in ed.switch_catalogue.clone() {
                all &= assets.object(&c.rel).is_some() || assets.object_failed(&c.rel);
            }
            for t in tracks {
                all &= assets.spline(t).is_some() || assets.spline_failed(t);
            }
            if all {
                break;
            }
            assets.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ed.best_switch(&mut assets, tracks[0], true).ends_with("EW190_L_S49_0_0.sco"));
        assert!(ed.best_switch(&mut assets, tracks[1], true).ends_with("EW190_L_Beton.sco"));
        assert!(ed.best_switch(&mut assets, tracks[2], true).ends_with("EW190_L_Klp_0_0.sco"));
        assert!(ed.best_switch(&mut assets, tracks[3], true).ends_with("EW190_L_Beton.sco"));
    }

    #[test]
    fn a_road_drawn_on_from_a_road_of_another_width_is_tapered() {
        let narrow = "Splines\\Marcel\\str_2spur_8m_borkumer1.sli";
        let wide = "Splines\\Marcel\\str_2spur_13m_Omnibushof.sli";
        let Some(mut assets) = installed_spline(narrow) else { return };
        while assets.spline(wide).is_none() && !assets.spline_failed(wide) { assets.poll(); std::thread::sleep(std::time::Duration::from_millis(10)); }
        if assets.spline(wide).is_none() {
            return;
        }
        let content = std::env::temp_dir().join(format!("om-adapter-{}", std::process::id()));
        std::fs::create_dir_all(&content).unwrap();
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("Adapter");
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(narrow.into());
        let lib = Library::default();
        let click = |ed: &mut Editor, a: &mut Assets, x: f64, y: f64| {
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            ed.plan_road(a);
            ed.road_click(a, &lib);
            ed.tick(a, 0.0, false);
        };
        click(&mut ed, &mut assets, 60.0, 150.0);
        click(&mut ed, &mut assets, 150.0, 150.0);
        ed.right_click();
        ed.road_type = Some(wide.into());
        ed.inherit_type = false;
        click(&mut ed, &mut assets, 150.0, 150.0);
        click(&mut ed, &mut assets, 240.0, 150.0);
        ed.right_click();
        for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        let j: Vec<_> = t.objects.iter().filter(|o| o.file.contains("junction_")).collect();
        assert_eq!(j.len(), 1, "one plate between the two widths");
        let centre = DVec2::new(j[0].pos[0], j[0].pos[1]);
        assert_eq!(crate::selection::tests::free_ends_near(doc, centre, 25.0), 2, "two arms");
        let _ = std::fs::remove_dir_all(&content);
    }

    /// Two road types made here, the second with a pocket lane, in a scratch content folder.
    fn pocket_fixture(tag: &str) -> (Assets, std::path::PathBuf, String, String) {
        let dir = std::env::temp_dir().join(format!("om-pocket-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tex = dir.join("asph.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 80, 80, 255])).save(&tex).unwrap();
        let side = dir.join("side.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([150, 150, 150, 255])).save(&side).unwrap();
        let content = dir.join("content");
        let plain = crate::road_maker::RoadSpec { name: "Plain".into(), roadway_tex: tex.clone(), footway_tex: side.clone(), ..Default::default() };
        let bay = crate::road_maker::RoadSpec { name: "Bay".into(), pocket: 3.0, roadway_tex: tex, footway_tex: side, ..Default::default() };
        let (a, b) = (plain.write(&content).unwrap(), bay.write(&content).unwrap());
        let mut assets = Assets::new(dir.join("omsi"), content.clone());
        for rel in [&a, &b] {
            assert!(assets.spline_wait(rel, 5000).is_some(), "{rel} loads");
        }
        (assets, dir, a, b)
    }

    #[test]
    fn a_bus_pocket_retypes_a_stretch_and_tapers_its_ends() {
        let (mut assets, dir, plain, _) = pocket_fixture("lay");
        let mut doc = flat_map(1);
        doc.dir = dir.join("content").join("maps").join("Pocket");
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: plain.clone(), id: 1, pos: [150.0, 80.0, 0.0], heading: 0.0, length: 100.0, ..Default::default() });
        let mut ed = Editor::default();
        ed.content_dir = Some(dir.join("content"));
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.bus_pocket = true;
        ed.pocket_len = 12.0;
        ed.cursor = Some(DVec3::new(150.0, 125.0, 0.0));
        ed.pocket_click(&mut assets);
        for _ in 0..30 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        // (the road was made in the road maker: the bay is that road with a pocket lane, not the type picked)
        let pocket: Vec<_> = t.splines.iter().filter(|s| !s.deleted && !s.file.eq_ignore_ascii_case(&plain)).collect();
        assert!(!pocket.is_empty(), "a stretch of the pocket's type");
        assert!(pocket.iter().all(|s| s.file.to_lowercase().contains("bay")), "{:?}", pocket.iter().map(|s| &s.file).collect::<Vec<_>>());
        // the full-width bay (the tapers' plates take the rest of the stretch)
        let len: f64 = pocket.iter().map(|s| s.length).sum();
        assert!((len - 12.0).abs() < 0.5, "{len}");
        assert!(t.splines.iter().any(|s| !s.deleted && s.file.eq_ignore_ascii_case(&plain)), "the road stays beside it");
        let plates = t.objects.iter().filter(|o| o.file.contains("junction_")).count();
        assert_eq!(plates, 2, "a tapering plate at each end");
        // undo gives the road back whole
        ed.undo();
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        assert_eq!(t.splines.iter().filter(|s| !s.deleted).count(), 1);
        assert_eq!(t.objects.iter().filter(|o| o.file.contains("junction_")).count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bay is the road type's own text widened along the kerb its traffic keeps to (a road
    /// of the installation, with no spec beside it, as well): the lanes stay on the road's
    /// line, and the other side of the road stays where it was.
    #[test]
    fn a_bus_pocket_widens_the_roads_own_type_on_its_kerb_side() {
        let (mut assets, dir, plain, _) = pocket_fixture("lanes");
        let content = dir.join("content");
        // (the road is as a road of the installation: no spec beside it)
        let _ = std::fs::remove_file(content.join("Splines").join("openMapper").join("Plain.road.json"));
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("Pocket");
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: plain.clone(), id: 1, pos: [150.0, 80.0, 0.0], heading: 0.0, length: 100.0, ..Default::default() });
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.bus_pocket = true;
        ed.pocket_len = 12.0;
        ed.cursor = Some(DVec3::new(150.0, 125.0, 0.0));
        ed.pocket_click(&mut assets);
        for _ in 0..30 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        let bay: Vec<_> = t.splines.iter().filter(|s| !s.deleted && s.file.to_lowercase().contains("_bay_right")).collect();
        assert_eq!(bay.len(), 1, "{:?}", t.splines.iter().map(|s| &s.file).collect::<Vec<_>>());
        assert!((bay[0].pos[0] - 150.0).abs() < 0.01, "lanes stay on the road's line: {:?}", bay[0].pos);
        assert_eq!(t.objects.iter().filter(|o| o.file.contains("junction_")).count(), 2);
        // the road's own text, widened on its right only
        let own = omsi_cfg::vfs::read_text(&omsi_cfg::resolve_path(&content, &plain)).unwrap();
        let made = omsi_cfg::vfs::read_text(&omsi_cfg::resolve_path(&content, &bay[0].file)).unwrap();
        let m = assets.spline_wait(&bay[0].file, 5000).unwrap();
        let o = assets.spline_wait(&plain, 5000).unwrap();
        let (me, oe) = (crate::pocket::extent(&m).unwrap(), crate::pocket::extent(&o).unwrap());
        assert!((me.0 - oe.0).abs() < 1e-3, "the left side stays: {me:?} {oe:?}");
        assert!((me.1 - oe.1 - 3.0).abs() < 1e-3, "the right side is 3 m wider: {me:?} {oe:?}");
        assert!(made.len() > own.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A custom junction drawn as a T of the road type, baked to an object, is used where
    /// roads of that type meet like it, the roads bent onto its helpers.
    #[test]
    fn a_custom_junction_stands_in_for_the_plate_where_roads_meet_as_it_was_drawn() {
        use crate::cjunction::{Arm, LSeg, Layout, Piece};
        let (mut assets, dir, plain, _) = pocket_fixture("custom");
        let content = dir.join("content");
        let roots = crate::install::Roots { omsi: dir.join("omsi"), content: content.clone() };
        let line = |x: f64, y: f64, h: f64, len: f64| LSeg { x, y, heading: h, radius: 0.0, length: len };
        let mut layout = Layout {
            name: "My T".into(),
            pieces: vec![Piece { file: plain.clone(), segs: vec![line(-14.0, 0.0, 90.0, 28.0)], lift: 0.0, hidden: false, auto: false }, Piece { file: plain.clone(), segs: vec![line(0.0, 0.0, 0.0, 14.0)], lift: 0.0, hidden: false, auto: false }],
            arms: vec![Arm { x: -14.0, y: 0.0, out: 270.0, road: plain.clone(), ..Default::default() }, Arm { x: 14.0, y: 0.0, out: 90.0, road: plain.clone(), ..Default::default() }, Arm { x: 0.0, y: 14.0, out: 0.0, road: plain.clone(), ..Default::default() }],
            auto: true,
            baked: None,
            ..Default::default()
        };
        layout.build.plate = false;
        let rel = crate::cjunction::bake(&roots, &layout, &mut assets, true).unwrap();
        layout.baked = Some(rel.clone());
        layout.save(&content).unwrap();
        // the object as the game reads it: three helpers, and the lanes of its pieces
        let sco = omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(&content, &rel)).unwrap();
        assert_eq!(sco.spline_helpers.len(), 3);
        assert_eq!(sco.paths.len(), 2 * 4, "four lanes (two car, two footway) along each piece");
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("Custom");
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(plain.clone());
        let lib = Library::default();
        let click = |ed: &mut Editor, a: &mut Assets, x: f64, y: f64| {
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            ed.plan_road(a);
            ed.road_click(a, &lib);
            ed.tick(a, 0.0, false);
        };
        click(&mut ed, &mut assets, 60.0, 150.0);
        click(&mut ed, &mut assets, 240.0, 150.0);
        ed.right_click();
        click(&mut ed, &mut assets, 150.0, 260.0);
        click(&mut ed, &mut assets, 150.0, 151.0);
        ed.right_click();
        for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        assert_eq!(t.objects.iter().filter(|o| o.file.contains("junction_")).count(), 0, "no plate of openMapper's own");
        let mine: Vec<_> = t.objects.iter().filter(|o| o.file == rel).collect();
        assert_eq!(mine.len(), 1, "the custom junction");
        // every helper has a road end on it: the roads were bent onto the arms
        let pts = crate::selection::helper_points(doc, (0, 0), mine[0].id, true, &sco.spline_helpers.iter().map(|h| [h.x, h.z, h.y, h.heading + 180.0]).collect::<Vec<_>>());
        assert_eq!(pts.len(), 3);
        for (p, _) in &pts {
            assert!(crate::selection::free_end_at(doc, p.truncate()).is_some(), "a road ends at {p:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Roads drawn across one another, one ending on another, or several ending at one place
    /// are cut back and given a plate, as the Roads tool does; the arms stay at the roads' outer ends.
    #[test]
    fn auto_junction_cuts_roads_back_and_makes_a_plate_as_the_roads_tool_does() {
        use crate::cjunction::{LSeg, Layout, Piece};
        let (mut assets, dir, plain, _) = pocket_fixture("plate-layout");
        let content = dir.join("content");
        let roots = crate::install::Roots { omsi: dir.join("omsi"), content: content.clone() };
        let line = |x: f64, y: f64, h: f64, len: f64| Piece { file: plain.clone(), segs: vec![LSeg { x, y, heading: h, radius: 0.0, length: len }], lift: 0.0, hidden: false, auto: false };
        // a crossing: two roads of 80 m through the middle
        let mut layout = Layout { name: "Plate".into(), pieces: vec![line(0.0, -40.0, 0.0, 80.0), line(-40.0, 0.0, 90.0, 80.0)], auto: true, ..Default::default() };
        let kinds = crate::cjunction::road_kinds(&layout, &mut assets);
        let (is_road, half) = (|f: &str| kinds.get(&f.to_lowercase()).map(|k| k.0).unwrap_or(false), |f: &str| kinds.get(&f.to_lowercase()).map(|k| k.1).unwrap_or(1.5));
        let r = crate::cjunction::Layout::resolve(&layout, &is_road, &half);
        assert_eq!(r.plates.len(), 1);
        assert_eq!(r.plates[0].len(), 4, "a road each way");
        assert_eq!(r.pieces.len(), 4, "each road cut in two");
        let room = crate::junction::radius_at(&[half(&plain)], 90.0);
        for a in &r.plates[0] {
            assert!((a.pos.length() - room).abs() < 0.5, "cut back by the room the Roads tool leaves: {} vs {room}", a.pos.length());
        }
        // a T: one road ends on the other's middle
        layout.pieces = vec![line(-40.0, 0.0, 90.0, 80.0), line(0.0, 0.0, 0.0, 40.0)];
        let r = layout.resolve(&is_road, &half);
        assert_eq!((r.plates.len(), r.plates[0].len(), r.pieces.len()), (1, 3, 3));
        // a star: three roads end at the origin
        layout.pieces = vec![line(0.0, 0.0, 0.0, 40.0), line(0.0, 0.0, 120.0, 40.0), line(0.0, 0.0, 240.0, 40.0)];
        let r = layout.resolve(&is_road, &half);
        assert_eq!((r.plates.len(), r.plates[0].len()), (1, 3));
        // the arms are where the drawn roads end outside, none inside
        layout.sync_arms(&is_road, &half);
        assert_eq!(layout.arms.len(), 3);
        // saved: the object has the plate's lanes besides the roads'
        layout.pieces = vec![line(0.0, -40.0, 0.0, 80.0), line(-40.0, 0.0, 90.0, 80.0)];
        layout.sync_arms(&is_road, &half);
        let with = crate::cjunction::bake(&roots, &layout, &mut assets, true).unwrap();
        let sco = omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(&content, &with)).unwrap();
        layout.build.plate = false;
        layout.baked = None;
        layout.name = "No plate".into();
        let without = crate::cjunction::bake(&roots, &layout, &mut assets, true).unwrap();
        let bare = omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(&content, &without)).unwrap();
        assert_eq!(sco.spline_helpers.len(), 4);
        assert!(sco.paths.len() > bare.paths.len(), "the plate's lanes join the roads' ({} against {})", sco.paths.len(), bare.paths.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A custom junction drawn as a T of the road type, baked to an object, is used where
    /// roads of that type meet like it, the roads bent onto its helpers.
    #[test]
    fn a_complex_junction_is_put_on_the_map_as_splines_in_one_undo_step() {
        use crate::cjunction::{LSeg, Layout, Piece};
        let mut doc = flat_map(1);
        doc.dir = std::env::temp_dir().join("om-complex");
        let mut ed = Editor::default();
        ed.open(doc);
        let layout = Layout { name: "C".into(), pieces: vec![Piece { file: "Splines\\a.sli".into(), segs: vec![LSeg { x: -8.0, y: 0.0, heading: 90.0, radius: 0.0, length: 16.0 }], lift: 0.0, hidden: false, auto: false }, Piece { file: "Splines\\b.sli".into(), segs: vec![LSeg { x: 0.0, y: -8.0, heading: 0.0, radius: 0.0, length: 16.0 }], lift: 0.02, hidden: false, auto: false }], ..Default::default() };
        let mut assets = Assets::new(std::env::temp_dir().join("om-complex-omsi"), std::env::temp_dir().join("om-complex-content"));
        ed.place_layout(&mut assets, &layout, DVec2::new(150.0, 150.0), 0.0);
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        assert_eq!(t.splines.len(), 2);
        assert!(t.splines.iter().all(|s| s.prev_id == 0 && s.next_id == 0), "separate pieces: nothing joined");
        assert_eq!(ed.selection().len(), 2, "all of it selected, to move together");
        ed.undo();
        assert!(ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.is_empty());
    }

    /// The roadway of a junction lies as high as the roads' surface (a road type drawn above
    /// the line it is laid along), not under it.
    #[test]
    fn a_junctions_roadway_is_as_high_as_the_roads_surface() {
        let (mut assets, dir, plain, _) = pocket_fixture("plate");
        let content = dir.join("content");
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("Plate");
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(plain.clone());
        let lib = Library::default();
        let click = |ed: &mut Editor, a: &mut Assets, x: f64, y: f64| {
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            ed.plan_road(a);
            ed.road_click(a, &lib);
            ed.tick(a, 0.0, false);
        };
        click(&mut ed, &mut assets, 60.0, 150.0);
        click(&mut ed, &mut assets, 240.0, 150.0);
        ed.right_click();
        click(&mut ed, &mut assets, 150.0, 260.0);
        click(&mut ed, &mut assets, 150.0, 151.0);
        ed.right_click();
        for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        let j = t.objects.iter().find(|o| o.file.contains("junction_")).expect("a junction");
        let x = std::fs::read_to_string(omsi_cfg::resolve_path(&content, &j.file).with_file_name("model").join(std::path::Path::new(&j.file.replace('\\', "/")).file_name().unwrap()).with_extension("x")).unwrap();
        // the vertices' heights (the second number of each `x;y;z` line): none under the road surface
        let heights: Vec<f64> = x.lines().take_while(|l| !l.contains("3;")).filter_map(|l| { let p: Vec<&str> = l.trim().trim_end_matches([',', ';']).split(';').collect(); (p.len() == 3).then(|| p[1].parse().ok()).flatten() }).collect();
        assert!(!heights.is_empty());
        let low = heights.iter().cloned().fold(f64::MAX, f64::min);
        assert!(low > 0.09, "lowest vertex at {low}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_road_into_another_makes_a_t_junction() {
        let road_s = std::env::var("TJ_MAIN").unwrap_or("Splines\\Marcel\\str_2spur_8m_borkumer1.sli".into());
        let road = road_s.as_str();
        let Some(mut assets) = installed_spline(road) else { return };
        let content = std::env::temp_dir().join(format!("om-tj-{}", std::process::id()));
        std::fs::create_dir_all(&content).unwrap();
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("TJ");
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(road.into());
        let lib = Library::default();
        let click = |ed: &mut Editor, a: &mut Assets, x: f64, y: f64| {
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            ed.plan_road(a);
            ed.road_click(a, &lib);
            ed.tick(a, 0.0, false);
        };
        click(&mut ed, &mut assets, 60.0, 150.0);
        click(&mut ed, &mut assets, 240.0, 150.0);
        ed.right_click();
        if let Ok(b) = std::env::var("TJ_BRANCH") {
            while assets.spline(&b).is_none() && !assets.spline_failed(&b) { assets.poll(); std::thread::sleep(std::time::Duration::from_millis(10)); }
            ed.road_type = Some(b);
            ed.inherit_type = false;
        }
        click(&mut ed, &mut assets, 150.0, 260.0);
        click(&mut ed, &mut assets, 150.0, 151.0);
        ed.right_click();
        for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        // one junction at the meeting point, three roads ending at its edge
        let j: Vec<_> = t.objects.iter().filter(|o| o.file.contains("junction_")).collect();
        assert_eq!(j.len(), 1);
        let centre = DVec2::new(j[0].pos[0], j[0].pos[1]);
        let ends = crate::selection::tests::free_ends_near(doc, centre, 25.0);
        assert_eq!(ends, 3, "three arms");
        // its file lost (the map moved without it): made again for the roads facing it
        let (old, old_file) = (j[0].id, j[0].file.clone());
        let file = omsi_cfg::resolve_path(&content, &old_file);
        std::fs::remove_file(&file).unwrap();
        let Some(mut fresh) = installed_spline(road) else { return };
        assert!(ed.remake_junction(old));
        for _ in 0..50 { fresh.poll(); ed.tick(&mut fresh, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let doc = ed.doc.as_ref().unwrap();
        let j: Vec<_> = doc.tiles[&(0, 0)].tile.objects.iter().filter(|o| o.file.contains("junction_")).collect();
        assert_eq!(j.len(), 1);
        assert_ne!(j[0].id, old);
        assert!(omsi_cfg::resolve_path(&content, &j[0].file).is_file());
        assert_eq!(crate::selection::tests::free_ends_near(doc, DVec2::new(j[0].pos[0], j[0].pos[1]), 25.0), 3);
        // every junction made again at once (one step), still one, for the same three roads
        assert_eq!(ed.remake_all_junctions(), 1);
        for _ in 0..50 { fresh.poll(); ed.tick(&mut fresh, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let doc = ed.doc.as_ref().unwrap();
        let j: Vec<_> = doc.tiles[&(0, 0)].tile.objects.iter().filter(|o| o.file.contains("junction_")).collect();
        assert_eq!(j.len(), 1);
        assert_eq!(crate::selection::tests::free_ends_near(doc, DVec2::new(j[0].pos[0], j[0].pos[1]), 25.0), 3);
        let _ = std::fs::remove_dir_all(content);
    }

    #[test]
    fn a_road_drawn_out_of_a_junction_becomes_a_fourth_arm() {
        let road = "Splines\\Marcel\\str_2spur_8m_borkumer1.sli";
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let content = std::env::temp_dir().join(format!("om-tjout-{}", std::process::id()));
        std::fs::create_dir_all(&content).unwrap();
        let mut assets = Assets::new(omsi, content.clone());
        for _ in 0..500 { if assets.spline(road).is_some() { break; } assets.poll(); std::thread::sleep(std::time::Duration::from_millis(10)); }
        if assets.spline(road).is_none() { return }
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("TJO");
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(road.into());
        let lib = Library::default();
        let click = |ed: &mut Editor, a: &mut Assets, x: f64, y: f64| {
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            ed.plan_road(a);
            ed.road_click(a, &lib);
            ed.tick(a, 0.0, false);
        };
        click(&mut ed, &mut assets, 60.0, 150.0);
        click(&mut ed, &mut assets, 240.0, 150.0);
        ed.right_click();
        click(&mut ed, &mut assets, 150.0, 260.0);
        click(&mut ed, &mut assets, 150.0, 151.0);
        ed.right_click();
        for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let centre = {
            let doc = ed.doc.as_ref().unwrap();
            let o = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.file.contains("junction_")).expect("a junction");
            DVec2::new(o.pos[0], o.pos[1])
        };
        let r = {
            let mut r = None;
            for _ in 0..500 { assets.poll(); ed.tick(&mut assets, 0.0, false); r = ed.plate_at(&mut assets, centre).map(|p| p.2); if r.is_some() { break; } std::thread::sleep(std::time::Duration::from_millis(10)); }
            r.expect("the plate")
        };
        eprintln!("plate at {centre:?} r {r}");
        // out of the open (northern) side: from the plate's middle, and from near its edge
        for (dx, from) in [(0.0, 0.0), (0.0, -r * 0.7), (r * 0.9, -r * 0.5)] {
            click(&mut ed, &mut assets, centre.x + dx, centre.y + from);
            let start = ed.road_start.expect("a start").0.pos;
            assert!((start.truncate() - centre).length() < 0.5, "from the plate's middle, not {start:?}");
            click(&mut ed, &mut assets, centre.x, centre.y - 90.0);
            ed.right_click();
            for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
            let doc = ed.doc.as_ref().unwrap();
            let j = doc.tiles[&(0, 0)].tile.objects.iter().filter(|o| o.file.contains("junction_")).count();
            let ends = crate::selection::tests::free_ends_near(doc, centre, r + 15.0);
            eprintln!("from {from}: {j} junctions, {ends} free ends, {} splines", doc.tiles[&(0, 0)].tile.splines.len());
            assert_eq!(j, 1);
            assert_eq!(ends, 4, "a fourth arm from {from}");
            ed.undo();
        }
        let _ = std::fs::remove_dir_all(content);
    }

    #[test]
    fn a_road_drawn_into_a_junction_becomes_a_fourth_arm() {
        let road = "Splines\\Marcel\\str_3spur_12m_Stresowstr2.sli";
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let content = std::env::temp_dir().join(format!("om-tjin-{}", std::process::id()));
        std::fs::create_dir_all(&content).unwrap();
        let mut assets = Assets::new(omsi, content.clone());
        for _ in 0..500 { if assets.spline(road).is_some() { break; } assets.poll(); std::thread::sleep(std::time::Duration::from_millis(10)); }
        if assets.spline(road).is_none() { return }
        let mut doc = flat_map(1);
        doc.dir = content.join("maps").join("TJI");
        let mut ed = Editor::default();
        ed.content_dir = Some(content.clone());
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(road.into());
        let lib = Library::default();
        let click = |ed: &mut Editor, a: &mut Assets, x: f64, y: f64| {
            ed.cursor = Some(DVec3::new(x, y, 0.0));
            ed.plan_road(a);
            ed.road_click(a, &lib);
            ed.tick(a, 0.0, false);
        };
        click(&mut ed, &mut assets, 52.6, 127.5);
        click(&mut ed, &mut assets, 247.4, 172.5);
        ed.right_click();
        click(&mut ed, &mut assets, 141.6, 265.2);
        click(&mut ed, &mut assets, 149.0, 150.0);
        ed.right_click();
        for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
        let centre = {
            let doc = ed.doc.as_ref().unwrap();
            let o = doc.tiles[&(0, 0)].tile.objects.iter().find(|o| o.file.contains("junction_")).expect("a junction");
            DVec2::new(o.pos[0], o.pos[1])
        };
        let r = {
            let mut r = None;
            for _ in 0..500 { assets.poll(); ed.tick(&mut assets, 0.0, false); r = ed.plate_at(&mut assets, centre).map(|p| p.2); if r.is_some() { break; } std::thread::sleep(std::time::Duration::from_millis(10)); }
            r.expect("the plate")
        };
        eprintln!("plate at {centre:?} r {r}");
        eprintln!("T: {} splines, {} objects", ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.splines.len(), ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile.objects.len());
        // into the open (southern) side from outside, to the plate's middle and to near its edge
        for (dx, to) in [(0.0, -r * 0.7), (0.0, -r * 0.8), (0.0, -r * 0.9), (0.0, -r), (3.0, -r * 1.2), (0.0, -r * 1.6)] {
            click(&mut ed, &mut assets, centre.x - 6.0, centre.y - 90.0);
            click(&mut ed, &mut assets, centre.x + dx, centre.y + to);
            ed.right_click();
            for _ in 0..50 { assets.poll(); ed.tick(&mut assets, 0.0, false); std::thread::sleep(std::time::Duration::from_millis(10)); }
            let doc = ed.doc.as_ref().unwrap();
            let j = doc.tiles[&(0, 0)].tile.objects.iter().filter(|o| o.file.contains("junction_")).count();
            let ends = crate::selection::tests::free_ends_near(doc, centre, r + 15.0);
            eprintln!("to {to}: {j} junctions, {ends} free ends, {} splines", doc.tiles[&(0, 0)].tile.splines.len());
            assert_eq!(j, 1);
            assert_eq!(ends, 4, "a fourth arm");
            ed.undo();
        }
        let _ = std::fs::remove_dir_all(content);
    }

    #[test]
    fn a_switch_on_a_bending_track_is_joined_again_gently() {
        let rail = "Splines\\Ruede\\rail_concrete_01.sli";
        let Some(mut assets) = installed_spline(rail) else { return };
        let sw = crate::switches::object_for(rail, true);
        for s in [sw.clone(), crate::switches::object_for(rail, false)] { for _ in 0..500 { if assets.object(&s).is_some() { break; } assets.poll(); std::thread::sleep(std::time::Duration::from_millis(10)); } }
        let mut doc = flat_map(1);
        add_spline(&mut doc, (0, 0), 3, 150.0, 30.0, 0.0, 200.0, 0, 0);
        { let sp = &mut doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines[0]; sp.file = rail.into(); sp.radius = -300.0; }
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Roads);
        ed.road_type = Some(rail.into());
        ed.inherit_type = false;
        let lib = Library::default();
        let c = omsi_geometry::SplineCurve::from_map(ed.doc.as_ref().unwrap().spline((0,0),3).unwrap(), DVec2::ZERO);
        let p = c.point_at(80.0);
        ed.cursor = Some(p);
        ed.plan_road(&mut assets);
        ed.road_click(&mut assets, &lib);
        ed.cursor = Some(DVec3::new(p.x + 20.0, p.y + 60.0, 0.0));
        ed.plan_road(&mut assets);
        ed.road_click(&mut assets, &lib);
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        let o = t.objects.iter().find(|o| o.file.contains("EW190_R")).expect("a switch");
        // the main track goes on from the straight heel, tangent, and bends back to its own
        // curve no tighter than a track should
        let model = assets.object(&o.file).unwrap();
        let shape = crate::switches::Shape::of(&model).unwrap();
        let (heel, hh) = shape.world(shape.straight, DVec3::new(o.pos[0], o.pos[1], 0.0), o.rot[0]);
        let first = t.splines.iter().find(|s| (DVec2::new(s.pos[0], s.pos[1]) - heel.truncate()).length() < 0.05).expect("a track from the heel");
        assert!((roads::norm_deg(first.heading - hh + 180.0) - 180.0).abs() < 0.1);
        let mut cur = first.clone();
        for _ in 0..8 {
            assert!(cur.radius == 0.0 || cur.radius.abs() >= 100.0, "{}", cur.radius);
            let Some(n) = doc.spline((0, 0), cur.next_id) else { break };
            if (n.radius + 300.0).abs() < 1e-6 {
                break;
            }
            cur = n.clone();
        }
    }

    /// A measurement, not a check: what a frame costs with a whole big map selected, and what
    /// a grab of it costs as the pointer goes (`cargo test what_a_big_selection_costs --
    /// --ignored --nocapture`). Selecting everything is what a map of 485 tiles answers
    /// slowly with: the parts timed here are the ones that must not look at the map's size
    /// for every thing selected.
    #[test]
    #[ignore]
    fn what_a_big_selection_costs() {
        let _map = crate::doc::one_map();
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2 installation: skipped");
            return;
        };
        let mut cfg = omsi.join("maps").join("NT_merged_v2").join("global.cfg");
        if !cfg.is_file() {
            cfg = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("world").join("NT_merged_v2").join("global.cfg");
        }
        if !cfg.is_file() {
            eprintln!("no NT_merged_v2: skipped");
            return;
        }
        let roots = crate::install::Roots { omsi: omsi.clone(), content: std::env::temp_dir().join("openmapper-pick-content") };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let mut assets = Assets::new(omsi, roots.content);
        let mut ed = Editor::default();
        ed.open(doc);
        ed.set_tool(Tool::Select);
        let now = || std::time::Instant::now();
        let t = now();
        ed.select_all();
        eprintln!("\nselect all: {} things in {:?}", ed.also.len() + 1, t.elapsed());
        let t = now();
        let s = ed.selection();
        eprintln!("the selection read: {} in {:?}", s.len(), t.elapsed());
        // (the middle of the map, high enough to see a good part of it)
        let (mut lo, mut hi) = (DVec2::new(f64::MAX, f64::MAX), DVec2::new(f64::MIN, f64::MIN));
        for k in ed.doc.as_ref().unwrap().tiles.keys() {
            let o = crate::doc::tile_origin(*k);
            lo = DVec2::new(lo.x.min(o.x), lo.y.min(o.y));
            hi = DVec2::new(hi.x.max(o.x), hi.y.max(o.y));
        }
        let mut cam = crate::camera::Camera::default();
        cam.target = ((lo + hi) * 0.5).extend(0.0);
        cam.distance = 900.0;
        let t = now();
        let x = ed.extras(&cam, &mut assets);
        eprintln!("extras: {:?} ({} overlay, {} ghosts)", t.elapsed(), x.overlay.len(), x.ghosts.len());
        // (which part of it: the selection read, the records looked up one by one, the paths)
        {
            let t = now();
            let s = ed.selection();
            let _more: std::collections::HashSet<Sel> = s.iter().skip(1).copied().collect();
            eprintln!("  the picked set built: {:?}", t.elapsed());
            let doc = ed.doc.as_ref().unwrap();
            let t = now();
            let mut n = 0;
            for s in &s {
                if let Sel::Spline(k, id) = *s {
                    if doc.spline(k, id).is_some() {
                        n += 1;
                    }
                }
            }
            eprintln!("  a spline looked up for each ({n}): {:?}", t.elapsed());
            let t = now();
            let mut n = 0;
            for s in &s {
                if let Sel::Object(k, id) = *s {
                    if doc.object(k, id).is_some() {
                        n += 1;
                    }
                }
            }
            eprintln!("  an object looked up for each ({n}): {:?}", t.elapsed());
            let t = now();
            let mut x2 = FrameExtras::default();
            ed.selection_extras(&mut x2, &cam, 1.0, &mut assets);
            eprintln!("  the selection's own extras: {:?}", t.elapsed());
        }
        ed.cursor = Some(cam.target);
        let t = now();
        ed.grab_begin((100.0, 100.0));
        eprintln!("grab begin: {:?}", t.elapsed());
        for i in 1..4 {
            let t = now();
            ed.grab_follow(cam.target + DVec3::new(i as f64, 0.0, 0.0), (100.0 + i as f32, 100.0));
            eprintln!("grab follow {i}: {:?}", t.elapsed());
        }
        ed.grab_cancel();
        // (a chain: one road's pieces, which is what a drag of a road carries)
        let chain: Vec<Sel> = {
            let doc = ed.doc.as_ref().unwrap();
            let (k, t) = doc.tiles.iter().find(|(_, t)| t.tile.splines.len() > 40).unwrap();
            let mut id = t.tile.splines.iter().find(|s| !s.deleted).unwrap().id;
            let mut v = Vec::new();
            while v.len() < 40 {
                let Some(sp) = doc.spline(*k, id) else { break };
                v.push(Sel::Spline(*k, id));
                if sp.next_id == 0 || v.iter().any(|s| *s == Sel::Spline(*k, sp.next_id)) {
                    break;
                }
                id = sp.next_id;
            }
            v
        };
        eprintln!("a chain of {} pieces", chain.len());
        ed.set_selection(chain);
        let t = now();
        ed.grab_begin((100.0, 100.0));
        eprintln!("grab begin (chain): {:?}", t.elapsed());
        for i in 1..4 {
            let t = now();
            ed.grab_follow(cam.target + DVec3::new(i as f64, 0.0, 0.0), (100.0 + i as f32, 100.0));
            eprintln!("grab follow (chain) {i}: {:?}", t.elapsed());
        }
        ed.grab_cancel();
    }
}
