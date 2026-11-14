//! The map's files as they are on disk now, against the map open in the editor: what another
//! program (OMSI 2's editor, a tool of its own) changed since openMapper read or saved them,
//! read in again where nothing here waits to be saved.
//!
//! A map of the installation that openMapper saved lives on as its copy in the content folder
//! (the installation's own is never written). OMSI 2's editor keeps working on the
//! installation's, so the copy remembers how the installation's looked when it was taken
//! (`openMapper/twin.json`) and what changed there since can be brought over.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::{load_tile, next_rev, Document, TileKey};

/// The files of a map folder that make the map: by their path in it (lower case, `/`), their
/// size and when they were last written (nanoseconds since 1970).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub files: BTreeMap<String, (u64, u64)>,
}

/// The sub-folders whose files are the map's (the others are chrono scenarios, backups,
/// openMapper's own).
const FOLDERS: [&str; 3] = ["ttdata", "car_use", "texture/map"];

/// Whether a file directly in the map folder is one of the map's.
fn wanted(name: &str) -> bool {
    name.ends_with(".map") || name.contains(".map.") || [".cfg", ".txt", ".dsc", ".hof"].iter().any(|x| name.ends_with(x))
}

fn stat(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    Some((m.len(), t))
}

fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, (u64, u64)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        let p = e.path();
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if p.is_dir() {
            if FOLDERS.iter().any(|f| *f == r || f.starts_with(&format!("{r}/")) || r.starts_with(&format!("{f}/"))) {
                walk(&p, &r, out);
            }
        } else if (!rel.is_empty() && FOLDERS.iter().any(|f| r.starts_with(&format!("{f}/")))) || (rel.is_empty() && wanted(&name)) {
            if let Some(s) = stat(&p) {
                out.insert(r, s);
            }
        }
    }
}

/// Look at a map folder's files now.
pub fn stamp(dir: &Path) -> Stamp {
    let mut files = BTreeMap::new();
    walk(dir, "", &mut files);
    Stamp { files }
}

/// The files that came, went or were written since `old`.
pub fn changed_files(old: &Stamp, new: &Stamp) -> Vec<String> {
    let mut v: Vec<String> = new.files.iter().filter(|(k, s)| old.files.get(*k) != Some(*s)).map(|(k, _)| k.clone()).collect();
    v.extend(old.files.keys().filter(|k| !new.files.contains_key(*k)).cloned());
    v.sort();
    v
}

/// The parts of the map some files are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub global: bool,
    pub tiles: BTreeSet<TileKey>,
    pub timetable: bool,
    pub traffic: bool,
    pub descriptions: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }

    /// What `self` has that `other` has not.
    pub fn without(&self, other: &Changes) -> Changes {
        Changes {
            global: self.global && !other.global,
            tiles: self.tiles.difference(&other.tiles).copied().collect(),
            timetable: self.timetable && !other.timetable,
            traffic: self.traffic && !other.traffic,
            descriptions: self.descriptions && !other.descriptions,
        }
    }

    /// In words, for a question or a note ("3 tiles, the global settings").
    pub fn describe(&self) -> String {
        use crate::i18n::tr;
        let mut parts: Vec<String> = Vec::new();
        if !self.tiles.is_empty() {
            parts.push(format!("{} {}", self.tiles.len(), tr("tile(s)", "тайл(ов)")));
        }
        if self.global {
            parts.push(tr("the global settings", "общие настройки").to_string());
        }
        if self.timetable {
            parts.push(tr("the timetable", "расписание").to_string());
        }
        if self.traffic {
            parts.push(tr("the traffic", "трафик").to_string());
        }
        if self.descriptions {
            parts.push(tr("the descriptions", "описания").to_string());
        }
        parts.join(", ")
    }
}

