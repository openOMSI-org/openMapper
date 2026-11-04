//! The keyboard commands and the keys they are on. Every one has a default; the ones the
//! user changes are kept in the settings (`Prefs::keys`, command id → key as text).

use std::collections::BTreeMap;
use winit::keyboard::KeyCode;

use crate::i18n::tr;

/// A key with the modifiers held with it (Ctrl is Cmd on a Mac).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
}

impl Chord {
    const fn key(code: KeyCode) -> Chord {
        Chord { code, ctrl: false, shift: false }
    }
    const fn ctrl(code: KeyCode) -> Chord {
        Chord { code, ctrl: true, shift: false }
    }

    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s += "Ctrl+";
        }
        if self.shift {
            s += "Shift+";
        }
        s + code_name(self.code)
    }

    fn parse(text: &str) -> Option<Chord> {
        let (mut ctrl, mut shift) = (false, false);
        let mut rest = text;
        loop {
            if let Some(r) = rest.strip_prefix("Ctrl+") {
                ctrl = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("Shift+") {
                shift = true;
                rest = r;
            } else {
                break;
            }
        }
        KEYS.iter().find(|(_, n)| *n == rest).map(|(code, _)| Chord { code: *code, ctrl, shift })
    }
}

macro_rules! keys {
    ($($c:ident $n:expr_2021),* $(,)?) => { &[$((KeyCode::$c, $n)),*] };
}

/// The keys that can be chosen, with the names shown and kept.
const KEYS: &[(KeyCode, &str)] = keys![
    KeyA "A", KeyB "B", KeyC "C", KeyD "D", KeyE "E", KeyF "F", KeyG "G", KeyH "H", KeyI "I", KeyJ "J", KeyK "K", KeyL "L", KeyM "M",
    KeyN "N", KeyO "O", KeyP "P", KeyQ "Q", KeyR "R", KeyS "S", KeyT "T", KeyU "U", KeyV "V", KeyW "W", KeyX "X", KeyY "Y", KeyZ "Z",
    Digit0 "0", Digit1 "1", Digit2 "2", Digit3 "3", Digit4 "4", Digit5 "5", Digit6 "6", Digit7 "7", Digit8 "8", Digit9 "9",
    F1 "F1", F2 "F2", F3 "F3", F4 "F4", F5 "F5", F6 "F6", F7 "F7", F8 "F8", F9 "F9", F10 "F10", F11 "F11", F12 "F12",
    Numpad0 "Num 0", Numpad1 "Num 1", Numpad2 "Num 2", Numpad3 "Num 3", Numpad4 "Num 4", Numpad5 "Num 5", Numpad6 "Num 6", Numpad7 "Num 7", Numpad8 "Num 8", Numpad9 "Num 9",
    Home "Home", End "End", PageUp "PageUp", PageDown "PageDown", Insert "Insert", Delete "Del", Backspace "Backspace",
    Enter "Enter", NumpadEnter "Num Enter", Space "Space", Tab "Tab", BracketLeft "[", BracketRight "]", Minus "-", Equal "=", Comma ",", Period ".",
    Slash "/", Semicolon ";", Quote "'", Backquote "`", Backslash "\\",
];

pub fn code_name(code: KeyCode) -> &'static str {
    KEYS.iter().find(|(c, _)| *c == code).map(|(_, n)| *n).unwrap_or("?")
}

/// The key a name stands for ("G", "3", "Enter": as they are shown).
pub fn code_of(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(c, _)| *c)
}

/// Whether a pressed key can be chosen for a command.
pub fn bindable(code: KeyCode) -> bool {
    KEYS.iter().any(|(c, _)| *c == code)
}

/// The set of keys the editor starts from. Every set has every command; they differ in the
/// keys the editor being copied answers to, and a command the copy has no key for is left on
/// none in its set rather than on openMapper's own.
///
/// - `Mapper`: openMapper's own.
/// - `Omsi`: OMSI 2's map editor (G grabs, N makes an object, Enter confirms, the tile grid
///   on Shift+G); goes with its mouse (`Prefs::omsi_controls`).
/// - `Blender`: Blender's (G grab, Shift+S scale, X delete, Shift+A add, Shift+D
///   duplicate, F3 search, Tab edit the mesh). W A S D and Q E go round as in openMapper's
///   own set (they are the camera first, as they are in every other program here), so
///   Blender's scale is on Shift+S and its select all is left to the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Mapper,
    Omsi,
    Blender,
}

