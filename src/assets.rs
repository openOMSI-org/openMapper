//! Loading what the map shows, off the main thread: scenery objects (their model.cfg and
//! meshes), spline types and textures (with their PBR maps, found as openOMSI finds them).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use glam::{Vec2, Vec3};
use omsi_geometry::MeshData;
use omsi_scenery::{SceneryObject, Spline};
use omsi_texture::Image;

/// How a material treats the texture's alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alpha {
    Opaque,
    Test,
    Blend,
}

#[derive(Debug, Clone)]
pub struct Material {
    pub texture: Option<PathBuf>,
    pub color: [f32; 4],
    pub alpha: Alpha,
    /// What lights it at night: its `[matl_nightmap]` (shown glowing; true) or its
    /// `[matl_lightmap]` (lighting its texture; false).
    pub glow: Option<(PathBuf, bool)>,
}

/// One mesh of a model with its materials (`ranges` name them by slot).
pub struct Part {
    pub mesh: MeshData,
    pub materials: Vec<Material>,
    /// Material slots showing one of the model's text textures: (slot, text texture).
    pub texts: Vec<(usize, usize)>,
    /// Material slots whose picture an object's label picks (`[matl_freetex]`): (slot, the
    /// label). The label's value is a picture's name, looked for where the object's own
    /// textures are; an empty one leaves the slot's own picture, as in Omsi.exe.
    pub freetex: Vec<(usize, usize)>,
    /// The script variable that shows it (`[visible]`) and the value it shows at: a signal
    /// head's lens, lit by its crossing's program (None: always shown).
    pub visible: Option<(String, f32)>,
    /// The script variable that fades it (`[alphascale] red` and its like): the other way a
    /// signal head's lens is lit - its "on" picture shows while the variable is 1 (None: not
    /// faded; the lens meshes of the Hong Kong signal heads, which have no `[visible]`).
    pub faded: Option<String>,
}

impl Part {
    /// The lamp whose lens this is, if it is one: shown by `[visible]`, or faded in by
    /// `[alphascale]`. Both name the lamp variable `red`, `yellow` or `green`.
    pub fn lamp(&self) -> Option<crate::editable::Lamp> {
        let shown = self.visible.as_ref().and_then(|(var, _)| crate::editable::Lamp::of_var(var));
        shown.or_else(|| self.faded.as_deref().and_then(crate::editable::Lamp::of_var))
    }
}

#[derive(Default)]
pub struct ObjectModel {
    pub parts: Vec<Part>,
    pub lo: Vec3,
    pub hi: Vec3,
    pub abs_height: bool,
    /// A tree: drawn as two crossed cards of this texture folder, (texture, height, ratio).
    pub tree: Option<(Vec<PathBuf>, String, f32, f32)>,
    /// Where splines meet the object: (x, y, z, heading) in its frame, both ends of each.
    /// (and the spline type each one is for)
    pub helpers: Vec<([f32; 4], String)>,
    /// Its `[new_attachment]` points, in its frame.
    pub attachments: Vec<glam::Mat4>,
    /// Laid on the ground (`[surface]` or a ground-layer `[rendertype]`): the terrain is cut
    /// under it.
    pub surface: bool,
    /// `[terrainhole]` cutter meshes (object frame).
    pub holes: Vec<MeshData>,
    /// Its own lanes (`[path]`: junctions, stops), in its frame.
    pub paths: Vec<omsi_scenery::PathDef>,
    /// Not seen in the game (`[onlyeditor]`, a start point, a car park, a help arrow,
    /// nothing to draw): the editor shows it as a marker that casts no shadow.
    pub technical: bool,
    /// Its meshes are seen in the editor only (`[onlyeditor]`): a tree's helper box, while
    /// its tree is seen in the game.
    pub editor_parts: bool,
    /// Its `[texttexture]`s (drawn from the object's labels).
    pub texts: Vec<omsi_model::TextTexture>,
    /// What its labels mean (`[stringvarnamelist]`; empty names where only a text texture
    /// says a label is read). Empty: it reads no labels.
    pub labels: Vec<String>,
    /// The lights of its traffic light program (`[traffic_light]`, each with its phases): a
    /// crossing whose signal heads take their state from it (`[varparent]`).
    pub lights: Vec<omsi_scenery::TrafficLight>,
    /// Its `[traffic_lights_group]`: the seconds of the program's cycle (None: the phases'
    /// own sum).
    pub cycle: Option<f32>,
    /// A signal head (`[trafficlight]` with no program of its own): its first label names a
    /// light of its parent's program.
    pub light_head: bool,
}

impl ObjectModel {
    pub fn radius(&self) -> f32 {
        (self.hi - self.lo).length() * 0.5
    }
    pub fn centre(&self) -> Vec3 {
        (self.hi + self.lo) * 0.5
    }
    /// The names of its program's lights, in the order the crossing counts them.
    pub fn light_names(&self) -> Vec<String> {
        self.lights.iter().map(|l| l.name.clone()).collect()
    }
}

pub struct SplineModel {
    pub def: Spline,
    pub materials: Vec<Material>,
    pub half_width: f32,
}

impl SplineModel {
    /// A road one can see and join: drawn (a profile, not `[onlyeditor]`), and either
    /// carrying a street lane or drawn across its centre line. The invisible AI splines that
    /// maps lay under their roads are not.
    /// A track: rail lanes only (OMSI's path type 2). Tracks meet in switches and cross on
    /// the level, never in a junction plate.
    pub fn is_track(&self) -> bool {
        !self.def.paths.is_empty() && self.def.paths.iter().all(|q| q.kind == 2)
    }

