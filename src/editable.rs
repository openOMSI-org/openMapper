//! openMapper's own objects: scenery objects whose meshes can be edited in the view (vertices,
//! edges, faces), cut and joined with booleans, given modifiers, materials, UV maps, lamps and
//! signal lights. What they are made of is kept beside the `.sco` as `<name>.openmapper.json`;
//! the `.sco` and its `.x` meshes are written from it after every change, so OMSI 2 and
//! openOMSI read them like any other object.
//!
//! They live in `Sceneryobjects\openMapper\<map folder>\<name>\` of the content folder. Any
//! other object (from the installation, a mod, an import) becomes one by a copy of it.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DMat4, DVec3, Mat4, Vec2, Vec3};
use openmapper_mesh::{Face, Material, Mesh, Model, Part};
use serde::{Deserialize, Serialize};

use crate::i18n::tr;
use crate::install::Roots;

const SUFFIX: &str = ".openmapper.json";

/// When a light shines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum When {
    Always,
    /// At night (OMSI's `NightlightA`: street lamps).
    Night,
    /// While a variable of the object's script is on (a signal's lamp).
    Var(String),
}

impl When {
    fn variable(&self) -> String {
        match self {
            When::Always => "1".into(),
            When::Night => "NightlightA".into(),
            When::Var(v) => v.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    /// A glow seen where the lamp is (`[light_enh_2]`): a lamp's bulb, a lit sign, a signal.
    Glow,
    /// Light thrown on the ground round it at night (`[maplight]`): a street lamp's pool.
    Ground,
}

/// A lamp of an object, in the object's frame (x right, y forward, z up, metres).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub kind: LightKind,
    pub pos: Vec3,
    /// Which way it shines (a glow with a cone; the ground light ignores it).
    pub dir: Vec3,
    pub color: [u8; 3],
    /// How big the glow is (m).
    pub size: f32,
    /// How far the ground light reaches (m).
    pub range: f32,
    /// A cone (inner and outer angle, degrees) or all round.
    pub cone: Option<(f32, f32)>,
    pub when: When,
    /// A star of rays round the glow.
    pub star: bool,
    pub brightness: f32,
}

impl Default for Light {
    fn default() -> Light {
        Light { kind: LightKind::Glow, pos: Vec3::new(0.0, 0.0, 1.0), dir: Vec3::new(0.0, -1.0, 0.0), color: [255, 220, 160], size: 0.5, range: 6.0, cone: None, when: When::Night, star: false, brightness: 1.0 }
    }
}

/// A lamp of a signal head: its colour and the variable of the script that lights it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lamp {
    Red,
    Yellow,
    Green,
}

impl Lamp {
    pub fn var(self) -> &'static str {
        match self {
            Lamp::Red => "Red",
            Lamp::Yellow => "Yellow",
            Lamp::Green => "Green",
        }
    }

    /// The lamp a script variable names (the `[visible]` of a lens): the content spells it
    /// `Red` as often as `red`.
    pub fn of_var(var: &str) -> Option<Lamp> {
        match var.trim().to_ascii_lowercase().as_str() {
            "red" => Some(Lamp::Red),
            "yellow" => Some(Lamp::Yellow),
            "green" => Some(Lamp::Green),
            _ => None,
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Lamp::Red => [255, 30, 20],
            Lamp::Yellow => [255, 170, 0],
            Lamp::Green => [40, 255, 120],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lamp::Red => tr("Red", "Красный"),
            Lamp::Yellow => tr("Yellow", "Жёлтый"),
            Lamp::Green => tr("Green", "Зелёный"),
        }
    }
}

/// What makes an object a signal head that a crossing's light program drives (as OMSI 2's
/// `Ampel_Kfz_1`): the lamps' variables, which its script sets from the crossing's phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub lamps: Vec<Lamp>,
}

/// A part shown only while a variable has a value (`[visible]`): a lit lamp's lens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartVar {
    pub part: String,
    pub var: String,
    pub value: f32,
}

/// Everything an openMapper object is made of.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub model: Model,
    #[serde(default)]
    pub lights: Vec<Light>,
    #[serde(default)]
    pub signal: Option<Signal>,
    #[serde(default)]
    pub part_vars: Vec<PartVar>,
    /// Its height is above sea level, not above the ground under it.
    #[serde(default)]
    pub abs_height: bool,
    /// Buses and cars collide with it (its meshes are its collision mesh too).
    #[serde(default)]
    pub collision: bool,
    /// Lines of the `.sco` openMapper does not make itself, kept from the object it was
    /// made from (paths, helpers, scripts...).
    #[serde(default)]
    pub extra: String,
}

