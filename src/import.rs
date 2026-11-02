//! Bringing new things in, by dropping them on the window or with the Import button:
//!
//! * a mod (a folder or a `.zip` laid out like OMSI 2: `Sceneryobjects`, `Splines`,
//!   `Texture`, ...) is merged into openOMSI's content folder;
//! * a folder of `.sco` or `.sli` files becomes a folder of `Sceneryobjects` / `Splines`;
//! * a 3D model (`.obj`, `.gltf` / `.glb`, `.fbx`, `.x`, `.o3d`) becomes a new scenery object: the model goes into
//!   `model\`, its textures and their PBR maps (`_nn`, `_orm`, ...) into `texture\`, and a
//!   `.sco` is written for it. A Wavefront `.obj` (what Blender and most tools export) is
//!   turned into a DirectX `.x` that OMSI 2 and openOMSI both read.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::i18n::tr;

pub struct Report {
    /// Files copied.
    #[allow(dead_code)]
    pub copied: usize,
    pub objects: Vec<String>,
    pub message: String,
}

const CONTENT_FOLDERS: [&str; 6] = ["sceneryobjects", "splines", "texture", "maps", "vehicles", "fonts"];

fn gltf_uri_path(base: &Path, uri: &str) -> anyhow::Result<Option<PathBuf>> {
    if uri.starts_with("data:") {
        return Ok(None);
    }
    if let Some(path) = uri.strip_prefix("file://").or_else(|| uri.strip_prefix("file:")) {
        return Ok(Some(PathBuf::from(path)));
    }
    if uri.contains(':') {
        return Ok(None);
    }
    let uri = uri.as_bytes();
    let mut path = Vec::with_capacity(uri.len());
    let mut i = 0;
    while i < uri.len() {
        if uri[i] == b'%' {
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            let hi = uri.get(i + 1).and_then(|b| hex(*b));
            let lo = uri.get(i + 2).and_then(|b| hex(*b));
            let (Some(hi), Some(lo)) = (hi, lo) else {
                anyhow::bail!("glTF contains an invalid percent-encoded path");
            };
            path.push((hi << 4) | lo);
            i += 3;
        } else {
            path.push(uri[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(path)?;
    Ok(Some(base.join(path)))
}

fn include_gltf_sources(path: &Path, provenance: &mut crate::provenance::Source) -> anyhow::Result<()> {
    let gltf = gltf::Gltf::open(path)?;
    let base = path.parent().unwrap_or(Path::new("."));
    for buffer in gltf.document.buffers() {
        if let gltf::buffer::Source::Uri(uri) = buffer.source() {
            if let Some(source) = gltf_uri_path(base, uri)? {
                provenance.include(&source)?;
            }
        }
    }
    for image in gltf.document.images() {
        if let gltf::image::Source::Uri { uri, .. } = image.source() {
            if let Some(source) = gltf_uri_path(base, uri)? {
                provenance.include(&source)?;
            }
        }
    }
    Ok(())
}

fn include_obj_sources(path: &Path, provenance: &mut crate::provenance::Source) -> anyhow::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    for line in BufReader::new(std::fs::File::open(path)?).lines() {
        let line = line?;
        let line = line.trim_start();
        if line.split_whitespace().next() == Some("mtllib") {
            let source = dir.join(line[6..].trim());
            if source.is_file() {
                provenance.include(&source)?;
            }
        }
    }
    Ok(())
}

fn include_fbx_sources(path: &Path, provenance: &mut crate::provenance::Source) -> anyhow::Result<()> {
    let scene = ufbx::load_file(
        &path.to_string_lossy(),
        ufbx::LoadOpts { load_external_files: false, ..Default::default() },
    )
    .map_err(|e| anyhow::anyhow!("FBX: {}", e.description.as_ref() as &str))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut used = std::collections::HashSet::new();
    for node in scene.nodes.iter() {
        let Some(mesh) = node.mesh.as_ref() else { continue };
        for (index, _) in mesh.faces.iter().enumerate() {
            let material = mesh.face_material.get(index).copied().unwrap_or(0) as usize;
            if let Some(material) = node.materials.get(material).or_else(|| mesh.materials.get(material)) {
                used.insert((**material).element.element_id);
            }
        }
    }
    for material in scene.materials.iter().filter(|material| used.contains(&material.element.element_id)) {
        for texture in [
            &material.pbr.base_color.texture,
            &material.pbr.normal_map.texture,
            &material.pbr.roughness.texture,
            &material.pbr.metalness.texture,
            &material.pbr.ambient_occlusion.texture,
            &material.fbx.diffuse_color.texture,
            &material.fbx.normal_map.texture,
        ]
        .into_iter()
        .flatten()
        {
            if !texture.content.is_empty() && image::load_from_memory(&texture.content).is_ok() {
                continue;
            }
            let files = [
                texture.absolute_filename.as_ref() as &str,
                texture.filename.as_ref(),
                texture.relative_filename.as_ref(),
            ];
            let file = files
                .iter()
                .filter(|f| !f.is_empty())
                .map(|f| Path::new(&f.replace('\\', "/")).to_path_buf())
                .flat_map(|p| [p.clone(), dir.join(&p), dir.join(p.file_name().unwrap_or_default())])
                .find(|p| p.is_file());
            if let Some(file) = file {
                provenance.include(&file)?;
            }
        }
    }
    Ok(())
}

/// Import whatever `path` is into the content folder.
pub fn import(path: &Path, content: &Path) -> anyhow::Result<Report> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if path.is_file() {
        return match ext.as_str() {
            "zip" => import_zip(path, content),
            "obj" | "x" | "o3d" | "gltf" | "glb" | "fbx" => import_model(path, content),
            e if crate::import_image::EXTENSIONS.contains(&e) => crate::import_image::import_image(path, content),
            "sco" | "sli" => {
                let dir = path.parent().ok_or_else(|| anyhow::anyhow!("no folder"))?;
                import_folder(dir, content)
            }
            _ => anyhow::bail!("{}", tr("openMapper cannot import this kind of file (use a folder, .zip, .obj, .gltf, .glb, .fbx, .x, .o3d or a picture)", "Этот тип файла не импортируется (подходят папка, .zip, .obj, .gltf, .glb, .fbx, .x, .o3d, картинка)")),
        };
    }
    import_folder(path, content)
}

/// A folder laid out like OMSI 2's: it has a `Sceneryobjects`, `Splines`, `Maps`, `Vehicles`
/// or `Fonts` folder (a `texture` folder alone is what an object's or model's own folder has).
fn is_content_root(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|rd| rd.flatten().any(|e| e.path().is_dir() && CONTENT_FOLDERS.iter().filter(|f| **f != "texture").any(|f| e.file_name().to_string_lossy().eq_ignore_ascii_case(f)))).unwrap_or(false)
}

/// The folder inside `dir` that is laid out like OMSI 2 (archives often wrap it).
fn find_content_root(dir: &Path, depth: usize) -> Option<PathBuf> {
    if is_content_root(dir) {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).find_map(|p| find_content_root(&p, depth - 1))
}

fn import_folder(dir: &Path, content: &Path) -> anyhow::Result<Report> {
    if let Some(root) = find_content_root(dir, 3) {
        let mut copied = 0;
        for e in std::fs::read_dir(&root)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && CONTENT_FOLDERS.contains(&name.to_lowercase().as_str()) {
                // into the content folder's own spelling of it
                let target = existing_child(content, &name).unwrap_or_else(|| content.join(&name));
                copied += copy_tree(&e.path(), &target)?;
            }
        }
        return Ok(Report { copied, objects: Vec::new(), message: format!("{} {copied}", tr("Mod installed, files:", "Мод установлен, файлов:")) });
    }
    let has = |ext: &str| walk_has(dir, ext);
    let name = dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Imported".into());
    if has("sco") {
        // an object's folder (its .sco, model and texture folders) or a folder of them
        let target = existing_child(content, "Sceneryobjects").unwrap_or_else(|| content.join("Sceneryobjects")).join(&name);
        let copied = copy_tree(dir, &target)?;
        let mut objects = Vec::new();
        walk_files(&target, &mut |p| {
            if p.extension().map(|e| e.eq_ignore_ascii_case("sco")).unwrap_or(false) {
                if let Ok(r) = p.strip_prefix(content) {
                    objects.push(r.to_string_lossy().replace('/', "\\"));
                }
            }
        });
        objects.sort();
        return Ok(Report { copied, message: format!("{} Sceneryobjects\\{name} ({})", tr("Objects added to", "Объекты добавлены в"), objects.len()), objects });
    }
    // models anywhere in it (not the textures' folders): each becomes an object
    let mut models = Vec::new();
    walk_files(dir, &mut |p| {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if MODEL_EXTENSIONS.contains(&ext.as_str()) {
            models.push(p.to_path_buf());
        }
    });
    models.sort();
    let mut objects = Vec::new();
    let mut copied = 0;
    let mut notes = Vec::new();
    if has("sli") {
        let target = existing_child(content, "Splines").unwrap_or_else(|| content.join("Splines")).join(&name);
        copied += copy_tree(dir, &target)?;
        notes.push(format!("{} Splines\\{name}", tr("Road types added to", "Типы дорог добавлены в")));
    }
    for p in &models {
        let r = import_model(p, content)?;
        copied += r.copied;
        objects.extend(r.objects);
    }
    if !objects.is_empty() {
        notes.push(format!("{} {}", tr("Models imported:", "Импортировано моделей:"), objects.len()));
    }
    if notes.is_empty() {
        anyhow::bail!("{}", tr("Nothing to import was found in this folder", "В этой папке нечего импортировать"));
    }
    Ok(Report { copied, objects, message: notes.join("; ") })
}

