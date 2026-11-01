//! The map check: what would go wrong when the map is driven, explained for someone new to
//! OMSI maps. Missing object, road and texture files, road ends that lie on one another but
//! are not joined, bus stops and start points away from any road, IDCodes used twice,
//! tiles without ground. Every issue says where it is (a click flies there) and many can be
//! put right with one more click.
//!
//! What the check needs is copied from the map on the main thread; the files are looked at
//! on a thread of their own, so a big map does not hold the window up.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui::{Color32, RichText};
use glam::{DVec2, DVec3};
use omsi_geometry::SplineCurve;

use crate::doc::{tile_origin, tile_size, Document, Sel, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{Category, Library};
use crate::ui::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The game will not load or show it right.
    Error,
    /// Probably not what was meant.
    Warning,
    /// Worth knowing.
    Info,
}

/// One spline end: tile, IDCode, its end (true) or its start.
pub type End = (TileKey, i64, bool);

/// What one click can put right.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// Join two free road ends that lie on one another.
    Link(End, End),
    /// Forget a link to a spline that is not there.
    Unlink(End),
    /// Give new objects IDCodes above every one in use.
    RaiseNextId(i64),
    /// Take away a start point whose object is gone.
    DropEntry(i64),
    /// Hand an object over to the tile it stands on.
    MoveToTile(TileKey, i64),
    /// Have the game light a tile's ground from its lamps at night.
    LampLight(TileKey),
    /// Forget the object an object takes its state from (`[varparent]`).
    DropVarParent(TileKey, i64),
    /// Copy the car groups the traffic names but the AI lists lack from OMSI 2's own maps
    /// (in the installation there).
    StockGroups(PathBuf),
    /// Put a start point on a lane before a bus stop (the content folder and the
    /// installation, where the roads' files are).
    AddStart(PathBuf, PathBuf),
    /// Give a line's tour (line, tour number) the map's depot, or a depot of OMSI 2's MAN
    /// buses where the map has none (true: their files are there).
    TourDepot(String, String, bool),
    /// Make one of openMapper's junction plates again whose file is gone (by its IDCode).
    RemakeJunction(i64),
    /// Move a bus stop to the kerb of the lane nearest it, facing along it (the content
    /// folder and the installation, where the roads' files are).
    StopToKerb(TileKey, i64, PathBuf, PathBuf),
    /// Make the map a depot file of this name from its lines (empty: named after the
    /// map's folder), and have the map's depot groups that name none use it.
    MakeDepot(String),
    /// Add to the map's depot file of this name what the lines need.
    FillDepot(String),
    /// Make the map its own depot file from its lines, in place of another map's of this
    /// name its depot groups use.
    OwnDepot(String),
    /// Give a bus stop a name no stop far from it has.
    UniqueStopName(TileKey, i64),
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub severity: Severity,
    pub kind: Kind,
    pub title: String,
    pub detail: String,
    /// Where it is (world metres).
    pub at: Option<DVec2>,
    pub sel: Option<Sel>,
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    MissingObject,
    MissingSpline,
    MissingTexture,
    UnlinkedEnds,
    BrokenLink,
    StopOffRoad,
    StopName,
    EntryOffRoad,
    EntryLost,
    NoEntry,
    DuplicateId,
    NextId,
    OutsideTile,
    TileMissing,
    NoGround,
    TripNoRoute,
    TourEmpty,
    TourNoDepot,
    UnschedMissing,
    StopUnused,
    SignalRouteBroken,
    ChronoUnmatched,
    LampsDark,
    HeavyTile,
    LinkLost,
    HeadUnlinked,
    DepotFile,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::MissingObject => tr("Missing objects", "Нет файлов объектов"),
            Kind::MissingSpline => tr("Missing roads and splines", "Нет файлов дорог и сплайнов"),
            Kind::MissingTexture => tr("Missing textures", "Нет текстур"),
            Kind::UnlinkedEnds => tr("Road ends not joined", "Несоединённые концы дорог"),
            Kind::BrokenLink => tr("Links to splines that are gone", "Ссылки на исчезнувшие сплайны"),
            Kind::StopOffRoad => tr("Bus stops away from a road", "Остановки вдали от дороги"),
            Kind::StopName => tr("Stops sharing a name", "Остановки с одним именем"),
            Kind::EntryOffRoad => tr("Start points away from a road", "Точки старта вдали от дороги"),
            Kind::EntryLost => tr("Start points without their object", "Точки старта без объекта"),
            Kind::NoEntry => tr("No start point", "Нет точки старта"),
            Kind::DuplicateId => tr("IDCodes used twice", "Повторные IDCode"),
            Kind::NextId => tr("Next IDCode too low", "Слишком малый NextIDCode"),
            Kind::OutsideTile => tr("Objects off their tile", "Объекты вне своего тайла"),
            Kind::TileMissing => tr("Tiles without a file", "Тайлы без файла"),
            Kind::NoGround => tr("Tiles without ground", "Тайлы без земли"),
            Kind::TripNoRoute => tr("Trips without a way", "Рейсы без пути"),
            Kind::TourEmpty => tr("Tours without departures", "Выходы без рейсов"),
            Kind::TourNoDepot => tr("Tours without a depot", "Выходы без депо"),
            Kind::UnschedMissing => tr("Traffic groups not in the AI lists", "Группы трафика не в списках ИИ"),
            Kind::StopUnused => tr("Stops no trip calls at", "Остановки без рейсов"),
            Kind::SignalRouteBroken => tr("Signal routes naming what is gone", "Маршруты сигналов с исчезнувшим"),
            Kind::ChronoUnmatched => tr("Chrono changes of what is gone", "Хроно-изменения исчезнувшего"),
            Kind::LampsDark => tr("Lamps lighting no ground at night", "Фонари не освещают землю ночью"),
            Kind::HeavyTile => tr("Tiles with very many objects", "Тайлы с очень большим числом объектов"),
            Kind::LinkLost => tr("Objects linked to one that is gone", "Объекты, связанные с исчезнувшим"),
            Kind::HeadUnlinked => tr("Signal heads linked to no crossing", "Светофоры без перекрёстка"),
            Kind::DepotFile => tr("Depot files (destinations)", "Файлы депо (конечные)"),
        }
    }

    /// What it means and what to do, for someone new to it.
    pub fn help(self) -> &'static str {
        match self {
            Kind::MissingObject => tr("The map uses an object (.sco) that is not installed. The game leaves it out. Install the mod it comes from, or delete the object. A junction of openMapper's whose file is gone (the map was moved without it) is made again for its roads with \"Fix\".", "Карта использует объект (.sco), которого нет. Игра его пропустит. Установите мод, из которого он, или удалите объект. Перекрёсток openMapper, файл которого пропал (карту перенесли без него), «Исправить» построит заново для его дорог."),
            Kind::MissingSpline => tr("The map uses a road or spline type (.sli) that is not installed: that road is missing in the game, and buses cannot drive there.", "Карта использует тип дороги или сплайна (.sli), которого нет: этой дороги в игре не будет, и автобусы там не проедут."),
            Kind::MissingTexture => tr("An object or road names a picture that cannot be found. It will look white or black in the game.", "Объект или дорога ссылается на картинку, которой нет. В игре он будет белым или чёрным."),
            Kind::UnlinkedEnds => tr("Two road ends lie on one another but are not joined. Traffic still finds its way (the game joins lanes by their ends), but rows of lamps and posts and the road's texture do not run on across, and the editor does not treat them as one road. \"Join\" links them.", "Два конца дорог лежат друг на друге, но не соединены. Транспорт проедет (игра соединяет полосы по концам), но ряды фонарей и столбиков и текстура дороги не продолжаются через стык, а редактор не считает их одной дорогой. «Соединить» свяжет их."),
            Kind::BrokenLink => tr("A spline says it goes on into another one that no longer exists. \"Fix\" forgets that link.", "Сплайн ссылается на продолжение, которого больше нет. «Исправить» уберёт эту ссылку."),
            Kind::StopOffRoad => tr("A bus stop belongs to the lane it stands beside (within about 25 m). This one has none near, so buses cannot stop at it. Move it next to the road, or \"Fix\" puts it at the kerb of the nearest lane (within 80 m).", "Остановка относится к полосе, у которой стоит (не дальше примерно 25 м). У этой рядом полосы нет, автобус не сможет у неё остановиться. Передвиньте её к дороге или «Исправить» поставит её у бордюра ближайшей полосы (до 80 м)."),
            Kind::StopName => tr("Two stops far from each other have one name. The two across a road may share it (they are a pair), but elsewhere the game, the displays and the people who choose where to get off cannot tell them apart. \"Fix\" gives the later one a number after its name.", "У двух далёких друг от друга остановок одно имя. Напротив через дорогу оно может совпадать (это пара), но в остальных случаях игра, табло и пассажиры, выбирающие, где выйти, не отличат их. «Исправить» добавит позднейшей номер после имени."),
            Kind::EntryOffRoad => tr("A start point is where the bus appears. This one is away from any lane: fine in a depot yard, otherwise the bus starts off the road.", "Точка старта: где появляется автобус. Эта далеко от полос: в парке это нормально, иначе автобус окажется вне дороги."),
            Kind::EntryLost => tr("A start point refers to an object that was deleted. \"Fix\" removes the start point.", "Точка старта ссылается на удалённый объект. «Исправить» удалит точку старта."),
            Kind::NoEntry => tr("There is nowhere for the bus to appear. Place a start point (Game logic > Start points) on a road, or \"Fix\" puts one on a lane some 40 m before a bus stop.", "Автобусу негде появиться. Поставьте точку старта (Служебное > Точки старта) на дорогу или «Исправить» поставит её на полосу метрах в 40 перед остановкой."),
            Kind::DuplicateId => tr("Two things on the map share one IDCode: links, timetables and start points may pick the wrong one.", "Две вещи на карте с одним IDCode: связи, расписания и точки старта могут указать не на ту."),
            Kind::NextId => tr("The counter for new IDCodes is below codes already used, so new objects would get codes twice. \"Fix\" raises it.", "Счётчик новых IDCode ниже уже занятых, новые объекты получат повторные коды. «Исправить» поднимет его."),
            Kind::OutsideTile => tr("An object standing on the ground lies outside its own tile. The game does not show it. Move it a little or put it again.", "Объект, стоящий на земле, лежит вне своего тайла. Игра его не покажет. Сдвиньте его или поставьте заново."),
            Kind::TileMissing => tr("global.cfg lists tiles whose file is not there. The game leaves them out; they stay listed, as start points and timetables count tiles by their place in the list.", "В global.cfg указаны тайлы без файла. Игра их пропускает; они остаются в списке, ведь точки старта и расписания считают тайлы по месту в нём."),
            Kind::NoGround => tr("A tile has no height file: its ground is flat. Saving writes one.", "У тайла нет файла высот: земля плоская. Сохранение запишет его."),
            Kind::TripNoRoute => tr("No way along the roads was found through this trip's stops, so buses cannot drive it. Open it in Map > Lines and timetable to see where it breaks.", "Через остановки этого рейса не найден путь по дорогам, автобусы его не проедут. Откройте его в Карта > Линии и расписание, чтобы увидеть, где он рвётся."),
            Kind::TourEmpty => tr("A tour (a bus's day on the line) has no trips, so no bus runs it.", "У выхода (дня автобуса на линии) нет рейсов, по нему никто не поедет."),
            Kind::TourNoDepot => tr("A tour names no depot or group of the AI lists (or one that is not there): the game has no vehicle to send on it. Choose a depot for it in the Lines window, or \"Fix\" gives it the map's depot (one of OMSI 2's MAN buses added where the map has none).", "Выход не указывает депо или группу из списка ИИ (или указывает несуществующую): игре нечем его обслужить. Выберите депо в окне линий или «Исправить» назначит ему депо карты (а если его нет, добавит депо с автобусами MAN из OMSI 2)."),
            Kind::UnschedMissing => tr("The unscheduled traffic names a group that the AI lists do not have: those cars never appear. Add the group in Map > Traffic and AI, or \"Fix\" copies it from OMSI 2's own maps where they have it.", "Свободное движение указывает группу, которой нет в списке ИИ: эти машины не появятся. Добавьте группу в Карта > Трафик и ИИ или «Исправить» скопирует её из карт OMSI 2, если она там есть."),
            Kind::StopUnused => tr("A stop of the timetable that no trip calls at. Nothing is wrong, it is only not used yet.", "Остановка расписания, на которой не останавливается ни один рейс. Ничего страшного, просто она пока не используется."),
            Kind::ChronoUnmatched => tr("A chrono scenario takes away, retypes or relabels something that is no longer on the map (it was deleted or its IDCode changed): that change does nothing. Open Map > Chrono scenarios, the scenario's properties, and take the change out.", "Хроно-сценарий убирает, меняет тип или тексты того, чего на карте больше нет (удалено или сменился IDCode): это изменение ничего не делает. Откройте Карта > Хроно-сценарии, свойства сценария, и уберите изменение."),
            Kind::HeavyTile => tr("A tile holds far more objects than the stock maps' tiles (up to some 600): the game may stutter when it loads it or looks across it. Thin out a wood planted too densely (Plant brush, Clear) or plant it with more space between the trees.", "На тайле объектов намного больше, чем на тайлах стоковых карт (до 600): игра может подтормаживать, загружая его или глядя через него. Проредите слишком густой лес (кисть растений, «Убирать») или сажайте реже."),
            Kind::LampsDark => tr("Street lamps stand on a tile the game does not light from its lamps ([variable_terrainlightmap] is off): at night the ground under them stays dark (unless the tile's light map file was made by OMSI 2's editor). \"Fix\" turns it on, as on nearly every tile of the stock maps.", "Фонари стоят на тайле, который игра не освещает от фонарей ([variable_terrainlightmap] выключен): ночью земля под ними тёмная (если файл карты освещения не сделан редактором OMSI 2). «Исправить» включит его, как почти на всех тайлах стоковых карт."),
            Kind::LinkLost => tr("An object takes its state ([varparent]) from an object no longer on the map: a signal head stays dark, a display blank. Link it to another in its properties (Linked objects), or \"Fix\" forgets the link.", "Объект берёт состояние ([varparent]) от объекта, которого больше нет на карте: светофор не горит, табло пустое. Свяжите его с другим в свойствах («Связанные объекты») или «Исправить» уберёт связь."),
            Kind::HeadUnlinked => tr("A traffic light's signal head takes its light from a crossing's program ([varparent]); this one is linked to none while others like it are, so it stays dark in the game. Select it and link it in its properties (Linked objects).", "Головка светофора берёт сигнал из программы перекрёстка ([varparent]); эта ни с чем не связана, хотя такие же связаны, и в игре она не горит. Выберите её и свяжите в свойствах («Связанные объекты»)."),
            Kind::DepotFile => tr("A depot file (.hof) gives the buses their destinations, the stop names on their displays and the routes their IBIS knows. Without one the signs stay empty and the IBIS knows no line. \"Fix\" makes it from the map's lines, or adds what the lines need to the map's own (Map > Depot files to see and change it).", "Файл депо (.hof) даёт автобусам конечные, имена остановок на табло и маршруты, которые знает IBIS. Без него указатели пусты, а IBIS не знает ни одной линии. «Исправить» создаст его по линиям карты или добавит в собственный файл карты то, чего не хватает линиям (Карта > Файлы депо, чтобы посмотреть и изменить)."),
            Kind::SignalRouteBroken => tr("A signal route names a signal or track pieces no longer on the map: the signal may never clear. Open Map > Signal routes and take the track again, or delete the route.", "Маршрут сигнала указывает сигнал или участки пути, которых больше нет на карте: сигнал может так и не открыться. Откройте Карта > Маршруты сигналов и возьмите пути заново или удалите маршрут."),
        }
    }
}