/// The parts of `doc` the files are (a tile file by the name global.cfg gives it; files that
/// are none of the map's parts, a light map say, are left out).
pub fn classify(doc: &Document, files: &[String]) -> Changes {
    // (as global.cfg names them; a tile made here since, by the name its save gives it)
    let mut tiles: BTreeMap<String, TileKey> = doc.tiles.keys().map(|k| (super::save::file_of(doc, *k).replace('\\', "/").to_lowercase(), *k)).collect();
    tiles.extend(doc.global.tiles.iter().map(|t| (t.file.replace('\\', "/").to_lowercase(), (t.x, t.y))));
    let mut c = Changes::default();
    for f in files {
        if f == "global.cfg" {
            c.global = true;
        } else if f.starts_with("ttdata/") || f.starts_with("car_use/") || f == "holidays.txt" {
            c.timetable = true;
        } else if f.starts_with("global_") && f.ends_with(".dsc") {
            c.descriptions = true;
        } else if let Some(rest) = f.strip_prefix("texture/map/") {
            // (`<tile file>.<layer>.dds`)
            if let Some(k) = rest.rsplitn(3, '.').nth(2).and_then(|t| tiles.get(t)) {
                c.tiles.insert(*k);
            }
        } else if let Some(k) = tiles.get(f.as_str()).or_else(|| [".terrain", ".water"].iter().find_map(|x| f.strip_suffix(x)).and_then(|t| tiles.get(t))) {
            c.tiles.insert(*k);
        } else if !f.contains('/') && (f.ends_with(".txt") || f.ends_with(".cfg") || f.ends_with(".hof")) {
            c.traffic = true;
        }
    }
    c
}

/// The parts of `changes` that the map open here has changed too and not saved yet.
/// `global_base`: the global settings as they were read or saved (a map of the installation
/// not saved yet is marked changed while it is only a copy to make).
pub fn conflicts(doc: &Document, changes: &Changes, global_base: &str) -> Changes {
    Changes {
        global: changes.global && doc.global_dirty && super::global_text(doc) != global_base,
        tiles: changes.tiles.iter().filter(|k| doc.tiles.get(k).map(|t| t.dirty).unwrap_or(false)).copied().collect(),
        timetable: changes.timetable && doc.timetable.dirty,
        traffic: changes.traffic && doc.traffic.dirty,
        descriptions: changes.descriptions && doc.descriptions.dirty,
    }
}

/// Read the `changes` parts of the map from `dir` (the folder it is read from) again, in place
/// of what the document has. A tile with changes here not saved yet is left as it is unless
/// `force`. What was read in again.
pub fn reload(doc: &mut Document, dir: &Path, changes: &Changes, force: bool) -> Changes {
    let mut done = Changes::default();
    let chrono_dirs = omsi_map::active_chrono_dirs(dir, super::today_code());
    let mut tiles = changes.tiles.clone();
    if changes.global {
        if let Ok(global) = omsi_map::GlobalCfg::load(&dir.join("global.cfg")) {
            omsi_map::configure_grid(&global);
            let listed: BTreeSet<TileKey> = global.tiles.iter().map(|t| (t.x, t.y)).collect();
            // tiles that came: read; tiles that went: gone, but not one with changes here
            tiles.extend(listed.iter().filter(|k| !doc.tiles.contains_key(k)).copied());
            let gone: Vec<TileKey> = doc.tiles.iter().filter(|(k, t)| !listed.contains(k) && (force || !t.dirty)).map(|(k, _)| *k).collect();
            for k in gone {
                doc.tiles.remove(&k);
                done.tiles.insert(k);
            }
            let mut order = global.raw_tiles.clone();
            order.extend(doc.tiles.keys().filter(|k| !order.contains(k)).copied().collect::<Vec<_>>());
            doc.order = order;
            doc.global = global;
            doc.global_dirty = false;
            doc.global_rev = next_rev();
            doc.sync_mask_slots();
            done.global = true;
        }
    }
    for k in tiles {
        if doc.tiles.get(&k).map(|t| t.dirty && !force).unwrap_or(false) {
            continue;
        }
        let t = doc.global.tiles.iter().find(|t| (t.x, t.y) == k).cloned().unwrap_or_else(|| omsi_map::MapTileRef { x: k.0, y: k.1, file: super::save::file_of(doc, k), index: 0 });
        match load_tile(dir, &doc.global, &chrono_dirs, &t) {
            Some(mut td) => {
                if let Some(old) = doc.tiles.get(&k) {
                    // (the chrono patches shown stay those of the date looked at)
                    td.chrono = old.chrono.clone();
                }
                td.masks.resize(doc.global.ground_textures.len(), None);
                doc.tiles.insert(k, td);
            }
            // (its file went: the tile goes, as a map read now would not have it)
            None => {
                doc.tiles.remove(&k);
            }
        }
        done.tiles.insert(k);
    }
    if changes.timetable && (force || !doc.timetable.dirty) {
        doc.timetable = crate::timetable::Timetable::load(dir, &doc.order);
        done.timetable = true;
    }
    if changes.traffic && (force || !doc.traffic.dirty) {
        doc.traffic = crate::traffic::Traffic::load(dir);
        done.traffic = true;
    }
    if changes.descriptions && (force || !doc.descriptions.dirty) {
        doc.descriptions = crate::descriptions::Descriptions::load(dir);
        done.descriptions = true;
    }
    done
}