const MODEL_EXTENSIONS: [&str; 6] = ["obj", "x", "o3d", "gltf", "glb", "fbx"];

/// Every file below `dir`, the textures' folders left out.
fn walk_files(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if !p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("texture")).unwrap_or(false) {
                walk_files(&p, f);
            }
        } else {
            f(&p);
        }
    }
}

fn walk_has(dir: &Path, ext: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if walk_has(&p, ext) {
                return true;
            }
        } else if p.extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case(ext)).unwrap_or(false) {
            return true;
        }
    }
    false
}

fn existing_child(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

/// Copy a folder over (files that are there already are replaced).
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dst)?;
    let mut n = 0;
    for e in std::fs::read_dir(src)?.flatten() {
        let from = e.path();
        let name = e.file_name();
        // (a folder of another spelling at the destination is used)
        if from.is_dir() {
            let to = existing_child(dst, &name.to_string_lossy()).unwrap_or_else(|| dst.join(&name));
            n += copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, dst.join(&name))?;
            n += 1;
        }
    }
    Ok(n)
}

fn import_zip(path: &Path, content: &Path) -> anyhow::Result<Report> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mod".into());
    // (in a folder of the archive's name: loose files are imported under that name)
    let outer = std::env::temp_dir().join(format!("openmapper-import-{}", std::process::id()));
    let tmp = outer.join(&stem);
    let _ = std::fs::remove_dir_all(&outer);
    std::fs::create_dir_all(&tmp)?;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        // (an entry that would land outside the folder is skipped)
        let Some(rel) = f.enclosed_name() else { continue };
        let rel = PathBuf::from(rel.to_string_lossy().replace('\\', "/"));
        let out = tmp.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        std::fs::write(&out, buf)?;
    }
    // a single folder of objects or models inside: import that folder by its name
    let r = import_folder(&tmp, content).or_else(|e| {
        let only: Vec<PathBuf> = std::fs::read_dir(&tmp).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        if only.len() == 1 {
            import_folder(&only[0], content)
        } else {
            Err(e)
        }
    });
    let _ = std::fs::remove_dir_all(&outer);
    r
}