    /// An AI path: lanes and nothing to see (`[onlyeditor]` or no drawn profile).
    pub fn is_ai_path(&self) -> bool {
        !self.def.paths.is_empty() && (self.def.only_editor || !self.def.profiles.iter().any(|p| p.points.len() >= 2))
    }

    /// How far above its line a spline is drawn (a wall, a fence, a hedge stand up from the
    /// line; a road, a marking and a kerb lie on it).
    pub fn top(&self) -> f32 {
        self.def.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.z).fold(0.0, f32::max).max(0.0)
    }

    /// Where the spline is drawn, in metres to the right of its line (`x0`..`x1`) and how far
    /// it stands up. A road, a kerb or a marking is the ribbon between its profile points; a
    /// wall or a fence (`standing`) is the face there. A line thinner than a hand is widened
    /// so it can be clicked at all. Nothing drawn: a metre and a half either side, as an AI path.
    pub fn surface_span(&self) -> (f32, f32, f32, bool) {
        let mut x0 = f32::INFINITY;
        let mut x1 = f32::NEG_INFINITY;
        let mut any = false;
        for q in self.def.profiles.iter().flat_map(|p| p.points.iter()) {
            any = true;
            x0 = x0.min(q.x);
            x1 = x1.max(q.x);
        }
        let top = self.top();
        if !any {
            return (-1.5, 1.5, 0.0, false);
        }
        let standing = top > 0.5;
        let slack = if standing { 0.06 } else { 0.2 };
        if x1 - x0 < slack {
            let mid = (x0 + x1) * 0.5;
            let half = slack * 0.5;
            x0 = mid - half;
            x1 = mid + half;
        }
        (x0, x1, top, standing)
    }

    pub fn joinable(&self) -> bool {
        let drawn = !self.def.only_editor && self.def.profiles.iter().any(|p| p.points.len() >= 2);
        let across = self.def.profiles.iter().any(|p| p.points.iter().any(|q| q.x < -0.5) && p.points.iter().any(|q| q.x > 0.5))
            || (self.def.profiles.iter().any(|p| p.points.iter().any(|q| q.x.abs() < 0.05)) && self.half_width > 2.5);
        drawn && (self.def.paths.iter().any(|q| q.kind == 0) || across)
    }
}

/// A tree's picture reduced to *where it is solid*: a click on a tree goes by the picture, as
/// the view cuts its transparent parts away. `w` × `h` texels, alpha only, `u` left to right,
/// `v` top to bottom.
pub struct TreeMask {
    pub w: usize,
    pub h: usize,
    pub alpha: Vec<u8>,
}

impl TreeMask {
    fn of(image: &Image) -> TreeMask {
        // (a click needs the shape, not the picture: 96 texels across is well under a pixel)
        let side = 96usize;
        let (w, h) = (image.width.max(1) as usize, image.height.max(1) as usize);
        let (mw, mh) = if w >= h { (side, (side * h / w).max(1)) } else { ((side * w / h).max(1), side) };
        let mut alpha = Vec::with_capacity(mw * mh);
        for y in 0..mh {
            let sy = (y * h / mh).min(h - 1);
            for x in 0..mw {
                let sx = (x * w / mw).min(w - 1);
                alpha.push(image.rgba[(sy * w + sx) * 4 + 3]);
            }
        }
        TreeMask { w: mw, h: mh, alpha }
    }

    /// Is the picture solid at `(u, v)`, each from 0 to 1?
    pub fn solid(&self, u: f32, v: f32) -> bool {
        let x = ((u * self.w as f32) as isize).clamp(0, self.w as isize - 1) as usize;
        let y = ((v * self.h as f32) as isize).clamp(0, self.h as isize - 1) as usize;
        self.alpha[y * self.w + x] > 100
    }
}

pub struct TextureImage {
    pub image: Image,
    pub normal: Option<Image>,
    pub orm: Option<Image>,
    /// normal (1 / 2 GL style), occlusion, roughness, metalness present
    pub pbr_flags: [f32; 4],
}

enum Req {
    Object(String, PathBuf),
    Spline(String, PathBuf),
    Texture(PathBuf),
}

enum Resp {
    Object(String, Option<Arc<ObjectModel>>),
    Spline(String, Option<Arc<SplineModel>>),
    Texture(PathBuf, Option<Arc<TextureImage>>),
}

pub enum Slot<T> {
    Loading,
    Ready(Arc<T>),
    Failed,
}

/// Everything loaded so far, keyed by lower-case paths.
pub struct Assets {
    tx: mpsc::Sender<Req>,
    rx: mpsc::Receiver<Resp>,
    pub objects: HashMap<String, Slot<ObjectModel>>,
    pub splines: HashMap<String, Slot<SplineModel>>,
    pub textures: HashMap<PathBuf, Slot<TextureImage>>,
    /// Textures that arrived since the renderer last looked.
    pub new_textures: Vec<PathBuf>,
    /// The solid part of each tree's picture, by its path (None: the picture says nothing,
    /// a tree drawn solid).
    tree_masks: HashMap<PathBuf, Option<Arc<TreeMask>>>,
    pub pending: usize,
    omsi: PathBuf,
    content: PathBuf,
}

/// The largest texture side kept (the editor need not hold 4K pictures).
const MAX_SIDE: u32 = 1024;