/// The folder the open map's files are read from now: its own, or the one it is a copy of
/// until it is first saved. None for a map never saved.
pub fn read_dir(doc: &Document) -> Option<PathBuf> {
    if doc.fresh {
        return None;
    }
    Some(doc.source.clone().unwrap_or_else(|| doc.dir.clone()))
}

/// A folder of `parent` by its name, whatever its case.
fn child(parent: &Path, name: &str) -> Option<PathBuf> {
    let exact = parent.join(name);
    if exact.is_dir() {
        return Some(exact);
    }
    std::fs::read_dir(parent).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

/// The installation's map that a map of the content folder is the copy of (the same folder
/// name in the installation's `maps`).
pub fn twin_of(roots: &crate::install::Roots, map_dir: &Path) -> Option<PathBuf> {
    if !map_dir.starts_with(&roots.content) {
        return None;
    }
    let name = map_dir.file_name()?.to_string_lossy().to_string();
    let twin = child(&child(&roots.omsi, "maps")?, &name)?;
    twin.join("global.cfg").is_file().then_some(twin)
}

/// The copy in the content folder of an installation's map (`cfg` its global.cfg), when
/// openMapper made one: that copy is the map openMapper edits.
pub fn copy_of(roots: &crate::install::Roots, cfg: &Path) -> Option<PathBuf> {
    let dir = cfg.parent()?;
    if dir.starts_with(&roots.content) {
        return None;
    }
    let name = dir.file_name()?.to_string_lossy().to_string();
    let copy = child(&child(&roots.content, "maps")?, &name)?;
    copy.join("global.cfg").is_file().then(|| copy.join("global.cfg"))
}

fn twin_file(copy: &Path) -> PathBuf {
    copy.join("openMapper").join("twin.json")
}

/// How the installation's map looked when its copy last took it in.
pub fn twin_seen(copy: &Path) -> Option<Stamp> {
    serde_json::from_str(&std::fs::read_to_string(twin_file(copy)).ok()?).ok()
}

/// Remember how the installation's map looks now (taken in, or its changes declined).
pub fn set_twin_seen(copy: &Path, twin: &Stamp) {
    let p = twin_file(copy);
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(copy));
    match serde_json::to_string(twin) {
        Ok(s) => {
            if let Err(e) = super::safe::write_atomic(&p, s.as_bytes()) {
                log::warn!("{}: {e}", p.display());
            }
        }
        Err(e) => log::warn!("{e}"),
    }
}

/// The installation's map files changed since its copy took it in: against what it
/// remembers, or, for a copy made before it remembered, what was written after the copy's
/// own global.cfg (its last save).
pub fn twin_changes(copy: &Path, twin: &Stamp) -> Vec<String> {
    match twin_seen(copy) {
        Some(seen) => changed_files(&seen, twin).into_iter().filter(|f| twin.files.contains_key(f)).collect(),
        None => {
            let saved = stat(&copy.join("global.cfg")).map(|s| s.1).unwrap_or(u64::MAX);
            twin.files.iter().filter(|(_, s)| s.1 > saved).map(|(k, _)| k.clone()).collect()
        }
    }
}

/// Copy the installation's changed files over the copy's (what they replace kept in a
/// backup). The copy is then read in again like any change made outside.
pub fn take_from_twin(twin: &Path, copy: &Path, files: &[String]) -> anyhow::Result<usize> {
    use super::safe::Op;
    let mut ops = Vec::new();
    for f in files {
        let from = f.split('/').fold(twin.to_path_buf(), |p, c| child(&p, c).unwrap_or_else(|| real_file(&p, c)));
        let Ok(bytes) = std::fs::read(&from) else { continue };
        let rel = from.strip_prefix(twin).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(f));
        let to = super::safe::own_path(copy, &rel.to_string_lossy());
        ops.push(Op::Write(to, bytes));
    }
    if ops.is_empty() {
        return Ok(0);
    }
    for o in &ops {
        if let Some(d) = o.path().parent() {
            std::fs::create_dir_all(d)?;
        }
    }
    let kept = super::safe::backup(copy, &ops, &[])?;
    super::safe::commit(&ops)?;
    if let Some(b) = kept {
        log::info!("the files taken from {} replaced ones kept in {}", twin.display(), b.display());
    }
    Ok(ops.len())
}

