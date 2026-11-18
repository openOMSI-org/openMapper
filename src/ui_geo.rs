//! The "Satellite and terrain" window: where the map lies on the Earth (a map to search
//! and click on), satellite imagery and terrain heights loaded for its tiles, the aerial
//! photos OMSI's own editor already has for the map, the user's own pictures, and the
//! overlay on the ground with its visibility and opacity per tile.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use glam::DVec2;

use crate::doc::{tile_origin, tile_size, TileKey};
use crate::editor::Editor;
use crate::geo::fetch::{self, Place};
use crate::geo::job::{self, Job, Msg};
use crate::geo::own::{self, Fit, Picture};
use crate::geo::sources::{self, GeoPrefs, Kind, Source};
use crate::geo::{self as g, Anchor, GeoFile, Georef};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Prefs;
use crate::render::imagery::{Imagery, Mips};
use crate::theme::pal;

/// What the main interface (the View menu, the Tiles tool) shows of the overlay and asks of
/// it; filled by [`GeoUi::fill_bridge`] before the frame, read by [`GeoUi::take_bridge`].
#[derive(Default)]
pub struct Bridge {
    pub overlay: bool,
    /// The map lies on the Earth (imagery can be loaded).
    pub placed: bool,
    pub tile: Option<TileRow>,
    pub requests: Vec<Request>,
}

pub struct TileRow {
    pub key: TileKey,
    pub visible: bool,
    pub opacity: f32,
    /// What is laid on it, in words.
    pub what: String,
    pub placed: bool,
}

pub enum Request {
    Open,
    Overlay(bool),
    Visible(TileKey, bool),
    Opacity(TileKey, f32),
    LoadImagery(TileKey),
    /// The OpenStreetMap map (as the site draws it) for a tile, or for all tiles (None).
    LoadOsmMap(Option<TileKey>),
    OwnPicture(TileKey),
}

/// View menu: the overlay on or off.
pub fn view_menu(ui: &mut egui::Ui, b: &mut Bridge) {
    let mut on = b.overlay;
    if crate::ui::check(ui, &mut on, icons::labelled(Icon::Layers, tr("Satellite overlay", "Спутниковая подложка"))).changed() {
        b.requests.push(Request::Overlay(on));
    }
}

/// The Tiles tool's panel: the overlay of the tile clicked last.
pub fn tiles_panel(ui: &mut egui::Ui, b: &mut Bridge) {
    ui.add_space(10.0);
    ui.label(RichText::new(tr("Satellite and own pictures", "Спутник и свои картинки")).strong());
    let Some(t) = &b.tile else {
        ui.label(RichText::new(tr("Click a tile to see its overlay here.", "Кликните тайл, чтобы увидеть здесь его подложку.")).color(pal().text_dim));
        osm_map_row(ui, b, None);
        if ui.button(tr("Satellite and terrain…", "Спутник и рельеф…")).clicked() {
            b.requests.push(Request::Open);
        }
        return;
    };
    let k = t.key;
    ui.label(format!("{} {}, {}: {}", tr("Tile", "Тайл"), k.0, k.1, t.what));
    let mut vis = t.visible;
    if crate::ui::check(ui, &mut vis, tr("Show the overlay", "Показывать подложку")).changed() {
        b.requests.push(Request::Visible(k, vis));
    }
    let mut op = t.opacity;
    if ui.add(egui::Slider::new(&mut op, 0.0..=1.0).text(tr("opacity", "непрозрачность"))).changed() {
        b.requests.push(Request::Opacity(k, op));
    }
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(t.placed, egui::Button::new(tr("Load satellite imagery", "Загрузить спутник"))).on_disabled_hover_text(tr("Place the map on the Earth first (Satellite and terrain…)", "Сначала разместите карту на Земле (Спутник и рельеф…)")).clicked() {
            b.requests.push(Request::LoadImagery(k));
        }
        if ui.button(tr("Load image file…", "Загрузить картинку…")).on_hover_text(tr("Your own picture for this tile (a scan, a screenshot): or drop it on the window", "Своя картинка для этого тайла (скан, скриншот): или перетащите её в окно")).clicked() {
            b.requests.push(Request::OwnPicture(k));
        }
        if ui.button(tr("Satellite and terrain…", "Спутник и рельеф…")).clicked() {
            b.requests.push(Request::Open);
        }
    });
    osm_map_row(ui, b, Some(k));
}

/// The map as the OpenStreetMap site draws it, laid on the ground like a satellite picture: its
/// streets, names, signs and houses to build over (for this tile, or for all of them).
fn osm_map_row(ui: &mut egui::Ui, b: &mut Bridge, tile: Option<TileKey>) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let label = if tile.is_some() { tr("Load the OpenStreetMap map", "Загрузить карту OpenStreetMap") } else { tr("OpenStreetMap map for all tiles", "Карта OpenStreetMap для всех тайлов") };
        let r = ui.add_enabled(b.placed, egui::Button::new(icons::labelled(Icon::Map, label)));
        let r = r.on_hover_text(tr("The map as openstreetmap.org draws it (streets with their names, houses, water, green, stop signs), drawn sharp from OpenStreetMap data and laid on the tile in place of a satellite picture: to build roads and houses over by eye. Nothing is generated from it.", "Карта как на openstreetmap.org (улицы с названиями, дома, вода, зелень, значки остановок) рисуется чётко из данных OpenStreetMap и кладётся на тайл вместо спутника: чтобы строить дороги и дома поверх. Ничего не генерируется."));
        let r = r.on_disabled_hover_text(tr("Place the map on the Earth first (Satellite and terrain…)", "Сначала разместите карту на Земле (Спутник и рельеф…)"));
        if r.clicked() {
            b.requests.push(Request::LoadOsmMap(tile));
        }
    });
}

/// Pictures for the image file dialog.
const PICTURES: [&str; 7] = ["jpg", "jpeg", "png", "tif", "tiff", "bmp", "webp"];

pub fn is_picture(p: &Path) -> bool {
    p.extension().map(|e| PICTURES[..6].iter().any(|x| e.eq_ignore_ascii_case(x))).unwrap_or(false)
}

// ------------------------------------------------------------------ the slippy map

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct TileId(String, u8, u32, u32);

enum Tex {
    Loading,
    Ready(egui::TextureHandle, u64),
    Missing,
}

type Want = (Arc<Source>, Arc<GeoPrefs>, TileId);

/// The map of the world in the window: cached imagery tiles, dragged and zoomed.
struct Slippy {
    /// Web Mercator (0..1) at the middle.
    centre: DVec2,
    zoom: f64,
    tex: HashMap<TileId, Tex>,
    queue: Arc<(Mutex<Vec<Want>>, Condvar)>,
    tx: Sender<(TileId, Option<egui::ColorImage>)>,
    rx: Receiver<(TileId, Option<egui::ColorImage>)>,
    started: bool,
    frame: u64,
}

impl Slippy {
    fn new() -> Slippy {
        let (tx, rx) = channel();
        Slippy { centre: DVec2::new(0.5, 0.36), zoom: 3.0, tex: HashMap::new(), queue: Arc::new((Mutex::new(Vec::new()), Condvar::new())), tx, rx, started: false, frame: 0 }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        for _ in 0..3 {
            let q = self.queue.clone();
            let tx = self.tx.clone();
            std::thread::spawn(move || loop {
                let (src, prefs, id) = {
                    let (m, cv) = &*q;
                    let mut v = m.lock().unwrap();
                    while v.is_empty() {
                        v = cv.wait(v).unwrap();
                    }
                    // (the newest wish first: what is on the screen now)
                    v.pop().unwrap()
                };
                let img = match fetch::tile(&src, &prefs, id.1, id.2, id.3) {
                    Ok(fetch::Tile::Data(b)) => image::load_from_memory(&b).ok().map(|i| {
                        let i = i.to_rgba8();
                        egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw())
                    }),
                    _ => None,
                };
                if tx.send((id, img)).is_err() {
                    return;
                }
            });
        }
    }

    fn scale(&self) -> f64 {
        256.0 * 2f64.powf(self.zoom)
    }

    fn to_screen(&self, rect: Rect, m: DVec2) -> Pos2 {
        let d = (m - self.centre) * self.scale();
        rect.center() + Vec2::new(d.x as f32, d.y as f32)
    }

    fn to_merc(&self, rect: Rect, p: Pos2) -> DVec2 {
        let d = p - rect.center();
        self.centre + DVec2::new(d.x as f64, d.y as f64) / self.scale()
    }

    fn look_at(&mut self, lat: f64, lon: f64, zoom: f64) {
        self.centre = g::merc_of(lat, lon);
        self.zoom = zoom;
    }

    /// Draw the tiles of `src` under `rect`; returns the response.
    fn show(&mut self, ui: &mut egui::Ui, size: Vec2, src: &Arc<Source>, prefs: &Arc<GeoPrefs>) -> (egui::Response, Rect) {
        self.start();
        self.frame += 1;
        let ctx = ui.ctx().clone();
        while let Ok((id, img)) = self.rx.try_recv() {
            let t = match img {
                Some(img) => Tex::Ready(ctx.load_texture(format!("geo {}/{}/{}/{}", id.0, id.1, id.2, id.3), img, egui::TextureOptions::LINEAR), self.frame),
                None => Tex::Missing,
            };
            self.tex.insert(id, t);
        }
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if resp.dragged() {
            let d = resp.drag_delta();
            self.centre -= DVec2::new(d.x as f64, d.y as f64) / self.scale();
        }
        if resp.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            // (the wheel over the map is the map's: the panel round it must not scroll with it)
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            let dz = scroll as f64 / 90.0 + (pinch as f64).log2();
            if dz != 0.0 {
                let at = resp.hover_pos().map(|p| self.to_merc(rect, p)).unwrap_or(self.centre);
                let before = self.to_screen(rect, at);
                self.zoom = (self.zoom + dz).clamp(1.0, 21.5);
                let after = self.to_screen(rect, at);
                let d = after - before;
                self.centre += DVec2::new(d.x as f64, d.y as f64) / self.scale();
            }
        }
        self.centre = self.centre.clamp(DVec2::ZERO, DVec2::ONE);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, pal().view);
        let z = (self.zoom.round() as i64).clamp(0, src.max_zoom as i64) as u8;
        let n = 1u32 << z;
        let lo = self.to_merc(rect, rect.min);
        let hi = self.to_merc(rect, rect.max);
        let cell = |v: f64| ((v * n as f64).floor().max(0.0) as u32).min(n - 1);
        let mut wants: Vec<Want> = Vec::new();
        for y in cell(lo.y)..=cell(hi.y) {
            for x in cell(lo.x)..=cell(hi.x) {
                let id = TileId(src.name.clone(), z, x, y);
                let r = Rect::from_min_max(self.to_screen(rect, DVec2::new(x as f64, y as f64) / n as f64), self.to_screen(rect, DVec2::new((x + 1) as f64, (y + 1) as f64) / n as f64));
                match self.tex.get_mut(&id) {
                    Some(Tex::Ready(t, used)) => {
                        *used = self.frame;
                        painter.image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        continue;
                    }
                    Some(Tex::Loading) => {}
                    Some(Tex::Missing) => {}
                    None => {
                        self.tex.insert(id.clone(), Tex::Loading);
                        wants.push((src.clone(), prefs.clone(), id.clone()));
                    }
                }
                // (meanwhile a coarser tile, enlarged)
                for up in 1..=6u8.min(z) {
                    let pid = TileId(src.name.clone(), z - up, x >> up, y >> up);
                    if let Some(Tex::Ready(t, used)) = self.tex.get_mut(&pid) {
                        *used = self.frame;
                        let f = 1.0 / (1u32 << up) as f32;
                        let (ox, oy) = ((x & ((1 << up) - 1)) as f32 * f, (y & ((1 << up) - 1)) as f32 * f);
                        painter.image(t.id(), r, Rect::from_min_max(Pos2::new(ox, oy), Pos2::new(ox + f, oy + f)), Color32::WHITE);
                        break;
                    }
                }
            }
        }
        if !wants.is_empty() {
            let (m, cv) = &*self.queue;
            let mut q = m.lock().unwrap();
            // (what was wished before and not fetched yet is forgotten: asked again if it
            // is still on the screen)
            for w in q.drain(..) {
                self.tex.remove(&w.2);
            }
            q.extend(wants);
            cv.notify_all();
        }
        // (at most a few hundred textures kept)
        if self.tex.len() > 500 {
            let mut used: Vec<(u64, TileId)> = self.tex.iter().filter_map(|(k, t)| if let Tex::Ready(_, u) = t { Some((*u, k.clone())) } else { None }).collect();
            used.sort();
            for (_, k) in used.into_iter().take(150) {
                self.tex.remove(&k);
            }
        }
        (resp, rect)
    }
}

