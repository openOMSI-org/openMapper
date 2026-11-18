//! What shows the trolleybus wire on the map, made from a **kit**: a `.cfg` file the map maker
//! writes (in the Trolleybus wire window, or by hand) saying what the wire is - how many wires,
//! how far apart, how high, how thick, what colour - how masts are laid (every so many metres,
//! the arm reaching so far), and what the mast, the frog and the section insulator look like,
//! each built of parts (boxes, cylinders, cones, spheres, tubes, capsules, or any object of the
//! content). openMapper ships none: from a kit it writes ordinary OMSI content - a wire spline
//! (`Splines\openMapper\Trolley`, wires high over a lane, as trolleybus maps for OMSI 2 draw
//! theirs, so openOMSI also reads them as the wire) and the objects
//! (`Sceneryobjects\openMapper\Trolley`) - named after the kit and what it says, so a changed
//! kit makes new files.
//!
//! A kit is the map's (`<map>\trolley\<name>.cfg`) or shared by every map
//! (`<content>\openMapper\trolley\<name>.cfg`).
//!
//! Laying wire over a road puts a copy of each road piece of the wire type on the map and masts
//! beside it every so many metres; wire over a junction's lanes (or drawn by hand) is laid as
//! short straight wire pieces. What a span laid is kept with it (`Span::laid`) and goes with
//! it; the frogs and insulators are put again wherever the wire's are after every change. All
//! of it with the wire's file in one undo step.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DMat4, DVec2, DVec3, Mat4, Vec3};
use omsi_map::{MapObject, MapSpline};
use openmapper_mesh::primitives::Primitive;
use openmapper_mesh::{Material, Mesh, Model, Part};

use crate::doc::{tile_of, tile_origin, Document};
use crate::editable::Source;
use crate::editor::Editor;
use crate::history::History;
use crate::i18n::tr;
use crate::install::Roots;
use crate::trolley::{RoadLane, Span, Wire};

/// The folders of what openMapper makes from the kits.
const SPLINES: &str = "Splines\\openMapper\\Trolley";
const OBJECTS: &str = "Sceneryobjects\\openMapper\\Trolley";

// ------------------------------------------------------------------------------ the kit

/// A part of a kit's object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Box,
    Cylinder,
    Cone,
    Sphere,
    Tube,
    Capsule,
    /// Another object of the content, as it is (`PartDef::file`).
    Object,
}

pub const SHAPES: [Shape; 7] = [Shape::Box, Shape::Cylinder, Shape::Cone, Shape::Sphere, Shape::Tube, Shape::Capsule, Shape::Object];