/// What the check needs of the map (copied, so it can run on a thread).
struct Snapshot {
    objects: Vec<ObjRec>,
    splines: Vec<SplineRec>,
    /// Splines the active chrono folders add (their lanes count for stops and start points).
    chrono_lanes: Vec<(String, Vec<DVec2>)>,
    /// Files' kinds from the catalogue (lower case).
    kinds: HashMap<String, KindRec>,
    entries: Vec<(i64, String, i32)>,
    next_id: i64,
    listed_missing: Vec<(i32, i32, String)>,
    no_terrain: Vec<TileKey>,
    /// Tiles the game does not light from their lamps (no `[variable_terrainlightmap]`).
    unlit: HashSet<TileKey>,
    ground: Vec<String>,
    map_dir: PathBuf,
    tiles: HashSet<TileKey>,
    /// Objects something hangs on (`[attachObj]`).
    parents: HashSet<i64>,
    /// Objects that take their state from another (`[varparent]`): its IDCode.
    var_links: Vec<(TileKey, i64, DVec2, i64)>,
    /// Every object's IDCode, the chrono scenarios' too.
    object_ids: HashSet<i64>,
    /// The map for the timetable's checks (routes are found on it).
    tt_doc: Option<Document>,
    /// The bus stops, with the tile each is on.
    stops: Vec<(TileKey, crate::stops::Found)>,
}

/// What the catalogue says of a file.
struct KindRec {
    cat: Category,
    half_width: f32,
    bus_stop: bool,
    /// Carries a street or rail lane (a visible road or an AI path under one).
    lanes: bool,
    /// Stands at its own height, not on the ground.
    abs_height: bool,
    lamp: bool,
    light_head: bool,
}

struct ObjRec {
    file: String,
    tile: TileKey,
    id: i64,
    world: DVec2,
    rot: [f64; 3],
    local: [f64; 2],
    attached: bool,
}

struct SplineRec {
    file: String,
    tile: TileKey,
    id: i64,
    prev: i64,
    next: i64,
    /// Start and end: point, heading out of the spline there.
    ends: [(DVec3, f64); 2],
    /// Points along it every few metres (world x, y).
    samples: Vec<DVec2>,
}

/// Points along a spline every few metres (world x, y).
fn samples_of(s: &omsi_map::MapSpline, org: DVec2) -> Vec<DVec2> {
    let c = SplineCurve::from_map(s, org);
    let len = s.length.max(0.0);
    let n = (len / 4.0).ceil().clamp(1.0, 400.0) as usize;
    (0..=n).map(|i| c.point_at(len * i as f64 / n as f64).truncate()).collect()
}

fn snapshot(doc: &Document, lib: &Library) -> Snapshot {
    let mut objects = Vec::new();
    let mut splines = Vec::new();
    let mut chrono_lanes = Vec::new();
    let mut kinds = HashMap::new();
    let mut kind_of = |file: &str| {
        let key = file.to_lowercase();
        if !kinds.contains_key(&key) {
            if let Some(a) = lib.find(file) {
                kinds.insert(key, KindRec { cat: a.category, half_width: a.half_width, bus_stop: a.is_bus_stop, lanes: a.lanes.0 > 0 || a.lanes.2 > 0, abs_height: a.abs_height, lamp: a.lamp, light_head: a.light_head });
            }
        }
    };
    for (&k, t) in &doc.tiles {
        let org = tile_origin(k);
        for o in t.tile.objects.iter() {
            kind_of(&o.file);
            objects.push(ObjRec { file: o.file.clone(), tile: k, id: o.id, world: org + DVec2::new(o.pos[0], o.pos[1]), rot: o.rot, local: [o.pos[0], o.pos[1]], attached: false });
        }
        for o in t.tile.attach_objects.iter() {
            kind_of(&o.file);
            objects.push(ObjRec { file: o.file.clone(), tile: k, id: o.id, world: org + DVec2::new(o.pos[0], o.pos[1]), rot: o.rot, local: [o.pos[0], o.pos[1]], attached: true });
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            kind_of(&s.file);
            let c = SplineCurve::from_map(s, org);
            let len = s.length.max(0.0);
            let samples = samples_of(s, org);
            // (out of the spline: backwards at its start, forwards at its end)
            let ends = [(c.point_at(0.0), c.heading_at(0.0) + 180.0), (c.point_at(len), c.heading_at(len))];
            splines.push(SplineRec { file: s.file.clone(), tile: k, id: s.id, prev: s.prev_id, next: s.next_id, ends, samples });
        }
        if !t.chrono.is_empty() {
            let own: HashSet<i64> = t.tile.splines.iter().map(|s| s.id).collect();
            for s in t.display(k.1).splines.iter().filter(|s| !s.deleted && !own.contains(&s.id)) {
                kind_of(&s.file);
                chrono_lanes.push((s.file.clone(), samples_of(s, org)));
            }
        }
    }
    let present: HashSet<TileKey> = doc.tiles.keys().copied().collect();
    let listed_missing = doc.global.tiles.iter().filter(|t| !present.contains(&(t.x, t.y))).map(|t| (t.x, t.y, t.file.clone())).collect();
    let src = doc.source.clone().unwrap_or_else(|| doc.dir.clone());
    let no_terrain = doc.tiles.keys().copied().filter(|k| !src.join(format!("tile_{}_{}.map.terrain", k.0, k.1)).is_file() && !doc.tiles[k].dirty).collect();
    Snapshot {
        objects,
        splines,
        chrono_lanes,
        kinds,
        entries: doc.global.entry_points.iter().map(|e| (e.object_id, e.name.clone(), e.group)).collect(),
        next_id: doc.global.next_id_code,
        listed_missing,
        no_terrain,
        unlit: doc.tiles.iter().filter(|(_, t)| !t.tile.variable_terrain_lightmap).map(|(k, _)| *k).collect(),
        ground: doc.global.ground_textures.iter().flat_map(|g| [g.texture.clone(), g.detail_texture.clone()]).collect(),
        map_dir: src,
        tiles: present,
        parents: doc.tiles.values().flat_map(|t| t.tile.attach_objects.iter().filter_map(|a| a.parent_id)).collect(),
        var_links: doc.tiles.iter().flat_map(|(k, t)| {
            let org = tile_origin(*k);
            t.tile
                .objects
                .iter()
                .chain(&t.tile.attach_objects)
                .filter_map(move |o| o.var_parent.map(|p| (*k, o.id, org + DVec2::new(o.pos[0], o.pos[1]), p)))
                .chain(t.tile.spline_attachments.iter().filter_map(move |a| a.var_parent.map(|p| (*k, a.id, org, p))))
        }).collect(),
        object_ids: doc.tiles.values().flat_map(|t| t.tile.objects.iter().chain(&t.tile.attach_objects).map(|o| o.id).chain(t.tile.spline_attachments.iter().map(|a| a.id)).chain(t.chrono.iter().flat_map(|c| c.objects.iter().chain(&c.attach_objects).map(|o| o.id)))).collect(),
        stops: doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.flag == 7 && o.extra.len() >= 4).map(move |o| (*k, crate::stops::Found { id: o.id, name: o.extra[0].clone(), at: tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1]) }))).collect(),
        tt_doc: (!doc.timetable.lines.is_empty() || !doc.timetable.trips.is_empty() || !doc.traffic.veh_groups.is_empty()).then(|| doc.for_reading()),
    }
}

/// A content file (relative to the roots) that is there.
fn find_file(rel: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    if rel.trim().is_empty() {
        return None;
    }
    roots.iter().map(|r| omsi_cfg::resolve_path(r, rel)).find(|p| omsi_cfg::vfs::is_file(p))
}

