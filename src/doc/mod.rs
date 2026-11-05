//! The map being edited: `global.cfg`, its tiles (objects, splines, ground) and their
//! painting masks, read with openOMSI's own readers and written back in OMSI's format.

pub mod autosave;
pub mod safe;
mod transaction;
pub(crate) mod save;
#[cfg(test)]
pub(crate) mod tests;
pub mod dds;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicU64, Ordering}, Arc};

use glam::{DVec2, DVec3};
use omsi_map::{GlobalCfg, GroundTex, MapObject, MapSpline, SplineAttachment, Terrain, Tile, TERRAIN_SAMPLES};

use crate::i18n::tr;

pub use save::save;
pub(crate) use save::{global_text, tile_text};

pub type TileKey = (i32, i32);

/// The edge of a tile (m).
pub fn tile_size() -> f64 {
    omsi_map::tile_size()
}

/// World position of a tile's south-west corner.
pub fn tile_origin(k: TileKey) -> DVec2 {
    DVec2::new(k.0 as f64 * tile_size(), k.1 as f64 * tile_size())
}

/// The tile a world position lies in.
pub fn tile_of(p: DVec2) -> TileKey {
    ((p.x / tile_size()).floor() as i32, (p.y / tile_size()).floor() as i32)
}

static REVISION: AtomicU64 = AtomicU64::new(1);

/// A fresh revision number: whatever shows a tile rebuilds it when its number changes.
pub fn next_rev() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed)
}

/// One tile of the map.
#[derive(Clone)]
pub struct TileDoc {
    /// Objects, splines and everything else of the `.map` file (tile-local metres).
    pub tile: Tile,
    pub terrain: Terrain,
    /// Painting mask of each `[groundtex]` (index 0, the base ground, has none): alpha
    /// texels, row 0 the north edge. None: nothing painted.
    pub masks: Vec<Option<Vec<u8>>>,
    /// The active chrono folders' patches of this tile, in order (read only: the game lays
    /// them over the tile, and so does the view; the tile itself is what is edited).
    pub chrono: Vec<Tile>,
    /// Its water (`[water]` and `tile.map.water`): the surface's height at the corners
    /// (0, 0), (300, 0), (0, 300), (300, 300).
    pub water: Option<[f32; 4]>,
    /// Revisions of the ground, the paint and the contents.
    pub terrain_rev: u64,
    pub mask_rev: u64,
    pub content_rev: u64,
    /// Changed since it was loaded or saved.
    pub dirty: bool,
}

impl TileDoc {
    /// The tile as the game shows it: with its chrono patches laid over (in the tile's own
    /// measure, as they are written, then onto the grid again).
    pub fn display(&self, ty: i32) -> std::borrow::Cow<'_, Tile> {
        if self.chrono.is_empty() {
            return std::borrow::Cow::Borrowed(&self.tile);
        }
        let mut t = self.tile.clone();
        unfit_from_world_grid(&mut t, ty);
        for p in &self.chrono {
            t.apply_chrono(p);
        }
        t.fit_to_world_grid(ty);
        std::borrow::Cow::Owned(t)
    }

    pub fn new_flat(height: f32) -> TileDoc {
        let mut terrain = Terrain::flat();
        terrain.heights.iter_mut().for_each(|h| *h = height);
        TileDoc {
            // (lamps light the ground round them at night: the game bakes the tile's light
            // map from them, as on nearly every tile of the stock maps)
            tile: Tile { version: 14, has_terrain: true, variable_terrain_lightmap: true, ..Default::default() },
            chrono: Vec::new(),
            water: None,
            terrain,
            masks: Vec::new(),
            terrain_rev: next_rev(),
            mask_rev: next_rev(),
            content_rev: next_rev(),
            dirty: true,
        }
    }

    pub fn touch_terrain(&mut self) {
        self.terrain_rev = next_rev();
        self.dirty = true;
    }

    pub fn touch_masks(&mut self) {
        self.mask_rev = next_rev();
        self.dirty = true;
    }

    pub fn touch_content(&mut self) {
        self.content_rev = next_rev();
        self.dirty = true;
    }

    /// Height of the ground at tile-local metres.
    pub fn height_at(&self, x: f64, y: f64) -> f32 {
        omsi_geometry::terrain_height(&self.terrain, x as f32, y as f32)
    }
}

impl Document {
    /// A number that changes when anything on the map does: the tiles' own revisions summed
    /// (a revision only grows, so it grows with them). Cheap enough to ask every frame,
    /// unlike walking the map for what changed.
    pub fn content_rev(&self) -> u64 {
        self.tiles.values().fold(0u64, |a, t| a.wrapping_add(t.content_rev))
    }
}