impl Shape {
    fn word(self) -> &'static str {
        match self {
            Shape::Box => "box",
            Shape::Cylinder => "cylinder",
            Shape::Cone => "cone",
            Shape::Sphere => "sphere",
            Shape::Tube => "tube",
            Shape::Capsule => "capsule",
            Shape::Object => "object",
        }
    }

    fn of(w: &str) -> Option<Shape> {
        SHAPES.into_iter().find(|s| s.word().eq_ignore_ascii_case(w.trim()))
    }

    pub fn label(self) -> &'static str {
        match self {
            Shape::Box => tr("Box", "Брус"),
            Shape::Cylinder => tr("Cylinder", "Цилиндр"),
            Shape::Cone => tr("Cone", "Конус"),
            Shape::Sphere => tr("Sphere", "Сфера"),
            Shape::Tube => tr("Tube", "Труба"),
            Shape::Capsule => tr("Capsule", "Капсула"),
            Shape::Object => tr("An object", "Объект"),
        }
    }

    /// What the three numbers of its size are.
    pub fn size_names(self) -> [&'static str; 3] {
        match self {
            Shape::Box => [tr("width", "ширина"), tr("depth", "глубина"), tr("height", "высота")],
            Shape::Cylinder | Shape::Capsule => [tr("radius", "радиус"), tr("length", "длина"), ""],
            Shape::Cone => [tr("radius below", "радиус снизу"), tr("length", "длина"), tr("radius above", "радиус сверху")],
            Shape::Sphere => [tr("radius", "радиус"), "", ""],
            Shape::Tube => [tr("outer radius", "внешний радиус"), tr("length", "длина"), tr("inner radius", "внутренний радиус")],
            Shape::Object => [tr("scale", "масштаб"), "", ""],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartDef {
    pub shape: Shape,
    /// Metres: x right, y forward, z up, from the object's origin (a mast's foot).
    pub pos: [f32; 3],
    /// Degrees about x, y and z, in that order.
    pub rot: [f32; 3],
    pub size: [f32; 3],
    pub colour: [u8; 3],
    /// For `Shape::Object`: its file in the content (`Sceneryobjects\…\x.sco`).
    pub file: String,
}

impl PartDef {
    pub fn new(shape: Shape) -> PartDef {
        let size = match shape {
            Shape::Box => [0.2, 0.2, 0.2],
            Shape::Cylinder | Shape::Capsule => [0.1, 1.0, 0.0],
            Shape::Cone => [0.15, 1.0, 0.1],
            Shape::Sphere => [0.15, 0.0, 0.0],
            Shape::Tube => [0.1, 1.0, 0.08],
            Shape::Object => [1.0, 0.0, 0.0],
        };
        PartDef { shape, pos: [0.0; 3], rot: [0.0; 3], size, colour: [110, 112, 116], file: String::new() }
    }

    /// Its mesh where it stands (none for an object of the content).
    pub fn mesh(&self) -> Option<Mesh> {
        let [a, b, c] = self.size.map(|v| v.max(0.001));
        let p = match self.shape {
            Shape::Box => Primitive::Cube { size: Vec3::new(a, b, c) },
            Shape::Cylinder => Primitive::Cylinder { radius: a, height: b, segments: 12 },
            Shape::Cone => Primitive::Cone { radius_bottom: a, radius_top: self.size[2].max(0.0), height: b, segments: 12 },
            Shape::Sphere => Primitive::UvSphere { radius: a, segments: 12, rings: 8 },
            Shape::Tube => Primitive::Tube { outer: a, inner: c.min(a * 0.99), height: b, segments: 12 },
            Shape::Capsule => Primitive::Capsule { radius: a, height: b, segments: 12, rings: 4 },
            Shape::Object => return None,
        };
        let mut m = p.build();
        m.transform(self.matrix());
        for f in &mut m.faces {
            f.smooth = p.round();
        }
        Some(m)
    }

    pub fn matrix(&self) -> Mat4 {
        let r = self.rot.map(f32::to_radians);
        Mat4::from_translation(Vec3::from(self.pos)) * Mat4::from_rotation_z(r[2]) * Mat4::from_rotation_y(r[1]) * Mat4::from_rotation_x(r[0])
    }
}

/// The three objects of a kit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    Mast,
    Frog,
    Insulator,
}

pub const PIECES: [Piece; 3] = [Piece::Mast, Piece::Frog, Piece::Insulator];

impl Piece {
    fn word(self) -> &'static str {
        match self {
            Piece::Mast => "mast",
            Piece::Frog => "frog",
            Piece::Insulator => "insulator",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Piece::Mast => tr("Mast", "Опора"),
            Piece::Frog => tr("Frog", "Развилка"),
            Piece::Insulator => tr("Section insulator", "Секционный изолятор"),
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Piece::Mast => tr("Stands beside the road, its foot at the origin; its arm reaches along -y to the wire, which hangs `reach` metres away at the wire's height (the guide in the drawing).", "Стоит у дороги, основание в начале координат; кронштейн тянется вдоль -y к проводу, который висит в `вылет` метрах на высоте провода (направляющая на рисунке)."),
            Piece::Frog => tr("Hangs where the wire branches, the origin on the wire, y along it.", "Висит там, где провод ветвится; начало координат на проводе, y вдоль него."),
            Piece::Insulator => tr("Hangs where two sections meet, the origin on the wire, y along it.", "Висит на стыке двух секций; начало координат на проводе, y вдоль него."),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KitDef {
    pub name: String,
    /// How many wires (2: a trolleybus's pair).
    pub wires: u32,
    /// Metres between neighbouring wires.
    pub spacing: f64,
    /// The wires' height over the road.
    pub height: f64,
    /// A wire's thickness (m).
    pub thickness: f64,
    pub colour: [u8; 3],
    /// Metres between masts along a road.
    pub step: f64,
    /// Metres from a mast to the wire it holds (its arm).
    pub reach: f64,
    pub meshes: [Vec<PartDef>; 3],
}

impl Default for KitDef {
    /// A kit to start from: the wire as trolleybus nets mostly have it, no mesh at all.
    fn default() -> KitDef {
        KitDef { name: String::new(), wires: 2, spacing: 0.6, height: 5.6, thickness: 0.012, colour: [46, 42, 38], step: 30.0, reach: 2.5, meshes: Default::default() }
    }
}

fn f(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl KitDef {
    pub fn mesh(&self, p: Piece) -> &Vec<PartDef> {
        &self.meshes[p as usize]
    }

    pub fn mesh_mut(&mut self, p: Piece) -> &mut Vec<PartDef> {
        &mut self.meshes[p as usize]
    }

    /// The kit as its `.cfg` file.
    pub fn text(&self) -> String {
        let mut s = String::from("# A trolleybus wire kit of openMapper (Map > Trolleybus wire). Lengths in metres,\n# angles in degrees, colours as red green blue (0-255); x right, y forward, z up.\n\n");
        let _ = writeln!(s, "[name]\n{}\n", self.name.replace(['\r', '\n'], " "));
        let _ = writeln!(s, "[wire]\n# wires, metres apart, height over the road, thickness, colour\n{}\n{}\n{}\n{}\n{} {} {}\n", self.wires, f(self.spacing), f(self.height), f(self.thickness), self.colour[0], self.colour[1], self.colour[2]);
        let _ = writeln!(s, "[masts]\n# metres between masts, metres from a mast to its wire\n{}\n{}\n", f(self.step), f(self.reach));
        for p in PIECES {
            for part in self.mesh(p) {
                let v = |a: [f32; 3]| a.map(|x| f(x as f64)).join(" ");
                let _ = writeln!(s, "[part]\n{}\n{}\n{}\n{}\n{}\n{} {} {}", p.word(), part.shape.word(), v(part.pos), v(part.rot), v(part.size), part.colour[0], part.colour[1], part.colour[2]);
                if part.shape == Shape::Object {
                    let _ = writeln!(s, "{}", part.file);
                }
                s.push('\n');
            }
        }
        s
    }

    /// A kit read from its `.cfg` (what it does not understand is left out).
    pub fn parse(text: &str) -> KitDef {
        let mut k = KitDef { meshes: Default::default(), ..KitDef::default() };
        let mut lines = text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()).peekable();
        let nums = |l: &str| -> Vec<f64> { l.split_whitespace().filter_map(|x| x.parse().ok()).collect() };
        let three = |l: Option<&str>| -> [f32; 3] {
            let n = l.map(nums).unwrap_or_default();
            [0, 1, 2].map(|i| n.get(i).copied().unwrap_or(0.0) as f32)
        };
        let rgb = |l: Option<&str>| -> [u8; 3] { three(l).map(|v| v.clamp(0.0, 255.0) as u8) };
        while let Some(l) = lines.next() {
            match l.to_ascii_lowercase().as_str() {
                "[name]" => k.name = lines.next().unwrap_or("").to_string(),
                "[wire]" => {
                    k.wires = lines.next().and_then(|l| l.parse().ok()).unwrap_or(2).clamp(1, 8);
                    k.spacing = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0.6);
                    k.height = lines.next().and_then(|l| l.parse().ok()).unwrap_or(5.6);
                    k.thickness = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0.012);
                    k.colour = rgb(lines.next());
                }
                "[masts]" => {
                    k.step = lines.next().and_then(|l| l.parse().ok()).unwrap_or(30.0);
                    k.reach = lines.next().and_then(|l| l.parse().ok()).unwrap_or(2.5);
                }
                "[part]" => {
                    let piece = lines.next().unwrap_or("");
                    let shape = lines.next().and_then(Shape::of);
                    let (pos, rot, size, colour) = (three(lines.next()), three(lines.next()), three(lines.next()), rgb(lines.next()));
                    let file = if shape == Some(Shape::Object) && lines.peek().is_some_and(|l| !l.starts_with('[')) { lines.next().unwrap_or("").to_string() } else { String::new() };
                    if let (Some(p), Some(shape)) = (PIECES.into_iter().find(|p| p.word().eq_ignore_ascii_case(piece)), shape) {
                        k.mesh_mut(p).push(PartDef { shape, pos, rot, size, colour, file });
                    }
                }
                _ => {}
            }
        }
        k
    }

    /// What the files made from the kit are named after: its name and what it says.
    fn stamp(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text().hash(&mut h);
        let slug: String = self.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        format!("{}_{:08x}", if slug.trim_matches('_').is_empty() { "Kit" } else { slug.trim_matches('_') }, h.finish() as u32)
    }

    /// The object a piece is made of, as an openMapper object (its parts' meshes, an object of
    /// the content as it is); None when the kit has no parts for it.
    pub fn source(&self, roots: &Roots, p: Piece, tmp: &Path) -> Option<Source> {
        let parts = self.mesh(p);
        if parts.is_empty() {
            return None;
        }
        let mut materials: Vec<Material> = Vec::new();
        let mut body = Mesh::default();
        for part in parts {
            let Some(m) = part.mesh() else { continue };
            let c = part.colour.map(|v| v as f32 / 255.0);
            let i = materials.iter().position(|x| x.color[..3] == c).unwrap_or_else(|| {
                materials.push(Material::colored(&format!("Colour {} {} {}", part.colour[0], part.colour[1], part.colour[2]), [c[0], c[1], c[2], 1.0]));
                materials.len() - 1
            });
            body.append(&m, i as u32);
        }
        let mut src = Source { name: format!("{} ({})", self.name, p.label()), model: Model { parts: Vec::new(), materials }, ..Default::default() };
        if !body.faces.is_empty() {
            src.model.parts.push(Part::new("Body", body));
        }
        for part in parts.iter().filter(|x| x.shape == Shape::Object && !x.file.trim().is_empty()) {
            match crate::editable::source_of(roots, part.file.trim(), tmp) {
                Ok(other) => {
                    let s = part.size[0].max(0.001) as f64;
                    let m = DMat4::from_cols_array(&part.matrix().to_cols_array().map(f64::from)) * DMat4::from_scale(DVec3::splat(s));
                    crate::editable::merge_into(&mut src, DMat4::IDENTITY, &other, m);
                }
                Err(e) => log::warn!("{}: {e}", part.file),
            }
        }
        (!src.model.parts.is_empty()).then_some(src)
    }
}

/// Where a kit is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// In the map's folder: this map only.
    Map,
    /// In the content folder: every map.
    Shared,
}