/// The folder an object's files are in, and its file stem.
pub fn folder_of(roots: &Roots, rel: &str) -> (PathBuf, String) {
    let path = omsi_cfg::resolve_path(&roots.content, rel);
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    (path.parent().map(Path::to_path_buf).unwrap_or_default(), stem)
}

fn json_of(roots: &Roots, rel: &str) -> PathBuf {
    let (dir, stem) = folder_of(roots, rel);
    dir.join(format!("{stem}{SUFFIX}"))
}

/// Whether `rel` is an openMapper object (its source is beside it).
pub fn is_editable(roots: &Roots, rel: &str) -> bool {
    json_of(roots, rel).is_file()
}

pub fn load(roots: &Roots, rel: &str) -> Option<Source> {
    let text = std::fs::read_to_string(json_of(roots, rel)).ok()?;
    serde_json::from_str(&text).map_err(|e| log::warn!("{rel}: {e}")).ok()
}

/// A file name of letters, digits, `_` and `-`.
pub fn safe_name(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if s.is_empty() {
        "Object".into()
    } else {
        s
    }
}

/// Where a new object called `name` goes on map `map` (a name not taken yet).
pub fn new_rel(roots: &Roots, map: &str, name: &str) -> String {
    let base = safe_name(name);
    let map = safe_name(map);
    let taken = |n: &str| omsi_cfg::resolve_path(&roots.content, &format!("Sceneryobjects\\openMapper\\{map}\\{n}")).exists();
    let n = openmapper_mesh::free_name(&base, taken).replace('.', "_");
    format!("Sceneryobjects\\openMapper\\{map}\\{n}\\{n}.sco")
}

/// Write the object: its source, a `.x` for each part (modifiers applied), the `.sco`.
pub fn save(roots: &Roots, rel: &str, src: &Source) -> anyhow::Result<()> {
    save_on(roots, rel, src, None)
}