impl Assets {
    pub fn new(omsi: PathBuf, content: PathBuf) -> Assets {
        let (tx, wrx) = mpsc::channel::<Req>();
        let (wtx, rx) = mpsc::channel::<Resp>();
        let wrx = Arc::new(Mutex::new(wrx));
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8) - 1;
        for _ in 0..n.max(1) {
            let wrx = wrx.clone();
            let wtx = wtx.clone();
            let (omsi, content) = (omsi.clone(), content.clone());
            std::thread::spawn(move || loop {
                let req = { wrx.lock().unwrap().recv() };
                let Ok(req) = req else { break };
                let resp = match req {
                    Req::Object(k, p) => Resp::Object(k, load_object(&p, &omsi, &content).map(Arc::new)),
                    Req::Spline(k, p) => Resp::Spline(k, load_spline(&p, &omsi, &content).map(Arc::new)),
                    Req::Texture(p) => {
                        let t = load_texture(&p, &omsi, &content).map(Arc::new);
                        Resp::Texture(p, t)
                    }
                };
                if wtx.send(resp).is_err() {
                    break;
                }
            });
        }
        Assets { tx, rx, objects: HashMap::new(), splines: HashMap::new(), textures: HashMap::new(), new_textures: Vec::new(), tree_masks: HashMap::new(), pending: 0, omsi, content }
    }

    pub(crate) fn resolve(&self, rel: &str) -> PathBuf {
        let c = omsi_cfg::resolve_path(&self.content, rel);
        if omsi_cfg::vfs::exists(&c) {
            c
        } else {
            omsi_cfg::resolve_path(&self.omsi, rel)
        }
    }

    pub fn object(&mut self, rel: &str) -> Option<Arc<ObjectModel>> {
        let key = rel.to_lowercase();
        match self.objects.get(&key) {
            Some(Slot::Ready(m)) => Some(m.clone()),
            Some(_) => None,
            None => {
                let p = self.resolve(rel);
                self.objects.insert(key.clone(), Slot::Loading);
                self.pending += 1;
                let _ = self.tx.send(Req::Object(key, p));
                None
            }
        }
    }

    /// [`Assets::object`], waiting for the model when it has to be read first (for what is done
    /// once, on a click, not every frame): None if it cannot be read in `ms` milliseconds.
    pub fn object_wait(&mut self, rel: &str, ms: u64) -> Option<Arc<ObjectModel>> {
        let start = std::time::Instant::now();
        loop {
            if let Some(m) = self.object(rel) {
                return Some(m);
            }
            if matches!(self.objects.get(&rel.to_lowercase()), Some(Slot::Failed)) || start.elapsed().as_millis() as u64 > ms {
                return None;
            }
            self.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// [`Assets::spline`], waiting for the model (see [`Assets::object_wait`]).
    pub fn spline_wait(&mut self, rel: &str, ms: u64) -> Option<Arc<SplineModel>> {
        let start = std::time::Instant::now();
        loop {
            if let Some(m) = self.spline(rel) {
                return Some(m);
            }
            if matches!(self.splines.get(&rel.to_lowercase()), Some(Slot::Failed)) || start.elapsed().as_millis() as u64 > ms {
                return None;
            }
            self.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    pub fn spline(&mut self, rel: &str) -> Option<Arc<SplineModel>> {
        let key = rel.to_lowercase();
        match self.splines.get(&key) {
            Some(Slot::Ready(m)) => Some(m.clone()),
            Some(_) => None,
            None => {
                let p = self.resolve(rel);
                self.splines.insert(key.clone(), Slot::Loading);
                self.pending += 1;
                let _ = self.tx.send(Req::Spline(key, p));
                None
            }
        }
    }

    /// Where a tree's picture is solid, for the pick: read once per picture.
    ///
    /// A tree is drawn as two crossed cards cut by the picture's alpha (`Alpha::Test` in the
    /// view). The pick goes by the same picture, so a click through the gap between two
    /// branches, or through the empty corners of the card, reaches the road behind it - and a
    /// click on the leaves does not.
    pub fn tree_mask(&mut self, dirs: &[PathBuf], tex: &str) -> Option<Arc<TreeMask>> {
        if std::env::var_os("OPENMAPPER_TREE_SOLID").is_some() {
            return None;
        }
        let refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        let path = omsi_texture::find_texture(tex, &refs)?;
        if let Some(m) = self.tree_masks.get(&path) {
            return m.clone();
        }
        // (a picture with no alpha is drawn as a solid card: no shape to go by)
        let mask = omsi_texture::decode_file(&path)
            .ok()
            .filter(|i| i.has_alpha)
            .map(|i| Arc::new(TreeMask::of(&i)));
        self.tree_masks.insert(path, mask.clone());
        mask
    }

    /// The picture a `[matl_freetex]` label of `object` names: looked for where the object's
    /// own textures are, as Omsi.exe looks for it. The label's value may name a subfolder of
    /// the texture folder (`freetex\LTR12.png`, with backslashes as OMSI's labels use them).
    pub fn freetex_texture(&self, object: &str, name: &str) -> Option<PathBuf> {
        let name = name.trim().trim_matches('"').trim();
        if name.is_empty() {
            return None;
        }
        let p = self.resolve(object);
        let dir = p.parent()?;
        let dirs = texture_dirs(&self.omsi, &self.content, dir);
        find_tex(name, &dirs)
    }

    pub fn texture(&mut self, path: &Path) -> Option<Arc<TextureImage>> {
        match self.textures.get(path) {
            Some(Slot::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                self.textures.insert(path.to_path_buf(), Slot::Loading);
                self.pending += 1;
                let _ = self.tx.send(Req::Texture(path.to_path_buf()));
                None
            }
        }
    }

    /// Take finished loads; true when anything arrived.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(r) = self.rx.try_recv() {
            any = true;
            self.pending = self.pending.saturating_sub(1);
            match r {
                Resp::Object(k, m) => {
                    self.objects.insert(k, m.map(Slot::Ready).unwrap_or(Slot::Failed));
                }
                Resp::Spline(k, m) => {
                    self.splines.insert(k, m.map(Slot::Ready).unwrap_or(Slot::Failed));
                }
                Resp::Texture(p, t) => {
                    self.textures.insert(p.clone(), t.map(Slot::Ready).unwrap_or(Slot::Failed));
                    self.new_textures.push(p);
                }
            }
        }
        any
    }

    /// The picture could not be read (missing, cut short, a format nothing reads).
    pub fn texture_failed(&self, path: &Path) -> bool {
        matches!(self.textures.get(path), Some(Slot::Failed))
    }

    pub fn spline_failed(&self, rel: &str) -> bool {
        matches!(self.splines.get(&rel.to_lowercase()), Some(Slot::Failed))
    }

    pub fn object_failed(&self, rel: &str) -> bool {
        matches!(self.objects.get(&rel.to_lowercase()), Some(Slot::Failed))
    }

    /// Forget one object's model, and the pictures in its folder, so they are read again
    /// (its files were written anew).
    pub fn forget_object(&mut self, rel: &str, dir: &Path) {
        self.objects.remove(&rel.to_lowercase());
        self.textures.retain(|k, s| matches!(s, Slot::Loading) || !k.starts_with(dir));
    }

    /// Forget everything (after an import changed files on disk).
    pub fn clear(&mut self) {
        self.objects.retain(|_, s| matches!(s, Slot::Loading));
        self.splines.retain(|_, s| matches!(s, Slot::Loading));
        // (pictures too: a texture's PBR maps may have been made or taken away)
        self.textures.retain(|_, s| matches!(s, Slot::Loading));
    }
}

/// The folders a content file's textures are looked for in (as openOMSI does).
pub fn texture_dirs(omsi: &Path, content: &Path, dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![omsi_cfg::resolve_path(dir, "texture"), dir.to_path_buf()];
    if let Some(parent) = dir.parent() {
        dirs.push(omsi_cfg::resolve_path(parent, "texture"));
    }
    dirs.push(omsi_cfg::resolve_path(content, "Texture"));
    dirs.push(omsi_cfg::resolve_path(omsi, "Texture"));
    dirs
}

fn find_tex(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if name.trim().is_empty() {
        return None;
    }
    let refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
    omsi_texture::find_texture(name, &refs)
}

/// Which label of an object picks the picture of a `[matl_freetex]` slot.
///
/// The variable the tag names is a label of the object's `[stringvarnamelist]`, and its value
/// (the map's `[object]` record holds it) is the picture's name. Failing that, OMSI falls back
/// to the order the freetex slots are declared in, one label each — which is what a model whose
/// labels are unnamed relies on.
fn freetex_label(names: &[String], var: &str, order: usize) -> Option<usize> {
    let var = var.trim();
    if !var.is_empty() {
        // (by name, which is how the tutorial says to write it)
        if let Some(i) = names.iter().position(|n| n.trim().eq_ignore_ascii_case(var)) {
            return Some(i);
        }
        // (a variable that is a number names a label by its place, as a text texture does)
        if let Ok(i) = var.parse::<usize>() {
            return Some(i);
        }
    }
    (order < names.len()).then_some(order)
}

pub(crate) fn load_object(path: &Path, omsi: &Path, content: &Path) -> Option<ObjectModel> {
    let mut sco = SceneryObject::load(path).map_err(|e| log::debug!("{}: {e}", path.display())).ok()?;
    let sco_dir = path.parent()?.to_path_buf();
    let (model, model_dir) = match &sco.model_file {
        Some(m) => {
            let mp = omsi_cfg::resolve_path(&sco_dir, m);
            let model = omsi_model::Model::load(&mp).ok()?;
            let d = mp.parent()?.to_path_buf();
            (model, d)
        }
        None => (sco.model.clone(), sco_dir.clone()),
    };
    sco.inherit_model_tags(&model);
    let mut dirs = texture_dirs(omsi, content, &model_dir);
    if sco.model_file.is_some() {
        let own = omsi_cfg::resolve_path(&sco_dir, "texture");
        dirs.retain(|d| *d != own);
        dirs.insert(0, own);
    }
    let mesh_defs: Vec<&omsi_model::MeshDef> = if model.lods.is_empty() { model.meshes.iter().collect() } else { model.lod_meshes(0).iter().collect() };
    // (the names of the object's labels: a `[matl_freetex]` variable is one of them)
    let label_names = crate::texts::label_names(&sco.scripts.stringvarlists, &model.text_textures);
    let mut parts = Vec::new();
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    // (a signal head's lenses: the German ones say `[visible] red 1`, the Hong Kong ones
    // fade an "on" picture in by `[alphascale] red`; both name the lamp)
    let signal = sco.is_traffic_light;
    for md in mesh_defs {
        // (the meshes a script switches on are left out unless they show by default; but a
        // signal head's lenses, which its crossing's program lights, are kept - dark until
        // it lights one; the seasons' props stay shown, as the cows and hay bales of summer)
        let visible = match gate_of(md.visible.as_ref()) {
            Gate::Shown => None,
            Gate::Skipped => continue,
            Gate::Lamp(var, value) => Some((var, value)),
        };
        let faded = signal.then(|| md.materials.iter().find_map(|d| d.alphascale.as_deref()).filter(|v| crate::editable::Lamp::of_var(v).is_some()).map(str::to_string)).flatten();
        let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), &md.file);
        let mesh_path = if omsi_cfg::vfs::is_file(&in_model) { in_model } else { omsi_cfg::resolve_path(&model_dir, &md.file) };
        let Ok(m) = omsi_o3d::load_mesh(&mesh_path) else { continue };
        let mesh = omsi_geometry::mesh_from_o3d(&m);
        if mesh.is_empty() {
            continue;
        }
        for p in &mesh.positions {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        // a material's settings by its texture and the how-many-th use of it
        let mut seen: HashMap<String, i32> = HashMap::new();
        let mut texts = Vec::new();
        let mut freetex = Vec::new();
        let mut freetex_order = 0usize;
        let materials = m
            .materials
            .iter()
            .enumerate()
            .map(|(slot, mat)| {
                let key = mat.texture.to_lowercase();
                let n = seen.entry(key.clone()).or_insert(-1);
                *n += 1;
                let def = md.materials.iter().find(|d| d.texture.to_lowercase() == key && d.index == *n);
                if let Some(t) = def.and_then(|d| d.use_text_texture).filter(|t| *t >= 0 && (*t as usize) < model.text_textures.len()) {
                    texts.push((slot, t as usize));
                }
                // (a slot whose picture the map's label picks: `[matl_freetex] texture var`,
                // where `var` names a label of the object's `[stringvarnamelist]`)
                if let Some((_, var)) = def.and_then(|d| d.freetex.as_ref()) {
                    if let Some(i) = freetex_label(&label_names, var, freetex_order) {
                        freetex.push((slot, i));
                    }
                    freetex_order += 1;
                }
                let alpha = match def.map(|d| d.alpha).unwrap_or(0) {
                    1 => Alpha::Test,
                    2 => Alpha::Blend,
                    _ => Alpha::Opaque,
                };
                let glow = def.and_then(|d| d.nightmap.as_ref().and_then(|n| find_tex(n, &dirs)).map(|p| (p, true)).or_else(|| d.lightmap.as_ref().and_then(|l| find_tex(&l.0, &dirs)).map(|p| (p, false))));
                Material { texture: find_tex(&mat.texture, &dirs), color: mat.diffuse, alpha, glow }
            })
            .collect();
        parts.push(Part { mesh, materials, texts, freetex, visible, faded });
    }
    // a junction plate draped onto its `[crossing_heightdeformation]` field, as openOMSI
    // (and Omsi.exe, as it loads the type) does: its arms meet roads that do not lie at the
    // plate's height, and the file's normals give way to ones made from the faces
    if let Some(f) = &sco.crossing_height_deformation {
        let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), f);
        let p = if omsi_cfg::vfs::is_file(&in_model) { in_model } else { omsi_cfg::resolve_path(&model_dir, f) };
        match omsi_o3d::load_mesh(&p) {
            Ok(m) => {
                let field = omsi_geometry::mesh_from_o3d(&m);
                for part in &mut parts {
                    for v in part.mesh.positions.iter_mut() {
                        if let Some(d) = field_height(&field, v.x, v.y) {
                            v.z += d;
                        }
                    }
                    omsi_geometry::compute_normals_d3d(&mut part.mesh);
                }
            }
            Err(e) => log::warn!("crossing height deformation {}: {e}", p.display()),
        }
    }
    let tree = sco.tree.as_ref().map(|t| {
        let (tex, a, b, c, d) = t.clone();
        // (texture, the height range and the width ratio range)
        let height = if a > 0.0 && b > 0.0 { (a + b) * 0.5 } else { 10.0 };
        let ratio = if c > 0.0 && d > 0.0 { (c + d) * 0.5 } else { 0.7 };
        (dirs.clone(), tex, height, ratio)
    });
    if let Some((_, _, h, r)) = &tree {
        let w = h * r * 0.5;
        lo = lo.min(Vec3::new(-w, -w, 0.0));
        hi = hi.max(Vec3::new(w, w, *h));
    }
    if parts.is_empty() && tree.is_none() {
        // nothing to draw (sound sources, invisible helpers): a small marker
        lo = Vec3::new(-1.0, -1.0, 0.0);
        hi = Vec3::new(1.0, 1.0, 2.0);
    }
    let mut helpers = Vec::new();
    // OMSI writes a helper's place as x, height, forward (Direct3D's y up); openMapper's
    // junctions of before wrote x, forward, height and are read so still
    let legacy = std::fs::read(path).map(|b| { let t = omsi_cfg::decode_text(&b); t.contains("openMapper junction:") && !t.contains("helpers=omsi") }).unwrap_or(false);
    for h in &sco.spline_helpers {
        let (fwd, up) = if legacy { (h.y, h.z) } else { (h.z, h.y) };
        // both ends, each with the heading that leads out of the junction there
        helpers.push(([h.x, fwd, up, h.heading + 180.0], h.spline.clone()));
        let c = omsi_geometry::SplineCurve {
            start: glam::DVec3::new(h.x as f64, fwd as f64, up as f64),
            heading_deg: h.heading as f64,
            length: h.length as f64,
            radius: h.radius as f64,
            grad_start: 0.0,
            grad_end: 0.0,
            delta_h: None,
            cant_start: 0.0,
            cant_end: 0.0,
            skew_start: 0.0,
            skew_end: 0.0,
            tex_offset: 0.0,
            seed: 0,
            half_cant_width: 0.0,
        };
        let e = c.end_point();
        helpers.push(([e.x as f32, e.y as f32, e.z as f32, c.heading_at(c.length) as f32], h.spline.clone()));
    }
    let attachments = sco.attachments.iter().map(crate::place::attachment_matrix).collect();
    let surface = sco.render_type.is_ground_layer() || sco.surface;
    let mut holes = Vec::new();
    if sco.tree.is_none() && !sco.only_editor {
        for (dir, file) in sco.terrain_hole_sources(&model) {
            let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(dir, "model"), file);
            let p = if omsi_cfg::vfs::is_file(&in_model) { in_model } else { omsi_cfg::resolve_path(dir, file) };
            if let Ok(m) = omsi_o3d::load_mesh(&p) {
                holes.push(omsi_geometry::mesh_from_o3d(&m));
            }
        }
    }
    // (a tree with an editor-only helper box is seen in the game: only the box is not)
    let technical = (sco.only_editor && tree.is_none()) || sco.is_entry_point || sco.is_car_park || sco.is_help_arrow || (parts.is_empty() && tree.is_none());
    Some(ObjectModel { parts, lo, hi, abs_height: sco.absolute_height(), tree, helpers, attachments, surface, holes, technical, editor_parts: sco.only_editor, paths: sco.paths.clone(), texts: model.text_textures.clone(), labels: label_names, lights: sco.traffic_lights.clone(), cycle: sco.traffic_lights_group, light_head: sco.is_traffic_light && sco.traffic_lights.is_empty() })
}