/// The PBR maps beside a texture (`bus.png` > `bus_nn.png`, `bus_orm.png`, ...).
fn pbr_siblings(tex: &Path) -> Vec<PathBuf> {
    let Some(dir) = tex.parent() else { return Vec::new() };
    let Some(stem) = tex.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else { return Vec::new() };
    const SUFFIXES: [&str; 20] = ["_nn", "_normal", "_nrm", "_nn_gl", "_normal_gl", "_nrm_gl", "_rr", "_rough", "_roughness", "_gg", "_gloss", "_glossiness", "_mm", "_metal", "_metallic", "_metalness", "_aa", "_ao", "_orm", "_mra"];
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            let s = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            s.starts_with(&stem) && SUFFIXES.iter().any(|suf| s == format!("{stem}{suf}")) || s == format!("{stem}_arm") || s == format!("{stem}_occlusion")
        })
        .collect()
}

/// A model file becomes an openMapper object of its own folder in
/// `Sceneryobjects\openMapper\Imported`: a part for each piece the file has (they can be
/// edited, separated into objects of their own or joined later), its materials with their
/// colours, textures and transparency. It stands on the ground: a model whose origin is in its
/// middle would otherwise sink half into it.
fn import_model(path: &Path, content: &Path) -> anyhow::Result<Report> {
    let mut provenance = crate::provenance::Source::capture(path)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "model".into());
    let safe = crate::editable::safe_name(&stem);
    let roots = crate::install::Roots { omsi: PathBuf::new(), content: content.to_path_buf() };
    let rel = crate::editable::new_rel(&roots, "Imported", &safe);
    let (dir, own) = crate::editable::folder_of(&roots, &rel);
    let tex_dir = dir.join("texture");
    std::fs::create_dir_all(&tex_dir)?;
    let src_dir = path.parent().unwrap_or(Path::new("."));
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut src = crate::editable::Source { name: stem.clone(), ..Default::default() };
    let mut copied = 1;
    if matches!(ext.as_str(), "obj" | "gltf" | "glb" | "fbx") {
        if ext == "obj" {
            include_obj_sources(path, &mut provenance)?;
        } else if ext == "fbx" {
            include_fbx_sources(path, &mut provenance)?;
        } else if ext == "gltf" || ext == "glb" {
            include_gltf_sources(path, &mut provenance)?;
        }
        let (mut m, mut tex_names) = if ext == "obj" {
            obj_to_model(path)?
        } else {
            let m = if ext == "fbx" { crate::models::load_fbx(path)? } else { crate::models::load_gltf(path)? };
            // (its textures, read out of the file or from beside it, go in as pngs)
            let names = crate::models::save_textures(&m, &own, &tex_dir)?;
            (m, names)
        };
        if ext == "obj" {
            // (a texture in a format OMSI does not read goes in as a png; the others are copied
            // whole, with their maps)
            let mut textures: Vec<String> = Vec::new();
            for t in tex_names.iter_mut().flatten() {
                let name = Path::new(&t.replace('\\', "/")).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let e = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let found = [src_dir.join(t.replace('\\', "/")), src_dir.join(&name)].into_iter().find(|p| p.is_file());
                if let Some(p) = &found { provenance.include(p)?; }
                if !name.is_empty() && !omsi_texture::EXTENSIONS.contains(&e.as_str()) {
                    if let Some(img) = crate::models::picture(None, found.as_deref()) {
                        let png = format!("{}.png", Path::new(&name).file_stem().unwrap().to_string_lossy());
                        img.save(tex_dir.join(&png))?;
                        *t = png;
                        continue;
                    }
                }
                *t = name.clone();
                textures.push(name);
            }
            copy_textures(&textures, src_dir, &tex_dir, &mut copied, &mut provenance)?;
            for (mat, name) in m.mats.iter_mut().zip(&tex_names) {
                if let (0, Some(n)) = (mat.alpha, name) {
                    mat.alpha = crate::models::picture(None, Some(&tex_dir.join(n))).map(|i| crate::models::alpha_of(&i)).unwrap_or(0);
                }
            }
        }
        src.model.materials = m
            .mats
            .iter()
            .enumerate()
            .map(|(i, mat)| {
                let tex = tex_names.get(i).cloned().flatten();
                // (a textured material shows its texture as it is: the game applies only its
                // transparency, and a glTF's colour factor is already in the picture)
                let color = if tex.is_some() { [1.0, 1.0, 1.0, mat.color[3]] } else { mat.color };
                openmapper_mesh::Material { name: tex.clone().unwrap_or_else(|| format!("Material{i}")), color, texture: tex, alpha: mat.alpha, ..Default::default() }
            })
            .collect();
        for (i, part) in m.parts.iter().enumerate().filter(|(_, p)| !p.faces.is_empty()) {
            let o3d = omsi_o3d::Mesh {
                vertices: part.verts.iter().map(|(p, n, uv)| omsi_o3d::Vertex { position: glam::Vec3::from(*p), normal: glam::Vec3::from(*n), uv: glam::Vec2::from(*uv) }).collect(),
                triangles: part.faces.iter().map(|(f, mat)| omsi_o3d::Triangle { indices: *f, material: *mat as u16 }).collect(),
                ..Default::default()
            };
            let base = if part.name.trim().is_empty() { format!("Part{i}") } else { part.name.trim().to_string() };
            let name = src.model.free_name(&base);
            src.model.parts.push(crate::editable::part_from_mesh(&o3d, &name, 0));
        }
    } else {
        let m = omsi_o3d::load_mesh(path)?;
        let tex: Vec<String> = m.materials.iter().map(|m| m.texture.clone()).filter(|t| !t.is_empty()).collect();
        copy_textures(&tex, src_dir, &tex_dir, &mut copied, &mut provenance)?;
        src.model.materials = m
            .materials
            .iter()
            .enumerate()
            .map(|(i, mat)| {
                let file = Path::new(&mat.texture.replace('\\', "/")).file_name().map(|f| f.to_string_lossy().to_string()).filter(|f| !f.is_empty());
                // (a texture with transparency gets alpha testing: leaves, fences, windows)
                let alpha = file.as_ref().and_then(|f| crate::models::picture(None, Some(&tex_dir.join(f)))).map(|i| crate::models::alpha_of(&i)).unwrap_or(0);
                openmapper_mesh::Material { name: file.clone().unwrap_or_else(|| format!("Material{i}")), color: mat.diffuse, texture: file, alpha, emissive: mat.emissive, specular: mat.specular, power: mat.specular_power, ..Default::default() }
            })
            .collect();
        src.model.parts.push(crate::editable::part_from_mesh(&m, &safe, 0));
    }
    if src.model.parts.iter().all(|p| p.mesh.is_empty()) {
        anyhow::bail!("{}", tr("The model has no triangles", "В модели нет треугольников"));
    }
    stand_on_ground(&mut src.model);
    crate::editable::merge_materials(&mut src.model);
    crate::editable::save(&roots, &rel, &src)?;
    provenance.record(content, &dir)?;
    Ok(Report { copied, objects: vec![rel], message: format!("{} {stem}", tr("Model imported:", "Модель импортирована:")) })
}

