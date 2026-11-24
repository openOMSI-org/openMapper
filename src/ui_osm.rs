//! From OpenStreetMap (Map menu): the streets of the map's place downloaded, shown by kind
//! with how much of each there is, and built with the road types chosen for them; then its
//! bus stops, its bus lines and its trees and woods.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

use egui::RichText;
use glam::DVec2;

use crate::doc::{tile_of, tile_origin, tile_size};
use crate::editor::Editor;
use crate::geo::Georef;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::osm::{self, Class};
use crate::theme::pal;

pub struct OsmUi {
    pub open: bool,
    rx: Option<Receiver<Result<Downloaded, String>>>,
    ways: Option<Vec<osm::Way>>,
    stops: Vec<osm::Stop>,
    lines: Vec<crate::osm_lines::Route>,
    green: crate::osm_green::Green,
    buildings: Vec<crate::osm_buildings::Building>,
    water: crate::osm_water::Water,
    /// Lakes and rivers dug this deep below their banks.
    with_water: bool,
    water_depth: f64,
    /// Buildings put with the models of this catalogue folder.
    with_buildings: bool,
    building_dirs: Vec<String>,
    /// Trees and woods planted with the plant mix, so far apart in woods.
    with_trees: bool,
    tree_spacing: f64,
    /// Lines and trips made of the routes (after the stops are put down).
    with_lines: bool,
    /// Stops put down too, with this stop object.
    with_stops: bool,
    stop_type: Option<String>,
    /// The roads made of them for the map as it is (its tiles' count when made).
    roads: Option<(usize, Vec<osm::Road>)>,
    status: Option<(String, bool)>,
    types: HashMap<Class, String>,
    /// The type one-way roads are built with (none: their class's).
    oneway_type: Option<String>,
    include: HashMap<Class, bool>,
    /// Asked for: build these (the app has the assets).
    pub build: Option<Build>,
    /// What the download makes for the map as it is (the tiles' count and the download's
    /// number when made): worked out once, not each frame.
    derived: Option<((usize, u64), std::rc::Rc<Derived>)>,
    downloads: u64,
}

/// The downloaded things within the map, in world coordinates.
#[derive(Default)]
struct Derived {
    stops: Vec<(DVec2, String)>,
    lines: Vec<(String, String, Vec<DVec2>)>,
    trees: Vec<DVec2>,
    rows: Vec<Vec<DVec2>>,
    woods: Vec<Vec<DVec2>>,
    holes: Vec<Vec<DVec2>>,
    /// The woods' area within the map (ha).
    ha: f64,
    buildings: Vec<(Vec<DVec2>, f64, bool)>,
    water: Vec<Vec<DVec2>>,
    islands: Vec<Vec<DVec2>>,
    /// Water areas reaching into the map.
    water_in: usize,
}

/// Roads with their types, then stops (where, name) with their object.
pub struct Build {
    pub roads: Vec<osm::Road>,
    pub types: Vec<(Class, String)>,
    pub oneway_type: Option<String>,
    pub stops: Vec<(DVec2, String)>,
    pub stop_type: Option<String>,
    /// Bus routes: number, name, their stops in order.
    pub lines: Vec<(String, String, Vec<DVec2>)>,
    /// Where trees go, and the plants they are.
    pub trees: Vec<DVec2>,
    pub mix: Vec<String>,
    /// Building outlines, and the models that may fill them.
    /// (outline, height, whether OSM says it)
    pub buildings: Vec<(Vec<DVec2>, f64, bool)>,
    pub pool: Vec<String>,
    /// Water areas, their islands, how deep.
    pub water: Option<(Vec<Vec<DVec2>>, Vec<Vec<DVec2>>, f64)>,
}

impl Default for OsmUi {
    fn default() -> Self {
        OsmUi {
            open: false,
            rx: None,
            ways: None,
            stops: Vec::new(),
            lines: Vec::new(),
            green: Default::default(),
            buildings: Vec::new(),
            water: Default::default(),
            with_water: false,
            water_depth: 2.0,
            with_buildings: false,
            building_dirs: Vec::new(),
            with_trees: false,
            tree_spacing: 8.0,
            with_lines: true,
            with_stops: true,
            stop_type: None,
            roads: None,
            status: None,
            types: HashMap::new(),
            oneway_type: None,
            include: [(Class::Main, true), (Class::Street, true), (Class::Service, false), (Class::Tram, false), (Class::Rail, false)].into_iter().collect(),
            build: None,
            derived: None,
            downloads: 0,
        }
    }
}