/// A kit on disk: `map:<stem>` or `shared:<stem>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitRef {
    pub scope: Scope,
    pub stem: String,
}

impl KitRef {
    pub fn id(&self) -> String {
        format!("{}:{}", if self.scope == Scope::Map { "map" } else { "shared" }, self.stem)
    }

    pub fn of_id(id: &str) -> Option<KitRef> {
        let (s, stem) = id.split_once(':')?;
        let scope = match s {
            "map" => Scope::Map,
            "shared" => Scope::Shared,
            _ => return None,
        };
        Some(KitRef { scope, stem: stem.to_string() })
    }

    fn dir(scope: Scope, roots: &Roots, map_dir: &Path) -> PathBuf {
        match scope {
            Scope::Map => map_dir.join("trolley"),
            Scope::Shared => roots.content.join("openMapper").join("trolley"),
        }
    }

    pub fn path(&self, roots: &Roots, map_dir: &Path) -> PathBuf {
        KitRef::dir(self.scope, roots, map_dir).join(format!("{}.cfg", self.stem))
    }
}

/// A file name for a kit's name.
pub fn stem_of(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.trim_matches('_').is_empty() { "kit".into() } else { s }
}

/// Every kit the map can use: its own, then the shared ones.
pub fn list(roots: &Roots, map_dir: &Path) -> Vec<(KitRef, KitDef)> {
    let mut out = Vec::new();
    for scope in [Scope::Map, Scope::Shared] {
        let mut here: Vec<(KitRef, KitDef)> = std::fs::read_dir(KitRef::dir(scope, roots, map_dir))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "cfg").then_some(())?;
                let stem = p.file_stem()?.to_string_lossy().to_string();
                let text = omsi_cfg::decode_text(&std::fs::read(&p).ok()?);
                Some((KitRef { scope, stem }, KitDef::parse(&text)))
            })
            .collect();
        here.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()));
        out.extend(here);
    }
    out
}