/// Mask edge in texels for a ground layer.
/// A painting mask of `from` × `from` texels at `to` × `to`: bilinear when it grows (smooth
/// edges), averaged when it shrinks.
pub fn resample_mask(m: &[u8], from: usize, to: usize) -> Vec<u8> {
    if from == to || m.len() != from * from {
        return m.to_vec();
    }
    let mut out = vec![0u8; to * to];
    if to > from {
        let f = from as f64 / to as f64;
        for y in 0..to {
            let sy = ((y as f64 + 0.5) * f - 0.5).clamp(0.0, (from - 1) as f64);
            let (y0, ty) = (sy.floor() as usize, sy.fract());
            let y1 = (y0 + 1).min(from - 1);
            for x in 0..to {
                let sx = ((x as f64 + 0.5) * f - 0.5).clamp(0.0, (from - 1) as f64);
                let (x0, tx) = (sx.floor() as usize, sx.fract());
                let x1 = (x0 + 1).min(from - 1);
                let v = |xx: usize, yy: usize| m[yy * from + xx] as f64;
                let top = v(x0, y0) * (1.0 - tx) + v(x1, y0) * tx;
                let bottom = v(x0, y1) * (1.0 - tx) + v(x1, y1) * tx;
                out[y * to + x] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
        }
    } else {
        let k = from / to;
        for y in 0..to {
            for x in 0..to {
                let mut sum = 0u32;
                for yy in 0..k {
                    for xx in 0..k {
                        sum += m[(y * k + yy) * from + x * k + xx] as u32;
                    }
                }
                out[y * to + x] = (sum / (k * k) as u32) as u8;
            }
        }
    }
    out
}

pub fn mask_size(g: &GroundTex) -> usize {
    g.mask_size().clamp(16, 1024) as usize
}

/// What the editor has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sel {
    Object(TileKey, i64),
    Spline(TileKey, i64),
    /// A `[splineAttachement]` record, by its IDCode: one object placed on a spline, alone or
    /// as a row of copies. It is edited like a free object, with the road's place instead of
    /// east/north.
    Attached(TileKey, i64),
}

/// The map.
pub struct Document {
    pub(crate) _publisher_mounts: Arc<omsi_cfg::publisher::DependencyMounts>,
    pub(crate) _publisher_protected_source: bool,
    pub global: GlobalCfg,
    pub tiles: BTreeMap<TileKey, TileDoc>,
    /// The `[map]` list in file order: entry points and repeaters name tiles by their place
    /// in it.
    pub order: Vec<TileKey>,
    /// The folder the map is saved to (in openOMSI's content folder).
    pub dir: PathBuf,
    /// The folder it was read from, when that is another (the installation's copy).
    pub source: Option<PathBuf>,
    /// The global settings changed since the last save.
    pub global_dirty: bool,
    /// The map was never saved (made in openMapper).
    pub fresh: bool,
    pub global_rev: u64,
    /// Lines, trips and tours (`TTData`).
    pub timetable: crate::timetable::Timetable,
    /// AI groups, unscheduled traffic, parked cars and people.
    pub traffic: crate::traffic::Traffic,
    /// The name and description in other languages (`global_<LANG>.dsc`).
    pub descriptions: crate::descriptions::Descriptions,
    /// Things kept together (openMapper's own: `openMapper/groups.json` beside the map).
    pub groups: Vec<crate::mapgroups::MapGroup>,
}

impl Document {
    /// A new map from OMSI's own template (`template/NewMap` of the installation), `size`
    /// × `size` tiles of flat ground around tile 0, 0.
    pub fn new_map(roots: &crate::install::Roots, folder: &str, friendly: &str, size: i32) -> Document {
        let template = roots.omsi.join("template").join("NewMap");
        let mut global = GlobalCfg::load(&template.join("global.cfg")).unwrap_or_default();
        global.name = folder.to_string();
        global.friendly_name = friendly.to_string();
        global.description = String::new();
        global.version = 14;
        if global.next_id_code < 1 {
            global.next_id_code = 1;
        }
        if global.ground_textures.is_empty() {
            global.ground_textures.push(GroundTex {
                texture: "Texture\\gras.bmp".into(),
                detail_texture: "Texture\\gras_det.bmp".into(),
                params: [0.0, 1.0, 60.0],
            });
        }
        global.tiles.clear();
        global.raw_tiles.clear();
        global.entry_points.clear();
        omsi_map::set_world_coordinates(false);
        omsi_map::set_tile_size(omsi_map::TILE_SIZE);
        let mut doc = Document {
            global,
            _publisher_mounts: Arc::new(Default::default()),
            _publisher_protected_source: false,
            tiles: BTreeMap::new(),
            order: Vec::new(),
            dir: roots.content.join("maps").join(folder),
            traffic: crate::traffic::Traffic::load(&template),
            descriptions: Default::default(),
            groups: Vec::new(),
            source: Some(template),
            global_dirty: true,
            fresh: true,
            global_rev: next_rev(),
            timetable: Default::default(),
        };
        // (the cars the template's traffic names, so its streets are not empty)
        crate::traffic::fill_named_groups(&mut doc.traffic, &roots.omsi);
        let half = size / 2;
        for y in -half..=(size - 1 - half) {
            for x in -half..=(size - 1 - half) {
                doc.add_tile((x, y));
            }
        }
        doc
    }

