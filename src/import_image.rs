//! A picture made into an object: a flat sign standing upright (a photo, a poster, a
//! drawing), or, if the picture has see-through parts, a thin standee cut to its outline.
//! The picture goes into the object's `texture` folder; a person turns it, sizes it and
//! places it as any object, and opens it in the edit mode to go on from there.

use std::path::Path;

use glam::{Vec2, Vec3};
use openmapper_mesh::{Face, Mesh};

use crate::i18n::tr;
use crate::import::Report;

/// Pictures the importer takes.
pub const EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "bmp", "tga", "tif"];

/// How tall the object stands when imported (m): sized after in the properties.
const HEIGHT: f32 = 3.0;
/// Cells along the longer side of the cut-out grid.
const GRID: u32 = 96;
const THICKNESS: f32 = 0.12;

/// The picture's solid cells (alpha above half at the cell's centre), `w` by `h`.
fn solid_cells(img: &image::RgbaImage, w: u32, h: u32) -> Vec<bool> {
    (0..w * h)
        .map(|k| {
            let (x, y) = (k % w, k / w);
            let px = ((x as f32 + 0.5) / w as f32 * img.width() as f32) as u32;
            let py = ((y as f32 + 0.5) / h as f32 * img.height() as f32) as u32;
            img.get_pixel(px.min(img.width() - 1), py.min(img.height() - 1))[3] > 127
        })
        .collect()
}

/// The upright sign: front and back, `width` by `height` metres, bottom edge on the ground.
pub fn sign(width: f32, height: f32) -> Mesh {
    let mut m = Mesh::default();
    let p: Vec<u32> = [(-0.5, 0.0), (0.5, 0.0), (0.5, 1.0), (-0.5, 1.0)].iter().map(|(x, z)| m.add_vertex(Vec3::new(x * width, 0.0, z * height))).collect();
    let uv = |u: f32, v: f32| Vec2::new(u, v);
    m.faces.push(Face::new(vec![p[0], p[1], p[2], p[3]], vec![uv(0.0, 1.0), uv(1.0, 1.0), uv(1.0, 0.0), uv(0.0, 0.0)], 0));
    m.faces.push(Face::new(vec![p[1], p[0], p[3], p[2]], vec![uv(0.0, 1.0), uv(1.0, 1.0), uv(1.0, 0.0), uv(0.0, 0.0)], 0));
    m
}

/// The picture's outline stood up `THICKNESS` thick: the solid cells of a grid, front and back
/// with the picture on them, and walls where a solid cell borders an empty one.
pub fn standee(img: &image::RgbaImage, width: f32, height: f32) -> Mesh {
    let (gw, gh) = if width >= height { (GRID, ((GRID as f32 * height / width).round() as u32).max(2)) } else { (((GRID as f32 * width / height).round() as u32).max(2), GRID) };
    let solid = solid_cells(img, gw, gh);
    let at = |x: i64, y: i64| x >= 0 && y >= 0 && x < gw as i64 && y < gh as i64 && solid[(y as u32 * gw + x as u32) as usize];
    let mut m = Mesh::default();
    // (grid corners, front and back: x across from the left, row 0 at the top)
    let mut corner = std::collections::HashMap::new();
    let mut vert = |m: &mut Mesh, x: u32, y: u32, back: bool| -> u32 {
        *corner.entry((x, y, back)).or_insert_with(|| {
            let px = (x as f32 / gw as f32 - 0.5) * width;
            let pz = (1.0 - y as f32 / gh as f32) * height;
            m.add_vertex(Vec3::new(px, if back { THICKNESS * 0.5 } else { -THICKNESS * 0.5 }, pz))
        })
    };
    let uvc = |x: u32, y: u32| Vec2::new(x as f32 / gw as f32, y as f32 / gh as f32);
    for y in 0..gh {
        for x in 0..gw {
            if !at(x as i64, y as i64) {
                continue;
            }
            let (a, b, c, d) = (vert(&mut m, x, y + 1, false), vert(&mut m, x + 1, y + 1, false), vert(&mut m, x + 1, y, false), vert(&mut m, x, y, false));
            m.faces.push(Face::new(vec![a, b, c, d], vec![uvc(x, y + 1), uvc(x + 1, y + 1), uvc(x + 1, y), uvc(x, y)], 0));
            let (a, b, c, d) = (vert(&mut m, x + 1, y + 1, true), vert(&mut m, x, y + 1, true), vert(&mut m, x, y, true), vert(&mut m, x + 1, y, true));
            m.faces.push(Face::new(vec![a, b, c, d], vec![uvc(x + 1, y + 1), uvc(x, y + 1), uvc(x, y), uvc(x + 1, y)], 0));
            // walls (the edge colour of the cell: the picture's own, stretched)
            let wall = |m: &mut Mesh, vert: &mut dyn FnMut(&mut Mesh, u32, u32, bool) -> u32, (x0, y0): (u32, u32), (x1, y1): (u32, u32)| {
                let (f0, f1, b1, b0) = (vert(m, x0, y0, false), vert(m, x1, y1, false), vert(m, x1, y1, true), vert(m, x0, y0, true));
                let cuv = uvc(x, y);
                m.faces.push(Face::new(vec![f0, f1, b1, b0], vec![cuv; 4], 0));
            };
            if !at(x as i64, y as i64 - 1) {
                wall(&mut m, &mut vert, (x + 1, y), (x, y));
            }
            if !at(x as i64, y as i64 + 1) {
                wall(&mut m, &mut vert, (x, y + 1), (x + 1, y + 1));
            }
            if !at(x as i64 - 1, y as i64) {
                wall(&mut m, &mut vert, (x, y), (x, y + 1));
            }
            if !at(x as i64 + 1, y as i64) {
                wall(&mut m, &mut vert, (x + 1, y + 1), (x + 1, y));
            }
        }
    }
    m
}