fn load_spline(path: &Path, omsi: &Path, content: &Path) -> Option<SplineModel> {
    let def = Spline::load(path).ok()?;
    let dir = path.parent()?.to_path_buf();
    let dirs = texture_dirs(omsi, content, &dir);
    let materials = def
        .textures
        .iter()
        .map(|t| Material {
            texture: find_tex(&t.file, &dirs),
            glow: None,
            color: [1.0; 4],
            alpha: match t.alpha {
                1 => Alpha::Test,
                2 => Alpha::Blend,
                _ => Alpha::Opaque,
            },
        })
        .collect();
    let half_width = def.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.x.abs()).fold(0.0, f32::max);
    omsi_geometry::register_half_cant_width(&path.to_string_lossy(), &def);
    Some(SplineModel { def, materials, half_width })
}

fn load_texture(path: &Path, omsi: &Path, content: &Path) -> Option<TextureImage> {
    if crate::texts::is_text(path) {
        return Some(TextureImage { image: crate::texts::draw(path, omsi, content)?, normal: None, orm: None, pbr_flags: [0.0; 4] });
    }
    let image = shrink(omsi_texture::decode_file(path).map_err(|e| log::debug!("{e}")).ok()?);
    let mut files = omsi_texture::pbr::find(path);
    // (maps made here this session: the finder lists a folder once and would miss them)
    let [nn, orm, _] = crate::pbr_maker::files(path, omsi, content);
    if nn.is_file() && orm.is_file() {
        files.normal = Some(nn.clone());
        files.normal_gl = false;
        files.normals = vec![(nn, false)];
        files.packed = Some((orm, [0, 1, 2]));
        files.rough = None;
        files.metal = None;
        files.ao = None;
    }
    let (mut normal, mut orm, mut flags) = (None, None, [0.0; 4]);
    if !files.is_empty() {
        if let Some(set) = omsi_texture::pbr::load_set(&files) {
            normal = set.normal.map(shrink);
            orm = set.orm.map(shrink);
            flags = set.flags;
        }
    }
    Some(TextureImage { image, normal, orm, pbr_flags: flags })
}