    /// Read a map. One of the installation is saved to the content folder (the original
    /// stays as it is).
    pub fn load(roots: &crate::install::Roots, cfg: &Path, mut progress: impl FnMut(f32)) -> anyhow::Result<Document> {
        let started = std::time::Instant::now();
        let publisher_mounts = Arc::new(omsi_cfg::publisher::validate_map(cfg.parent().unwrap_or(Path::new(".")))
            .map_err(|e| anyhow::anyhow!("publisher dependencies: {e}"))?);
        let publisher_protected_source = omsi_cfg::vfs::is_sealed(cfg);
        let global = GlobalCfg::load(cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
        omsi_map::configure_grid(&global);
        let src_dir = cfg.parent().unwrap_or(Path::new(".")).to_path_buf();
        let folder = src_dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Map".into());
        let in_content = src_dir.starts_with(&roots.content);
        let dir = if in_content { src_dir.clone() } else { roots.content.join("maps").join(&folder) };
        // the chrono folders active today, as the game picks them
        let chrono_dirs = omsi_map::active_chrono_dirs(&src_dir, today_code());
        // (the tiles are read side by side: a big map has hundreds)
        let n = global.tiles.len();
        let next = AtomicU64::new(0);
        let done = AtomicU64::new(0);
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let read: Vec<(usize, Option<TileDoc>)> = std::thread::scope(|sc| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    sc.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                            let Some(t) = global.tiles.get(i) else { break };
                            // (a tile that makes the reader panic is left out like a missing one)
                            let tile = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_tile(&src_dir, &global, &chrono_dirs, t))).unwrap_or_else(|_| {
                                log::warn!("{}: could not be read", t.file);
                                None
                            });
                            out.push((i, tile));
                            done.fetch_add(1, Ordering::Relaxed);
                        }
                        out
                    })
                })
                .collect();
            // (the caller's progress, from this thread)
            while workers.iter().any(|w| !w.is_finished()) {
                progress(done.load(Ordering::Relaxed) as f32 / n.max(1) as f32);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
        });
        let mut tiles = BTreeMap::new();
        let mut missing = 0;
        let mut read = read;
        read.sort_by_key(|r| r.0);
        for (i, t) in read {
            match t {
                Some(t) => {
                    let e = &global.tiles[i];
                    tiles.insert((e.x, e.y), t);
                }
                None => missing += 1,
            }
        }
        if !chrono_dirs.is_empty() {
            log::info!("chrono: {} folder(s) active today", chrono_dirs.len());
        }
        if missing > 0 {
            log::info!("{}: {} of {} tiles listed in global.cfg have no file (left out)", src_dir.display(), missing, global.tiles.len());
        }
        let order = global.raw_tiles.clone();
        let timetable = crate::timetable::Timetable::load(&src_dir, &order);
        let traffic = crate::traffic::Traffic::load(&src_dir);
        let descriptions = crate::descriptions::Descriptions::load(&src_dir);
        progress(1.0);
        crate::render::profile::log_time(&format!("read {} tiles", tiles.len()), started);
        Ok(Document {
            global,
            _publisher_mounts: publisher_mounts,
            _publisher_protected_source: publisher_protected_source,
            tiles,
            order,
            dir,
            source: (!in_content).then_some(src_dir),
            global_dirty: !in_content,
            fresh: false,
            global_rev: next_rev(),
            timetable,
            traffic,
            descriptions,
            groups: Vec::new(),
        })
    }

    /// The folder its chrono scenarios are in (the saved copy's, else the one it was read from).
    pub fn chrono_root(&self) -> PathBuf {
        let own = crate::doc::safe::own_path(&self.dir, "Chrono");
        if own.is_dir() {
            return self.dir.clone();
        }
        self.source.clone().unwrap_or_else(|| self.dir.clone())
    }

    /// The map shown as on `date` (YYYYMMDD): every tile's patches of the chrono scenarios
    /// active then, as the game lays them over it. Nothing of the map itself changes.
    pub fn show_chrono(&mut self, date: i32) -> usize {
        self.show_chrono_without(date, None)
    }

    /// The same without the scenario in folder `left_out` (its things are being edited in the
    /// map itself: they would show twice).
    pub fn show_chrono_without(&mut self, date: i32, left_out: Option<&Path>) -> usize {
        let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).map(|(a, b)| a == b).unwrap_or(a == b);
        let dirs: Vec<PathBuf> = omsi_map::active_chrono_dirs(&self.chrono_root(), date).into_iter().filter(|d| left_out.map(|l| !same(d, l)).unwrap_or(true)).collect();
        let files: HashMap<TileKey, String> = self.global.tiles.iter().map(|t| ((t.x, t.y), t.file.clone())).collect();
        for (k, t) in self.tiles.iter_mut() {
            let patches = files.get(k).map(|f| chrono_patches(&dirs, f)).unwrap_or_default();
            if patches != t.chrono {
                t.chrono = patches;
                // (shown anew, not changed: no save is due)
                t.content_rev = next_rev();
            }
        }
        dirs.len()
    }

    /// A copy to read on another thread (the map check): the tiles without their painting
    /// and chrono patches, which take room and are not looked at.
    pub fn for_reading(&self) -> Document {
        Document {
            global: self.global.clone(),
            _publisher_mounts: self._publisher_mounts.clone(),
            _publisher_protected_source: self._publisher_protected_source,
            tiles: self.tiles.iter().map(|(k, t)| (*k, TileDoc { tile: t.tile.clone(), terrain: t.terrain.clone(), masks: Vec::new(), chrono: Vec::new(), water: t.water, terrain_rev: t.terrain_rev, mask_rev: t.mask_rev, content_rev: t.content_rev, dirty: t.dirty })).collect(),
            order: self.order.clone(),
            dir: self.dir.clone(),
            source: self.source.clone(),
            global_dirty: self.global_dirty,
            fresh: self.fresh,
            global_rev: self.global_rev,
            timetable: self.timetable.clone(),
            traffic: self.traffic.clone(),
            descriptions: Default::default(),
            groups: Vec::new(),
        }
    }

    pub fn display_name(&self) -> &str {
        if self.global.friendly_name.trim().is_empty() {
            &self.global.name
        } else {
            &self.global.friendly_name
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.global_dirty || self.timetable.dirty || self.traffic.dirty || self.descriptions.dirty || self.tiles.values().any(|t| t.dirty)
    }

    /// A fresh IDCode for a new object or spline. Maps often keep `NextIDCode` below an
    /// IDCode that is already in use: a new spline would then be the same one as a road
    /// that is already there, drawn on top of it, and deleting it would take that road.
    pub fn new_id(&mut self) -> i64 {
        let highest = self.highest_id();
        if self.global.next_id_code <= highest {
            self.global.next_id_code = highest + 1;
        }
        let id = self.global.next_id_code.max(1);
        self.global.next_id_code = id + 1;
        self.global_dirty = true;
        id
    }

    /// The greatest IDCode any object or spline on the map already has.
    fn highest_id(&self) -> i64 {
        let mut m = 0i64;
        for t in self.tiles.values() {
            for o in t.tile.objects.iter().chain(&t.tile.attach_objects) {
                m = m.max(o.id);
            }
            for s in &t.tile.splines {
                m = m.max(s.id);
            }
            for a in &t.tile.spline_attachments {
                m = m.max(a.id);
            }
        }
        m
    }

    /// Add a tile of flat ground at the mean height of its neighbours' edges.
    pub fn add_tile(&mut self, k: TileKey) {
        if self.tiles.contains_key(&k) {
            return;
        }
        let mut sum = 0.0f32;
        let mut n = 0;
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if let Some(t) = self.tiles.get(&(k.0 + dx, k.1 + dy)) {
                let s = TERRAIN_SAMPLES;
                for i in 0..s {
                    let (ix, iy) = match (dx, dy) {
                        (-1, _) => (s - 1, i),
                        (1, _) => (0, i),
                        (_, -1) => (i, s - 1),
                        _ => (i, 0),
                    };
                    sum += t.terrain.height_at(ix, iy);
                    n += 1;
                }
            }
        }
        let h = if n > 0 { sum / n as f32 } else { 0.0 };
        let mut t = TileDoc::new_flat(h);
        // the edge samples are shared with the neighbours
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if let Some(nb) = self.tiles.get(&(k.0 + dx, k.1 + dy)) {
                let s = TERRAIN_SAMPLES;
                for i in 0..s {
                    let (mine, theirs) = match (dx, dy) {
                        (-1, _) => ((0, i), (s - 1, i)),
                        (1, _) => ((s - 1, i), (0, i)),
                        (_, -1) => ((i, 0), (i, s - 1)),
                        _ => ((i, s - 1), (i, 0)),
                    };
                    t.terrain.heights[mine.1 * s + mine.0] = nb.terrain.height_at(theirs.0, theirs.1);
                }
            }
        }
        t.masks = vec![None; self.global.ground_textures.len()];
        self.tiles.insert(k, t);
        if !self.order.contains(&k) {
            self.order.push(k);
        }
        self.global_dirty = true;
        self.global_rev = next_rev();
    }

    /// Take a tile away. Its place in the `[map]` list goes too, and the numbers that name
    /// tiles by their place (entry points, attachment repeaters) follow.
    pub fn remove_tile(&mut self, k: TileKey) {
        let Some(gone) = self.tiles.remove(&k) else { return };
        // roads running on into it end where it was
        let ids: std::collections::HashSet<i64> = gone.tile.splines.iter().map(|s| s.id).collect();
        for t in self.tiles.values_mut() {
            let mut hit = false;
            for s in t.tile.splines.iter_mut() {
                for l in [&mut s.prev_id, &mut s.next_id] {
                    if *l != 0 && ids.contains(l) {
                        *l = 0;
                        hit = true;
                    }
                }
            }
            if hit {
                t.touch_content();
            }
        }
        let old = self.order.clone();
        self.order.retain(|o| *o != k);
        let remap = |i: usize| -> Option<usize> {
            let key = *old.get(i)?;
            self.order.iter().position(|o| *o == key)
        };
        let mut eps = std::mem::take(&mut self.global.entry_points);
        eps.retain_mut(|e| match remap(e.group.max(0) as usize) {
            Some(n) => {
                e.group = n as i32;
                true
            }
            None => false,
        });
        self.global.entry_points = eps;
        let order = self.order.clone();
        for t in self.tiles.values_mut() {
            let mut changed = false;
            t.tile.spline_attachments.retain_mut(|a| match a.repeater {
                Some((ti, first)) => match old.get(ti).and_then(|key| order.iter().position(|o| o == key)) {
                    Some(n) => {
                        if n != ti {
                            a.repeater = Some((n, first));
                            changed = true;
                        }
                        true
                    }
                    None => {
                        changed = true;
                        false
                    }
                },
                None => true,
            });
            if changed {
                t.touch_content();
            }
        }
        self.global_dirty = true;
        self.global_rev = next_rev();
    }

    /// Ground height at a world position (None: no tile there).
    pub fn height_at(&self, p: DVec2) -> Option<f32> {
        let k = tile_of(p);
        let t = self.tiles.get(&k)?;
        let o = tile_origin(k);
        Some(t.height_at(p.x - o.x, p.y - o.y))
    }

    /// Ground height, or 0 outside the map.
    pub fn ground(&self, p: DVec2) -> f64 {
        self.height_at(p).unwrap_or(0.0) as f64
    }

    pub fn object(&self, k: TileKey, id: i64) -> Option<&MapObject> {
        self.tiles.get(&k)?.tile.objects.iter().find(|o| o.id == id)
    }

    pub fn object_mut(&mut self, k: TileKey, id: i64) -> Option<&mut MapObject> {
        self.tiles.get_mut(&k)?.tile.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn spline(&self, k: TileKey, id: i64) -> Option<&MapSpline> {
        self.tiles.get(&k)?.tile.splines.iter().find(|s| s.id == id && !s.deleted)
    }

    /// A `[splineAttachement]` record by its IDCode.
    pub fn attachment(&self, k: TileKey, id: i64) -> Option<&SplineAttachment> {
        self.tiles.get(&k)?.tile.spline_attachments.iter().find(|a| a.id == id)
    }

    pub fn attachment_mut(&mut self, k: TileKey, id: i64) -> Option<&mut SplineAttachment> {
        self.tiles.get_mut(&k)?.tile.spline_attachments.iter_mut().find(|a| a.id == id)
    }

    /// Take a `[splineAttachement]` record away, with whatever hangs on its objects.
    pub fn delete_attachment(&mut self, k: TileKey, id: i64) {
        let Some(t) = self.tiles.get_mut(&k) else { return };
        let Some(i) = t.tile.spline_attachments.iter().position(|a| a.id == id) else { return };
        let a = t.tile.spline_attachments.remove(i);
        let mut gone = vec![a.id];
        let mut j = 0;
        while j < gone.len() {
            let p = gone[j];
            gone.extend(t.tile.attach_objects.iter().filter(|o| o.parent_id == Some(p)).map(|o| o.id));
            j += 1;
        }
        t.tile.attach_objects.retain(|o| !gone.contains(&o.id));
        t.touch_content();
    }

    /// The world position of an object (its ground height added unless `abs`).
    pub fn object_world(&self, k: TileKey, o: &MapObject, abs: bool) -> DVec3 {
        let org = tile_origin(k);
        let p = DVec2::new(org.x + o.pos[0], org.y + o.pos[1]);
        let z = if abs { o.pos[2] } else { self.ground(p) + o.pos[2] };
        DVec3::new(p.x, p.y, z)
    }

    /// Remove a spline, unlink its neighbours and drop the attachment rows that ran on it.
    pub fn delete_spline(&mut self, k: TileKey, id: i64) {
        let Some(t) = self.tiles.get_mut(&k) else { return };
        let Some(idx) = t.tile.splines.iter().position(|s| s.id == id) else { return };
        let s = t.tile.splines.remove(idx);
        // its rows, and what hangs on their objects
        let mut gone: Vec<i64> = t.tile.spline_attachments.iter().filter(|a| a.spline_index == idx as i32).map(|a| a.id).collect();
        let mut i = 0;
        while i < gone.len() {
            let p = gone[i];
            gone.extend(t.tile.attach_objects.iter().filter(|o| o.parent_id == Some(p)).map(|o| o.id));
            i += 1;
        }
        t.tile.attach_objects.retain(|o| !gone.contains(&o.id));
        t.tile.spline_attachments.retain(|a| a.spline_index != idx as i32);
        for a in &mut t.tile.spline_attachments {
            if a.spline_index > idx as i32 {
                a.spline_index -= 1;
            }
        }
        t.touch_content();
        for other in [s.prev_id, s.next_id] {
            if other == 0 {
                continue;
            }
            for t in self.tiles.values_mut() {
                let mut hit = false;
                for o in t.tile.splines.iter_mut().filter(|o| o.id == other) {
                    if o.prev_id == id {
                        o.prev_id = 0;
                        hit = true;
                    }
                    if o.next_id == id {
                        o.next_id = 0;
                        hit = true;
                    }
                }
                if hit {
                    t.touch_content();
                }
            }
        }
    }

    /// Cut a spline in two at distance `at` along it (the second half gets a new IDCode and
    /// follows it in the chain; heights, gradients, cant and texture run on unchanged).
    pub fn split_spline(&mut self, k: TileKey, id: i64, at: f64) -> Option<i64> {
        let new = self.new_id();
        let org = tile_origin(k);
        let t = self.tiles.get_mut(&k)?;
        let idx = t.tile.splines.iter().position(|s| s.id == id)?;
        let a = t.tile.splines[idx].clone();
        if at < 0.5 || at > a.length - 0.5 {
            return None;
        }
        let c = omsi_geometry::SplineCurve::from_map(&a, org);
        let p = c.point_at(at);
        let grad = c.slope_at(at) * 100.0;
        let z_end = c.point_at(a.length).z;
        let cant = c.cant_at(at);
        let mut b = a.clone();
        b.id = new;
        b.pos = [p.x - org.x, p.y - org.y, p.z];
        b.heading = c.heading_at(at).rem_euclid(360.0);
        b.length = a.length - at;
        b.grad_start = grad;
        b.cant_start = cant;
        b.skew_start = 0.0;
        b.prev_id = a.id;
        // (a [spline_h] halves exactly: a cubic is fixed by its ends' heights and slopes)
        if a.is_h {
            b.delta_h = Some(z_end - p.z);
        }
        b.tex_offset = a.tex_offset + at;
        b.map_chain_offset = a.map_chain_offset.map(|o| o + at);
        let old_next = a.next_id;
        {
            let a = &mut t.tile.splines[idx];
            if a.is_h {
                a.delta_h = Some(p.z - a.pos[2]);
            }
            a.length = at;
            a.grad_end = grad;
            a.cant_end = cant;
            a.skew_end = 0.0;
            a.next_id = new;
        }
        t.tile.splines.push(b);
        t.touch_content();
        // the spline that came after it now follows the second half
        if old_next != 0 {
            for t in self.tiles.values_mut() {
                let mut hit = false;
                for s in t.tile.splines.iter_mut().filter(|s| s.id == old_next) {
                    if s.prev_id == id {
                        s.prev_id = new;
                        hit = true;
                    } else if s.next_id == id {
                        s.next_id = new;
                        hit = true;
                    }
                }
                if hit {
                    t.touch_content();
                }
            }
        }
        Some(new)
    }

    /// Join spline `id` and the one after it into one spline, as if it had been laid in one
    /// go: the rows of objects on the second carried over. Only where the two make one curve
    /// (same type, bend, and a climb and banking that run on evenly); otherwise why not.
    pub fn join_splines(&mut self, k: TileKey, id: i64) -> Result<(), &'static str> {
        let a = self.spline(k, id).cloned().ok_or("")?;
        let Some((bk, b)) = self.find_spline(a.next_id).map(|(bk, b)| (bk, b.clone())) else {
            return Err(tr("Nothing comes after it to join", "За ней нечего присоединять"));
        };
        if b.prev_id != a.id || b.id == a.id {
            return Err(tr("The next piece runs the other way: reverse it first", "Следующий кусок идёт в другую сторону: сначала разверните его"));
        }
        if bk != k {
            return Err(tr("The next piece lies on another tile", "Следующий кусок лежит на другой плитке"));
        }
        if !a.file.eq_ignore_ascii_case(&b.file) || a.mirror != b.mirror {
            return Err(tr("The two pieces are of different road types", "Куски разных типов дорог"));
        }
        if a.is_h != b.is_h || a.rules != b.rules || a.terrain_align != b.terrain_align || a.terrain_align_flag != b.terrain_align_flag || a.skew_end != 0.0 || b.skew_start != 0.0 {
            return Err(tr("The two pieces have different settings (path rules, ground, skew)", "У кусков разные настройки (правила путей, земля, скос)"));
        }
        let t = self.tiles.get(&k).ok_or("")?;
        let ib = t.tile.splines.iter().position(|s| s.id == b.id).ok_or("")?;
        if t.tile.spline_attachments.iter().any(|r| r.spline_index == ib as i32 && r.repeater.is_some()) {
            return Err(tr("A row of objects comes onto the next piece from another tile", "На следующий кусок заходит ряд объектов с другой плитки"));
        }
        let org = tile_origin(k);
        let mut m = a.clone();
        m.length = a.length + b.length;
        m.grad_end = b.grad_end;
        m.cant_end = b.cant_end;
        m.skew_end = b.skew_end;
        m.next_id = b.next_id;
        if a.is_h {
            let (ca, cb) = (omsi_geometry::SplineCurve::from_map(&a, org), omsi_geometry::SplineCurve::from_map(&b, org));
            m.delta_h = Some(cb.point_at(b.length).z - ca.point_at(0.0).z);
        }
        // the joined curve must lie where the two did (a bend's radius, the climb, the banking)
        let (ca, cb, cm) = (omsi_geometry::SplineCurve::from_map(&a, org), omsi_geometry::SplineCurve::from_map(&b, org), omsi_geometry::SplineCurve::from_map(&m, org));
        for i in 0..=8 {
            let f = i as f64 / 8.0;
            let checks = [(&ca, f * a.length, f * a.length), (&cb, f * b.length, a.length + f * b.length)];
            for (c, own, joined) in checks {
                let off = (c.point_at(own) - cm.point_at(joined)).length();
                if off > 0.05 || (c.cant_at(own) - cm.cant_at(joined)).abs() > 0.05 {
                    return Err(tr("The two pieces do not bend, climb or bank alike: joined they would change shape", "Куски изгибаются, поднимаются или наклоняются по-разному: при соединении изменилась бы форма"));
                }
            }
        }
        // the second's rows measured along the joined spline (and the objects hanging on them stay)
        let ia = t.tile.splines.iter().position(|s| s.id == a.id).ok_or("")?;
        let shift = a.map_chain_offset.unwrap_or(0.0) + a.length - b.map_chain_offset.unwrap_or(0.0);
        let after = b.next_id;
        let t = self.tiles.get_mut(&k).unwrap();
        for r in t.tile.spline_attachments.iter_mut().filter(|r| r.spline_index == ib as i32) {
            r.spline_index = ia as i32;
            r.offset[2] += shift;
        }
        // (the one after the second followed it, by its start or its end: now the joined one)
        let by_start = self.find_spline(after).map(|(ck, c)| (ck, c.prev_id == b.id));
        self.delete_spline(k, b.id);
        if let Some((ck, by_start)) = by_start {
            let t = self.tiles.get_mut(&ck).unwrap();
            if let Some(c) = t.tile.splines.iter_mut().find(|c| c.id == after && !c.deleted) {
                if by_start {
                    c.prev_id = a.id;
                } else {
                    c.next_id = a.id;
                }
            }
            t.touch_content();
        }
        let t = self.tiles.get_mut(&k).unwrap();
        if let Some(s) = t.tile.splines.iter_mut().find(|s| s.id == a.id) {
            *s = m;
        }
        t.touch_content();
        Ok(())
    }

    /// Find a spline by IDCode anywhere on the map.
    pub fn find_spline(&self, id: i64) -> Option<(TileKey, &MapSpline)> {
        if id == 0 {
            return None;
        }
        self.tiles.iter().find_map(|(k, t)| t.tile.splines.iter().find(|s| s.id == id && !s.deleted).map(|s| (*k, s)))
    }

    /// Cut a chain of splines at distance `at` along spline `id`: the two sides that now end
    /// there, each as (tile, spline, its end lies there rather than its start). Inside a
    /// spline it is split in two; at a joint the two splines already meeting there are the
    /// sides (no cut); at a free end there is only one side.
    pub fn cut_at(&mut self, k: TileKey, id: i64, at: f64) -> (Option<(TileKey, i64, bool)>, Option<(TileKey, i64, bool)>) {
        let Some(s) = self.spline(k, id).cloned() else { return (None, None) };
        if at > 0.5 && at < s.length - 0.5 {
            if let Some(b) = self.split_spline(k, id, at) {
                return (Some((k, id, true)), Some((k, b, false)));
            }
        }
        // (a neighbour's side at the joint: its end if it runs on into this spline)
        let side = |doc: &Document, other: i64| doc.find_spline(other).map(|(nk, n)| (nk, n.id, n.next_id == id));
        if at * 2.0 >= s.length {
            (Some((k, id, true)), side(self, s.next_id))
        } else {
            (side(self, s.prev_id), Some((k, id, false)))
        }
    }

    /// Take `by` metres off a chain of splines at one spline's end (`at_end`) or start, the
    /// splines that lie wholly inside going too. Returns the spline (tile, id, at its end)
    /// whose end is now where the chain stops.
    pub fn trim(&mut self, k: TileKey, id: i64, at_end: bool, by: f64) -> Option<(TileKey, i64, bool)> {
        let mut cur = (k, id, at_end);
        let mut left = by;
        for _ in 0..64 {
            let s = self.spline(cur.0, cur.1)?.clone();
            // (the cut falls on a joint, or as near one as a spline can be cut: the chain
            // stops there)
            if left <= 0.5 {
                return Some(cur);
            }
            if left < s.length - 0.5 {
                if cur.2 {
                    let b = self.split_spline(cur.0, cur.1, s.length - left)?;
                    self.delete_spline(cur.0, b);
                } else {
                    let b = self.split_spline(cur.0, cur.1, left)?;
                    self.delete_spline(cur.0, cur.1);
                    cur.1 = b;
                }
                return Some(cur);
            }
            // the whole spline goes: on along the chain
            let next = if cur.2 { s.prev_id } else { s.next_id };
            let neighbour = self.find_spline(next).map(|(nk, n)| (nk, n.id, n.next_id == cur.1));
            left -= s.length;
            self.delete_spline(cur.0, cur.1);
            cur = neighbour?;
        }
        None
    }

    /// Remove an object and whatever hangs on it.
    pub fn delete_object(&mut self, k: TileKey, id: i64) {
        let Some(t) = self.tiles.get_mut(&k) else { return };
        t.tile.objects.retain(|o| o.id != id);
        // (and what hangs on those, in turn)
        let mut gone = vec![id];
        let mut i = 0;
        while i < gone.len() {
            let p = gone[i];
            gone.extend(t.tile.attach_objects.iter().filter(|o| o.parent_id == Some(p)).map(|o| o.id));
            i += 1;
        }
        t.tile.attach_objects.retain(|o| !gone[1..].contains(&o.id));
        t.touch_content();
        self.global.entry_points.retain(|e| e.object_id != id);
        self.global_dirty = true;
    }

    /// Make sure every tile has a mask slot per ground layer.
    pub fn sync_mask_slots(&mut self) {
        let n = self.global.ground_textures.len();
        for t in self.tiles.values_mut() {
            t.masks.resize(n, None);
        }
    }
}