/// A model that reaches below its origin is lifted onto the ground, and one that lies far off
/// its origin is brought over it.
fn stand_on_ground(model: &mut openmapper_mesh::Model) {
    let mut lo = glam::Vec3::splat(f32::MAX);
    let mut hi = glam::Vec3::splat(f32::MIN);
    for p in model.parts.iter().flat_map(|p| p.mesh.positions.iter()) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    if lo.x > hi.x {
        return;
    }
    let mut shift = glam::Vec3::ZERO;
    if lo.z < 0.0 {
        shift.z = -lo.z;
    }
    let mid = (lo + hi) * 0.5;
    let half = (hi - lo).truncate().length() * 0.5;
    if mid.truncate().length() > half.max(1.0) {
        shift.x = -mid.x;
        shift.y = -mid.y;
    }
    if shift != glam::Vec3::ZERO {
        for p in &mut model.parts {
            p.mesh.transform(glam::Mat4::from_translation(shift));
        }
    }
}

/// Sets how a material of an imported object treats transparency (0 opaque, 1 cut out, 2
/// blended): the `[matl]` block of its texture and use number in the `n`-th mesh of the `.sco`
/// gets its `[matl_alpha]`, or loses it.
pub fn set_alpha(sco: &Path, mesh: usize, texture: &str, index: i32, alpha: u8) -> anyhow::Result<()> {
    let text = omsi_cfg::decode_text(&std::fs::read(sco)?);
    let mut lines: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
    let is = |l: &str, key: &str| l.trim().eq_ignore_ascii_case(key);
    // the n-th mesh's lines: from its [mesh] to the next one
    let starts: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| is(l, "[mesh]")).map(|(i, _)| i).collect();
    let from = *starts.get(mesh).ok_or_else(|| anyhow::anyhow!("no such mesh"))?;
    let to = starts.get(mesh + 1).copied().unwrap_or(lines.len());
    // its [matl] block: the line after [matl] names the texture, the next its use number
    let found = (from..to).find(|&i| is(&lines[i], "[matl]") && lines.get(i + 1).map(|t| t.trim().eq_ignore_ascii_case(texture)).unwrap_or(false) && lines.get(i + 2).and_then(|n| n.trim().parse::<i32>().ok()) == Some(index));
    match found {
        Some(i) => {
            // the block ends at the next [matl] or [mesh] or at the end
            let end = (i + 3..to).find(|&j| is(&lines[j], "[matl]") || is(&lines[j], "[matl_change]")).unwrap_or(to);
            let a = (i + 3..end).find(|&j| is(&lines[j], "[matl_alpha]"));
            match (a, alpha) {
                (Some(j), 0) => {
                    lines.drain(j..(j + 2).min(lines.len()));
                }
                (Some(j), v) => lines[j + 1] = v.to_string(),
                (None, 0) => {}
                (None, v) => {
                    lines.insert(i + 3, v.to_string());
                    lines.insert(i + 3, "[matl_alpha]".into());
                }
            }
        }
        None if alpha > 0 => {
            let mut at = to;
            while at > from + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            for (k, l) in ["", "[matl]", texture, &index.to_string(), "[matl_alpha]", &alpha.to_string()].iter().enumerate() {
                lines.insert(at + k, l.to_string());
            }
        }
        None => {}
    }
    std::fs::write(sco, omsi_cfg::encode_text(&(lines.join("\r\n") + "\r\n")))?;
    Ok(())
}

/// The textures a model names, from beside it or a texture folder next to it, with their
/// PBR maps, into the object's own texture folder.
fn copy_textures(
    names: &[String],
    src_dir: &Path,
    tex_dir: &Path,
    copied: &mut usize,
    provenance: &mut crate::provenance::Source,
) -> anyhow::Result<()> {
    let mut seen = std::collections::HashSet::new();
    for t in names {
        let name = Path::new(&t.replace('\\', "/")).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            continue;
        }
        let cands = [src_dir.join(t.replace('\\', "/")), src_dir.join(&name), src_dir.join("texture").join(&name), src_dir.join("textures").join(&name), src_dir.join("Texture").join(&name)];
        if let Some(src) = cands.iter().find(|p| p.is_file()) {
            provenance.include(src)?;
            std::fs::copy(src, tex_dir.join(&name))?;
            *copied += 1;
            for p in pbr_siblings(src) {
                provenance.include(&p)?;
                std::fs::copy(&p, tex_dir.join(p.file_name().unwrap()))?;
                *copied += 1;
            }
        }
    }
    Ok(())
}