/// [`save`], the modifiers that follow the ground given the ground under it where it stands
/// (its height under a point of its frame, 0 under its origin).
pub fn save_on(roots: &Roots, rel: &str, src: &Source, ground: Option<&dyn Fn(glam::Vec2) -> f32>) -> anyhow::Result<()> {
    let target = omsi_cfg::resolve_path(&roots.content, rel);
    if omsi_cfg::vfs::is_sealed(&target) {
        anyhow::bail!("publisher-protected objects cannot be edited or overwritten");
    }
    let (dir, stem) = folder_of(roots, rel);
    std::fs::create_dir_all(dir.join("model"))?;
    std::fs::create_dir_all(dir.join("texture"))?;
    std::fs::write(dir.join(format!("{stem}{SUFFIX}")), serde_json::to_string_pretty(src)?)?;
    let mut sco = String::new();
    let _ = write!(sco, "[friendlyname]\n{}\n\n[groups]\n1\nopenMapper\n\n", if src.name.trim().is_empty() { &stem } else { src.name.trim() });
    if src.abs_height {
        sco.push_str("[absheight]\n\n");
    }
    if src.signal.is_some() {
        sco.push_str("[trafficlight]\n\n");
    }
    // the old meshes go: a part renamed or removed must not leave its file behind
    if let Ok(rd) = std::fs::read_dir(dir.join("model")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with(&format!("{stem}_")) && n.ends_with(".x") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let evaluated = src.model.evaluated(ground);
    let mut first = true;
    let mut collision = Mesh::default();
    for (i, mesh) in &evaluated {
        if mesh.is_empty() {
            continue;
        }
        let part = &src.model.parts[*i];
        let file = format!("{stem}_{}.x", safe_name(&part.name));
        let (x, used) = openmapper_mesh::export::to_x(mesh, &src.model.materials);
        std::fs::write(dir.join("model").join(&file), x)?;
        let _ = write!(sco, "[mesh]\n{file}\n\n");
        if let Some(v) = src.part_vars.iter().find(|v| v.part == part.name) {
            let _ = write!(sco, "[visible]\n{}\n{}\n\n", v.var, v.value);
        }
        // a material's settings: by its texture and the how-many-th use of it in the mesh
        let mut seen: HashMap<String, i32> = HashMap::new();
        for &m in &used {
            let Some(mat) = src.model.materials.get(m as usize) else { continue };
            let Some(tex) = &mat.texture else { continue };
            let n = seen.entry(tex.to_lowercase()).or_insert(-1);
            *n += 1;
            let mut block = String::new();
            if mat.alpha > 0 {
                let _ = write!(block, "[matl_alpha]\n{}\n", mat.alpha);
            }
            if let Some(nm) = &mat.night_map {
                let _ = write!(block, "[matl_nightmap]\n{nm}\n");
            }
            if let Some(lm) = &mat.light_map {
                let _ = write!(block, "[matl_lightmap]\n{lm}\n\n");
            }
            if !block.is_empty() {
                let _ = write!(sco, "[matl]\n{tex}\n{n}\n{block}\n");
            }
        }
        // (the lamps' glows belong to the first mesh that is always shown)
        if first && !src.part_vars.iter().any(|v| v.part == part.name) {
            first = false;
            for l in src.lights.iter().filter(|l| l.kind == LightKind::Glow) {
                write_glow(&mut sco, l);
            }
        }
        if src.collision {
            collision.append(mesh, 0);
        }
    }
    if first {
        for l in src.lights.iter().filter(|l| l.kind == LightKind::Glow) {
            write_glow(&mut sco, l);
        }
    }
    for l in src.lights.iter().filter(|l| l.kind == LightKind::Ground) {
        let c = l.color.map(|c| c as f32 / 255.0);
        let _ = write!(sco, "[maplight]\n{}\n{}\n{}\n{:.3}\n{:.3}\n{:.3}\n{}\n\n", l.pos.x, l.pos.y, l.pos.z, c[0], c[1], c[2], l.range);
    }
    if src.collision && !collision.is_empty() {
        let file = format!("{stem}_collision.x");
        let (x, _) = openmapper_mesh::export::to_x(&collision, &src.model.materials);
        std::fs::write(dir.join("model").join(&file), x)?;
        let _ = write!(sco, "[collision_mesh]\n{file}\n\n");
    }
    if let Some(sig) = &src.signal {
        write_signal_script(&dir, sig)?;
        sco.push_str("[script]\n1\nscript\\signal.osc\n\n[varnamelist]\n1\nscript\\signal_varlist.txt\n\n");
    }
    if !src.extra.trim().is_empty() {
        sco.push_str(src.extra.trim_end());
        sco.push('\n');
    }
    std::fs::write(dir.join(format!("{stem}.sco")), omsi_cfg::encode_text(&sco.replace("\r\n", "\n").replace('\n', "\r\n")))?;
    crate::hot::own_change();
    Ok(())
}

fn write_glow(sco: &mut String, l: &Light) {
    let d = l.dir.normalize_or(Vec3::NEG_Y);
    // (an up at right angles to the way it shines)
    let up = if d.z.abs() > 0.9 { Vec3::Y } else { Vec3::Z };
    let (inner, outer) = l.cone.unwrap_or((180.0, 180.0));
    let bits = if l.star { 1 } else { 0 };
    let _ = write!(
        sco,
        "[light_enh_2]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n0\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n0.1\n\n",
        l.pos.x,
        l.pos.y,
        l.pos.z,
        d.x,
        d.y,
        d.z,
        up.x,
        up.y,
        up.z,
        if l.cone.is_none() { 1 } else { 0 },
        l.color[0],
        l.color[1],
        l.color[2],
        l.size,
        inner,
        outer,
        l.when.variable(),
        l.brightness,
        0.05,
        bits,
        if l.cone.is_some() { 1 } else { 0 }
    );
}

/// The script a signal head runs: its lamps lit from the phase of the crossing it is linked
/// to (0..2 red, 3..5 red and yellow, 6..8 green, 9..11 yellow, as OMSI 2's own heads).
fn write_signal_script(dir: &Path, sig: &Signal) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir.join("script"))?;
    let mut osc = String::from("' A signal head made in openMapper: its lamps follow the phase\n' of the crossing it is linked to.\n\n{frame}\n\t(L.L.TrafficLightPhase) s0\n");
    let mut vars = String::new();
    for lamp in [Lamp::Red, Lamp::Yellow, Lamp::Green] {
        if !sig.lamps.contains(&lamp) {
            continue;
        }
        let rule = match lamp {
            Lamp::Red => "l0 0 >= l0 5 <= &&",
            Lamp::Yellow => "l0 3 >= l0 5 <= && l0 9 >= l0 11 <= && ||",
            Lamp::Green => "l0 6 >= l0 8 <= &&",
        };
        let _ = write!(osc, "\n\t{rule} (S.L.{})\n", lamp.var());
        let _ = writeln!(vars, "{}", lamp.var());
    }
    osc.push_str("{end}\n");
    std::fs::write(dir.join("script").join("signal.osc"), osc.replace('\n', "\r\n"))?;
    std::fs::write(dir.join("script").join("signal_varlist.txt"), vars.replace('\n', "\r\n"))?;
    Ok(())
}

// ------------------------------------------------------------------------- from other objects

