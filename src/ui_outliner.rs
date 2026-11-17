//! The Objects panel beside the Select tool: everything on the map as a tree of the folders
//! its files lie in (Sceneryobjects › Buildings_MC › …), each kind with how many there are
//! and each one of them a click away, as a scene's hierarchy in Unity.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use egui::RichText;
use glam::DVec2;

use crate::doc::{tile_origin, Document, Sel, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::ui::Action;
use crate::theme::pal;

/// How many of one kind are listed one by one (the rest are counted).
const LIST: usize = 200;

/// One thing on the map.
#[derive(Clone)]
struct Item {
    sel: Sel,
    at: DVec2,
    /// A stop's or a start point's name, else empty.
    name: String,
}

/// A folder: its subfolders and the kinds of thing (by file) in it.
#[derive(Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    /// File (as the map names it) → the things of it.
    files: BTreeMap<String, Vec<Item>>,
    count: usize,
}

impl Folder {
    fn add(&mut self, file: &str, item: Item) {
        self.count += 1;
        let parts: Vec<&str> = file.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
        let mut f = self;
        for p in &parts[..parts.len().saturating_sub(1)] {
            // (folders of one name in either case are one)
            let key = f.folders.keys().find(|k| k.eq_ignore_ascii_case(p)).cloned().unwrap_or_else(|| p.to_string());
            f = f.folders.entry(key).or_default();
            f.count += 1;
        }
        let key = f.files.keys().find(|k| k.eq_ignore_ascii_case(file)).cloned().unwrap_or_else(|| file.to_string());
        f.files.entry(key).or_default().push(item);
    }
}

#[derive(Default)]
pub struct Outliner {
    /// The Select panel shows the objects (else the tools).
    pub open: bool,
    search: String,
    key: u64,
    root: Folder,
    /// What the search keeps of `root`, and for which map state and search.
    view: Folder,
    view_for: Option<(u64, String)>,
}

impl Outliner {
    /// Open with `search` typed (checks without a person at the screen).
    pub fn show_with(&mut self, search: &str) {
        self.open = true;
        self.search = search.to_string();
    }

    /// The tree made again when the map's content changed.
    fn refresh(&mut self, doc: &Document) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        doc.dir.hash(&mut h);
        for (k, t) in &doc.tiles {
            k.hash(&mut h);
            t.content_rev.hash(&mut h);
        }
        doc.global.entry_points.len().hash(&mut h);
        let key = h.finish();
        if key == self.key {
            return;
        }
        self.key = key;
        let mut root = Folder::default();
        let mut keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        keys.sort();
        for k in keys {
            let t = &doc.tiles[&k];
            let org = tile_origin(k);
            for o in t.tile.objects.iter().chain(&t.tile.attach_objects) {
                let name = if o.flag == 7 {
                    o.extra.first().map(|n| n.replace('@', " ")).unwrap_or_default()
                } else {
                    doc.global.entry_points.iter().find(|e| e.object_id == o.id).map(|e| e.name.clone()).unwrap_or_default()
                };
                root.add(&o.file, Item { sel: Sel::Object(k, o.id), at: org + DVec2::new(o.pos[0], o.pos[1]), name });
            }
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                root.add(&s.file, Item { sel: Sel::Spline(k, s.id), at: org + DVec2::new(s.pos[0], s.pos[1]), name: String::new() });
            }
            // (each [splineAttachement] record is a thing of its own)
            for a in &t.tile.spline_attachments {
                let at = crate::selection::attachment_first(doc, k, a.id).map(|f| f.pose.pos.truncate()).unwrap_or(org);
                root.add(&a.file, Item { sel: Sel::Attached(k, a.id), at, name: a.strings.first().cloned().unwrap_or_default() });
            }
        }
        self.root = root;
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, ed: &mut Editor, lib: &Library, actions: &mut Vec<Action>) {
        let Some(doc) = ed.doc.as_ref() else { return };
        self.refresh(doc);
        ui.horizontal(|ui| {
            ui.label(Icon::Search.text(16.0).color(pal().text_dim));
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("Name, file or IDCode", "Имя, файл или IDCode")).desired_width(f32::INFINITY));
        });
        ui.label(RichText::new(format!("{} {}", self.root.count, tr("things on the map", "объектов на карте"))).small().color(pal().text_dim));
        let q = self.search.trim().to_lowercase();
        if self.view_for.as_ref() != Some(&(self.key, q.clone())) {
            let mut view = Folder::default();
            if !q.is_empty() {
                self.root.keep(&q, lib, &mut view);
            }
            self.view = view;
            self.view_for = Some((self.key, q.clone()));
        }
        let shown = if q.is_empty() { &self.root } else { &self.view };
        let selected = ed.selection();
        // the map's groups first
        let mut pick_group: Option<usize> = None;
        if !ed.groups().is_empty() {
            let current = ed.selected_group();
            egui::CollapsingHeader::new(RichText::new(format!("{}  ({})", tr("Groups", "Группы"), ed.groups().len())).color(pal().text_strong)).id_salt("outliner groups").icon(folder_icon).default_open(true).show(ui, |ui| {
                for (i, g) in ed.groups().iter().enumerate().filter(|(_, g)| q.is_empty() || g.name.to_lowercase().contains(&q)) {
                    if ui.selectable_label(current == Some(i), format!("{}  ·  {}", g.name, g.ids.len())).on_hover_text(tr("Select the group and go there", "Выделить группу и перейти")).clicked() {
                        pick_group = Some(i);
                    }
                }
            });
        }
        let mut go: Option<(Sel, DVec2)> = None;
        let mut alike: Option<Sel> = None;
        let mut all: Option<Vec<Sel>> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            show_folder(ui, shown, "", &q, lib, &selected, &mut Picks { go: &mut go, alike: &mut alike, all: &mut all });
        });
        if let Some(g) = pick_group {
            ed.select_group(g);
            if let Some(at) = ed.doc.as_ref().and_then(|d| crate::selection::centre(d, &ed.selection())) {
                actions.push(Action::LookAt(at));
            }
        }
        if let Some(v) = all {
            ed.toast(format!("{} {}", tr("Selected:", "Выбрано:"), v.len()));
            ed.set_selection(v);
        }
        if let Some((s, at)) = go {
            ed.select(Some(s));
            actions.push(Action::LookAt(at));
        }
        if let Some(s) = alike {
            let n = ed.select_alike(s);
            ed.toast(format!("{} {n}", tr("Selected:", "Выбрано:")));
        }
    }
}

