//! The asset browser, as Unity's Project window: a panel under the view with the folders
//! of scenery objects and spline types on the left and their pictures in a grid on the
//! right. Search, a filter by kind, the size of the pictures; a click shows an asset's facts
//! and what can be done with it, a double click puts it to use, and a picture dragged into
//! the view places the object where it is let go.

use std::collections::{BTreeMap, HashMap};

use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::editor::{Editor, Sample, Tool};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{Asset, Category, Kind, Library};
use crate::render::Thumb;
use crate::ui::Action;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Show {
    All,
    Objects,
    Splines,
    Textures,
}

/// What the panel works out from the library, the search and the folder, kept until one of
/// them changes (the library is thousands of assets: not to be sorted again every frame).
struct Cache {
    key: u64,
    /// Per kind: its folders, each with its asset count and its subfolders.
    tree: Vec<(Kind, Vec<(String, usize, Vec<(String, usize)>)>)>,
    /// The folders of the textures, the same way.
    tex_tree: Vec<(String, usize, Vec<(String, usize)>)>,
    /// The assets shown (indices into the library), or the textures shown (into the files).
    items: Vec<usize>,
}

/// A folder path as (its top folder, the rest).
fn split_folder(d: &str) -> (String, String) {
    d.split_once('\\').map(|(t, r)| (t.to_string(), r.to_string())).unwrap_or((d.to_string(), String::new()))
}

fn tree_of<'a>(dirs: impl Iterator<Item = &'a str>) -> Vec<(String, usize, Vec<(String, usize)>)> {
    let mut tree: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for d in dirs {
        let (top, rest) = split_folder(d);
        *tree.entry(top).or_default().entry(rest).or_default() += 1;
    }
    tree.into_iter().map(|(t, subs)| (t, subs.values().sum(), subs.into_iter().filter(|(s, _)| !s.is_empty()).collect())).collect()
}

/// The folder a texture file lies in.
fn tex_dir(rel: &str) -> &str {
    rel.rsplit_once('\\').map(|r| r.0).unwrap_or("")
}

pub struct AssetBrowser {
    pub open: bool,
    search: String,
    show: Show,
    /// The folder shown: (objects or splines, its path below `Sceneryobjects` / `Splines`);
    /// None: everything.
    folder: Option<(Kind, String)>,
    size: f32,
    picked: Option<String>,
    /// A picture being dragged towards the view.
    pub dragging: Option<String>,
    confirm_delete: Option<String>,
    /// The texture folder shown (None: all) and the texture picked.
    tex_folder: Option<String>,
    tex_picked: Option<String>,
    cache: Option<Cache>,
}

impl Default for AssetBrowser {
    fn default() -> Self {
        AssetBrowser { open: false, search: String::new(), show: Show::All, folder: None, size: 88.0, picked: None, dragging: None, confirm_delete: None, tex_folder: None, tex_picked: None, cache: None }
    }
}

/// An asset's folder path as the tree shows it.
fn folder_of(a: &Asset) -> &str {
    if a.dir.is_empty() { crate::i18n::tr("(not in a folder)", "(вне папок)") } else { &a.dir }
}