impl Preset {
    /// The set the settings keep ("" : as the mouse setting says, the default).
    pub fn of(key_set: &str, omsi_controls: bool) -> Preset {
        match key_set {
            "mapper" => Preset::Mapper,
            "omsi" => Preset::Omsi,
            "blender" => Preset::Blender,
            _ if omsi_controls => Preset::Omsi,
            _ => Preset::Mapper,
        }
    }

    /// What the settings keep for it.
    pub fn id(self) -> &'static str {
        match self {
            Preset::Mapper => "mapper",
            Preset::Omsi => "omsi",
            Preset::Blender => "blender",
        }
    }

    pub const ALL: [Preset; 3] = [Preset::Mapper, Preset::Omsi, Preset::Blender];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Mapper => "openMapper",
            Preset::Omsi => tr("OMSI 2's editor", "Редактор OMSI 2"),
            Preset::Blender => "Blender",
        }
    }

    /// The key of a command in this set (None: none): the editor copied has one, else the
    /// command's default.
    pub fn chord(self, c: Cmd) -> Option<Chord> {
        match (self, c) {
            // (W A S D and Q E move the camera here too: they are the camera before anything
            // else, so Blender's scale moves to Shift+S and its select all is left to the menu)
            (Preset::Blender, XformMove) => Some(Chord::key(KeyCode::KeyG)),
            (Preset::Blender, XformScale) => Some(Chord { code: KeyCode::KeyS, ctrl: false, shift: true }),
            (Preset::Blender, Delete) => Some(Chord::key(KeyCode::KeyX)),
            (Preset::Blender, SelectAll) => None,
            (Preset::Blender, NewObject) => Some(Chord { code: KeyCode::KeyA, ctrl: false, shift: true }),
            (Preset::Blender, Duplicate) => Some(Chord { code: KeyCode::KeyD, ctrl: false, shift: true }),
            (Preset::Blender, Commands) => Some(Chord::key(KeyCode::F3)),
            (Preset::Blender, LookDown) => Some(Chord::key(KeyCode::Numpad7)),
            (Preset::Blender, TileGrid) => Some(Chord { code: KeyCode::KeyG, ctrl: false, shift: true }),
            // (OMSI 2's editor: G grabs the selection, so the grid moves aside)
            (Preset::Omsi, TileGrid) => Some(Chord { code: KeyCode::KeyG, ctrl: false, shift: true }),
            (Preset::Omsi, XformMove) => Some(Chord::key(KeyCode::KeyG)),
            // (Enter confirms; the road is built on Ctrl+Enter the same way)
            (Preset::Omsi, Confirm) => Some(Chord::key(KeyCode::Enter)),
            (Preset::Omsi, BuildRoad) => Some(Chord::ctrl(KeyCode::Enter)),
            _ => Some(c.default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Forward,
    Back,
    Left,
    Right,
    TurnLeft,
    TurnRight,
    Undo,
    Redo,
    Save,
    Copy,
    Cut,
    Paste,
    Duplicate,
    SelectAll,
    Find,
    Commands,
    ToolSelect,
    ToolRoads,
    ToolObjects,
    ToolLogic,
    ToolTerrain,
    ToolPaint,
    ToolTiles,
    NewObject,
    Rotate,
    XformMove,
    XformRotate,
    XformScale,
    EditMesh,
    JoinObjects,
    Ruler,
    Knife,
    BuildRoad,
    Confirm,
    TileGrid,
    Group,
    Ungroup,
    LookDown,
    View2d,
    Assets,
    Overview,
    Lines,
    Traffic,
    Imagery,
    BrushSmaller,
    BrushLarger,
    Delete,
    Focus,
    TestDrive,
    Picture,
    Polygon,
    PolygonClick,
}

use Cmd::*;

pub const ALL: &[Cmd] = &[
    Forward, Back, Left, Right, TurnLeft, TurnRight, LookDown, View2d, Focus, TileGrid, Overview, Assets, Lines, Traffic, Imagery, ToolSelect, ToolRoads, ToolObjects, ToolLogic,
    ToolTerrain, ToolPaint, ToolTiles, NewObject, XformMove, XformRotate, XformScale, EditMesh, JoinObjects, Rotate, Ruler, Knife, BuildRoad, Confirm, BrushSmaller, BrushLarger, Delete, Group, Ungroup, Copy, Cut, Paste, Duplicate, SelectAll, Undo,
    Redo, Save, Find, Commands, TestDrive, Picture, Polygon, PolygonClick,
];

impl Cmd {
    /// The name kept in the settings.
    pub fn id(self) -> &'static str {
        match self {
            Forward => "forward",
            Back => "back",
            Left => "left",
            Right => "right",
            TurnLeft => "turn_left",
            TurnRight => "turn_right",
            Undo => "undo",
            Redo => "redo",
            Save => "save",
            Copy => "copy",
            Cut => "cut",
            Paste => "paste",
            Duplicate => "duplicate",
            SelectAll => "select_all",
            Find => "find",
            Commands => "commands",
            ToolSelect => "tool_select",
            ToolRoads => "tool_roads",
            ToolObjects => "tool_objects",
            ToolLogic => "tool_logic",
            ToolTerrain => "tool_terrain",
            ToolPaint => "tool_paint",
            ToolTiles => "tool_tiles",
            NewObject => "new_object",
            Rotate => "rotate",
            XformMove => "gizmo_move",
            XformRotate => "gizmo_rotate",
            XformScale => "gizmo_scale",
            EditMesh => "edit_mesh",
            JoinObjects => "join_objects",
            Ruler => "ruler",
            Knife => "knife",
            BuildRoad => "build_road",
            Confirm => "confirm",
            TileGrid => "tile_grid",
            Group => "group",
            Ungroup => "ungroup",
            LookDown => "look_down",
            View2d => "view_2d",
            Assets => "assets",
            Overview => "overview",
            Lines => "lines",
            Traffic => "traffic",
            Imagery => "imagery",
            BrushSmaller => "brush_smaller",
            BrushLarger => "brush_larger",
            Delete => "delete",
            Focus => "focus",
            TestDrive => "test_drive",
            Picture => "picture",
            Polygon => "polygon",
            PolygonClick => "polygon_click",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Forward => tr("Move forward", "Вперёд"),
            Back => tr("Move back", "Назад"),
            Left => tr("Move left", "Влево"),
            Right => tr("Move right", "Вправо"),
            TurnLeft => tr("Turn the camera left", "Повернуть камеру влево"),
            TurnRight => tr("Turn the camera right", "Повернуть камеру вправо"),
            Undo => tr("Undo", "Отменить"),
            Redo => tr("Redo", "Повторить"),
            Save => tr("Save", "Сохранить"),
            Copy => tr("Copy", "Копировать"),
            Cut => tr("Cut", "Вырезать"),
            Paste => tr("Paste at the cursor", "Вставить у курсора"),
            Duplicate => tr("Duplicate", "Дублировать"),
            SelectAll => tr("Select all", "Выбрать всё"),
            Find => tr("Find in the map", "Поиск по карте"),
            Commands => tr("Search for a command", "Поиск команды"),
            ToolSelect => tr("Tool: Select", "Инструмент: Выбор"),
            ToolRoads => tr("Tool: Roads", "Инструмент: Дороги"),
            ToolObjects => tr("Tool: Objects", "Инструмент: Объекты"),
            ToolLogic => tr("Tool: Game logic", "Инструмент: Служебное"),
            ToolTerrain => tr("Tool: Terrain", "Инструмент: Рельеф"),
            ToolPaint => tr("Tool: Paint", "Инструмент: Покраска"),
            ToolTiles => tr("Tool: Tiles", "Инструмент: Тайлы"),
            NewObject => tr("Place a new object at the cursor", "Поставить новый объект у курсора"),
            Rotate => tr("Turn the object (Shift: the other way)", "Повернуть объект (Shift: в другую сторону)"),
            Ruler => tr("Ruler", "Линейка"),
            Knife => tr("Knife", "Нож"),
            XformMove => tr("Move handles", "Ручки перемещения"),
            XformRotate => tr("Rotate handles", "Ручки поворота"),
            XformScale => tr("Scale handles", "Ручки масштаба"),
            EditMesh => tr("Edit the mesh (vertices, edges, faces)", "Редактировать меш (вершины, рёбра, грани)"),
            JoinObjects => tr("Join the selected objects into one", "Объединить выделенные объекты в один"),
            BuildRoad => tr("Build the planned road", "Построить намеченную дорогу"),
            Confirm => tr("Confirm (finish what is being drawn)", "Подтвердить (закончить начатое)"),
            TileGrid => tr("Tile grid on / off", "Сетка тайлов"),
            Group => tr("Group the selection", "Сгруппировать выбранное"),
            Ungroup => tr("Ungroup", "Разгруппировать"),
            LookDown => tr("Look straight down, north up", "Смотреть сверху, север вверху"),
            View2d => tr("2D plan view on / off (3D: tilt and turn again)", "2D-вид сверху вкл / выкл (3D: снова наклон и поворот)"),
            Assets => tr("Catalogue on / off", "Каталог"),
            Overview => tr("Overview map on / off", "Обзорная карта"),
            Lines => tr("Lines window", "Окно линий"),
            Traffic => tr("Traffic window", "Окно трафика"),
            Imagery => tr("Satellite and terrain window", "Окно спутника и рельефа"),
            BrushSmaller => tr("Smaller brush", "Кисть меньше"),
            BrushLarger => tr("Larger brush", "Кисть больше"),
            Delete => tr("Delete the selection", "Удалить выбранное"),
            Focus => tr("Go to the selection", "Перейти к выбранному"),
            TestDrive => tr("Test drive in openOMSI (Shift: from the camera)", "Тест-драйв в openOMSI (Shift: от камеры)"),
            Picture => tr("Save a picture of the view", "Сохранить картинку вида"),
            Polygon => tr("Polygon: fill the selected splines, or click its corners", "Многоугольник: заполнить выделенные сплайны или кликать по углам"),
            PolygonClick => tr("Polygon: click its corners", "Многоугольник: кликать по углам"),
        }
    }

    pub fn default(self) -> Chord {
        match self {
            Forward => Chord::key(KeyCode::KeyW),
            Back => Chord::key(KeyCode::KeyS),
            Left => Chord::key(KeyCode::KeyA),
            Right => Chord::key(KeyCode::KeyD),
            TurnLeft => Chord::key(KeyCode::KeyQ),
            TurnRight => Chord::key(KeyCode::KeyE),
            Undo => Chord::ctrl(KeyCode::KeyZ),
            Redo => Chord { code: KeyCode::KeyZ, ctrl: true, shift: true },
            Save => Chord::ctrl(KeyCode::KeyS),
            Copy => Chord::ctrl(KeyCode::KeyC),
            Cut => Chord::ctrl(KeyCode::KeyX),
            Paste => Chord::ctrl(KeyCode::KeyV),
            Duplicate => Chord::ctrl(KeyCode::KeyD),
            SelectAll => Chord::ctrl(KeyCode::KeyA),
            Find => Chord::ctrl(KeyCode::KeyF),
            Commands => Chord::ctrl(KeyCode::KeyK),
            ToolSelect => Chord::key(KeyCode::Digit1),
            ToolRoads => Chord::key(KeyCode::Digit2),
            ToolObjects => Chord::key(KeyCode::Digit3),
            ToolLogic => Chord::key(KeyCode::Digit4),
            ToolTerrain => Chord::key(KeyCode::Digit5),
            ToolPaint => Chord::key(KeyCode::Digit6),
            ToolTiles => Chord::key(KeyCode::Digit7),
            NewObject => Chord::key(KeyCode::KeyN),
            Rotate => Chord::key(KeyCode::KeyR),
            XformMove => Chord::key(KeyCode::KeyV),
            XformRotate => Chord::key(KeyCode::KeyC),
            XformScale => Chord::key(KeyCode::KeyX),
            EditMesh => Chord::key(KeyCode::Tab),
            JoinObjects => Chord::ctrl(KeyCode::KeyJ),
            Ruler => Chord::key(KeyCode::KeyU),
            Knife => Chord::key(KeyCode::KeyK),
            BuildRoad => Chord::key(KeyCode::Enter),
            Confirm => Chord::key(KeyCode::NumpadEnter),
            TileGrid => Chord::key(KeyCode::KeyG),
            Group => Chord::ctrl(KeyCode::KeyG),
            Ungroup => Chord { code: KeyCode::KeyG, ctrl: true, shift: true },
            LookDown => Chord::key(KeyCode::Home),
            View2d => Chord::key(KeyCode::Digit0),
            Assets => Chord::key(KeyCode::KeyB),
            Overview => Chord::key(KeyCode::KeyM),
            Lines => Chord::key(KeyCode::KeyL),
            Traffic => Chord::key(KeyCode::KeyT),
            Imagery => Chord::key(KeyCode::KeyI),
            BrushSmaller => Chord::key(KeyCode::BracketLeft),
            BrushLarger => Chord::key(KeyCode::BracketRight),
            Delete => Chord::key(KeyCode::Delete),
            Focus => Chord::key(KeyCode::KeyF),
            TestDrive => Chord::key(KeyCode::F5),
            Picture => Chord::key(KeyCode::F12),
            Polygon => Chord::key(KeyCode::KeyP),
            PolygonClick => Chord { code: KeyCode::KeyP, ctrl: false, shift: true },
        }
    }

    /// Whether Shift only changes what the command does (it is not part of the key).
    fn shift_free(self) -> bool {
        matches!(self, Rotate | TestDrive)
    }

    /// The keys that also do it and are not changed.
    pub fn also(self) -> &'static str {
        match self {
            Forward => "↑",
            Back => "↓",
            Left => "←",
            Right => "→",
            Redo => "Ctrl+Y",
            Commands => "Ctrl+Shift+P",
            Delete => "Backspace",
            _ => "",
        }
    }
}

