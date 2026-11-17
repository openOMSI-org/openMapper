//! Getting started: the steps to a map that can be driven in service, each ticked by itself
//! as the map gets it, with a line of how-to and a button that shows where to do it.

use std::hash::{Hash, Hasher};

use egui::{RichText, Vec2};
use glam::DVec2;
use omsi_geometry::SplineCurve;

use crate::doc::{tile_origin, Document};
use crate::editor::{Editor, LogicMode, Tool};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{Category, Library};
use crate::theme::pal;

/// How near a road a stop must stand to be served (m), as the Lines window finds them.
const STOP_REACH: f64 = 15.0;

/// What a "Show me" button asks the application to do.
pub enum Go {
    Tool(Tool),
    Logic(LogicMode),
    Lines,
    Traffic,
    Save,
    Check,
    Play,
    LookAt(DVec2),
    /// Open the guide by itself for new maps (or not).
    Auto(bool),
}

/// What the map has, looked at again when it changes.
#[derive(Default, Clone)]
struct State {
    tiles: bool,
    road: bool,
    stops: usize,
    /// A stop that stands off every road (to fly to).
    stop_off_road: Option<DVec2>,
    stop_on_road: bool,
    start: bool,
    routed: bool,
    tour: bool,
    depot: bool,
}

#[derive(Default)]
pub struct Guide {
    pub open: bool,
    /// openOMSI was started on the map.
    pub driven: bool,
    key: u64,
    state: State,
}

impl Guide {
    /// A map was made: the guide opens unless that was switched off.
    pub fn map_made(&mut self, prefs: &crate::install::Prefs) {
        self.driven = false;
        if !prefs.guide_off {
            self.open = true;
        }
    }

