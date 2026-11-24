//! Changes made in a chrono scenario rather than the map, as OMSI 2's editor does it: what
//! is selected put into the scenario (it appears from the scenario's first day), taken away
//! from that day on (`[selobject]` / `[selspline]` with `[delete]`), or given another type
//! (`[typ]`). Each is written into the scenario's patch of the tile at once.

use std::path::{Path, PathBuf};

use omsi_map::Tile;

use crate::doc::save::{object, spline, utf16};
use crate::doc::{Document, Sel, TileKey};

/// A whole patch written again (to take something out of it): what it adds, then its
/// changes of the map's things. None: it has something this does not write.
pub fn patch_text(t: &Tile) -> Option<String> {
    if !t.unknown_keywords.is_empty() {
        return None;
    }
    let mut s = crate::doc::save::header();
    s.push_str("[version]\n14\n\n");
    // (its own ground and water, which lie beside it)
    if t.has_terrain {
        s.push_str("[terrain]\n\n\n");
    }
    if t.has_water {
        s.push_str("[water]\n\n\n");
    }
    if t.variable_terrain_lightmap {
        s.push_str("[variable_terrainlightmap]\n\n");
    }
    if t.variable_terrain {
        s.push_str("[variable_terrain]\n\n");
    }
    // (deleted ones too: the rows along them count the splines by their place)
    for sp in &t.splines {
        spline(&mut s, sp);
    }
    for o in &t.objects {
        object(&mut s, o, false);
    }
    for a in &t.spline_attachments {
        crate::doc::save::attachment(&mut s, a);
    }
    for o in &t.attach_objects {
        object(&mut s, o, true);
    }
    for c in &t.chrono_changes {
        s.push_str(&format!("[{}]\n{}\n\n", if c.is_spline { "selspline" } else { "selobject" }, c.id));
        if let Some(f) = &c.new_type {
            s.push_str(&format!("[typ]\n{f}\n\n"));
        }
        if let Some(l) = &c.relabel {
            s.push_str(&format!("[relabel]\n{}\n", l.len()));
            l.iter().for_each(|x| s.push_str(&format!("{x}\n")));
            s.push('\n');
        }
        crate::doc::save::rules(&mut s, &c.rules);
        if c.delete {
            s.push_str("[delete]\n\n");
        }
    }
    Some(s)
}

/// The scenario folder `dir`'s patches: the tile each is of, its file name and its contents
/// (on the map's grid).
pub fn patches(dir: &Path, doc: &Document) -> Vec<(TileKey, String, Tile)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.to_lowercase().ends_with(".map") {
            continue;
        }
        let key = doc.global.tiles.iter().find(|t| t.file.eq_ignore_ascii_case(&name)).map(|t| (t.x, t.y)).or_else(|| Tile::coords_from_name(&name));
        if let (Some(k), Ok(mut t)) = (key, crate::doc::read_tile_file(&e.path())) {
            // (onto the map's grid, as the view has the tile)
            t.fit_to_world_grid(k.1);
            out.push((k, name, t));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// One thing of a patch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Part {
    Object(i64),
    Spline(i64),
    /// A change of the map's things, by its place in the patch's list.
    Change(usize),
}