/// Halve a picture until it fits `MAX_SIDE`.
fn shrink(mut img: Image) -> Image {
    while img.width > MAX_SIDE || img.height > MAX_SIDE {
        img = half(&img);
    }
    img
}

/// A picture at half the size (2×2 box filter).
pub fn half(img: &Image) -> Image {
    let w = (img.width / 2).max(1);
    let h = (img.height / 2).max(1);
    let mut out = vec![0u8; (w * h * 4) as usize];
    let sw = img.width as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..4 {
                let mut s = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(img.width as usize - 1);
                    let sy = (y * 2 + dy).min(img.height as usize - 1);
                    s += img.rgba[(sy * sw + sx) * 4 + c] as u32;
                }
                out[(y * w as usize + x) * 4 + c] = (s / 4) as u8;
            }
        }
    }
    Image { width: w, height: h, rgba: out, has_alpha: img.has_alpha }
}

/// Two crossed upright cards, 1 m wide and 1 m high (scaled per tree).
pub fn tree_mesh() -> MeshData {
    let mut m = MeshData::default();
    for (dx, dy) in [(1.0f32, 0.0f32), (0.0, 1.0)] {
        let base = m.positions.len() as u32;
        let a = Vec3::new(-0.5 * dx, -0.5 * dy, 0.0);
        let b = Vec3::new(0.5 * dx, 0.5 * dy, 0.0);
        m.positions.extend([a, b, b + Vec3::Z, a + Vec3::Z]);
        let n = Vec3::new(-dy, dx, 0.0);
        m.normals.extend([n; 4]);
        m.uvs.extend([Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)]);
        m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m.ranges.push((0, m.indices.len() as u32, 0));
    m
}