/// A Wavefront `.obj` in OMSI's frame (x right, y up, z forward, left-handed; the `.obj` is
/// right-handed with y up), a piece for each of its objects and groups, and each material's
/// texture as the `.mtl` names it.
pub fn obj_to_model(path: &Path) -> anyhow::Result<(crate::models::Model, Vec<Option<String>>)> {
    use crate::models::{Mat, Model, Part};
    let text = std::fs::read_to_string(path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut v: Vec<[f32; 3]> = Vec::new();
    let mut vt: Vec<[f32; 2]> = Vec::new();
    let mut vn: Vec<[f32; 3]> = Vec::new();
    let mut mtl: HashMap<String, MtlMat> = HashMap::new();
    let mut mat_order: Vec<String> = Vec::new();
    let mut current = String::from("default");
    let mut model = Model::default();
    // (each piece has its own vertices: the first of a name takes the name, a second of the
    // same name goes on in the first)
    let mut index: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut part: Option<usize> = None;
    let new_piece = |model: &mut Model, name: &str, index: &mut HashMap<(i64, i64, i64), u32>| -> usize {
        index.clear();
        model.parts.push(Part { name: name.to_string(), ..Default::default() });
        model.parts.len() - 1
    };
    let fix = |i: i64, n: usize| -> i64 { if i < 0 { n as i64 + i } else { i - 1 } };
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let p: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if p.len() == 3 {
                    v.push([p[0], p[1], p[2]]);
                }
            }
            Some("vt") => {
                let p: Vec<f32> = it.take(2).filter_map(|x| x.parse().ok()).collect();
                vt.push([p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0)]);
            }
            Some("vn") => {
                let p: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if p.len() == 3 {
                    vn.push([p[0], p[1], p[2]]);
                }
            }
            Some("o") | Some("g") => {
                // (a group with no faces yet is reused for the next name)
                let name = it.collect::<Vec<_>>().join(" ");
                match part {
                    Some(i) if model.parts[i].faces.is_empty() => model.parts[i].name = name,
                    _ => part = Some(new_piece(&mut model, &name, &mut index)),
                }
            }
            Some("mtllib") => {
                let name = line.trim_start()[6..].trim();
                if let Ok(t) = std::fs::read_to_string(dir.join(name)) {
                    parse_mtl(&t, &mut mtl);
                }
            }
            Some("usemtl") => {
                current = it.collect::<Vec<_>>().join(" ");
            }
            Some("f") => {
                let mat = match mat_order.iter().position(|m| *m == current) {
                    Some(i) => i,
                    None => {
                        mat_order.push(current.clone());
                        mat_order.len() - 1
                    }
                };
                let pi = match part {
                    Some(i) => i,
                    None => {
                        let i = new_piece(&mut model, "model", &mut index);
                        part = Some(i);
                        i
                    }
                };
                let mut poly: Vec<u32> = Vec::new();
                for c in it {
                    let mut parts = c.split('/');
                    let vi = parts.next().and_then(|x| x.parse::<i64>().ok()).map(|i| fix(i, v.len())).unwrap_or(-1);
                    let ti = parts.next().and_then(|x| x.parse::<i64>().ok()).map(|i| fix(i, vt.len())).unwrap_or(-1);
                    let ni = parts.next().and_then(|x| x.parse::<i64>().ok()).map(|i| fix(i, vn.len())).unwrap_or(-1);
                    if vi < 0 || vi as usize >= v.len() {
                        continue;
                    }
                    let verts = &mut model.parts[pi].verts;
                    let id = *index.entry((vi, ti, ni)).or_insert_with(|| {
                        let p = v[vi as usize];
                        let n = if ni >= 0 { vn.get(ni as usize).copied().unwrap_or([0.0, 1.0, 0.0]) } else { [0.0, 1.0, 0.0] };
                        let t = if ti >= 0 { vt.get(ti as usize).copied().unwrap_or([0.0, 0.0]) } else { [0.0, 0.0] };
                        // right-handed y-up > OMSI: z turns round; v runs down
                        verts.push(([p[0], p[1], -p[2]], [n[0], n[1], -n[2]], [t[0], 1.0 - t[1]]));
                        (verts.len() - 1) as u32
                    });
                    poly.push(id);
                }
                for k in 1..poly.len().saturating_sub(1) {
                    // (the mirrored axis turns the winding round)
                    model.parts[pi].faces.push(([poly[0], poly[k + 1], poly[k]], mat));
                }
            }
            _ => {}
        }
    }
    model.parts.retain(|p| !p.faces.is_empty());
    if model.parts.is_empty() {
        anyhow::bail!("{}", tr("The .obj file has no faces", "В файле .obj нет полигонов"));
    }
    if mat_order.is_empty() {
        mat_order.push("default".into());
    }
    // flat normals where the file had none
    if vn.is_empty() {
        for p in &mut model.parts {
            let mut acc = vec![glam::Vec3::ZERO; p.verts.len()];
            for (f, _) in &p.faces {
                let a = glam::Vec3::from(p.verts[f[0] as usize].0);
                let b = glam::Vec3::from(p.verts[f[1] as usize].0);
                let c = glam::Vec3::from(p.verts[f[2] as usize].0);
                let n = (b - a).cross(c - a);
                for i in f {
                    acc[*i as usize] -= n;
                }
            }
            for (v, n) in p.verts.iter_mut().zip(acc) {
                v.1 = n.normalize_or_zero().to_array();
            }
        }
    }
    let mut textures = Vec::new();
    for m in &mat_order {
        let e = mtl.get(m).cloned().unwrap_or(MtlMat { kd: [0.8, 0.8, 0.8], tex: None, opacity: 1.0, mask: None });
        // (a map_d picture gives the transparency of the colour picture)
        let alpha = if e.opacity < 0.98 { 2 } else { 0 };
        model.mats.push(Mat { color: [e.kd[0], e.kd[1], e.kd[2], e.opacity.clamp(0.0, 1.0)], alpha, ..Default::default() });
        textures.push(e.tex.or(e.mask));
    }
    Ok((model, textures))
}

#[derive(Clone)]
struct MtlMat {
    kd: [f32; 3],
    tex: Option<String>,
    opacity: f32,
    /// `map_d`: used for the texture where no `map_Kd` is.
    mask: Option<String>,
}

fn parse_mtl(text: &str, out: &mut HashMap<String, MtlMat>) {
    let mut cur: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        let mut it = l.split_whitespace();
        let word = it.next();
        // (options like -bm come before the file name, which is last)
        let last = l.split_whitespace().last().unwrap_or("").to_string();
        let Some(e) = cur.as_ref().and_then(|c| out.get_mut(c)) else {
            if word == Some("newmtl") {
                let n = it.collect::<Vec<_>>().join(" ");
                out.insert(n.clone(), MtlMat { kd: [0.8; 3], tex: None, opacity: 1.0, mask: None });
                cur = Some(n);
            }
            continue;
        };
        match word {
            Some("newmtl") => {
                let n = it.collect::<Vec<_>>().join(" ");
                out.insert(n.clone(), MtlMat { kd: [0.8; 3], tex: None, opacity: 1.0, mask: None });
                cur = Some(n);
            }
            Some("Kd") => {
                let p: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if p.len() == 3 {
                    e.kd = [p[0], p[1], p[2]];
                }
            }
            Some("d") => e.opacity = it.next().and_then(|x| x.parse().ok()).unwrap_or(1.0),
            Some("Tr") => e.opacity = 1.0 - it.next().and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0),
            Some("map_Kd") => e.tex = Some(last),
            Some("map_d") => e.mask = Some(last),
            _ => {}
        }
    }
}

