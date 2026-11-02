//! Autosave: every few minutes the changed tiles and the global settings go into a folder of
//! their own (never into the map). Saving, closing or quitting normally clears it; found at
//! the next start, it means openMapper ended without saving, and the changes are offered
//! back.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{Document, TileKey};
use crate::i18n::tr;

/// How often the changes are kept (`OPENMAPPER_AUTOSAVE=<seconds>` for checks).
fn every() -> Duration {
    Duration::from_secs(std::env::var("OPENMAPPER_AUTOSAVE").ok().and_then(|s| s.parse().ok()).unwrap_or(180))
}

const META: &str = "autosave.json";

/// What an autosave is of.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    /// The map's name, for the question.
    pub name: String,
    /// The map's own folder (where it is saved to).
    pub dir: PathBuf,
    /// Where it was read from when that is another (the installation's copy, the template).
    pub source: Option<PathBuf>,
    pub fresh: bool,
    /// The tiles kept (the changed ones).
    pub tiles: Vec<TileKey>,
    /// The timetable (`TTData`) and the traffic files kept whole (they had changed).
    #[serde(default)]
    pub timetable: bool,
    #[serde(default)]
    pub traffic: bool,
    /// The descriptions in other languages, kept whole.
    #[serde(default)]
    pub descriptions: bool,
    /// Seconds since 1970.
    pub time: u64,
}

impl Meta {
    /// How long ago, in words.
    pub fn age(&self) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let min = now.saturating_sub(self.time) / 60;
        match min {
            0 => tr("just now", "только что").to_string(),
            m if m < 120 => format!("{m} {}", tr("min ago", "мин назад")),
            m => format!("{} {}", m / 60, tr("h ago", "ч назад")),
        }
    }
}

/// The autosave folder of a content folder.
pub fn dir(content: &Path) -> PathBuf {
    content.join("openMapper").join("autosave")
}

/// Keep the map's changes (the changed tiles, the global settings). The folder is written
/// beside the old one and takes its place whole, so one cut short leaves the last.
#[cfg(test)]
pub fn write(doc: &Document, content: &Path) -> anyhow::Result<()> {
    prepare(doc, content).run()
}

/// Counts up whenever the autosave is cleared: one still being written then is dropped.
static EPOCH: std::sync::Mutex<u64> = std::sync::Mutex::new(0);

/// An autosave made ready on the main thread (what it writes, in memory), to be written
/// on another.
pub struct Job {
    protected_source: bool,
    target: PathBuf,
    ops: Vec<super::safe::Op>,
    meta: Meta,
    epoch: u64,
}

/// What an autosave needs of the map, copied on the main thread (only the changed tiles are
/// copied: cheap) so the text of it can be made on another and the window never waits.
pub struct Snapshot {
    doc: Document,
    tiles: std::collections::HashSet<TileKey>,
    epoch: u64,
}

/// Copy what an autosave of the map writes.
pub fn snapshot(doc: &Document) -> Snapshot {
    let copy = Document {
        global: doc.global.clone(),
        _publisher_mounts: doc._publisher_mounts.clone(),
        _publisher_protected_source: doc._publisher_protected_source,
        tiles: doc.tiles.iter().filter(|(_, t)| t.dirty).map(|(k, t)| (*k, t.clone())).collect(),
        order: doc.order.clone(),
        dir: doc.dir.clone(),
        source: doc.source.clone(),
        global_dirty: doc.global_dirty,
        fresh: doc.fresh,
        global_rev: doc.global_rev,
        timetable: if doc.timetable.dirty { doc.timetable.clone() } else { Default::default() },
        traffic: if doc.traffic.dirty { doc.traffic.clone() } else { Default::default() },
        descriptions: if doc.descriptions.dirty { doc.descriptions.clone() } else { Default::default() },
        groups: Vec::new(),
    };
    Snapshot { doc: copy, tiles: doc.tiles.keys().copied().collect(), epoch: *EPOCH.lock().unwrap_or_else(|e| e.into_inner()) }
}

/// What an autosave of the map writes (the tiles' text is made here; nothing is written).
#[cfg(test)]
pub fn prepare(doc: &Document, content: &Path) -> Job {
    let known: std::collections::HashSet<TileKey> = doc.tiles.keys().copied().collect();
    let epoch = *EPOCH.lock().unwrap_or_else(|e| e.into_inner());
    prepare_from(doc, &known, epoch, content)
}

impl Snapshot {
    pub fn prepare(&self, content: &Path) -> Job {
        prepare_from(&self.doc, &self.tiles, self.epoch, content)
    }
}