/// The keys as chosen: the preset's with the changes from the settings.
pub struct Keymap(Vec<(Cmd, Option<Chord>)>);

impl Keymap {
    pub fn of(changed: &BTreeMap<String, String>, preset: Preset) -> Keymap {
        let mut map: Vec<(Cmd, Option<Chord>)> = ALL.iter().map(|c| (*c, None)).collect();
        let mut taken: Vec<Chord> = Vec::new();
        // (the keys the user chose first, so that a set switching under them never takes one
        // of theirs away for a command of its own)
        for (i, c) in ALL.iter().enumerate() {
            let chord = match changed.get(c.id()) {
                Some(t) if t.is_empty() => None,
                Some(t) => Some(Chord::parse(t).unwrap_or_else(|| preset.chord(*c).unwrap_or_else(|| c.default()))),
                None => continue,
            };
            match chord {
                Some(k) if taken.contains(&k) => {}
                Some(k) => {
                    taken.push(k);
                    map[i].1 = Some(k);
                }
                None => {}
            }
        }
        for (i, c) in ALL.iter().enumerate() {
            if changed.contains_key(c.id()) {
                continue;
            }
            let Some(k) = preset.chord(*c) else { continue };
            if !taken.contains(&k) {
                taken.push(k);
                map[i].1 = Some(k);
            }
        }
        Keymap(map)
    }

