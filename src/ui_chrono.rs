//! Chrono scenarios (Map menu): the map's changes over the years (`Chrono/<scenario>`, each
//! with its `Chrono.cfg`: from when to when, lines it takes off, its own tickets), and the
//! view shown as on any day, every scenario active then laid over the map as the game does.

use std::path::{Path, PathBuf};

use egui::{Color32, RichText};

use crate::doc::Sel;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::pal;

struct Scenario {
    dir: PathBuf,
    /// Its own name and description (`Chrono_ENG.dsc`), as written.
    title: String,
    description: String,
    name: String,
    cfg: omsi_map::ailists::ChronoCfg,
    /// Its description in the language shown, else the first it has.
    about: String,
    tiles: usize,
}

#[derive(Default)]
pub struct ChronoWindow {
    pub open: bool,
    /// The day the view shows (YYYYMMDD; 0: not chosen yet, today).
    date: i32,
    list: Option<Vec<Scenario>>,
    /// The scenario whose properties are open, and their numbers while edited.
    editing: Option<Edit>,
    /// A new scenario's name, as typed.
    new_name: String,
    /// The one object selected and its texts as they are to read in the scenario.
    relabel: Option<(i64, Vec<String>)>,
    /// The one road or object selected, its rules in the map and as they are to be from the
    /// scenario's day.
    rules: Option<(Sel, Vec<omsi_map::MapRule>, Vec<omsi_map::MapRule>)>,
    /// The scenario whose own things are in the map being edited.
    session: Option<crate::chrono_edit::Session>,
    /// The check hook ran.
    checked: bool,
    /// The scenario whose timetable is in the Lines window being edited.
    tt_session: Option<crate::chrono_tt::TtSession>,
    /// The money systems and ticket packs there are (read once).
    packs: Option<(Vec<String>, Vec<String>)>,
    /// The scenario whose AI lists are in the Traffic window being edited.
    lists_session: Option<crate::chrono_tt::ListsSession>,
    /// The patches of the scenario being edited (read again after each change).
    contents: Option<Vec<(crate::doc::TileKey, String, omsi_map::Tile)>>,
}

struct Edit {
    dir: PathBuf,
    title: String,
    description: String,
    start: i32,
    end: i32,
    lines: Vec<String>,
    /// Its own ticket pack and money from its day (None: the map's).
    tickets: Option<String>,
    money: Option<String>,
}

/// Chrono.cfg with a one-line block (`[ticketpack]`, `[moneysystem]`) set, or taken away.
pub fn cfg_with(old: &str, key: &str, value: Option<&str>) -> String {
    let mut keep = String::new();
    let mut skip = false;
    for l in old.replace("\r\n", "\n").lines() {
        let t = l.trim();
        if t.starts_with('[') {
            skip = t.eq_ignore_ascii_case(&format!("[{key}]"));
        } else if t.is_empty() && skip {
            skip = false;
            continue;
        }
        if !skip {
            keep.push_str(l);
            keep.push('\n');
        }
    }
    match value.filter(|v| !v.trim().is_empty()) {
        Some(v) => format!("[{key}]\n{v}\n\n{keep}"),
        None => keep,
    }
}

/// Chrono.cfg with its dates and the lines it takes off set: those blocks are written anew
/// at the top, everything else it has stays as it is.
pub fn chrono_cfg_text(old: &str, start: i32, end: i32, lines: &[String]) -> String {
    let mut keep = String::new();
    let mut skip = false;
    for l in old.replace("\r\n", "\n").lines() {
        let t = l.trim();
        if t.starts_with('[') {
            skip = ["[startdate]", "[enddate]", "[deactivate_lines]"].iter().any(|k| t.eq_ignore_ascii_case(k));
        } else if t.is_empty() {
            // (a block ends at an empty line; [deactivate_lines] holds no empty one)
            if skip {
                skip = false;
                continue;
            }
        }
        if !skip {
            keep.push_str(l);
            keep.push('\n');
        }
    }
    let mut o = format!("[startdate]\n{start}\n\n");
    if end > 0 {
        o.push_str(&format!("[enddate]\n{end}\n\n"));
    }
    if !lines.is_empty() {
        o.push_str(&format!("[deactivate_lines]\n{}\n{}\n\n", lines.len(), lines.join("\n")));
    }
    o.push_str(keep.trim_start_matches('\n'));
    o
}

/// A date field (day, month, year) of YYYYMMDD; 0 stays "none" until set.
fn date_edit(ui: &mut egui::Ui, v: &mut i32) {
    let (mut y, mut m, mut d) = if *v > 0 { (*v / 10_000, *v / 100 % 100, *v % 100) } else { (2000, 1, 1) };
    let a = ui.add(egui::DragValue::new(&mut d).range(1..=31)).changed();
    ui.label(".");
    let b = ui.add(egui::DragValue::new(&mut m).range(1..=12)).changed();
    ui.label(".");
    let c = ui.add(egui::DragValue::new(&mut y).range(1900..=2100)).changed();
    if a || b || c {
        *v = y * 10_000 + m * 100 + d;
    }
}

fn scan(root: &Path, out: &mut Vec<Scenario>) {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort_by_key(|d| d.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default());
    for d in dirs {
        let cfg = crate::doc::safe::own_path(&d, "Chrono.cfg");
        if let Ok(f) = omsi_cfg::CfgFile::read(&cfg) {
            let files: Vec<String> = std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            // (English, else whichever language it has)
            let dsc = files.iter().find(|n| n.eq_ignore_ascii_case("Chrono_ENG.dsc")).or_else(|| files.iter().find(|n| n.to_lowercase().ends_with(".dsc")));
            let (title, about) = dsc.and_then(|n| std::fs::read(d.join(n)).ok()).map(|b| crate::descriptions::parse(&omsi_cfg::decode_text(&b))).unwrap_or_default();
            let about_all = about.trim().to_string();
            let about = about.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
            let tiles = files.iter().filter(|n| n.to_lowercase().ends_with(".map")).count();
            let folder = d.file_name().unwrap_or_default().to_string_lossy().to_string();
            out.push(Scenario { dir: d.clone(), title: title.trim().to_string(), description: about_all.clone(), name: if title.trim().is_empty() { folder } else { format!("{} · {folder}", title.trim()) }, cfg: omsi_map::ailists::parse_chrono_cfg(&f), about, tiles });
        }
        scan(&d, out);
    }
}