pub fn import_image(path: &Path, content: &Path) -> anyhow::Result<Report> {
    let provenance = crate::provenance::Source::capture(path)?;
    let mut img = crate::models::picture(None, Some(path)).ok_or_else(|| anyhow::anyhow!("{}", tr("The picture cannot be read", "Картинку не удалось прочитать")))?;
    // (a picture too large for a game's texture is brought down; OMSI textures are powers of two best, but any size works)
    let longest = img.width().max(img.height());
    if longest > 2048 {
        let k = 2048.0 / longest as f32;
        img = image::imageops::resize(&img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), image::imageops::FilterType::Lanczos3);
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "picture".into());
    let safe = crate::editable::safe_name(&stem);
    let roots = crate::install::Roots { omsi: Default::default(), content: content.to_path_buf() };
    let rel = crate::editable::new_rel(&roots, "Imported", &safe);
    let (dir, own) = crate::editable::folder_of(&roots, &rel);
    let tex_dir = dir.join("texture");
    std::fs::create_dir_all(&tex_dir)?;
    let tex = format!("{own}.png");
    img.save(tex_dir.join(&tex))?;
    let aspect = img.width() as f32 / img.height() as f32;
    let (height, width) = (HEIGHT, (HEIGHT * aspect).clamp(0.1, 60.0));
    let alpha = crate::models::alpha_of(&img);
    let cut_out = alpha > 0;
    let mesh = if cut_out { standee(&img, width, height) } else { sign(width, height) };
    let material = openmapper_mesh::Material { name: tex.clone(), color: [1.0; 4], texture: Some(tex), alpha: alpha.min(2), ..Default::default() };
    let mut src = crate::editable::Source { name: stem.clone(), ..Default::default() };
    src.model.materials = vec![material];
    src.model.parts.push(openmapper_mesh::Part::new(&safe, mesh));
    crate::editable::save(&roots, &rel, &src)?;
    provenance.record(content, &dir)?;
    let what = if cut_out { tr("Picture made into a standee:", "Картинка стала фигурой по контуру:") } else { tr("Picture made into a sign:", "Картинка стала табличкой:") };
    Ok(Report { copied: 1, objects: vec![rel], message: format!("{what} {stem}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_with_a_hole_becomes_a_closed_standee_with_walls_round_the_hole() {
        // (a 8 by 8 picture, the middle clear)
        let img = image::RgbaImage::from_fn(8, 8, |x, y| if (3..5).contains(&x) && (3..5).contains(&y) { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([200, 20, 20, 255]) });
        let m = standee(&img, 2.0, 2.0);
        assert!(!m.faces.is_empty());
        // every wall edge is shared by exactly one wall and one cap: the mesh has no open edges
        let open = m.edge_faces().values().filter(|f| f.len() != 2).count();
        assert_eq!(open, 0, "{open} open edges");
        // (stands on the ground, a metre either side of the middle)
        let (lo, hi) = m.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(l, h), p| (l.min(*p), h.max(*p)));
        assert!(lo.z.abs() < 1e-5 && (hi.z - 2.0).abs() < 1e-5 && (hi.x - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_plain_picture_is_a_sign_seen_from_both_sides() {
        let m = sign(2.0, 1.0);
        assert_eq!(m.faces.len(), 2);
        assert!(m.face_normal(0).y < 0.0 && m.face_normal(1).y > 0.0);
    }
}