/// What of a file the search finds: all of it when its name or file matches, else the ones
/// whose own name or IDCode does.
fn matching<'a>(file: &str, items: &'a [Item], name: &str, q: &str) -> Vec<&'a Item> {
    if q.is_empty() || file.to_lowercase().contains(q) || name.to_lowercase().contains(q) {
        return items.iter().collect();
    }
    items.iter().filter(|i| i.name.to_lowercase().contains(q) || id_of(i.sel).to_string() == q).collect()
}

fn id_of(s: Sel) -> i64 {
    match s {
        Sel::Object(_, id) | Sel::Spline(_, id) | Sel::Attached(_, id) => id,
    }
}

/// How many things of a folder the search keeps.
fn found(f: &Folder) -> usize {
    f.count
}

fn name_of(lib: &Library, file: &str) -> String {
    lib.find(file).map(|a| a.name.clone()).unwrap_or_else(|| file.rsplit('\\').next().unwrap_or(file).to_string())
}

/// What a click in the tree asks for.
struct Picks<'a> {
    /// Select one and go there.
    go: &'a mut Option<(Sel, DVec2)>,
    /// Select every one of its kind.
    alike: &'a mut Option<Sel>,
    /// Select these (a folder's).
    all: &'a mut Option<Vec<Sel>>,
}

impl Folder {
    /// What the search keeps, put into `out` in its folders.
    fn keep(&self, q: &str, lib: &Library, out: &mut Folder) {
        for f in self.folders.values() {
            f.keep(q, lib, out);
        }
        for (file, items) in &self.files {
            for i in matching(file, items, &name_of(lib, file), q) {
                out.add(file, i.clone());
            }
        }
    }

    /// Everything in it.
    fn every(&self, out: &mut Vec<Sel>) {
        for f in self.folders.values() {
            f.every(out);
        }
        for items in self.files.values() {
            out.extend(items.iter().map(|i| i.sel));
        }
    }
}