    fn look(&mut self, doc: &Document, lib: &Library) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (k, t) in &doc.tiles {
            k.hash(&mut h);
            t.content_rev.hash(&mut h);
        }
        doc.global.entry_points.len().hash(&mut h);
        doc.global_rev.hash(&mut h);
        let key = h.finish();
        if key == self.key {
            return;
        }
        self.key = key;
        let mut s = State { tiles: !doc.tiles.is_empty(), start: !doc.global.entry_points.is_empty(), ..Default::default() };
        // the roads' centre lines, every few metres
        let mut road_pts: Vec<DVec2> = Vec::new();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for sp in t.tile.splines.iter().filter(|x| !x.deleted) {
                if lib.find(&sp.file).map(|a| a.category == Category::Roads).unwrap_or(false) {
                    s.road = true;
                    let c = SplineCurve::from_map(sp, org);
                    let n = (sp.length / 4.0).ceil().max(1.0) as usize;
                    road_pts.extend((0..=n).map(|i| c.point_at(sp.length * i as f64 / n as f64).truncate()));
                }
            }
        }
        for (k, t) in &doc.tiles {
            for o in &t.tile.objects {
                if !lib.find(&o.file).map(|a| a.is_bus_stop).unwrap_or(o.flag == 7) {
                    continue;
                }
                s.stops += 1;
                let p = doc.object_world(*k, o, false).truncate();
                // (a road's lane lies a few metres off its centre line)
                if road_pts.iter().any(|q| (*q - p).length() < STOP_REACH + 4.0) {
                    s.stop_on_road = true;
                } else if s.stop_off_road.is_none() {
                    s.stop_off_road = Some(p);
                }
            }
        }
        let tt = &doc.timetable;
        s.routed = tt.trips.iter().any(|t| t.stations.len() >= 2 && tt.tracks.iter().any(|x| crate::timetable::track_name(x).eq_ignore_ascii_case(&t.name)));
        let tours: Vec<&omsi_timetable::Tour> = tt.lines.iter().flat_map(|l| l.tours.iter()).filter(|t| !t.trips.is_empty()).collect();
        s.tour = !tours.is_empty();
        s.depot = tours.iter().any(|t| doc.traffic.depots().any(|d| d.name.trim().eq_ignore_ascii_case(t.ai_group.trim())));
        self.state = s;
    }

    /// `checked`: the map check ran and left no error unfixed.
    pub fn window(&mut self, ctx: &egui::Context, ed: &Editor, lib: &Library, prefs: &crate::install::Prefs, game_found: bool, checked: bool) -> Vec<Go> {
        let mut go = Vec::new();
        let Some(doc) = ed.doc.as_ref() else { return go };
        if !self.open {
            return go;
        }
        self.look(doc, lib);
        let s = self.state.clone();
        let saved = !doc.fresh && !doc.is_dirty();
        let steps: [(bool, &str, &str, Option<Go>); 10] = [
            (s.tiles, tr("Ground", "Земля"), tr("The map's tiles: 300 m squares. Add more beside them with the Tiles tool.", "Тайлы карты: квадраты по 300 м. Новые добавляются инструментом «Тайлы»."), Some(Go::Tool(Tool::Tiles))),
            (s.road, tr("A road", "Дорога"), tr("Roads tool: click where it starts, then where it ends. Click on a road's end to go on from it.", "Инструмент «Дороги»: клик в начале, затем в конце. Клик по концу дороги: продолжить её."), Some(Go::Tool(Tool::Roads))),
            (s.stop_on_road, tr("A bus stop by the road", "Остановка у дороги"), tr("Game logic > Bus stops: put one at the kerb, on the side the doors open. Name it on the right.", "Служебное > Остановки: у бордюра, со стороны дверей. Имя: справа."), Some(match s.stop_off_road.filter(|_| s.stops > 0 && !s.stop_on_road) {
                Some(p) => Go::LookAt(p),
                None => Go::Logic(LogicMode::Stops),
            })),
            (s.start, tr("A start point", "Точка старта"), tr("Game logic > Start points: where your bus appears, on a road, facing the way to drive.", "Служебное > Точки старта: где появится автобус: на дороге, лицом по ходу."), Some(Go::Logic(LogicMode::Start))),
            (s.routed, tr("A line with a trip", "Линия с рейсом"), tr("Lines window: New line, New trip, then click the stops in order. The way is found along the roads.", "Окно «Линии»: новая линия, новый рейс, затем кликайте остановки по порядку. Путь найдётся по дорогам."), Some(Go::Lines)),
            (s.tour, tr("A tour with departures", "Выход с отправлениями"), tr("Lines window > Tours and times: New tour, then Fill from … to … .", "Окно «Линии» > Выходы и время: новый выход, затем «Заполнить с … до …»."), Some(Go::Lines)),
            (s.depot, tr("Buses for the tours", "Автобусы для выходов"), tr("A depot the tours take their buses from (Tours: Add a depot, or Traffic and AI).", "Депо, из которого выходы берут автобусы (Выходы: «Добавить депо» или «Трафик и ИИ»)."), Some(Go::Traffic)),
            (checked, tr("Checked", "Проверено"), tr("Map > Check map: what would go wrong in the game, most of it put right with one click.", "Карта > Проверить карту: что пойдёт не так в игре; почти всё исправляется одним кликом."), Some(Go::Check)),
            (saved, tr("Saved", "Сохранено"), tr("Ctrl+S. Maps are saved into openOMSI's content folder.", "Ctrl+S. Карты сохраняются в папку контента openOMSI."), Some(Go::Save)),
            (self.driven, tr("A test drive", "Тест-драйв"), tr("F5 saves and starts openOMSI on the map: pick line 1 and drive.", "F5 сохраняет и запускает openOMSI на карте: выберите линию и езжайте."), game_found.then_some(Go::Play)),
        ];
        let done = steps.iter().filter(|x| x.0).count();
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Help, tr("Getting started", "С чего начать")))
            .id(egui::Id::new("guide window"))
            .open(&mut open)
            .default_pos([ctx.content_rect().right() - 640.0, 80.0])
            .default_width(340.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("{} {done} / {}", tr("A map you can drive in service:", "Карта, по которой можно ездить по расписанию:"), steps.len())).color(pal().text_strong));
                ui.add(egui::ProgressBar::new(done as f32 / steps.len() as f32).desired_height(6.0));
                ui.add_space(4.0);
                let mut first_open = true;
                for (ok, title, how, button) in steps {
                    ui.horizontal(|ui| {
                        let (icon, color) = if ok { (Icon::CheckCircle, pal().ok) } else { (Icon::RadioOff, if first_open { pal().warn } else { pal().text_dim }) };
                        ui.label(icon.text(18.0).color(color));
                        ui.vertical(|ui| {
                            ui.set_width(250.0);
                            ui.label(RichText::new(title).strong().color(if ok { pal().text_dim } else { pal().text_strong }));
                            if !ok && first_open {
                                ui.label(RichText::new(how).small().color(pal().text_dim));
                            }
                        });
                        if let Some(b) = button {
                            if !ok && ui.add(egui::Button::new(tr("Show me", "Показать")).min_size(Vec2::new(0.0, 22.0))).clicked() {
                                go.push(b);
                            }
                        }
                    });
                    if !ok {
                        first_open = false;
                    }
                }
                if s.stops > 0 && !s.stop_on_road {
                    ui.label(RichText::new(tr("Your stops stand away from the roads: a bus serves a stop within 15 m of its lane.", "Остановки стоят далеко от дорог: автобус обслуживает остановку в пределах 15 м от полосы.")).small().color(pal().warn));
                }
                // (a real place: most of it can come at once)
                if !s.road {
                    ui.label(RichText::new(tr("A real place? Put the map on the Earth (Map > Satellite and terrain), then Map > From OpenStreetMap brings its streets, stops, lines and trees.", "Реальное место? Привяжите карту к Земле («Карта > Спутник и рельеф»), затем «Карта > Из OpenStreetMap» привезёт её улицы, остановки, линии и деревья.")).small().color(pal().text_dim));
                }
                ui.separator();
                let auto = !prefs.guide_off;
                let mut a = auto;
                if ui.checkbox(&mut a, tr("Show this for new maps", "Показывать для новых карт")).changed() {
                    go.push(Go::Auto(a));
                }
            });
        self.open = open;
        go
    }
}