/// Materials from other tools' files, their pictures brought into `tex_dir` (named after
/// `stem`): every material of a Wavefront `.mtl` (or the `.mtl` an `.obj` names), of a glTF /
/// GLB / FBX, and one PBR material of the pictures chosen (by their names' endings: base
/// colour / albedo / diffuse, normal, roughness, metallic, ambient occlusion, opacity,
/// emissive).
pub fn materials_from(files: &[PathBuf], tex_dir: &Path, stem: &str) -> anyhow::Result<Vec<openmapper_mesh::Material>> {
    let mut out = Vec::new();
    let mut pictures = Vec::new();
    for f in files {
        let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "mtl" | "obj" => {
                let mtl_path = if ext == "obj" {
                    let text = std::fs::read_to_string(f)?;
                    let name = text.lines().find_map(|l| l.trim().strip_prefix("mtllib ").map(|n| n.trim().to_string()));
                    match name {
                        Some(n) => f.parent().unwrap_or(Path::new(".")).join(n),
                        None => continue,
                    }
                } else {
                    f.clone()
                };
                let mut mtl = HashMap::new();
                parse_mtl(&std::fs::read_to_string(&mtl_path)?, &mut mtl);
                let dir = mtl_path.parent().unwrap_or(Path::new("."));
                let mut names: Vec<&String> = mtl.keys().collect();
                names.sort();
                for name in names {
                    let m = &mtl[name];
                    let mut mat = openmapper_mesh::Material { name: name.clone(), color: [m.kd[0], m.kd[1], m.kd[2], m.opacity], ..Default::default() };
                    if let Some(t) = m.tex.as_ref().or(m.mask.as_ref()) {
                        let src = dir.join(t.replace('\\', "/"));
                        if let Ok(n) = crate::app::bring_picture(&src, tex_dir, &format!("{stem}_{}", crate::editable::safe_name(name))) {
                            mat.alpha = crate::models::picture(None, Some(&tex_dir.join(&n))).map(|i| crate::models::alpha_of(&i)).unwrap_or(0);
                            mat.texture = Some(n);
                            mat.color = [1.0, 1.0, 1.0, m.opacity];
                        }
                    }
                    if m.opacity < 0.99 && mat.alpha == 0 {
                        mat.alpha = 2;
                    }
                    out.push(mat);
                }
            }
            "gltf" | "glb" | "fbx" => {
                let m = if ext == "fbx" { crate::models::load_fbx(f)? } else { crate::models::load_gltf(f)? };
                let names = crate::models::save_textures(&m, &format!("{stem}_{}", crate::editable::safe_name(&f.file_stem().unwrap_or_default().to_string_lossy())), tex_dir)?;
                for (i, mat) in m.mats.iter().enumerate() {
                    let tex = names.get(i).cloned().flatten();
                    out.push(openmapper_mesh::Material { name: tex.clone().unwrap_or_else(|| format!("Material{i}")), color: if tex.is_some() { [1.0, 1.0, 1.0, mat.color[3]] } else { mat.color }, texture: tex, alpha: mat.alpha, ..Default::default() });
                }
            }
            _ => pictures.push(f.clone()),
        }
    }
    if !pictures.is_empty() {
        out.push(pbr_material(&pictures, tex_dir, stem)?);
    }
    Ok(out)
}