/// The textures a content file names that cannot be found, and an object's own street and
/// rail lanes (`[path]`).
fn inspect(path: &Path, is_spline: bool, roots: &[PathBuf]) -> (Vec<String>, Vec<omsi_scenery::PathDef>) {
    let mut paths = Vec::new();
    if !is_spline {
        if let Ok(sco) = omsi_scenery::SceneryObject::load(path) {
            paths = sco.paths.into_iter().filter(|p| p.kind == 0 || p.kind == 2).collect();
        }
    }
    (missing_textures(path, is_spline, roots), paths)
}

/// The textures a content file names that cannot be found.
pub(crate) fn missing_textures(path: &Path, is_spline: bool, roots: &[PathBuf]) -> Vec<String> {
    let (content, omsi) = match (roots.first(), roots.last()) {
        (Some(c), Some(o)) => (c.clone(), o.clone()),
        _ => return Vec::new(),
    };
    let Some(dir) = path.parent() else { return Vec::new() };
    let find = |name: &str, dirs: &[PathBuf]| {
        let refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        omsi_texture::find_texture(name, &refs).is_some()
    };
    let mut out = Vec::new();
    if is_spline {
        let Ok(def) = omsi_scenery::Spline::load(path) else { return out };
        let dirs = crate::assets::texture_dirs(&omsi, &content, dir);
        for t in &def.textures {
            if !t.file.trim().is_empty() && !find(&t.file, &dirs) {
                out.push(t.file.clone());
            }
        }
        return out;
    }
    let Ok(sco) = omsi_scenery::SceneryObject::load(path) else { return out };
    let (model, model_dir) = match &sco.model_file {
        Some(m) => {
            let mp = omsi_cfg::resolve_path(dir, m);
            match omsi_model::Model::load(&mp) {
                Ok(model) => (model, mp.parent().map(Path::to_path_buf).unwrap_or_default()),
                Err(_) => {
                    out.push(m.clone());
                    return out;
                }
            }
        }
        None => (sco.model.clone(), dir.to_path_buf()),
    };
    let mut dirs = crate::assets::texture_dirs(&omsi, &content, &model_dir);
    dirs.insert(0, omsi_cfg::resolve_path(dir, "texture"));
    // (a text on the object needs its font: without it the sign stays blank, in the game too)
    for tt in model.text_textures.iter().filter(|t| crate::texts::label_of(t).is_some()) {
        if !crate::texts::has_font(&tt.font, &omsi, &content) {
            out.push(format!("{} {}", tr("font", "шрифт"), tt.font.trim()));
        }
    }
    for md in &model.meshes {
        let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), &md.file);
        if !omsi_cfg::vfs::is_file(&in_model) && !omsi_cfg::vfs::is_file(&omsi_cfg::resolve_path(&model_dir, &md.file)) {
            out.push(md.file.clone());
            continue;
        }
        // (script and text textures are made as the game runs, and a texture change swaps
        // in a picture of its own folder; a `[matl_freetex]` slot takes the picture the map's
        // label names, so a model that ships none of its own is complete)
        for m in md.materials.iter().filter(|m| m.use_script_texture.is_none() && m.use_text_texture.is_none() && m.change.is_none() && m.freetex.is_none()) {
            if !m.texture.trim().is_empty() && !find(&m.texture, &dirs) {
                out.push(m.texture.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Points on a grid, to find the near ones quickly.
struct Grid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    fn new(cell: f64) -> Grid {
        Grid { cell, cells: HashMap::new() }
    }
    fn key(&self, p: DVec2) -> (i64, i64) {
        ((p.x / self.cell).floor() as i64, (p.y / self.cell).floor() as i64)
    }
    fn add(&mut self, p: DVec2, i: usize) {
        let k = self.key(p);
        self.cells.entry(k).or_default().push(i);
    }
    /// What lies in the cells round `p` (within one cell of it).
    fn near(&self, p: DVec2) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = self.key(p);
        (-1..=1).flat_map(move |dx| (-1..=1).map(move |dy| (x + dx, y + dy))).flat_map(move |k| self.cells.get(&k).into_iter().flatten().copied())
    }
}

/// How many issues of one kind are listed (the rest are counted).
const LIST: usize = 300;

fn run(s: Snapshot, roots: Vec<PathBuf>) -> Vec<Issue> {
    let mut out: Vec<Issue> = Vec::new();
    let issue = |severity, kind, title: String, detail: String, at: Option<DVec2>, sel: Option<Sel>, fix: Option<Fix>| Issue { severity, kind, title, detail, at, sel, fix };

    // files: each missing one once, with how often and where it is first used
    let mut uses: HashMap<String, (bool, usize, DVec2, Sel)> = HashMap::new();
    for o in s.objects.iter().filter(|o| !o.file.trim().is_empty()) {
        let e = uses.entry(o.file.to_lowercase()).or_insert((false, 0, o.world, Sel::Object(o.tile, o.id)));
        e.1 += 1;
    }
    for sp in s.splines.iter().filter(|x| !x.file.trim().is_empty()) {
        let e = uses.entry(sp.file.to_lowercase()).or_insert((true, 0, sp.ends[0].0.truncate(), Sel::Spline(sp.tile, sp.id)));
        e.1 += 1;
    }
    let mut files: Vec<_> = uses.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut tex_missing: HashMap<String, Vec<String>> = HashMap::new();
    let mut obj_paths: HashMap<String, Vec<omsi_scenery::PathDef>> = HashMap::new();
    for (file, (is_spline, n, at, sel)) in &files {
        let name = s.objects.iter().map(|o| &o.file).chain(s.splines.iter().map(|x| &x.file)).find(|f| f.to_lowercase() == *file).cloned().unwrap_or_else(|| file.clone());
        match find_file(&name, &roots) {
            None => {
                let (kind, sev) = if *is_spline { (Kind::MissingSpline, Severity::Error) } else { (Kind::MissingObject, Severity::Error) };
                // (openMapper's own junction plates are made again for their roads)
                let fix = match sel {
                    Sel::Object(_, id) if crate::editor::is_own_junction(&name) => Some(Fix::RemakeJunction(*id)),
                    _ => None,
                };
                out.push(issue(sev, kind, name.clone(), format!("{} {}", tr("used", "используется"), times(*n)), Some(*at), Some(*sel), fix));
            }
            Some(path) => {
                let (tex, paths) = inspect(&path, *is_spline, &roots);
                for t in tex {
                    tex_missing.entry(t).or_default().push(name.clone());
                }
                if !paths.is_empty() {
                    obj_paths.insert(file.clone(), paths);
                }
            }
        }
    }
    let mut tex: Vec<_> = tex_missing.into_iter().collect();
    tex.sort();
    for (t, users) in tex {
        let first = users.first().cloned().unwrap_or_default();
        let at = s.objects.iter().find(|o| o.file == first).map(|o| (o.world, Sel::Object(o.tile, o.id))).or_else(|| s.splines.iter().find(|x| x.file == first).map(|x| (x.ends[0].0.truncate(), Sel::Spline(x.tile, x.id))));
        let more = if users.len() > 1 { format!(" (+{})", users.len() - 1) } else { String::new() };
        out.push(issue(Severity::Warning, Kind::MissingTexture, t, format!("{} {first}{more}", tr("in", "в")), at.map(|a| a.0), at.map(|a| a.1), None));
    }
    for g in &s.ground {
        if g.trim().is_empty() {
            continue;
        }
        let here = omsi_cfg::resolve_path(&s.map_dir, g);
        if !omsi_cfg::vfs::is_file(&here) && find_file(g, &roots).is_none() {
            out.push(issue(Severity::Warning, Kind::MissingTexture, g.clone(), tr("a ground texture of the map", "текстура земли карты").to_string(), None, None, None));
        }
    }

    // links: free ends lying on one another, and links to nothing
    let by_id: HashMap<i64, usize> = s.splines.iter().enumerate().map(|(i, x)| (x.id, i)).collect();
    // (only what carries lanes is worth joining: wires and kerbs branch and end anywhere)
    let kind_of = |file: &str| s.kinds.get(&file.to_lowercase()).filter(|k| matches!(k.cat, Category::Roads | Category::Tracks | Category::Paths) || k.lanes).map(|k| k.cat);
    let mut grid = Grid::new(2.0);
    for (i, sp) in s.splines.iter().enumerate() {
        for (e, free) in [(0, sp.prev == 0), (1, sp.next == 0)] {
            if free {
                grid.add(sp.ends[e].0.truncate(), i * 2 + e);
            }
        }
    }
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    for (i, sp) in s.splines.iter().enumerate() {
        for e in 0..2 {
            if (e == 0 && sp.prev != 0) || (e == 1 && sp.next != 0) {
                continue;
            }
            let (p, h) = sp.ends[e];
            // (three or more free ends at one point are a branch: which two belong together
            // is not for the check to guess)
            let here = grid.near(p.truncate()).filter(|&j| (s.splines[j / 2].ends[j % 2].0 - p).length() < 0.5).count();
            if here > 2 {
                continue;
            }
            for j in grid.near(p.truncate()) {
                let (oi, oe) = (j / 2, j % 2);
                if oi == i || seen.contains(&(j.min(i * 2 + e), j.max(i * 2 + e))) {
                    continue;
                }
                let o = &s.splines[oi];
                let (q, g) = o.ends[oe];
                // (on one another, at one height, running on: their outward headings opposite)
                let turn = ((h - g - 180.0).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
                let same_kind = kind_of(&sp.file) == kind_of(&o.file) && kind_of(&sp.file).is_some();
                if (p - q).truncate().length() < 0.5 && (p.z - q.z).abs() < 0.5 && turn.abs() < 12.0 && same_kind {
                    seen.insert((j.min(i * 2 + e), j.max(i * 2 + e)));
                    out.push(issue(
                        Severity::Info,
                        Kind::UnlinkedEnds,
                        format!("{} {} / {}", tr("splines", "сплайны"), sp.id, o.id),
                        format!("{}, {:.2} m {}", short(&sp.file), (p - q).truncate().length(), tr("apart", "между ними")),
                        Some(p.truncate()),
                        Some(Sel::Spline(sp.tile, sp.id)),
                        Some(Fix::Link((sp.tile, sp.id, e == 1), (o.tile, o.id, oe == 1))),
                    ));
                }
            }
        }
        for (e, link) in [(0, sp.prev), (1, sp.next)] {
            if link != 0 && !by_id.contains_key(&link) {
                out.push(issue(Severity::Warning, Kind::BrokenLink, format!("{} {}", tr("spline", "сплайн"), sp.id), format!("{} {link}", tr("goes on into", "продолжается в")), Some(sp.ends[e].0.truncate()), Some(Sel::Spline(sp.tile, sp.id)), Some(Fix::Unlink((sp.tile, sp.id, e == 1)))));
            }
        }
    }

    // stops and start points beside a lane: the lanes' points on a grid
    let mut lanes = Grid::new(25.0);
    let mut lane_pts: Vec<(DVec2, f64)> = Vec::new();
    for (file, samples) in s.splines.iter().map(|x| (&x.file, &x.samples)).chain(s.chrono_lanes.iter().map(|x| (&x.0, &x.1))) {
        let Some(k) = s.kinds.get(&file.to_lowercase()) else { continue };
        if !k.lanes {
            continue;
        }
        for p in samples {
            lanes.add(*p, lane_pts.len());
            lane_pts.push((*p, k.half_width as f64));
        }
    }
    // (junctions, crossings and whole street pieces carry their own lanes)
    for o in &s.objects {
        let Some(paths) = obj_paths.get(&o.file.to_lowercase()) else { continue };
        let pose = crate::place::Pose::of_object(DVec3::new(o.world.x, o.world.y, 0.0), o.rot);
        for p in paths {
            let c = SplineCurve { start: DVec3::new(p.start[0] as f64, p.start[1] as f64, p.start[2] as f64), heading_deg: p.end[0] as f64, length: p.end[2] as f64, radius: p.end[1] as f64, grad_start: 0.0, grad_end: 0.0, delta_h: None, cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0, half_cant_width: 0.0 };
            if !(c.length > 0.05 && c.length < 5000.0) {
                continue;
            }
            let n = (c.length / 4.0).ceil().max(1.0) as usize;
            for i in 0..=n {
                let q = c.point_at(c.length * i as f64 / n as f64);
                let w = pose.pos + pose.rot.transform_vector3(glam::Vec3::new(q.x as f32, q.y as f32, q.z as f32)).as_dvec3();
                lanes.add(w.truncate(), lane_pts.len());
                lane_pts.push((w.truncate(), 0.0));
            }
        }
    }
    let road_gap = |p: DVec2| -> f64 { lanes.near(p).map(|i| ((lane_pts[i].0 - p).length() - lane_pts[i].1).max(0.0)).fold(f64::MAX, f64::min) };
    let mut stops = 0;
    for o in s.objects.iter().filter(|o| s.kinds.get(&o.file.to_lowercase()).map(|k| k.bus_stop).unwrap_or(false)) {
        let gap = road_gap(o.world);
        if gap > 20.0 {
            stops += 1;
            if stops <= LIST {
                let how = if gap == f64::MAX { tr("no road near", "рядом нет дороги").to_string() } else { format!("{:.0} m {}", gap, tr("from the road", "от дороги")) };
                let fix = (gap < 80.0 && roots.len() >= 2 && !o.attached).then(|| Fix::StopToKerb(o.tile, o.id, roots[0].clone(), roots[roots.len() - 1].clone()));
                out.push(issue(Severity::Warning, Kind::StopOffRoad, short(&o.file), how, Some(o.world), Some(Sel::Object(o.tile, o.id)), fix));
            }
        }
    }
    // stops of one name that are not a pair: the first keeps it, the later ones are listed
    let found: Vec<crate::stops::Found> = s.stops.iter().map(|x| x.1.clone()).collect();
    let mut named = 0;
    for (tile, f) in &s.stops {
        let others = crate::stops::clashes(&found, f.id);
        if others.is_empty() || others.iter().all(|o| o.id > f.id) {
            continue;
        }
        named += 1;
        if named <= LIST {
            let first = others.iter().filter(|o| o.id < f.id).map(|o| o.id).min().unwrap_or(0);
            out.push(issue(Severity::Warning, Kind::StopName, f.name.trim().to_string(), format!("{} {first}", tr("same name as the stop", "то же имя, что у остановки")), Some(f.at), Some(Sel::Object(*tile, f.id)), Some(Fix::UniqueStopName(*tile, f.id))));
        }
    }
    let objects_by_id: HashMap<i64, &ObjRec> = s.objects.iter().map(|o| (o.id, o)).collect();
    for (id, name, _) in &s.entries {
        let label = if name.trim().is_empty() { format!("{} {id}", tr("start point", "точка старта")) } else { name.clone() };
        match objects_by_id.get(id) {
            None => out.push(issue(Severity::Error, Kind::EntryLost, label, format!("{} {id}", tr("object", "объект")), None, None, Some(Fix::DropEntry(*id)))),
            Some(o) => {
                let gap = road_gap(o.world);
                if gap > 10.0 {
                    let how = if gap == f64::MAX { tr("no road near", "рядом нет дороги").to_string() } else { format!("{:.0} m {}", gap, tr("from the road", "от дороги")) };
                    out.push(issue(Severity::Info, Kind::EntryOffRoad, label, how, Some(o.world), Some(Sel::Object(o.tile, o.id)), None));
                }
            }
        }
    }
    if s.entries.is_empty() {
        let fix = (s.splines.len() > 0 && roots.len() >= 2).then(|| Fix::AddStart(roots[0].clone(), roots[roots.len() - 1].clone()));
        out.push(issue(Severity::Warning, Kind::NoEntry, tr("The map has no start point", "На карте нет точки старта").to_string(), String::new(), None, None, fix));
    }

    // IDCodes
    let mut ids: HashMap<i64, Vec<(Sel, DVec2)>> = HashMap::new();
    for o in &s.objects {
        ids.entry(o.id).or_default().push((Sel::Object(o.tile, o.id), o.world));
    }
    for sp in &s.splines {
        ids.entry(sp.id).or_default().push((Sel::Spline(sp.tile, sp.id), sp.ends[0].0.truncate()));
    }
    let mut dups: Vec<_> = ids.iter().filter(|(id, v)| **id > 0 && v.len() > 1).collect();
    dups.sort_by_key(|d| *d.0);
    for (id, v) in dups.iter().take(LIST) {
        out.push(issue(Severity::Warning, Kind::DuplicateId, format!("IDCode {id}"), format!("{} {}", v.len(), tr("things have it", "объекта с ним")), Some(v[0].1), Some(v[0].0), None));
    }
    let max_id = ids.keys().copied().max().unwrap_or(0);
    if max_id >= s.next_id {
        out.push(issue(Severity::Error, Kind::NextId, format!("NextIDCode {}", s.next_id), format!("{} {max_id}", tr("highest in use:", "наибольший занятый:")), None, None, Some(Fix::RaiseNextId(max_id + 1))));
    }

    // objects standing off their own tile (the game looks for their ground on it alone)
    let edge = tile_size() + 1e-3;
    let mut off = 0;
    for o in s.objects.iter().filter(|o| !o.attached) {
        let abs = s.kinds.get(&o.file.to_lowercase()).map(|k| k.abs_height).unwrap_or(true);
        if !abs && !((-1e-3..=edge).contains(&o.local[0]) && (-1e-3..=edge).contains(&o.local[1])) {
            off += 1;
            if off <= LIST {
                // (only when the tile it stands on is there, and nothing hangs on it)
                let to = crate::doc::tile_of(o.world);
                let fix = (s.tiles.contains(&to) && !s.parents.contains(&o.id)).then_some(Fix::MoveToTile(o.tile, o.id));
                out.push(issue(Severity::Warning, Kind::OutsideTile, short(&o.file), format!("{} {},{}", tr("tile", "тайл"), o.tile.0, o.tile.1), Some(o.world), Some(Sel::Object(o.tile, o.id)), fix));
            }
        }
    }

    if let Some(d) = &s.tt_doc {
        timetable_issues(d, &roots, &mut out);
    }

    // tiles
    for (x, y, file) in &s.listed_missing {
        out.push(issue(Severity::Info, Kind::TileMissing, file.clone(), format!("{} {x},{y}", tr("tile", "тайл")), Some(tile_origin((*x, *y)) + DVec2::splat(tile_size() / 2.0)), None, None));
    }
    for k in &s.no_terrain {
        out.push(issue(Severity::Info, Kind::NoGround, format!("tile_{}_{}.map", k.0, k.1), tr("no .terrain file: flat", "нет файла .terrain: плоская").to_string(), Some(tile_origin(*k) + DVec2::splat(tile_size() / 2.0)), None, None));
    }
    // tiles far fuller than the stock maps'
    let mut per_tile: HashMap<TileKey, usize> = HashMap::new();
    for o in &s.objects {
        *per_tile.entry(o.tile).or_default() += 1;
    }
    let mut heavy: Vec<(TileKey, usize)> = per_tile.into_iter().filter(|(_, n)| *n > 1500).collect();
    heavy.sort();
    for (k, n) in heavy {
        out.push(issue(Severity::Info, Kind::HeavyTile, format!("tile_{}_{}.map", k.0, k.1), format!("{n} {}", tr("objects", "объектов")), Some(tile_origin(k) + DVec2::splat(tile_size() / 2.0)), None, None));
    }
    // lamps on tiles the game lights none from
    let mut lamps: HashMap<TileKey, (usize, DVec2)> = HashMap::new();
    for o in s.objects.iter().filter(|o| s.unlit.contains(&o.tile) && s.kinds.get(&o.file.to_lowercase()).map(|k| k.lamp).unwrap_or(false)) {
        lamps.entry(o.tile).or_insert((0, o.world)).0 += 1;
    }
    let mut lamps: Vec<_> = lamps.into_iter().collect();
    lamps.sort_by_key(|(k, _)| *k);
    for (k, (n, at)) in lamps {
        out.push(issue(Severity::Info, Kind::LampsDark, format!("tile_{}_{}.map", k.0, k.1), format!("{n} {}", tr("lamps", "фонарей")), Some(at), None, Some(Fix::LampLight(k))));
    }
    // signal heads that take their light from nothing (a head hanging on a post takes it
    // from the post)
    // (only kinds of head the map links elsewhere: a level crossing's barrier or a flashing
    // warning light runs on its own script)
    let linked: HashSet<i64> = s.var_links.iter().map(|l| l.1).collect();
    let linked_kinds: HashSet<String> = s.objects.iter().filter(|o| linked.contains(&o.id)).map(|o| o.file.to_lowercase()).collect();
    for o in s.objects.iter().filter(|o| !o.attached && !linked.contains(&o.id) && linked_kinds.contains(&o.file.to_lowercase()) && s.kinds.get(&o.file.to_lowercase()).map(|k| k.light_head).unwrap_or(false)) {
        out.push(issue(Severity::Info, Kind::HeadUnlinked, short(&o.file), format!("IDCode {}", o.id), Some(o.world), Some(Sel::Object(o.tile, o.id)), None));
    }
    // objects taking their state from one that is gone
    for (k, id, at, p) in s.var_links.iter().filter(|l| !s.object_ids.contains(&l.3)) {
        let file = s.objects.iter().find(|o| o.id == *id).map(|o| short(&o.file)).unwrap_or_default();
        out.push(issue(Severity::Warning, Kind::LinkLost, file, format!("[varparent] {p}"), Some(*at), Some(Sel::Object(*k, *id)), Some(Fix::DropVarParent(*k, *id))));
    }
    out.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.kind.cmp(&b.kind)));
    out
}