    pub fn chord(&self, cmd: Cmd) -> Option<Chord> {
        self.0.iter().find(|(c, _)| *c == cmd).and_then(|(_, k)| *k)
    }

    /// The command on a key pressed with these modifiers.
    pub fn find(&self, code: KeyCode, ctrl: bool, shift: bool) -> Option<Cmd> {
        let exact = self.0.iter().find(|(_, k)| k.is_some_and(|k| k.code == code && k.ctrl == ctrl && k.shift == shift));
        let loose = || self.0.iter().find(|(c, k)| c.shift_free() && k.is_some_and(|k| k.code == code && k.ctrl == ctrl));
        let cmd = exact.or_else(loose).map(|(c, _)| *c);
        // The menus say Ctrl+D. Blender's own key for this is Shift+D, and Ctrl+D is free
        // there, so the key the menu shows still duplicates.
        if cmd.is_none() && code == KeyCode::KeyD && ctrl && !shift && self.chord(Cmd::Duplicate).is_some() {
            return Some(Cmd::Duplicate);
        }
        cmd
    }

    /// Whether `code` is the key of a command (whatever is held with it): for the keys
    /// that are held down, as the camera's.
    pub fn is_key(&self, cmd: Cmd, code: KeyCode) -> bool {
        self.chord(cmd).is_some_and(|k| k.code == code)
    }
}

