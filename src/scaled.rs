//! Objects at another size. OMSI 2's map says only where an object stands and how it is
//! turned, so an object scaled in the view becomes a copy of its type made bigger or smaller:
//! `name_scaled_1500_1500_2000.sco` (the factors in thousandths) beside the original (in the
//! content folder, so the original's textures are found as they are), its meshes, collision
//! mesh, lights and paths scaled. Copies used to be called `name~s1.5_1.5_2.sco`, which
//! OMSI 2 does not read: a map that has them gets copies of the new name when it is opened.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::DVec3;

use crate::i18n::tr;
use crate::install::Roots;

/// The type an object of `rel` is a scaled copy of, and by how much (the type itself, by
/// one, if it is none).
pub fn base_of(rel: &str) -> (String, DVec3) {
    let (dir, file) = match rel.rfind('\\') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    let stem = file.strip_suffix(".sco").or_else(|| file.strip_suffix(".SCO")).unwrap_or(file);
    if let Some((base, s)) = stem.rsplit_once(SCALED) {
        let f: Vec<f64> = s.split('_').filter_map(thousandths_of).collect();
        if f.len() == 3 {
            return (format!("{dir}{base}.sco"), DVec3::new(f[0], f[1], f[2]));
        }
    }
    if let Some((base, s)) = old_scale(stem) {
        return (format!("{dir}{base}.sco"), s);
    }
    (rel.to_string(), DVec3::ONE)
}

/// What comes between the original's name and the factors.
const SCALED: &str = "_scaled_";

/// The base and the scale of a copy of the old name (`lamp~s1.5_2_0.25`).
fn old_scale(stem: &str) -> Option<(&str, DVec3)> {
    let (base, s) = stem.rsplit_once("~s")?;
    let f: Vec<f64> = s.split('_').filter_map(|v| v.parse().ok()).collect();
    (f.len() == 3).then(|| (base, DVec3::new(f[0], f[1], f[2])))
}

/// A factor as the name writes it: thousandths, `m` for a negative one (a mirrored copy).
fn thousandths(v: f64) -> String {
    let n = (v * 1000.0).round() as i64;
    if n < 0 { format!("m{}", -n) } else { n.to_string() }
}

fn thousandths_of(s: &str) -> Option<f64> {
    let (sign, digits) = match s.strip_prefix('m') {
        Some(d) => (-1.0, d),
        None => (1.0, s),
    };
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| sign * digits.parse::<f64>().ok().unwrap_or(0.0) / 1000.0)
}

