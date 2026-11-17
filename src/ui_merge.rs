//! The window of Map > Tiles from another map: the other map chosen, its tiles shown over
//! this map's on a small plan, picked by a click and moved into place, then added (see
//! `merge`).

use std::path::PathBuf;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::doc::TileKey;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::MapEntry;
use crate::library::Library;
use crate::merge::Source;
use crate::theme::pal;

#[derive(Default)]
pub struct MergeWindow {
    pub open: bool,
    maps: Vec<MapEntry>,
    source: Option<Source>,
    error: Option<String>,
    chosen: Vec<TileKey>,
    offset: (i32, i32),
    dz: f32,
    blend: bool,
}

impl MergeWindow {
    /// Open it with the maps to choose from.
    pub fn show(&mut self, maps: Vec<MapEntry>) {
        self.open = true;
        self.maps = maps;
        self.blend = true;
    }

    fn pick(&mut self, cfg: PathBuf, ed: &Editor) {
        self.error = None;
        self.source = None;
        match crate::merge::open(&cfg) {
            Ok(s) => {
                // (beside the map, to the east, its south edges in line)
                if let (Some(doc), Some(lo)) = (ed.doc.as_ref(), s.keys.iter().copied().reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))) {
                    let east = doc.tiles.keys().map(|k| k.0).max().unwrap_or(-1) + 1;
                    let south = doc.tiles.keys().map(|k| k.1).min().unwrap_or(0);
                    self.offset = (east - lo.0, south - lo.1);
                }
                self.chosen = s.keys.clone();
                self.dz = 0.0;
                self.source = Some(s);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Open on a map (checks without a person at the screen); `add`: all its tiles added.
    pub fn open_on(&mut self, cfg: PathBuf, ed: &mut Editor, lib: &Library, add: bool) {
        self.pick(cfg, ed);
        if let (true, Some(src), Some(doc)) = (add, &self.source, ed.doc.as_mut()) {
            let r = crate::merge::add_tiles(doc, &mut ed.history, src, &self.chosen, self.offset, self.dz, self.blend, |f| lib.find(f).map(|a| a.abs_height).unwrap_or(false));
            log::info!("tiles from another map: {r:?}");
        }
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library) {
        if !self.open || ed.doc.is_none() {
            return;
        }
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Tiles, tr("Tiles from another map", "Тайлы из другой карты"))).id(egui::Id::new("merge tiles")).open(&mut open).default_pos([340.0, 110.0]).resizable(false).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(tr("Its ground, painting, water, objects and roads, put beside or into this map. Lines, timetable and AI lists stay with the other map.", "Её земля, покраска, вода, объекты и дороги — рядом с этой картой или в неё. Линии, расписание и списки ИИ остаются в той карте.")).small().color(pal().text_dim));
            ui.add_space(4.0);
            let here = ed.doc.as_ref().map(|d| d.dir.clone()).unwrap_or_default();
            let name = self.source.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| tr("Choose a map…", "Выберите карту…").to_string());
            let mut pick: Option<PathBuf> = None;
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("merge map").selected_text(name).width(240.0).show_ui(ui, |ui| {
                    for m in self.maps.iter().filter(|m| m.cfg.parent() != Some(here.as_path())) {
                        if ui.selectable_label(false, &m.name).clicked() {
                            pick = Some(m.cfg.clone());
                        }
                    }
                });
                if ui.button(icons::labelled(Icon::Open, tr("Other…", "Другая…"))).clicked() {
                    pick = rfd::FileDialog::new().add_filter("OMSI map", &["cfg"]).set_title(tr("A map to take tiles from (its global.cfg)", "Карта, из которой взять тайлы (её global.cfg)")).pick_file();
                }
            });
            if let Some(p) = pick {
                self.pick(p, ed);
            }
            if let Some(e) = &self.error {
                ui.colored_label(pal().error, e);
            }
            let Some(src) = &self.source else { return };
            let Some(doc) = ed.doc.as_ref() else { return };
            let to = |k: TileKey, o: (i32, i32)| (k.0 + o.0, k.1 + o.1);
            // the plan: this map's tiles grey, the other's where they would go
            let all: Vec<TileKey> = doc.tiles.keys().copied().chain(src.keys.iter().map(|k| to(*k, self.offset))).collect();
            let (lo, hi) = all.iter().fold(((i32::MAX, i32::MAX), (i32::MIN, i32::MIN)), |(a, b), k| ((a.0.min(k.0), a.1.min(k.1)), (b.0.max(k.0), b.1.max(k.1))));
            let (w, h) = ((hi.0 - lo.0 + 1) as f32, (hi.1 - lo.1 + 1) as f32);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(380.0, 260.0), Sense::click());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 2.0, pal().field);
            let cell = ((rect.width() - 8.0) / w).min((rect.height() - 8.0) / h).min(28.0);
            let o = rect.center() - Vec2::new(w * cell, h * cell) * 0.5;
            // (north up)
            let sq = |k: TileKey| Rect::from_min_size(Pos2::new(o.x + (k.0 - lo.0) as f32 * cell, o.y + (hi.1 - k.1) as f32 * cell), Vec2::splat(cell)).shrink(0.5);
            for k in doc.tiles.keys() {
                p.rect_filled(sq(*k), 0.0, pal().ink_dim);
            }
            let mut toggle = None;
            for k in &src.keys {
                let at = to(*k, self.offset);
                let r = sq(at);
                let on = self.chosen.contains(k);
                let clash = doc.tiles.contains_key(&at);
                let (fill, stroke) = match (on, clash) {
                    (true, true) => (Color32::from_rgba_unmultiplied(220, 70, 60, 150), Color32::from_rgb(235, 90, 80)),
                    (true, false) => (Color32::from_rgba_unmultiplied(232, 122, 50, 170), pal().accent),
                    _ => (Color32::TRANSPARENT, pal().text_dim),
                };
                p.rect_filled(r, 0.0, fill);
                p.rect_stroke(r, 0.0, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
                if resp.clicked() && resp.interact_pointer_pos().map(|q| r.contains(q)).unwrap_or(false) {
                    toggle = Some(*k);
                }
            }
            if let Some(k) = toggle {
                match self.chosen.iter().position(|c| *c == k) {
                    Some(i) => {
                        self.chosen.remove(i);
                    }
                    None => self.chosen.push(k),
                }
            }
            let fits: Vec<TileKey> = self.chosen.iter().copied().filter(|k| !doc.tiles.contains_key(&to(*k, self.offset))).collect();
            ui.label(RichText::new(tr("Grey: this map · orange: the tiles to add (click to take one or leave it) · red: on a tile of this map, left out", "Серые: эта карта · оранжевые: тайлы для добавления (клик: взять или оставить) · красные: на тайле этой карты, не берутся")).small().color(pal().text_dim));
            ui.horizontal(|ui| {
                if ui.small_button(tr("All", "Все")).clicked() {
                    self.chosen = src.keys.clone();
                }
                if ui.small_button(tr("None", "Ни одного")).clicked() {
                    self.chosen.clear();
                }
                ui.add_space(12.0);
                ui.label(tr("Move:", "Сдвиг:"));
                for (text, d) in [(tr("west", "запад"), (-1, 0)), (tr("east", "восток"), (1, 0)), (tr("north", "север"), (0, 1)), (tr("south", "юг"), (0, -1))] {
                    if ui.small_button(text).clicked() {
                        self.offset = (self.offset.0 + d.0, self.offset.1 + d.1);
                    }
                }
            });
            egui::Grid::new("merge opts").num_columns(2).show(ui, |ui| {
                ui.label(tr("Raise it by", "Поднять на"));
                ui.add(egui::DragValue::new(&mut self.dz).speed(0.1).suffix(" m")).on_hover_text(tr("Its ground, water, roads and objects at a height of their own", "Её земля, вода, дороги и объекты на собственной высоте"));
                ui.end_row();
            });
            crate::ui::check(ui, &mut self.blend, tr("Ease the ground where the maps meet", "Сгладить землю на стыке карт"));
            ui.add_space(4.0);
            let label = format!("{} {} {}", tr("Add", "Добавить"), fits.len(), tr("tiles", "тайлов"));
            if ui.add_enabled(!fits.is_empty(), egui::Button::new(icons::labelled(Icon::Add, &label))).clicked() {
                let Some(doc) = ed.doc.as_mut() else { return };
                match crate::merge::add_tiles(doc, &mut ed.history, src, &fits, self.offset, self.dz, self.blend, |f| lib.find(f).map(|a| a.abs_height).unwrap_or(false)) {
                    Ok(n) => {
                        ed.toast(format!("{} {n} {}", tr("Added", "Добавлено"), tr("tiles from the other map", "тайлов из другой карты")));
                        self.chosen.retain(|k| !fits.contains(k));
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        });
        self.open = open;
    }
}