/// From the map's frame to a mesh file's (Direct3D's: y up, z forward) and back: y and z
/// change places.
fn swap(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

/// A mesh file read into a part: triangles welded into one surface, faces turned to agree
/// with the file's normals, smooth where its normals are.
pub fn part_from_mesh(m: &omsi_o3d::Mesh, name: &str, material_offset: u32) -> Part {
    let mut mesh = Mesh::default();
    let pos: Vec<Vec3> = m.vertices.iter().map(|v| swap(v.position)).collect();
    let map = openmapper_mesh::weld_map(&pos, 1e-5);
    let mut first_of = vec![u32::MAX; pos.len()];
    for (i, p) in pos.iter().enumerate() {
        let root = map[i] as usize;
        if first_of[root] == u32::MAX {
            first_of[root] = mesh.add_vertex(*p);
        }
    }
    for t in &m.triangles {
        let idx = t.indices.map(|i| i as usize);
        if idx.iter().any(|&i| i >= pos.len()) {
            continue;
        }
        let verts: Vec<u32> = idx.iter().map(|&i| first_of[map[i] as usize]).collect();
        if verts[0] == verts[1] || verts[1] == verts[2] || verts[0] == verts[2] {
            continue;
        }
        let uvs: Vec<Vec2> = idx.iter().map(|&i| m.vertices[i].uv).collect();
        let mut f = Face::new(verts, uvs, t.material as u32 + material_offset);
        // (the file's corners may wind either way: its normals say which side is out)
        let n_file: Vec3 = idx.iter().map(|&i| swap(m.vertices[i].normal)).sum::<Vec3>();
        let p: Vec<Vec3> = f.verts.iter().map(|&v| mesh.positions[v as usize]).collect();
        let n_face = (p[1] - p[0]).cross(p[2] - p[0]);
        if n_face.dot(n_file) < 0.0 {
            f.reverse();
        }
        let nf = n_face.normalize_or_zero();
        f.smooth = idx.iter().any(|&i| swap(m.vertices[i].normal).normalize_or_zero().dot(nf).abs() < 0.9995);
        mesh.faces.push(f);
    }
    Part::new(name, mesh)
}

/// Materials of a mesh file as openMapper's: colours, textures, and the `.sco`'s settings
/// for them (transparency, night maps) by texture and use.
fn materials_of(m: &omsi_o3d::Mesh, defs: &[omsi_model::MaterialDef]) -> Vec<Material> {
    let mut seen: HashMap<String, i32> = HashMap::new();
    m.materials
        .iter()
        .enumerate()
        .map(|(i, mat)| {
            let key = mat.texture.to_lowercase();
            let n = seen.entry(key.clone()).or_insert(-1);
            *n += 1;
            let def = defs.iter().find(|d| d.texture.to_lowercase() == key && d.index == *n);
            let tex = Path::new(&mat.texture.replace('\\', "/")).file_name().map(|f| f.to_string_lossy().to_string()).filter(|t| !t.is_empty());
            Material {
                name: tex.clone().map(|t| t.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(t)).unwrap_or_else(|| format!("Material{i}")),
                color: mat.diffuse,
                texture: tex,
                alpha: def.map(|d| d.alpha.clamp(0, 2) as u8).unwrap_or(0),
                emissive: mat.emissive,
                specular: mat.specular,
                power: mat.specular_power,
                normal_map: None,
                orm_map: None,
                night_map: def.and_then(|d| d.nightmap.clone()),
                light_map: def.and_then(|d| d.lightmap.as_ref().map(|l| l.0.clone())),
            }
        })
        .collect()
}

/// The `.sco` of an object found where openOMSI looks (content folder, then installation).
pub fn sco_path(roots: &Roots, rel: &str) -> Option<PathBuf> {
    [omsi_cfg::resolve_path(&roots.content, rel), omsi_cfg::resolve_path(&roots.omsi, rel)].into_iter().find(|p| p.is_file())
}

/// Any object as an openMapper object's source: its meshes (the nearest level of detail),
/// their materials, and the rest of its `.sco` kept as it was. The files it uses (textures,
/// scripts) are copied into `dir`, where the new object will be.
pub fn source_of(roots: &Roots, rel: &str, dir: &Path) -> anyhow::Result<Source> {
    let path = sco_path(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: {}", tr("file not found", "файл не найден")))?;
    if omsi_cfg::vfs::is_sealed(&path) {
        anyhow::bail!("publisher-protected objects cannot be copied for editing");
    }
    if let Some(src) = load(roots, rel) {
        let (from, _) = folder_of(roots, rel);
        copy_tree_missing(&from.join("texture"), &dir.join("texture"));
        copy_tree_missing(&from.join("script"), &dir.join("script"));
        return Ok(src);
    }
    let sco = omsi_scenery::sco::SceneryObject::load(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
    if sco.model_file.is_some() {
        anyhow::bail!("{}", tr("this object takes its model from a model file and cannot be edited here", "модель этого объекта лежит в отдельном файле модели: её нельзя редактировать здесь"));
    }
    let sco_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let model = &sco.model;
    let defs: Vec<&omsi_model::MeshDef> = if model.lods.is_empty() { model.meshes.iter().collect() } else { model.lod_meshes(0).iter().collect() };
    let tex_dirs = crate::assets::texture_dirs(&roots.omsi, &roots.content, &sco_dir);
    let refs: Vec<&Path> = tex_dirs.iter().map(PathBuf::as_path).collect();
    let name = if sco.friendly_name.trim().is_empty() { path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { sco.friendly_name.trim().to_string() };
    let mut out = Source { name, abs_height: sco.absolute_height(), ..Default::default() };
    let mut textures = Vec::new();
    for md in defs {
        let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&sco_dir, "model"), &md.file);
        let mesh_path = if omsi_cfg::vfs::is_file(&in_model) { in_model } else { omsi_cfg::resolve_path(&sco_dir, &md.file) };
        if omsi_cfg::vfs::is_sealed(&mesh_path) {
            anyhow::bail!("publisher-protected model files cannot be copied for editing");
        }
        let Ok(m) = omsi_o3d::load_mesh(&mesh_path) else { continue };
        let mats = materials_of(&m, &md.materials);
        for mat in &mats {
            for t in [&mat.texture, &mat.night_map, &mat.light_map].into_iter().flatten() {
                if omsi_texture::find_texture(t, &refs).is_some_and(|p| omsi_cfg::vfs::is_sealed(&p)) {
                    anyhow::bail!("publisher-protected textures cannot be copied for editing");
                }
                textures.push(t.clone());
            }
        }
        let offset = out.model.materials.len() as u32;
        out.model.materials.extend(mats);
        let stem = Path::new(&md.file.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Part".into());
        let name = out.model.free_name(&stem);
        let part = part_from_mesh(&m, &name, offset);
        if let Some((var, value)) = &md.visible {
            out.part_vars.push(PartVar { part: name.clone(), var: var.clone(), value: *value });
        }
        out.lights.extend(md.light_enh_2.iter().map(|l| {
            let raw = l.color.iter().all(|c| *c <= 1.0);
            let color = l.color.map(|c| (if raw { c * 255.0 } else { c }).round().clamp(0.0, 255.0) as u8);
            let when = match l.variable.trim() {
                v if v.eq_ignore_ascii_case("NightlightA") => When::Night,
                v if v.parse::<f32>().is_ok() => When::Always,
                v => When::Var(v.to_string()),
            };
            Light { kind: LightKind::Glow, pos: Vec3::from(l.pos), dir: Vec3::from(l.dir), color, size: l.size, range: 6.0, cone: (!l.omni).then_some((l.cone_inner, l.cone_outer)), when, star: false, brightness: l.factor }
        }));
        out.model.parts.push(part);
    }
    out.extra = kept_lines(&omsi_cfg::decode_text(&std::fs::read(&path)?), &sco_dir, dir);
    if out.model.parts.is_empty() {
        anyhow::bail!("{}", tr("the object has no mesh to edit", "у объекта нет меша для редактирования"));
    }
    if !textures.is_empty() {
        std::fs::create_dir_all(dir.join("texture"))?;
        for texture in &textures {
            copy_texture(texture, &refs, &dir.join("texture"));
        }
    }
    merge_materials(&mut out.model);
    Ok(out)
}

/// The `.sco`'s lines that are not about its meshes and their materials (those openMapper
/// writes itself): lights, paths, helpers, scripts. Files they name (scripts, collision
/// meshes) are copied into `dir`.
fn kept_lines(text: &str, from: &Path, dir: &Path) -> String {
    const MESH_TAGS: [&str; 31] = [
        "mesh", "matl", "matl_alpha", "matl_change", "matl_item", "matl_nightmap", "matl_lightmap", "matl_allcolor", "matl_freetex", "matl_envmap", "matl_envmap_mask", "matl_bumpmap", "matl_transmap", "matl_noz", "matl_nozwrite", "matl_nozcheck", "matl_zbias", "matl_texadress_clamp", "matl_texadress_border", "matl_texadress_mirror", "matl_texadress_mirroronce", "visible", "lod", "viewpoint", "mesh_ident", "smoothskin", "friendlyname", "groups", "absheight", "matl_raindropmap", "illumination_interior",
    ];
    const FILES: [&str; 4] = ["script", "varnamelist", "stringvarnamelist", "constfile"];
    let lines = omsi_cfg::split_lines(text);
    let mut out = String::new();
    let mut keep = false;
    let mut files: Option<&str> = None;
    let mut count: Option<usize> = None;
    for line in &lines {
        if let Some(kw) = omsi_cfg::keyword_of(line) {
            let k = kw.to_lowercase();
            // (lights and textures of a mesh: dropped with it, the new meshes have their own)
            keep = !MESH_TAGS.contains(&k.as_str()) && !k.starts_with("matl") && k != "light_enh" && k != "light_enh_2" && k != "collision_mesh" && k != "crossing_heightdeformation";
            files = FILES.iter().copied().find(|f| *f == k);
            count = None;
            if keep {
                let _ = writeln!(out, "{line}");
            }
            continue;
        }
        if !keep {
            continue;
        }
        if files.is_some() {
            if count.is_none() {
                count = line.trim().parse().ok();
            } else if !line.trim().is_empty() {
                let rel = line.trim();
                let src = omsi_cfg::resolve_path(from, rel);
                let dst = omsi_cfg::resolve_path(dir, rel);
                if src.is_file() && !dst.exists() {
                    if let Some(p) = dst.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    let _ = std::fs::copy(&src, &dst);
                }
            }
        }
        let _ = writeln!(out, "{line}");
    }
    out
}

fn copy_texture(name: &str, dirs: &[&Path], to: &Path) {
    let Some(found) = omsi_texture::find_texture(name, dirs) else { return };
    let Some(file) = found.file_name() else { return };
    let dst = to.join(file);
    if !dst.exists() {
        let _ = std::fs::copy(&found, &dst);
    }
    // and its PBR maps beside it
    let stem = found.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let (Some(parent), false) = (found.parent(), stem.is_empty()) {
        if let Ok(rd) = std::fs::read_dir(parent) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let s = Path::new(&n).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                if s != stem && s.starts_with(&format!("{stem}_")) && ["_nn", "_nn_gl", "_orm", "_lm", "_night"].iter().any(|suf| s == format!("{stem}{suf}")) && !to.join(&n).exists() {
                    let _ = std::fs::copy(e.path(), to.join(&n));
                }
            }
        }
    }
}

fn copy_tree_missing(from: &Path, to: &Path) {
    let Ok(rd) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for e in rd.flatten() {
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree_missing(&e.path(), &dst);
        } else if !dst.exists() {
            let _ = std::fs::copy(e.path(), &dst);
        }
    }
}