/// Where the depot file of this name is that a bus finds: the map's own (None), the
/// shared `HOFs` folder or any bus's folder (OMSI 2's maps keep theirs there); by its file
/// name or the `[name]` in it, as the game looks. `Some(None)`: nowhere.
fn depot_file(name: &str, d: &Document, roots: &[PathBuf]) -> Option<Option<PathBuf>> {
    let name = name.trim();
    if d.traffic.depots.iter().any(|h| h.file.eq_ignore_ascii_case(name) || h.name.trim().eq_ignore_ascii_case(name)) {
        return None;
    }
    let named = |p: &Path| {
        p.extension().map(|x| x.eq_ignore_ascii_case("hof")).unwrap_or(false)
            && (p.file_stem().map(|s| s.to_string_lossy().trim().eq_ignore_ascii_case(name)).unwrap_or(false)
                || omsi_cfg::CfgFile::read(p).ok().map(|f| f.lines.windows(2).any(|w| w[0].trim_end().eq_ignore_ascii_case("[name]") && w[1].trim().eq_ignore_ascii_case(name))).unwrap_or(false))
    };
    let mut dirs = roots.iter().flat_map(|r| std::iter::once(r.join("HOFs")).chain(std::fs::read_dir(r.join("Vehicles")).into_iter().flatten().flatten().map(|v| v.path())));
    Some(dirs.find_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).find(|p| named(p))))
}