/// Where scenario `dir` (under `root`) is in the map's own folder `map_dir`: the map's
/// chrono folders are brought there first when they are still the installation's (`source`),
/// which is never written to.
fn own_dir(dir: &Path, root: &Path, map_dir: &Path, source: Option<&Path>) -> anyhow::Result<PathBuf> {
    let rel = dir.strip_prefix(root).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from("Chrono").join(dir.file_name().unwrap_or_default()));
    if root != map_dir {
        if let Some(src) = source {
            let from = omsi_cfg::resolve_path(src, "Chrono");
            if from.is_dir() {
                crate::doc::save::copy_missing(&from, &map_dir.join("Chrono"))?;
            }
        }
    }
    Ok(map_dir.join(rel))
}

/// Write a scenario's Chrono.cfg and description (in the map's own folder: `own_dir`).
/// Where the scenario now is.
fn write_scenario(e: &Edit, root: &Path, map_dir: &Path, source: Option<&Path>) -> anyhow::Result<PathBuf> {
    let dir = own_dir(&e.dir, root, map_dir, source)?;
    std::fs::create_dir_all(&dir)?;
    let cfg = crate::doc::safe::own_path(&dir, "Chrono.cfg");
    let old = std::fs::read(&cfg).map(|b| omsi_cfg::decode_text(&b)).unwrap_or_default();
    let text = chrono_cfg_text(&old, e.start, e.end, &e.lines);
    let text = cfg_with(&text, "ticketpack", e.tickets.as_deref());
    let text = cfg_with(&text, "moneysystem", e.money.as_deref());
    crate::doc::safe::write_atomic(&cfg, &crate::timetable::encode(&text))?;
    if !e.title.is_empty() || !e.description.is_empty() {
        let dsc = crate::doc::safe::own_path(&dir, "Chrono_ENG.dsc");
        crate::doc::safe::write_atomic(&dsc, &crate::timetable::encode(&crate::descriptions::text(&e.title, &e.description)))?;
    }
    Ok(dir)
}

/// YYYYMMDD as "29.05.1994".
fn date_text(d: i32) -> String {
    if d <= 0 {
        return "–".into();
    }
    format!("{:02}.{:02}.{}", d % 100, d / 100 % 100, d / 10_000)
}

impl ChronoWindow {
    pub fn in_session(&self) -> bool {
        self.session.is_some() || self.tt_session.is_some() || self.lists_session.is_some()
    }

    /// The scenario's AI lists written back and the map's put in their place again.
    fn end_lists_session(&mut self, ed: &mut Editor) {
        let Some(s) = self.lists_session.take() else { return };
        ed.history.commit();
        let Some(doc) = ed.doc.as_mut() else { return };
        match crate::chrono_tt::end_lists(doc, s) {
            Ok(n) => ed.toast(format!("{} {n}", tr("The scenario's AI lists are written, groups changed:", "Списки ИИ сценария записаны, групп изменено:"))),
            Err(e) => ed.error(format!("{}: {e}", tr("Could not write the scenario", "Не удалось записать сценарий"))),
        }
        ed.history.clear();
    }

    /// The scenario's timetable written back and the map's put in its place again.
    fn end_tt_session(&mut self, ed: &mut Editor) {
        let Some(s) = self.tt_session.take() else { return };
        ed.history.commit();
        let Some(doc) = ed.doc.as_mut() else { return };
        match crate::chrono_tt::end(doc, s) {
            Ok(n) => ed.toast(format!("{} {n}", tr("The scenario's timetable is written, files:", "Расписание сценария записано, файлов:"))),
            Err(e) => ed.error(format!("{}: {e}", tr("Could not write the scenario", "Не удалось записать сценарий"))),
        }
        ed.history.clear();
        self.list = None;
    }

    /// The scenario's own things out of the map and back into it (before a save, a test
    /// drive, another map). Undo cannot reach across it.
    pub fn end_session(&mut self, ed: &mut Editor) {
        self.end_tt_session(ed);
        self.end_lists_session(ed);
        let Some(s) = self.session.take() else { return };
        ed.history.commit();
        let Some(doc) = ed.doc.as_mut() else { return };
        match crate::chrono_edit::end(doc, s) {
            Ok(n) => ed.toast(format!("{} {n}", tr("Back in the scenario:", "Снова в сценарии:"))),
            Err(e) => ed.error(format!("{}: {e}", tr("Could not write the scenario", "Не удалось записать сценарий"))),
        }
        ed.history.clear();
        ed.select(None);
        if let Some(doc) = ed.doc.as_mut() {
            doc.show_chrono(if self.date > 0 { self.date } else { crate::doc::today_code() });
        }
        self.contents = None;
    }

    /// Scenario `dir`'s own things into the map, to select, move and change as any others;
    /// what is placed meanwhile is the scenario's too.
    fn begin_session(&mut self, ed: &mut Editor, dir: &Path, root: &Path) {
        self.end_session(ed);
        ed.history.commit();
        let Some(doc) = ed.doc.as_mut() else { return };
        let own = match own_dir(dir, root, &doc.dir, doc.source.as_deref()) {
            Ok(d) => d,
            Err(e) => {
                ed.error(format!("{e}"));
                return;
            }
        };
        let (s, n) = crate::chrono_edit::begin(doc, &own);
        doc.show_chrono_without(self.date, Some(&own));
        self.session = Some(s);
        ed.history.clear();
        ed.toast(format!("{} {n}. {}", tr("The scenario's things are on the map:", "Вещи сценария на карте:"), tr("what you place now is the scenario's too", "то, что вы ставите сейчас, тоже в сценарии")));
    }