/// Materials that are the same made one (meshes read one by one repeat them).
pub fn merge_materials(model: &mut Model) {
    let mut keep: Vec<Material> = Vec::new();
    let mut map = Vec::with_capacity(model.materials.len());
    for m in &model.materials {
        let same = keep.iter().position(|k| k.color == m.color && k.texture == m.texture && k.alpha == m.alpha && k.emissive == m.emissive && k.night_map == m.night_map && k.light_map == m.light_map);
        match same {
            Some(i) => map.push(i as u32),
            None => {
                map.push(keep.len() as u32);
                keep.push(m.clone());
            }
        }
    }
    for p in &mut model.parts {
        for f in &mut p.mesh.faces {
            f.material = map.get(f.material as usize).copied().unwrap_or(0);
        }
    }
    model.materials = keep;
}

// ------------------------------------------------------------------------------ placing

/// The matrix of a map object standing at `world`, turned by its heading, pitch and bank.
pub fn placement(world: DVec3, rot: [f64; 3]) -> DMat4 {
    DMat4::from_rotation_translation(crate::gizmo::object_quat(rot), world)
}

/// Bring `other`'s parts into the frame of an object placed by `into` (theirs placed by
/// `from`), their materials added to `into_src`'s; the names of the parts added.
pub fn merge_into(into_src: &mut Source, into: DMat4, other: &Source, from: DMat4) -> Vec<String> {
    let m: Mat4 = (into.inverse() * from).as_mat4();
    let offset = into_src.model.materials.len() as u32;
    into_src.model.materials.extend(other.model.materials.iter().cloned());
    let mut names = Vec::new();
    for p in &other.model.parts {
        let mut part = p.clone();
        part.mesh.transform(m);
        for f in &mut part.mesh.faces {
            f.material += offset;
        }
        part.name = into_src.model.free_name(&p.name);
        part.primitive = None;
        names.push(part.name.clone());
        if let Some(v) = other.part_vars.iter().find(|v| v.part == p.name) {
            into_src.part_vars.push(PartVar { part: part.name.clone(), ..v.clone() });
        }
        into_src.model.parts.push(part);
    }
    for l in &other.lights {
        let mut l = l.clone();
        l.pos = m.transform_point3(l.pos);
        l.dir = m.transform_vector3(l.dir);
        into_src.lights.push(l);
    }
    merge_materials(&mut into_src.model);
    names
}