/// A file of `dir` by its name, whatever its case (the name as given when there is none).
fn real_file(dir: &Path, name: &str) -> PathBuf {
    std::fs::read_dir(dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))).unwrap_or_else(|| dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{sample_map, scratch};
    use super::*;

    /// A file written again a little later (file times can be coarse).
    fn touch(p: &Path, bytes: &[u8]) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn a_change_outside_is_seen_and_an_unchanged_folder_is_not() {
        let _map = super::super::one_map();
        let roots = scratch("sync-stamp");
        let mut doc = sample_map(&roots);
        super::super::save(&mut doc).unwrap();
        let a = stamp(&doc.dir);
        assert!(a.files.contains_key("global.cfg") && a.files.contains_key("tile_0_0.map") && a.files.contains_key("tile_0_0.map.terrain"));
        // nothing written: nothing changed
        assert!(changed_files(&a, &stamp(&doc.dir)).is_empty());
        // another program saves a tile and the global settings
        let tile = std::fs::read(doc.dir.join("tile_1_0.map")).unwrap();
        touch(&doc.dir.join("tile_1_0.map"), &tile);
        let cfg = std::fs::read(doc.dir.join("global.cfg")).unwrap();
        touch(&doc.dir.join("global.cfg"), &cfg);
        // and leaves a file of its own the map does not read from
        std::fs::create_dir_all(doc.dir.join("backup")).unwrap();
        std::fs::write(doc.dir.join("backup").join("x.map"), b"x").unwrap();
        let files = changed_files(&a, &stamp(&doc.dir));
        assert_eq!(files, vec!["global.cfg".to_string(), "tile_1_0.map".to_string()]);
        let c = classify(&doc, &files);
        assert!(c.global && c.tiles == BTreeSet::from([(1, 0)]) && !c.timetable);
        // a painting mask and the timetable count too
        let c = classify(&doc, &["texture/map/tile_0_0.map.1.dds".into(), "ttdata/x.ttl".into(), "tile_0_-1.map.terrain".into(), "tile_0_0.map.lm.bmp".into()]);
        assert_eq!(c.tiles, BTreeSet::from([(0, 0), (0, -1)]));
        assert!(c.timetable && !c.global && !c.traffic);
        let _ = std::fs::remove_dir_all(&roots.content);
    }

    #[test]
    fn a_tile_changed_outside_is_read_again_unless_it_has_changes_here() {
        let _map = super::super::one_map();
        let roots = scratch("sync-reload");
        let mut doc = sample_map(&roots);
        super::super::save(&mut doc).unwrap();
        let base = stamp(&doc.dir);
        let global_base = super::super::global_text(&doc);
        // OMSI 2's editor puts an object on tile 1, 0 and on tile 0, 0 and adds a tile
        let mut other = Document::load(&roots, &doc.dir.join("global.cfg"), |_| {}).unwrap();
        for k in [(1, 0), (0, 0)] {
            other.tiles.get_mut(&k).unwrap().tile.objects.push(omsi_map::MapObject { file: "Sceneryobjects\\new.sco".into(), id: 500 + k.0 as i64, pos: [5.0, 5.0, 0.0], ..Default::default() });
            other.tiles.get_mut(&k).unwrap().touch_content();
        }
        other.add_tile((2, 0));
        std::thread::sleep(std::time::Duration::from_millis(20));
        super::super::save(&mut other).unwrap();
        // here tile 0, 0 has a change not saved yet
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: "Sceneryobjects\\mine.sco".into(), id: 900, pos: [1.0, 1.0, 0.0], ..Default::default() });
        doc.tiles.get_mut(&(0, 0)).unwrap().touch_content();
        let files = changed_files(&base, &stamp(&doc.dir));
        let changes = classify(&doc, &files);
        assert!(changes.global && changes.tiles.contains(&(1, 0)) && changes.tiles.contains(&(0, 0)));
        let mine = conflicts(&doc, &changes, &global_base);
        assert_eq!(mine.tiles, BTreeSet::from([(0, 0)]));
        assert!(!mine.global);
        // keep mine: the rest is read in
        let dir = doc.dir.clone();
        let done = reload(&mut doc, &dir, &changes, false);
        assert!(done.global && done.tiles.contains(&(1, 0)) && done.tiles.contains(&(2, 0)) && !done.tiles.contains(&(0, 0)));
        assert!(doc.object((1, 0), 501).is_some() && doc.tiles.contains_key(&(2, 0)));
        assert!(doc.object((0, 0), 900).is_some() && doc.object((0, 0), 500).is_none() && doc.tiles[&(0, 0)].dirty);
        assert!(!doc.tiles[&(1, 0)].dirty && doc.order.contains(&(2, 0)));
        // reload from disk: theirs
        reload(&mut doc, &dir, &mine, true);
        assert!(doc.object((0, 0), 500).is_some() && doc.object((0, 0), 900).is_none() && !doc.is_dirty());
        let _ = std::fs::remove_dir_all(&roots.content);
    }

    #[test]
    fn a_copy_finds_what_changed_in_the_installation() {
        let _map = super::super::one_map();
        let mut roots = scratch("sync-twin");
        // (an installation beside the content folder, not in it)
        roots.omsi = roots.content.with_extension("omsi");
        let _ = std::fs::remove_dir_all(&roots.omsi);
        // the installation's map, and openMapper's copy of it
        std::fs::create_dir_all(roots.omsi.join("maps")).unwrap();
        let mut doc = sample_map(&roots);
        doc.dir = roots.omsi.join("maps").join("Roundtrip");
        super::super::save(&mut doc).unwrap();
        let twin = doc.dir.clone();
        let loaded = Document::load(&roots, &twin.join("global.cfg"), |_| {}).unwrap();
        assert_eq!(copy_of(&roots, &twin.join("global.cfg")), None);
        let mut copy = loaded;
        super::super::save(&mut copy).unwrap();
        assert_eq!(copy.dir, roots.content.join("maps").join("Roundtrip"));
        assert_eq!(copy_of(&roots, &twin.join("global.cfg")), Some(copy.dir.join("global.cfg")));
        assert_eq!(twin_of(&roots, &copy.dir), Some(twin.clone()));
        // a copy made before it remembered: nothing written after its save, nothing to take
        assert!(twin_changes(&copy.dir, &stamp(&twin)).is_empty());
        set_twin_seen(&copy.dir, &stamp(&twin));
        assert!(twin_changes(&copy.dir, &stamp(&twin)).is_empty());
        // OMSI 2's editor changes a tile of the installation's
        let mut other = Document::load(&roots, &twin.join("global.cfg"), |_| {}).unwrap();
        other.dir = twin.clone();
        other.source = None;
        other.tiles.get_mut(&(1, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: "Sceneryobjects\\omsi.sco".into(), id: 700, pos: [5.0, 5.0, 0.0], ..Default::default() });
        other.tiles.get_mut(&(1, 0)).unwrap().touch_content();
        std::thread::sleep(std::time::Duration::from_millis(20));
        super::super::save(&mut other).unwrap();
        let now = stamp(&twin);
        let files = twin_changes(&copy.dir, &now);
        assert!(files.contains(&"tile_1_0.map".to_string()) && !files.contains(&"tile_0_0.map".to_string()));
        // taken over: the copy reads it
        let before = stamp(&copy.dir);
        assert!(take_from_twin(&twin, &copy.dir, &files).unwrap() > 0);
        let changes = classify(&copy, &changed_files(&before, &stamp(&copy.dir)));
        assert!(changes.tiles.contains(&(1, 0)));
        let dir = copy.dir.clone();
        reload(&mut copy, &dir, &changes, false);
        assert!(copy.object((1, 0), 700).is_some());
        set_twin_seen(&copy.dir, &now);
        assert!(twin_changes(&copy.dir, &stamp(&twin)).is_empty());
        let _ = std::fs::remove_dir_all(&roots.content);
        let _ = std::fs::remove_dir_all(&roots.omsi);
    }
}
