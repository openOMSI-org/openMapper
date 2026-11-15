//! The New map dialog: what a map is before anything is built on it: its name and
//! description, size, the height and texture of its ground, which side traffic keeps to,
//! its years, money and tickets, how busy its streets are, its seasons, and whether to start
//! from a real place.

use std::collections::HashMap;

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::i18n::tr;
use crate::ui::{Action, ACCENT};

const TEXT_DIM: Color32 = Color32::from_rgb(140, 143, 150);

/// The seasons a new map starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seasons {
    /// Summer all year (none set).
    Summer,
    /// Spring, summer, autumn and a winter with snow, as in central Europe.
    Four,
    /// Winter without snow instead.
    Mild,
}

/// What a new map is made with.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMapOptions {
    pub description: String,
    /// The base ground (None: the template's grass) and its height above sea level (m).
    pub ground: Option<String>,
    pub height: f32,
    pub left_hand: bool,
    pub real_rail: bool,
    pub years: (i32, i32),
    pub year_offset: i32,
    /// Money system and ticket pack (empty: the template's).
    pub money: String,
    pub tickets: String,
    /// How busy the streets and stops are (1 = the template's).
    pub traffic: f32,
    pub passengers: f32,
    pub seasons: Seasons,
    /// The window for real places (satellite imagery, terrain, OpenStreetMap) opened at once.
    pub real_place: bool,
}

impl Default for NewMapOptions {
    fn default() -> Self {
        NewMapOptions { description: String::new(), ground: None, height: 0.0, left_hand: false, real_rail: true, years: (2000, 2030), year_offset: 0, money: String::new(), tickets: String::new(), traffic: 1.0, passengers: 1.0, seasons: Seasons::Four, real_place: false }
    }
}

/// What the dialog asks.
pub struct NewMapForm {
    pub name: String,
    pub size: i32,
    pub options: NewMapOptions,
    /// Money systems and ticket packs to choose from (read when the dialog opens).
    lists: Option<(Vec<String>, Vec<String>)>,
}

impl Default for NewMapForm {
    fn default() -> Self {
        NewMapForm { name: String::new(), size: 3, options: NewMapOptions::default(), lists: None }
    }
}

/// The ground textures offered first: grass, meadows, fields.
fn is_ground(f: &str) -> bool {
    let l = f.to_lowercase();
    if l.contains("_det") || l.contains("noise") {
        return false;
    }
    ["gras", "wiese", "meadow", "feld", "field", "acker", "erde", "sand", "kies"].iter().any(|k| l.contains(k))
}