/// The height of a `[crossing_heightdeformation]` field over (x, y) in the object's frame,
/// where the field reaches (the highest face there; openOMSI's `field_height`).
fn field_height(m: &omsi_geometry::MeshData, x: f32, y: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
        let det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if det.abs() < 1e-9 {
            continue;
        }
        let l1 = ((b.x - a.x) * (y - a.y) - (x - a.x) * (b.y - a.y)) / det;
        let l2 = ((x - a.x) * (c.y - a.y) - (c.x - a.x) * (y - a.y)) / det;
        let l0 = 1.0 - l1 - l2;
        if l0 >= -1e-4 && l1 >= -1e-4 && l2 >= -1e-4 {
            let h = l0 * a.z + l2 * b.z + l1 * c.z;
            best = Some(best.map_or(h, |o: f32| o.max(h)));
        }
    }
    best
}

/// A script variable that only says the time of year (`sommerhalbjahr`): the meshes it
/// switches are there most of the time and the editor shows them.
fn is_season(var: &str) -> bool {
    let v = var.to_lowercase();
    ["sommer", "summer", "winter", "season", "jahreszeit"].iter().any(|w| v.contains(w))
}

/// How a mesh gated by `[visible]` is kept: as it comes (a season's prop, a mesh shown while
/// its variable is off), left out (a script's own switch, as a shop's sign), or a signal
/// head's lens, kept dark until its crossing's program lights it.
enum Gate {
    Shown,
    Skipped,
    Lamp(String, f32),
}

