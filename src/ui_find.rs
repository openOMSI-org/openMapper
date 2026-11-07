//! Find in map (Edit menu, Ctrl+F): stops and start points by name, a point by its
//! coordinates, and every kind of object and spline the map uses, by name, file or IDCode,
//! with how many there are; select them all, go from one to the next, or put the
//! catalogue's choice in their place.

use std::collections::HashMap;

use egui::{Color32, RichText};
use glam::DVec2;

use crate::doc::{tile_origin, Document, Sel, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::ui::{Action, ACCENT, TEXT_DIM};

/// One kind of thing the map uses, and where.
struct Group {
    file: String,
    spline: bool,
    at: Vec<(Sel, DVec2)>,
}

#[derive(Default)]
pub struct Find {
    pub open: bool,
    query: String,
    /// The map's groups, made again when its tiles change (the key says what they were made of).
    groups: Vec<Group>,
    key: u64,
    /// When they were made (while a drag changes the map each frame, now and then only).
    made: Option<std::time::Instant>,
    /// The group gone through with Next, and how far.
    walk: Option<(String, usize)>,
    focus: bool,
}

fn key_of(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    doc.dir.hash(&mut h);
    for (k, t) in &doc.tiles {
        (k, t.content_rev).hash(&mut h);
    }
    h.finish()
}

fn gather(doc: &Document) -> Vec<Group> {
    let mut by: HashMap<(String, bool), Group> = HashMap::new();
    let mut keys: Vec<&TileKey> = doc.tiles.keys().collect();
    keys.sort();
    for k in keys {
        let t = &doc.tiles[k];
        let org = tile_origin(*k);
        let mut add = |file: &str, spline: bool, sel: Sel, p: DVec2| {
            by.entry((file.to_lowercase(), spline)).or_insert_with(|| Group { file: file.to_string(), spline, at: Vec::new() }).at.push((sel, p));
        };
        for o in &t.tile.objects {
            add(&o.file, false, Sel::Object(*k, o.id), org + DVec2::new(o.pos[0], o.pos[1]));
        }
        // (one hanging on another is found where its parent stands)
        for o in &t.tile.attach_objects {
            let p = o.parent_id.and_then(|p| t.tile.objects.iter().find(|x| x.id == p)).map(|x| org + DVec2::new(x.pos[0], x.pos[1])).unwrap_or(org);
            add(&o.file, false, Sel::Object(*k, o.id), p);
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted && !s.file.trim().is_empty()) {
            let c = omsi_geometry::SplineCurve::from_map(s, org);
            add(&s.file, true, Sel::Spline(*k, s.id), c.point_at(s.length * 0.5).truncate());
        }
    }
    let mut v: Vec<Group> = by.into_values().collect();
    v.sort_by(|a, b| b.at.len().cmp(&a.at.len()).then_with(|| a.file.to_lowercase().cmp(&b.file.to_lowercase())));
    v
}

impl Find {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
    }

    /// Open with a search typed (`OPENMAPPER_FIND`, for checks).
    pub fn show_with(&mut self, q: &str) {
        self.show();
        self.query = q.to_string();
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let Some(doc) = ed.doc.as_ref() else { return };
        let key = key_of(doc);
        if key != self.key && self.made.map(|t| t.elapsed().as_millis() > 300).unwrap_or(true) {
            self.groups = gather(doc);
            self.key = key;
            self.made = Some(std::time::Instant::now());
        } else if key != self.key {
            ctx.request_repaint_after(std::time::Duration::from_millis(320));
        }
        // the named places (stops, start points) and where they are
        let q = self.query.trim().to_lowercase();
        let mut places: Vec<(String, &str, Sel, DVec2)> = Vec::new();
        if q.chars().count() >= 2 && q.parse::<i64>().is_err() {
            let at = |id: i64| self.groups.iter().flat_map(|g| g.at.iter()).find(|(s, _)| matches!(s, Sel::Object(_, i) if *i == id)).copied();
            let named = doc.timetable.stops.iter().map(|s| (s.name.trim().to_string(), s.object_id, tr("stop", "остановка"))).chain(doc.global.entry_points.iter().map(|e| (e.name.trim().to_string(), e.object_id, tr("start point", "точка старта"))));
            for (name, id, kind) in named {
                let Some((sel, pos)) = at(id).filter(|_| name.to_lowercase().contains(&q)) else { continue };
                // (a stop's two sides are one place)
                if !places.iter().any(|p| p.0 == name && p.1 == kind && (p.3 - pos).length() < 60.0) {
                    places.push((name, kind, sel, pos));
                }
            }
        }
        // a place by its coordinates
        let point: Option<DVec2> = {
            let v: Vec<f64> = q.split([',', ';', ' ']).filter(|x| !x.is_empty()).map(|x| x.parse::<f64>()).collect::<Result<_, _>>().unwrap_or_default();
            (v.len() == 2).then(|| DVec2::new(v[0], v[1]))
        };
        let mut look: Option<DVec2> = None;
        let name_of = |g: &Group| lib.find(&g.file).map(|a| a.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| g.file.rsplit(['\\', '/']).next().unwrap_or(&g.file).to_string());
        let mut open = self.open;
        // (what to do once the window is drawn: the editor is borrowed by the list)
        let mut select: Option<Vec<Sel>> = None;
        let mut go: Option<(Sel, DVec2)> = None;
        let mut replace: Option<(String, bool, String)> = None;
        egui::Window::new(icons::labelled(Icon::Search, tr("Find in map", "Поиск по карте"))).id(egui::Id::new("find in map")).open(&mut open).default_pos([320.0, 100.0]).default_size([440.0, 480.0]).resizable(true).show(ctx, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(tr("Name, file, IDCode, stop or x, y", "Название, файл, IDCode, остановка или x, y")).desired_width(f32::INFINITY));
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            if let Some(p) = point {
                ui.horizontal(|ui| {
                    ui.label(format!("{} {:.1}, {:.1}", tr("The point", "Точка"), p.x, p.y));
                    if ui.button(icons::labelled(Icon::Focus, tr("Go to", "Перейти"))).clicked() {
                        look = Some(p);
                    }
                });
                ui.separator();
            }
            if !places.is_empty() {
                egui::ScrollArea::vertical().id_salt("places").max_height(150.0).show(ui, |ui| {
                    for (name, kind, sel, p) in places.iter().take(50) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).color(Color32::WHITE));
                            ui.label(RichText::new(*kind).small().color(TEXT_DIM));
                            if ui.small_button(icons::labelled(Icon::Focus, tr("Go to", "Перейти"))).clicked() {
                                go = Some((*sel, *p));
                            }
                        });
                    }
                });
                ui.separator();
            }
            // an IDCode: straight to it
            if let Ok(id) = q.parse::<i64>() {
                let hit = self.groups.iter().flat_map(|g| g.at.iter().map(move |a| (g, a))).find(|(_, (s, _))| match s {
                    Sel::Object(_, i) | Sel::Spline(_, i) | Sel::Attached(_, i) => *i == id,
                });
                match hit {
                    Some((g, &(s, p))) => {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} {id}: {}", if g.spline { tr("Spline", "Сплайн") } else { tr("Object", "Объект") }, name_of(g)));
                            if ui.button(icons::labelled(Icon::Focus, tr("Go to", "Перейти"))).clicked() {
                                go = Some((s, p));
                            }
                        });
                    }
                    None => {
                        ui.label(RichText::new(tr("Nothing on the map has this IDCode.", "На карте нет ничего с таким IDCode.")).color(TEXT_DIM));
                    }
                }
                ui.separator();
            }
            let shown: Vec<&Group> = self.groups.iter().filter(|g| q.is_empty() || g.file.to_lowercase().contains(&q) || name_of(g).to_lowercase().contains(&q)).collect();
            let total: usize = shown.iter().map(|g| g.at.len()).sum();
            ui.label(RichText::new(format!("{} {} · {} {total}", tr("kinds", "видов"), shown.len(), tr("on the map", "на карте"))).small().color(TEXT_DIM));
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for g in shown.iter().take(400) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{:>5}", g.at.len())).monospace().color(ACCENT));
                        ui.vertical(|ui| {
                            ui.label(RichText::new(name_of(g)).color(Color32::WHITE));
                            crate::ui::long_label(ui, &g.file, egui::TextStyle::Small, TEXT_DIM);
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button(tr("Select all", "Выбрать все")).clicked() {
                            select = Some(g.at.iter().map(|a| a.0).collect());
                        }
                        if ui.small_button(icons::labelled(Icon::Focus, tr("Next", "Следующий"))).on_hover_text(tr("Select the next one and go there", "Выбрать следующий и перейти к нему")).clicked() {
                            let n = match &self.walk {
                                Some((f, i)) if f.eq_ignore_ascii_case(&g.file) => (i + 1) % g.at.len(),
                                _ => 0,
                            };
                            self.walk = Some((g.file.clone(), n));
                            go = Some(g.at[n]);
                        }
                        let with = if g.spline { ed.road_type.clone() } else { ed.object_type.clone() };
                        if let Some(w) = with.filter(|w| !w.eq_ignore_ascii_case(&g.file)) {
                            let n = lib.find(&w).map(|a| a.name.clone()).unwrap_or_else(|| w.clone());
                            let tip = if g.spline { tr("Every one of these becomes the road type chosen in the Roads tool", "Все такие станут типом дороги, выбранным в инструменте «Дороги»") } else { tr("Every one of these becomes the object chosen in the catalogue", "Все такие станут объектом, выбранным в каталоге") };
                            if ui.small_button(icons::labelled(Icon::Swap, &format!("{} «{n}»", tr("Replace with", "Заменить на")))).on_hover_text(tip).clicked() {
                                replace = Some((g.file.clone(), g.spline, w));
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                if shown.len() > 400 {
                    ui.label(RichText::new(tr("More: type to narrow the list", "Есть ещё: уточните поиск")).color(TEXT_DIM));
                }
            });
        });
        self.open = open;
        if let Some(v) = select {
            ed.set_selection(v);
        }
        if let Some(p) = look {
            actions.push(Action::LookAt(p));
        }
        if let Some((s, p)) = go {
            ed.select(Some(s));
            actions.push(Action::LookAt(p));
        }
        if let Some((from, spline, to)) = replace {
            let n = ed.replace_everywhere(&from, &to, spline);
            ed.toast(format!("{} {n}", tr("Replaced:", "Заменено:")));
        }
    }
}