impl AssetBrowser {
    /// The panel; `view`: the 3D view's rectangle (a picture dropped there is placed).
    #[allow(clippy::too_many_arguments)]
    pub fn panel(&mut self, root: &mut egui::Ui, ed: &mut Editor, lib: &Library, thumbs: &HashMap<String, Option<Thumb>>, wanted: &mut Vec<(String, bool)>, favourites: &[String], textures: &[String], swatches: &HashMap<String, egui::TextureHandle>, wanted_swatches: &mut Vec<String>, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        if let Some((rel, true)) = ed.reveal.clone() {
            if let Some(a) = lib.find(&rel) {
                let name = rel.rsplit(['\\', '/']).next().unwrap_or(&rel);
                self.search = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_string();
                self.show = if a.kind == Kind::Spline { Show::Splines } else { Show::Objects };
                self.folder = None;
                self.picked = Some(a.rel.clone());
            }
        }
        // (worked out again only when the library, the search, the kind or the folder changed)
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (lib.assets.len(), textures.len(), self.search.trim().to_lowercase(), self.show as u8, &self.folder, &self.tex_folder, favourites).hash(&mut h);
            h.finish()
        };
        let cache = match self.cache.take() {
            Some(c) if c.key == key => c,
            _ => self.build_cache(key, lib, favourites, textures),
        };
        let on_textures = self.show == Show::Textures;
        egui::Panel::bottom("assets").resizable(true).default_size(260.0).size_range(140.0..=600.0).frame(egui::Frame::new().fill(Color32::from_rgb(33, 34, 38)).inner_margin(egui::Margin::same(6))).show(root, |ui| {
            // the bar: search, kind, size, import
            ui.horizontal(|ui| {
                ui.label(icons::labelled(Icon::Objects, tr("Assets", "Ассеты")));
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("Search", "Поиск")).desired_width(200.0));
                let armed = matches!(ed.sample, Some(Sample::Either));
                let tip = tr("Click an object or a spline in the view", "Щёлкните объект или сплайн в виде");
                if ui.add(egui::Button::new(Icon::Dropper.text(16.0)).selected(armed)).on_hover_text(tip).clicked() {
                    if armed {
                        ed.sample = None;
                    } else {
                        ed.sample = Some(Sample::Either);
                        ed.sample_browser = true;
                        ed.toast(tip);
                    }
                }
                for (s, label) in [(Show::All, tr("All", "Все")), (Show::Objects, tr("Objects", "Объекты")), (Show::Splines, tr("Roads and splines", "Дороги и сплайны")), (Show::Textures, tr("Textures", "Текстуры"))] {
                    if ui.selectable_label(self.show == s, label).clicked() {
                        self.show = s;
                        self.folder = None;
                        self.tex_folder = None;
                    }
                }
                ui.separator();
                ui.add(egui::Slider::new(&mut self.size, 56.0..=160.0).show_value(false)).on_hover_text(tr("Picture size", "Размер картинок"));
                if ui.button(icons::labelled(Icon::Import, tr("Import…", "Импорт…"))).clicked() {
                    actions.push(Action::Import);
                }
                ui.menu_button(tr("Shapes…", "Фигуры…"), |ui| crate::ui_mesh::primitives_menu(ui, actions));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text(tr("Close (B)", "Закрыть (B)")).clicked() {
                        self.open = false;
                    }
                });
            });
            ui.separator();
            let kinds: &[Kind] = match self.show {
                Show::All => &[Kind::Object, Kind::Spline],
                Show::Objects => &[Kind::Object],
                Show::Splines => &[Kind::Spline],
                Show::Textures => &[],
            };
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                // the folders
                ui.vertical(|ui| {
                    ui.set_width(220.0);
                    egui::ScrollArea::vertical().id_salt("asset folders").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
                        let none = if on_textures { self.tex_folder.is_none() } else { self.folder.is_none() };
                        if ui.selectable_label(none, RichText::new(tr("Everything", "Всё")).strong()).clicked() {
                            self.folder = None;
                            self.tex_folder = None;
                        }
                        if on_textures {
                            for (top, n, subs) in &cache.tex_tree {
                                let on = self.tex_folder.as_deref() == Some(top.as_str());
                                let text = RichText::new(format!("{top} ({n})"));
                                let r = egui::CollapsingHeader::new(if on { text.color(Color32::from_rgb(232, 150, 80)) } else { text }).id_salt(("tex top", top)).show(ui, |ui| {
                                    for (sub, n) in subs {
                                        let path = format!("{top}\\{sub}");
                                        if ui.selectable_label(self.tex_folder.as_deref() == Some(path.as_str()), format!("{sub} ({n})")).clicked() {
                                            self.tex_folder = Some(path);
                                        }
                                    }
                                });
                                if r.header_response.clicked() {
                                    self.tex_folder = Some(top.clone());
                                }
                            }
                            return;
                        }
                        if !favourites.is_empty() && ui.selectable_label(self.folder.as_ref().map(|f| f.1 == "\u{2605}").unwrap_or(false), icons::labelled(Icon::Star, tr("Favourites", "Избранное"))).clicked() {
                            self.folder = Some((Kind::Object, "\u{2605}".into()));
                        }
                        for (kind, tops) in cache.tree.iter().filter(|(k, _)| kinds.contains(k)) {
                            let title = if *kind == Kind::Object { tr("Scenery objects", "Объекты") } else { tr("Splines (roads, tracks…)", "Сплайны (дороги, рельсы…)") };
                            egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("asset kind", *kind == Kind::Spline)).default_open(true).show(ui, |ui| {
                                for (top, n, subs) in tops {
                                    let on = self.folder.as_ref().map(|f| f.0 == *kind && f.1 == *top).unwrap_or(false);
                                    let text = RichText::new(format!("{top} ({n})"));
                                    let r = egui::CollapsingHeader::new(if on { text.color(Color32::from_rgb(232, 150, 80)) } else { text }).id_salt(("asset top", *kind == Kind::Spline, top)).show(ui, |ui| {
                                        for (sub, n) in subs {
                                            let path = format!("{top}\\{sub}");
                                            let on = self.folder.as_ref().map(|f| f.0 == *kind && f.1 == path).unwrap_or(false);
                                            if ui.selectable_label(on, format!("{sub} ({n})")).clicked() {
                                                self.folder = Some((*kind, path));
                                            }
                                        }
                                    });
                                    if r.header_response.clicked() {
                                        self.folder = Some((*kind, top.clone()));
                                    }
                                }
                            });
                        }
                    });
                });
                ui.separator();
                // the pictures
                ui.vertical(|ui| {
                    let picked = if on_textures { self.tex_picked.is_some() } else { self.picked.is_some() };
                    let detail_w = if picked { 240.0 } else { 0.0 };
                    ui.horizontal_top(|ui| {
                        let grid_w = (ui.available_width() - detail_w - 8.0).max(120.0);
                        ui.vertical(|ui| {
                            ui.set_width(grid_w);
                            ui.label(RichText::new(format!("{} {}", cache.items.len(), tr("items", "шт."))).small().color(Color32::from_gray(140)));
                            let cell = Vec2::new(self.size + 8.0, self.size + 30.0);
                            let per_row = ((grid_w / cell.x).floor() as usize).max(1);
                            // (only the rows in view are drawn, and only those ask for pictures)
                            egui::ScrollArea::vertical().id_salt(("asset grid", on_textures)).max_height(height - 20.0).auto_shrink([false, false]).show_rows(ui, cell.y, cache.items.len().div_ceil(per_row), |ui, rows| {
                                for r in rows {
                                    ui.horizontal(|ui| {
                                        for &i in cache.items.iter().skip(r * per_row).take(per_row) {
                                            if on_textures {
                                                self.texture_cell(ui, &textures[i], cell, swatches, wanted_swatches);
                                            } else {
                                                self.cell(ui, &lib.assets[i], cell, ed, thumbs, wanted, actions);
                                            }
                                        }
                                    });
                                }
                            });
                        });
                        if on_textures {
                            if let Some(rel) = self.tex_picked.clone() {
                                ui.separator();
                                ui.vertical(|ui| {
                                    ui.set_width(detail_w);
                                    self.texture_details(ui, &rel, swatches, actions);
                                });
                            }
                        } else if let Some(rel) = self.picked.clone() {
                            if let Some(a) = lib.find(&rel) {
                                ui.separator();
                                ui.vertical(|ui| {
                                    ui.set_width(detail_w);
                                    self.details(ui, a, ed, thumbs, favourites, actions);
                                });
                            }
                        }
                    });
                });
            });
        });
        self.cache = Some(cache);
        if matches!(&ed.reveal, Some((_, true))) {
            ed.reveal = None;
        }
        // a picture let go over the view: placed there
        if let Some(rel) = self.dragging.clone() {
            if !root.ctx().input(|i| i.pointer.any_down()) {
                self.dragging = None;
                if ed.cursor_in_view {
                    actions.push(Action::PlaceAsset(rel));
                }
            } else if let Some(p) = root.ctx().pointer_latest_pos() {
                // (what is being dragged, under the hand)
                let painter = root.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("asset drag")));
                let name = lib.find(&rel).map(|a| a.name.clone()).unwrap_or(rel);
                painter.text(p + Vec2::new(14.0, 4.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(13.0), Color32::WHITE);
            }
        }
        self.confirm(root.ctx(), lib, actions);
    }

    fn build_cache(&self, key: u64, lib: &Library, favourites: &[String], textures: &[String]) -> Cache {
        let q = self.search.trim().to_lowercase();
        let tree = [Kind::Object, Kind::Spline].into_iter().map(|k| (k, tree_of(lib.assets.iter().filter(|a| a.kind == k).map(folder_of)))).collect();
        let tex_tree = tree_of(textures.iter().map(|t| tex_dir(t)));
        let items: Vec<usize> = if self.show == Show::Textures {
            let f = self.tex_folder.clone();
            textures
                .iter()
                .enumerate()
                .filter(|(_, t)| q.is_empty() || t.to_lowercase().contains(&q))
                .filter(|(_, t)| f.as_ref().map(|f| tex_dir(t) == f || tex_dir(t).starts_with(&format!("{f}\\"))).unwrap_or(true))
                .map(|(i, _)| i)
                .collect()
        } else {
            let kinds: &[Kind] = if self.show == Show::Objects { &[Kind::Object] } else if self.show == Show::Splines { &[Kind::Spline] } else { &[Kind::Object, Kind::Spline] };
            let fav = self.folder.as_ref().map(|f| f.1 == "\u{2605}").unwrap_or(false);
            lib.assets
                .iter()
                .enumerate()
                .filter(|(_, a)| kinds.contains(&a.kind))
                .filter(|(_, a)| q.is_empty() || a.haystack.contains(&q))
                .filter(|(_, a)| match &self.folder {
                    None => true,
                    Some(_) if fav => favourites.iter().any(|f| f.eq_ignore_ascii_case(&a.rel)),
                    Some((k, f)) => a.kind == *k && (folder_of(a) == f || folder_of(a).starts_with(&format!("{f}\\"))),
                })
                .map(|(i, _)| i)
                .collect()
        };
        Cache { key, tree, tex_tree, items }
    }

    fn texture_cell(&mut self, ui: &mut egui::Ui, rel: &str, cell: Vec2, swatches: &HashMap<String, egui::TextureHandle>, wanted: &mut Vec<String>) {
        let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
        let on = self.tex_picked.as_deref() == Some(rel);
        let bg = if on { Color32::from_rgb(64, 86, 122) } else if resp.hovered() { Color32::from_gray(48) } else { Color32::from_rgb(40, 41, 45) };
        ui.painter().rect_filled(rect.shrink(2.0), CornerRadius::same(4), bg);
        let img = Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::splat(self.size));
        match swatches.get(&rel.to_lowercase()) {
            Some(h) => {
                egui::Image::new((h.id(), img.size())).paint_at(ui, img);
            }
            None => {
                wanted.push(rel.to_string());
                ui.painter().rect_filled(img.shrink(self.size * 0.3), CornerRadius::same(3), Color32::from_gray(55));
            }
        }
        let name = rel.rsplit('\\').next().unwrap_or(rel);
        let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 4.0, img.bottom() + 2.0), rect.right_bottom() - Vec2::new(4.0, 2.0));
        let galley = ui.painter().layout(name.to_string(), egui::FontId::proportional(11.0), Color32::from_gray(220), text_rect.width());
        ui.painter().with_clip_rect(text_rect).galley(text_rect.min, galley, Color32::WHITE);
        if resp.on_hover_text(rel).clicked() {
            self.tex_picked = Some(rel.to_string());
        }
    }

    fn texture_details(&mut self, ui: &mut egui::Ui, rel: &str, swatches: &HashMap<String, egui::TextureHandle>, actions: &mut Vec<Action>) {
        if let Some(h) = swatches.get(&rel.to_lowercase()) {
            ui.add(egui::Image::new((h.id(), Vec2::splat(150.0))));
        }
        ui.label(RichText::new(rel.rsplit('\\').next().unwrap_or(rel)).strong().color(Color32::WHITE));
        ui.label(RichText::new(rel).small().color(Color32::from_gray(140)));
        ui.add_space(4.0);
        if ui.button(icons::labelled(Icon::Paint, tr("Add as a ground texture", "Добавить как текстуру земли"))).clicked() {
            actions.push(Action::AddGroundTexture(rel.to_string()));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn cell(&mut self, ui: &mut egui::Ui, a: &Asset, cell: Vec2, ed: &mut Editor, thumbs: &HashMap<String, Option<Thumb>>, wanted: &mut Vec<(String, bool)>, actions: &mut Vec<Action>) {
        let (rect, resp) = ui.allocate_exact_size(cell, Sense::click_and_drag());
        let on = self.picked.as_deref().map(|p| p.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
        let bg = if on { Color32::from_rgb(64, 86, 122) } else if resp.hovered() { Color32::from_gray(48) } else { Color32::from_rgb(40, 41, 45) };
        ui.painter().rect_filled(rect.shrink(2.0), CornerRadius::same(4), bg);
        let img = Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::splat(self.size));
        match thumbs.get(&a.rel.to_lowercase()) {
            Some(Some(t)) => {
                egui::Image::new((t.id, img.size())).paint_at(ui, img);
            }
            Some(None) => icons::paint(ui.painter(), a.category.icon(), img.center(), self.size * 0.4, Color32::from_gray(110)),
            None => {
                wanted.push((a.rel.clone(), a.kind == Kind::Spline));
                ui.painter().rect_filled(img.shrink(self.size * 0.3), CornerRadius::same(3), Color32::from_gray(55));
            }
        }
        if a.from_content {
            ui.painter().circle_filled(img.right_top() + Vec2::new(-6.0, 6.0), 4.0, Color32::from_rgb(232, 120, 40));
        }
        let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 4.0, img.bottom() + 2.0), rect.right_bottom() - Vec2::new(4.0, 2.0));
        let galley = ui.painter().layout(a.name.clone(), egui::FontId::proportional(11.0), Color32::from_gray(220), text_rect.width());
        ui.painter().with_clip_rect(text_rect).galley(text_rect.min, galley, Color32::WHITE);
        let resp = resp.on_hover_text(format!("{}\n{}", a.name, a.rel));
        if let Some((rel, true)) = &ed.reveal {
            if rel.eq_ignore_ascii_case(&a.rel) {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
        }
        if resp.clicked() {
            // choosing is using: the tool is ready at once (the button in the details stays for re-arming)
            self.picked = Some(a.rel.clone());
            use_asset(ed, a);
        }
        if resp.drag_started() && a.kind == Kind::Object {
            self.dragging = Some(a.rel.clone());
        }
        resp.context_menu(|ui| {
            if ui.button(icons::labelled(Icon::Star, tr("Favourite on / off", "Избранное вкл / выкл"))).clicked() {
                actions.push(Action::ToggleFavourite(a.rel.clone()));
                ui.close();
            }
            if ui.button(tr("Show in the file manager", "Показать в файловом менеджере")).clicked() {
                actions.push(Action::RevealAsset(a.rel.clone()));
                ui.close();
            }
        });
    }

    fn details(&mut self, ui: &mut egui::Ui, a: &Asset, ed: &mut Editor, thumbs: &HashMap<String, Option<Thumb>>, favourites: &[String], actions: &mut Vec<Action>) {
        if let Some(Some(t)) = thumbs.get(&a.rel.to_lowercase()) {
            ui.add(egui::Image::new((t.id, Vec2::splat(150.0))));
        }
        ui.label(RichText::new(&a.name).strong().color(Color32::WHITE));
        ui.label(RichText::new(&a.rel).small().color(Color32::from_gray(140)));
        ui.label(RichText::new(format!("{} · {}", a.category.label(), if a.from_content { tr("content folder (mod, import, own)", "папка контента (мод, импорт, своё)") } else { tr("OMSI 2", "OMSI 2") })).small());
        if a.kind == Kind::Spline && a.half_width > 0.0 {
            ui.label(RichText::new(format!("{} {:.1} m", tr("width", "ширина"), a.half_width * 2.0)).small());
        }
        ui.add_space(4.0);
        let use_label = if a.kind == Kind::Spline { tr("Build with it", "Строить им") } else { tr("Place it", "Поставить") };
        if ui.button(icons::labelled(if a.kind == Kind::Spline { Icon::Road } else { Icon::Objects }, use_label)).clicked() {
            use_asset(ed, a);
        }
        let starred = favourites.iter().any(|f| f.eq_ignore_ascii_case(&a.rel));
        if ui.button(icons::labelled(Icon::Star, if starred { tr("Remove from favourites", "Убрать из избранного") } else { tr("Add to favourites", "В избранное") })).clicked() {
            actions.push(Action::ToggleFavourite(a.rel.clone()));
        }
        if ui.button(icons::labelled(Icon::Layers, tr("PBR material…", "PBR-материал…"))).clicked() {
            actions.push(if a.kind == Kind::Spline { Action::PbrForSpline(a.rel.clone()) } else { Action::PbrForObject(a.rel.clone()) });
        }
        if ui.button(icons::labelled(Icon::Open, tr("Show in the file manager", "Показать в файловом менеджере"))).clicked() {
            actions.push(Action::RevealAsset(a.rel.clone()));
        }
        if a.from_content && ui.button(icons::labelled(Icon::Delete, tr("Delete…", "Удалить…"))).clicked() {
            self.confirm_delete = Some(a.rel.clone());
        }
        if matches!(a.category, Category::Technical) {
            ui.label(RichText::new(tr("Not seen in the game (a helper).", "В игре не видно (служебный).")).small().color(Color32::from_gray(140)));
        }
    }

    fn confirm(&mut self, ctx: &egui::Context, lib: &Library, actions: &mut Vec<Action>) {
        let Some(rel) = self.confirm_delete.clone() else { return };
        let name = lib.find(&rel).map(|a| a.name.clone()).unwrap_or_else(|| rel.clone());
        let mut close = false;
        egui::Window::new(tr("Delete the asset?", "Удалить ассет?")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            ui.label(format!("{name}\n{rel}"));
            ui.label(RichText::new(tr("Its folder goes into openMapper's bin in the content folder (openMapper\\bin), from where it can be put back by hand. Maps that use it show it missing.", "Его папка переносится в корзину openMapper в папке контента (openMapper\\bin), откуда её можно вернуть вручную. Карты, где он стоит, покажут его отсутствующим.")).small());
            ui.horizontal(|ui| {
                if ui.button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked() {
                    actions.push(Action::DeleteAsset(rel.clone()));
                    self.picked = None;
                    close = true;
                }
                if ui.button(tr("Cancel", "Отмена")).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.confirm_delete = None;
        }
        let _ = Stroke::NONE;
    }
}

/// Put an asset to use: objects go to the Objects tool, splines to the Roads tool (AI paths
/// to Game logic).
pub fn use_asset(ed: &mut Editor, a: &Asset) {
    match a.kind {
        Kind::Object => {
            ed.set_tool(Tool::Objects);
            ed.object_type = Some(a.rel.clone());
            ed.place_heading = 0.0;
        }
        Kind::Spline if a.category == Category::Technical && (a.lanes.0 > 0 || a.lanes.1 > 0) => {
            ed.set_tool(Tool::Logic);
            ed.set_logic(crate::editor::LogicMode::Paths);
            ed.path_edit = false;
            ed.set_path_type(a.rel.clone());
        }
        Kind::Spline => {
            ed.set_tool(Tool::Roads);
            ed.road_type = Some(a.rel.clone());
        }
    }
}