// ------------------------------------------------------------------ the window

/// A probe of the detail each source has at a place: (lat, lon) and per source its finest
/// zoom (None: no imagery there) or why it could not be asked.
struct Probe {
    at: (f64, f64),
    results: BTreeMap<String, Result<Option<u8>, String>>,
    rx: Option<Receiver<(String, Result<Option<u8>, String>)>>,
}

/// A bake of an own picture's pieces.
struct Bake {
    rx: Receiver<(TileKey, Mips)>,
    full: bool,
}

/// A folder of OMSI's photos being copied into the map on a thread of its own.
struct AerialJob {
    rx: Receiver<std::io::Result<Vec<TileKey>>>,
    /// How many are done (the thread sets it after each).
    at: Arc<AtomicUsize>,
    total: usize,
    folder: PathBuf,
    /// The placement to record with them.
    stamp: u64,
}

pub struct GeoUi {
    pub open: bool,
    /// A click on the map of the world: the 3D view goes there.
    pub look: Option<DVec2>,
    /// The map folder `file` belongs to.
    dir: Option<PathBuf>,
    pub file: GeoFile,
    save_at: Option<Instant>,
    /// The tiles seen last frame (new ones get imagery by themselves).
    known: BTreeSet<TileKey>,
    // the overlay on the ground
    epoch: u64,
    /// What each tile's texture shows (its file and the file's time).
    shown: HashMap<TileKey, (PathBuf, Option<std::time::SystemTime>)>,
    load_tx: Sender<(PathBuf, TileKey, PathBuf, u32)>,
    loaded_rx: Receiver<(PathBuf, TileKey, Option<Mips>)>,
    loading: BTreeSet<TileKey>,
    map: Slippy,
    search: String,
    places: Vec<Place>,
    search_rx: Option<Receiver<Result<Vec<Place>, String>>>,
    probe: Option<Probe>,
    job: Option<Job>,
    /// Tiles waiting for the job running now to end.
    queued: BTreeSet<TileKey>,
    heights: Vec<(TileKey, Vec<f32>)>,
    /// What the last load did (text, an error).
    status: Option<(String, bool)>,
    errors: Vec<String>,
    placing: bool,
    /// Tiles to load whose own pictures want a yes first.
    confirm: Option<(Vec<TileKey>, bool)>,
    /// The detail of the imagery loaded now (m a pixel).
    detail: Option<(u8, f64)>,
    // own pictures
    pic_sel: Option<String>,
    pic_cache: HashMap<String, Arc<image::RgbaImage>>,
    bake: Option<Bake>,
    /// A fit changed: bake a quick preview now and the full pieces when it rests.
    fit_changed: Option<Instant>,
    rect: Option<[i32; 4]>,
    custom: Source,
    /// Google's copyright line for the place.
    google: Option<Receiver<Result<sources::Viewport, String>>>,
    google_line: Option<String>,
    uploads: Vec<(TileKey, Mips)>,
    /// Pictures not kept on disk (Google's), made again at each opening.
    memory: HashMap<TileKey, Arc<image::RgbaImage>>,
    /// Tiles whose kept-nowhere imagery is to be made again.
    rebuild: Vec<TileKey>,
    /// Small copies for the overview map.
    mini: HashMap<TileKey, egui::TextureHandle>,
    mini_pending: Vec<(TileKey, egui::ColorImage)>,
    /// Unloaded tiles beside loaded heights (their shared edge steps).
    edges: Vec<TileKey>,
    /// The edge the pictures on the GPU were put up at (0: none up yet).
    shown_side: u32,
    /// OMSI's own aerial photos for this map: the folder found beside it and how many tiles
    /// it has photos for (None: the map has none).
    aerial: Option<(PathBuf, usize)>,
    /// A folder of that map's photos being copied now (a hundred megabytes is not copied on
    /// the frame's thread).
    aerial_job: Option<AerialJob>,
    /// The tiles whose photos are held, with the place and count they were worked out for.
    near: Option<(HashSet<TileKey>, DVec2, usize)>,
    /// What each tile's picture is, and when it was written: looked at on the disk now and then
    /// rather than in every frame (a map of hundreds of tiles meant a thousand calls a second).
    pictures: HashMap<TileKey, (Option<PathBuf>, Option<std::time::SystemTime>)>,
    /// When the disk was last looked at for them.
    pictures_at: Option<Instant>,
}

impl Default for GeoUi {
    fn default() -> Self {
        let (load_tx, load_rx) = channel::<(PathBuf, TileKey, PathBuf, u32)>();
        let (loaded_tx, loaded_rx) = channel();
        // (one thread reads the pictures from the disk and makes their mipmaps)
        std::thread::spawn(move || {
            while let Ok((dir, k, path, max)) = load_rx.recv() {
                let m = image::open(&path).ok().map(|i| Mips::of(i.to_rgba8(), max));
                if loaded_tx.send((dir, k, m)).is_err() {
                    return;
                }
            }
        });
        GeoUi {
            open: false,
            look: None,
            dir: None,
            file: GeoFile::default(),
            save_at: None,
            known: BTreeSet::new(),
            epoch: 0,
            shown: HashMap::new(),
            load_tx,
            loaded_rx,
            loading: BTreeSet::new(),
            map: Slippy::new(),
            search: String::new(),
            places: Vec::new(),
            search_rx: None,
            probe: None,
            job: None,
            queued: BTreeSet::new(),
            heights: Vec::new(),
            status: None,
            errors: Vec::new(),
            placing: false,
            confirm: None,
            detail: None,
            pic_sel: None,
            pic_cache: HashMap::new(),
            bake: None,
            fit_changed: None,
            rect: None,
            custom: Source { builtin: false, ..sources::builtin()[0].clone() },
            google: None,
            google_line: None,
            uploads: Vec::new(),
            memory: HashMap::new(),
            rebuild: Vec::new(),
            mini: HashMap::new(),
            mini_pending: Vec::new(),
            edges: Vec::new(),
            shown_side: 0,
            aerial: None,
            aerial_job: None,
            near: None,
            pictures: HashMap::new(),
            pictures_at: None,
        }
    }
}

fn ts() -> f64 {
    tile_size()
}

impl GeoUi {
    /// A load is running.
    pub fn busy(&self) -> bool {
        self.job.is_some() || !self.queued.is_empty() || self.bake.is_some() || !self.loading.is_empty() || self.aerial_job.is_some()
    }

    pub(crate) fn georef(&self) -> Option<Georef> {
        self.file.georef(omsi_map::world_coordinates(), ts())
    }

    fn touch(&mut self) {
        self.save_at = Some(Instant::now());
    }

    /// Follow the open map: its geo.json read when another map is opened.
    fn follow(&mut self, ed: &Editor) {
        let dir = ed.doc.as_ref().map(|d| d.dir.clone());
        if dir == self.dir {
            return;
        }
        self.flush();
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        // (a copy of the last map's photos is left to finish into the last map's folder: it is
        // not recorded on this map's tiles)
        self.aerial_job = None;
        self.near = None;
        self.pictures.clear();
        self.pictures_at = None;
        self.dir = dir.clone();
        self.file = dir.as_deref().map(GeoFile::load).unwrap_or_default();
        self.known = ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default();
        self.shown.clear();
        self.epoch = 0;
        self.queued.clear();
        self.heights.clear();
        self.status = None;
        self.errors.clear();
        self.probe = None;
        self.pic_sel = None;
        self.pic_cache.clear();
        self.rect = None;
        self.detail = None;
        self.google_line = None;
        self.memory.clear();
        self.mini.clear();
        self.mini_pending.clear();
        self.edges.clear();
        // (OMSI's own photos for this map, if the map has any: looked at once here, not every
        // frame, as the folder can hold hundreds of files)
        let known = &self.known;
        self.aerial = dir.as_deref().and_then(|d| g::aerial::folder_of(d, ed.doc.as_ref().and_then(|d| d.source.as_deref()))).map(|f| {
            let n = g::aerial::scan(&f).iter().filter(|(k, _)| known.contains(k)).count();
            (f, n)
        });
        // (Google's pictures were never kept: made again from the tile cache, fetched again
        // where it has run out)
        self.rebuild = self.file.tiles.iter().filter(|(_, t)| t.imagery.as_ref().map(|m| !m.kept).unwrap_or(false) && t.own.is_none()).filter_map(|(n, _)| {
            let (x, y) = n.split_once('_')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        }).collect();
        if let Some(geo) = self.georef() {
            let (la, lo) = geo.lat_lon(self.middle(ed));
            self.map.look_at(la, lo, 15.0);
        }
    }

    /// Write geo.json (a moment after the last change).
    fn flush(&mut self) {
        if self.save_at.take().is_some() {
            if let Some(d) = &self.dir {
                if let Err(e) = self.file.save(d) {
                    log::error!("geo.json: {e}");
                }
            }
            // (which tiles have a picture may have changed)
            self.pictures.clear();
            self.pictures_at = None;
        }
    }

    /// The middle of the tile nearest the map's middle.
    fn middle(&self, ed: &Editor) -> DVec2 {
        let Some(doc) = &ed.doc else { return DVec2::ZERO };
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        if keys.is_empty() {
            return DVec2::splat(ts() * 0.5);
        }
        let c = keys.iter().map(|k| tile_origin(*k)).fold(DVec2::ZERO, |a, b| a + b) / keys.len() as f64 + DVec2::splat(ts() * 0.5);
        keys.iter().map(|k| tile_origin(*k) + DVec2::splat(ts() * 0.5)).min_by(|a, b| (*a - c).length().total_cmp(&(*b - c).length())).unwrap()
    }