/// What a download brought: the roads and stops, the bus routes, the trees, and what of
/// it could not be had (the servers busy).
type Downloaded = (Vec<osm::Way>, Vec<osm::Stop>, Vec<crate::osm_lines::Route>, crate::osm_green::Green, Vec<crate::osm_buildings::Building>, crate::osm_water::Water, Vec<String>);

/// The models of these catalogue folders that may be buildings.
fn pool_of(lib: &Library, dirs: &[String]) -> Vec<String> {
    lib.assets.iter().filter(|a| a.kind == crate::library::Kind::Object && a.category == crate::library::Category::Objects && dirs.iter().any(|d| a.dir.eq_ignore_ascii_case(d))).map(|a| a.rel.clone()).collect()
}

/// The map's tiles' box (world).
fn map_box(ed: &Editor) -> (DVec2, DVec2) {
    let Some(doc) = ed.doc.as_ref() else { return (DVec2::ZERO, DVec2::ZERO) };
    doc.tiles.keys().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(a, b), k| (a.min(tile_origin(*k)), b.max(tile_origin(*k) + DVec2::splat(tile_size()))))
}

/// The map's tiles as a box of latitudes and longitudes (south, west, north, east).
fn bbox(ed: &Editor, g: &Georef) -> Option<[f64; 4]> {
    let doc = ed.doc.as_ref()?;
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for k in doc.tiles.keys() {
        let o = tile_origin(*k);
        for c in [o, o + DVec2::new(tile_size(), 0.0), o + DVec2::new(0.0, tile_size()), o + DVec2::splat(tile_size())] {
            let (la, lo) = g.lat_lon(c);
            b = [b[0].min(la), b[1].min(lo), b[2].max(la), b[3].max(lo)];
        }
    }
    (b[0] < b[2]).then_some(b)
}