/// The depot files the buses of the map's lines need.
fn depot_issues(d: &Document, roots: &[PathBuf], out: &mut Vec<Issue>) {
    if !d.timetable.trips.iter().any(|t| t.stations.len() >= 2) {
        return;
    }
    let mut named: Vec<String> = d.traffic.depots().filter_map(|g| g.hof.clone()).filter(|h| !h.trim().is_empty()).collect();
    named.sort_by_key(|n| n.to_lowercase());
    named.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if named.is_empty() && d.traffic.depots.is_empty() {
        out.push(Issue { severity: Severity::Warning, kind: Kind::DepotFile, title: tr("The map has no depot file", "У карты нет файла депо").into(), detail: tr("the buses show no destination", "автобусы не показывают конечную").into(), at: None, sel: None, fix: Some(Fix::MakeDepot(String::new())) });
        return;
    }
    // the destinations the trips go to
    let mut termini: Vec<String> = d.timetable.trips.iter().filter(|t| t.stations.len() >= 2).map(|t| t.terminus.trim().to_string()).filter(|t| !t.is_empty()).collect();
    termini.sort_by_key(|t| t.to_lowercase());
    termini.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    for n in &named {
        match depot_file(n, d, roots) {
            None => {}
            Some(None) => out.push(Issue { severity: Severity::Warning, kind: Kind::DepotFile, title: format!("{n}.hof"), detail: tr("named by the map's depot, but found nowhere", "указан депо карты, но нигде не найден").into(), at: None, sel: None, fix: Some(Fix::MakeDepot(n.clone())) }),
            Some(Some(p)) => {
                // another map's (a new map's depot has Grundorf's): does it know the lines'
                // destinations?
                let Some(h) = crate::hof::Hof::load(&p) else { continue };
                let lacking: Vec<&String> = termini.iter().filter(|t| h.terminus_named(t).is_none()).collect();
                if !lacking.is_empty() {
                    let all = lacking.len() == termini.len();
                    let detail = format!("{} {} {}: {}", if all { tr("knows none of the", "не знает ни одной из") } else { tr("lacks", "не хватает") }, lacking.len(), tr("destinations of the lines", "конечных линий"), lacking.iter().take(4).map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
                    out.push(Issue { severity: if all { Severity::Warning } else { Severity::Info }, kind: Kind::DepotFile, title: format!("{n}.hof"), detail, at: None, sel: None, fix: Some(Fix::OwnDepot(n.clone())) });
                }
            }
        }
    }
    for h in &d.traffic.depots {
        let (a, b, c) = h.clone().fill_from(&d.timetable);
        if (a, b, c) != (0, 0, 0) {
            let detail = format!("{} {a} {}, {b} {}, {c} {}", tr("the lines need", "линиям не хватает:"), tr("destinations", "конечных"), tr("stops", "остановок"), tr("routes", "маршрутов"));
            out.push(Issue { severity: Severity::Info, kind: Kind::DepotFile, title: h.file_name(), detail, at: None, sel: None, fix: Some(Fix::FillDepot(h.file.clone())) });
        }
    }
}

/// The timetable's and the traffic's issues, the trips' ways found as the Lines window
/// finds them.
fn timetable_issues(d: &Document, roots: &[PathBuf], out: &mut Vec<Issue>) {
    let tt = &d.timetable;
    let depots: HashSet<String> = d.traffic.depots().map(|g| g.name.to_lowercase()).collect();
    let groups: HashSet<String> = d.traffic.groups.iter().map(|g| g.name.to_lowercase()).collect();
    let mut files = crate::timetable::network::Files::new(roots.to_vec());
    let place = |files: &mut crate::timetable::network::Files, id: i64| -> (Option<DVec2>, Option<Sel>) {
        let p = crate::timetable::stop_world(d, files, id).map(|p| p.truncate());
        let k = d.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == id)).map(|(k, _)| *k);
        (p, k.map(|k| Sel::Object(k, id)))
    };
    let stop_name = |id: i64| tt.stop_name(id).map(str::to_string).unwrap_or_else(|| format!("#{id}"));
    let can_add_depot = find_file(crate::traffic::DEPOT_BUS, roots).is_some();
    for line in &tt.lines {
        for tour in &line.tours {
            let label = format!("{} {} {}", line.name, tr("tour", "выход"), tour.number);
            if tour.trips.is_empty() {
                out.push(Issue { severity: Severity::Warning, kind: Kind::TourEmpty, title: label.clone(), detail: String::new(), at: None, sel: None, fix: None });
            }
            // (a bus's tour names a depot; a train's an ordinary group of the AI lists)
            if tour.ai_group.trim().is_empty() || !(depots.contains(&tour.ai_group.to_lowercase()) || groups.contains(&tour.ai_group.to_lowercase())) {
                let detail = if tour.ai_group.trim().is_empty() { tr("no depot named", "депо не указано").to_string() } else { format!("{} «{}»", tr("not in the AI lists:", "нет в списке ИИ:"), tour.ai_group) };
                let fix = (!depots.is_empty() || can_add_depot).then(|| Fix::TourDepot(line.name.clone(), tour.number.clone(), can_add_depot));
                out.push(Issue { severity: Severity::Warning, kind: Kind::TourNoDepot, title: label, detail, at: None, sel: None, fix });
            }
        }
    }
    // (OMSI 2's own maps have the groups its template names)
    let omsi = roots.last();
    let stock: Vec<String> = match omsi {
        Some(o) if d.traffic.veh_groups.iter().any(|(g, _)| !groups.contains(&g.to_lowercase())) => crate::traffic::stock_groups(o).into_iter().map(|g| g.name.to_lowercase()).collect(),
        _ => Vec::new(),
    };
    for (g, _) in &d.traffic.veh_groups {
        if !groups.contains(&g.to_lowercase()) {
            let fix = omsi.filter(|_| stock.contains(&g.to_lowercase())).map(|o| Fix::StockGroups(o.clone()));
            out.push(Issue { severity: Severity::Warning, kind: Kind::UnschedMissing, title: g.clone(), detail: String::new(), at: None, sel: None, fix });
        }
    }
    depot_issues(d, roots, out);
    let trips: Vec<_> = tt.trips.iter().filter(|t| t.stations.len() >= 2).collect();
    if !trips.is_empty() {
        let net = crate::timetable::network::Network::build(d, &mut files);
        for trip in trips {
            let pos: Vec<Option<DVec3>> = trip.stations.iter().map(|id| crate::timetable::stop_world(d, &mut files, *id)).collect();
            let r = crate::timetable::route::find(&net, &pos, d.global.left_hand_traffic);
            if r.ok() {
                continue;
            }
            use crate::timetable::route::Problem;
            let (i, detail) = match r.problems.first() {
                Some(Problem::NoObject(i)) => (*i, format!("{} {}", tr("the stop has no object:", "у остановки нет объекта:"), stop_name(trip.stations[*i]))),
                Some(Problem::NoRoad(i)) => (*i, format!("{} {}", tr("no road beside", "нет дороги у"), stop_name(trip.stations[*i]))),
                Some(Problem::NoWay(i)) => (*i, format!("{} {} -> {}", tr("no way", "нет пути"), stop_name(trip.stations[*i]), trip.stations.get(i + 1).map(|s| stop_name(*s)).unwrap_or_default())),
                None => (0, tr("no way found", "путь не найден").to_string()),
            };
            let (at, sel) = trip.stations.get(i).map(|id| place(&mut files, *id)).unwrap_or((None, None));
            out.push(Issue { severity: Severity::Warning, kind: Kind::TripNoRoute, title: trip.name.clone(), detail, at, sel, fix: None });
        }
    }
    // signal routes: their signal and pieces on the map (or in a chrono folder's tile)
    let ids: HashSet<i64> = d.tiles.values().flat_map(|t| t.tile.objects.iter().chain(&t.tile.attach_objects).map(|o| o.id).chain(t.tile.spline_attachments.iter().map(|a| a.id)).chain(t.tile.splines.iter().filter(|s| !s.deleted).map(|s| s.id)).chain(t.chrono.iter().flat_map(|c| c.objects.iter().map(|o| o.id).chain(c.splines.iter().map(|s| s.id))))).collect();
    for (i, r) in d.traffic.signal_routes.iter().enumerate() {
        // (a piece in a tile the map lists but does not ship is not looked for)
        let shipped = |ti: i64| d.order.get(ti as usize).map(|k| d.tiles.contains_key(k)).unwrap_or(false);
        let gone = std::iter::once(r.signal.0).chain(r.entries.iter().filter(|e| shipped(e[2])).map(|e| e[0])).filter(|id| !ids.contains(id)).count();
        if gone > 0 {
            let k = d.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == r.signal.0)).map(|(k, _)| *k);
            out.push(Issue { severity: Severity::Warning, kind: Kind::SignalRouteBroken, title: format!("{} {i}", tr("Route", "Маршрут")), detail: format!("{gone} {}", tr("gone", "нет на карте")), at: None, sel: k.map(|k| Sel::Object(k, r.signal.0)), fix: None });
        }
    }
    // chrono scenarios: their changes name things of the map or of a scenario
    let root = omsi_cfg::resolve_path(&d.chrono_root(), "Chrono");
    let mut patches: Vec<(String, omsi_map::Tile)> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for p in std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x.eq_ignore_ascii_case("map")).unwrap_or(false) {
                if let Ok(t) = omsi_map::Tile::load(&p) {
                    let name = p.parent().and_then(|d| d.strip_prefix(&root).ok()).map(|r| r.display().to_string()).unwrap_or_default();
                    patches.push((name, t));
                }
            }
        }
    }
    let added: HashSet<i64> = patches.iter().flat_map(|(_, t)| t.objects.iter().chain(&t.attach_objects).map(|o| o.id).chain(t.splines.iter().map(|s| s.id)).chain(t.spline_attachments.iter().map(|a| a.id))).collect();
    let mut by_scenario: std::collections::BTreeMap<String, usize> = Default::default();
    for (name, t) in &patches {
        for c in &t.chrono_changes {
            if !ids.contains(&c.id) && !added.contains(&c.id) {
                *by_scenario.entry(name.clone()).or_default() += 1;
            }
        }
    }
    for (name, n) in by_scenario {
        out.push(Issue { severity: Severity::Info, kind: Kind::ChronoUnmatched, title: name, detail: format!("{n} {}", tr("changes of nothing", "изменений впустую")), at: None, sel: None, fix: None });
    }
    let used: HashSet<i64> = tt.trips.iter().flat_map(|t| t.stations.iter().copied()).collect();
    for st in tt.stops.iter().filter(|st| !used.contains(&st.object_id)) {
        let (at, sel) = place(&mut files, st.object_id);
        out.push(Issue { severity: Severity::Info, kind: Kind::StopUnused, title: st.name.clone(), detail: String::new(), at, sel, fix: None });
    }
}

fn times(n: usize) -> String {
    format!("{n} {}", if n == 1 { tr("time", "раз") } else { tr("times", "раз") })
}

/// A file's name without its folders.
fn short(file: &str) -> String {
    file.rsplit(['\\', '/']).next().unwrap_or(file).to_string()
}