fn show_folder(ui: &mut egui::Ui, f: &Folder, path: &str, q: &str, lib: &Library, selected: &[Sel], p: &mut Picks) {
    for (name, sub) in &f.folders {
        let n = found(sub);
        if n == 0 {
            continue;
        }
        let here = format!("{path}\\{name}");
        // (a search opens every folder it finds something in; the top ones are open)
        let open = if q.is_empty() { None } else { Some(true) };
        let r = egui::CollapsingHeader::new(RichText::new(format!("{name}  ({n})")).color(pal().text_strong))
            .id_salt(("outliner folder", &here))
            .icon(folder_icon)
            .default_open(path.is_empty())
            .open(open)
            .show(ui, |ui| show_folder(ui, sub, &here, q, lib, selected, p));
        r.header_response.context_menu(|ui| {
            if ui.button(icons::labelled(Icon::SelectAll, tr("Select everything in it", "Выбрать всё в ней"))).clicked() {
                let mut v = Vec::new();
                sub.every(&mut v);
                *p.all = Some(v);
                ui.close();
            }
        });
    }
    for (file, items) in &f.files {
        let name = name_of(lib, file);
        // (the tree shown is the search's already)
        let shown: Vec<&Item> = items.iter().collect();
        if shown.is_empty() {
            continue;
        }
        let has_sel = items.iter().any(|i| selected.contains(&i.sel));
        let title = RichText::new(format!(" {name}  ({})", shown.len())).color(if has_sel { pal().accent } else { pal().text });
        let open = if q.is_empty() { None } else if shown.len() <= 20 { Some(true) } else { None };
        let spline = file.to_lowercase().ends_with(".sli");
        let r = egui::CollapsingHeader::new(title).id_salt(("outliner file", file)).icon(if spline { road_icon } else { object_icon }).open(open).show(ui, |ui| {
            for it in shown.iter().take(LIST) {
                let tile = match it.sel {
                    Sel::Object(k, _) | Sel::Spline(k, _) | Sel::Attached(k, _) => k,
                };
                let label = if it.name.trim().is_empty() { format!("#{}  ·  {} {},{}", id_of(it.sel), tr("tile", "тайл"), tile.0, tile.1) } else { format!("{}  ·  #{}", it.name, id_of(it.sel)) };
                let on = selected.contains(&it.sel);
                if ui.selectable_label(on, label).on_hover_text(tr("Select it and go there", "Выбрать и перейти")).clicked() {
                    *p.go = Some((it.sel, it.at));
                }
            }
            if shown.len() > LIST {
                ui.label(RichText::new(format!("… {} {}", shown.len() - LIST, tr("more: narrow the search", "ещё: уточните поиск"))).small().color(pal().text_dim));
            }
        });
        r.header_response.on_hover_text(file.as_str()).context_menu(|ui| {
            if ui.button(icons::labelled(Icon::SelectAll, tr("Select all of them", "Выбрать все такие"))).clicked() {
                *p.alike = items.first().map(|i| i.sel);
                ui.close();
            }
            if ui.button(icons::labelled(Icon::Focus, tr("Go to the first", "Перейти к первому"))).clicked() {
                *p.go = items.first().map(|i| (i.sel, i.at));
                ui.close();
            }
        });
    }
}

/// A folder's arrow in the tree, turned as it opens.
fn folder_icon(ui: &mut egui::Ui, openness: f32, response: &egui::Response) {
    icons::paint(ui.painter(), if openness > 0.5 { Icon::ExpandMore } else { Icon::ChevronRight }, response.rect.center(), 16.0, pal().text_dim);
}

/// A kind of object in the tree: its icon, brighter while open.
fn object_icon(ui: &mut egui::Ui, openness: f32, response: &egui::Response) {
    kind_icon(ui, Icon::Objects, openness, response);
}

fn road_icon(ui: &mut egui::Ui, openness: f32, response: &egui::Response) {
    kind_icon(ui, Icon::Road, openness, response);
}

fn kind_icon(ui: &mut egui::Ui, icon: Icon, openness: f32, response: &egui::Response) {
    let c = if openness > 0.5 { pal().accent } else { pal().text_dim };
    icons::paint(ui.painter(), icon, response.rect.center(), 15.0, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn things_go_into_their_folders_and_the_search_keeps_its_own() {
        let item = |id: i64, name: &str| Item { sel: Sel::Object((0, 0), id), at: DVec2::ZERO, name: name.into() };
        let mut root = Folder::default();
        root.add("Sceneryobjects\\Trees_MC\\Tree_1.sco", item(1, ""));
        root.add("sceneryobjects\\trees_mc\\Tree_1.sco", item(2, ""));
        root.add("Sceneryobjects\\Generic\\bus_stop.sco", item(3, "Market"));
        root.add("Splines\\road.sli", item(4, ""));
        assert_eq!(root.count, 4);
        let so = &root.folders["Sceneryobjects"];
        assert_eq!((so.count, so.folders.len(), so.folders["Trees_MC"].files.len()), (3, 2, 1));
        let lib = Library::default();
        let mut v = Folder::default();
        root.keep("market", &lib, &mut v);
        let mut sels = Vec::new();
        v.every(&mut sels);
        assert_eq!(sels, [Sel::Object((0, 0), 3)]);
        let mut v = Folder::default();
        root.keep("tree_1", &lib, &mut v);
        assert_eq!(v.count, 2);
    }
}