pub fn load(roots: &Roots, map_dir: &Path, id: &str) -> Option<KitDef> {
    let r = KitRef::of_id(id)?;
    Some(KitDef::parse(&omsi_cfg::decode_text(&std::fs::read(r.path(roots, map_dir)).ok()?)))
}

pub fn save(roots: &Roots, map_dir: &Path, r: &KitRef, k: &KitDef) -> anyhow::Result<()> {
    let p = r.path(roots, map_dir);
    std::fs::create_dir_all(p.parent().unwrap_or(map_dir))?;
    crate::doc::safe::write_atomic(&p, k.text().replace('\n', "\r\n").as_bytes())?;
    crate::hot::own_change();
    Ok(())
}

/// A kit moved between the map and every map (its file: the map's wire names it again).
pub fn move_to(roots: &Roots, map_dir: &Path, r: &KitRef, to: Scope) -> anyhow::Result<KitRef> {
    let new = KitRef { scope: to, stem: r.stem.clone() };
    let (a, b) = (r.path(roots, map_dir), new.path(roots, map_dir));
    anyhow::ensure!(!b.exists(), "{}", tr("a kit of that name is there already", "комплект с таким именем уже есть"));
    std::fs::create_dir_all(b.parent().unwrap_or(map_dir))?;
    std::fs::rename(&a, &b).or_else(|_| std::fs::copy(&a, &b).and_then(|_| std::fs::remove_file(&a)))?;
    Ok(new)
}

// ------------------------------------------------------------------------- the files

/// The wire spline of a kit: its wires `x` right of the spline's line. Written once; its
/// relative path.
pub fn wire_spline(content: &Path, k: &KitDef, x: f64) -> anyhow::Result<String> {
    let stamp = k.stamp();
    let rel = format!("{SPLINES}\\{stamp}_x{}.sli", (x * 100.0).round() as i64);
    let path = omsi_cfg::resolve_path(content, &rel);
    if path.is_file() {
        return Ok(rel);
    }
    let dir = path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(dir.join("texture"))?;
    let tex_name = format!("{stamp}.tga");
    let tex = dir.join("texture").join(&tex_name);
    if !tex.is_file() {
        image::RgbaImage::from_pixel(4, 4, image::Rgba([k.colour[0], k.colour[1], k.colour[2], 255])).save(&tex)?;
    }
    // (each wire a narrow V: seen from below and the side, thin enough that openOMSI takes a
    // pair for the wire when the map has no trolley.txt)
    let r = (k.thickness * 0.5).max(0.002);
    let mut s = format!("File created with openMapper from the trolleybus wire kit {}\n\n[friendlyname]\n{} {:+.2} m\n\n[texture]\n{tex_name}\n\n", k.name, k.name, x);
    let n = k.wires.max(1);
    for i in 0..n {
        let c = x + (i as f64 - (n - 1) as f64 * 0.5) * k.spacing;
        s += "[profile]\n0\n\n";
        for (dx, dz, u) in [(-r, 0.0, 0.0), (0.0, -r, 0.5), (r, 0.0, 1.0)] {
            s += &format!("[profilepnt]\n{:.4}\n{:.4}\n{u}\n1\n\n", c + dx, k.height + dz);
        }
    }
    crate::doc::safe::write_atomic(&path, &omsi_cfg::encode_text(&s.replace('\n', "\r\n")))?;
    Ok(rel)
}