/// A tile's patches in the given chrono folders, in order.
fn chrono_patches(chrono_dirs: &[PathBuf], file: &str) -> Vec<Tile> {
    chrono_dirs
        .iter()
        .filter_map(|c| {
            let p = omsi_cfg::resolve_path(c, file);
            omsi_cfg::vfs::is_file(&p).then(|| Tile::load(&p).ok()).flatten()
        })
        .collect()
}

/// One tile of the map with its ground, painting and chrono patches (None: no file).
pub(crate) fn load_tile(src_dir: &Path, global: &GlobalCfg, chrono_dirs: &[PathBuf], t: &omsi_map::MapTileRef) -> Option<TileDoc> {
    let path = src_dir.join(&t.file);
    let mut tile = match Tile::load(&path) {
        Ok(t) => t,
        Err(e) => {
            // (maps list tiles they do not ship: Berlin-Spandau 369, 329 of them there)
            log::debug!("{e}");
            return None;
        }
    };
    // (a [worldcoordinates] tile is scaled onto the map's grid, as the game does; it is
    // scaled back when saved)
    tile.fit_to_world_grid(t.y);
    let chrono = chrono_patches(chrono_dirs, &t.file);
    let terrain_path = src_dir.join(format!("{}.terrain", t.file));
    let terrain = Terrain::load(&terrain_path).ok().filter(|t| t.samples() == TERRAIN_SAMPLES).unwrap_or_else(Terrain::flat);
    let mut masks = Vec::new();
    for (li, g) in global.ground_textures.iter().enumerate() {
        if li == 0 {
            masks.push(None);
            continue;
        }
        let p = src_dir.join("texture").join("map").join(format!("{}.{}.dds", t.file, li));
        masks.push(load_mask(&p, mask_size(g)));
    }
    // the water, as the game reads it: only a `[water]` tile has one; its corners start at
    // -5 m and take the file's heights when it says it has one surface (openOMSI's stage)
    let water = tile.has_water.then(|| match omsi_cfg::vfs::read(&src_dir.join(format!("{}.water", t.file))).ok().map(|b| omsi_map::terrain::Water::parse(&b)) {
        Some(w) if w.count == 1 && w.values.len() >= 4 => [w.values[0], w.values[1], w.values[2], w.values[3]],
        _ => [-5.0; 4],
    });
    Some(TileDoc { tile, chrono, water, terrain, masks, terrain_rev: next_rev(), mask_rev: next_rev(), content_rev: next_rev(), dirty: false })
}