    fn anchor_tile(&self, ed: &Editor) -> TileKey {
        match &self.file.anchor {
            Some(a) => (a.tile[0], a.tile[1]),
            None => crate::doc::tile_of(self.middle(ed)),
        }
    }

    /// The imagery sources usable here, finest first (with their detail when probed).
    fn ranked(&self, prefs: &GeoPrefs, lat: f64) -> Vec<(Source, Option<f64>)> {
        let mut v: Vec<(Source, Option<f64>)> = prefs
            .all()
            .into_iter()
            .filter(|s| !s.is_terrain() && prefs.usable(s))
            .filter_map(|s| {
                let r = self.probe.as_ref().and_then(|p| p.results.get(&s.name).cloned());
                match r {
                    Some(Ok(None)) => None,
                    Some(Ok(Some(z))) => {
                        let m = s.metres_per_pixel(lat, z);
                        Some((s, Some(m)))
                    }
                    _ => {
                        if let Some(p) = &self.probe {
                            if !s.covers(p.at.0, p.at.1) {
                                return None;
                            }
                        }
                        Some((s, None))
                    }
                }
            })
            .collect();
        sources::rank(&mut v);
        v
    }

    /// The imagery source to load from: the chosen one, else the finest here.
    fn source(&self, prefs: &GeoPrefs, lat: f64) -> Option<Source> {
        let all = prefs.all();
        if let Some(s) = all.iter().find(|s| s.name == self.file.source && prefs.usable(s)) {
            return Some(s.clone());
        }
        self.ranked(prefs, lat).into_iter().next().map(|r| r.0).or_else(|| all.into_iter().find(|s| s.name == sources::ESRI))
    }

    fn probe_here(&mut self, prefs: &GeoPrefs, lat: f64, lon: f64) {
        if self.probe.as_ref().map(|p| (p.at.0 - lat).abs() < 1e-3 && (p.at.1 - lon).abs() < 1e-3).unwrap_or(false) {
            return;
        }
        let (tx, rx) = channel();
        for s in prefs.all().into_iter().filter(|s| !s.is_terrain() && prefs.usable(s) && s.covers(lat, lon)) {
            let tx = tx.clone();
            let p = prefs.clone();
            std::thread::spawn(move || {
                let r = fetch::finest_zoom(&s, &p, lat, lon);
                let _ = tx.send((s.name.clone(), r));
            });
        }
        self.probe = Some(Probe { at: (lat, lon), results: BTreeMap::new(), rx: Some(rx) });
        if prefs.key(sources::GOOGLE).is_some() {
            let (tx, rx) = channel();
            let p = prefs.clone();
            std::thread::spawn(move || {
                let _ = tx.send(fetch::google_viewport(&p, lat, lon, 18));
            });
            self.google = Some(rx);
        }
    }

    /// What a tile shows, in words.
    fn what(&self, k: TileKey) -> String {
        let t = self.file.tile(k);
        if t.own.is_some() {
            return tr("own picture", "своя картинка").into();
        }
        match (&t.imagery, self.georef()) {
            // (a photo of OMSI's is of this tile by its name: it holds wherever the map lies)
            (Some(m), _) if m.source == g::aerial::SOURCE => m.source.clone(),
            (Some(m), Some(geo)) if m.stamp != geo.stamp() => format!("{} z{} ({})", m.source, m.zoom, tr("for an earlier placement", "для прежнего размещения")),
            (Some(m), _) if !m.kept => format!("Google z{} ({})", m.zoom, tr("not kept with the map", "не хранится с картой")),
            (Some(m), _) => format!("{} z{}{}", m.source, m.zoom, if m.coarser { tr(", partly coarser", ", местами грубее") } else { "" }),
            (None, _) => tr("nothing", "ничего").into(),
        }
    }

    /// The file a tile's overlay comes from.
    fn picture_of(&self, dir: &Path, k: TileKey) -> Option<PathBuf> {
        let t = self.file.tile(k);
        if t.own.is_some() {
            return Some(g::own_path(dir, k)).filter(|p| p.is_file());
        }
        t.imagery.as_ref().filter(|m| m.kept).map(|_| g::imagery_path(dir, k)).filter(|p| p.is_file())
    }