/// Puts a command on a key (None: on none) in the settings' list; whatever had the key
/// loses it. `chord` is compared with the preset's key: that one is kept out of the list.
pub fn bind(changed: &mut BTreeMap<String, String>, preset: Preset, cmd: Cmd, chord: Option<Chord>) {
    if let Some(k) = chord {
        let map = Keymap::of(changed, preset);
        for (other, had) in &map.0 {
            if *other != cmd && *had == Some(k) {
                changed.insert(other.id().into(), String::new());
            }
        }
    }
    let text = chord.map(|k| k.text()).unwrap_or_default();
    if chord == preset.chord(cmd) {
        changed.remove(cmd.id());
    } else {
        changed.insert(cmd.id().into(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_all_different_and_survive_the_settings() {
        for preset in Preset::ALL {
            let map = Keymap::of(&BTreeMap::new(), preset);
            for (c, k) in &map.0 {
                // (a set may leave a command on no key at all, as Blender's does the camera's)
                let Some(k) = k else { continue };
                assert_eq!(Chord::parse(&k.text()), Some(*k), "{} in {:?}", c.id(), preset);
                assert_eq!(map.0.iter().filter(|(_, o)| *o == Some(*k)).count(), 1, "{} shares its key in {:?}", c.id(), preset);
            }
        }
    }

    #[test]
    fn the_omsi_set_has_the_keys_of_omsi_2s_editor() {
        let omsi = Keymap::of(&BTreeMap::new(), Preset::Omsi);
        assert_eq!(omsi.find(KeyCode::KeyG, false, false), Some(Cmd::XformMove));
        assert_eq!(omsi.find(KeyCode::KeyG, false, true), Some(Cmd::TileGrid));
        assert_eq!(omsi.find(KeyCode::KeyN, false, false), Some(Cmd::NewObject));
        assert_eq!(omsi.find(KeyCode::KeyR, false, false), Some(Cmd::Rotate));
        assert_eq!(omsi.find(KeyCode::Enter, false, false), Some(Cmd::Confirm));
        assert_eq!(omsi.find(KeyCode::Enter, true, false), Some(Cmd::BuildRoad));
        assert_eq!(omsi.find(KeyCode::Delete, false, false), Some(Cmd::Delete));
        // (the keys openMapper had there are still there, on their own)
        assert_eq!(omsi.find(KeyCode::KeyV, false, false), None);
        let own = Keymap::of(&BTreeMap::new(), Preset::Mapper);
        assert_eq!(own.find(KeyCode::KeyG, false, false), Some(Cmd::TileGrid));
        assert_eq!(own.find(KeyCode::Enter, false, false), Some(Cmd::BuildRoad));
        assert_eq!(own.find(KeyCode::NumpadEnter, false, false), Some(Cmd::Confirm));
    }

    #[test]
    fn a_taken_key_is_taken_from_its_old_command() {
        let mut changed = BTreeMap::new();
        bind(&mut changed, Preset::Mapper, Cmd::Ruler, Some(Chord::key(KeyCode::KeyK)));
        let map = Keymap::of(&changed, Preset::Mapper);
        assert_eq!(map.find(KeyCode::KeyK, false, false), Some(Cmd::Ruler));
        assert_eq!(map.chord(Cmd::Knife), None);
        bind(&mut changed, Preset::Mapper, Cmd::Ruler, Some(Cmd::Ruler.default()));
        assert_eq!(Keymap::of(&changed, Preset::Mapper).find(KeyCode::KeyU, false, false), Some(Cmd::Ruler));
    }

    #[test]
    fn a_key_given_back_returns_to_the_presets_own() {
        let mut changed = BTreeMap::new();
        bind(&mut changed, Preset::Omsi, Cmd::XformMove, Some(Chord::key(KeyCode::KeyH)));
        assert_eq!(Keymap::of(&changed, Preset::Omsi).find(KeyCode::KeyH, false, false), Some(Cmd::XformMove));
        // (put back on the set's own key: the setting goes, G is the move handles again)
        bind(&mut changed, Preset::Omsi, Cmd::XformMove, Preset::Omsi.chord(Cmd::XformMove));
        assert!(changed.get("gizmo_move").is_none());
        let map = Keymap::of(&changed, Preset::Omsi);
        assert_eq!(map.find(KeyCode::KeyG, false, false), Some(Cmd::XformMove));
        assert_eq!(map.find(KeyCode::KeyV, false, false), None);
        // (openMapper's own set is not touched by the OMSI one)
        assert_eq!(Keymap::of(&changed, Preset::Mapper).find(KeyCode::KeyV, false, false), Some(Cmd::XformMove));
    }

    #[test]
    fn the_blender_set_has_the_keys_of_blender() {
        let b = Keymap::of(&BTreeMap::new(), Preset::Blender);
        assert_eq!(b.find(KeyCode::KeyG, false, false), Some(Cmd::XformMove));
        assert_eq!(b.find(KeyCode::KeyX, false, false), Some(Cmd::Delete));
        assert_eq!(b.find(KeyCode::KeyA, false, true), Some(Cmd::NewObject));
        assert_eq!(b.find(KeyCode::KeyD, false, true), Some(Cmd::Duplicate));
        assert_eq!(b.find(KeyCode::KeyD, true, false), Some(Cmd::Duplicate));
        assert_eq!(b.find(KeyCode::KeyR, false, false), Some(Cmd::Rotate));
        assert_eq!(b.find(KeyCode::Tab, false, false), Some(Cmd::EditMesh));
        assert_eq!(b.find(KeyCode::F3, false, false), Some(Cmd::Commands));
        assert_eq!(b.find(KeyCode::Numpad7, false, false), Some(Cmd::LookDown));
        assert_eq!(b.find(KeyCode::KeyG, false, true), Some(Cmd::TileGrid));
        // (W A S D and Q E go round, as in openMapper's own set: the camera first. Blender's
        // scale is on Shift+S, and its select all is left to the menu)
        assert_eq!(b.find(KeyCode::KeyS, false, false), Some(Cmd::Back));
        assert_eq!(b.find(KeyCode::KeyS, false, true), Some(Cmd::XformScale));
        assert_eq!(b.find(KeyCode::KeyA, false, false), Some(Cmd::Left));
        assert!(b.find(KeyCode::KeyA, true, false).is_none(), "A with Ctrl is not given a command");
        assert_eq!(b.chord(Cmd::SelectAll), None);
        for (k, c) in [(KeyCode::KeyW, Cmd::Forward), (KeyCode::KeyD, Cmd::Right), (KeyCode::KeyQ, Cmd::TurnLeft), (KeyCode::KeyE, Cmd::TurnRight)] {
            assert_eq!(b.find(k, false, false), Some(c), "{k:?}");
        }
        // (openMapper's own set has the same six)
        let own = Keymap::of(&BTreeMap::new(), Preset::Mapper);
        assert_eq!(own.find(KeyCode::KeyS, false, false), Some(Cmd::Back));
    }

    #[test]
    fn a_set_switched_under_does_not_take_a_chosen_key() {
        // (the settings say N is the ruler, as they could from the other set)
        let mut changed = BTreeMap::new();
        changed.insert("ruler".into(), "N".into());
        let omsi = Keymap::of(&changed, Preset::Omsi);
        assert_eq!(omsi.find(KeyCode::KeyN, false, false), Some(Cmd::Ruler));
        assert_eq!(omsi.chord(Cmd::NewObject), None);
        assert_eq!(omsi.find(KeyCode::KeyR, false, false), Some(Cmd::Rotate));
    }

    #[test]
    fn shift_turns_the_other_way_without_changing_the_key() {
        let map = Keymap::of(&BTreeMap::new(), Preset::Mapper);
        assert_eq!(map.find(KeyCode::KeyR, false, true), Some(Cmd::Rotate));
        assert_eq!(map.find(KeyCode::KeyG, true, true), Some(Cmd::Ungroup));
        assert_eq!(map.find(KeyCode::KeyG, true, false), Some(Cmd::Group));
    }
}