/// A patch without one thing (an object goes with what hangs on it). The file is removed
/// when nothing is left in it and it brings no ground of its own.
pub fn take_out(dir: &Path, file: &str, part: Part) -> anyhow::Result<()> {
    let p = crate::doc::safe::own_path(dir, file);
    let mut t = crate::doc::read_tile_file(&p).map_err(|e| anyhow::anyhow!("{e}"))?;
    match part {
        Part::Object(id) => {
            t.objects.retain(|o| o.id != id);
            t.attach_objects.retain(|o| o.id != id && o.parent_id != Some(id));
        }
        Part::Spline(id) => {
            let Some(i) = t.splines.iter().position(|s| s.id == id) else { return Ok(()) };
            t.splines.remove(i);
            // (the rows along it go; those along later splines count one place less)
            t.spline_attachments.retain(|a| a.spline_index != i as i32);
            t.spline_attachments.iter_mut().filter(|a| a.spline_index > i as i32).for_each(|a| a.spline_index -= 1);
        }
        Part::Change(i) => {
            if i < t.chrono_changes.len() {
                t.chrono_changes.remove(i);
            }
        }
    }
    let empty = t.objects.is_empty() && t.splines.is_empty() && t.attach_objects.is_empty() && t.spline_attachments.is_empty() && t.chrono_changes.is_empty();
    if empty && !t.has_terrain && !t.has_water {
        std::fs::remove_file(&p)?;
        return Ok(());
    }
    let text = patch_text(&t).ok_or_else(|| anyhow::anyhow!("{}: {}", p.display(), crate::i18n::tr("it has parts openMapper does not write", "в нём есть то, что openMapper не пишет")))?;
    crate::doc::safe::write_atomic(&p, &utf16(&text))?;
    Ok(())
}

/// A scenario's own things being edited in the map itself: the objects and splines it
/// brings in are put into the map's tiles for as long as it lasts, then written back into
/// the scenario's patches with whatever was placed meanwhile.
pub struct Session {
    pub dir: PathBuf,
    /// Every IDCode the map had before (what is not among them at the end is the scenario's).
    before: std::collections::HashSet<i64>,
    /// The patches as they were (each tile's file name and contents in its own measure).
    patches: Vec<(TileKey, String, Tile)>,
}

fn all_ids(doc: &Document) -> std::collections::HashSet<i64> {
    doc.tiles.values().flat_map(|t| t.tile.objects.iter().chain(&t.tile.attach_objects).map(|o| o.id).chain(t.tile.splines.iter().map(|s| s.id))).collect()
}

/// Which splines of a patch stay in it while it is edited: those with rows along them (the
/// rows count their spline by its place in the file).
fn kept(t: &Tile) -> Vec<bool> {
    (0..t.splines.len()).map(|i| t.spline_attachments.iter().any(|a| a.spline_index == i as i32)).collect()
}

/// Put scenario `dir`'s things (in the map's own folder) into the map. How many.
pub fn begin(doc: &mut Document, dir: &Path) -> (Session, usize) {
    let before = all_ids(doc);
    let mut patches = Vec::new();
    let mut n = 0;
    for (k, file, _) in patches_of(dir, doc) {
        let Ok(raw) = crate::doc::read_tile_file(&crate::doc::safe::own_path(dir, &file)) else { continue };
        let mut fitted = raw.clone();
        fitted.fit_to_world_grid(k.1);
        let keep = kept(&raw);
        if let Some(t) = doc.tiles.get_mut(&k) {
            n += fitted.objects.len() + fitted.attach_objects.len();
            t.tile.objects.extend(fitted.objects.iter().cloned());
            t.tile.attach_objects.extend(fitted.attach_objects.iter().cloned());
            for (s, keep) in fitted.splines.iter().zip(&keep) {
                if !keep && !s.deleted {
                    t.tile.splines.push(s.clone());
                    n += 1;
                }
            }
            t.content_rev = crate::doc::next_rev();
        }
        patches.push((k, file, raw));
    }
    (Session { dir: dir.to_path_buf(), before, patches }, n)
}