/// The factors as people read them (`1.5×2×0.25`).
fn shown(s: DVec3) -> String {
    let n = |v: f64| {
        let t = format!("{v:.3}");
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    format!("{}×{}×{}", n(s.x), n(s.y), n(s.z))
}

/// The copy's name: one dot, ASCII after the original's own name, as OMSI 2 reads it.
fn scaled_name(base_rel: &str, s: DVec3) -> String {
    let stem = base_rel.strip_suffix(".sco").unwrap_or(base_rel);
    format!("{stem}{SCALED}{}_{}_{}.sco", thousandths(s.x), thousandths(s.y), thousandths(s.z))
}

/// The type for an object of `rel` scaled by `factor` along its own axes (on top of any
/// scale it has): made in the content folder if it is not there yet.
pub fn variant(roots: &Roots, rel: &str, factor: DVec3) -> anyhow::Result<String> {
    let (base, had) = base_of(rel);
    let total = had * factor;
    if (total - DVec3::ONE).abs().max_element() < 1e-3 {
        return Ok(base);
    }
    let out_rel = scaled_name(&base, total);
    let out_path = omsi_cfg::resolve_path(&roots.content, &out_rel);
    if out_path.is_file() {
        return Ok(out_rel);
    }
    let src = [omsi_cfg::resolve_path(&roots.content, &base), omsi_cfg::resolve_path(&roots.omsi, &base)].into_iter().find(|p| p.is_file()).ok_or_else(|| anyhow::anyhow!("{base}: {}", tr("file not found", "файл не найден")))?;
    let src_dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let out_dir = out_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let text = omsi_cfg::decode_text(&std::fs::read(&src)?);
    let lines = omsi_cfg::split_lines(&text);
    let suffix = out_rel.rsplit('\\').next().and_then(|f| f.strip_suffix(".sco")).and_then(|f| f.rsplit_once(SCALED)).map(|(_, s)| s.to_string()).unwrap_or_default();
    let mut out = String::new();
    let avg_xy = (total.x * total.y).sqrt();
    let avg = (total.x * total.y * total.z).cbrt();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        let _ = writeln!(out, "{line}");
        i += 1;
        let kw = omsi_cfg::keyword_of(line).map(|k| k.to_lowercase());
        let Some(kw) = kw else { continue };
        // the lines after the keyword: which are scaled and how
        let plan: &[Scale] = match kw.as_str() {
            "model" => anyhow::bail!("{}", tr("this object takes its model from a model file: it cannot be scaled", "модель этого объекта лежит в отдельном файле модели: его нельзя масштабировать")),
            "mesh" | "collision_mesh" | "crossing_heightdeformation" => &[Scale::Mesh],
            "friendlyname" => &[Scale::Name],
            "light_enh" => &[Scale::X, Scale::Y, Scale::Z, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Avg],
            "light_enh_2" => &[Scale::X, Scale::Y, Scale::Z, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Avg],
            "maplight" => &[Scale::X, Scale::Y, Scale::Z, Scale::Keep, Scale::Keep, Scale::Keep, Scale::Avg],
            "spotlight" => &[Scale::X, Scale::Y, Scale::Z],
            "path" | "path_2" => &[Scale::X, Scale::Y, Scale::Z, Scale::Keep, Scale::AvgXy, Scale::AvgXy],
            _ => &[],
        };
        for s in plan {
            let Some(v) = lines.get(i) else { break };
            i += 1;
            let num = |k: f64| v.trim().parse::<f64>().map(|x| fmt_num(x * k)).unwrap_or_else(|_| v.clone());
            let new = match s {
                Scale::Keep => v.clone(),
                Scale::X => num(total.x),
                Scale::Y => num(total.y),
                Scale::Z => num(total.z),
                Scale::Avg => num(avg),
                Scale::AvgXy => num(avg_xy),
                Scale::Name => format!("{} (×{})", v.trim(), shown(total)),
                Scale::Mesh => {
                    let file = v.trim();
                    match scale_mesh(&src_dir, &out_dir, file, total, &suffix) {
                        Ok(f) => f,
                        Err(e) => {
                            log::warn!("{file}: {e}");
                            file.to_string()
                        }
                    }
                }
            };
            let _ = writeln!(out, "{new}");
        }
    }
    std::fs::create_dir_all(&out_dir)?;
    std::fs::write(&out_path, omsi_cfg::encode_text(&out.replace('\n', "\r\n")))?;
    crate::hot::own_change();
    Ok(out_rel)
}

/// The map's scaled copies of the old name (`lamp~s1.5_2_1.sco`, which OMSI 2 does not
/// read) replaced by copies of the new one, made where they are missing; the tiles that name
/// them are changed, to be saved. How many objects were given the new name.
pub fn renew_old_copies(roots: &Roots, doc: &mut crate::doc::Document) -> usize {
    let mut made: std::collections::HashMap<String, Option<String>> = Default::default();
    let mut renamed = 0;
    for t in doc.tiles.values_mut() {
        let mut changed = false;
        let files = t.tile.objects.iter_mut().chain(t.tile.attach_objects.iter_mut()).map(|o| &mut o.file).chain(t.tile.spline_attachments.iter_mut().map(|a| &mut a.file));
        for file in files {
            let (dir, name) = file.rsplit_once('\\').unwrap_or(("", file.as_str()));
            let stem = name.strip_suffix(".sco").or_else(|| name.strip_suffix(".SCO")).unwrap_or(name);
            let Some((base, s)) = old_scale(stem) else { continue };
            let base = if dir.is_empty() { format!("{base}.sco") } else { format!("{dir}\\{base}.sco") };
            let new = made.entry(file.clone()).or_insert_with(|| match variant(roots, &base, s) {
                Ok(n) => Some(n),
                Err(e) => {
                    log::warn!("{file}: no copy OMSI 2 reads could be made: {e}");
                    None
                }
            });
            if let Some(n) = new {
                *file = n.clone();
                renamed += 1;
                changed = true;
            }
        }
        if changed {
            t.touch_content();
        }
    }
    if renamed > 0 {
        log::info!("{renamed} scaled objects named as OMSI 2 reads them ({} types)", made.values().flatten().count());
    }
    renamed
}

#[derive(Clone, Copy)]
enum Scale {
    Keep,
    X,
    Y,
    Z,
    Avg,
    AvgXy,
    Name,
    Mesh,
}