/// A piece of the kit written once; its relative path (None: the kit has no mesh for it).
fn object(roots: &Roots, k: &KitDef, p: Piece) -> Option<String> {
    let stamp = k.stamp();
    let rel = format!("{OBJECTS}\\{stamp}\\{}.sco", p.word());
    if !omsi_cfg::resolve_path(&roots.content, &rel).is_file() {
        let tmp = omsi_cfg::resolve_path(&roots.content, &format!("{OBJECTS}\\{stamp}"));
        let src = k.source(roots, p, &tmp)?;
        if let Err(e) = crate::editable::save(roots, &rel, &src) {
            log::warn!("{rel}: {e}");
            return None;
        }
    }
    Some(rel)
}

/// Whether a file is one made from a kit (the map's wire, its masts, frogs and insulators).
pub fn is_kit(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    // (with the folder's backslash: a map called Trolley… has its junctions beside it)
    [SPLINES, OBJECTS].iter().any(|d| f.starts_with(&format!("{}\\", d.to_ascii_lowercase())))
}

/// A frog's or an insulator's object (put again after every change of the wire).
fn is_mark(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    is_kit(&f) && (f.ends_with("\\frog.sco") || f.ends_with("\\insulator.sco"))
}

/// Wire pieces and masts of a kit on the map that no span of the wire names (the wire's file
/// taken away by hand, or the span deleted outside openMapper): where they stand.
pub fn orphans(doc: &Document) -> Vec<(crate::doc::Sel, DVec2)> {
    let named: std::collections::HashSet<i64> = doc.traffic.trolley.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).collect();
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for o in t.tile.objects.iter().filter(|o| is_kit(&o.file) && !is_mark(&o.file) && !named.contains(&o.id)) {
            out.push((crate::doc::Sel::Object(*k, o.id), org + DVec2::new(o.pos[0], o.pos[1])));
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted && is_kit(&s.file) && !named.contains(&s.id)) {
            out.push((crate::doc::Sel::Spline(*k, s.id), org + DVec2::new(s.pos[0], s.pos[1])));
        }
    }
    out
}

// ------------------------------------------------------------------------- on the map

fn put_object(doc: &mut Document, hist: &mut History, file: &str, at: DVec3, heading: f64) -> i64 {
    let id = doc.new_id();
    let k = tile_of(at.truncate());
    if !doc.tiles.contains_key(&k) {
        return id;
    }
    hist.touch_tile(doc, k);
    let local = at.truncate() - tile_origin(k);
    let above = at.z - doc.ground(at.truncate());
    let t = doc.tiles.get_mut(&k).unwrap();
    t.tile.objects.push(MapObject { file: file.to_string(), id, pos: [local.x, local.y, above], rot: [crate::roads::norm_deg(heading), 0.0, 0.0], ..Default::default() });
    t.touch_content();
    id
}

fn put_spline(doc: &mut Document, hist: &mut History, s: MapSpline, at: DVec2) -> i64 {
    let k = tile_of(at);
    if !doc.tiles.contains_key(&k) {
        return s.id;
    }
    hist.touch_tile(doc, k);
    let id = s.id;
    let t = doc.tiles.get_mut(&k).unwrap();
    t.tile.splines.push(s);
    t.touch_content();
    id
}

