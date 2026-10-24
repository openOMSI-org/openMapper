//! Search for a command (`Ctrl+K`): every menu command, tool and window by its name or a
//! word about it, in either language, and where a feature lives in a panel (a road's
//! height profile, a junction's lights) a note that says how to get to it. Typing filters,
//! the arrows choose, `Enter` runs it.

use egui::{Color32, Key, Modifiers, RichText, Sense, Vec2};

use crate::editor::{Editor, LogicMode, Tool};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::ui::{Action, UiState, TEXT_DIM};

type Run = fn(&mut UiState, &mut Editor, &mut Vec<Action>);

struct Command {
    icon: Icon,
    name: &'static str,
    /// Other words it is found by (both languages).
    words: &'static str,
    key: &'static str,
    run: Run,
}

macro_rules! cmd {
    ($icon:expr_2021, $en:literal, $ru:literal, $words:literal, $key:literal, $run:expr_2021) => {
        Command { icon: $icon, name: tr($en, $ru), words: concat!($en, " ", $ru, " ", $words), key: $key, run: $run }
    };
}

fn commands() -> Vec<Command> {
    vec![
        // File
        cmd!(Icon::NewFile, "New map…", "Новая карта…", "create make start", "", |st, _, _| st.new_map = Some(Default::default())),
        cmd!(Icon::Open, "Open map…", "Открыть карту…", "load", "", |_, _, a| a.push(Action::OpenDialog)),
        cmd!(Icon::Save, "Save", "Сохранить", "write", "Ctrl+S", |_, _, a| a.push(Action::Save)),
        cmd!(Icon::Copy, "Export map for sharing…", "Экспорт карты для обмена…", "zip share publish upload экспорт поделиться", "", |_, _, a| a.push(Action::Export)),
        cmd!(Icon::Map, "Export a plan of the map…", "Экспорт плана карты…", "svg picture print network scheme картинка печать схема", "", |_, _, a| a.push(Action::ExportPlan)),
        cmd!(Icon::Import, "Import assets…", "Импорт ассетов…", "model obj fbx gltf blender mod zip модель мод", "", |_, _, a| a.push(Action::Import)),
        cmd!(Icon::History, "Earlier versions…", "Прошлые версии…", "backup restore бэкап восстановить", "", |st, _, _| st.versions_open = true),
        // Edit
        cmd!(Icon::Undo, "Undo", "Отменить", "back", "Ctrl+Z", |_, _, a| a.push(Action::Undo)),
        cmd!(Icon::Redo, "Redo", "Повторить", "", "Ctrl+Shift+Z", |_, _, a| a.push(Action::Redo)),
        cmd!(Icon::Copy, "Copy", "Копировать", "", "Ctrl+C", |_, _, a| a.push(Action::Copy)),
        cmd!(Icon::Paste, "Paste", "Вставить", "", "Ctrl+V", |_, _, a| a.push(Action::Paste)),
        cmd!(Icon::Copy, "Duplicate", "Дублировать", "clone", "Ctrl+D", |_, _, a| a.push(Action::Duplicate)),
        cmd!(Icon::SelectAll, "Select all", "Выбрать всё", "", "Ctrl+A", |_, _, a| a.push(Action::SelectAll)),
        cmd!(Icon::History, "Edit history…", "История правок…", "undo steps шаги", "", |st, _, _| st.history_open = true),
        cmd!(Icon::Objects, "Groups…", "Группы…", "prefab template шаблон", "", |st, _, _| st.groups.open = true),
        cmd!(Icon::Search, "Find in map…", "Поиск по карте…", "search replace stop найти заменить остановка", "Ctrl+F", |st, _, _| st.find.show()),
        cmd!(Icon::Junction, "Make the junctions again", "Перестроить перекрёстки", "junction plate rebuild asphalt texture перекрёсток плита асфальт текстура", "", |_, ed, _| {
            let n = ed.remake_all_junctions();
            ed.toast(format!("{} {n}", tr("Junctions made again:", "Перекрёстков перестроено:")));
        }),
        cmd!(Icon::Layers, "Objects on the map by folder", "Объекты карты по папкам", "outliner hierarchy list tree folders иерархия список дерево папки", "", |st, ed, _| {
            ed.set_tool(Tool::Select);
            st.outliner.open = true;
        }),
        // Tools
        cmd!(Icon::Select, "Select", "Выбор", "tool move инструмент", "1", |_, ed, _| ed.set_tool(Tool::Select)),
        cmd!(Icon::Road, "Roads", "Дороги", "tool draw street track rail spline инструмент рисовать улица рельсы сплайн", "2", |_, ed, _| ed.set_tool(Tool::Roads)),
        cmd!(Icon::Objects, "Objects", "Объекты", "tool place building house инструмент поставить здание дом", "3", |_, ed, _| ed.set_tool(Tool::Objects)),
        cmd!(Icon::Sensors, "Game logic", "Игровая логика", "tool инструмент", "4", |_, ed, _| ed.set_tool(Tool::Logic)),
        cmd!(Icon::Terrain, "Terrain", "Рельеф", "tool raise lower smooth flatten hill инструмент поднять опустить сгладить холм", "5", |_, ed, _| ed.set_tool(Tool::Terrain)),
        cmd!(Icon::Paint, "Paint", "Покраска", "tool ground texture инструмент текстура земли", "6", |_, ed, _| ed.set_tool(Tool::Paint)),
        cmd!(Icon::Tiles, "Tiles", "Тайлы", "tool add remove grow инструмент добавить удалить", "7", |_, ed, _| ed.set_tool(Tool::Tiles)),
        cmd!(Icon::Road, "Road types and junctions…", "Типы дорог и перекрёстки…", "road maker lanes width полосы ширина", "", |_, _, a| a.push(Action::RoadTypes)),
        cmd!(Icon::Layers, "PBR maps for a picture…", "PBR-карты для картинки…", "normal roughness нормали", "", |_, _, a| a.push(Action::PbrMaker)),
        cmd!(Icon::Road, "Knife: cut roads", "Нож: разрезать дороги", "cut split knife road spline разрезать разделить дорогу сплайн", "K", |_, ed, _| ed.toggle_cutting()),
        cmd!(Icon::Flag, "Ruler", "Линейка", "measure distance gradient измерить расстояние уклон", "U", |_, ed, _| ed.toggle_ruler()),
        // Map
        cmd!(Icon::Settings, "Map settings…", "Настройки карты…", "money tickets depot years seasons деньги билеты депо годы", "", |_, _, a| a.push(Action::MapSettings)),
        cmd!(Icon::Schedule, "Calendar and place…", "Календарь и место…", "holidays time zone summer number plates праздники часовой пояс номера", "", |_, _, a| a.push(Action::Calendar)),
        cmd!(Icon::Traffic, "Signal routes…", "Маршруты сигналов…", "railway signal железная дорога светофор", "", |_, _, a| a.push(Action::SignalRoutes)),
        cmd!(Icon::History, "Chrono scenarios…", "Хроно-сценарии…", "years history годы", "", |_, _, a| a.push(Action::Chrono)),
        cmd!(Icon::CheckCircle, "Check map…", "Проверить карту…", "errors problems validate ошибки проблемы", "", |st, _, _| {
            st.check.open = true;
            st.check.issues = None;
        }),
        cmd!(Icon::Bus, "Lines and timetable…", "Линии и расписание…", "route trips tours schedule bus line маршрут рейсы туры автобус", "L", |_, _, a| a.push(Action::Timetable)),
        cmd!(Icon::Traffic, "Traffic and AI…", "Трафик и ИИ…", "cars density ailists depot машины плотность", "T", |_, _, a| a.push(Action::Traffic)),
        cmd!(Icon::Flag, "Depot files (destinations)…", "Файлы депо (конечные)…", "hof destinations ibis routes terminus signs конечные указатели маршруты", "", |_, _, a| a.push(Action::Depots)),
        cmd!(Icon::Layers, "Satellite and terrain…", "Спутник и рельеф…", "imagery aerial photo heights place world спутник снимок высоты место", "I", |_, _, a| a.push(Action::Geo)),
        cmd!(Icon::Road, "From OpenStreetMap…", "Из OpenStreetMap…", "osm real place streets buildings реальное место улицы здания", "", |_, _, a| a.push(Action::Osm)),
        cmd!(Icon::Route, "GPS tracks to trace…", "GPS-треки для обводки…", "gpx kml geojson trace route recorded phone трек маршрут телефон", "", |_, _, a| a.push(Action::Trace)),
        cmd!(Icon::Tiles, "Tiles from another map…", "Тайлы из другой карты…", "merge join import combine объединить соединить импорт", "", |_, _, a| a.push(Action::MergeTiles)),
        cmd!(Icon::CheckCircle, "Check in openOMSI…", "Проверка в openOMSI…", "game verify validate offscreen игра проверить", "", |_, _, a| a.push(Action::GameCheck)),
        cmd!(Icon::Play, "Test drive in openOMSI", "Тест-драйв в openOMSI", "play run game drive играть запустить", "F5", |_, _, a| a.push(Action::Play)),
        cmd!(Icon::Play, "Test drive in OMSI 2 (the original)", "Тест-драйв в оригинальном OMSI 2", "play omsi original game drive оригинал игра запустить", "", |_, _, a| a.push(Action::PlayOriginal)),
        cmd!(Icon::Flag, "Test drive from the start point nearest the view", "Тест-драйв с ближайшей к виду точки старта", "play here entry start drive играть отсюда старт", "Shift+F5", |_, _, a| a.push(Action::PlayHere(false))),
        cmd!(Icon::Visible, "See this view in openOMSI", "Посмотреть этот вид в openOMSI", "free camera look game свободная камера игра вид", "", |_, _, a| a.push(Action::PlayHere(true))),
        // View
        cmd!(Icon::Flag, "Saved views…", "Сохранённые виды…", "bookmark camera закладка камера", "Ctrl+1…9", |st, _, _| st.views.open = true),
        cmd!(Icon::Focus, "Go to selection", "К выделенному", "focus фокус", "F", |_, _, a| a.push(Action::FocusSelection)),
        cmd!(Icon::Map, "Overview map", "Обзорная карта", "minimap миникарта", "M", |st, _, _| st.minimap.open = !st.minimap.open),
        cmd!(Icon::Objects, "Assets", "Ассеты", "catalogue browser каталог", "B", |st, _, _| st.assets.open = !st.assets.open),
        cmd!(Icon::Tiles, "Tile grid", "Сетка тайлов", "show показать", "G", |_, ed, _| ed.show_grid = !ed.show_grid),
        cmd!(Icon::Visible, "What the game does not show", "То, чего не видно в игре", "technical helpers invisible технические помощники", "", |_, ed, _| ed.show_technical = !ed.show_technical),
        cmd!(Icon::Route, "AI lanes", "Полосы ИИ", "paths arrows пути стрелки", "", |_, ed, _| ed.show_lanes = !ed.show_lanes),
        cmd!(Icon::Save, "Save a picture of the view", "Сохранить картинку вида", "screenshot скриншот", "F12", |_, _, a| a.push(Action::ViewPicture)),
        cmd!(Icon::Import, "Import the map's thumbnail", "Импорт картинки карты", "picture.jpg thumbnail превью", "", |_, _, a| a.push(Action::ImportMapPicture)),
        // Help
        cmd!(Icon::CheckCircle, "Getting started", "С чего начать", "guide tutorial help учебник помощь", "", |_, _, a| a.push(Action::Guide)),
        cmd!(Icon::Help, "Controls", "Управление", "keys mouse shortcuts клавиши мышь", "", |st, _, _| st.help_open = true),
        cmd!(Icon::Settings, "Next interface language", "Следующий язык интерфейса", "language interface язык интерфейс", "", |_, _, a| {
            let choices = crate::i18n::LANGUAGES;
            let current = choices.iter().position(|(lang, _)| *lang == crate::i18n::get()).unwrap_or(0);
            let other = choices[(current + 1) % choices.len()].0;
            a.push(Action::SetLang(other))
        }),
        // where features live
        cmd!(Icon::Bus, "Place a bus stop", "Поставить остановку", "stop busstop остановка", "", |_, ed, _| {
            ed.set_tool(Tool::Logic);
            ed.set_logic(LogicMode::Stops);
        }),
        cmd!(Icon::Flag, "Place a start point", "Поставить точку старта", "entrypoint spawn старт", "", |_, ed, _| {
            ed.set_tool(Tool::Logic);
            ed.set_logic(LogicMode::Start);
        }),
        cmd!(Icon::Parking, "Car parks", "Парковки", "parking row ряд", "", |_, ed, _| {
            ed.set_tool(Tool::Logic);
            ed.set_logic(LogicMode::Parking);
        }),
        cmd!(Icon::Route, "AI paths", "Пути ИИ", "lanes cars pedestrians полосы машины пешеходы", "", |_, ed, _| {
            ed.set_tool(Tool::Logic);
            ed.set_logic(LogicMode::Paths);
        }),
        cmd!(Icon::Tree, "Plant trees with a brush", "Сажать деревья кистью", "forest plants brush лес растения кисть", "", |_, ed, _| {
            ed.set_tool(Tool::Objects);
            ed.plant_brush = true;
            ed.plant_area = false;
            ed.toast(tr("Pick a tree in the catalogue, then paint", "Выберите дерево в каталоге и ведите кистью"));
        }),
        cmd!(Icon::Tree, "Plant a wood or a park", "Засадить лес или парк", "forest area outline fill лес участок контур заполнить", "", |_, ed, _| {
            ed.set_tool(Tool::Objects);
            ed.plant_brush = true;
            ed.plant_area = true;
            ed.toast(tr("Pick a tree in the catalogue, then click round the area", "Выберите дерево в каталоге и обведите участок кликами"));
        }),
        cmd!(Icon::Road, "Roundabout", "Кольцо", "circle ring круговое движение", "", |_, ed, _| {
            ed.set_tool(Tool::Roads);
            ed.roundabout = true;
            ed.toast(tr("Pick a road type, click the centre, then the ring", "Выберите тип дороги, кликните центр, потом кольцо"));
        }),
        cmd!(Icon::Terrain, "Height profile of a road", "Профиль высот дороги", "gradient slope elevation chart уклон высота график", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select a road (double-click: all of it): its properties show its height profile", "Выберите дорогу (двойной клик: всю): в её свойствах профиль высот"));
        }),
        cmd!(Icon::Traffic, "Traffic lights and zebra crossings", "Светофоры и зебры", "junction crossing перекрёсток переход", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select one of openMapper's junctions: its properties have Traffic lights and crossings", "Выберите перекрёсток openMapper: в его свойствах светофоры и переходы"));
        }),
        cmd!(Icon::Traffic, "Link a signal head to its crossing", "Связать светофор с перекрёстком", "varparent link parent signal head display связь родитель табло головка", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select the signal head (or a display): Linked objects in its properties, then click its crossing", "Выберите головку светофора (или табло): «Связанные объекты» в свойствах, затем щёлкните перекрёсток"));
        }),
        cmd!(Icon::Fence, "Objects along a road", "Объекты вдоль дороги", "lamps fence posts row фонари забор столбы ряд", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select a road, pick an object in the Objects catalogue: Objects along the road in its properties", "Выберите дорогу и объект в каталоге: «Объекты вдоль дороги» в её свойствах"));
        }),
        cmd!(Icon::Tram, "Overhead wire along a track", "Контактный провод вдоль пути", "tram catenary fahrdraht oberleitung трамвай контактная сеть", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select a track (double-click: all of it): Overhead wire under Parallel copy in its properties", "Выберите путь (двойной клик: весь): «Контактный провод» под «Параллельной копией» в свойствах"));
        }),
        cmd!(Icon::Road, "Parallel copy of a road", "Параллельная копия дороги", "second track carriageway второй путь проезжая часть", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select a road or a chain: Parallel copy in its properties", "Выберите дорогу или цепочку: «Параллельная копия» в свойствах"));
        }),
        cmd!(Icon::Paint, "Paint the ground along a road", "Покрасить землю вдоль дороги", "verge gravel shoulder dirt обочина гравий грунт", "", |_, ed, _| {
            ed.set_tool(Tool::Select);
            ed.toast(tr("Select a road (double-click: all of it): Paint the ground along it in its properties", "Выберите дорогу (двойной клик: всю): «Покрасить землю вдоль» в её свойствах"));
        }),
        cmd!(Icon::Tiles, "Water, lake, river", "Вода, озеро, река", "sea pond море пруд", "", |_, ed, _| {
            ed.set_tool(Tool::Tiles);
            ed.toast(tr("Click a tile: its water is in the panel; lower the ground under it with the Terrain tool", "Кликните тайл: вода в панели; опустите землю инструментом «Рельеф»"));
        }),
        cmd!(Icon::Route, "Ramp in the ground", "Пандус в земле", "embankment slope level bed насыпь склон уклон полотно", "", |_, ed, _| {
            ed.set_tool(Tool::Terrain);
            ed.terrain_mode = crate::editor::TerrainMode::Ramp;
        }),
        cmd!(Icon::Terrain, "Height map import or export", "Импорт или экспорт карты высот", "heightmap png world machine gaea", "", |_, ed, _| {
            ed.set_tool(Tool::Terrain);
            ed.toast(tr("Height map: at the end of the Terrain panel", "Карта высот: в конце панели «Рельеф»"));
        }),
        cmd!(Icon::Paint, "Paint by slope and height", "Покраска по уклону и высоте", "rock snow sand auto скалы снег песок авто", "", |_, ed, _| {
            ed.set_tool(Tool::Paint);
            ed.toast(tr("By slope and height: at the end of the Paint panel", "По уклону и высоте: в конце панели «Покраска»"));
        }),
        cmd!(Icon::Schedule, "Time of day and season", "Время суток и года", "night winter snow autumn ночь зима снег осень", "", |_, ed, _| {
            ed.toast(tr("View menu: Time of day, Season", "Меню «Вид»: время суток, время года"));
        }),
    ]
}