fn fmt_num(v: f64) -> String {
    let t = format!("{v:.4}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-" {
        "0".into()
    } else {
        t.to_string()
    }
}

/// A mesh of the object's (`file`, as its `.sco` names it) scaled into a `.x` beside the
/// copy; the name to write in its place.
fn scale_mesh(src_dir: &Path, out_dir: &Path, file: &str, s: DVec3, suffix: &str) -> anyhow::Result<String> {
    let in_model = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(src_dir, "model"), file);
    let path: PathBuf = if omsi_cfg::vfs::is_file(&in_model) { in_model } else { omsi_cfg::resolve_path(src_dir, file) };
    let m = omsi_o3d::load_mesh(&path)?;
    let stem = crate::editable::safe_name(&Path::new(&file.replace('\\', "/")).file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "mesh".into()));
    let name = format!("{stem}{SCALED}{suffix}.x");
    let dir = out_dir.join("model");
    std::fs::create_dir_all(&dir)?;
    // (mesh files are in Direct3D's frame: y is up, z forward)
    let k = glam::Vec3::new(s.x as f32, s.z as f32, s.y as f32);
    std::fs::write(dir.join(&name), x_of(&m, k))?;
    Ok(name)
}

/// A mesh as a `.x` text file, every material kept in its slot, positions scaled by `k`.
pub fn x_of(m: &omsi_o3d::Mesh, k: glam::Vec3) -> String {
    let n = m.vertices.len();
    let t = m.triangles.len();
    let flip = k.x * k.y * k.z < 0.0;
    let mut x = String::from("xof 0302txt 0032\n\nMesh {\n");
    let _ = writeln!(x, " {n};");
    for (i, v) in m.vertices.iter().enumerate() {
        let p = v.position * k;
        let _ = writeln!(x, " {:.6};{:.6};{:.6};{}", p.x, p.y, p.z, if i + 1 == n { ";" } else { "," });
    }
    let tri = |tr: &omsi_o3d::Triangle| if flip { [tr.indices[0], tr.indices[2], tr.indices[1]] } else { tr.indices };
    let _ = writeln!(x, " {t};");
    for (i, tr) in m.triangles.iter().enumerate() {
        let f = tri(tr);
        let _ = writeln!(x, " 3;{},{},{};{}", f[0], f[1], f[2], if i + 1 == t { ";" } else { "," });
    }
    // (normals of a stretched mesh: by the inverse scale)
    let _ = writeln!(x, " MeshNormals {{\n  {n};");
    for (i, v) in m.vertices.iter().enumerate() {
        let nn = (v.normal / k).normalize_or_zero();
        let _ = writeln!(x, "  {:.6};{:.6};{:.6};{}", nn.x, nn.y, nn.z, if i + 1 == n { ";" } else { "," });
    }
    let _ = writeln!(x, "  {t};");
    for (i, tr) in m.triangles.iter().enumerate() {
        let f = tri(tr);
        let _ = writeln!(x, "  3;{},{},{};{}", f[0], f[1], f[2], if i + 1 == t { ";" } else { "," });
    }
    x.push_str(" }\n");
    let _ = writeln!(x, " MeshTextureCoords {{\n  {n};");
    for (i, v) in m.vertices.iter().enumerate() {
        let _ = writeln!(x, "  {:.6};{:.6};{}", v.uv.x, v.uv.y, if i + 1 == n { ";" } else { "," });
    }
    x.push_str(" }\n");
    let mats = m.materials.len().max(1);
    let _ = writeln!(x, " MeshMaterialList {{\n  {mats};\n  {t};");
    for (i, tr) in m.triangles.iter().enumerate() {
        let _ = writeln!(x, "  {}{}", (tr.material as usize).min(mats - 1), if i + 1 == t { ";" } else { "," });
    }
    let fallback = omsi_o3d::Material::default();
    for i in 0..mats {
        let mat = m.materials.get(i).unwrap_or(&fallback);
        let d = mat.diffuse;
        let _ = writeln!(
            x,
            "  Material {{\n   {:.4};{:.4};{:.4};{:.4};;\n   {:.4};\n   {:.4};{:.4};{:.4};;\n   {:.4};{:.4};{:.4};;",
            d[0], d[1], d[2], d[3], mat.specular_power, mat.specular[0], mat.specular[1], mat.specular[2], mat.emissive[0], mat.emissive[1], mat.emissive[2]
        );
        if !mat.texture.is_empty() {
            let _ = writeln!(x, "   TextureFilename {{\n    \"{}\";\n   }}", mat.texture.replace('\\', "\\\\"));
        }
        x.push_str("  }\n");
    }
    x.push_str(" }\n}\n");
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scaled_name_reads_back_as_its_base_and_scale() {
        let n = scaled_name("Sceneryobjects\\A\\lamp.sco", DVec3::new(1.5, 2.0, 0.25));
        assert_eq!(n, "Sceneryobjects\\A\\lamp_scaled_1500_2000_250.sco");
        assert!(crate::omsi_name::readable(&n), "OMSI 2 reads it: {n}");
        let (b, s) = base_of(&n);
        assert_eq!(b, "Sceneryobjects\\A\\lamp.sco");
        assert_eq!(s, DVec3::new(1.5, 2.0, 0.25));
        assert_eq!(base_of("Sceneryobjects\\A\\lamp.sco").1, DVec3::ONE);
        // (a mirrored copy)
        assert_eq!(base_of(&scaled_name("Sceneryobjects\\A\\lamp.sco", DVec3::new(-1.0, 1.0, 1.0))).1, DVec3::new(-1.0, 1.0, 1.0));
        // (a copy of the old name, which OMSI 2 did not read, is still known for one)
        assert_eq!(base_of("Sceneryobjects\\A\\comercio18~s1.876_1.436_1.876.sco"), ("Sceneryobjects\\A\\comercio18.sco".to_string(), DVec3::new(1.876, 1.436, 1.876)));
        // (an object whose own name says "scaled" is not taken for a copy)
        assert_eq!(base_of("Sceneryobjects\\A\\tree_scaled_big.sco").1, DVec3::ONE);
    }

    #[test]
    fn a_scaled_copy_scales_its_mesh_and_lights_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("om-scaled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let content = dir.join("content");
        let omsi = dir.join("omsi");
        let obj = omsi.join("Sceneryobjects").join("T");
        std::fs::create_dir_all(obj.join("model")).unwrap();
        let cube = openmapper_mesh::primitives::Primitive::Cube { size: glam::Vec3::ONE }.build();
        let (x, _) = openmapper_mesh::export::to_x(&cube, &[openmapper_mesh::Material::default()]);
        std::fs::write(obj.join("model").join("box.x"), x).unwrap();
        let sco = "[friendlyname]\nBox\n\n[mesh]\nbox.x\n\n[maplight]\n1\n2\n3\n1\n1\n1\n5\n";
        std::fs::write(obj.join("box.sco"), omsi_cfg::encode_text(&sco.replace('\n', "\r\n"))).unwrap();
        let roots = Roots { omsi: omsi.clone(), content: content.clone() };
        let rel = variant(&roots, "Sceneryobjects\\T\\box.sco", DVec3::new(2.0, 2.0, 2.0)).unwrap();
        assert_eq!(rel, "Sceneryobjects\\T\\box_scaled_2000_2000_2000.sco");
        let path = omsi_cfg::resolve_path(&content, &rel);
        let text = omsi_cfg::decode_text(&std::fs::read(&path).unwrap());
        assert!(text.contains("box_scaled_2000_2000_2000.x"), "{text}");
        assert!(text.contains("Box (×2×2×2)"), "{text}");
        assert!(text.contains("\r\n2\r\n4\r\n6\r\n"), "{text}");
        let m = omsi_o3d::load_mesh(&content.join("Sceneryobjects/T/model/box_scaled_2000_2000_2000.x")).unwrap();
        let hi = m.vertices.iter().map(|v| v.position.y).fold(f32::MIN, f32::max);
        assert!((hi - 2.0).abs() < 1e-4);
        // scaled again by a half: the original back
        assert_eq!(variant(&roots, &rel, DVec3::splat(0.5)).unwrap(), "Sceneryobjects\\T\\box.sco");

        // a map saved by an earlier openMapper names a copy OMSI 2 cannot read: opened, it
        // names one it can, made beside it
        let mut doc = crate::selection::tests::flat_map(1);
        let old = "Sceneryobjects\\T\\box~s1.5_1.5_1.5.sco";
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject { file: old.into(), id: 1, ..Default::default() });
        doc.tiles.get_mut(&(0, 0)).unwrap().dirty = false;
        assert_eq!(renew_old_copies(&roots, &mut doc), 1);
        let t = &doc.tiles[&(0, 0)];
        assert_eq!(t.tile.objects[0].file, "Sceneryobjects\\T\\box_scaled_1500_1500_1500.sco");
        assert!(crate::omsi_name::readable(&t.tile.objects[0].file));
        assert!(omsi_cfg::resolve_path(&content, &t.tile.objects[0].file).is_file(), "the copy is made");
        assert!(t.dirty, "the tile is to be saved with the new name");
        assert_eq!(renew_old_copies(&roots, &mut doc), 0, "once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