/// The points of a line without those that lie on the straight between their neighbours.
fn simplified(pts: &[DVec3], tol: f64) -> Vec<DVec3> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let ab = b - a;
    let far = (1..pts.len() - 1)
        .map(|i| {
            let t = if ab.length_squared() > 1e-9 { ((pts[i] - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            (i, (a + ab * t).distance(pts[i]))
        })
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    if far.1 <= tol {
        return vec![a, b];
    }
    let mut left = simplified(&pts[..=far.0], tol);
    left.pop();
    left.extend(simplified(&pts[far.0..], tol));
    left
}

/// How new wire is shown with a kit: its wire splines, its masts.
#[derive(Clone, Copy)]
pub struct Use<'a> {
    pub kit: &'a KitDef,
    pub wires: bool,
    pub masts: bool,
}

/// The wire splines and masts of a new span, their IDCodes kept in it.
fn lay_span(doc: &mut Document, hist: &mut History, roots: &Roots, span: &mut Span, road: Option<RoadLane>, u: Use) {
    let k = u.kit;
    match road {
        Some(r) => {
            let Some(sp) = doc.spline(r.tile, r.id).cloned() else { return };
            if u.wires {
                if let Ok(file) = wire_spline(&roots.content, k, r.x) {
                    let id = doc.new_id();
                    // (a copy of the road piece: the wire follows its bends, gradients and cant)
                    let w = MapSpline { file, id, prev_id: 0, next_id: 0, mirror: false, terrain_align: None, terrain_align_flag: false, rules: Vec::new(), ..sp.clone() };
                    span.laid.push(put_spline(doc, hist, w, tile_origin(r.tile) + DVec2::new(sp.pos[0], sp.pos[1])));
                }
            }
            if u.masts && k.step >= 3.0 {
                let Some(file) = object(roots, k, Piece::Mast) else { return };
                let side = if r.x < 0.0 { -1.0 } else { 1.0 };
                // (the mast stands so its arm, `reach` long, ends over the wire)
                let foot_x = r.x + side * k.reach;
                let curve = omsi_geometry::SplineCurve::from_map(&sp, tile_origin(r.tile));
                let mut s = k.step * 0.5;
                while s < sp.length {
                    let foot = curve.offset_point(s, foot_x, 0.0);
                    let over = curve.offset_point(s, r.x, 0.0);
                    let foot = DVec3::new(foot.x, foot.y, doc.ground(foot.truncate()));
                    // (its arm, toward its -y, reaches over the wire)
                    let heading = crate::roads::heading_of(-(over - foot).truncate());
                    span.laid.push(put_object(doc, hist, &file, foot, heading));
                    s += k.step;
                }
            }
        }
        None if u.wires => {
            let Ok(file) = wire_spline(&roots.content, k, 0.0) else { return };
            let pts = simplified(&span.points, 0.05);
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let flat = (b - a).truncate().length();
                if flat < 0.05 {
                    continue;
                }
                let t = tile_of(a.truncate());
                let local = a.truncate() - tile_origin(t);
                let grad = (b.z - a.z) / flat * 100.0;
                let id = doc.new_id();
                let s = MapSpline { file: file.clone(), id, pos: [local.x, local.y, a.z - k.height], heading: crate::roads::heading_of((b - a).truncate()), length: flat, grad_start: grad, grad_end: grad, ..Default::default() };
                span.laid.push(put_spline(doc, hist, s, a.truncate()));
            }
        }
        None => {}
    }
}

/// Take away objects and splines by IDCode (what a span had laid).
fn remove(doc: &mut Document, hist: &mut History, ids: &[i64]) {
    for id in ids {
        if let Some((k, _)) = doc.find_spline(*id) {
            hist.touch_tile(doc, k);
            doc.delete_spline(k, *id);
            continue;
        }
        let k = doc.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == *id)).map(|(k, _)| *k);
        if let Some(k) = k {
            hist.touch_tile(doc, k);
            doc.delete_object(k, *id);
        }
    }
}

/// The frogs and insulators put again where the wire has them now, from the wire's kit.
fn put_marks(doc: &mut Document, hist: &mut History, roots: Option<&Roots>) {
    let keys: Vec<_> = doc.tiles.iter().filter(|(_, t)| t.tile.objects.iter().any(|o| is_mark(&o.file))).map(|(k, _)| *k).collect();
    for k in keys {
        hist.touch_tile(doc, k);
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.objects.retain(|o| !is_mark(&o.file));
        t.touch_content();
    }
    let (Some(roots), Some(w)) = (roots, doc.traffic.trolley.clone()) else { return };
    let Some(kit) = w.kit.as_deref().and_then(|id| load(roots, &doc.dir, id)) else { return };
    let js = crate::trolley::joints(&w);
    // (the way the wire runs through a joint: along its first span there)
    let heading = |j: &crate::trolley::Joint| {
        let (i, far) = j.ends[0];
        let p = &w.spans[i].points;
        let d = if far { p[p.len() - 1] - p[p.len() - 2] } else { p[1] - p[0] };
        crate::roads::heading_of(d.truncate())
    };
    let frog_file = object(roots, &kit, Piece::Frog);
    let ins_file = object(roots, &kit, Piece::Insulator);
    for j in js.iter().filter(|j| j.ends.len() == 3) {
        if let Some(f) = &frog_file {
            put_object(doc, hist, f, j.pos, heading(j));
        }
    }
    for (p, _, _) in crate::trolley::section_breaks(&w, &js) {
        if let (Some(f), Some(j)) = (&ins_file, js.iter().find(|j| j.pos.distance(p) < 0.01)) {
            put_object(doc, hist, f, p, heading(j));
        }
    }
}