/// One material of a set of PBR pictures, sorted by the endings of their names; the
/// openOMSI maps (`_nn`, `_orm`) written beside its colour texture.
fn pbr_material(pictures: &[PathBuf], tex_dir: &Path, stem: &str) -> anyhow::Result<openmapper_mesh::Material> {
    let kind = |p: &Path| -> &'static str {
        let s = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let has = |words: &[&str]| words.iter().any(|w| s.ends_with(w) || s.contains(&format!("_{w}")) || s.contains(&format!("-{w}")) || s.contains(&format!(" {w}")));
        if has(&["normal", "nrm", "nor", "norm", "_n", "normalgl", "normal_gl", "normaldx"]) {
            "normal"
        } else if has(&["roughness", "rough", "rgh"]) {
            "rough"
        } else if has(&["metallic", "metalness", "metal", "met"]) {
            "metal"
        } else if has(&["ambientocclusion", "occlusion", "ao"]) {
            "ao"
        } else if has(&["opacity", "alpha", "mask"]) {
            "opacity"
        } else if has(&["emissive", "emission", "emit", "glow"]) {
            "emit"
        } else if has(&["height", "displacement", "disp", "bump"]) {
            "height"
        } else {
            "color"
        }
    };
    let find = |k: &str| pictures.iter().find(|p| kind(p) == k);
    let base_path = find("color").ok_or_else(|| anyhow::anyhow!("{}", tr("no base colour picture among them", "среди них нет картинки базового цвета")))?;
    let name = format!("{stem}_{}", crate::editable::safe_name(&base_path.file_stem().unwrap_or_default().to_string_lossy()));
    let mut color = crate::models::picture(None, Some(base_path)).ok_or_else(|| anyhow::anyhow!("{}: {}", tr("cannot read the picture", "картинка не читается"), base_path.display()))?;
    if let Some(op) = find("opacity").and_then(|p| crate::models::picture(None, Some(p))) {
        let op = image::imageops::resize(&op, color.width(), color.height(), image::imageops::FilterType::Triangle);
        for (c, o) in color.pixels_mut().zip(op.pixels()) {
            c[3] = o[0];
        }
    }
    let tex = format!("{name}.png");
    color.save(tex_dir.join(&tex))?;
    let mut mat = openmapper_mesh::Material { name: base_path.file_stem().unwrap_or_default().to_string_lossy().to_string(), color: [1.0; 4], texture: Some(tex), alpha: crate::models::alpha_of(&color), ..Default::default() };
    if let Some(n) = find("normal").and_then(|p| crate::models::picture(None, Some(p))) {
        // (named …_gl / …GL: OpenGL's green-up maps)
        let gl = find("normal").map(|p| p.to_string_lossy().to_lowercase().contains("gl")).unwrap_or(false);
        let file = format!("{name}_nn{}.png", if gl { "_gl" } else { "" });
        n.save(tex_dir.join(&file))?;
        mat.normal_map = Some(file);
    }
    let ao = find("ao").and_then(|p| crate::models::picture(None, Some(p)));
    let rough = find("rough").and_then(|p| crate::models::picture(None, Some(p)));
    let metal = find("metal").and_then(|p| crate::models::picture(None, Some(p)));
    if ao.is_some() || rough.is_some() || metal.is_some() {
        let orm = crate::models::pack_orm(ao.as_ref().map(|i| (i, 0)), (rough.as_ref().map(|i| (i, 0)), 1.0), (metal.as_ref().map(|i| (i, 0)), if metal.is_some() { 1.0 } else { 0.0 }));
        let file = format!("{name}_orm.png");
        orm.save(tex_dir.join(&file))?;
        mat.orm_map = Some(file);
    }
    if let Some(e) = find("emit").and_then(|p| crate::models::picture(None, Some(p))) {
        let file = format!("{name}_lm.png");
        e.save(tex_dir.join(&file))?;
        mat.light_map = Some(file);
    }
    Ok(mat)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_set_of_pbr_pictures_becomes_one_material_with_openomsis_maps() {
        let dir = std::env::temp_dir().join(format!("om-pbr-{}", std::process::id()));
        let tex = dir.join("tex");
        std::fs::create_dir_all(&tex).unwrap();
        for n in ["Bricks_BaseColor.png", "Bricks_Normal.png", "Bricks_Roughness.png", "Bricks_AO.png"] {
            image::RgbaImage::from_pixel(4, 4, image::Rgba([100, 120, 140, 255])).save(dir.join(n)).unwrap();
        }
        let files: Vec<std::path::PathBuf> = ["Bricks_BaseColor.png", "Bricks_Normal.png", "Bricks_Roughness.png", "Bricks_AO.png"].iter().map(|n| dir.join(n)).collect();
        let mats = super::materials_from(&files, &tex, "wall").unwrap();
        assert_eq!(mats.len(), 1);
        let m = &mats[0];
        let t = m.texture.clone().unwrap();
        assert!(tex.join(&t).is_file());
        let stem = t.trim_end_matches(".png");
        assert_eq!(m.normal_map.as_deref(), Some(format!("{stem}_nn.png").as_str()));
        assert_eq!(m.orm_map.as_deref(), Some(format!("{stem}_orm.png").as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_obj_becomes_pieces_openomsi_reads() {
        let dir = std::env::temp_dir().join(format!("om-obj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("box.mtl"), "newmtl red\nKd 1 0 0\nmap_Kd wall.png\n").unwrap();
        std::fs::write(dir.join("box.obj"), "mtllib box.mtl\no a\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl red\nf 1/1 2/2 3/3 4/4\no b\nf 1/1 2/2 3/3\n").unwrap();
        let (m, tex) = super::obj_to_model(&dir.join("box.obj")).unwrap();
        assert_eq!(tex, vec![Some("wall.png".to_string())]);
        assert_eq!(m.parts.len(), 2);
        assert_eq!(m.parts[0].name, "a");
        // (written as openMapper writes an imported part, and read back as openOMSI reads it)
        let p = &m.parts[0];
        let o3d = omsi_o3d::Mesh { vertices: p.verts.iter().map(|(a, n, uv)| omsi_o3d::Vertex { position: glam::Vec3::from(*a), normal: glam::Vec3::from(*n), uv: glam::Vec2::from(*uv) }).collect(), triangles: p.faces.iter().map(|(f, mat)| omsi_o3d::Triangle { indices: *f, material: *mat as u16 }).collect(), ..Default::default() };
        let part = crate::editable::part_from_mesh(&o3d, "a", 0);
        let mat = openmapper_mesh::Material { texture: tex[0].clone(), ..Default::default() };
        let (x, used) = openmapper_mesh::export::to_x(&part.mesh, &[mat]);
        assert_eq!(used, vec![0]);
        std::fs::write(dir.join("box.x"), &x).unwrap();
        let back = omsi_o3d::load_mesh(&dir.join("box.x")).unwrap();
        assert_eq!(back.vertices.len(), 4);
        assert_eq!(back.triangles.len(), 2);
        assert_eq!(back.materials[0].texture, "wall.png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_a_model_records_its_external_pbr_sibling_bytes() {
        let dir = std::env::temp_dir().join(format!("om-provenance-pbr-{}", std::process::id()));
        let content = dir.join("content");
        std::fs::create_dir_all(&content).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([80, 120, 160, 255])).save(dir.join("wall.png")).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 255, 255])).save(dir.join("wall_nn.png")).unwrap();
        std::fs::write(dir.join("wall.mtl"), "newmtl wall\nmap_Kd wall.png\n").unwrap();
        std::fs::write(
            dir.join("wall.obj"),
            "mtllib wall.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl wall\nf 1/1 2/2 3/3\n",
        )
        .unwrap();

        let report = super::import(&dir.join("wall.obj"), &content).unwrap();

        let sibling_hash = crate::provenance::file_hash(&dir.join("wall_nn.png")).unwrap();
        let material_hash = crate::provenance::file_hash(&dir.join("wall.mtl")).unwrap();
        let history = crate::provenance::History::load(&content).unwrap();
        assert!(history.entries.values().any(|sources| sources.contains(&sibling_hash)));
        assert!(history.entries.values().any(|sources| sources.contains(&material_hash)));
        assert!(!report.objects.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_a_gltf_records_its_external_buffer_and_texture_bytes() {
        let dir = std::env::temp_dir().join(format!("om-provenance-gltf-{}", std::process::id()));
        let content = dir.join("content");
        std::fs::create_dir_all(&content).unwrap();
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let uvs: [f32; 6] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let mut buffer: Vec<u8> = positions.iter().chain(uvs.iter()).flat_map(|f| f.to_le_bytes()).collect();
        buffer.extend([0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
        let buffer_path = dir.join("mesh data.bin");
        std::fs::write(&buffer_path, buffer).unwrap();
        let texture_path = dir.join("external.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([80, 120, 160, 255])).save(&texture_path).unwrap();
        let source = dir.join("external.gltf");
        std::fs::write(
            &source,
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],
            "textures":[{"source":0}],"images":[{"uri":"external.png"}],
            "buffers":[{"byteLength":66,"uri":"mesh%20data.bin"}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24},{"buffer":0,"byteOffset":60,"byteLength":6}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"},{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}]}"#,
        )
        .unwrap();

        super::import(&source, &content).unwrap();

        let buffer_hash = crate::provenance::file_hash(&buffer_path).unwrap();
        let texture_hash = crate::provenance::file_hash(&texture_path).unwrap();
        let history = crate::provenance::History::load(&content).unwrap();
        assert!(history.entries.values().any(|sources| sources.contains(&buffer_hash)));
        assert!(history.entries.values().any(|sources| sources.contains(&texture_hash)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_materials_alpha_is_set_and_taken_away_in_the_sco() {
        let dir = std::env::temp_dir().join(format!("om-alpha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sco = dir.join("a.sco");
        std::fs::write(&sco, omsi_cfg::encode_text("[mesh]\r\nfirst.x\r\n\r\n[mesh]\r\nsecond.x\r\n\r\n[matl]\r\nleaf.png\r\n0\r\n[matl_alpha]\r\n1\r\n\r\n")).unwrap();
        let read = || omsi_cfg::decode_text(&std::fs::read(&sco).unwrap());
        super::set_alpha(&sco, 1, "leaf.png", 0, 2).unwrap();
        assert!(read().contains("[matl_alpha]\r\n2"), "{}", read());
        super::set_alpha(&sco, 1, "leaf.png", 0, 0).unwrap();
        assert!(!read().contains("matl_alpha"), "{}", read());
        super::set_alpha(&sco, 0, "glass.png", 0, 2).unwrap();
        let t = read();
        assert!(t.find("glass.png").unwrap() < t.find("second.x").unwrap() && t.contains("[matl_alpha]\r\n2"), "{t}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A glb-like model with two nodes, one of them a see-through leaf texture: two meshes, and
    /// the leaf's `[matl_alpha]` in the `.sco`.
    #[test]
    fn a_model_keeps_its_pieces_and_the_alpha_of_its_materials() {
        let dir = std::env::temp_dir().join(format!("om-pieces-{}", std::process::id()));
        let content = dir.join("content");
        std::fs::create_dir_all(&content).unwrap();
        let mut leaf = image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 200, 20, 255]));
        leaf.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        leaf.save(dir.join("leaf.png")).unwrap();
        std::fs::write(dir.join("two.mtl"), "newmtl solid\nKd 1 1 1\nnewmtl leaf\nKd 1 1 1\nmap_Kd leaf.png\n").unwrap();
        std::fs::write(dir.join("two.obj"), "mtllib two.mtl\no wall\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl solid\nf 1/1 2/2 3/3\no leaves\nusemtl leaf\nf 1/1 2/2 3/3\n").unwrap();
        super::import(&dir.join("two.obj"), &content).unwrap();
        let folder = content.join("Sceneryobjects").join("openMapper").join("Imported").join("two");
        let sco = String::from_utf8_lossy(&std::fs::read(folder.join("two.sco")).unwrap()).replace('\0', "");
        assert_eq!(sco.matches("[mesh]").count(), 2, "{sco}");
        assert!(sco.contains("[matl]\r\nleaf.png\r\n0\r\n[matl_alpha]\r\n1"), "{sco}");
        assert!(folder.join("model").join("two_wall.x").is_file() && folder.join("model").join("two_leaves.x").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod probe {
    /// Not a check: imports the file named by PROBE_MODEL into PROBE_CONTENT and says what came out.
    #[test]
    #[ignore]
    fn probe_import() {
        let (Ok(m), Ok(c)) = (std::env::var("PROBE_MODEL"), std::env::var("PROBE_CONTENT")) else { return };
        let r = super::import(std::path::Path::new(&m), std::path::Path::new(&c)).unwrap();
        eprintln!("{} {:?}", r.message, r.objects);
        let roots = crate::install::Roots { omsi: Default::default(), content: c.into() };
        let src = crate::editable::load(&roots, &r.objects[0]).unwrap();
        for (i, p) in src.model.parts.iter().enumerate() {
            eprintln!("part {i} {:?} faces {} modifiers {}", p.name, p.mesh.faces.len(), p.modifiers.len());
        }
        for (i, m) in src.model.materials.iter().enumerate() {
            eprintln!("mat {i} {:?} color {:?} tex {:?} alpha {}", m.name, m.color, m.texture, m.alpha);
        }
    }
}

#[cfg(test)]
mod provenance_tests {
    use sha2::{Digest, Sha256};
    #[test]
    fn original_source_hash_is_retained_after_real_model_conversion() {
        let root = std::env::temp_dir().join(format!("om-source-provenance-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let content = root.join("content");
        std::fs::create_dir_all(&content).unwrap();
        let source = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let file = root.join("owned.obj");
        std::fs::write(&file, source).unwrap();
        let hash: String = Sha256::digest(source).iter().map(|b| format!("{b:02x}")).collect();
        let report = super::import(&file, &content).unwrap();
        assert_eq!(report.objects.len(), 1);
        let history: serde_json::Value = serde_json::from_slice(&std::fs::read(content.join(".openmapper-provenance.json")).unwrap()).unwrap();
        assert_eq!(history["format_version"], 1);
        assert!(history["entries"].as_object().unwrap().values().any(|v| v.as_array().unwrap().iter().any(|s| s == &hash)));
        assert_eq!(std::fs::read(file).unwrap(), source);
        std::fs::remove_dir_all(root).unwrap();
    }
}