/// How well `q` (lower case, split into words) finds this command: every word must be in
/// it; a word its name starts with, or one of its name's words, counts most, then one
/// starting any of its words (the other language's name too).
fn score(c: &Command, q: &[String]) -> Option<i32> {
    let name = c.name.to_lowercase();
    let all = format!("{name} {}", c.words.to_lowercase());
    let mut s = 0;
    for w in q {
        if !all.contains(w.as_str()) {
            return None;
        }
        s += if name.starts_with(w.as_str()) {
            4
        } else if name.split(|ch: char| !ch.is_alphanumeric()).any(|n| n.starts_with(w.as_str())) {
            3
        } else if name.contains(w.as_str()) || all.split(|ch: char| !ch.is_alphanumeric()).any(|n| n.starts_with(w.as_str())) {
            2
        } else {
            1
        };
    }
    Some(s)
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    chosen: usize,
    focus: bool,
    /// The commands run last (by name), shown first while nothing is typed.
    recent: Vec<&'static str>,
}

impl Palette {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
        self.query.clear();
        self.chosen = 0;
    }

    /// Open with a search typed (`OPENMAPPER_PALETTE`, for checks).
    pub fn show_with(&mut self, q: &str) {
        self.show();
        self.query = q.to_string();
    }

    #[cfg(test)]
    fn found(&self) -> Vec<&'static str> {
        found(&commands(), &self.query, &self.recent).into_iter().map(|c| c.name).collect()
    }
}