impl OsmUi {
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, georef: Option<Georef>) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(Ok((w, st, li, gr, bu, wa, missed))) => {
                    self.green = gr;
                    self.water = wa;
                    self.buildings = bu;
                    let mut text = format!("{} {} · {} {} · {} {}", w.len(), tr("ways", "путей"), st.len(), tr("stops", "остановок"), li.len(), tr("bus routes downloaded", "маршрутов загружено"));
                    if !missed.is_empty() {
                        text += &format!(" ({} {})", tr("not had:", "не загружено:"), missed.join(", "));
                    }
                    self.status = Some((text, !missed.is_empty()));
                    self.ways = Some(w);
                    self.stops = st;
                    self.lines = li;
                    self.downloads += 1;
                    self.roads = None;
                    self.rx = None;
                }
                Ok(Err(e)) => {
                    self.status = Some((e, true));
                    self.rx = None;
                }
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }
        if !self.open {
            return;
        }
        let tiles = ed.doc.as_ref().map(|d| d.tiles.len()).unwrap_or(0);
        if let (Some(w), Some(g), Some(doc)) = (&self.ways, &georef, &ed.doc) {
            if self.roads.as_ref().map(|r| r.0 != tiles).unwrap_or(true) {
                let made = osm::roads(w, |la, lo| g.world(la, lo), |p| doc.tiles.contains_key(&tile_of(p)));
                self.roads = Some((tiles, made));
            }
        }
        if let (Some(g), Some(doc)) = (&georef, &ed.doc) {
            let key = (tiles, self.downloads);
            if self.derived.as_ref().map(|d| d.0 != key).unwrap_or(true) {
                self.derived = Some((key, std::rc::Rc::new(self.derive(g, doc, ed))));
            }
        }
        let derived = self.derived.as_ref().map(|d| d.1.clone()).unwrap_or_default();
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Public, tr("From OpenStreetMap", "Из OpenStreetMap"))).id(egui::Id::new("osm roads")).open(&mut open).default_pos([360.0, 110.0]).default_size([390.0, 700.0]).resizable(true).vscroll(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("The streets of the map's place built as roads with junctions where they meet, its bus stops, lines, trees and woods. Best on tiles with no roads yet: roads lying on ones the map has are left out.", "Улицы места карты, построенные дорогами с перекрёстками, её остановки, линии, деревья и леса. Лучше на тайлах без дорог: дороги поверх уже имеющихся пропускаются.")).small().color(pal().text_dim));
            ui.add_space(4.0);
            let Some(g) = georef else {
                ui.label(RichText::new(tr("The map does not lie on the Earth yet: place it in Satellite and terrain (Map menu, I) first.", "Карта ещё не привязана к Земле: сначала разместите её в «Спутник и рельеф» (меню Карта, I).")).color(pal().warn));
                return;
            };
            ui.label(RichText::new(tr("Load the terrain heights first (Satellite and terrain): the roads follow the ground as it is when they are built.", "Сначала загрузите высоты рельефа («Спутник и рельеф»): дороги ложатся на землю такой, какая она при постройке.")).small().color(pal().text_dim));
            let busy = self.rx.is_some();
            ui.horizontal(|ui| {
                let r = ui.add_enabled(!busy && tiles > 0, egui::Button::new(icons::labelled(Icon::Import, tr("Download for the map's tiles", "Загрузить для тайлов карты"))));
                if r.clicked() {
                    if let Some(b) = bbox(ed, &g) {
                        let (tx, rx) = channel();
                        let (q, ql, qg, qb, qw) = (osm::query(b), crate::osm_lines::query(b), crate::osm_green::query(b), crate::osm_buildings::query(b), crate::osm_water::query(b));
                        std::thread::spawn(move || {
                            // (the routes and the trees are questions of their own: without them
                            // the roads still come)
                            let mut missed = Vec::new();
                            let lines = crate::geo::fetch::overpass(&ql).and_then(|body| crate::osm_lines::parse(&body)).unwrap_or_else(|_| {
                                missed.push(tr("bus routes", "маршруты").to_string());
                                Vec::new()
                            });
                            let green = crate::geo::fetch::overpass(&qg).and_then(|body| crate::osm_green::parse(&body)).unwrap_or_else(|_| {
                                missed.push(tr("trees", "деревья").to_string());
                                Default::default()
                            });
                            let buildings = crate::geo::fetch::overpass(&qb).and_then(|body| crate::osm_buildings::parse(&body)).unwrap_or_else(|_| {
                                missed.push(tr("buildings", "здания").to_string());
                                Vec::new()
                            });
                            let water = crate::geo::fetch::overpass(&qw).and_then(|body| crate::osm_water::parse(&body)).unwrap_or_else(|_| {
                                missed.push(tr("water", "вода").to_string());
                                Default::default()
                            });
                            let _ = tx.send(crate::geo::fetch::overpass(&q).and_then(|body| Ok((osm::parse(&body)?, osm::parse_stops(&body)?, lines, green, buildings, water, missed))));
                        });
                        self.rx = Some(rx);
                        self.status = Some((tr("Downloading…", "Загрузка…").into(), false));
                    }
                }
                if busy {
                    ui.spinner();
                }
            });
            if let Some((t, err)) = &self.status {
                ui.label(RichText::new(t).small().color(if *err { pal().error } else { pal().text_dim }));
            }
            let Some((_, roads)) = &self.roads else { return };
            ui.add_space(6.0);
            egui::Grid::new("osm classes").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                for c in Class::ALL {
                    let of: Vec<&osm::Road> = roads.iter().filter(|r| r.class == c).collect();
                    let km: f64 = of.iter().map(|r| r.points.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f64>()).sum::<f64>() / 1000.0;
                    let on = self.include.entry(c).or_insert(false);
                    ui.vertical(|ui| {
                        crate::ui::check(ui, on, c.label()).on_hover_text(c.hint());
                        ui.label(RichText::new(format!("{} · {:.1} km", of.len(), km.max(0.0))).small().color(pal().text_dim));
                    });
                    let now = self.types.get(&c).cloned();
                    let name = now.as_ref().map(|t| lib.find(t).map(|a| a.name.clone()).unwrap_or_else(|| t.clone())).unwrap_or_else(|| tr("no type chosen", "тип не выбран").into());
                    ui.label(RichText::new(name).color(if now.is_some() { pal().text_strong } else { pal().text_dim }));
                    let pick = ed.road_type.clone();
                    let r = ui.add_enabled(pick.is_some(), egui::Button::new(icons::labelled(Icon::Dropper, tr("Use the chosen", "Взять выбранный"))).small());
                    let r = r.on_hover_text(tr("The road type chosen in the Roads tool's catalogue", "Тип дороги, выбранный в каталоге инструмента «Дороги»"));
                    if r.clicked() {
                        if let Some(p) = pick {
                            self.types.insert(c, p);
                        }
                    }
                    ui.end_row();
                }
                // one-way roads: the carriageways of a dual road, one-way streets
                let n = roads.iter().filter(|r| r.oneway && self.include.get(&r.class).copied().unwrap_or(false)).count();
                ui.vertical(|ui| {
                    ui.label(tr("One-way roads", "Односторонние")).on_hover_text(tr("Each carriageway of a dual road and one-way streets: with a one-way road type they lie side by side as they should; with none chosen they get their kind's", "Каждая половина дороги с разделительной полосой и улицы с односторонним движением: с односторонним типом они лежат рядом, как надо; без выбора — тип своего вида"));
                    ui.label(RichText::new(format!("{n}")).small().color(pal().text_dim));
                });
                let name = self.oneway_type.as_ref().map(|t| lib.find(t).map(|a| a.name.clone()).unwrap_or_else(|| t.clone())).unwrap_or_else(|| tr("as their kind", "как у их вида").into());
                ui.label(RichText::new(name).color(if self.oneway_type.is_some() { pal().text_strong } else { pal().text_dim }));
                ui.horizontal(|ui| {
                    let pick = ed.road_type.clone();
                    if ui.add_enabled(pick.is_some(), egui::Button::new(icons::labelled(Icon::Dropper, tr("Use the chosen", "Взять выбранный"))).small()).clicked() {
                        self.oneway_type = pick;
                    }
                    if self.oneway_type.is_some() && icons::small(ui, Icon::Close, tr("None", "Нет")).clicked() {
                        self.oneway_type = None;
                    }
                });
                ui.end_row();
            });
            // the stops, with the stop object chosen in the catalogue
            let stops_in = &derived.stops;
            ui.horizontal(|ui| {
                crate::ui::check(ui, &mut self.with_stops, tr("Bus stops", "Остановки"));
                ui.label(RichText::new(format!("{}", stops_in.len())).small().color(pal().text_dim));
                let name = self.stop_type.as_ref().map(|t| lib.find(t).map(|a| a.name.clone()).unwrap_or_else(|| t.clone())).unwrap_or_else(|| tr("no stop object", "объект не выбран").into());
                ui.label(RichText::new(name).color(if self.stop_type.is_some() { pal().text_strong } else { pal().text_dim }));
                let pick = ed.object_type.clone().filter(|t| lib.find(t).map(|a| a.is_bus_stop).unwrap_or(false));
                let r = ui.add_enabled(pick.is_some(), egui::Button::new(icons::labelled(Icon::Dropper, tr("Use the chosen", "Взять выбранный"))).small());
                if r.on_hover_text(tr("The stop chosen in Game logic > Bus stops", "Остановка, выбранная в «Игровая логика > Остановки»")).clicked() {
                    self.stop_type = pick;
                }
            });
            // the bus routes through the map's stops: lines and trips
            let lines_in = &derived.lines;
            ui.horizontal(|ui| {
                crate::ui::check(ui, &mut self.with_lines, tr("Bus lines", "Автобусные линии")).on_hover_text(tr("Each bus route through two or more of the map's stops a trip of its line, the way found along the roads; a tour from 06:00 to 22:00 for each new line", "Каждый маршрут через две или больше остановок карты: рейс своей линии, путь по дорогам находится сам; для каждой новой линии выход с 06:00 до 22:00"));
                let mut numbers: Vec<&str> = lines_in.iter().map(|l| l.0.as_str()).collect();
                numbers.sort();
                numbers.dedup();
                let shown: Vec<&str> = numbers.iter().take(8).copied().collect();
                ui.label(RichText::new(format!("{} {}{}", lines_in.len(), shown.join(", "), if numbers.len() > 8 { "…" } else { "" })).small().color(pal().text_dim));
            });
            // trees and woods, with the plant mix (or the tree chosen)
            let (trees, rows, woods, ha) = (&derived.trees, &derived.rows, &derived.woods, derived.ha);
            let mix: Vec<String> = if !ed.plant_mix.is_empty() { ed.plant_mix.clone() } else { ed.object_type.iter().filter(|t| lib.find(t).map(|a| a.category == crate::library::Category::Trees).unwrap_or(false)).cloned().collect() };
            ui.horizontal(|ui| {
                crate::ui::check(ui, &mut self.with_trees, tr("Trees and woods", "Деревья и леса")).on_hover_text(tr("Single trees, rows of them and woods, planted with the plant mix (Objects tool > Plant brush) or the tree chosen; none on a road", "Отдельные деревья, ряды и леса, засаженные смесью растений (инструмент «Объекты» > Кисть растений) или выбранным деревом; не на дорогах"));
                ui.label(RichText::new(format!("{} · {} · {:.1} ha", trees.len(), rows.len(), ha)).small().color(pal().text_dim));
            });
            if self.with_trees {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if mix.is_empty() { tr("no plants chosen", "растения не выбраны").to_string() } else { format!("{} {}", mix.len(), tr("kinds of plants", "видов растений")) }).color(if mix.is_empty() { pal().text_dim } else { pal().text_strong }));
                    ui.label(tr("apart in woods", "шаг в лесу"));
                    ui.add(egui::DragValue::new(&mut self.tree_spacing).range(3.0..=40.0).speed(0.2).suffix(" m"));
                    // (a wood of a hectare is some 10000 / spacing² trees)
                    ui.label(RichText::new(format!("≈ {}", (ha * 10_000.0 / (self.tree_spacing * self.tree_spacing)) as usize + trees.len())).small().color(pal().text_dim));
                });
            }
            // buildings, with the models of a catalogue folder
            let outlines = &derived.buildings;
            let pool = pool_of(lib, &self.building_dirs);
            ui.horizontal(|ui| {
                crate::ui::check(ui, &mut self.with_buildings, tr("Buildings", "Здания")).on_hover_text(tr("Each outline filled with the model of the folder chosen whose ground plan comes nearest, its front to the nearest road; an outline no model fits is left out", "Каждый контур заполняется моделью выбранной папки с самым похожим планом, фасадом к ближайшей дороге; контуры, которым ничто не подходит, пропускаются"));
                ui.label(RichText::new(format!("{}", outlines.len())).small().color(pal().text_dim));
                let name = if self.building_dirs.is_empty() { tr("no folder", "папка не выбрана").to_string() } else { format!("{} ({})", self.building_dirs.join(", "), pool.len()) };
                ui.label(RichText::new(name).color(if self.building_dirs.is_empty() { pal().text_dim } else { pal().text_strong }));
                let pick = ed.object_type.as_ref().and_then(|t| lib.find(t)).filter(|a| a.category == crate::library::Category::Objects).map(|a| a.dir.clone());
                let can = pick.as_ref().map(|p| !self.building_dirs.contains(p)).unwrap_or(false);
                let r = ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::Add, tr("its folder", "его папка"))).small());
                if r.on_hover_text(tr("Add the folder of the object chosen in the catalogue (Buildings_MC, …)", "Добавить папку объекта, выбранного в каталоге (Buildings_MC, …)")).clicked() {
                    self.building_dirs.extend(pick);
                }
                if !self.building_dirs.is_empty() && icons::small(ui, Icon::Close, tr("None", "Нет")).clicked() {
                    self.building_dirs.clear();
                }
            });
            // lakes and rivers
            let areas_in = derived.water_in;
            ui.horizontal(|ui| {
                crate::ui::check(ui, &mut self.with_water, tr("Water", "Вода")).on_hover_text(tr("Lakes and rivers: the ground inside them lowered below the banks, and the tiles they lie on given water a little under the banks' height (OMSI's water is one level for a tile: low ground elsewhere on it is flooded too)", "Озёра и реки: земля внутри опускается ниже берегов, тайлам ставится вода чуть ниже берегов (вода OMSI одна на тайл: низкие места на нём тоже затопит)"));
                ui.label(RichText::new(format!("{areas_in}")).small().color(pal().text_dim));
                if self.with_water {
                    ui.label(tr("deep", "глубина"));
                    ui.add(egui::DragValue::new(&mut self.water_depth).range(0.5..=10.0).speed(0.1).suffix(" m"));
                }
            });
            ui.add_space(4.0);
            let wanted: Vec<(Class, String)> = Class::ALL.iter().filter(|c| self.include.get(c).copied().unwrap_or(false)).filter_map(|c| self.types.get(c).map(|t| (*c, t.clone()))).collect();
            // what is still to choose, each said by name
            let mut wants: Vec<String> = Class::ALL.iter().filter(|c| self.include.get(c).copied().unwrap_or(false) && !self.types.contains_key(c)).map(|c| format!("{}: {}", c.label(), tr("a road type (Roads tool), then \"Use the chosen\"", "тип дороги (инструмент «Дороги»), затем «Взять выбранный»"))).collect();
            if self.with_stops && !stops_in.is_empty() && self.stop_type.is_none() {
                wants.push(format!("{}: {}", tr("Bus stops", "Остановки"), tr("a stop (Game logic > Bus stops), then \"Use the chosen\"", "остановку («Игровая логика > Остановки»), затем «Взять выбранный»")));
            }
            if self.with_trees && mix.is_empty() {
                wants.push(format!("{}: {}", tr("Trees and woods", "Деревья и леса"), tr("a tree in the Objects catalogue (or a plant mix)", "дерево в каталоге объектов (или смесь растений)")));
            }
            if self.with_buildings && !outlines.is_empty() && pool.is_empty() {
                wants.push(format!("{}: {}", tr("Buildings", "Здания"), tr("a building in the catalogue, then \"+ its folder\"", "здание в каталоге, затем «+ его папка»")));
            }
            let missing = !wants.is_empty();
            let count = roads.iter().filter(|r| wanted.iter().any(|w| w.0 == r.class)).count();
            if missing {
                ui.label(RichText::new(tr("To build, choose (or untick):", "Чтобы построить, выберите (или снимите галочку):")).small().color(pal().warn));
                for w in &wants {
                    ui.label(RichText::new(format!("• {w}")).small().color(pal().warn));
                }
            }
            ui.label(RichText::new(tr("A one-way street is built the way it runs in OpenStreetMap: give it a one-way road type", "Улица с односторонним движением строится в направлении OpenStreetMap: дайте ей односторонний тип дороги")).small().color(pal().text_dim));
            let stops_n = if self.with_stops { stops_in.len() } else { 0 };
            let r = ui.add_enabled(!missing && (count + stops_n > 0 || self.with_trees || self.with_buildings || self.with_water), egui::Button::new(icons::labelled(Icon::Construction, &format!("{} ({count} · {stops_n})", tr("Build", "Построить")))));
            if r.clicked() {
                let list: Vec<osm::Road> = roads.iter().filter(|r| wanted.iter().any(|w| w.0 == r.class)).cloned().collect();
                let mut rng = crate::osm_green::shaker();
                let spots = if self.with_trees { crate::osm_green::spots(trees, rows, woods, &derived.holes, map_box(ed), self.tree_spacing, &mut rng) } else { Vec::new() };
                self.build = Some(Build { roads: list, types: wanted, stops: if self.with_stops { stops_in.clone() } else { Vec::new() }, stop_type: self.stop_type.clone(), oneway_type: self.oneway_type.clone(), lines: if self.with_lines { lines_in.clone() } else { Vec::new() }, trees: spots, mix, buildings: if self.with_buildings { outlines.clone() } else { Vec::new() }, pool, water: self.with_water.then(|| (derived.water.clone(), derived.islands.clone(), self.water_depth)) });
            }
            ui.add_space(4.0);
            ui.hyperlink_to(RichText::new("© OpenStreetMap contributors (ODbL)").small(), "https://www.openstreetmap.org/copyright");
        });
        self.open = open;
    }

    /// The downloaded things within the map, in world coordinates.
    fn derive(&self, g: &Georef, doc: &crate::doc::Document, ed: &Editor) -> Derived {
        let inside = |p: &DVec2| doc.tiles.contains_key(&tile_of(*p));
        let w = |v: &Vec<(f64, f64)>| v.iter().map(|p| g.world(p.0, p.1)).collect::<Vec<DVec2>>();
        let woods: Vec<Vec<DVec2>> = self.green.woods.iter().map(w).collect();
        let holes: Vec<Vec<DVec2>> = self.green.holes.iter().map(w).collect();
        // (the woods' area within the map, from a coarse count of where they are)
        let mut none = || 0.5;
        let ha = crate::osm_green::spots(&[], &[], &woods, &holes, map_box(ed), 10.0, &mut none).iter().filter(|p| inside(p)).count() as f64 * 100.0 / 10_000.0;
        let water: Vec<Vec<DVec2>> = self.water.areas.iter().map(w).collect();
        Derived {
            stops: self.stops.iter().map(|st| (g.world(st.at.0, st.at.1), st.name.clone())).filter(|(p, _)| inside(p)).collect(),
            lines: self.lines.iter().map(|r| (r.number.clone(), r.name.clone(), w(&r.stops))).filter(|r| r.2.iter().filter(|p| inside(p)).count() >= 2).collect(),
            trees: self.green.trees.iter().map(|p| g.world(p.0, p.1)).filter(inside).collect(),
            rows: self.green.rows.iter().map(w).collect(),
            woods,
            holes,
            ha,
            buildings: self.buildings.iter().map(|b| (w(&b.outline), b.height, b.known)).filter(|(p, _, _)| inside(&(p.iter().copied().sum::<DVec2>() / p.len().max(1) as f64))).collect(),
            water_in: water.iter().filter(|a| a.iter().any(inside)).count(),
            islands: self.water.islands.iter().map(w).collect(),
            water,
        }
    }

    /// OPENMAPPER_OSM=open, or <main type>[;<street type>[;<stop object>[;<one-way type>[;<tree>[;<track type>[;<building folders, by commas>[;<water depth>]]]]]]]: downloaded at once and built (checks
    /// without a person at the screen).
    pub fn scripted(&mut self, ed: &Editor, lib: &Library, georef: Option<Georef>, arg: &str) {
        let Some(g) = georef else { return };
        let Some(b) = bbox(ed, &g) else { return };
        // (`open`: the window with what was downloaded, nothing built)
        if arg == "open" {
            let body = crate::geo::fetch::overpass(&osm::query(b));
            self.ways = body.as_ref().ok().and_then(|b| osm::parse(b).ok());
            self.stops = body.as_ref().ok().and_then(|b| osm::parse_stops(b).ok()).unwrap_or_default();
            self.lines = crate::geo::fetch::overpass(&crate::osm_lines::query(b)).and_then(|b| crate::osm_lines::parse(&b)).unwrap_or_default();
            self.green = crate::geo::fetch::overpass(&crate::osm_green::query(b)).and_then(|b| crate::osm_green::parse(&b)).unwrap_or_default();
            self.water = crate::geo::fetch::overpass(&crate::osm_water::query(b)).and_then(|b| crate::osm_water::parse(&b)).unwrap_or_default();
            self.buildings = crate::geo::fetch::overpass(&crate::osm_buildings::query(b)).and_then(|b| crate::osm_buildings::parse(&b)).unwrap_or_default();
            self.downloads += 1;
            self.open = true;
            return;
        }
        let mut types = arg.split(';').map(str::trim);
        let main = types.next().unwrap_or_default().to_string();
        let street = types.next().map(str::to_string).unwrap_or_else(|| main.clone());
        let stop = types.next().map(str::to_string).filter(|t| !t.is_empty());
        let oneway = types.next().map(str::to_string).filter(|t| !t.is_empty());
        let tree = types.next().map(str::to_string).filter(|t| !t.is_empty());
        let rail = types.next().map(str::to_string).filter(|t| !t.is_empty());
        let building_dir = types.next().map(str::to_string).filter(|t| !t.is_empty());
        let depth = types.next().and_then(|t| t.parse::<f64>().ok());
        if let Some(r) = &rail {
            self.types.insert(Class::Tram, r.clone());
            self.types.insert(Class::Rail, r.clone());
        }
        self.types.insert(Class::Main, main.clone());
        self.types.insert(Class::Street, street);
        let body = match crate::geo::fetch::overpass(&osm::query(b)) {
            Ok(b) => b,
            Err(e) => return log::warn!("osm: {e}"),
        };
        match osm::parse(&body) {
            Ok(w) => {
                let Some(doc) = ed.doc.as_ref() else { return };
                let roads = osm::roads(&w, |la, lo| g.world(la, lo), |p| doc.tiles.contains_key(&tile_of(p)));
                log::info!("osm: {} ways, {} roads", w.len(), roads.len());
                let wanted: Vec<(Class, String)> = [Class::Main, Class::Street, Class::Tram, Class::Rail].iter().filter_map(|c| self.types.get(c).map(|t| (*c, t.clone()))).collect();
                let stops: Vec<(DVec2, String)> = osm::parse_stops(&body).unwrap_or_default().into_iter().map(|st| (g.world(st.at.0, st.at.1), st.name)).collect();
                let trees = match &tree {
                    Some(_) => {
                        let gr = crate::geo::fetch::overpass(&crate::osm_green::query(b)).and_then(|body| crate::osm_green::parse(&body)).unwrap_or_else(|e| {
                            log::warn!("osm trees: {e}");
                            Default::default()
                        });
                        let w = |v: &Vec<(f64, f64)>| v.iter().map(|p| g.world(p.0, p.1)).collect::<Vec<DVec2>>();
                        let mut rng = crate::osm_green::shaker();
                        crate::osm_green::spots(&gr.trees.iter().map(|p| g.world(p.0, p.1)).collect::<Vec<_>>(), &gr.rows.iter().map(w).collect::<Vec<_>>(), &gr.woods.iter().map(w).collect::<Vec<_>>(), &gr.holes.iter().map(w).collect::<Vec<_>>(), map_box(ed), 8.0, &mut rng)
                    }
                    None => Vec::new(),
                };
                let (buildings, pool) = match &building_dir {
                    Some(d) => {
                        let dirs: Vec<String> = d.split(',').map(|x| x.trim().to_string()).collect();
                        let o = crate::geo::fetch::overpass(&crate::osm_buildings::query(b)).and_then(|body| crate::osm_buildings::parse(&body)).unwrap_or_default();
                        let pool = pool_of(lib, &dirs);
                        (o.iter().map(|b| (b.outline.iter().map(|p| g.world(p.0, p.1)).collect(), b.height, b.known)).collect(), pool)
                    }
                    None => (Vec::new(), Vec::new()),
                };
                let water = depth.map(|d| {
                    let wa = crate::geo::fetch::overpass(&crate::osm_water::query(b)).and_then(|body| crate::osm_water::parse(&body)).unwrap_or_default();
                    let w = |v: &Vec<(f64, f64)>| v.iter().map(|p| g.world(p.0, p.1)).collect::<Vec<DVec2>>();
                    (wa.areas.iter().map(w).collect(), wa.islands.iter().map(w).collect(), d)
                });
                let lines = crate::geo::fetch::overpass(&crate::osm_lines::query(b)).and_then(|body| crate::osm_lines::parse(&body)).unwrap_or_default();
                let lines = lines.into_iter().map(|r| (r.number, r.name, r.stops.iter().map(|p| g.world(p.0, p.1)).collect())).collect();
                self.build = Some(Build { roads: roads.into_iter().filter(|r| wanted.iter().any(|w| w.0 == r.class)).collect(), types: wanted, stops, stop_type: stop.clone(), oneway_type: oneway.clone(), lines, trees, mix: tree.iter().cloned().collect(), buildings, pool, water });
                self.ways = Some(w);
            }
            Err(e) => log::warn!("osm: {e}"),
        }
    }
}