/// Put an issue right (as one step that can be undone). Returns whether anything changed.
pub fn apply(ed: &mut Editor, fix: &Fix) -> bool {
    // (made at the next tick, which has the road models)
    if let Fix::RemakeJunction(id) = fix {
        return ed.remake_junction(*id);
    }
    let Some(doc) = ed.doc.as_mut() else { return false };
    ed.history.begin(tr("Fix from the map check", "Исправление из проверки карты"));
    let done = match fix {
        Fix::Link(a, b) => {
            // (still both free: the map may have changed since the check)
            let free = |doc: &Document, e: &End| doc.spline(e.0, e.1).map(|s| if e.2 { s.next_id == 0 } else { s.prev_id == 0 }).unwrap_or(false);
            if free(doc, a) && free(doc, b) && a.1 != b.1 {
                for (me, other) in [(a, b), (b, a)] {
                    ed.history.touch_tile(doc, me.0);
                    let t = doc.tiles.get_mut(&me.0).unwrap();
                    if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == me.1 && !s.deleted) {
                        if me.2 {
                            s.next_id = other.1;
                        } else {
                            s.prev_id = other.1;
                        }
                    }
                    t.touch_content();
                }
                true
            } else {
                false
            }
        }
        Fix::Unlink(e) => {
            ed.history.touch_tile(doc, e.0);
            let mut hit = false;
            if let Some(t) = doc.tiles.get_mut(&e.0) {
                if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == e.1 && !s.deleted) {
                    if e.2 {
                        s.next_id = 0;
                    } else {
                        s.prev_id = 0;
                    }
                    hit = true;
                }
                if hit {
                    t.touch_content();
                }
            }
            hit
        }
        Fix::RaiseNextId(n) => {
            ed.history.touch_global(doc);
            doc.global.next_id_code = doc.global.next_id_code.max(*n);
            doc.global_dirty = true;
            true
        }
        Fix::MoveToTile(from, id) => {
            let o = doc.object(*from, *id).cloned();
            match o {
                Some(o) => {
                    let world = tile_origin(*from) + DVec2::new(o.pos[0], o.pos[1]);
                    let to = crate::doc::tile_of(world);
                    if to != *from && doc.tiles.contains_key(&to) {
                        ed.history.touch_global(doc);
                        ed.history.touch_tile(doc, *from);
                        ed.history.touch_tile(doc, to);
                        // (with what hangs on it)
                        crate::selection::place_object_at(doc, *from, *id, world);
                        // (a start point names its object's tile)
                        crate::editor::sync_entry_points(doc);
                        if ed.selected == Some(Sel::Object(*from, *id)) {
                            ed.select(Some(Sel::Object(to, *id)));
                        }
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        }
        Fix::LampLight(k) => {
            ed.history.touch_tile(doc, *k);
            match doc.tiles.get_mut(k) {
                Some(t) if !t.tile.variable_terrain_lightmap => {
                    t.tile.variable_terrain_lightmap = true;
                    t.touch_content();
                    true
                }
                _ => false,
            }
        }
        Fix::DropVarParent(k, id) => {
            ed.history.touch_tile(doc, *k);
            let cleared = if let Some(o) = crate::selection::any_object_mut(doc, *k, *id) {
                o.var_parent.take().is_some()
            } else if let Some(a) = doc.attachment_mut(*k, *id) {
                a.var_parent.take().is_some()
            } else {
                false
            };
            if cleared {
                doc.tiles.get_mut(k).map(|t| t.touch_content());
            }
            cleared
        }
        Fix::UniqueStopName(k, id) => {
            let stops = crate::stops::all(doc);
            let Some(me) = stops.iter().find(|f| f.id == *id) else { return false };
            let name = crate::stops::unique_name(&stops, Some(*id), &me.name, me.at);
            ed.history.touch_tile(doc, *k);
            match crate::selection::any_object_mut(doc, *k, *id) {
                Some(o) if o.extra.first().map(|n| n.trim()) != Some(name.as_str()) => {
                    o.extra[0] = name;
                    doc.tiles.get_mut(k).map(|t| t.touch_content());
                    true
                }
                _ => false,
            }
        }
        Fix::StockGroups(omsi) => {
            let before = doc.traffic.clone();
            let n = crate::traffic::fill_named_groups(&mut doc.traffic, omsi);
            if n > 0 {
                ed.history.push_snapshot(tr("Car groups from OMSI 2's maps", "Группы машин из карт OMSI 2"), crate::history::Snapshot { traffic: Some(before), ..Default::default() });
            }
            n > 0
        }
        Fix::AddStart(content, omsi) => {
            let roots = crate::install::Roots { omsi: omsi.clone(), content: content.clone() };
            match crate::sample::start_spot(doc, &roots).filter(|_| doc.global.entry_points.is_empty()) {
                Some((at, heading)) => {
                    ed.history.touch_tile(doc, crate::doc::tile_of(at));
                    ed.history.touch_global(doc);
                    crate::sample::add_start(doc, at, heading, tr("Start", "Старт"));
                    true
                }
                None => false,
            }
        }
        Fix::TourDepot(line, number, can_add) => {
            let before = crate::history::Snapshot { timetable: Some(doc.timetable.clone()), traffic: Some(doc.traffic.clone()), ..Default::default() };
            if doc.traffic.depots().next().is_none() && *can_add {
                crate::ui_timetable::add_depot(doc);
            }
            let depot = doc.traffic.depots().next().map(|d| d.name.clone());
            let tour = doc.timetable.lines.iter_mut().find(|l| &l.name == line).and_then(|l| l.tours.iter_mut().find(|t| &t.number == number));
            match (depot, tour) {
                (Some(d), Some(t)) => {
                    t.ai_group = d;
                    doc.timetable.touch_line(line);
                    doc.timetable.dirty = true;
                    ed.history.push_snapshot(tr("A depot for the tour", "Депо для выхода"), before);
                    true
                }
                _ => false,
            }
        }
        Fix::RemakeJunction(_) => false,
        Fix::MakeDepot(name) | Fix::OwnDepot(name) => {
            let before = crate::history::Snapshot { traffic: Some(doc.traffic.clone()), ..Default::default() };
            let file = crate::ui_hof::make_named(doc, if matches!(fix, Fix::OwnDepot(_)) { "" } else { name });
            for g in doc.traffic.groups.iter_mut().filter(|g| g.is_depot && (g.hof.as_deref().unwrap_or("").trim().is_empty() || g.hof.as_deref().unwrap_or("").eq_ignore_ascii_case(name))) {
                g.hof = Some(file.clone());
            }
            doc.traffic.touch(crate::traffic::File::AiLists);
            ed.history.push_snapshot(tr("A depot file from the lines", "Файл депо по линиям"), before);
            true
        }
        Fix::FillDepot(file) => {
            let before = crate::history::Snapshot { traffic: Some(doc.traffic.clone()), ..Default::default() };
            let tt = &doc.timetable;
            match doc.traffic.depots.iter_mut().find(|h| h.file.eq_ignore_ascii_case(file)) {
                Some(h) => {
                    h.fill_from(tt);
                    doc.traffic.touch(crate::traffic::File::Depots);
                    ed.history.push_snapshot(tr("Depot file: what the lines need", "Файл депо: что нужно линиям"), before);
                    true
                }
                None => false,
            }
        }
        Fix::StopToKerb(k, id, content, omsi) => match doc.object(*k, *id).map(|o| tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1])) {
            Some(at) => {
                let net = crate::timetable::network::Network::build(doc, &mut crate::timetable::network::Files::new(vec![content.clone(), omsi.clone()]));
                // the nearest point of a lane, and the way the lane goes there
                let mut best: Option<(f64, DVec2, DVec2)> = None;
                for l in net.lanes.iter().filter(|l| !l.against) {
                    for w in l.pts.windows(2) {
                        let (a, b) = (w[0].truncate(), w[1].truncate());
                        let e = b - a;
                        if e.length_squared() < 1e-6 {
                            continue;
                        }
                        let q = a + e * ((at - a).dot(e) / e.length_squared()).clamp(0.0, 1.0);
                        let d = (q - at).length();
                        if best.map(|b| d < b.0).unwrap_or(true) {
                            best = Some((d, q, e.normalize()));
                        }
                    }
                }
                // (on the side the doors open to)
                let side = if doc.global.left_hand_traffic { -1.0 } else { 1.0 };
                match best.filter(|b| b.0 < 80.0) {
                    Some((_, q, dir)) if crate::doc::tile_of(q + DVec2::new(dir.y, -dir.x) * side * 3.2) == *k => {
                        let to = q + DVec2::new(dir.y, -dir.x) * side * 3.2;
                        ed.history.touch_tile(doc, *k);
                        let org = tile_origin(*k);
                        if let Some(o) = doc.object_mut(*k, *id) {
                            o.pos[0] = to.x - org.x;
                            o.pos[1] = to.y - org.y;
                            o.rot[0] = crate::roads::heading_of(dir);
                        }
                        doc.tiles.get_mut(k).map(|t| t.touch_content());
                        true
                    }
                    _ => false,
                }
            }
            None => false,
        },
        Fix::DropEntry(id) => {
            ed.history.touch_global(doc);
            let before = doc.global.entry_points.len();
            doc.global.entry_points.retain(|e| e.object_id != *id);
            doc.global_dirty = true;
            doc.global.entry_points.len() != before
        }
    };
    ed.history.commit();
    done
}

/// The check window and its last results.
#[derive(Default)]
pub struct Checker {
    pub open: bool,
    rx: Option<mpsc::Receiver<Vec<Issue>>>,
    pub issues: Option<Vec<Issue>>,
    /// Kinds folded away in the list.
    folded: HashSet<Kind>,
    /// Issues put right since the check ran (their place in the list).
    fixed: HashSet<usize>,
    started: Option<std::time::Instant>,
}