/// Take the scenario's things (and what was placed meanwhile) out of the map again and write
/// them into its patches. How many it has now.
pub fn end(doc: &mut Document, s: Session) -> anyhow::Result<usize> {
    let mut n = 0;
    let mut gone: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut by_tile: Vec<(TileKey, Tile)> = Vec::new();
    for (k, t) in doc.tiles.iter_mut() {
        let mut mine = Tile::default();
        let objects = std::mem::take(&mut t.tile.objects);
        let (theirs, ours): (Vec<_>, Vec<_>) = objects.into_iter().partition(|o| s.before.contains(&o.id));
        t.tile.objects = theirs;
        mine.objects = ours;
        let attach = std::mem::take(&mut t.tile.attach_objects);
        let (theirs, ours): (Vec<_>, Vec<_>) = attach.into_iter().partition(|o| s.before.contains(&o.id));
        t.tile.attach_objects = theirs;
        mine.attach_objects = ours;
        // (splines: rows of the map along them would lose their place: those stay where they are)
        let splines = std::mem::take(&mut t.tile.splines);
        let rows: std::collections::HashSet<usize> = t.tile.spline_attachments.iter().map(|a| a.spline_index.max(0) as usize).collect();
        let mut keep = Vec::new();
        for (i, sp) in splines.into_iter().enumerate() {
            if s.before.contains(&sp.id) || rows.contains(&i) {
                keep.push(sp);
            } else if !sp.deleted {
                mine.splines.push(sp);
            }
        }
        t.tile.splines = keep;
        if mine.objects.is_empty() && mine.attach_objects.is_empty() && mine.splines.is_empty() {
            continue;
        }
        gone.extend(mine.objects.iter().chain(&mine.attach_objects).map(|o| o.id).chain(mine.splines.iter().map(|x| x.id)));
        t.content_rev = crate::doc::next_rev();
        crate::doc::unfit_from_world_grid(&mut mine, k.1);
        by_tile.push((*k, mine));
    }
    // (the map's splines do not go on into the scenario's: the map alone has no such spline)
    for t in doc.tiles.values_mut() {
        for sp in t.tile.splines.iter_mut() {
            if gone.contains(&sp.prev_id) {
                sp.prev_id = 0;
            }
            if gone.contains(&sp.next_id) {
                sp.next_id = 0;
            }
        }
    }
    // every patch the scenario had or now needs, written whole
    let mut keys: Vec<TileKey> = s.patches.iter().map(|p| p.0).chain(by_tile.iter().map(|b| b.0)).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (file, mut patch) = match s.patches.iter().find(|p| p.0 == k) {
            Some((_, f, raw)) => {
                let mut t = raw.clone();
                // what was put into the map comes out of it (it comes back below as it is now)
                let keep = kept(raw);
                let mut map_index: Vec<Option<i32>> = Vec::new();
                let mut stay = Vec::new();
                for (sp, k) in t.splines.iter().zip(&keep) {
                    if *k || sp.deleted {
                        map_index.push(Some(stay.len() as i32));
                        stay.push(sp.clone());
                    } else {
                        map_index.push(None);
                    }
                }
                t.splines = stay;
                for a in t.spline_attachments.iter_mut() {
                    a.spline_index = map_index.get(a.spline_index.max(0) as usize).copied().flatten().unwrap_or(a.spline_index);
                }
                t.objects.clear();
                t.attach_objects.clear();
                (f.clone(), t)
            }
            None => (doc.global.tiles.iter().find(|t| (t.x, t.y) == k).map(|t| t.file.clone()).unwrap_or_else(|| crate::doc::save::tile_file(k)), Tile { version: 14, ..Default::default() }),
        };
        if let Some((_, mine)) = by_tile.iter().find(|b| b.0 == k) {
            n += mine.objects.len() + mine.attach_objects.len() + mine.splines.len();
            patch.objects.extend(mine.objects.iter().cloned());
            patch.attach_objects.extend(mine.attach_objects.iter().cloned());
            patch.splines.extend(mine.splines.iter().cloned());
        }
        let p = crate::doc::safe::own_path(&s.dir, &file);
        let empty = patch.objects.is_empty() && patch.splines.is_empty() && patch.attach_objects.is_empty() && patch.spline_attachments.is_empty() && patch.chrono_changes.is_empty();
        if empty && !patch.has_terrain && !patch.has_water {
            if p.is_file() {
                std::fs::remove_file(&p)?;
            }
            continue;
        }
        let text = patch_text(&patch).ok_or_else(|| anyhow::anyhow!("{}: {}", p.display(), crate::i18n::tr("it has parts openMapper does not write", "в нём есть то, что openMapper не пишет")))?;
        std::fs::create_dir_all(&s.dir)?;
        crate::doc::safe::write_atomic(&p, &utf16(&text))?;
    }
    Ok(n)
}