/// A painting mask from its `.dds` (or any picture), as alpha texels of `size` × `size`.
fn load_mask(p: &Path, size: usize) -> Option<Vec<u8>> {
    if !p.is_file() {
        return None;
    }
    let img = omsi_texture::decode_file(p).ok()?;
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let mut out = vec![0u8; size * size];
    for y in 0..size {
        for x in 0..size {
            let sx = x * w / size;
            let sy = y * h / size;
            let px = &img.rgba[(sy * w + sx) * 4..][..4];
            // an alpha-only DDS decodes into the alpha channel; a grey picture into rgb
            out[y * size + x] = if img.has_alpha { px[3] } else { px[0] };
        }
    }
    Some(out)
}

/// The inverse of `Tile::fit_to_world_grid`: back into the tile's own measure, as its file
/// keeps it.
pub fn unfit_from_world_grid(t: &mut Tile, ty: i32) {
    let (kx, ky) = omsi_map::world_tile_scale(ty);
    if (kx - 1.0).abs() < 1e-12 && (ky - 1.0).abs() < 1e-12 {
        return;
    }
    let k = (kx + ky) / 2.0;
    for o in t.objects.iter_mut().chain(t.attach_objects.iter_mut()) {
        o.pos[0] /= kx;
        o.pos[1] /= ky;
    }
    for s in t.splines.iter_mut() {
        s.pos[0] /= kx;
        s.pos[1] /= ky;
        s.length /= k;
        s.radius /= k;
    }
    for a in t.spline_attachments.iter_mut() {
        a.offset[2] /= k;
        a.interval /= k;
        a.range /= k;
    }
}