fn prepare_from(doc: &Document, known: &std::collections::HashSet<TileKey>, epoch: u64, content: &Path) -> Job {
    let target = dir(content);
    let fresh_dir = target.with_extension("new");
    let (ops, tiles) = super::save::plan_known(doc, &fresh_dir, false, &|k| known.contains(k));
    let mut ops: Vec<_> = ops.into_iter().filter(|o| matches!(o, super::safe::Op::Write(..))).collect();
    // the timetable and the traffic through their own writers, every file of them (a copy
    // marked as all changed: the map's own stays as it is)
    if doc.timetable.dirty {
        let mut tt = doc.timetable.clone();
        tt.restored(&Default::default());
        ops.extend(crate::timetable::plan(&mut tt, &fresh_dir, &doc.order).into_iter().filter(|o| matches!(o, super::safe::Op::Write(..))));
    }
    if doc.traffic.dirty {
        let mut t = doc.traffic.clone();
        t.restored();
        ops.extend(crate::traffic::plan(&t, &fresh_dir).into_iter().filter(|o| matches!(o, super::safe::Op::Write(..))));
    }
    if doc.descriptions.dirty {
        let mut d = doc.descriptions.clone();
        d.restored(&Default::default());
        ops.extend(d.plan(&fresh_dir).into_iter().filter(|o| matches!(o, super::safe::Op::Write(..))));
    }
    let meta = Meta {
        name: doc.display_name().to_string(),
        dir: doc.dir.clone(),
        source: doc.source.clone(),
        fresh: doc.fresh,
        tiles,
        timetable: doc.timetable.dirty,
        traffic: doc.traffic.dirty,
        descriptions: doc.descriptions.dirty,
        time: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    Job {
        target,
        ops,
        meta,
        epoch,
        protected_source: doc._publisher_protected_source,
    }
}

impl Job {
    pub fn run(self) -> anyhow::Result<()> {
        if self.protected_source {
            anyhow::bail!("publisher-protected maps cannot be written to an autosave");
        }
        let fresh_dir = self.target.with_extension("new");
        let _ = std::fs::remove_dir_all(&fresh_dir);
        std::fs::create_dir_all(&fresh_dir)?;
        super::safe::commit(&self.ops)?;
        super::safe::write_atomic(&fresh_dir.join(META), serde_json::to_string_pretty(&self.meta)?.as_bytes())?;
        // (saved or dropped since it was made ready: it is not wanted)
        let epoch = EPOCH.lock().unwrap_or_else(|e| e.into_inner());
        if *epoch != self.epoch {
            let _ = std::fs::remove_dir_all(&fresh_dir);
            return Ok(());
        }
        let _ = std::fs::remove_dir_all(&self.target);
        std::fs::rename(&fresh_dir, &self.target)?;
        drop(epoch);
        Ok(())
    }
}

/// The autosave left by a session that did not end normally.
pub fn pending(content: &Path) -> Option<Meta> {
    let s = std::fs::read_to_string(dir(content).join(META)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Nothing to keep any more (saved, or the changes were dropped).
pub fn clear(content: &Path) {
    let mut epoch = EPOCH.lock().unwrap_or_else(|e| e.into_inner());
    *epoch += 1;
    let d = dir(content);
    if d.exists() {
        if let Err(e) = std::fs::remove_dir_all(&d) {
            log::warn!("{}: {e}", d.display());
        }
    }
}

/// The map with the autosaved changes laid over it: read from its folder (or where it came
/// from), then the kept global settings and tiles put in, marked as not saved yet.
pub fn restore(roots: &crate::install::Roots, meta: &Meta) -> anyhow::Result<Document> {
    let saved = dir(&roots.content);
    let base = [meta.dir.join("global.cfg"), meta.source.clone().unwrap_or_default().join("global.cfg")].into_iter().find(|p| p.is_file() && !meta.fresh);
    let mut doc = match &base {
        Some(cfg) => Document::load(roots, cfg, |_| {})?,
        // (a map never saved: all of it is in the autosave)
        None => Document::load(roots, &saved.join("global.cfg"), |_| {})?,
    };
    let global = omsi_map::GlobalCfg::load(&saved.join("global.cfg")).map_err(|e| anyhow::anyhow!("{e}"))?;
    omsi_map::configure_grid(&global);
    let listed: std::collections::HashSet<TileKey> = global.tiles.iter().map(|t| (t.x, t.y)).collect();
    for t in global.tiles.iter().filter(|t| meta.tiles.contains(&(t.x, t.y))) {
        let Some(mut td) = super::load_tile(&saved, &global, &[], t) else { continue };
        if let Some(old) = doc.tiles.get(&(t.x, t.y)) {
            td.chrono = old.chrono.clone();
        }
        td.dirty = true;
        doc.tiles.insert((t.x, t.y), td);
    }
    doc.tiles.retain(|k, _| listed.contains(k));
    doc.order = global.raw_tiles.clone();
    doc.global = global;
    doc.global_dirty = true;
    doc.global_rev = super::next_rev();
    doc.dir = meta.dir.clone();
    doc.source = meta.source.clone().or(doc.source.take()).filter(|s| *s != meta.dir);
    doc.fresh = meta.fresh;
    doc.sync_mask_slots();
    if meta.timetable {
        let mut tt = crate::timetable::Timetable::load(&saved, &doc.order);
        // (written whole on the next save; what only the map's had goes)
        tt.restored(&doc.timetable);
        doc.timetable = tt;
    }
    if meta.traffic {
        let mut t = crate::traffic::Traffic::load(&saved);
        t.restored();
        doc.traffic = t;
    }
    if meta.descriptions {
        let mut d = crate::descriptions::Descriptions::load(&saved);
        // (what only the map's had goes)
        d.restored(&doc.descriptions);
        doc.descriptions = d;
    }
    Ok(doc)
}

/// When to autosave next.
pub struct Timer {
    last: Instant,
    /// The autosave being written (on a thread of its own).
    running: Option<std::thread::JoinHandle<()>>,
}

impl Default for Timer {
    fn default() -> Self {
        Timer { last: Instant::now(), running: None }
    }
}

impl Timer {
    /// Autosave the map if it has changes and the time has come (not in the middle of an
    /// edit). Errors go to the log and are tried again later.
    pub fn tick(&mut self, doc: Option<&Document>, content: Option<&Path>, editing: bool) {
        let (Some(doc), Some(content)) = (doc, content) else { return };
        if doc._publisher_protected_source || editing || self.last.elapsed() < every() || !doc.is_dirty() || self.running.as_ref().map(|h| !h.is_finished()).unwrap_or(false) {
            return;
        }
        self.last = Instant::now();
        let t = Instant::now();
        // (only the changed tiles are copied here; their text is made and written off the
        // main thread: a big map's do not hold the window)
        let snap = snapshot(doc);
        let copied = t.elapsed().as_secs_f32() * 1000.0;
        let at = dir(content);
        let content = content.to_path_buf();
        self.running = Some(std::thread::spawn(move || match snap.prepare(&content).run() {
            Ok(()) => log::info!("autosave: {} ({copied:.0} ms to copy on the main thread, {:.0} ms in all)", at.display(), t.elapsed().as_secs_f32() * 1000.0),
            Err(e) => log::warn!("autosave: {e}"),
        }));
    }

    /// Start counting again (just saved).
    pub fn reset(&mut self) {
        self.last = Instant::now();
    }
}

/// The question at start when an autosave was left behind: Some(true) restore, Some(false)
/// drop it.
pub fn offer(ctx: &egui::Context, meta: &Meta) -> Option<bool> {
    let mut answer = None;
    egui::Window::new(tr("Unsaved changes found", "Найдены несохранённые изменения")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(format!(
            "{} «{}» ({}).",
            tr("openMapper was closed without saving the map", "openMapper был закрыт, не сохранив карту"),
            meta.name,
            meta.age()
        ));
        ui.label(egui::RichText::new(tr("The changes kept by the autosave can be brought back. The map itself was not changed.", "Изменения, сохранённые автосохранением, можно вернуть. Сама карта не изменялась.")).small().color(egui::Color32::GRAY));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(tr("Restore the changes", "Вернуть изменения")).clicked() {
                answer = Some(true);
            }
            if ui.button(tr("Drop them", "Отбросить")).clicked() {
                answer = Some(false);
            }
        });
    });
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_autosave_holds_the_changed_tiles() {
        let _map = crate::doc::one_map();
        let content = std::env::temp_dir().join(format!("openmapper-test-autosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&content);
        let mut doc = Document {
            global: Default::default(),
            _publisher_mounts: std::sync::Arc::new(Default::default()),
            _publisher_protected_source: false,
            tiles: Default::default(),
            order: Vec::new(),
            dir: content.join("maps").join("M"),
            source: None,
            global_dirty: true,
            fresh: false,
            global_rev: 0,
            timetable: Default::default(),
            traffic: Default::default(),
            descriptions: Default::default(),
            groups: Vec::new(),
        };
        doc.global.name = "M".into();
        for k in [(0, 0), (1, 0)] {
            doc.add_tile(k);
        }
        doc.tiles.get_mut(&(1, 0)).unwrap().dirty = false;
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: "Sceneryobjects\\x.sco".into(), id: 3, pos: [5.0, 6.0, 0.0], ..Default::default() });
        write(&doc, &content).unwrap();
        let meta = pending(&content).unwrap();
        assert_eq!(meta.tiles, vec![(0, 0)]);
        let d = dir(&content);
        assert!(d.join("tile_0_0.map").is_file() && !d.join("tile_1_0.map").exists() && d.join("global.cfg").is_file());
        // the tile reads back with what was on it
        let t = omsi_map::Tile::load(&d.join("tile_0_0.map")).unwrap();
        assert_eq!(t.objects.len(), 1);
        clear(&content);
        assert!(pending(&content).is_none());
        let _ = std::fs::remove_dir_all(&content);
    }
}