/// The scenario folder's patches (file names, of which tiles), unread.
fn patches_of(dir: &Path, doc: &Document) -> Vec<(TileKey, String, ())> {
    patches(dir, doc).into_iter().map(|(k, f, _)| (k, f, ())).collect()
}

/// What a patch gets.
pub enum Change {
    /// Objects and splines that appear (the map's own copies are taken away by the caller).
    Add(Tile),
    /// The map's objects and splines (IDCode, is a spline) that go from the scenario's day.
    Delete(Vec<(i64, bool)>),
    /// The map's objects (and splines) that take another type.
    Retype(Vec<(i64, bool)>, String),
    /// A map object whose texts become these.
    Relabel(i64, Vec<String>),
    /// A map object's or spline's (IDCode, is a spline) path rules: those it had taken away,
    /// these instead.
    Rules(i64, bool, Vec<omsi_map::MapRule>, Vec<omsi_map::MapRule>),
}

/// The text a change adds to a patch.
fn text(c: &Change) -> String {
    let mut s = String::new();
    match c {
        Change::Add(t) => {
            for sp in t.splines.iter().filter(|s| !s.deleted) {
                spline(&mut s, sp);
            }
            for o in &t.objects {
                object(&mut s, o, false);
            }
            for o in &t.attach_objects {
                object(&mut s, o, true);
            }
        }
        Change::Delete(ids) => {
            for (id, is_spline) in ids {
                s.push_str(&format!("[{}]\n{id}\n\n[delete]\n\n", if *is_spline { "selspline" } else { "selobject" }));
            }
        }
        Change::Relabel(id, strings) => {
            s.push_str(&format!("[selobject]\n{id}\n\n[relabel]\n{}\n", strings.len()));
            strings.iter().for_each(|x| s.push_str(&format!("{x}\n")));
            s.push('\n');
        }
        Change::Rules(id, is_spline, old, new) => {
            s.push_str(&format!("[{}]\n{id}\n\n", if *is_spline { "selspline" } else { "selobject" }));
            let kill: Vec<omsi_map::MapRule> = old.iter().map(|r| omsi_map::MapRule { kill: true, ..r.clone() }).collect();
            crate::doc::save::rules(&mut s, &kill);
            crate::doc::save::rules(&mut s, new);
        }
        Change::Retype(ids, file) => {
            for (id, is_spline) in ids {
                s.push_str(&format!("[{}]\n{id}\n\n[typ]\n{file}\n\n", if *is_spline { "selspline" } else { "selobject" }));
            }
        }
    }
    s
}

/// Add a change to the scenario folder `dir`'s patch of the tile kept in `file` (made when
/// it has none: only its version, so the tile keeps the map's ground).
pub fn write(dir: &Path, file: &str, c: &Change) -> anyhow::Result<PathBuf> {
    let p = crate::doc::safe::own_path(dir, file);
    let old = std::fs::read(&p).map(|b| omsi_cfg::decode_text(&b)).unwrap_or_default();
    let mut t = if old.trim().is_empty() { format!("{}[version]\n14\n\n", crate::doc::save::header()) } else { old.replace("\r\n", "\n") };
    if !t.ends_with("\n\n") {
        t.push('\n');
    }
    t.push_str(&text(c));
    std::fs::create_dir_all(dir)?;
    crate::doc::safe::write_atomic(&p, &utf16(&t))?;
    Ok(p)
}