pub fn dialog(ctx: &egui::Context, form: &mut Option<NewMapForm>, roots: Option<&crate::install::Roots>, textures: &[String], swatches: &HashMap<String, egui::TextureHandle>, wanted: &mut Vec<String>, actions: &mut Vec<Action>) {
    let Some(f) = form else { return };
    if f.lists.is_none() {
        f.lists = Some(roots.map(|r| (crate::ui_mapsettings::content_files(r, "Money", "cfg"), crate::ui_mapsettings::content_files(r, "TicketPacks", "cfg"))).unwrap_or_default());
    }
    let mut close = false;
    egui::Window::new(tr("New map", "Новая карта")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
        ui.set_width(480.0);
        let o = &mut f.options;
        egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.75).show(ui, |ui| {
            egui::Grid::new("new map").num_columns(2).spacing(Vec2::new(10.0, 8.0)).show(ui, |ui| {
                ui.label(tr("Name:", "Название:"));
                let r = ui.add(egui::TextEdit::singleline(&mut f.name).hint_text(tr("My town", "Мой город")).desired_width(300.0));
                if !r.has_focus() && f.name.is_empty() {
                    r.request_focus();
                }
                ui.end_row();
                ui.label(tr("Description:", "Описание:"));
                ui.add(egui::TextEdit::multiline(&mut o.description).hint_text(tr("What the map is, for the game's map list", "Что это за карта (для списка карт в игре)")).desired_rows(2).desired_width(300.0));
                ui.end_row();
                ui.label(tr("Size:", "Размер:"));
                egui::ComboBox::from_id_salt("size").selected_text(format!("{n} × {n} {}", tr("tiles", "тайлов"), n = f.size)).show_ui(ui, |ui| {
                    for n in [1, 2, 3, 4, 5, 7, 9] {
                        ui.selectable_value(&mut f.size, n, format!("{n} × {n} ({} × {} m)", n * 300, n * 300));
                    }
                });
                ui.end_row();
                ui.label(tr("Ground height:", "Высота земли:"));
                ui.add(egui::DragValue::new(&mut o.height).range(-100.0..=3000.0).speed(1.0).suffix(tr(" m above sea level", " м над уровнем моря")));
                ui.end_row();
                ui.label(tr("Traffic:", "Движение:"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut o.left_hand, false, tr("keeps right", "правостороннее"));
                    ui.selectable_value(&mut o.left_hand, true, tr("keeps left", "левостороннее"));
                });
                ui.end_row();
                ui.label(tr("Busy:", "Загруженность:"));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut o.traffic, 0.0..=2.0).text(tr("cars", "машины")));
                });
                ui.end_row();
                ui.label("");
                ui.add(egui::Slider::new(&mut o.passengers, 0.0..=2.0).text(tr("passengers", "пассажиры")));
                ui.end_row();
                ui.label(tr("Years:", "Годы:"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut o.years.0).range(1900..=2100));
                    ui.label("–");
                    ui.add(egui::DragValue::new(&mut o.years.1).range(1900..=2100));
                    ui.label(RichText::new(tr("year offset", "сдвиг года")).color(TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut o.year_offset).range(-200..=200)).on_hover_text(tr("Added to today's year when the game starts at the real date", "Прибавляется к текущему году, когда игра начинается в реальную дату"));
                });
                ui.end_row();
                let (money, tickets) = f.lists.clone().unwrap_or_default();
                ui.label(tr("Money:", "Валюта:"));
                choose(ui, "money", &mut o.money, &money);
                ui.end_row();
                ui.label(tr("Tickets:", "Билеты:"));
                choose(ui, "tickets", &mut o.tickets, &tickets);
                ui.end_row();
                ui.label(tr("Seasons:", "Времена года:"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut o.seasons, Seasons::Four, tr("four, snowy winter", "четыре, зима со снегом"));
                    ui.selectable_value(&mut o.seasons, Seasons::Mild, tr("mild winter", "мягкая зима"));
                    ui.selectable_value(&mut o.seasons, Seasons::Summer, tr("summer only", "только лето"));
                });
                ui.end_row();
                ui.label(tr("Rail:", "Рельсы:"));
                ui.checkbox(&mut o.real_rail, tr("trams and trains follow their tracks' switches", "трамваи и поезда идут по стрелкам"));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(tr("Ground texture (all tiles):", "Текстура земли (все тайлы):"));
            let mut list: Vec<&String> = textures.iter().filter(|t| is_ground(t)).collect();
            if list.is_empty() {
                list = textures.iter().collect();
            }
            egui::ScrollArea::horizontal().id_salt("ground choice").max_height(96.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut cell = |ui: &mut egui::Ui, rel: Option<&String>| {
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(70.0, 84.0), Sense::click());
                        let on = o.ground.as_ref() == rel;
                        let img = egui::Rect::from_min_size(rect.min + Vec2::new(3.0, 2.0), Vec2::splat(64.0));
                        match rel {
                            Some(r) => match swatches.get(&r.to_lowercase()) {
                                Some(h) => {
                                    egui::Image::new((h.id(), img.size())).paint_at(ui, img);
                                }
                                None => {
                                    wanted.push(r.clone());
                                    ui.painter().rect_filled(img, 2.0, Color32::from_gray(40));
                                }
                            },
                            None => {
                                ui.painter().rect_filled(img, 2.0, Color32::from_rgb(70, 95, 45));
                            }
                        }
                        if on || resp.hovered() {
                            ui.painter().rect_stroke(img, 2.0, Stroke::new(2.0, if on { ACCENT } else { Color32::from_gray(150) }), egui::StrokeKind::Outside);
                        }
                        let name = rel.map(|r| r.rsplit('\\').next().unwrap_or(r).to_string()).unwrap_or_else(|| tr("template", "шаблон").to_string());
                        ui.painter().text(egui::pos2(rect.center().x, rect.max.y - 7.0), egui::Align2::CENTER_CENTER, name.chars().take(12).collect::<String>(), egui::FontId::proportional(10.0), Color32::from_gray(200));
                        if resp.clicked() {
                            o.ground = rel.cloned();
                        }
                    };
                    cell(ui, None);
                    for t in list.iter().take(60) {
                        cell(ui, Some(t));
                    }
                });
            });
            ui.label(RichText::new(tr("More ground layers are painted later (Paint tool); the map's settings can all be changed in Map > Map settings.", "Другие слои земли рисуются позже (инструмент «Покраска»); все настройки карты меняются в «Карта > Настройки карты».")).small().color(TEXT_DIM));
        });
        ui.separator();
        crate::ui::button_row(ui, |ui| {
            if ui.button(tr("Cancel", "Отмена")).clicked() {
                close = true;
            }
            let ok = !f.name.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(tr("Create", "Создать"))).clicked() || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)) {
                let friendly = f.name.trim().to_string();
                // (OMSI 2 opens no map whose folder has a letter outside ASCII)
                let folder = friendly.split(' ').map(crate::omsi_name::ascii_stem).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
                let folder = if folder.is_empty() { "Map".to_string() } else { folder };
                actions.push(Action::NewMap { folder, friendly, size: f.size, options: f.options.clone() });
                close = true;
            }
        });
    });
    if close {
        *form = None;
    }
}

fn choose(ui: &mut egui::Ui, id: &str, value: &mut String, list: &[String]) {
    let shown = if value.is_empty() { tr("as the template", "как в шаблоне").to_string() } else { value.rsplit('\\').nth(1).unwrap_or(value).to_string() };
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(240.0).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), tr("as the template", "как в шаблоне"));
        for v in list {
            let name = v.rsplit('\\').nth(1).unwrap_or(v);
            ui.selectable_value(value, v.clone(), name);
        }
    });
}