/// The map's wire made `after` in one undo step, with what shows it: the new spans (their
/// places in `after`, and the road lane each runs over) get their wire splines and masts from
/// the kit, what spans no longer there had laid goes, the frogs and insulators stand where the
/// wire's are.
pub fn apply(ed: &mut Editor, roots: Option<&Roots>, mut after: Option<Wire>, new: Vec<(usize, Option<RoadLane>)>, u: Option<Use>) {
    let Some(doc) = ed.doc.as_mut() else { return };
    let hist = &mut ed.history;
    hist.begin(tr("Trolleybus wire", "Контактная сеть"));
    hist.touch_traffic(doc);
    hist.touch_global(doc);
    let kept: std::collections::HashSet<i64> = after.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).collect();
    let gone: Vec<i64> = doc.traffic.trolley.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).filter(|id| !kept.contains(id)).collect();
    remove(doc, hist, &gone);
    if let (Some(w), Some(roots), Some(u)) = (after.as_mut(), roots, u) {
        for (i, road) in new {
            if let Some(span) = w.spans.get_mut(i) {
                lay_span(doc, hist, roots, span, road, u);
            }
        }
        omsi_cfg::content_changed();
    }
    doc.traffic.trolley = after;
    doc.traffic.touch(crate::traffic::File::Trolley);
    put_marks(doc, hist, roots);
    hist.commit();
}