fn gate_of(visible: Option<&(String, f32)>) -> Gate {
    let Some((var, value)) = visible else { return Gate::Shown };
    if *value == 0.0 || is_season(var) {
        return Gate::Shown;
    }
    match crate::editable::Lamp::of_var(var) {
        Some(_) => Gate::Lamp(var.clone(), *value),
        None => Gate::Skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mesh gated by a script variable: a season's prop stays, a shop's sign is left out,
    /// and a signal's lens is kept (dark) for the crossing's program.
    #[test]
    fn a_mesh_switched_by_a_script_is_kept_only_for_a_lamp_or_a_season() {
        assert!(matches!(gate_of(None), Gate::Shown));
        // (shown while its variable is off, as openMapper shows them today)
        assert!(matches!(gate_of(Some(&("ShopSign".into(), 0.0))), Gate::Shown));
        assert!(matches!(gate_of(Some(&("ShopSign".into(), 1.0))), Gate::Skipped));
        assert!(matches!(gate_of(Some(&("sommerhalbjahr".into(), 1.0))), Gate::Shown));
        for (var, lamp) in [("Red", crate::editable::Lamp::Red), ("red", crate::editable::Lamp::Red), ("YELLOW", crate::editable::Lamp::Yellow), ("green", crate::editable::Lamp::Green)] {
            assert!(matches!(gate_of(Some(&(var.into(), 1.0))), Gate::Lamp(..)), "{var}");
            assert_eq!(crate::editable::Lamp::of_var(var), Some(lamp));
        }
        // (the other names a lamp's variable goes by: no lamp, no crash)
        assert!(matches!(gate_of(Some(&("Left".into(), 1.0))), Gate::Skipped));
        assert_eq!(crate::editable::Lamp::of_var("#12"), None);
    }

    /// A lens faded in by `[alphascale]` (the Hong Kong signal heads: no `[visible]`) is a
    /// lamp too, so the crossing's program lights it as it lights the German heads' lenses.
    #[test]
    fn a_lens_faded_by_a_variable_is_a_lamp() {
        let part = |faded: Option<&str>| Part { mesh: MeshData::default(), materials: Vec::new(), texts: Vec::new(), freetex: Vec::new(), visible: None, faded: faded.map(str::to_string) };
        assert_eq!(part(Some("red")).lamp(), Some(crate::editable::Lamp::Red));
        assert_eq!(part(Some("Yellow")).lamp(), Some(crate::editable::Lamp::Yellow));
        assert_eq!(part(Some("green")).lamp(), Some(crate::editable::Lamp::Green));
        // (another `[alphascale]`, as a sign's brightness: not a lamp)
        assert_eq!(part(Some("Licht")).lamp(), None);
        assert_eq!(part(None).lamp(), None);
        // (a `[visible]` lens, as the German heads have them, is still seen)
        let shown = Part { visible: Some(("red".into(), 1.0)), ..part(None) };
        assert_eq!(shown.lamp(), Some(crate::editable::Lamp::Red));
    }

    #[test]
    fn a_freetex_variable_names_one_of_the_objects_labels() {
        let names = vec!["Texture".to_string(), "Route".to_string()];
        // (the tutorial's way: the tag's variable is the label's name)
        assert_eq!(freetex_label(&names, "Texture", 0), Some(0));
        assert_eq!(freetex_label(&names, "route", 0), Some(1));
        assert_eq!(freetex_label(&names, " Texture ", 0), Some(0));
        // (a variable that is a number names a label by its place)
        assert_eq!(freetex_label(&names, "1", 0), Some(1));
        // (a name that matches nothing falls back to the order the slots are declared in)
        assert_eq!(freetex_label(&names, "nothing", 1), Some(1));
        // (no labels at all: nothing to pick, the slot keeps its own picture)
        assert_eq!(freetex_label(&[], "Texture", 0), None);
        // (fewer labels than slots: the later slots keep theirs)
        assert_eq!(freetex_label(&names, "nothing", 2), None);
    }

    /// The Hong Kong content's gantry: its two slots are painted with a picture the object does
    /// not ship, and the map's label names the one to use instead.
    #[test]
    fn a_gantry_takes_the_picture_its_label_names() {
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2 installation: skipped");
            return;
        };
        let content = std::env::temp_dir().join("openmapper-freetex-none");
        let sco = omsi_cfg::resolve_path(&omsi, "Sceneryobjects/76K_GM/FreetexGantry.sco");
        if !sco.is_file() {
            eprintln!("no FreetexGantry: skipped");
            return;
        }
        let m = load_object(&sco, &omsi, &content).expect("the gantry reads");
        assert_eq!(m.labels.first().map(String::as_str), Some("Texture"), "the label the freetex names");
        let (slot, label) = m.parts.iter().flat_map(|p| p.freetex.iter()).copied().next().expect("a freetex slot");
        assert_eq!(label, 0, "the label named Texture is the first");
        // (the slot's own picture is not in the object's folders, so it is bare without a label)
        let assets = Assets::new(omsi.clone(), content);
        let own = &m.parts[0].materials[slot];
        assert!(own.texture.is_none(), "the gantry ships no picture of its own");
        // (the picture the map's label names is found where the object's textures are)
        let found = assets.freetex_texture("Sceneryobjects/76K_GM/FreetexGantry.sco", "freetex\\LTR12.png");
        assert!(found.is_some(), "the label's picture is not found");
        assert!(found.unwrap().to_string_lossy().to_lowercase().ends_with("ltr12.png"));
        // (an empty label picks nothing: the slot keeps its own, as Omsi.exe leaves it)
        assert!(assets.freetex_texture("Sceneryobjects/76K_GM/FreetexGantry.sco", "  ").is_none());
    }

    /// The stock content's signal head: its body and its three lenses, which OMSI switches by
    /// the lamps' variables - all four kept, the lenses dark until a crossing lights them.
    #[test]
    fn a_signal_head_keeps_its_lenses_and_shows_no_lamp_by_itself() {
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else {
            eprintln!("no OMSI 2 installation: skipped");
            return;
        };
        let content = std::env::temp_dir().join("openmapper-ampel-none");
        let sco = omsi_cfg::resolve_path(&omsi, "Sceneryobjects/Verkehrszeichen_MC/Ampel_Kfz_1.sco");
        if !sco.is_file() {
            eprintln!("no Ampel_Kfz_1: skipped");
            return;
        }
        let m = load_object(&sco, &omsi, &content).expect("the signal head reads");
        assert_eq!(m.parts.len(), 4, "the body and a lens for each lamp");
        let lamps: Vec<crate::editable::Lamp> = m.parts.iter().filter_map(Part::lamp).collect();
        assert_eq!(lamps, [crate::editable::Lamp::Red, crate::editable::Lamp::Yellow, crate::editable::Lamp::Green]);
        assert_eq!(m.parts.iter().filter(|p| p.visible.is_none()).count(), 1, "the body is not switched");
        assert!(m.light_head, "it takes its state from a crossing");
    }

    /// A crossing's program: not only the lights' names but their phases and the cycle, which
    /// is what the signal heads standing at it are lit by.
    #[test]
    fn a_crossing_keeps_its_light_program() {
        let dir = std::env::temp_dir().join(format!("om-crossing-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("crossing.sco");
        // (a main road's light and a side road's, as openMapper's junctions write them)
        let text = "[friendlyname]\nCrossing\n\n[traffic_lights_group]\n30\n\n[traffic_light]\nMain road\n\n[phase]\n3\n2\n\n[phase]\n6\n20\n\n[phase]\n9\n3\n\n[phase]\n0\n0\n\n[traffic_light]\nSide roads\n\n[phase]\n0\n22\n\n[phase]\n3\n2\n\n[phase]\n6\n5\n\n[phase]\n9\n3\n";
        std::fs::write(&path, text).expect("the crossing writes");
        let m = load_object(&path, &dir, &dir).expect("the crossing reads");
        assert_eq!(m.cycle, Some(30.0), "the group's cycle");
        assert_eq!(m.light_names(), ["Main road", "Side roads"]);
        let phases: Vec<(i32, f32)> = m.lights[0].phases.iter().map(|p| (p.state, p.duration)).collect();
        assert_eq!(phases, [(3, 2.0), (6, 20.0), (9, 3.0), (0, 0.0)]);
        assert_eq!(m.lights[1].phases.len(), 4, "the side road's light keeps its own");
        assert!(!m.light_head, "it runs a program of its own");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A wall, a fence or a hedge is drawn *up* from its line; a road and a marking lie on it.
    /// What a click is matched against is that drawn height: London's 4 m retaining walls were
    /// unclickable down their faces - a click two metres up was a click on the air above the
    /// line - until the pick went by it.
    #[test]
    fn a_spline_is_as_tall_as_it_is_drawn() {
        let dir = std::env::temp_dir().join(format!("om-sli-top-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let load = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).expect("the spline writes");
            let def = Spline::load(&path).expect("the spline reads");
            // (as `load_spline` measures it)
            let half_width = def.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.x.abs()).fold(0.0, f32::max);
            SplineModel { def, materials: Vec::new(), half_width }
        };
        // (3 m of wall on its line, nothing across)
        let wall = load("wall.sli", "[profile]\n0\n[profilepnt]\n0.0\n0.0\n0\n0.3\n[profilepnt]\n0.0\n3.0\n1\n0.3\n");
        assert_eq!(wall.top(), 3.0, "the wall stands up from its line");
        // (a road: 7 m across, flat on its line)
        let road = load("road.sli", "[profile]\n0\n[profilepnt]\n-3.5\n0.0\n0\n0.3\n[profilepnt]\n3.5\n0.0\n1\n0.3\n");
        assert_eq!(road.top(), 0.0, "a road lies on its line");
        assert_eq!(road.half_width, 3.5, "and is as wide as it is drawn");
        assert_eq!(road.surface_span(), (-3.5, 3.5, 0.0, false), "a click anywhere on the drawn road");
        let kerb_face = load("kerb_face.sli", "[profile]\n0\n[profilepnt]\n1.8\n0.0\n0\n0.3\n[profilepnt]\n2.1\n0.15\n1\n0.3\n");
        let (a, b, _, up) = kerb_face.surface_span();
        assert!(!up, "a kerb lies on the ground");
        assert!(a > 1.0 && b < 2.5 && a < 2.0 && b > 2.0, "its surface is beside the line, not on it: {a}..{b}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