/// The selection, tile by tile: the file each tile is kept in, and the selected things there.
pub fn by_tile(doc: &Document, sels: &[Sel]) -> Vec<(TileKey, String, Vec<Sel>)> {
    let mut out: Vec<(TileKey, String, Vec<Sel>)> = Vec::new();
    for s in sels {
        let k = match *s {
            Sel::Object(k, _) | Sel::Spline(k, _) | Sel::Attached(k, _) => k,
        };
        let file = doc.global.tiles.iter().find(|t| (t.x, t.y) == k).map(|t| t.file.clone()).unwrap_or_else(|| crate::doc::save::tile_file(k));
        match out.iter_mut().find(|o| o.0 == k) {
            Some(o) => o.2.push(*s),
            None => out.push((k, file, vec![*s])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_map::{MapObject, MapSpline};

    #[test]
    fn a_thing_is_taken_out_of_a_patch() {
        let dir = std::env::temp_dir().join(format!("openmapper-takeout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut add = Tile::default();
        add.objects.push(MapObject { file: "a.sco".into(), id: 50, ..Default::default() });
        add.attach_objects.push(MapObject { file: "sign.sco".into(), id: 52, parent_id: Some(50), ..Default::default() });
        add.splines.push(MapSpline { file: "r.sli".into(), id: 51, length: 10.0, ..Default::default() });
        write(&dir, "t.map", &Change::Add(add)).unwrap();
        write(&dir, "t.map", &Change::Delete(vec![(7, false)])).unwrap();
        take_out(&dir, "t.map", Part::Object(50)).unwrap();
        let t = Tile::load(&dir.join("t.map")).unwrap();
        assert!(t.objects.is_empty() && t.attach_objects.is_empty() && t.splines.len() == 1 && t.chrono_changes.len() == 1);
        take_out(&dir, "t.map", Part::Spline(51)).unwrap();
        take_out(&dir, "t.map", Part::Change(0)).unwrap();
        assert!(!dir.join("t.map").exists(), "nothing left: the file goes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scenario's things edited in the map: one moved, one placed meanwhile, and both are
    /// the scenario's again afterwards, the map as it was.
    #[test]
    fn a_session_puts_the_scenario_back_with_what_was_placed() {
        let dir = std::env::temp_dir().join(format!("openmapper-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut doc = crate::selection::tests::flat_map(1);
        crate::selection::tests::add_object(&mut doc, (0, 0), 1, 10.0, 10.0);
        let mut add = Tile::default();
        add.objects.push(MapObject { file: "a.sco".into(), id: 50, pos: [20.0, 20.0, 0.0], ..Default::default() });
        write(&dir, "tile_0_0.map", &Change::Add(add)).unwrap();
        write(&dir, "tile_0_0.map", &Change::Delete(vec![(1, false)])).unwrap();
        let (s, n) = begin(&mut doc, &dir);
        assert_eq!(n, 1);
        let t = &mut doc.tiles.get_mut(&(0, 0)).unwrap().tile;
        t.objects.iter_mut().find(|o| o.id == 50).unwrap().pos[0] = 30.0;
        t.objects.push(MapObject { file: "b.sco".into(), id: 60, pos: [5.0, 5.0, 0.0], ..Default::default() });
        assert_eq!(end(&mut doc, s).unwrap(), 2);
        assert_eq!(doc.tiles[&(0, 0)].tile.objects.iter().map(|o| o.id).collect::<Vec<_>>(), vec![1]);
        let patch = Tile::load(&dir.join("tile_0_0.map")).unwrap();
        assert_eq!(patch.objects.iter().map(|o| (o.id, o.pos[0])).collect::<Vec<_>>(), vec![(50, 30.0), (60, 5.0)]);
        assert_eq!(patch.chrono_changes.len(), 1, "its changes of the map stay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every chrono patch of the stock maps written again reads back the same.
    #[test]
    fn stock_patches_round_trip() {
        let Some(omsi) = crate::install::find_omsi_root(&Default::default()) else { return };
        let mut n = 0;
        for m in ["Grundorf", "Berlin-Spandau"] {
            let chrono = omsi.join("maps").join(m).join("Chrono");
            for d in std::fs::read_dir(&chrono).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                for f in std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "map").unwrap_or(false)) {
                    let t = Tile::load(&f).unwrap();
                    let Some(text) = patch_text(&t) else { continue };
                    let back = Tile::parse(&omsi_cfg::CfgFile::from_str(&f, &text));
                    // (numbers to ten places, as every save writes them: the same once written)
                    assert_eq!(patch_text(&back).as_deref(), Some(text.as_str()), "{}", f.display());
                    let ids = |t: &Tile| (t.objects.iter().map(|o| (o.id, o.file.clone(), o.extra.clone())).collect::<Vec<_>>(), t.splines.iter().map(|s| (s.id, s.prev_id, s.next_id)).collect::<Vec<_>>(), t.attach_objects.iter().map(|o| (o.id, o.parent_id, o.attach_index)).collect::<Vec<_>>(), t.spline_attachments.len(), t.has_terrain, t.has_water);
                    assert_eq!(ids(&back), ids(&t), "{}", f.display());
                    assert_eq!(back.chrono_changes, t.chrono_changes, "{}", f.display());
                    n += 1;
                }
            }
        }
        eprintln!("{n} patches read back the same");
    }

    /// What a scenario gets is what the game reads from its patch: new things, and the
    /// map's things taken away or of another type.
    #[test]
    fn patches_read_back_as_written() {
        let dir = std::env::temp_dir().join(format!("openmapper-patch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut add = Tile::default();
        add.objects.push(MapObject { file: "Sceneryobjects\\a.sco".into(), id: 50, pos: [1.0, 2.0, 0.0], ..Default::default() });
        add.splines.push(MapSpline { file: "Splines\\r.sli".into(), id: 51, length: 10.0, ..Default::default() });
        write(&dir, "tile_0_0.map", &Change::Add(add)).unwrap();
        write(&dir, "tile_0_0.map", &Change::Delete(vec![(7, false), (8, true)])).unwrap();
        write(&dir, "tile_0_0.map", &Change::Retype(vec![(9, false)], "Sceneryobjects\\b.sco".into())).unwrap();
        let p = write(&dir, "tile_0_0.map", &Change::Relabel(9, vec!["Neu".into(), "Platz".into()])).unwrap();
        let patch = Tile::load(&p).unwrap();
        assert!(!patch.has_terrain, "the map's ground stays");
        assert_eq!((patch.objects.len(), patch.splines.len()), (1, 1));
        let mut base = Tile::default();
        base.objects.push(MapObject { file: "x.sco".into(), id: 7, ..Default::default() });
        base.objects.push(MapObject { file: "x.sco".into(), id: 9, ..Default::default() });
        base.splines.push(MapSpline { file: "y.sli".into(), id: 8, ..Default::default() });
        assert_eq!(base.apply_chrono(&patch), 0);
        assert!(!base.objects.iter().any(|o| o.id == 7));
        assert!(base.splines.iter().any(|s| s.id == 8 && s.deleted));
        assert_eq!(base.objects.iter().find(|o| o.id == 9).unwrap().file, "Sceneryobjects\\b.sco");
        assert_eq!(base.objects.iter().find(|o| o.id == 9).unwrap().extra, vec!["Neu".to_string(), "Platz".to_string()]);
        assert!(base.objects.iter().any(|o| o.id == 50));
        // a spline's rules from the day on: the old one gone, the new one there
        let old = omsi_map::MapRule { path_index: 0, kind: "speedlimit".into(), value: 50.0, extra: 0.0, kill: false };
        let new = omsi_map::MapRule { value: 30.0, ..old.clone() };
        let p = write(&dir, "tile_0_1.map", &Change::Rules(8, true, vec![old.clone()], vec![new.clone()])).unwrap();
        let mut base = Tile::default();
        base.splines.push(MapSpline { file: "y.sli".into(), id: 8, rules: vec![old], ..Default::default() });
        base.apply_chrono(&Tile::load(&p).unwrap());
        assert_eq!(base.splines[0].rules, vec![new]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