/// A copy of an openMapper object scaled along its own axes by `f` (its meshes, lamps and
/// the rest of its files), beside it; the copy's file.
pub fn scaled_copy(roots: &Roots, rel: &str, f: DVec3) -> anyhow::Result<String> {
    let mut src = load(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: {}", tr("cannot read the object", "объект не читается")))?;
    let k = f.as_vec3();
    for p in &mut src.model.parts {
        p.mesh.transform(Mat4::from_scale(k));
        p.primitive = None;
    }
    for l in &mut src.lights {
        l.pos *= k;
        l.size *= (k.x * k.y * k.z).cbrt();
        l.range *= (k.x * k.y).sqrt();
    }
    let (from, stem) = folder_of(roots, rel);
    let map = from.parent().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Shared".into());
    let new = new_rel(roots, &map, &stem);
    let (dir, _) = folder_of(roots, &new);
    copy_tree_missing(&from.join("texture"), &dir.join("texture"));
    copy_tree_missing(&from.join("script"), &dir.join("script"));
    save(roots, &new, &src)?;
    Ok(new)
}

/// A new object of one primitive shape, with one plain material.
pub fn primitive_source(p: openmapper_mesh::primitives::Primitive) -> Source {
    let mut mesh = p.build();
    if p.round() {
        for f in &mut mesh.faces {
            f.smooth = true;
        }
    }
    let mut part = Part::new(p.name(), mesh);
    part.primitive = Some(p);
    Source { name: p.name().into(), model: Model { parts: vec![part], materials: vec![Material::colored("Material", [0.8, 0.8, 0.8, 1.0])] }, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(tag: &str) -> (PathBuf, Roots) {
        let dir = std::env::temp_dir().join(format!("om-editable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("content")).unwrap();
        std::fs::create_dir_all(dir.join("omsi")).unwrap();
        (dir.clone(), Roots { omsi: dir.join("omsi"), content: dir.join("content") })
    }


    #[test]
    fn a_primitive_is_written_as_an_object_openomsi_reads_and_read_back() {
        let (dir, roots) = roots("prim");
        let mut src = primitive_source(openmapper_mesh::primitives::Primitive::Cube { size: Vec3::new(2.0, 1.0, 3.0) });
        src.lights.push(Light::default());
        src.lights.push(Light { kind: LightKind::Ground, ..Default::default() });
        let rel = new_rel(&roots, "My Map", "Cube");
        assert_eq!(rel, "Sceneryobjects\\openMapper\\My_Map\\Cube\\Cube.sco");
        save(&roots, &rel, &src).unwrap();
        assert!(is_editable(&roots, &rel));
        assert_eq!(load(&roots, &rel).unwrap(), src);
        let path = omsi_cfg::resolve_path(&roots.content, &rel);
        let sco = omsi_scenery::sco::SceneryObject::load(&path).unwrap();
        assert_eq!(sco.model.meshes.len(), 1);
        assert_eq!(sco.model.meshes[0].light_enh_2.len(), 1);
        let m = omsi_o3d::load_mesh(&path.parent().unwrap().join("model").join(&sco.model.meshes[0].file)).unwrap();
        assert_eq!(m.triangles.len(), 12);
        // (a box 3 m high stands 3 m up in the file's frame: y up)
        let hi = m.vertices.iter().map(|v| v.position.y).fold(f32::MIN, f32::max);
        assert!((hi - 3.0).abs() < 1e-5);
        // the next one gets a name of its own
        assert_eq!(new_rel(&roots, "My Map", "Cube"), "Sceneryobjects\\openMapper\\My_Map\\Cube_001\\Cube_001.sco");
        // and any object becomes one: read back from its own files
        let back = source_of(&roots, &rel, &dir.join("copy")).unwrap();
        assert_eq!(back.model.parts.len(), 1);
        let t = back.model.parts[0].mesh.triangles(30.0);
        let vol: f32 = t.indices.chunks(3).map(|c| t.positions[c[0] as usize].dot(t.positions[c[1] as usize].cross(t.positions[c[2] as usize]))).sum::<f32>() / 6.0;
        assert!((vol - 6.0).abs() < 1e-3, "{vol}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn protected_virtual_mesh_cannot_be_copied_by_editing_or_overwritten_by_save() {
        use std::sync::Arc;

        let (dir, roots) = roots("protected-source");
        let package = dir.join("publisher-fixture");
        std::fs::create_dir_all(&package).unwrap();
        let object = omsi_cfg::encode_text("[friendlyname]\r\nOwned fixture\r\n[mesh]\r\nmodel/sign.x\r\n");
        let cube = openmapper_mesh::primitives::Primitive::Cube { size: Vec3::ONE }.build();
        let mesh = openmapper_mesh::export::to_x(&cube, &[Material::default()]).0.into_bytes();
        let key = Arc::new(omsi_cfg::vfs::SealKey::new(&omsi_cfg::vfs::random_key().unwrap()));
        let files = [
            ("Sceneryobjects/Protected/sign.sco", object.clone()),
            ("Sceneryobjects/Fixture/model/sign.x", mesh),
        ].into_iter().enumerate().map(|(i, (rel, plain))| {
            let aad = format!("fixture asset {i}").into_bytes();
            let blob = package.join(format!("{i}.sealed"));
            std::fs::write(&blob, omsi_cfg::vfs::seal(&key, &aad, &plain).unwrap()).unwrap();
            omsi_cfg::vfs::SealedFile { rel: rel.into(), blob, aad, size: plain.len() as u64 }
        }).collect();
        let mount = roots.content.join("Packs/fixture.oopp/.mount");
        omsi_cfg::vfs::mount_sealed(&mount, key, files);
        omsi_cfg::add_content_root(roots.content.clone());
        omsi_cfg::add_content_root(mount.clone());

        let ordinary = "Sceneryobjects\\Fixture\\sign.sco";
        let ordinary_file = roots.content.join(ordinary.replace('\\', "/"));
        std::fs::create_dir_all(ordinary_file.parent().unwrap()).unwrap();
        std::fs::write(&ordinary_file, &object).unwrap();
        let mesh_path = omsi_cfg::resolve_path(&ordinary_file.parent().unwrap().join("model"), "sign.x");
        assert!(omsi_cfg::vfs::is_sealed(&mesh_path));
        assert!(omsi_o3d::load_mesh(&mesh_path).is_ok());

        let output = dir.join("converted");
        let source_refused = source_of(&roots, ordinary, &output).is_err();
        let copied_to_edit_folder = output.exists();
        let protected = "Sceneryobjects\\Protected\\sign.sco";
        let save_refused = save(&roots, protected, &Source::default()).is_err();
        let wrote_into_pack_folder = mount.join("Sceneryobjects/Protected/sign.openmapper.json").exists();
        omsi_cfg::vfs::unmount_sealed(&mount);
        omsi_cfg::remove_content_root(&mount);
        omsi_cfg::remove_content_root(&roots.content);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(source_refused, "the editor copied a mesh from a protected pack");
        assert!(!copied_to_edit_folder, "the editor left a copy of a protected object on disk");
        assert!(save_refused, "the editor overwrote an object in a protected pack");
        assert!(!wrote_into_pack_folder, "the editor wrote plaintext into the protected package directory");
    }

    #[test]
    fn a_mesh_file_comes_back_as_one_closed_surface_facing_out() {
        let cube = openmapper_mesh::primitives::Primitive::Cube { size: Vec3::ONE }.build();
        let (x, _) = openmapper_mesh::export::to_x(&cube, &[Material::default()]);
        let m = omsi_o3d::xfile::parse_x(x.as_bytes()).unwrap();
        let p = part_from_mesh(&m, "Cube", 0);
        assert!(p.mesh.is_closed());
        assert_eq!(p.mesh.positions.len(), 8);
        let c = Vec3::new(0.0, 0.0, 0.5);
        for f in 0..p.mesh.faces.len() {
            assert!(p.mesh.face_normal(f).dot(p.mesh.face_center(f) - c) > 0.0);
        }
    }

    #[test]
    fn a_part_merged_into_another_object_lands_where_it_stood() {
        let mut a = primitive_source(openmapper_mesh::primitives::Primitive::Cube { size: Vec3::ONE });
        let b = primitive_source(openmapper_mesh::primitives::Primitive::Cube { size: Vec3::ONE });
        // b stands 10 m east of a, which is turned to face east (heading 90)
        let pa = placement(DVec3::new(0.0, 0.0, 0.0), [90.0, 0.0, 0.0]);
        let pb = placement(DVec3::new(10.0, 0.0, 0.0), [0.0, 0.0, 0.0]);
        merge_into(&mut a, pa, &b, pb);
        assert_eq!(a.model.parts.len(), 2);
        assert_eq!(a.model.parts[1].name, "Cube.001");
        assert_eq!(a.model.materials.len(), 1);
        let (lo, hi) = a.model.parts[1].mesh.bounds().unwrap();
        let c = (lo + hi) * 0.5;
        // east of a heading-90 object is straight ahead of it (its +y)
        assert!((c - Vec3::new(0.0, 10.0, 0.5)).length() < 1e-4, "{c}");
    }
}