/// What is on the map from an older state of a kit made from it as it is now: every wire piece
/// and mast of the kit's earlier files takes the new ones (in place: a changed reach or step
/// shows on wire laid from now on). How many were changed.
pub fn renew(ed: &mut Editor, roots: &Roots, kit: &KitDef) -> usize {
    let Some(doc) = ed.doc.as_mut() else { return 0 };
    // (the kit's files of any state: its name, then a stamp of 8 hex digits)
    let stamp = kit.stamp();
    let base = stamp[..stamp.len() - 9].to_ascii_lowercase();
    let mine = |file: &str| {
        let f = file.to_ascii_lowercase();
        let name = if f.ends_with(".sli") { f.rsplit('\\').next() } else { f.rsplit('\\').nth(1) }.unwrap_or("");
        is_kit(&f) && name.strip_prefix(&format!("{base}_")).is_some_and(|rest| rest.len() >= 8 && rest[..8].chars().all(|c| c.is_ascii_hexdigit()))
    };
    let mast = object(roots, kit, Piece::Mast);
    let mut n = 0;
    ed.history.begin(tr("Renew the wire from its kit", "Обновить провод по комплекту"));
    let keys: Vec<_> = doc.tiles.keys().copied().collect();
    for k in keys {
        let t = &doc.tiles[&k];
        let touches = t.tile.splines.iter().any(|s| s.file.ends_with(".sli") && mine(&s.file)) || t.tile.objects.iter().any(|o| mine(&o.file) && !is_mark(&o.file));
        if !touches {
            continue;
        }
        ed.history.touch_tile(doc, k);
        let t = doc.tiles.get_mut(&k).unwrap();
        for s in t.tile.splines.iter_mut().filter(|s| mine(&s.file)) {
            // (the wire's offset is kept in its file name)
            let x = s.file.rsplit("_x").next().and_then(|v| v.trim_end_matches(".sli").parse::<f64>().ok()).unwrap_or(0.0) / 100.0;
            if let Ok(f) = wire_spline(&roots.content, kit, x) {
                if f != s.file {
                    s.file = f;
                    n += 1;
                }
            }
        }
        for o in t.tile.objects.iter_mut().filter(|o| mine(&o.file) && !is_mark(&o.file)) {
            if let Some(f) = &mast {
                if *f != o.file {
                    o.file = f.clone();
                    n += 1;
                }
            }
        }
        t.touch_content();
    }
    put_marks(doc, &mut ed.history, Some(roots));
    ed.history.commit();
    omsi_cfg::content_changed();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> KitDef {
        let mut k = KitDef { name: "Test net".into(), ..KitDef::default() };
        k.mesh_mut(Piece::Mast).push(PartDef { pos: [0.0, 0.0, 3.0], ..PartDef::new(Shape::Cylinder) });
        k.mesh_mut(Piece::Mast).push(PartDef { pos: [0.0, -1.25, 5.8], rot: [90.0, 0.0, 0.0], size: [0.04, 2.5, 0.0], ..PartDef::new(Shape::Cylinder) });
        k.mesh_mut(Piece::Frog).push(PartDef::new(Shape::Box));
        k
    }

    #[test]
    fn a_kit_reads_back_as_it_was_written() {
        let mut k = kit();
        k.mesh_mut(Piece::Insulator).push(PartDef { file: "Sceneryobjects\\x\\y.sco".into(), ..PartDef::new(Shape::Object) });
        let text = k.text();
        assert_eq!(KitDef::parse(&text), k, "{text}");
        assert_eq!(KitDef::parse(&text).text(), text);
        // (a kit is changed by hand: what it says is read)
        let hand = text.replace("[masts]\n# metres between masts, metres from a mast to its wire\n30\n2.5", "[masts]\n40 # wider\n3");
        let h = KitDef::parse(&hand);
        assert_eq!((h.step, h.reach), (40.0, 3.0));
    }

    #[test]
    fn a_line_keeps_only_its_corners() {
        let pts: Vec<DVec3> = [(0.0, 0.0), (5.0, 0.01), (10.0, 0.0), (10.0, 10.0)].iter().map(|p| DVec3::new(p.0, p.1, 6.0)).collect();
        assert_eq!(simplified(&pts, 0.05), vec![pts[0], pts[2], pts[3]]);
    }

    /// The wire spline is the pair openOMSI reads as a wire, where the kit says.
    #[test]
    fn the_wire_spline_reads_as_a_pair_of_wires() {
        let content = crate::doc::tests::scratch("trolley_kit").content;
        let rel = wire_spline(&content, &kit(), 1.6).unwrap();
        let def = omsi_scenery::sli::Spline::load(&omsi_cfg::resolve_path(&content, &rel)).unwrap();
        let (mid, spacing, h) = crate::trolley::wire_profile(&def).unwrap();
        assert!((mid - 1.6).abs() < 1e-3 && (spacing - 0.6).abs() < 1e-3 && (h - 5.594).abs() < 0.01, "{mid} {spacing} {h}");
        assert!(is_kit(&rel) && !is_mark(&rel));
        assert!(!is_kit("Sceneryobjects\\openMapper\\TrolleyTest\\junction_1.sco"));
    }

    #[test]
    fn kits_are_the_maps_or_every_maps() {
        let roots = crate::doc::tests::scratch("trolley_kits");
        let map = roots.content.join("maps").join("M");
        let r = KitRef { scope: Scope::Map, stem: stem_of("Test net") };
        save(&roots, &map, &r, &kit()).unwrap();
        assert_eq!(list(&roots, &map).len(), 1);
        let shared = move_to(&roots, &map, &r, Scope::Shared).unwrap();
        assert_eq!(shared.id(), "shared:Test_net");
        assert!(roots.content.join("openMapper/trolley/Test_net.cfg").is_file());
        assert_eq!(load(&roots, &map, &shared.id()), Some(kit()));
        assert_eq!(list(&roots, &roots.content.join("maps").join("Other"))[0].0, shared);
    }

    /// Laid over a road: the wire's copy of the road and masts beside it, the frog where three
    /// spans meet (the kit has no insulator: none), all gone again with one undo.
    #[test]
    fn the_wire_is_shown_from_its_kit_and_undone_in_one_step() {
        let roots = crate::doc::tests::scratch("trolley_kit_lay");
        let mut ed = Editor::default();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: "Splines\\road.sli".into(), id: 500, pos: [10.0, 50.0, 0.0], heading: 90.0, length: 100.0, ..Default::default() });
        let k = kit();
        let r = KitRef { scope: Scope::Map, stem: stem_of(&k.name) };
        save(&roots, &doc.dir, &r, &k).unwrap();
        ed.doc = Some(doc);
        let at = |x: f64, y: f64| DVec3::new(x, y, 5.6);
        let spans = vec![
            (Span { section: 1, points: vec![at(10.0, 51.6), at(110.0, 51.6)], ..Default::default() }, Some(RoadLane { tile: (0, 0), id: 500, x: 1.6, height: 5.6 })),
            (Span { section: 1, points: vec![at(110.0, 51.6), at(150.0, 51.6)], ..Default::default() }, None),
            (Span { section: 2, points: vec![at(110.0, 51.6), at(130.0, 80.0)], ..Default::default() }, None),
        ];
        let mut w = crate::trolley::or_new(None);
        w.kit = Some(r.id());
        let added = crate::trolley::add_spans(&mut w, spans);
        apply(&mut ed, Some(&roots), Some(w), added, Some(Use { kit: &k, wires: true, masts: true }));
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        let count = |what: &str| t.objects.iter().filter(|o| o.file.ends_with(what)).count();
        let wires = t.splines.iter().filter(|s| is_kit(&s.file)).count();
        assert_eq!((wires, count("\\mast.sco"), count("\\frog.sco"), count("\\insulator.sco")), (3, 3, 1, 0));
        let w = doc.traffic.trolley.as_ref().unwrap();
        assert_eq!(w.spans[0].laid.len(), 4);
        assert_eq!(crate::trolley::parse(&crate::trolley::text(w)).kit, w.kit);
        // a changed kit renews what is laid
        let mut k2 = k.clone();
        k2.colour = [200, 0, 0];
        let n = renew(&mut ed, &roots, &k2);
        assert_eq!(n, 6, "3 wires and 3 masts");
        ed.undo();
        ed.undo();
        let doc = ed.doc.as_ref().unwrap();
        assert!(doc.traffic.trolley.is_none());
        assert!(!doc.tiles[&(0, 0)].tile.objects.iter().any(|o| is_kit(&o.file)) && !doc.tiles[&(0, 0)].tile.splines.iter().any(|s| is_kit(&s.file)));
    }
}