    /// A change of the selection made in scenario `dir` (0 appears, 1 is gone, 2 becomes
    /// `file`). How many things it touched.
    fn apply(&mut self, ed: &mut Editor, what: u8, dir: PathBuf, root: &Path, file: Option<String>) -> anyhow::Result<usize> {
        use crate::chrono_edit::{by_tile, write, Change};
        let sels = ed.selection();
        let Some(doc) = ed.doc.as_ref() else { return Ok(0) };
        let dir = own_dir(&dir, root, &doc.dir, doc.source.as_deref())?;
        let mut n = 0;
        for (k, tile_file, items) in by_tile(doc, &sels) {
            let ids: Vec<(i64, bool)> = items.iter().map(|s| match *s {
                Sel::Object(_, id) => (id, false),
                Sel::Spline(_, id) => (id, true),
                Sel::Attached(_, id) => (id, false),
            }).collect();
            n += ids.len();
            let c = match what {
                0 => {
                    let Some(t) = doc.tiles.get(&k) else { continue };
                    let mut patch = omsi_map::Tile::default();
                    for (id, is_spline) in &ids {
                        if *is_spline {
                            patch.splines.extend(t.tile.splines.iter().filter(|s| s.id == *id && !s.deleted).cloned());
                        } else {
                            patch.objects.extend(t.tile.objects.iter().filter(|o| o.id == *id).cloned());
                            // (and what hangs on it)
                            patch.attach_objects.extend(t.tile.attach_objects.iter().filter(|a| a.parent_id == Some(*id) || a.id == *id).cloned());
                        }
                    }
                    // (in the tile's own measure, as the file keeps it)
                    crate::doc::unfit_from_world_grid(&mut patch, k.1);
                    Change::Add(patch)
                }
                1 => Change::Delete(ids),
                _ => Change::Retype(ids, file.clone().unwrap_or_default()),
            };
            write(&dir, &tile_file, &c)?;
        }
        if what == 0 {
            // (out of the map: it is the scenario's now)
            ed.delete_selected();
        }
        Ok(n)
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &crate::library::Library, roots: Option<&crate::install::Roots>, actions: &mut Vec<crate::ui::Action>) {
        // (while a scenario's things are on the map, a bar says so, window open or not)
        if let Some(dir) = self.session.as_ref().map(|s| s.dir.clone()).or_else(|| self.tt_session.as_ref().map(|s| s.dir.clone())).or_else(|| self.lists_session.as_ref().map(|s| s.dir.clone())) {
            let name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
            let mut done = false;
            egui::Area::new(egui::Id::new("chrono session")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(120, 60, 20, 235)).corner_radius(4).inner_margin(egui::Margin::symmetric(10, 5)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} «{name}»", tr("Editing the scenario", "Правка сценария"))).color(Color32::WHITE).strong());
                        done = ui.button(icons::labelled(Icon::Check, tr("Done", "Готово"))).clicked();
                    });
                });
            });
            if done {
                self.end_session(ed);
            }
        }
        if !self.open {
            self.list = None;
            return;
        }
        let Some(doc) = ed.doc.as_ref() else { return };
        if self.date == 0 && self.editing.is_none() {
            self.date = crate::doc::today_code();
        }
        let list = self.list.get_or_insert_with(|| {
            let mut v = Vec::new();
            scan(&crate::doc::safe::own_path(&doc.chrono_root(), "Chrono"), &mut v);
            v
        });
        // OPENMAPPER_CHRONO_EDIT=<n>: the properties of scenario n open (checks)
        if self.editing.is_none() && self.contents.is_none() {
            if let Some(s) = std::env::var("OPENMAPPER_CHRONO_EDIT").ok().and_then(|n| n.parse::<usize>().ok()).and_then(|n| list.get(n)) {
                self.editing = Some(Edit { dir: s.dir.clone(), title: s.title.clone(), description: s.description.clone(), start: s.cfg.start_date, end: s.cfg.end_date, lines: s.cfg.deactivate_lines.clone(), tickets: s.cfg.ticket_pack.clone(), money: s.cfg.money_system.clone() });
                self.date = s.cfg.start_date.max(self.date.min(1));
            }
        }
        let mut date = self.date;
        let mut open = self.open;
        let lines: Vec<String> = doc.timetable.lines.iter().map(|l| l.name.clone()).collect();
        let root = doc.chrono_root();
        let (map_dir, source) = (doc.dir.clone(), doc.source.clone());
        let mut edit: Option<Edit> = None;
        let mut write = false;
        let mut close_edit = false;
        let mut new_scenario: Option<String> = None;
        let mut change: Option<u8> = None;
        let mut session_toggle = false;
        let mut tt_toggle = false;
        let packs = self.packs.get_or_insert_with(|| roots.map(|r| (crate::ui_mapsettings::content_files(r, "Money", "cfg"), crate::ui_mapsettings::content_files(r, "TicketPacks", "otp"))).unwrap_or_default()).clone();
        let mut lists_toggle = false;
        let mut relabel: Option<(i64, Vec<String>)> = None;
        // (the texts of the one object selected, kept while it stays selected)
        let one = match ed.selection().as_slice() {
            [Sel::Object(k, id)] => crate::selection::any_object(doc, *k, *id).filter(|o| !o.extra.is_empty()).map(|o| (*id, o.extra.clone())),
            _ => None,
        };
        if one.as_ref().map(|o| o.0) != self.relabel.as_ref().map(|r| r.0) {
            self.relabel = one;
        }
        let ruled = match ed.selection().as_slice() {
            [Sel::Spline(k, id)] => doc.spline(*k, *id).map(|x| (Sel::Spline(*k, *id), x.rules.clone())),
            [Sel::Object(k, id)] => doc.object(*k, *id).map(|o| (Sel::Object(*k, *id), o.rules.clone())),
            _ => None,
        };
        if ruled.as_ref().map(|r| r.0) != self.rules.as_ref().map(|r| r.0) {
            self.rules = ruled.map(|(s, r)| (s, r.clone(), r));
        }
        let mut write_rules: Option<(Sel, Vec<omsi_map::MapRule>, Vec<omsi_map::MapRule>)> = None;
        let mut look: Option<glam::DVec2> = None;
        let mut take: Option<(crate::doc::TileKey, String, crate::chrono_edit::Part, bool)> = None;
        // the map's things a change names: their names and places
        let mut map_names: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        let mut map_places: std::collections::HashMap<i64, glam::DVec2> = std::collections::HashMap::new();
        if self.editing.is_some() {
            for (k, t) in &doc.tiles {
                let org = crate::doc::tile_origin(*k);
                for o in t.tile.objects.iter() {
                    map_names.insert(o.id, lib.find(&o.file).map(|a| a.name.clone()).unwrap_or_else(|| o.file.clone()));
                    map_places.insert(o.id, org + glam::DVec2::new(o.pos[0], o.pos[1]));
                }
                for sp in t.tile.splines.iter() {
                    map_names.insert(sp.id, lib.find(&sp.file).map(|a| a.name.clone()).unwrap_or_else(|| sp.file.clone()));
                    map_places.insert(sp.id, org + glam::DVec2::new(sp.pos[0], sp.pos[1]));
                }
            }
        }
        let doc_ref: &crate::doc::Document = doc;
        let n_sel = ed.selection().len();
        let catalogue = ed.object_type.clone();
        egui::Window::new(icons::labelled(Icon::Chrono, tr("Chrono scenarios", "Хроно-сценарии"))).id(egui::Id::new("chrono")).open(&mut open).default_pos([330.0, 90.0]).default_size([500.0, 520.0]).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("How the map changes over the years: each scenario from its first day lays its own tiles, lines and tickets over the map. The view shows the map as on the day chosen here; what you edit is always the map itself.", "Как карта меняется со временем: каждый сценарий с первого дня накладывает на карту свои тайлы, линии и билеты. Вид показывает карту на выбранный здесь день; правится всегда сама карта.")).small().color(pal().text_dim));
            ui.horizontal(|ui| {
                ui.label(tr("Show the map as on", "Показать карту на"));
                let (mut y, mut m, mut d) = (date / 10_000, date / 100 % 100, date % 100);
                let a = ui.add(egui::DragValue::new(&mut d).range(1..=31)).changed();
                ui.label(".");
                let b = ui.add(egui::DragValue::new(&mut m).range(1..=12)).changed();
                ui.label(".");
                let c = ui.add(egui::DragValue::new(&mut y).range(1900..=2100)).changed();
                if a || b || c {
                    date = y * 10_000 + m * 100 + d;
                }
                if ui.button(icons::labelled(Icon::Calendar, tr("Today", "Сегодня"))).clicked() {
                    date = crate::doc::today_code();
                }
            });
            let active = list.iter().filter(|s| s.cfg.active_on(date)).count();
            ui.label(RichText::new(format!("{} {} / {}", tr("Active on that day:", "Действуют в этот день:"), active, list.len())).color(pal().text_dim));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text(tr("New scenario's name", "Имя нового сценария")).desired_width(200.0));
                if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new(icons::labelled(Icon::Add, tr("New scenario", "Новый сценарий")))).on_hover_text(tr("A scenario starting on the day shown, after the others", "Сценарий с показанного дня, после остальных")).clicked() {
                    new_scenario = Some(self.new_name.trim().to_string());
                }
            });
            if let Some(e) = &mut self.editing {
                ui.separator();
                ui.label(RichText::new(e.dir.file_name().unwrap_or_default().to_string_lossy()).strong().color(pal().accent));
                egui::Grid::new("chrono edit").num_columns(2).spacing(egui::Vec2::new(8.0, 5.0)).show(ui, |ui| {
                    crate::ui::prop_row(ui, tr("Name", "Название"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut e.title).desired_width(260.0));
                    });
                    crate::ui::prop_row(ui, tr("From", "С"), |ui| {
                        ui.horizontal(|ui| date_edit(ui, &mut e.start));
                    });
                    crate::ui::prop_row(ui, tr("To", "По"), |ui| {
                        ui.horizontal(|ui| {
                            let mut on = e.end > 0;
                            if ui.checkbox(&mut on, "").changed() {
                                e.end = if on { e.start.max(20000101) + 10_000 } else { 0 };
                            }
                            if on {
                                date_edit(ui, &mut e.end);
                            } else {
                                ui.label(RichText::new(tr("for good", "навсегда")).color(pal().text_dim));
                            }
                        });
                    });
                    for (label, v, list) in [(tr("Own tickets", "Свои билеты"), &mut e.tickets, &packs.1), (tr("Own money", "Своя валюта"), &mut e.money, &packs.0)] {
                        crate::ui::prop_row(ui, label, |ui| {
                            let shown = v.clone().unwrap_or_else(|| tr("the map's", "как у карты").to_string());
                            egui::ComboBox::from_id_salt(label).selected_text(shown).width(260.0).show_ui(ui, |ui| {
                                ui.selectable_value(v, None, tr("the map's", "как у карты"));
                                for f in list.iter() {
                                    ui.selectable_value(v, Some(f.clone()), f);
                                }
                            });
                        });
                    }
                    crate::ui::prop_row(ui, tr("Takes off", "Снимает линии"), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut all: Vec<String> = lines.clone();
                            for l in &e.lines {
                                if !all.iter().any(|x| x.eq_ignore_ascii_case(l)) {
                                    all.push(l.clone());
                                }
                            }
                            for l in all {
                                let mut on = e.lines.iter().any(|x| x.eq_ignore_ascii_case(&l));
                                if ui.checkbox(&mut on, &l).changed() {
                                    if on {
                                        e.lines.push(l.clone());
                                    } else {
                                        e.lines.retain(|x| !x.eq_ignore_ascii_case(&l));
                                    }
                                }
                            }
                        });
                    });
                });
                ui.add(egui::TextEdit::multiline(&mut e.description).hint_text(tr("What changes (shown where the date is chosen)", "Что меняется (видно при выборе даты)")).desired_rows(2).desired_width(f32::INFINITY));
                ui.add_space(4.0);
                ui.label(RichText::new(tr("From its first day, the selection in the view:", "С его первого дня — выделенное в окне вида:")).strong());
                ui.horizontal_wrapped(|ui| {
                    let any = n_sel > 0;
                    if ui.add_enabled(any, egui::Button::new(icons::labelled(Icon::Add, tr("appears", "появляется")))).on_hover_text(tr("Taken out of the map into the scenario: it is there from the scenario's first day on (place it first, then press this)", "Убирается из карты в сценарий: появляется с первого дня сценария (сначала поставьте, потом нажмите)")).clicked() {
                        change = Some(0);
                    }
                    if ui.add_enabled(any, egui::Button::new(icons::labelled(Icon::Delete, tr("is gone", "исчезает")))).on_hover_text(tr("Stays in the map, gone from the scenario's first day on", "Остаётся в карте, исчезает с первого дня сценария")).clicked() {
                        change = Some(1);
                    }
                    if let Some(n) = &catalogue {
                        if ui.add_enabled(any, egui::Button::new(icons::labelled(Icon::Swap, &format!("{} «{n}»", tr("becomes", "становится"))))).clicked() {
                            change = Some(2);
                        }
                    }
                });
                // one object with texts: they may read otherwise from the scenario's day
                if let Some((id, texts)) = &mut self.relabel {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("its texts become", "его тексты становятся"));
                        for t in texts.iter_mut() {
                            ui.add(egui::TextEdit::singleline(t).desired_width(140.0));
                        }
                        if ui.button(icons::labelled(Icon::Save, tr("Write", "Записать"))).clicked() {
                            relabel = Some((*id, texts.clone()));
                        }
                    });
                }
                // one road or object: its path rules may be others from the scenario's day
                if let Some((sel, old, new)) = &mut self.rules {
                    egui::CollapsingHeader::new(format!("{} ({} -> {})", tr("Its path rules from that day", "Его правила путей с этого дня"), old.len(), new.len())).id_salt("chrono rules").show(ui, |ui| {
                        let mut remove = None;
                        for (i, r) in new.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut r.path_index).range(0..=64).prefix("#"));
                                egui::ComboBox::from_id_salt(("chrono rule", i)).selected_text(crate::ui_edit::rule_name(&r.kind)).width(104.0).show_ui(ui, |ui| {
                                    for (k, _) in crate::ui_edit::RULES {
                                        ui.selectable_value(&mut r.kind, k.to_string(), crate::ui_edit::rule_name(k));
                                    }
                                });
                                if crate::ui_edit::RULES.iter().any(|(k, v)| *k == r.kind && *v) {
                                    ui.add(egui::DragValue::new(&mut r.value).speed(0.1).max_decimals(2));
                                }
                                if icons::small(ui, Icon::Delete, tr("Delete", "Удалить")).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            new.remove(i);
                        }
                        ui.horizontal(|ui| {
                            if ui.button(icons::labelled(Icon::Add, tr("Add a rule", "Добавить правило"))).clicked() {
                                new.push(omsi_map::MapRule { path_index: 0, kind: "speedlimit".into(), value: 30.0, extra: 0.0, kill: false });
                            }
                            if ui.add_enabled(new != old, egui::Button::new(icons::labelled(Icon::Save, tr("Write", "Записать")))).clicked() {
                                write_rules = Some((*sel, old.clone(), new.clone()));
                            }
                        });
                    });
                }
                ui.label(RichText::new(tr("Written into the scenario at once (undo does not take it out of the scenario).", "Записывается в сценарий сразу (отмена не убирает это из сценария).")).small().color(pal().text_dim));
                // what the scenario has, tile by tile
                // (the map's own copy once it has one)
                let contents = self.contents.get_or_insert_with(|| {
                    let rel = e.dir.strip_prefix(&root).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from("Chrono").join(e.dir.file_name().unwrap_or_default()));
                    let own = doc_ref.dir.join(rel);
                    crate::chrono_edit::patches(if own.is_dir() { &own } else { &e.dir }, doc_ref)
                });
                let total: usize = contents.iter().map(|(_, _, t)| t.objects.len() + t.splines.iter().filter(|s| !s.deleted).count() + t.chrono_changes.len()).sum();
                egui::CollapsingHeader::new(RichText::new(format!("{} ({total})", tr("What it changes", "Что он меняет"))).strong()).id_salt("chrono contents").default_open(std::env::var_os("OPENMAPPER_CHRONO_EDIT").is_some()).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("chrono contents list").max_height(220.0).show(ui, |ui| {
                        let name = |f: &str| lib.find(f).map(|a| a.name.clone()).unwrap_or_else(|| f.rsplit('\\').next().unwrap_or(f).to_string());
                        for (k, file, t) in contents.iter() {
                            ui.label(RichText::new(format!("{} {}, {} · {file}", tr("Tile", "Тайл"), k.0, k.1)).small().color(pal().text_dim));
                            let org = crate::doc::tile_origin(*k);
                            let mut row = |ui: &mut egui::Ui, mark: &str, text: String, at: Option<glam::DVec2>, part: crate::chrono_edit::Part, back: bool| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(mark).monospace().color(pal().accent));
                                    ui.label(text);
                                    if let Some(p) = at {
                                        if icons::small(ui, Icon::Focus, tr("Go there", "Перейти туда")).clicked() {
                                            look = Some(p);
                                        }
                                    }
                                    if back && ui.small_button(tr("into the map", "в карту")).on_hover_text(tr("Out of the scenario into the map itself: there on every day", "Из сценария в саму карту: будет во все дни")).clicked() {
                                        take = Some((*k, file.clone(), part, true));
                                    }
                                    if icons::small(ui, Icon::Delete, tr("Take it out of the scenario", "Убрать из сценария")).clicked() {
                                        take = Some((*k, file.clone(), part, false));
                                    }
                                });
                            };
                            for o in &t.objects {
                                row(ui, "+", name(&o.file), Some(org + glam::DVec2::new(o.pos[0], o.pos[1])), crate::chrono_edit::Part::Object(o.id), true);
                            }
                            for (i, sp) in t.splines.iter().enumerate().filter(|(_, s)| !s.deleted) {
                                let rows = t.spline_attachments.iter().any(|a| a.spline_index == i as i32);
                                row(ui, "+", name(&sp.file), Some(org + glam::DVec2::new(sp.pos[0], sp.pos[1])), crate::chrono_edit::Part::Spline(sp.id), !rows);
                            }
                            for (i, c) in t.chrono_changes.iter().enumerate() {
                                let what = map_names.get(&c.id).cloned().unwrap_or_else(|| format!("#{}", c.id));
                                let (mark, text) = if c.delete {
                                    ("-", what)
                                } else if let Some(f) = &c.new_type {
                                    (">", format!("{what} -> {}", name(f)))
                                } else if c.relabel.is_some() {
                                    ("*", format!("{what}: {}", tr("new texts", "новые тексты")))
                                } else {
                                    ("*", format!("{what}: {} {}", c.rules.len(), tr("rules", "правил")))
                                };
                                row(ui, mark, text, map_places.get(&c.id).copied(), crate::chrono_edit::Part::Change(i), false);
                            }
                        }
                        if contents.is_empty() {
                            ui.label(RichText::new(tr("Nothing yet.", "Пока ничего.")).color(pal().text_dim));
                        }
                    });
                });
                ui.add_space(4.0);
                if self.session.is_some() {
                    ui.label(RichText::new(tr("Its own things are on the map: select, move, change and place as usual. The map's own things stay the map's.", "Его вещи на карте: выделяйте, двигайте, меняйте и ставьте как обычно. Вещи самой карты остаются в карте.")).color(pal().accent));
                    if ui.button(icons::labelled(Icon::Check, tr("Done: back into the scenario", "Готово: обратно в сценарий"))).clicked() {
                        session_toggle = true;
                    }
                } else if ui.button(icons::labelled(Icon::Edit, tr("Edit its own things on the map", "Править его вещи на карте"))).on_hover_text(tr("What it brings in is put on the map to work with as with anything else; Done writes it back (undo starts afresh)", "То, что он добавляет, ставится на карту, с ним работают как с любым другим; «Готово» записывает обратно (история отмены начинается заново)")).clicked() {
                    session_toggle = true;
                }
                if self.tt_session.is_some() {
                    ui.label(RichText::new(tr("The Lines window shows its timetable (the map's with its own laid over): lines, trips and tours edited there are the scenario's; a line of the map taken away is taken off from its day.", "Окно линий показывает его расписание (карты с наложенным своим): правки линий, рейсов и выходов там — сценария; убранная линия карты снимается с его дня.")).color(pal().accent));
                    ui.horizontal(|ui| {
                        if ui.button(icons::labelled(Icon::Check, tr("Done with its timetable", "Готово с расписанием"))).clicked() {
                            tt_toggle = true;
                        }
                        if ui.button(icons::labelled(Icon::Bus, tr("Lines window", "Окно линий"))).clicked() {
                            actions.push(crate::ui::Action::OpenLines);
                        }
                    });
                } else if ui.button(icons::labelled(Icon::Edit, tr("Edit its timetable", "Править его расписание"))).on_hover_text(tr("Its own lines, trips and tours in the Lines window; Done writes only what differs from the map's (undo starts afresh)", "Его линии, рейсы и выходы в окне линий; «Готово» записывает только отличия от карты (история отмены начинается заново)")).clicked() {
                    tt_toggle = true;
                }
                if self.lists_session.is_some() {
                    ui.label(RichText::new(tr("The Traffic window shows its AI lists (the map's with its own added): vehicles and type groups added there, and a depot's own depot file, are the scenario's from its day.", "Окно трафика показывает его списки ИИ (карты с добавленным своим): добавленные там машины и группы типов и свой файл депо — сценария с его дня.")).color(pal().accent));
                    ui.horizontal(|ui| {
                        if ui.button(icons::labelled(Icon::Check, tr("Done with its AI lists", "Готово со списками ИИ"))).clicked() {
                            lists_toggle = true;
                        }
                        if ui.button(icons::labelled(Icon::Car, tr("Traffic window", "Окно трафика"))).clicked() {
                            actions.push(crate::ui::Action::OpenTraffic);
                        }
                    });
                } else if ui.button(icons::labelled(Icon::Edit, tr("Edit its AI lists", "Править его списки ИИ"))).on_hover_text(tr("Vehicles it adds to the groups and depots (OMSI's ailists_#upd.cfg); taking one away is the map's own business", "Машины, которые он добавляет в группы и депо (ailists_#upd.cfg OMSI); убирать машины — дело самой карты")).clicked() {
                    lists_toggle = true;
                }
                ui.horizontal(|ui| {
                    if ui.button(icons::labelled(Icon::Save, tr("Write the scenario", "Записать сценарий"))).clicked() {
                        write = true;
                    }
                    if ui.button(icons::labelled(Icon::Close, tr("Close", "Закрыть"))).clicked() {
                        close_edit = true;
                    }
                });
            }
            ui.separator();
            if list.is_empty() {
                ui.label(RichText::new(tr("The map has no chrono scenarios.", "У карты нет хроно-сценариев.")).color(pal().text_dim));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for s in list.iter() {
                    let on = s.cfg.active_on(date);
                    ui.horizontal(|ui| {
                        ui.label(if on { Icon::RadioOn.text(14.0).color(pal().accent) } else { Icon::RadioOff.text(14.0).color(pal().text_dim) });
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&s.name).color(if on { pal().text_strong } else { pal().text_dim }));
                            let mut facts = format!("{} {}", tr("from", "с"), date_text(s.cfg.start_date));
                            if s.cfg.end_date > 0 {
                                facts.push_str(&format!(" {} {}", tr("to", "по"), date_text(s.cfg.end_date)));
                            }
                            if s.tiles > 0 {
                                facts.push_str(&format!(" · {} {}", s.tiles, tr("tiles", "тайлов")));
                            }
                            if !s.cfg.deactivate_lines.is_empty() {
                                facts.push_str(&format!(" · {} {}", tr("takes off", "снимает"), s.cfg.deactivate_lines.join(", ")));
                            }
                            if s.cfg.ticket_pack.is_some() {
                                facts.push_str(&format!(" · {}", tr("own tickets", "свои билеты")));
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(facts).small().color(pal().text_dim));
                                if s.cfg.start_date > 0 && ui.small_button(tr("Its first day", "Первый день")).clicked() {
                                    date = s.cfg.start_date;
                                }
                                if ui.small_button(icons::labelled(Icon::Tune, tr("Properties", "Свойства"))).clicked() {
                                    edit = Some(Edit { dir: s.dir.clone(), title: s.title.clone(), description: s.description.clone(), start: s.cfg.start_date, end: s.cfg.end_date, lines: s.cfg.deactivate_lines.clone(), tickets: s.cfg.ticket_pack.clone(), money: s.cfg.money_system.clone() });
                                }
                            });
                            if !s.about.is_empty() {
                                ui.label(RichText::new(&s.about).small());
                            }
                        });
                    });
                    ui.add_space(2.0);
                }
            });
        });
        self.open = open;
        if let Some(e) = edit {
            self.editing = Some(e);
            self.contents = None;
        }
        if close_edit {
            self.editing = None;
        }
        if let Some(name) = new_scenario {
            // (numbered after the last, as the game orders them by name)
            let last = list.iter().filter_map(|s| s.dir.file_name()?.to_string_lossy().split('_').next()?.parse::<u32>().ok()).max().unwrap_or(0);
            let folder: String = format!("{:04}_{}", (last / 100 + 1) * 100, name).chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
            let rel = Path::new("Chrono").join(&folder);
            self.editing = Some(Edit { dir: root.join(&rel), title: name, description: String::new(), start: date, end: 0, lines: Vec::new(), tickets: None, money: None });
            self.new_name.clear();
            write = true;
        }
        if lists_toggle {
            if self.lists_session.is_some() {
                self.end_lists_session(ed);
            } else if let Some(e) = &self.editing {
                let dir = e.dir.clone();
                ed.history.commit();
                if let Some(doc) = ed.doc.as_mut() {
                    match own_dir(&dir, &root, &doc.dir, doc.source.as_deref()) {
                        Ok(own) => {
                            self.lists_session = Some(crate::chrono_tt::begin_lists(doc, &own));
                            ed.history.clear();
                            actions.push(crate::ui::Action::OpenTraffic);
                        }
                        Err(err) => ed.error(format!("{err}")),
                    }
                }
            }
        }
        if tt_toggle {
            if self.tt_session.is_some() {
                self.end_tt_session(ed);
            } else if let Some(e) = &self.editing {
                let dir = e.dir.clone();
                ed.history.commit();
                if let Some(doc) = ed.doc.as_mut() {
                    match own_dir(&dir, &root, &doc.dir, doc.source.as_deref()) {
                        Ok(own) => {
                            self.tt_session = Some(crate::chrono_tt::begin(doc, &own));
                            ed.history.clear();
                            actions.push(crate::ui::Action::OpenLines);
                        }
                        Err(err) => ed.error(format!("{err}")),
                    }
                }
            }
        }
        if session_toggle {
            if self.session.is_some() {
                self.end_session(ed);
            } else if let Some(e) = &self.editing {
                let dir = e.dir.clone();
                self.begin_session(ed, &dir, &root);
            }
        }
        // OPENMAPPER_CHRONO_SESSION=1 with OPENMAPPER_CHRONO_EDIT: its things on the map (checks)
        if self.session.is_none() && !self.checked && std::env::var_os("OPENMAPPER_CHRONO_SESSION").is_some() {
            self.checked = true;
            if let Some(e) = &self.editing {
                let dir = e.dir.clone();
                self.begin_session(ed, &dir, &root);
            }
        }
        if let Some(p) = look {
            actions.push(crate::ui::Action::LookAt(p));
        }
        if let (Some((k, file, part, back)), Some(e)) = (take, &self.editing) {
            // (what goes back into the map, as the patch has it, onto the map's grid)
            let moved = self.contents.as_ref().and_then(|c| c.iter().find(|x| x.0 == k && x.1 == file)).map(|(_, _, t)| match part {
                crate::chrono_edit::Part::Object(id) => (t.objects.iter().filter(|o| o.id == id).cloned().collect::<Vec<_>>(), t.attach_objects.iter().filter(|o| o.parent_id == Some(id)).cloned().collect::<Vec<_>>(), Vec::new()),
                crate::chrono_edit::Part::Spline(id) => (Vec::new(), Vec::new(), t.splines.iter().filter(|s| s.id == id).cloned().collect()),
                crate::chrono_edit::Part::Change(_) => (Vec::new(), Vec::new(), Vec::new()),
            });
            let own = ed.doc.as_ref().map(|d| own_dir(&e.dir, &root, &d.dir, d.source.as_deref()));
            let done = match own {
                Some(Ok(dir)) => crate::chrono_edit::take_out(&dir, &file, part),
                Some(Err(err)) => Err(err),
                None => Ok(()),
            };
            match done {
                Ok(()) => {
                    if let (true, Some((o, h, s))) = (back, moved) {
                        ed.put_back(k, o, h, s);
                    }
                }
                Err(err) => ed.error(format!("{err}")),
            }
            self.contents = None;
            if let Some(doc) = ed.doc.as_mut() {
                doc.show_chrono(self.date);
            }
        }
        if let (Some((sel, old, new)), Some(e)) = (write_rules, &self.editing) {
            let file = ed.doc.as_ref().and_then(|d| crate::chrono_edit::by_tile(d, &[sel]).into_iter().next()).map(|t| t.1);
            let (id, is_spline) = match sel {
                Sel::Object(_, id) => (id, false),
                Sel::Spline(_, id) => (id, true),
                Sel::Attached(_, id) => (id, false),
            };
            let res = (|| -> anyhow::Result<()> {
                let doc = ed.doc.as_ref().ok_or_else(|| anyhow::anyhow!("no map"))?;
                let dir = own_dir(&e.dir, &root, &doc.dir, doc.source.as_deref())?;
                crate::chrono_edit::write(&dir, &file.unwrap_or_default(), &crate::chrono_edit::Change::Rules(id, is_spline, old, new))?;
                Ok(())
            })();
            match res {
                Ok(()) => ed.toast(tr("Written into the scenario", "Записано в сценарий")),
                Err(err) => ed.error(format!("{err}")),
            }
            self.rules = None;
            self.contents = None;
            if let Some(doc) = ed.doc.as_mut() {
                doc.show_chrono(self.date);
            }
        }
        if let (Some((id, texts)), Some(e)) = (relabel, &self.editing) {
            let file = ed.doc.as_ref().and_then(|d| crate::chrono_edit::by_tile(d, &ed.selection()).into_iter().next()).map(|t| t.1);
            let res = (|| -> anyhow::Result<()> {
                let doc = ed.doc.as_ref().ok_or_else(|| anyhow::anyhow!("no map"))?;
                let dir = own_dir(&e.dir, &root, &doc.dir, doc.source.as_deref())?;
                crate::chrono_edit::write(&dir, &file.unwrap_or_default(), &crate::chrono_edit::Change::Relabel(id, texts))?;
                Ok(())
            })();
            match res {
                Ok(()) => ed.toast(tr("Written into the scenario", "Записано в сценарий")),
                Err(err) => ed.error(format!("{err}")),
            }
            self.contents = None;
            if let Some(doc) = ed.doc.as_mut() {
                doc.show_chrono(self.date);
            }
        }
        if let (Some(c), Some(e)) = (change, &self.editing) {
            match self.apply(ed, c, e.dir.clone(), &root, catalogue.clone()) {
                Ok(n) => ed.toast(format!("{} {n}", tr("Written into the scenario:", "Записано в сценарий:"))),
                Err(err) => ed.error(format!("{}: {err}", tr("Could not write the scenario", "Не удалось записать сценарий"))),
            }
            self.list = None;
            self.contents = None;
            if let Some(doc) = ed.doc.as_mut() {
                doc.show_chrono(self.date);
            }
        }
        if write {
            if let Some(e) = &mut self.editing {
                match write_scenario(e, &root, &map_dir, source.as_deref()) {
                    Ok(dir) => {
                        e.dir = dir;
                        ed.toast(tr("Scenario written", "Сценарий записан"));
                        self.list = None;
                        // (the view as on its first day)
                        date = e.start;
                        self.date = 0;
                    }
                    Err(err) => ed.error(format!("{}: {err}", tr("Could not write the scenario", "Не удалось записать сценарий"))),
                }
            }
        }
        if date != self.date {
            self.date = date;
            if let Some(doc) = ed.doc.as_mut() {
                let n = doc.show_chrono(date);
                ed.toast(format!("{} {} ({} {n})", tr("The map as on", "Карта на"), date_text(date), tr("scenarios:", "сценариев:")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scenario added to a map still read from the installation: the map's own copy gets
    /// every scenario, the new one with them; the installation is not written to.
    #[test]
    fn a_new_scenario_goes_into_the_maps_own_copy() {
        let base = std::env::temp_dir().join(format!("openmapper-chrono-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src, own) = (base.join("install"), base.join("content"));
        std::fs::create_dir_all(src.join("Chrono").join("0100_A")).unwrap();
        std::fs::write(src.join("Chrono").join("0100_A").join("Chrono.cfg"), "[startdate]\r\n19900101\r\n\r\n").unwrap();
        let e = Edit { dir: src.join("Chrono").join("0200_B"), title: "B".into(), description: "new".into(), start: 19950101, end: 0, lines: vec![], tickets: None, money: None };
        let dir = write_scenario(&e, &src, &own, Some(&src)).unwrap();
        assert_eq!(dir, own.join("Chrono").join("0200_B"));
        assert_eq!(omsi_map::active_chrono_dirs(&own, 19960101).len(), 2);
        assert!(!src.join("Chrono").join("0200_B").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An object of Grundorf (read from the installation) taken away in one of its scenarios:
    /// the change lands in the map's own copy, and the view as on that day has it gone.
    #[test]
    fn a_change_in_a_scenario_of_an_installation_map() {
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let cfg = omsi.join("maps").join("Grundorf").join("global.cfg");
        if !cfg.is_file() {
            return;
        }
        let _map = crate::timetable::tests::one_map();
        let content = std::env::temp_dir().join(format!("openmapper-chrono-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&content);
        let roots = crate::install::Roots { omsi: omsi.clone(), content: content.clone() };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let root = doc.chrono_root();
        let scenario = std::fs::read_dir(root.join("Chrono")).unwrap().flatten().map(|e| e.path()).find(|p| p.is_dir()).unwrap();
        let start = omsi_map::ailists::parse_chrono_cfg(&omsi_cfg::CfgFile::read(scenario.join("Chrono.cfg")).unwrap()).start_date;
        let (k, id) = doc.tiles.iter().find_map(|(k, t)| t.tile.objects.first().map(|o| (*k, o.id))).unwrap();
        let mut ed = Editor::default();
        ed.open(doc);
        ed.select(Some(Sel::Object(k, id)));
        let mut w = ChronoWindow::default();
        assert_eq!(w.apply(&mut ed, 1, scenario.clone(), &root, None).unwrap(), 1);
        let own = content.join("maps").join("Grundorf").join("Chrono").join(scenario.file_name().unwrap());
        assert!(own.join("Chrono.cfg").is_file(), "the scenarios came along");
        let doc = ed.doc.as_mut().unwrap();
        assert_eq!(doc.chrono_root(), doc.dir);
        doc.show_chrono(start.max(19000101));
        let shown = doc.tiles[&k].display(k.1);
        assert!(!shown.objects.iter().any(|o| o.id == id), "gone from the scenario's day");
        assert!(doc.tiles[&k].tile.objects.iter().any(|o| o.id == id), "still in the map");
        let _ = std::fs::remove_dir_all(&content);
    }

    /// Grundorf's road works scenario edited on the map: its barriers come onto the map, one
    /// is moved, and afterwards they are the scenario's again (in the map's own copy), moved.
    #[test]
    fn a_scenarios_things_are_edited_on_the_map() {
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let cfg = omsi.join("maps").join("Grundorf").join("global.cfg");
        let scen = omsi.join("maps").join("Grundorf").join("Chrono").join("0100_BaustelleNordspitze");
        if !scen.is_dir() {
            return;
        }
        let _map = crate::timetable::tests::one_map();
        let content = std::env::temp_dir().join(format!("openmapper-chrono-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&content);
        let roots = crate::install::Roots { omsi: omsi.clone(), content: content.clone() };
        let doc = crate::doc::Document::load(&roots, &cfg, |_| {}).unwrap();
        let root = doc.chrono_root();
        let before: usize = doc.tiles.values().map(|t| t.tile.objects.len()).sum();
        let mut ed = Editor::default();
        ed.open(doc);
        let mut w = ChronoWindow { date: 20000101, ..Default::default() };
        w.begin_session(&mut ed, &scen, &root);
        assert!(w.in_session());
        let doc = ed.doc.as_mut().unwrap();
        let during: usize = doc.tiles.values().map(|t| t.tile.objects.len()).sum();
        assert!(during > before, "its things are on the map");
        let theirs: std::collections::HashSet<i64> = omsi_map::Tile::load(&scen.join("tile_1_1.map")).unwrap().objects.iter().map(|o| o.id).collect();
        let (k, o) = doc.tiles.iter_mut().find_map(|(k, t)| t.tile.objects.iter_mut().find(|o| theirs.contains(&o.id)).map(|o| (*k, o))).unwrap();
        let id = o.id;
        o.pos[0] += 1.5;
        let x = o.pos[0];
        w.end_session(&mut ed);
        let doc = ed.doc.as_ref().unwrap();
        assert_eq!(doc.tiles.values().map(|t| t.tile.objects.len()).sum::<usize>(), before, "the map as it was");
        let own = content.join("maps").join("Grundorf").join("Chrono").join("0100_BaustelleNordspitze");
        let file = doc.global.tiles.iter().find(|t| (t.x, t.y) == k).unwrap().file.clone();
        let patch = omsi_map::Tile::load(&own.join(&file)).unwrap();
        assert!((patch.objects.iter().find(|o| o.id == id).unwrap().pos[0] - x).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&content);
    }

    #[test]
    fn a_ticket_pack_of_its_own_is_set_and_taken_away() {
        let with = cfg_with("[startdate]\n19900101\n\n", "ticketpack", Some("TicketPacks\\B\\B.otp"));
        let c = omsi_map::ailists::parse_chrono_cfg(&omsi_cfg::CfgFile::from_str("Chrono.cfg", &with));
        assert_eq!((c.start_date, c.ticket_pack.as_deref()), (19900101, Some("TicketPacks\\B\\B.otp")));
        let without = cfg_with(&with, "ticketpack", None);
        assert!(omsi_map::ailists::parse_chrono_cfg(&omsi_cfg::CfgFile::from_str("Chrono.cfg", &without)).ticket_pack.is_none());
    }

    #[test]
    fn a_chrono_cfg_keeps_what_it_does_not_set() {
        let old = "[startdate]\r\n19891114\r\n\r\n[enddate]\r\n19900929\r\n\r\n[ticketpack]\r\nTicketPacks\\X.otp\r\n\r\n[deactivate_lines]\r\n1\r\nRelais\r\n\r\n";
        let new = chrono_cfg_text(old, 19900101, 0, &["E".to_string()]);
        let c = omsi_map::ailists::parse_chrono_cfg(&omsi_cfg::CfgFile::from_str("Chrono.cfg", &new));
        assert_eq!((c.start_date, c.end_date), (19900101, 0));
        assert_eq!(c.deactivate_lines, vec!["E".to_string()]);
        assert_eq!(c.ticket_pack.as_deref(), Some("TicketPacks\\X.otp"));
    }
}