    /// [`picture_of`](Self::picture_of) and when the file was written, remembered (the disk is
    /// asked once, not in every frame).
    fn picture_cached(&mut self, dir: &Path, k: TileKey) -> Option<(PathBuf, Option<std::time::SystemTime>)> {
        if let Some((p, t)) = self.pictures.get(&k) {
            return p.clone().map(|p| (p, *t));
        }
        let found = self.picture_of(dir, k).map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, t)
        });
        let entry = (found.as_ref().map(|f| f.0.clone()), found.as_ref().and_then(|f| f.1));
        self.pictures.insert(k, entry);
        found
    }

    // -------------------------------------------------------------- the main interface

    pub fn fill_bridge(&mut self, b: &mut Bridge, ed: &Editor) {
        self.follow(ed);
        b.overlay = self.file.overlay;
        b.placed = self.georef().is_some();
        b.tile = ed.picked_tile.filter(|k| ed.doc.as_ref().map(|d| d.tiles.contains_key(k)).unwrap_or(false)).map(|k| {
            let t = self.file.tile(k);
            TileRow { key: k, visible: t.visible, opacity: t.opacity, what: self.what(k), placed: self.georef().is_some() }
        });
    }

    pub fn take_bridge(&mut self, b: &mut Bridge, ed: &mut Editor, prefs: &Prefs) {
        for r in b.requests.drain(..) {
            match r {
                Request::Open => self.open = true,
                Request::Overlay(on) => {
                    self.file.overlay = on;
                    self.touch();
                }
                Request::Visible(k, v) => {
                    self.file.tile_mut(k).visible = v;
                    self.touch();
                }
                Request::Opacity(k, o) => {
                    self.file.tile_mut(k).opacity = o;
                    self.touch();
                }
                Request::LoadImagery(k) => {
                    // (the satellite button brings the satellite, not the drawn map chosen before)
                    if self.file.source == sources::OSM_STANDARD {
                        self.file.source.clear();
                    }
                    self.load(ed, &prefs.geo, vec![k], true, self.file.with_terrain)
                }
                Request::LoadOsmMap(k) => {
                    let keys: Vec<TileKey> = match k {
                        Some(k) => vec![k],
                        None => ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default(),
                    };
                    self.file.source = sources::OSM_STANDARD.to_string();
                    self.file.overlay = true;
                    self.touch();
                    self.load(ed, &prefs.geo, keys, true, false)
                }
                Request::OwnPicture(k) => self.pick_picture(ed, [k.0, k.1, k.0, k.1]),
            }
        }
    }

    /// A picture dropped on the window with the Tiles tool on: the clicked tile's own
    /// picture. False: not taken (no tile, not a picture).
    pub fn drop_picture(&mut self, path: &Path, ed: &mut Editor) -> bool {
        let Some(k) = ed.picked_tile else { return false };
        if !is_picture(path) || ed.doc.as_ref().map(|d| !d.tiles.contains_key(&k)).unwrap_or(true) {
            return false;
        }
        self.own_picture(ed, path, [k.0, k.1, k.0, k.1]);
        true
    }

    // -------------------------------------------------------------- loading

    /// Load imagery (`imagery`) and / or heights (`terrain`) for tiles.
    fn load(&mut self, ed: &mut Editor, prefs: &GeoPrefs, keys: Vec<TileKey>, imagery: bool, terrain: bool) {
        self.load_with(ed, prefs, keys, imagery, None, terrain)
    }

    /// `from`: this source at this zoom (else the chosen or the sharpest one).
    fn load_with(&mut self, ed: &mut Editor, prefs: &GeoPrefs, keys: Vec<TileKey>, imagery: bool, from: Option<(Source, u8)>, terrain: bool) {
        let (Some(geo), Some(doc), Some(dir)) = (self.georef(), ed.doc.as_ref(), self.dir.clone()) else {
            self.status = Some((tr("Place the map on the Earth first.", "Сначала разместите карту на Земле.").into(), true));
            return;
        };
        if self.job.is_some() {
            self.queued.extend(keys);
            return;
        }
        let tiles: Vec<(TileKey, usize)> = keys.iter().filter_map(|k| doc.tiles.get(k).map(|t| (*k, t.terrain.cells))).collect();
        if tiles.is_empty() {
            return;
        }
        let (lat, _) = geo.lat_lon(tile_origin(tiles[0].0));
        // (the sharpest here unless one is chosen: the load finds it out itself when the
        // detail of every source is not known yet)
        let chosen = prefs.all().into_iter().find(|s| s.name == self.file.source && prefs.usable(s));
        let chosen = from.clone().map(|f| f.0).or(chosen);
        let src: Option<(Vec<Source>, u8)> = match (imagery, chosen) {
            (false, _) => None,
            (true, Some(s)) if from.is_some() => Some((vec![s], from.as_ref().unwrap().1)),
            (true, Some(s)) => Some((vec![s], self.file.zoom)),
            (true, None) => Some((self.ranked(prefs, lat).into_iter().map(|r| r.0).collect(), 0)),
        };
        let terr = if terrain { prefs.all().into_iter().find(|s| s.name == sources::TERRARIUM) } else { None };
        let anchor = self.anchor_tile(ed);
        let spec = job::Spec {
            dir,
            geo,
            ts: ts(),
            tiles,
            imagery: src.map(|(s, z)| (s, z, self.file.size)),
            terrain: terr.map(|s| (s, self.file.terrain.clone(), anchor)),
            prefs: prefs.clone(),
        };
        self.errors.clear();
        self.heights.clear();
        self.status = None;
        self.job = Some(job::start(spec));
    }

    /// Ask before own pictures are covered by downloaded imagery.
    fn load_checked(&mut self, ed: &mut Editor, prefs: &GeoPrefs, keys: Vec<TileKey>, terrain: bool) {
        if keys.iter().any(|k| self.file.tile(*k).own.is_some()) {
            self.confirm = Some((keys, terrain));
        } else {
            self.load(ed, prefs, keys, true, terrain);
        }
    }

    /// Follow the running load; at its end the heights are put in as one undo step.
    fn poll(&mut self, ed: &mut Editor, prefs: &GeoPrefs) {
        let Some(j) = &self.job else {
            if !self.queued.is_empty() {
                let keys: Vec<TileKey> = std::mem::take(&mut self.queued).into_iter().collect();
                let terrain = self.file.with_terrain;
                self.load(ed, prefs, keys, true, terrain);
            }
            return;
        };
        let mine = Some(&j.dir) == self.dir.as_ref();
        let mut end = None;
        while let Ok(m) = j.rx.try_recv() {
            match m {
                Msg::Image(k, meta, memory) if mine => {
                    let t = self.file.tile_mut(k);
                    t.imagery = Some(meta);
                    t.own = None;
                    self.shown.remove(&k);
                    match memory {
                        Some(m) => {
                            self.memory.insert(k, Arc::new(m));
                        }
                        None => {
                            self.memory.remove(&k);
                        }
                    }
                    self.save_at = Some(Instant::now());
                }
                Msg::Heights(k, h) => self.heights.push((k, h)),
                Msg::Base(b) if mine => {
                    self.file.terrain.base = Some(b);
                    self.save_at = Some(Instant::now());
                }
                Msg::Zoom(z, mpp) => self.detail = Some((z, mpp)),
                Msg::Error(k, e) => self.errors.push(format!("{} {}, {}: {e}", tr("Tile", "Тайл"), k.0, k.1)),
                Msg::Done(e) => end = Some(e),
                _ => {}
            }
        }
        let Some(e) = end else { return };
        let j = self.job.take().unwrap();
        let n = j.done.load(Ordering::Relaxed);
        if mine && !self.heights.is_empty() {
            if let Some(doc) = ed.doc.as_mut() {
                ed.history.begin(tr("Load terrain heights", "Загрузка высот рельефа"));
                let batch: BTreeSet<TileKey> = self.heights.iter().map(|h| h.0).collect();
                for (k, h) in std::mem::take(&mut self.heights) {
                    let Some(t) = doc.tiles.get(&k) else { continue };
                    if t.terrain.heights.len() != h.len() {
                        continue;
                    }
                    ed.history.touch_tile(doc, k);
                    let t = doc.tiles.get_mut(&k).unwrap();
                    t.terrain.heights = h;
                    t.touch_terrain();
                    self.file.tile_mut(k).terrain = Some(sources::TERRARIUM.into());
                }
                ed.history.commit();
                self.touch();
                // (tiles beside them left flat: their edge steps now)
                self.edges = doc.tiles.keys().copied().filter(|n| !batch.contains(n) && self.file.tile(*n).terrain.is_none() && [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| batch.contains(&(n.0 + dx, n.1 + dy)))).collect();
            }
        }
        self.status = Some(match e {
            None if self.errors.is_empty() => (format!("{} {n} {}", tr("Done:", "Готово:"), tr("tiles", "тайлов")), false),
            None => (format!("{} {n}, {} {}", tr("Done:", "Готово:"), self.errors.len(), tr("with problems", "с ошибками")), true),
            Some(e) => (e, true),
        });
    }

    /// New tiles get imagery by themselves (when the map is placed and that is on).
    fn watch_new(&mut self, ed: &mut Editor, prefs: &GeoPrefs) {
        let Some(doc) = &ed.doc else { return };
        let now: BTreeSet<TileKey> = doc.tiles.keys().copied().collect();
        let fresh: Vec<TileKey> = now.difference(&self.known).copied().collect();
        self.known = now;
        if fresh.is_empty() || !self.file.auto_new || self.georef().is_none() {
            return;
        }
        // (a tile back by undo has its imagery still)
        let fresh: Vec<TileKey> = fresh.into_iter().filter(|k| self.file.tile(*k).imagery.is_none() && self.file.tile(*k).own.is_none()).collect();
        // (only on a map that has imagery already: the first load is the user's)
        if !fresh.is_empty() && self.file.tiles.values().any(|t| t.imagery.is_some()) {
            let terrain = self.file.with_terrain;
            self.load(ed, prefs, fresh, true, terrain);
        }
    }

    // -------------------------------------------------------------- OMSI's own photos

    /// The aerial photos OMSI's own editor lays on the ground for this map, which openMapper
    /// has never read: one picture a tile, so they are taken as they are.
    fn aerial_ui(&mut self, ui: &mut egui::Ui, ed: &mut Editor) {
        if self.dir.is_none() {
            return;
        }
        crate::ui::section(ui, tr("OMSI's aerial photos", "Аэроснимки OMSI"));
        // (copying now: how far it is, and nothing else to press)
        if let Some(job) = &self.aerial_job {
            let at = job.at.load(Ordering::Relaxed);
            ui.label(RichText::new(format!("{} {at} / {}", tr("Copying the photos", "Копирование снимков"), job.total)).small().color(pal().text_dim));
            return;
        }
        match self.aerial.clone() {
            Some((folder, n)) => {
                ui.label(
                    RichText::new(tr(
                        "One photo a tile, as OMSI's editor lays them on the ground: they are copied into the map and used as they are.",
                        "По одному снимку на тайл, как их кладёт на землю редактор OMSI: копируются в карту и используются как есть.",
                    ))
                    .small()
                    .color(pal().text_dim),
                );
                ui.label(RichText::new(folder.display().to_string()).small().color(pal().text_dim));
                let label = format!("{} ({n})", tr("Import them", "Импортировать"));
                if ui.button(icons::labelled(Icon::Import, &label)).clicked() {
                    self.import_aerial(ed, &folder);
                }
            }
            None => {
                ui.label(
                    RichText::new(tr(
                        "This map has none beside it: OMSI keeps them in maps/aerials/<the map's folder>.",
                        "У этой карты их нет рядом: OMSI хранит их в maps/aerials/<папка карты>.",
                    ))
                    .small()
                    .color(pal().text_dim),
                );
                if ui.button(icons::labelled(Icon::Import, tr("Choose a folder of them…", "Выбрать папку с ними…"))).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title(tr("A folder of aerial photos", "Папка аэроснимков")).pick_folder() {
                        self.import_aerial(ed, &p);
                    }
                }
            }
        }
        // (a map's own photos are hundreds of megabytes: a window of them is cheap)
        if self.file.tiles.values().any(|t| t.imagery.is_some() || t.own.is_some()) {
            self.photos_near_ui(ui);
        }
    }

    /// "Only the tiles near the camera" for the photos, and how many.
    fn photos_near_ui(&mut self, ui: &mut egui::Ui) {
        let mut on = self.file.photos_near.is_some();
        if crate::ui::check(ui, &mut on, tr("Hold only the photos of the tiles near the camera", "Держать только снимки тайлов рядом с камерой")).changed() {
            self.file.photos_near = on.then_some(25);
            self.near = None;
            self.touch();
        }
        if let Some(n) = &mut self.file.photos_near {
            let mut c = false;
            ui.horizontal(|ui| {
                ui.label(tr("Tiles", "Тайлов"));
                c = ui.add(egui::DragValue::new(n).speed(1.0).range(1..=2000)).changed();
                let side = ((*n as f64).sqrt() * ts() * 0.5 / 1000.0) as f32;
                ui.label(RichText::new(format!("≈ ±{side:.1} km")).small().color(pal().text_dim));
            });
            ui.label(
                RichText::new(tr(
                    "The others keep their photo in the map's folder, and are put up again when you go back to them.",
                    "У остальных снимок остаётся в папке карты и ставится снова, когда вы к ним вернётесь.",
                ))
                .small()
                .color(pal().text_dim),
            );
            if c {
                self.near = None;
                self.touch();
            }
        }
    }

    /// Copy a folder's photos over the map's tiles (the ones the map has; a photo of a tile
    /// that is not there is left out), and show them. The copying runs on a thread: the window
    /// keeps drawing while a hundred megabytes go by.
    fn import_aerial(&mut self, ed: &mut Editor, folder: &Path) {
        let Some(dir) = self.dir.clone() else { return };
        // (one at a time: a second press while the first copies would take the same files)
        if self.aerial_job.is_some() {
            return;
        }
        let tiles: HashSet<TileKey> = ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default();
        let found = match g::aerial::pick(folder, &tiles) {
            Ok(f) => f,
            Err(g::aerial::NotTaken::NoPhotos) => {
                ed.error(format!("{}: {}", tr("No aerial photos in that folder", "В той папке нет аэроснимков"), folder.display()));
                return;
            }
            Err(_) => {
                ed.error(tr("The photos are of tiles this map does not have", "Снимки относятся к тайлам, которых у этой карты нет"));
                return;
            }
        };
        let total = found.len();
        let (tx, rx) = channel();
        let at = Arc::new(AtomicUsize::new(0));
        let (map, progress) = (dir.clone(), at.clone());
        std::thread::spawn(move || {
            let r = g::aerial::import(&map, &found, |i, _| progress.store(i, Ordering::Relaxed));
            let _ = tx.send(r);
        });
        self.aerial_job = Some(AerialJob { rx, at, total, folder: folder.to_path_buf(), stamp: self.georef().map(|g| g.stamp()).unwrap_or(0) });
    }

    /// The copy running now, when it ends: the tiles show their photos (saved with the map).
    fn poll_aerial(&mut self, ed: &mut Editor) {
        let Some(job) = self.aerial_job.as_ref() else { return };
        let got = match job.rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            // (the thread ended without an answer: nothing to record)
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
        };
        let Some(got) = got else { return };
        let job = self.aerial_job.take().expect("asked for above");
        match got {
            Ok(done) => {
                g::aerial::note(&mut self.file, &done, job.stamp);
                // (the files are there now: read them again on the next frame)
                self.shown.clear();
                self.pictures.clear();
                self.pictures_at = None;
                // (a folder chosen by hand becomes the one the panel offers, as the map's own)
                match &mut self.aerial {
                    Some(a) => a.0 = job.folder,
                    None => self.aerial = Some((job.folder, done.len())),
                }
                self.touch();
                ed.toast(format!("{} ({})", tr("The photos are on the tiles", "Снимки на тайлах"), done.len()));
            }
            Err(e) => ed.error(format!("{}: {e}", tr("Could not copy the photos", "Не удалось скопировать снимки"))),
        }
    }

    // -------------------------------------------------------------- own pictures

    fn pick_picture(&mut self, ed: &mut Editor, rect: [i32; 4]) {
        let Some(p) = rfd::FileDialog::new().add_filter(tr("Pictures", "Картинки"), &PICTURES).set_title(tr("A picture for the tiles", "Картинка для тайлов")).pick_file() else { return };
        self.own_picture(ed, &p, rect);
    }

    /// Lay a picture file on a rectangle of tiles (copied into the map's folder).
    fn own_picture(&mut self, ed: &mut Editor, path: &Path, rect: [i32; 4]) {
        let Some(dir) = self.dir.clone() else { return };
        let img = match image::open(path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                ed.error(format!("{}: {e}", tr("Could not read the picture", "Не удалось прочитать картинку")));
                return;
            }
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
        let mut id = format!("own_{}_{}", rect[0], rect[1]);
        let mut i = 2;
        while self.file.pictures.contains_key(&id) {
            id = format!("own_{}_{}_{i}", rect[0], rect[1]);
            i += 1;
        }
        let file = format!("{id}.{ext}");
        let target = g::dir_of(&dir).join("imagery").join(&file);
        if let Err(e) = std::fs::create_dir_all(target.parent().unwrap()).and_then(|_| std::fs::copy(path, &target)) {
            ed.error(format!("{}: {e}", tr("Could not copy the picture", "Не удалось скопировать картинку")));
            return;
        }
        let pic = Picture { file, rect, fit: Fit::default() };
        // (a tile shows one picture: those it covered before lose it)
        for k in pic.tiles().collect::<Vec<_>>() {
            if let Some(old) = self.file.tile(k).own {
                if let Some(o) = self.file.pictures.get_mut(&old) {
                    if o.tiles().all(|t| pic.covers(t)) {
                        self.file.pictures.remove(&old);
                    }
                }
            }
            self.file.tile_mut(k).own = Some(id.clone());
        }
        self.file.pictures.insert(id.clone(), pic);
        self.pic_cache.insert(id.clone(), Arc::new(img));
        self.pic_sel = Some(id.clone());
        self.bake_pieces(&id, true);
        self.touch();
        self.open = true;
        ed.toast(tr("The picture is on the tiles: line it up in Satellite and terrain, Own pictures.", "Картинка на тайлах: подгоните её в «Спутник и рельеф», «Свои картинки»."));
    }

    fn picture_image(&mut self, id: &str) -> Option<Arc<image::RgbaImage>> {
        if let Some(i) = self.pic_cache.get(id) {
            return Some(i.clone());
        }
        let dir = self.dir.clone()?;
        let p = self.file.pictures.get(id)?;
        let img = Arc::new(image::open(g::dir_of(&dir).join("imagery").join(&p.file)).ok()?.to_rgba8());
        self.pic_cache.insert(id.to_string(), img.clone());
        Some(img)
    }

    /// Cut a picture into its tiles' pieces: a quick preview, or the full ones written.
    fn bake_pieces(&mut self, id: &str, full: bool) {
        let (Some(dir), Some(img), Some(pic)) = (self.dir.clone(), self.picture_image(id), self.file.pictures.get(id).cloned()) else { return };
        let keys: Vec<TileKey> = pic.tiles().filter(|k| self.file.tile(*k).own.as_deref() == Some(id)).collect();
        let (tx, rx) = channel();
        let t = ts();
        let max = self.gpu_side(keys.len());
        std::thread::spawn(move || {
            let n = if full { pic.piece_size(t, img.width()) } else { 384 };
            for k in keys {
                let piece = own::cut(&pic, &img, k, t, n);
                if full {
                    let p = g::own_path(&dir, k);
                    let _ = std::fs::create_dir_all(p.parent().unwrap());
                    let _ = piece.save(&p);
                }
                if tx.send((k, Mips::of(piece, max))).is_err() {
                    return;
                }
            }
        });
        self.bake = Some(Bake { rx, full });
    }

    fn poll_bake(&mut self) {
        if let Some(b) = &self.bake {
            loop {
                match b.rx.try_recv() {
                    Ok((k, m)) => {
                        if b.full {
                            // (the file is new: the next look at it reads it again)
                            self.shown.remove(&k);
                            self.pictures.remove(&k);
                        }
                        self.uploads.push((k, m));
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(_) => {
                        self.bake = None;
                        break;
                    }
                }
            }
        }
        if let Some(at) = self.fit_changed {
            if self.bake.is_none() {
                let id = self.pic_sel.clone();
                if let Some(id) = id {
                    let rest = at.elapsed().as_secs_f32() > 0.6;
                    self.bake_pieces(&id, rest);
                    if rest {
                        self.fit_changed = None;
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------- the overlay

    /// Edge of the textures on the GPU: the more of them are held at once, the smaller. What
    /// counts is what is up on the card now, not how many tiles the map has: a map's own aerial
    /// photos are 1024 or 2048 a side, and 485 of those kept at their own size would be 43 GB,
    /// while a window of sixteen round the camera is a quarter of a gigabyte.
    fn gpu_side(&self, held: usize) -> u32 {
        match held {
            0..=4 => 4096,
            5..=16 => 2048,
            17..=64 => 1024,
            _ => 512,
        }
    }

    /// The tiles whose photos are kept: the `photos_near` ones nearest `at`, or None for every
    /// tile. Worked out again only when the camera has moved a tile's width, so a still view
    /// costs nothing.
    fn near_set(&mut self, doc: &crate::doc::Document, at: DVec2) -> Option<HashSet<TileKey>> {
        let Some(n) = self.file.photos_near.filter(|n| *n > 0) else {
            self.near = None;
            return None;
        };
        let n = n as usize;
        if doc.tiles.len() <= n {
            self.near = None;
            return None;
        }
        let half = DVec2::splat(ts() * 0.5);
        if let Some((set, from, was)) = &self.near {
            if *was == n && (*from - at).length() < ts() * 0.5 {
                return Some(set.clone());
            }
        }
        let mut near: Vec<(f64, TileKey)> = doc.tiles.keys().map(|k| ((tile_origin(*k) + half - at).length_squared(), *k)).collect();
        near.select_nth_unstable_by(n, |a, b| a.0.total_cmp(&b.0));
        let set: HashSet<TileKey> = near[..n].iter().map(|p| p.1).collect();
        self.near = Some((set.clone(), at, n));
        Some(set)
    }

    /// Keep the renderer's pictures and their opacity as the map's settings say. `at` is where
    /// the camera looks: with `photos_near` set, only the photos of the tiles round it are
    /// held (a map's own aerial photos are hundreds of megabytes).
    pub fn feed(&mut self, imagery: &mut Imagery, gpu: &crate::gpu::Gpu, ed: &Editor, at: DVec2) {
        let Some(dir) = self.dir.clone() else { return };
        let Some(doc) = &ed.doc else { return };
        if self.epoch != imagery.epoch {
            self.epoch = imagery.epoch;
            imagery.clear();
            self.shown.clear();
        }
        while let Ok((d, k, m)) = self.loaded_rx.try_recv() {
            self.loading.remove(&k);
            if d != dir {
                continue;
            }
            match m {
                Some(m) => {
                    imagery.upload(gpu, k, &m, 0.0);
                    self.mini_pending.push((k, mini_of(&m)));
                }
                None => imagery.remove(k),
            }
        }
        for (k, m) in std::mem::take(&mut self.uploads) {
            imagery.upload(gpu, k, &m, 0.0);
            self.mini_pending.push((k, mini_of(&m)));
        }
        // (the disk is looked at now and then, not in every frame; a change of the map's own
        // settings clears it at once, through `flush`)
        if self.pictures_at.map(|t| t.elapsed().as_secs_f32() > 1.0).unwrap_or(true) {
            self.pictures_at = Some(Instant::now());
            self.pictures.clear();
        }
        // (the photos of the tiles round the camera, when the map asks for a window of them)
        let near = self.near_set(doc, at);
        // (the edge of the textures: what is held at once, not what the map has - a window of a
        // few tiles can keep its photos near their own size, the whole map cannot)
        let held = near.as_ref().map(|s| s.len()).unwrap_or_else(|| self.file.tiles.values().filter(|t| t.imagery.is_some() || t.own.is_some()).count());
        let max = self.gpu_side(held);
        // (a different edge from the one the pictures went up at: they go up again)
        if self.shown_side != max {
            self.shown_side = max;
            self.shown.clear();
        }
        for k in doc.tiles.keys() {
            let t = self.file.tile(*k);
            if let Some(set) = &near {
                if !set.contains(k) {
                    // (out of the window: nothing of it stays on the graphics card)
                    if self.shown.remove(k).is_some() {
                        imagery.remove(*k);
                        self.mini.remove(k);
                    }
                    imagery.set_opacity(*k, 0.0);
                    continue;
                }
            }
            let want = self.picture_cached(&dir, *k);
            if let (Some(m), true) = (self.memory.get(k), t.own.is_none()) {
                // (held in memory only: put up once)
                let now = (PathBuf::new(), None);
                if self.shown.get(k) != Some(&now) {
                    self.shown.insert(*k, now);
                    let mips = Mips::of((**m).clone(), max);
                    imagery.upload(gpu, *k, &mips, 0.0);
                    self.mini_pending.push((*k, mini_of(&mips)));
                }
            } else {
                match want {
                    Some((p, time)) => {
                        let now = (p, time);
                        if self.shown.get(k) != Some(&now) && !self.loading.contains(k) {
                            let p = now.0.clone();
                            self.shown.insert(*k, now);
                            self.loading.insert(*k);
                            let _ = self.load_tx.send((dir.clone(), *k, p, max));
                        }
                    }
                    None => {
                        if self.shown.remove(k).is_some() {
                            imagery.remove(*k);
                            self.mini.remove(k);
                        }
                    }
                }
            }
            let on = self.file.overlay && t.visible;
            imagery.set_opacity(*k, if on { (self.file.opacity * t.opacity).clamp(0.0, 1.0) } else { 0.0 });
        }
    }

    // -------------------------------------------------------------- the window

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, prefs: &mut Prefs) {
        self.follow(ed);
        self.poll_aerial(ed);
        self.poll(ed, &prefs.geo);
        self.watch_new(ed, &prefs.geo);
        self.poll_bake();
        if self.save_at.map(|t| t.elapsed().as_secs_f32() > 0.5).unwrap_or(false) {
            self.flush();
        }
        if let Some(rx) = &self.search_rx {
            if let Ok(r) = rx.try_recv() {
                self.search_rx = None;
                match r {
                    Ok(p) => {
                        if let Some(first) = p.first() {
                            self.map.look_at(first.lat, first.lon, 16.0);
                        }
                        if p.is_empty() {
                            self.status = Some((tr("Nothing found by that name.", "Ничего не найдено.").into(), true));
                        }
                        self.places = p;
                    }
                    Err(e) => self.status = Some((e, true)),
                }
            }
        }
        if let Some(p) = &mut self.probe {
            if let Some(rx) = &p.rx {
                while let Ok((n, r)) = rx.try_recv() {
                    p.results.insert(n, r);
                }
            }
        }
        if let Some(rx) = &self.google {
            if let Ok(r) = rx.try_recv() {
                self.google_line = r.ok().map(|v| v.copyright);
                self.google = None;
            }
        }
        for (k, img) in self.mini_pending.drain(..) {
            self.mini.insert(k, ctx.load_texture(format!("geo mini {} {}", k.0, k.1), img, egui::TextureOptions::LINEAR));
        }
        if !self.rebuild.is_empty() && self.job.is_none() && self.georef().is_some() {
            let keys = std::mem::take(&mut self.rebuild);
            let z = keys.first().and_then(|k| self.file.tile(*k).imagery.map(|m| m.zoom)).unwrap_or(0);
            let google = prefs.geo.all().into_iter().find(|s| s.kind == Kind::Google);
            match (google, prefs.geo.key(sources::GOOGLE).is_some()) {
                (Some(src), true) => self.load_with(ed, &prefs.geo, keys, true, Some((src, z)), false),
                _ => self.status = Some((tr("Some tiles had Google imagery: enter your Google key to see it again (it is not kept with the map).", "У некоторых тайлов были снимки Google: введите ключ Google, чтобы снова их видеть (они не хранятся с картой).").into(), true)),
            }
        }
        self.confirm_dialog(ctx, ed, &prefs.geo);
        if !self.open || ed.doc.is_none() {
            return;
        }
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Map, tr("Satellite and terrain", "Спутник и рельеф")))
            .id(egui::Id::new("geo window"))
            .open(&mut open)
            .default_pos([300.0, 70.0])
            .default_size([640.0, 760.0])
            .min_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.contents(ui, ed, prefs));
            });
        self.open = open;
    }

    fn contents(&mut self, ui: &mut egui::Ui, ed: &mut Editor, prefs: &mut Prefs) {
        let world = omsi_map::world_coordinates();
        let geo = self.georef();
        // ---- where the map is
        ui.horizontal(|ui| {
            ui.label(icons::labelled(Icon::Focus, ""));
            let r = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("Find a place: a town, a street, an address… (Enter)", "Найти место: город, улица, адрес… (Enter)")).desired_width(ui.available_width() - 8.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.search.trim().is_empty() && self.search_rx.is_none() {
                let (tx, rx) = channel();
                let q = self.search.trim().to_string();
                std::thread::spawn(move || {
                    let _ = tx.send(fetch::search(&q));
                });
                self.search_rx = Some(rx);
            }
        });
        if self.search_rx.is_some() {
            ui.label(RichText::new(tr("Searching…", "Поиск…")).color(pal().text_dim));
        }
        if !self.places.is_empty() {
            let mut pick = None;
            egui::ScrollArea::vertical().id_salt("places").max_height(90.0).show(ui, |ui| {
                for (i, p) in self.places.iter().enumerate() {
                    if ui.selectable_label(false, &p.name).clicked() {
                        pick = Some(i);
                    }
                }
            });
            if let Some(i) = pick {
                let p = self.places[i].clone();
                self.map.look_at(p.lat, p.lon, 16.0);
                self.places.clear();
            }
        }
        // the map of the world, with the map's tiles on it
        let all = prefs.geo.all();
        let view_src = Arc::new(all.iter().find(|s| s.name == sources::ESRI).cloned().unwrap());
        let gp = Arc::new(prefs.geo.clone());
        let w = ui.available_width();
        let (resp, rect) = self.map.show(ui, Vec2::new(w, (w * 0.62).clamp(260.0, 460.0)), &view_src, &gp);
        let anchor_tile = self.anchor_tile(ed);
        let hover = resp.hover_pos().map(|p| self.map.to_merc(rect, p));
        // (placing: the tiles follow the cursor)
        let shown_geo = match (self.placing, hover) {
            (true, Some(m)) => {
                let (la, lo) = g::lat_lon_of_merc(m);
                let rot = self.file.anchor.map(|a| a.rotation).unwrap_or(0.0);
                Some(Georef::local(&Anchor { tile: [anchor_tile.0, anchor_tile.1], lat: la, lon: lo, rotation: rot }, ts()))
            }
            _ => geo,
        };
        if let (Some(gr), Some(doc)) = (shown_geo, &ed.doc) {
            let painter = ui.painter_at(rect);
            for k in doc.tiles.keys() {
                let o = tile_origin(*k);
                let pts: Vec<Pos2> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().map(|(x, y)| self.map.to_screen(rect, gr.merc(o + DVec2::new(*x, *y) * ts()))).collect();
                let t = self.file.tile(*k);
                let fill = if *k == anchor_tile { Color32::from_rgba_unmultiplied(232, 122, 50, 60) } else if t.imagery.is_some() || t.own.is_some() { Color32::from_rgba_unmultiplied(120, 200, 120, 30) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 20) };
                painter.add(egui::Shape::convex_polygon(pts.clone(), fill, Stroke::new(1.5, Color32::from_rgb(255, 210, 90))));
            }
        }
        // (the source's attribution on the map, as its terms ask)
        let att = format!("{}  ·  {}", view_src.attribution, tr("Search © OpenStreetMap contributors", "Поиск © участники OpenStreetMap"));
        let painter = ui.painter_at(rect);
        let galley = painter.layout_no_wrap(att, egui::FontId::proportional(10.0), Color32::WHITE);
        let at = rect.right_bottom() - galley.size() - Vec2::splat(3.0);
        painter.rect_filled(Rect::from_min_size(at - Vec2::splat(2.0), galley.size() + Vec2::splat(4.0)), 2.0, Color32::from_black_alpha(150));
        painter.galley(at, galley, Color32::WHITE);
        if let (true, false, Some(gr), Some(m)) = (resp.clicked(), self.placing, geo, hover) {
            let (la, lo) = g::lat_lon_of_merc(m);
            self.look = Some(gr.world(la, lo));
        }
        if resp.clicked() && self.placing {
            if let Some(m) = hover {
                let (la, lo) = g::lat_lon_of_merc(m);
                let rot = self.file.anchor.map(|a| a.rotation).unwrap_or(0.0);
                self.file.anchor = Some(Anchor { tile: [anchor_tile.0, anchor_tile.1], lat: la, lon: lo, rotation: rot });
                self.placing = false;
                self.probe = None;
                self.touch();
            }
        }
        ui.add_space(4.0);
        if world {
            ui.label(RichText::new(tr("This map has world coordinates ([worldcoordinates]): it lies on the Earth by itself, its tiles are Web Mercator tiles.", "У этой карты мировые координаты ([worldcoordinates]): она сама лежит на Земле, её тайлы: тайлы Web Mercator.")).color(pal().text_dim));
        } else {
            ui.horizontal_wrapped(|ui| {
                let label = if self.placing { tr("Click on the map where the orange tile goes…", "Кликните на карте, где будет оранжевый тайл…") } else { tr("Place the map here", "Разместить карту здесь") };
                if ui.add(egui::Button::new(icons::labelled(Icon::Flag, label)).selected(self.placing)).on_hover_text(tr("Then click on the map: the middle of the orange tile goes there", "Затем кликните на карте: туда встанет середина оранжевого тайла")).clicked() {
                    self.placing = !self.placing;
                }
                if ui.button(tr("At the view's middle", "В центр вида")).clicked() {
                    let (la, lo) = g::lat_lon_of_merc(self.map.centre);
                    let rot = self.file.anchor.map(|a| a.rotation).unwrap_or(0.0);
                    self.file.anchor = Some(Anchor { tile: [anchor_tile.0, anchor_tile.1], lat: la, lon: lo, rotation: rot });
                    self.probe = None;
                    self.touch();
                }
                if let Some(a) = &mut self.file.anchor {
                    ui.label(tr("North turned", "Поворот севера"));
                    if ui.add(egui::DragValue::new(&mut a.rotation).speed(0.1).range(-180.0..=180.0).suffix("°")).changed() {
                        self.save_at = Some(Instant::now());
                    }
                }
            });
            match &self.file.anchor {
                Some(a) => {
                    ui.label(RichText::new(format!("{} {}, {}: {:.6}°, {:.6}° · {}", tr("Orange tile", "Оранжевый тайл"), a.tile[0], a.tile[1], a.lat, a.lon, tr("click the map to fly there in the view", "клик по карте: перелёт туда в окне вида"))).color(pal().text_dim));
                }
                None => {
                    ui.label(RichText::new(tr("1. Find the place.  2. Press \"Place the map here\" and click on the map: the orange tile (the map's middle) goes there.", "1. Найдите место.  2. Нажмите «Разместить карту здесь» и кликните на карте: туда встанет оранжевый тайл (середина карты).")).color(pal().text_dim));
                }
            }
        }
        ui.separator();
        // ---- sources and their detail here
        let here = geo.map(|gr| gr.lat_lon(self.middle(ed)));
        if let Some((la, lo)) = here {
            self.probe_here(&prefs.geo, la, lo);
        }
        let lat = here.map(|h| h.0).unwrap_or(50.0);
        crate::ui::section(ui, tr("Imagery", "Снимки"));
        let ranked = self.ranked(&prefs.geo, lat);
        let current = self.source(&prefs.geo, lat);
        let best = ranked.first().map(|r| r.0.name.clone());
        ui.horizontal(|ui| {
            if ui.radio(self.file.source.is_empty(), tr("The sharpest here", "Самые чёткие здесь")).clicked() {
                self.file.source.clear();
                self.touch();
            }
            if let (true, Some(b)) = (self.file.source.is_empty(), &best) {
                ui.label(RichText::new(format!("({b})")).color(pal().text_dim));
            }
        });
        for (s, m) in &ranked {
            ui.horizontal(|ui| {
                if ui.radio(self.file.source == s.name, &s.name).clicked() {
                    self.file.source = s.name.clone();
                    self.touch();
                }
                let note = match (m, self.probe.as_ref().and_then(|p| p.results.get(&s.name))) {
                    (Some(m), _) => format!("~{:.0} {}", m * 100.0, tr("cm a pixel here", "см на пиксель здесь")),
                    (None, Some(Err(e))) => e.clone(),
                    (None, None) if here.is_some() => tr("checking…", "проверка…").into(),
                    _ => s.note(),
                };
                ui.label(RichText::new(note).color(pal().text_dim));
            });
        }
        if let (Some(c), Some((_, Some(bm)))) = (&current, ranked.first()) {
            let cm = ranked.iter().find(|r| r.0.name == c.name).and_then(|r| r.1);
            if let Some(cm) = cm {
                if cm > bm * 1.5 {
                    ui.colored_label(pal().warn, format!("{} {} ({:.0} cm)", ranked[0].0.name, tr("is sharper here", "здесь чётче"), bm * 100.0));
                }
            }
            if *bm > 0.25 && !prefs.geo.keys.values().any(|k| !k.trim().is_empty()) {
                ui.label(RichText::new(tr("Sharper imagery may be had here with your own free key for Mapbox, MapTiler or Google (Keys below).", "Более чёткие снимки здесь могут быть с вашим бесплатным ключом Mapbox, MapTiler или Google (ниже, «Ключи»).")).color(pal().text_dim));
            }
        }
        if let Some(c) = &current {
            let mut line = c.attribution.clone();
            if c.kind == Kind::Google {
                line = format!("Google · {}", self.google_line.clone().unwrap_or_default());
            }
            ui.label(RichText::new(line).small().color(pal().text_dim));
            if !c.key_link.is_empty() && !c.needs_key {
                ui.hyperlink_to(RichText::new(tr("Terms of use", "Условия использования")).small(), &c.key_link);
            }
        }
        self.keys_ui(ui, prefs);
        self.custom_ui(ui, prefs);
        ui.add_space(4.0);
        // ---- loading
        egui::Grid::new("geo opts").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label(tr("Zoom", "Масштаб"));
            let src_max = current.as_ref().map(|s| s.useful_zoom(lat)).unwrap_or(19);
            let finest = current.as_ref().and_then(|c| self.probe.as_ref().and_then(|p| p.results.get(&c.name)).and_then(|r| r.clone().ok().flatten()));
            let name = |z: u8| if z == 0 { format!("{}{}", tr("the finest here", "самый подробный здесь"), finest.map(|f| format!(" (z{f})")).unwrap_or_default()) } else { format!("z{z}") };
            egui::ComboBox::from_id_salt("geo zoom").selected_text(name(self.file.zoom)).show_ui(ui, |ui| {
                for z in std::iter::once(0).chain((14..=src_max).rev()) {
                    if ui.selectable_value(&mut self.file.zoom, z, name(z)).changed() {
                        self.save_at = Some(Instant::now());
                    }
                }
            });
            ui.end_row();
            ui.label(tr("Picture of a tile", "Картинка тайла"));
            let sz = |s: u32| if s == 0 { tr("every pixel of the source (up to 4096)", "каждый пиксель источника (до 4096)").to_string() } else { format!("{s} × {s}") };
            egui::ComboBox::from_id_salt("geo size").selected_text(sz(self.file.size)).show_ui(ui, |ui| {
                for s in [0, 1024, 2048, 4096] {
                    if ui.selectable_value(&mut self.file.size, s, sz(s)).changed() {
                        self.save_at = Some(Instant::now());
                    }
                }
            });
            ui.end_row();
        });
        if let Some((z, m)) = self.detail {
            ui.label(RichText::new(format!("{} z{z}, ~{:.0} {}", tr("Loaded at", "Загружено с"), m * 100.0, tr("cm a pixel", "см на пиксель"))).color(pal().text_dim));
        }
        let mut c = false;
        c |= crate::ui::check(ui, &mut self.file.with_terrain, tr("Also load terrain heights", "Загружать и высоты рельефа")).changed();
        c |= crate::ui::check(ui, &mut self.file.auto_new, tr("New tiles get imagery by themselves", "Новые тайлы получают снимки сами")).changed();
        if c {
            self.touch();
        }
        let keys: Vec<TileKey> = ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default();
        let picked = ed.picked_tile.filter(|k| keys.contains(k));
        let busy = self.job.is_some();
        ui.horizontal_wrapped(|ui| {
            let placed = geo.is_some();
            if ui.add_enabled(placed && !busy, egui::Button::new(icons::labelled(Icon::Import, tr("Load for all tiles", "Загрузить для всех тайлов")))).clicked() {
                self.load_checked(ed, &prefs.geo, keys.clone(), self.file.with_terrain);
            }
            let r = ui.add_enabled(placed && !busy && picked.is_some(), egui::Button::new(tr("For the selected tile", "Для выбранного тайла")));
            let r = r.on_disabled_hover_text(tr("Click a tile with the Tiles tool (7)", "Кликните тайл инструментом «Тайлы» (7)"));
            if r.clicked() {
                if let Some(k) = picked {
                    self.load_checked(ed, &prefs.geo, vec![k], self.file.with_terrain);
                }
            }
            if ui.add_enabled(placed && !busy, egui::Button::new(icons::labelled(Icon::Terrain, tr("Terrain heights only", "Только высоты рельефа")))).clicked() {
                self.load(ed, &prefs.geo, keys.clone(), false, true);
            }
        });
        if let Some(j) = &self.job {
            ui.horizontal(|ui| {
                let d = j.done.load(Ordering::Relaxed);
                ui.add(egui::ProgressBar::new(d as f32 / j.total.max(1) as f32).text(format!("{d} {} {} {}", tr("of", "из"), j.total, tr("tiles", "тайлов"))).desired_width(ui.available_width() - 90.0));
                if ui.button(tr("Cancel", "Отмена")).clicked() {
                    j.cancel.store(true, Ordering::Relaxed);
                }
            });
        }
        if let Some((s, err)) = &self.status {
            ui.colored_label(if *err { pal().error } else { pal().ok }, s);
        }
        for e in self.errors.iter().take(4) {
            ui.label(RichText::new(e).small().color(pal().error));
        }
        if !self.edges.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(pal().warn, format!("{} {}", self.edges.len(), tr("tiles beside the loaded heights were not loaded: the ground steps at their edge.", "тайлов рядом с загруженными высотами не загружены: на их краю ступенька.")));
                if ui.button(tr("Fit their edges", "Подогнать их края")).on_hover_text(tr("Their edge takes the loaded heights, fading out 60 m into the tile", "Их край берёт загруженные высоты, переходя на нет за 60 м вглубь тайла")).clicked() {
                    self.fit_edges(ed);
                }
                if ui.small_button(tr("Leave them", "Оставить")).clicked() {
                    self.edges.clear();
                }
            });
        }
        ui.collapsing(tr("Terrain heights", "Высоты рельефа"), |ui| {
            let t = &mut self.file.terrain;
            let mut c = crate::ui::check(ui, &mut t.relative, tr("Relative: the orange tile's mean height is 0 m", "Относительные: средняя высота оранжевого тайла: 0 м")).changed();
            ui.horizontal(|ui| {
                ui.label(tr("Raise everything by", "Поднять всё на"));
                c |= ui.add(egui::DragValue::new(&mut t.offset).speed(0.5).suffix(" m")).changed();
                ui.label(tr("Smoothing", "Сглаживание"));
                c |= ui.add(egui::DragValue::new(&mut t.smooth).speed(0.5).range(0.0..=60.0).suffix(" m")).changed();
            });
            if let Some(b) = t.base {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} {b:.2} m", tr("Taken off:", "Вычтено:"))).color(pal().text_dim));
                    if ui.small_button(tr("Measure again", "Измерить заново")).clicked() {
                        t.base = None;
                        c = true;
                    }
                });
            }
            ui.label(RichText::new(sources::builtin().into_iter().find(|s| s.is_terrain()).map(|s| s.attribution).unwrap_or_default()).small().color(pal().text_dim));
            ui.hyperlink_to(RichText::new(tr("Terrain data attribution", "Источники данных рельефа")).small(), "https://github.com/tilezen/joerd/blob/master/docs/attribution.md");
            if c {
                self.save_at = Some(Instant::now());
            }
        });
        ui.separator();
        // ---- the overlay
        crate::ui::section(ui, tr("Overlay on the ground", "Подложка на земле"));
        ui.horizontal(|ui| {
            let mut c = crate::ui::check(ui, &mut self.file.overlay, tr("Show", "Показывать")).changed();
            c |= ui.add(egui::Slider::new(&mut self.file.opacity, 0.0..=1.0).text(tr("opacity", "непрозрачность"))).changed();
            if c {
                self.save_at = Some(Instant::now());
            }
        });
        if crate::ui::check(ui, &mut self.file.export_imagery, tr("Put the imagery into maps exported for sharing (check its licence first)", "Класть снимки в карты для обмена (сначала проверьте лицензию)")).changed() {
            self.touch();
        }
        egui::ScrollArea::vertical().id_salt("geo tiles").max_height(170.0).show(ui, |ui| {
            egui::Grid::new("geo tile list").num_columns(4).striped(true).show(ui, |ui| {
                for k in &keys {
                    let what = self.what(*k);
                    let t = self.file.tile_mut(*k);
                    let mut c = false;
                    ui.label(format!("{}, {}", k.0, k.1));
                    c |= ui.checkbox(&mut t.visible, "").changed();
                    c |= ui.add(egui::Slider::new(&mut t.opacity, 0.0..=1.0).show_value(false)).changed();
                    ui.label(RichText::new(what).small().color(pal().text_dim));
                    ui.end_row();
                    if c {
                        self.save_at = Some(Instant::now());
                    }
                }
            });
        });
        if self.dir.is_some() {
            ui.separator();
            self.aerial_ui(ui, ed);
        }
        ui.separator();
        self.own_ui(ui, ed, &keys);
    }

    /// Bend the unloaded tiles beside loaded heights to meet them (one undo step).
    fn fit_edges(&mut self, ed: &mut Editor) {
        let Some(doc) = ed.doc.as_mut() else { return };
        ed.history.begin(tr("Fit tile edges", "Подгонка краёв тайлов"));
        for nk in std::mem::take(&mut self.edges) {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let lk = (nk.0 + dx, nk.1 + dy);
                if self.file.tile(lk).terrain.is_none() {
                    continue;
                }
                let (Some(l), Some(t)) = (doc.tiles.get(&lk), doc.tiles.get(&nk)) else { continue };
                let n = t.terrain.samples();
                if l.terrain.samples() != n {
                    continue;
                }
                let loaded = l.terrain.heights.clone();
                ed.history.touch_tile(doc, nk);
                let t = doc.tiles.get_mut(&nk).unwrap();
                // (12 samples: 60 m on a 300 m tile)
                crate::geo::elevation::fit_edge(&mut t.terrain.heights, &loaded, n, (-dx, -dy), 12);
                t.touch_terrain();
            }
        }
        ed.history.commit();
    }

    /// The overview map's pictures: each tile's corner, its small copy and its opacity.
    pub fn minimap_tiles(&self) -> Vec<(DVec2, egui::TextureId, f32)> {
        if !self.file.overlay {
            return Vec::new();
        }
        self.mini
            .iter()
            .filter_map(|(k, t)| {
                let g = self.file.tile(*k);
                g.visible.then(|| (tile_origin(*k), t.id(), (self.file.opacity * g.opacity).clamp(0.0, 1.0)))
            })
            .collect()
    }

    fn keys_ui(&mut self, ui: &mut egui::Ui, prefs: &mut Prefs) {
        ui.collapsing(tr("Keys for more sources (optional)", "Ключи для других источников (необязательно)"), |ui| {
            ui.label(RichText::new(tr("Everything works without a key. With your own free key these may be sharper in some places; the key stays on this computer.", "Всё работает без ключа. С вашим бесплатным ключом эти источники местами чётче; ключ остаётся на этом компьютере.")).color(pal().text_dim));
            let mut changed = false;
            for s in sources::builtin().into_iter().filter(|s| s.needs_key) {
                let name = if s.kind == Kind::Google { tr("Google satellite (needs your API key)", "Google спутник (нужен ваш API-ключ)").to_string() } else { s.name.clone() };
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.hyperlink_to(tr("get a key", "получить ключ"), &s.key_link);
                });
                if s.kind == Kind::Google {
                    ui.label(RichText::new(tr("In Google Cloud: make a project, enable the Map Tiles API, create an API key, paste it here. Google bills by use above its free amount.", "В Google Cloud: создайте проект, включите Map Tiles API, создайте API-ключ и вставьте сюда. Сверх бесплатного объёма Google берёт плату.")).small().color(pal().text_dim));
                }
                let key = prefs.geo.keys.entry(s.name.clone()).or_default();
                changed |= ui.add(egui::TextEdit::singleline(key).password(true).hint_text(tr("key", "ключ")).desired_width(f32::INFINITY)).changed();
            }
            if changed {
                prefs.geo.keys.retain(|_, v| !v.trim().is_empty());
                prefs.save();
                self.probe = None;
            }
        });
    }

    fn custom_ui(&mut self, ui: &mut egui::Ui, prefs: &mut Prefs) {
        ui.collapsing(tr("Your own tile server", "Свой сервер тайлов"), |ui| {
            let mut remove = None;
            for (i, s) in prefs.geo.sources.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(&s.name);
                    ui.label(RichText::new(&s.url).small().color(pal().text_dim));
                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                prefs.geo.sources.remove(i);
                prefs.save();
            }
            let c = &mut self.custom;
            egui::Grid::new("geo custom").num_columns(2).show(ui, |ui| {
                ui.label(tr("Name", "Название"));
                ui.text_edit_singleline(&mut c.name);
                ui.end_row();
                ui.label(tr("URL", "URL"));
                ui.add(egui::TextEdit::singleline(&mut c.url).hint_text("https://…/{z}/{x}/{y}.jpg").desired_width(f32::INFINITY));
                ui.end_row();
                ui.label(tr("Attribution", "Авторство"));
                ui.text_edit_singleline(&mut c.attribution);
                ui.end_row();
                ui.label(tr("Finest zoom", "Наибольший масштаб"));
                ui.add(egui::DragValue::new(&mut c.max_zoom).range(1..=23));
                ui.end_row();
                ui.label(tr("Kind", "Вид"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut c.kind, Kind::Imagery, tr("XYZ / WMTS tiles", "тайлы XYZ / WMTS"));
                    ui.radio_value(&mut c.kind, Kind::Wms, "WMS ({bbox})");
                    ui.radio_value(&mut c.kind, Kind::Terrarium, tr("Terrarium heights", "высоты Terrarium"));
                });
                ui.end_row();
            });
            ui.label(RichText::new(tr("{z} {x} {y} are filled in; a WMS gets {bbox} (EPSG:3857) and {width} {height}; {key} is your key.", "Подставляются {z} {x} {y}; WMS получает {bbox} (EPSG:3857) и {width} {height}; {key}: ваш ключ.")).small().color(pal().text_dim));
            let ok = !c.name.trim().is_empty() && c.url.starts_with("http") && (c.url.contains("{z}") || c.url.contains("{bbox}"));
            if ui.add_enabled(ok, egui::Button::new(icons::labelled(Icon::Add, tr("Add", "Добавить")))).clicked() {
                let mut s = c.clone();
                s.builtin = false;
                s.ortho = false;
                s.bounds = None;
                s.resolution = 0.0;
                s.needs_key = s.url.contains("{key}");
                s.tile_px = if s.kind == Kind::Wms { 512 } else { 256 };
                prefs.geo.sources.retain(|x| x.name != s.name);
                prefs.geo.sources.push(s);
                prefs.save();
                self.probe = None;
            }
        });
    }

    fn own_ui(&mut self, ui: &mut egui::Ui, ed: &mut Editor, keys: &[TileKey]) {
        crate::ui::section(ui, tr("Own pictures", "Свои картинки"));
        ui.label(RichText::new(tr("A scan, a screenshot, an old plan: laid on tiles and lined up by eye. No place on the Earth needed.", "Скан, скриншот, старый план: кладётся на тайлы и подгоняется на глаз. Размещение на Земле не нужно.")).color(pal().text_dim));
        let bound = keys.iter().fold(None, |r: Option<[i32; 4]>, k| Some(match r {
            None => [k.0, k.1, k.0, k.1],
            Some([a, b, c, d]) => [a.min(k.0), b.min(k.1), c.max(k.0), d.max(k.1)],
        }));
        let rect = self.rect.get_or_insert(bound.unwrap_or([0, 0, 0, 0]));
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Tiles from", "Тайлы от"));
            ui.add(egui::DragValue::new(&mut rect[0]).prefix("x "));
            ui.add(egui::DragValue::new(&mut rect[1]).prefix("y "));
            ui.label(tr("to", "до"));
            ui.add(egui::DragValue::new(&mut rect[2]).prefix("x "));
            ui.add(egui::DragValue::new(&mut rect[3]).prefix("y "));
        });
        let r = *rect;
        let r = [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])];
        if ui.button(icons::labelled(Icon::Import, tr("Load image file over these tiles…", "Загрузить картинку на эти тайлы…"))).clicked() {
            self.pick_picture(ed, r);
        }
        let ids: Vec<String> = self.file.pictures.keys().cloned().collect();
        for id in &ids {
            let p = &self.file.pictures[id];
            let n = p.tiles().count();
            let label = format!("{} ({} {}, {} {})", p.file, n, tr("tiles", "тайлов"), tr("from", "от"), format_args!("{}, {}", p.rect[0], p.rect[1]));
            if ui.selectable_label(self.pic_sel.as_deref() == Some(id), label).clicked() {
                self.pic_sel = Some(id.clone());
            }
        }
        let Some(id) = self.pic_sel.clone().filter(|i| self.file.pictures.contains_key(i)) else { return };
        let mut fit = self.file.pictures[&id].fit;
        let before = fit;
        let mut remove = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Turn left 90°", "Влево на 90°")).clicked() {
                fit.rotation = ((fit.rotation - 90.0 + 180.0).rem_euclid(360.0)) - 180.0;
            }
            if ui.button(tr("Turn right 90°", "Вправо на 90°")).clicked() {
                fit.rotation = ((fit.rotation + 90.0 + 180.0).rem_euclid(360.0)) - 180.0;
            }
            ui.add(egui::DragValue::new(&mut fit.rotation).speed(0.1).range(-180.0..=180.0).suffix("°"));
            ui.checkbox(&mut fit.flip_x, tr("mirror left-right", "отразить слева направо"));
            ui.checkbox(&mut fit.flip_y, tr("mirror top-bottom", "отразить сверху вниз"));
        });
        let (_, w, h, _) = Picture { fit, ..self.file.pictures[&id].clone() }.frame(ts());
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Move", "Сдвиг"));
            ui.add(egui::DragValue::new(&mut fit.offset[0]).speed(0.2).prefix("x ").suffix(" m"));
            ui.add(egui::DragValue::new(&mut fit.offset[1]).speed(0.2).prefix("y ").suffix(" m"));
            ui.label(tr("Size", "Размер"));
            let s0 = fit.scale.max(1e-3);
            ui.add(egui::DragValue::new(&mut fit.scale).speed(0.002).range(0.05..=20.0).custom_formatter(move |v, _| format!("{:.1} × {:.1} m", w / s0 * v, h / s0 * v)));
            if ui.small_button(tr("Reset", "Сбросить")).clicked() {
                fit = Fit::default();
            }
            if ui.small_button(icons::labelled(Icon::Delete, tr("Remove", "Убрать"))).clicked() {
                remove = true;
            }
        });
        if remove {
            let p = self.file.pictures.remove(&id).unwrap();
            for k in p.tiles() {
                if self.file.tile(k).own.as_deref() == Some(id.as_str()) {
                    self.file.tile_mut(k).own = None;
                    if let Some(d) = &self.dir {
                        let _ = std::fs::remove_file(g::own_path(d, k));
                    }
                }
            }
            if let Some(d) = &self.dir {
                let _ = std::fs::remove_file(g::dir_of(d).join("imagery").join(&p.file));
            }
            self.pic_sel = None;
            self.touch();
            return;
        }
        if fit != before {
            self.file.pictures.get_mut(&id).unwrap().fit = fit;
            self.fit_changed = Some(Instant::now());
            self.touch();
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, ed: &mut Editor, prefs: &GeoPrefs) {
        let Some((keys, terrain)) = self.confirm.clone() else { return };
        let own = keys.iter().filter(|k| self.file.tile(**k).own.is_some()).count();
        let mut answer = None;
        egui::Window::new(tr("Your own pictures", "Ваши картинки")).id(egui::Id::new("geo confirm")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("{own} {}", tr("of these tiles have your own pictures. Replace them with satellite imagery?", "из этих тайлов с вашими картинками. Заменить их спутниковыми снимками?")));
            ui.horizontal(|ui| {
                if ui.button(tr("Replace them", "Заменить")).clicked() {
                    answer = Some(true);
                }
                if ui.button(tr("Keep my pictures", "Оставить мои")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let Some(yes) = answer else { return };
        self.confirm = None;
        let keys: Vec<TileKey> = if yes { keys } else { keys.into_iter().filter(|k| self.file.tile(*k).own.is_none()).collect() };
        if !keys.is_empty() {
            self.load(ed, prefs, keys, true, terrain);
        }
    }

    /// `OPENMAPPER_GEO=lat,lon[,load][,hide][,picture=<file>[,turn=<deg>]][,aerial[=<folder>]]`
    /// (checks without a person at the screen): the map placed with its orange tile there, the
    /// window open (unless `hide`), with `load` imagery and heights loaded for every tile, with
    /// `picture` that file laid over all tiles, with `aerial` OMSI's own photos of the map taken
    /// (from the folder beside it, or from `=folder`).
    pub fn scripted(&mut self, ed: &mut Editor, prefs: &Prefs, arg: &str) {
        self.follow(ed);
        let parts: Vec<&str> = arg.split(',').map(str::trim).collect();
        if let Some(p) = parts.iter().find_map(|p| p.strip_prefix("aerial=")) {
            self.import_aerial(ed, Path::new(p));
        } else if parts.contains(&"aerial") {
            if let Some(f) = ed.doc.as_ref().and_then(|d| g::aerial::folder_of(&d.dir, d.source.as_deref())) {
                self.import_aerial(ed, &f);
            }
        }
        if let Some(p) = parts.iter().find_map(|p| p.strip_prefix("picture=")) {
            let keys: Vec<TileKey> = ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default();
            let r = keys.iter().fold([i32::MAX, i32::MAX, i32::MIN, i32::MIN], |r, k| [r[0].min(k.0), r[1].min(k.1), r[2].max(k.0), r[3].max(k.1)]);
            self.own_picture(ed, Path::new(p), r);
            if let (Some(t), Some(id)) = (parts.iter().find_map(|p| p.strip_prefix("turn=")).and_then(|t| t.parse::<f64>().ok()), self.pic_sel.clone()) {
                if let Some(pic) = self.file.pictures.get_mut(&id) {
                    pic.fit.rotation = t;
                }
                self.bake_pieces(&id, true);
            }
        }
        let (Some(la), Some(lo)) = (parts.first().and_then(|v| v.parse::<f64>().ok()), parts.get(1).and_then(|v| v.parse::<f64>().ok())) else { return };
        if !omsi_map::world_coordinates() {
            let t = self.anchor_tile(ed);
            self.file.anchor = Some(Anchor { tile: [t.0, t.1], lat: la, lon: lo, rotation: 0.0 });
        }
        self.map.look_at(la, lo, 15.5);
        self.open = !parts.contains(&"hide");
        // (`osm_map`: the drawn OpenStreetMap map laid on every tile, no terrain)
        if parts.contains(&"osm_map") {
            self.file.source = sources::OSM_STANDARD.to_string();
            self.file.overlay = true;
            let keys: Vec<TileKey> = ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default();
            self.load(ed, &prefs.geo, keys, true, false);
        } else if parts.contains(&"load") {
            self.file.with_terrain = true;
            let keys: Vec<TileKey> = ed.doc.as_ref().map(|d| d.tiles.keys().copied().collect()).unwrap_or_default();
            self.load(ed, &prefs.geo, keys, true, true);
        }
        self.touch();
    }
}

/// The overview map's copy of a picture: its mip level of at most 128 pixels.
fn mini_of(m: &Mips) -> egui::ColorImage {
    let l = m.0.iter().find(|l| l.width() <= 128).unwrap_or(m.0.last().unwrap());
    egui::ColorImage::from_rgba_unmultiplied([l.width() as usize, l.height() as usize], l.as_raw())
}

impl Drop for GeoUi {
    fn drop(&mut self) {
        self.flush();
    }
}