fn found<'a>(all: &'a [Command], query: &str, recent: &[&str]) -> Vec<&'a Command> {
    let q: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    if q.is_empty() {
        // (the ones run last first, then the rest as listed)
        let mut v: Vec<&Command> = recent.iter().filter_map(|r| all.iter().find(|c| c.name == *r)).collect();
        v.extend(all.iter().filter(|c| !recent.contains(&c.name)));
        return v;
    }
    let mut v: Vec<(i32, &Command)> = all.iter().filter_map(|c| score(c, &q).map(|s| (s, c))).collect();
    v.sort_by_key(|c| std::cmp::Reverse(c.0));
    v.into_iter().map(|(_, c)| c).collect()
}

/// The search window; a command chosen is run on the interface, the editor or as an action.
pub fn window(ctx: &egui::Context, st: &mut UiState, ed: &mut Editor, actions: &mut Vec<Action>) {
    if !st.palette.open {
        return;
    }
    let all = commands();
    let p = &mut st.palette;
    // (taken before the text field sees them)
    let (down, up, enter, esc) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
    let list = found(&all, &p.query, &p.recent);
    if down {
        p.chosen = (p.chosen + 1).min(list.len().saturating_sub(1));
    }
    if up {
        p.chosen = p.chosen.saturating_sub(1);
    }
    let mut run: Option<usize> = enter.then_some(p.chosen);
    let width = 440.0;
    let area = egui::Area::new(egui::Id::new("command palette")).anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 90.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(width);
            let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(tr("What do you want to do?", "Что вы хотите сделать?")).desired_width(width));
            if p.focus {
                r.request_focus();
                p.focus = false;
            }
            if r.changed() {
                p.chosen = 0;
            }
            ui.add_space(4.0);
            if list.is_empty() {
                ui.label(RichText::new(tr("Nothing found", "Ничего не найдено")).italics().color(TEXT_DIM));
            }
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for (i, c) in list.iter().enumerate() {
                    let on = i == p.chosen;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
                    if on {
                        ui.painter().rect_filled(rect, 3.0, crate::ui::SELECT);
                        if down || up {
                            ui.scroll_to_rect(rect, None);
                        }
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 3.0, Color32::from_gray(45));
                    }
                    icons::paint(ui.painter(), c.icon, rect.left_center() + Vec2::new(12.0, 0.0), 15.0, Color32::from_gray(210));
                    ui.painter().text(rect.left_center() + Vec2::new(28.0, 0.0), egui::Align2::LEFT_CENTER, c.name, egui::FontId::proportional(13.5), Color32::from_gray(235));
                    if !c.key.is_empty() {
                        ui.painter().text(rect.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, c.key, egui::FontId::proportional(11.5), TEXT_DIM);
                    }
                    if resp.clicked() {
                        run = Some(i);
                    }
                }
            });
        });
    });
    // (a click elsewhere, or Esc, puts it away)
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().map(|q| !area.response.rect.contains(q)).unwrap_or(false));
    if esc || outside {
        p.open = false;
    }
    if let Some(c) = run.and_then(|i| list.get(i)) {
        let f = c.run;
        let name = c.name;
        p.open = false;
        p.recent.retain(|r| *r != name);
        p.recent.insert(0, name);
        p.recent.truncate(5);
        f(st, ed, actions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_found_by_their_names_and_words_in_either_language() {
        let mut p = Palette::default();
        p.query = "time".into();
        assert_eq!(p.found()[0], "Time of day and season");
        p.query = "lines".into();
        assert_eq!(p.found()[0], "Lines and timetable…");
        p.query = "расписание".into();
        assert_eq!(p.found()[0], "Lines and timetable…");
        p.query = "gradient".into();
        assert!(p.found().contains(&"Height profile of a road"));
        p.query = "wood".into();
        assert_eq!(p.found()[0], "Plant a wood or a park");
        p.query = "zzz".into();
        assert!(p.found().is_empty());
        // nothing typed: those run last first
        p.query.clear();
        p.recent = vec!["Check map…"];
        assert_eq!(p.found()[0], "Check map…");
        assert_eq!(p.found().len(), commands().len());
    }
}