impl Checker {
    /// The errors the last check found that are not put right yet (None: not checked).
    pub fn errors_left(&self) -> Option<usize> {
        self.issues.as_ref().map(|l| l.iter().enumerate().filter(|(i, x)| x.severity == Severity::Error && !self.fixed.contains(i)).count())
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    /// Check the map (on a thread; the results come in later).
    pub fn start(&mut self, doc: &Document, lib: &Library) {
        let snap = snapshot(doc, lib);
        let roots = omsi_cfg::content_roots();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // (a panic on an odd file is a failed check, not a crash)
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(snap, roots))).unwrap_or_default();
            let _ = tx.send(r);
        });
        self.rx = Some(rx);
        self.started = Some(std::time::Instant::now());
        self.fixed.clear();
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.issues = Some(r);
                    self.rx = None;
                    crate::render::profile::log_time("map check", self.started.unwrap_or_else(std::time::Instant::now));
                }
                Err(mpsc::TryRecvError::Disconnected) => self.rx = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// The window (Map > Check map).
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, actions: &mut Vec<Action>) {
        self.poll();
        if !self.open {
            return;
        }
        if self.issues.is_none() && !self.running() && !lib.scanning {
            if let Some(doc) = &ed.doc {
                self.start(doc, lib);
            }
        }
        let mut open = self.open;
        let mut fix_now: Vec<(usize, Fix)> = Vec::new();
        egui::Window::new(icons::labelled(Icon::CheckCircle, tr("Map check", "Проверка карты"))).id(egui::Id::new("map check")).open(&mut open).default_pos([300.0, 90.0]).default_size([460.0, 520.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let can = !self.running() && !lib.scanning && ed.doc.is_some();
                if ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::CheckCircle, tr("Check again", "Проверить снова")))).clicked() {
                    if let Some(doc) = &ed.doc {
                        self.start(doc, lib);
                    }
                }
                if self.running() || lib.scanning {
                    ui.spinner();
                    ui.label(RichText::new(tr("Checking the map…", "Проверяем карту…")).color(Color32::GRAY));
                } else if let Some(list) = &self.issues {
                    let errors = list.iter().filter(|i| i.severity == Severity::Error).count();
                    let warnings = list.iter().filter(|i| i.severity == Severity::Warning).count();
                    let notes = list.len() - errors - warnings;
                    ui.label(RichText::new(format!("{} {errors} · {} {warnings} · {} {notes}", tr("errors", "ошибок"), tr("warnings", "предупреждений"), tr("notes", "заметок"))).color(Color32::GRAY));
                }
            });
            ui.separator();
            let Some(list) = &self.issues else { return };
            if list.is_empty() {
                ui.label(icons::labelled(Icon::CheckCircle, tr("Nothing found: the map looks fine.", "Ничего не найдено: карта в порядке.")));
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut i = 0;
                while i < list.len() {
                    let kind = list[i].kind;
                    let end = list[i..].iter().position(|x| x.kind != kind).map(|n| i + n).unwrap_or(list.len());
                    let group = i..end;
                    let sev = list[i].severity;
                    let (icon, color) = match sev {
                        Severity::Error => (Icon::Error, Color32::from_rgb(235, 100, 90)),
                        Severity::Warning => (Icon::Warning, Color32::from_rgb(235, 180, 70)),
                        Severity::Info => (Icon::Info, Color32::from_rgb(130, 170, 230)),
                    };
                    let left = group.clone().filter(|n| !self.fixed.contains(n)).count();
                    let folded = self.folded.contains(&kind);
                    ui.horizontal(|ui| {
                        let arrow = if folded { Icon::ChevronRight } else { Icon::ExpandMore };
                        if ui.add(egui::Button::new(icons::labelled(arrow, &format!("{} ({left})", kind.label()))).frame(false)).clicked() {
                            if folded {
                                self.folded.remove(&kind);
                            } else {
                                self.folded.insert(kind);
                            }
                        }
                        ui.label(icon.text(15.0).color(color));
                        let fixable: Vec<usize> = group.clone().filter(|n| list[*n].fix.is_some() && !self.fixed.contains(n)).collect();
                        if fixable.len() > 1 && ui.small_button(tr("Fix all", "Исправить все")).clicked() {
                            for n in fixable {
                                fix_now.push((n, list[n].fix.clone().unwrap()));
                            }
                        }
                    });
                    if !folded {
                        ui.label(RichText::new(kind.help()).small().color(Color32::GRAY));
                        for n in group.clone() {
                            let it = &list[n];
                            let done = self.fixed.contains(&n);
                            // (the button first, at the right: the text gets what is left and is
                            // cut short there, shown whole when the mouse is over it)
                            let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let (Some(f), false) = (&it.fix, done) {
                                    let label = if matches!(f, Fix::Link(..)) { tr("Join", "Соединить") } else { tr("Fix", "Исправить") };
                                    if ui.small_button(label).clicked() {
                                        fix_now.push((n, f.clone()));
                                    }
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add_space(12.0);
                                    let full = format!("{}  {}", it.title, it.detail);
                                    let text = RichText::new(full.clone());
                                    let text = if done { text.strikethrough().color(Color32::DARK_GRAY) } else { text };
                                    let r = ui.add(egui::Label::new(text).sense(egui::Sense::click()).truncate());
                                    let hint = if it.at.is_some() { format!("{full}\n{}", tr("Click to fly there", "Клик: перелететь туда")) } else { full };
                                    let r = r.on_hover_text(hint);
                                    if let (Some(at), true) = (it.at, r.clicked()) {
                                        actions.push(Action::LookAt(at));
                                        if it.sel.is_some() {
                                            ed.select(it.sel);
                                        }
                                    }
                                });
                            });
                        }
                        ui.add_space(6.0);
                    }
                    i = end;
                }
            });
        });
        self.open = open;
        let mut n_done = 0;
        for (n, f) in fix_now {
            if self.fixed.contains(&n) {
                continue;
            }
            if apply(ed, &f) {
                self.fixed.insert(n);
                n_done += 1;
                // (one fix may put right several: the car groups all copied at once)
                let same: Vec<usize> = self.issues.iter().flatten().enumerate().filter(|(_, it)| it.fix.as_ref() == Some(&f)).map(|(i, _)| i).collect();
                self.fixed.extend(same);
            }
        }
        if n_done > 0 {
            ed.toast(format!("{} {n_done}", tr("Fixed:", "Исправлено:")));
        }
    }
}

/// The map's problems, found without a window (the library must have been read).
pub fn issues(doc: &Document, lib: &Library) -> Vec<Issue> {
    run(snapshot(doc, lib), omsi_cfg::content_roots())
}