/// The map's grid is global (`omsi_map::configure_grid`): tests that read or make a map
/// hold this, one map at a time (a thread holding it already gets it again).
#[cfg(test)]
pub fn one_map() -> OneMap {
    static MAP: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if HELD.with(|h| h.get()) {
        return OneMap(None);
    }
    let g = MAP.lock().unwrap_or_else(|e| e.into_inner());
    HELD.with(|h| h.set(true));
    OneMap(Some(g))
}

#[cfg(test)]
thread_local! {
    static HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static KEPT: std::cell::RefCell<Option<OneMap>> = const { std::cell::RefCell::new(None) };
}

/// The hold kept until the test's thread ends (for helpers that make a map to test on).
#[cfg(test)]
pub fn hold_map_for_test() {
    if !HELD.with(|h| h.get()) {
        let g = one_map();
        KEPT.with(|k| *k.borrow_mut() = Some(g));
    }
}

#[cfg(test)]
/// The hold on the map-wide grid; let go, the grid is the plain one again (a map in world
/// coordinates read under it must not leave its tile size to the tests after it).
pub struct OneMap(#[allow(dead_code)] Option<std::sync::MutexGuard<'static, ()>>);

#[cfg(test)]
impl Drop for OneMap {
    fn drop(&mut self) {
        if self.0.is_some() {
            omsi_map::set_world_coordinates(false);
            omsi_map::set_tile_size(omsi_map::TILE_SIZE);
            let _ = HELD.try_with(|h| h.set(false));
        }
    }
}

/// Today as OMSI's date code (YYYYMMDD).
pub fn today_code() -> i32 {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    civil(days)
}

/// A day (since 1970-01-01) as YYYYMMDD.
pub fn civil(days: i64) -> i32 {
    // (civil date from days since 1970-01-01, H. Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y * 10_000 + m * 100 + d) as i32
}