/// `openmapper --check path/to/global.cfg`: read the map, check it and print what was
/// found, without a window. The exit code is 1 when there are errors.
pub fn cli(cfg: &Path) -> i32 {
    let prefs = crate::install::Prefs::load();
    let Some(omsi) = crate::install::find_omsi_root(&prefs) else {
        eprintln!("no OMSI 2 installation found");
        return 2;
    };
    let roots = crate::install::Roots::new(omsi);
    let mut lib = Library::default();
    lib.start_scan(&roots);
    let t = std::time::Instant::now();
    let doc = match Document::load(&roots, cfg, |_| {}) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", cfg.display());
            return 2;
        }
    };
    println!("{}: {} tiles read in {:.2} s", cfg.display(), doc.tiles.len(), t.elapsed().as_secs_f32());
    while lib.scanning {
        lib.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let t = std::time::Instant::now();
    let issues = run(snapshot(&doc, &lib), omsi_cfg::content_roots());
    let mut kinds: Vec<Kind> = issues.iter().map(|i| i.kind).collect();
    kinds.dedup();
    for k in kinds {
        let of: Vec<&Issue> = issues.iter().filter(|i| i.kind == k).collect();
        println!("\n{:?} {} ({})", of[0].severity, k.label(), of.len());
        for i in of.iter().take(8) {
            let at = i.at.map(|p| format!("  at {:.0}, {:.0}", p.x, p.y)).unwrap_or_default();
            println!("  {}  {}{at}{}", i.title, i.detail, if i.fix.is_some() { "  [fix]" } else { "" });
        }
        if of.len() > 8 {
            println!("  …");
        }
    }
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    println!("\n{} issues ({errors} errors), checked in {:.2} s", issues.len(), t.elapsed().as_secs_f32());
    (errors > 0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spline(id: i64, x: f64, heading: f64, prev: i64, next: i64) -> SplineRec {
        let p = DVec3::new(x, 0.0, 0.0);
        let q = p + DVec3::new(heading.to_radians().sin(), heading.to_radians().cos(), 0.0) * 10.0;
        SplineRec { file: "Splines\\road.sli".into(), tile: (0, 0), id, prev, next, ends: [(p, heading + 180.0), (q, heading)], samples: vec![p.truncate(), q.truncate()] }
    }

    fn snap(splines: Vec<SplineRec>) -> Snapshot {
        let mut kinds = HashMap::new();
        kinds.insert("splines\\road.sli".to_string(), KindRec { cat: Category::Roads, half_width: 4.0, bus_stop: false, lanes: true, abs_height: false, lamp: false, light_head: false });
        Snapshot { objects: Vec::new(), splines, chrono_lanes: Vec::new(), kinds, entries: Vec::new(), next_id: 100, listed_missing: Vec::new(), no_terrain: Vec::new(), unlit: HashSet::new(), ground: Vec::new(), map_dir: PathBuf::new(), tiles: HashSet::new(), parents: HashSet::new(), var_links: Vec::new(), object_ids: HashSet::new(), tt_doc: None, stops: Vec::new() }
    }

    #[test]
    fn far_stops_of_one_name_are_listed_but_a_pair_is_not() {
        let stop = |id: i64, name: &str, x: f64| ((0, 0), crate::stops::Found { id, name: name.into(), at: DVec2::new(x, 0.0) });
        let mut s = snap(Vec::new());
        s.stops = vec![stop(1, "Market", 0.0), stop(2, "Market", 15.0), stop(3, "Market", 600.0), stop(4, "Station", 0.0)];
        let issues = run(s, Vec::new());
        let named: Vec<_> = issues.iter().filter(|i| i.kind == Kind::StopName).collect();
        // the third is the one listed: the first keeps the name, the second is its pair
        assert_eq!(named.len(), 1, "{named:?}");
        assert_eq!(named[0].fix, Some(Fix::UniqueStopName((0, 0), 3)));
    }

    fn doc_with(tiles: &[TileKey]) -> Document {
        let mut doc = Document {
            global: Default::default(),
            tiles: Default::default(),
            order: Vec::new(),
            dir: PathBuf::new(),
            source: None,
            global_dirty: false,
            fresh: false,
            global_rev: 0,
            timetable: Default::default(),
            traffic: Default::default(),
            descriptions: Default::default(),
            groups: Vec::new(),
        };
        for k in tiles {
            doc.tiles.insert(*k, crate::doc::TileDoc::new_flat(0.0));
            doc.order.push(*k);
        }
        doc
    }

    #[test]
    fn join_links_both_ends_and_undoes() {
        let _map = crate::doc::one_map();
        let mut doc = doc_with(&[(0, 0)]);
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        for id in [1, 2] {
            t.tile.splines.push(omsi_map::MapSpline { id, length: 10.0, ..Default::default() });
        }
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        assert!(apply(&mut ed, &Fix::Link(((0, 0), 1, true), ((0, 0), 2, false))));
        let d = ed.doc.as_ref().unwrap();
        assert_eq!((d.spline((0, 0), 1).unwrap().next_id, d.spline((0, 0), 2).unwrap().prev_id), (2, 1));
        // (not twice: the ends are no longer free)
        assert!(!apply(&mut ed, &Fix::Link(((0, 0), 1, true), ((0, 0), 2, false))));
        let mut d = ed.doc.take().unwrap();
        ed.history.undo(&mut d);
        assert_eq!(d.spline((0, 0), 1).unwrap().next_id, 0);
    }

    #[test]
    fn links_are_made_picked_in_a_ring_refused_and_forgotten() {
        let _map = crate::doc::one_map();
        let mut doc = doc_with(&[(0, 0), (1, 0)]);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { id: 1, file: "crossing.sco".into(), ..Default::default() });
        doc.tiles.get_mut(&(1, 0)).unwrap().tile.objects.push(omsi_map::MapObject { id: 2, file: "head.sco".into(), extra: vec!["0".into()], flag: 1, ..Default::default() });
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        // a head on the next tile takes its light from the crossing
        ed.set_var_parent((1, 0), 2, Some(1));
        ed.set_light_index((1, 0), 2, 3);
        let d = ed.doc.as_ref().unwrap();
        assert_eq!(crate::selection::var_parent_of(d, 2), Some(1));
        assert_eq!(crate::selection::var_children(d, 1).iter().map(|(k, o)| (*k, o.id, o.extra[0].clone())).collect::<Vec<_>>(), vec![((1, 0), 2, "3".to_string())]);
        // the crossing cannot take its state from its own head
        ed.link_pick = Some(((0, 0), 1));
        ed.hover = Some(Sel::Object((1, 0), 2));
        ed.left_press(&mut crate::assets::Assets::new(PathBuf::new(), PathBuf::new()), &Library::default(), false);
        assert_eq!(ed.link_pick, None);
        assert_eq!(crate::selection::var_parent_of(ed.doc.as_ref().unwrap(), 1), None);
        // the crossing gone: the check names the head, its fix forgets the link (one step)
        ed.doc.as_mut().unwrap().tiles.get_mut(&(0, 0)).unwrap().tile.objects.clear();
        let lost: Vec<_> = run(snapshot(ed.doc.as_ref().unwrap(), &Library::default()), Vec::new()).into_iter().filter(|i| i.kind == Kind::LinkLost).collect();
        assert_eq!(lost.len(), 1);
        assert!(apply(&mut ed, lost[0].fix.as_ref().unwrap()));
        assert_eq!(crate::selection::var_parent_of(ed.doc.as_ref().unwrap(), 2), None);
        let mut d = ed.doc.take().unwrap();
        ed.history.undo(&mut d);
        assert_eq!(crate::selection::var_parent_of(&d, 2), Some(1));
    }

    #[test]
    fn car_groups_the_traffic_names_are_copied_from_the_stock_maps() {
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
        let _map = crate::doc::one_map();
        let mut doc = doc_with(&[(0, 0)]);
        doc.traffic.veh_groups = vec![("NormalCars".into(), 10), ("NoSuchGroup".into(), 1)];
        let mut out = Vec::new();
        timetable_issues(&doc, &[PathBuf::new(), omsi.clone()], &mut out);
        let fixes: Vec<_> = out.iter().filter(|i| i.kind == Kind::UnschedMissing).map(|i| (i.title.as_str(), i.fix.is_some())).collect();
        assert_eq!(fixes, [("NormalCars", true), ("NoSuchGroup", false)]);
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        assert!(apply(&mut ed, &Fix::StockGroups(omsi.clone())));
        assert!(ed.doc.as_ref().unwrap().traffic.groups.iter().any(|g| g.name == "NormalCars" && !g.vehicles.is_empty()));
        assert!(!apply(&mut ed, &Fix::StockGroups(omsi)));
        let mut d = ed.doc.take().unwrap();
        ed.history.undo(&mut d);
        assert!(d.traffic.groups.is_empty());
    }

    #[test]
    fn a_tour_without_a_depot_gets_one() {
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
        let _map = crate::doc::one_map();
        let mut doc = doc_with(&[(0, 0)]);
        doc.timetable.lines.push(omsi_timetable::Line { name: "1".into(), tours: vec![omsi_timetable::Tour { number: "1".into(), ..Default::default() }], ..Default::default() });
        let mut out = Vec::new();
        timetable_issues(&doc, &[PathBuf::new(), omsi], &mut out);
        let fix = out.iter().find(|i| i.kind == Kind::TourNoDepot).and_then(|i| i.fix.clone()).unwrap();
        assert_eq!(fix, Fix::TourDepot("1".into(), "1".into(), true));
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        assert!(apply(&mut ed, &fix));
        let d = ed.doc.as_ref().unwrap();
        let depot = d.traffic.depots().next().unwrap().name.clone();
        assert_eq!(d.timetable.lines[0].tours[0].ai_group, depot);
        let mut d = ed.doc.take().unwrap();
        ed.history.undo(&mut d);
        assert!(d.traffic.depots().next().is_none() && d.timetable.lines[0].tours[0].ai_group.is_empty());
    }

    #[test]
    fn a_map_with_another_maps_depot_file_gets_its_own() {
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
        let _map = crate::doc::one_map();
        let mut doc = doc_with(&[(0, 0)]);
        for (id, n) in [(1, "Am Markt"), (2, "Bahnhof")] {
            doc.timetable.stops.push(omsi_timetable::BusStopEntry { name: n.into(), group: 0, object_id: id, params: [0.0; 3] });
        }
        doc.timetable.trips.push(omsi_timetable::Trip { name: "a".into(), line: "3".into(), terminus: "Bahnhof".into(), stations: vec![1, 2], ..Default::default() });
        // (a new map's depot: OMSI 2's MAN buses with Grundorf's depot file)
        doc.traffic.groups.push(crate::traffic::stock_depot("Depot"));
        let mut out = Vec::new();
        depot_issues(&doc, &[PathBuf::new(), omsi], &mut out);
        let issue = out.iter().find(|i| i.kind == Kind::DepotFile).expect("Grundorf's depot file knows no Bahnhof");
        assert_eq!(issue.fix, Some(Fix::OwnDepot("Grundorf".into())));
        let fix = issue.fix.clone().unwrap();
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        assert!(apply(&mut ed, &fix));
        let d = ed.doc.as_ref().unwrap();
        let own = &d.traffic.depots[0];
        assert!(own.terminus_named("Bahnhof").is_some() && own.routes.len() == 1 && own.routes[0].code == "301");
        assert_eq!(d.traffic.groups[0].hof.as_deref(), Some(own.file.as_str()));
        let mut out = Vec::new();
        depot_issues(d, &[], &mut out);
        assert!(out.is_empty(), "{out:?}");
        let mut d = ed.doc.take().unwrap();
        ed.history.undo(&mut d);
        assert!(d.traffic.depots.is_empty() && d.traffic.groups[0].hof.as_deref() == Some("Grundorf"));
    }

    #[test]
    fn an_object_goes_over_to_the_tile_it_stands_on() {
        let _map = crate::doc::one_map();
        let mut doc = doc_with(&[(0, 0), (1, 0)]);
        let ts = tile_size();
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { id: 5, pos: [ts + 10.0, 20.0, 0.0], ..Default::default() });
        let mut ed = Editor::default();
        ed.doc = Some(doc);
        assert!(apply(&mut ed, &Fix::MoveToTile((0, 0), 5)));
        let d = ed.doc.as_ref().unwrap();
        assert!(d.object((0, 0), 5).is_none());
        let o = d.object((1, 0), 5).unwrap();
        assert!((o.pos[0] - 10.0).abs() < 1e-9 && (o.pos[1] - 20.0).abs() < 1e-9);
    }

    #[test]
    fn tours_and_traffic_groups_are_checked() {
        let mut d = doc_with(&[(0, 0)]);
        let l = d.timetable.add_line("7");
        let line = d.timetable.lines.iter_mut().find(|x| x.name == l).unwrap();
        line.tours.push(omsi_timetable::Tour { number: "1".into(), ai_group: "Nowhere".into(), ..Default::default() });
        d.traffic.veh_groups.push(("Cars".into(), 1));
        let mut out = Vec::new();
        timetable_issues(&d, &[], &mut out);
        assert!(out.iter().any(|i| i.kind == Kind::TourEmpty));
        assert!(out.iter().any(|i| i.kind == Kind::TourNoDepot && i.detail.contains("Nowhere")));
        assert!(out.iter().any(|i| i.kind == Kind::UnschedMissing && i.title == "Cars"));
    }

    #[test]
    fn a_tile_far_fuller_than_the_stock_ones_is_named() {
        let mut s = snap(Vec::new());
        let o = |id: i64, tile: TileKey| ObjRec { file: "tree.sco".into(), tile, id, world: DVec2::new(10.0, 10.0), rot: [0.0; 3], local: [10.0, 10.0], attached: false };
        s.objects = (0..1600).map(|i| o(i, (0, 0))).chain((0..700).map(|i| o(5000 + i, (1, 0)))).collect();
        s.tiles = [(0, 0), (1, 0)].into_iter().collect();
        let heavy: Vec<_> = run(s, Vec::new()).into_iter().filter(|i| i.kind == Kind::HeavyTile).collect();
        assert_eq!(heavy.len(), 1);
        assert!(heavy[0].title.contains("tile_0_0"));
    }

    #[test]
    fn lamps_on_a_tile_lit_from_none_are_named_once_per_tile() {
        let mut s = snap(Vec::new());
        s.kinds.insert("lamp.sco".into(), KindRec { cat: Category::Objects, half_width: 0.0, bus_stop: false, lanes: false, abs_height: false, lamp: true, light_head: false });
        let o = |id: i64, tile: TileKey| ObjRec { file: "Lamp.sco".into(), tile, id, world: DVec2::new(10.0, 10.0), rot: [0.0; 3], local: [10.0, 10.0], attached: false };
        s.objects = vec![o(1, (0, 0)), o(2, (0, 0)), o(3, (1, 0))];
        s.tiles = [(0, 0), (1, 0)].into_iter().collect();
        s.unlit = [(0, 0)].into_iter().collect();
        let issues = run(s, Vec::new());
        let dark: Vec<_> = issues.iter().filter(|i| i.kind == Kind::LampsDark).collect();
        assert_eq!(dark.len(), 1);
        assert_eq!(dark[0].fix, Some(Fix::LampLight((0, 0))));
        assert!(dark[0].detail.starts_with('2'));
    }

    #[test]
    fn a_head_unlinked_where_its_like_are_linked_is_named() {
        let mut s = snap(Vec::new());
        let k = |head| KindRec { cat: Category::Objects, half_width: 0.0, bus_stop: false, lanes: false, abs_height: false, lamp: false, light_head: head };
        s.kinds.insert("head.sco".into(), k(true));
        s.kinds.insert("barrier.sco".into(), k(true));
        let o = |id: i64, f: &str| ObjRec { file: f.into(), tile: (0, 0), id, world: DVec2::ZERO, rot: [0.0; 3], local: [0.0; 2], attached: false };
        s.objects = vec![o(1, "Head.sco"), o(2, "Head.sco"), o(3, "barrier.sco"), o(9, "crossing.sco")];
        s.object_ids = [1, 2, 3, 9].into_iter().collect();
        s.var_links = vec![((0, 0), 1, DVec2::ZERO, 9)];
        let heads: Vec<_> = run(s, Vec::new()).into_iter().filter(|i| i.kind == Kind::HeadUnlinked).map(|i| i.sel).collect();
        assert_eq!(heads, [Some(Sel::Object((0, 0), 2))]);
    }

    #[test]
    fn objects_linked_to_one_that_is_gone_are_named() {
        let mut s = snap(Vec::new());
        let o = |id: i64| ObjRec { file: "Ampel.sco".into(), tile: (0, 0), id, world: DVec2::new(10.0, 10.0), rot: [0.0; 3], local: [10.0, 10.0], attached: false };
        s.objects = vec![o(1), o(2), o(3)];
        s.object_ids = [1, 2, 3, 40].into_iter().collect();
        // (3 takes its state from 1, 2 from one gone; 40 is a chrono scenario's)
        s.var_links = vec![((0, 0), 3, DVec2::ZERO, 1), ((0, 0), 2, DVec2::ZERO, 9), ((0, 0), 1, DVec2::ZERO, 40)];
        let issues = run(s, Vec::new());
        let lost: Vec<_> = issues.iter().filter(|i| i.kind == Kind::LinkLost).collect();
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].fix, Some(Fix::DropVarParent((0, 0), 2)));
    }

    #[test]
    fn ends_on_one_another_are_found_once() {
        // two roads north, the second starting where the first ends
        let mut b = spline(2, 0.0, 0.0, 0, 0);
        b.ends[0].0.y = 10.0;
        b.ends[1].0.y = 20.0;
        let issues = run(snap(vec![spline(1, 0.0, 0.0, 0, 0), b]), Vec::new());
        let links: Vec<_> = issues.iter().filter(|i| i.kind == Kind::UnlinkedEnds).collect();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].fix, Some(Fix::Link(((0, 0), 1, true), ((0, 0), 2, false))));
    }

    #[test]
    fn ends_at_an_angle_are_left_alone() {
        let mut b = spline(2, 0.0, 90.0, 0, 0);
        b.ends[0].0.y = 10.0;
        let issues = run(snap(vec![spline(1, 0.0, 0.0, 0, 0), b]), Vec::new());
        assert!(!issues.iter().any(|i| i.kind == Kind::UnlinkedEnds));
    }

    #[test]
    fn links_to_nothing_duplicates_and_the_id_counter() {
        let issues = run(snap(vec![spline(1, 0.0, 0.0, 0, 77), spline(1, 50.0, 0.0, 0, 0), spline(300, 90.0, 0.0, 0, 0)]), Vec::new());
        assert!(issues.iter().any(|i| i.kind == Kind::BrokenLink && i.fix == Some(Fix::Unlink(((0, 0), 1, true)))));
        assert!(issues.iter().any(|i| i.kind == Kind::DuplicateId));
        assert!(issues.iter().any(|i| i.fix == Some(Fix::RaiseNextId(301))));
        assert!(issues.iter().any(|i| i.kind == Kind::NoEntry));
        // a missing spline file (no roots to find it in)
        assert!(issues.iter().any(|i| i.kind == Kind::MissingSpline));
    }
}
