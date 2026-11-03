//! More objects made in one click, ready to put on the map and edit further: a picture or a
//! repeating texture on a flat panel (a decal, a pattern, a sign, a billboard), a light
//! source, and a sound source.

use glam::{Mat4, Vec2, Vec3};
use openmapper_mesh::primitives::Primitive;
use openmapper_mesh::{Material, Mesh, Model, Part};

use crate::editable::{Light, LightKind, Source, When};

/// What the makers are set to (kept by the interface).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// A picture's width (m); its height follows the picture's own proportions.
    pub picture_width: f32,
    /// How often the picture repeats along and across the panel (a pattern; 1 and 1: once).
    pub picture_repeat: [f32; 2],
    /// Stands up like a sign (else it lies on the ground like a decal).
    pub picture_standing: bool,
    pub light_color: [u8; 3],
    /// How far the light reaches on the ground (m).
    pub light_range: f32,
    pub light_night_only: bool,
    /// A sound's reach (m) and volume (0..1).
    pub sound_radius: f32,
    pub sound_volume: f32,
    /// A polygon's picture or texture (a file), else its plain colour.
    pub poly_texture: Option<std::path::PathBuf>,
    pub poly_color: [u8; 3],
    /// How many metres its texture covers before it repeats.
    pub poly_scale: f32,
    /// A polygon's thickness (m): 0 is a flat sheet, more a slab with sides and a bottom.
    pub poly_thickness: f32,
    /// How far over the corners' height it lies (m), so it does not flicker with the ground.
    pub poly_lift: f32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { picture_width: 4.0, picture_repeat: [1.0, 1.0], picture_standing: false, light_color: [255, 220, 160], light_range: 12.0, light_night_only: true, sound_radius: 40.0, sound_volume: 0.8, poly_texture: None, poly_color: [72, 72, 78], poly_scale: 4.0, poly_thickness: 0.0, poly_lift: 0.03 }
    }
}

/// A flat panel (one side up, one down, so it shows from both) `w` × `h` m, its picture
/// `repeat` times along and across; lying on the ground a hair above it, or standing.
pub fn picture(texture: &str, aspect: f32, alpha: u8, s: &Settings) -> Source {
    let w = s.picture_width.clamp(0.1, 300.0);
    let h = (w / aspect.max(0.01)).clamp(0.1, 300.0);
    let mut front = Primitive::Plane { size: Vec2::new(w, h) }.build();
    for f in &mut front.faces {
        for uv in &mut f.uvs {
            uv.x *= s.picture_repeat[0].max(0.01) / w;
            uv.y *= s.picture_repeat[1].max(0.01) / h;
        }
    }
    let mut back = front.clone();
    back.flip_normals();
    let mut mesh = Mesh::default();
    mesh.append(&front, 0);
    mesh.append(&back, 0);
    let place = if s.picture_standing { Mat4::from_translation(Vec3::new(0.0, 0.0, h / 2.0)) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2) } else { Mat4::from_translation(Vec3::new(0.0, 0.0, 0.03)) };
    mesh.transform(place);
    let material = Material { name: "Picture".into(), color: [1.0, 1.0, 1.0, 1.0], texture: Some(texture.to_string()), alpha, specular: [0.0; 3], ..Default::default() };
    Source { name: "Picture".into(), model: Model { parts: vec![Part::new("Panel", mesh)], materials: vec![material] }, ..Default::default() }
}

/// A polygon filled between `pts` (corners in metres, z up, round in either direction): a
/// sheet at their heights, or a slab `poly_thickness` thick; its texture repeating every
/// `poly_scale` m. None for fewer than three corners or no area.
pub fn polygon(corners: &[Vec3], texture: Option<&str>, alpha: u8, s: &Settings) -> Option<Source> {
    let mut pts: Vec<Vec3> = Vec::with_capacity(corners.len());
    for p in corners {
        if pts.last().map_or(true, |q| (q.truncate() - p.truncate()).length() > 0.02) {
            pts.push(*p);
        }
    }
    while pts.len() > 1 && (pts[0].truncate() - pts[pts.len() - 1].truncate()).length() <= 0.02 {
        pts.pop();
    }
    if pts.len() < 3 {
        return None;
    }
    let area: f32 = (0..pts.len()).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        a.x * b.y - b.x * a.y
    }).sum::<f32>() / 2.0;
    if area.abs() < 0.01 {
        return None;
    }
    // (anticlockwise seen from above, so the top faces up)
    if area < 0.0 {
        pts.reverse();
    }
    let scale = s.poly_scale.clamp(0.1, 500.0);
    let lift = Vec3::new(0.0, 0.0, s.poly_lift.clamp(-5.0, 5.0));
    let thick = s.poly_thickness.clamp(0.0, 50.0);
    let n = pts.len();
    let uv = |p: Vec3| Vec2::new(p.x / scale, p.y / scale);
    let mut mesh = Mesh::default();
    let top: Vec<u32> = pts.iter().map(|p| mesh.add_vertex(*p + lift)).collect();
    mesh.add_face(top.clone(), pts.iter().map(|p| uv(*p)).collect(), 0);
    if thick > 0.001 {
        let bottom: Vec<u32> = pts.iter().map(|p| mesh.add_vertex(*p + lift - Vec3::new(0.0, 0.0, thick))).collect();
        mesh.add_face(bottom.iter().rev().copied().collect(), pts.iter().rev().map(|p| uv(*p)).collect(), 0);
        let mut along = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            let len = (pts[j].truncate() - pts[i].truncate()).length();
            let (u0, u1) = (along / scale, (along + len) / scale);
            along += len;
            mesh.add_face(vec![top[i], bottom[i], bottom[j], top[j]], vec![Vec2::new(u0, 0.0), Vec2::new(u0, thick / scale), Vec2::new(u1, thick / scale), Vec2::new(u1, 0.0)], 0);
        }
    }
    let c = s.poly_color.map(|v| v as f32 / 255.0);
    let material = match texture {
        Some(t) => Material { name: "Polygon".into(), color: [1.0, 1.0, 1.0, 1.0], texture: Some(t.to_string()), alpha, specular: [0.0; 3], ..Default::default() },
        None => Material { name: "Polygon".into(), color: [c[0], c[1], c[2], 1.0], specular: [0.0; 3], ..Default::default() },
    };
    Some(Source { name: "Polygon".into(), model: Model { parts: vec![Part::new("Polygon", mesh)], materials: vec![material] }, abs_height: true, ..Default::default() })
}

/// Lines joined end to end into one outline, whatever order and way round they come in; each
/// end meets the next within `tol` m. None if they do not all join.
pub fn outline(lines: Vec<Vec<glam::DVec3>>, tol: f64) -> Option<Vec<glam::DVec3>> {
    let mut rest: Vec<Vec<glam::DVec3>> = lines.into_iter().filter(|l| !l.is_empty()).collect();
    if rest.is_empty() {
        return None;
    }
    let mut ring = rest.remove(0);
    while !rest.is_empty() {
        let (head, tail) = (ring[0], *ring.last()?);
        // (distance, which line, attach to the tail (else the head), the line reversed)
        let mut best: Option<(f64, usize, bool, bool)> = None;
        for (i, l) in rest.iter().enumerate() {
            let (a, b) = (l[0], *l.last()?);
            for (d, at_tail, rev) in [(tail.distance(a), true, false), (tail.distance(b), true, true), (head.distance(b), false, false), (head.distance(a), false, true)] {
                if d <= tol && best.map_or(true, |x| d < x.0) {
                    best = Some((d, i, at_tail, rev));
                }
            }
        }
        let (_, i, at_tail, rev) = best?;
        let mut l = rest.remove(i);
        if rev {
            l.reverse();
        }
        if at_tail {
            ring.extend(l.into_iter().skip(1));
        } else {
            l.pop();
            l.extend(ring);
            ring = l;
        }
    }
    Some(ring)
}

/// A light: a small lamp that glows and lights the ground round it (at night, or always).
pub fn light_source(s: &Settings) -> Source {
    let bulb = {
        let mut m = Primitive::UvSphere { radius: 0.12, segments: 12, rings: 6 }.build();
        m.transform(Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0)));
        for f in &mut m.faces {
            f.material = 0;
            f.smooth = true;
        }
        m
    };
    let c = s.light_color.map(|v| v as f32 / 255.0);
    let materials = vec![Material { name: "Bulb".into(), color: [c[0], c[1], c[2], 1.0], emissive: c, ..Default::default() }];
    let when = if s.light_night_only { When::Night } else { When::Always };
    let glow = Light { kind: LightKind::Glow, pos: Vec3::new(0.0, 0.0, 1.0), color: s.light_color, size: 0.8, cone: None, when: when.clone(), ..Default::default() };
    let ground = Light { kind: LightKind::Ground, pos: Vec3::new(0.0, 0.0, 1.0), color: s.light_color, range: s.light_range.clamp(1.0, 200.0), when, ..Default::default() };
    Source { name: "Light source".into(), model: Model { parts: vec![Part::new("Bulb", bulb)], materials }, lights: vec![glow, ground], ..Default::default() }
}

/// A sound source: a marker only the editor shows, and OMSI's sound description file (what
/// plays, how loud, how far it carries) written beside the object by [`write_sound`].
pub fn sound_source(stem: &str) -> Source {
    let mut m = Primitive::Cube { size: Vec3::new(0.4, 0.4, 0.4) }.build();
    m.transform(Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)));
    for f in &mut m.faces {
        f.material = 0;
    }
    let materials = vec![Material::colored("Marker", [0.2, 0.7, 1.0, 1.0])];
    Source {
        name: "Sound source".into(),
        model: Model { parts: vec![Part::new("Marker", m)], materials },
        extra: format!("[onlyeditor]\n\n[sound]\nsound\\{stem}.cfg\n"),
        ..Default::default()
    }
}

/// The sound description (`sound\<stem>.cfg`) and the sound file beside it, in the object's
/// folder.
pub fn write_sound(dir: &std::path::Path, stem: &str, wav: &std::path::Path, s: &Settings) -> anyhow::Result<()> {
    let sound_dir = dir.join("sound");
    std::fs::create_dir_all(&sound_dir)?;
    let file = wav.file_name().map(|f| f.to_string_lossy().to_string()).ok_or_else(|| anyhow::anyhow!("no sound file"))?;
    std::fs::copy(wav, sound_dir.join(&file))?;
    // (the layout of OMSI 2's own ambient sounds: the file and the volume, the position of
    // the source in the object and how far it carries)
    let cfg = format!("\n\n[sound]\n{file}\n{:.2}\n\n[3d]\n0\n0\n0\n{}\n", s.sound_volume.clamp(0.0, 1.0), s.sound_radius.clamp(1.0, 1000.0).round());
    std::fs::write(sound_dir.join(format!("{stem}.cfg")), omsi_cfg::encode_text(&cfg.replace('\n', "\r\n")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_makers_write_objects_openomsi_reads() {
        let dir = std::env::temp_dir().join(format!("om-makers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let roots = crate::install::Roots { omsi: dir.join("omsi"), content: dir.join("content") };
        let s = Settings::default();
        // a light: a glow and a pool of light on the ground
        let rel = crate::editable::new_rel(&roots, "M", "Light");
        crate::editable::save(&roots, &rel, &light_source(&s)).unwrap();
        let sco = omsi_scenery::sco::SceneryObject::load(&omsi_cfg::resolve_path(&roots.content, &rel)).unwrap();
        assert_eq!(sco.map_lights.len(), 1);
        assert_eq!(sco.model.meshes[0].light_enh_2.len(), 1);
        // a sound: named by the object, and its description beside it
        let rel = crate::editable::new_rel(&roots, "M", "Sound");
        let (folder, stem) = crate::editable::folder_of(&roots, &rel);
        crate::editable::save(&roots, &rel, &sound_source(&stem)).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("bird.wav");
        std::fs::write(&wav, b"RIFF").unwrap();
        write_sound(&folder, &stem, &wav, &s).unwrap();
        let sco = omsi_scenery::sco::SceneryObject::load(&omsi_cfg::resolve_path(&roots.content, &rel)).unwrap();
        assert_eq!(sco.sound.as_deref(), Some(format!("sound\\{stem}.cfg").as_str()));
        assert!(folder.join("sound").join("bird.wav").is_file());
        assert!(folder.join("sound").join(format!("{stem}.cfg")).is_file());
        // a picture: one panel with the texture
        let rel = crate::editable::new_rel(&roots, "M", "Picture");
        crate::editable::save(&roots, &rel, &picture("p.png", 2.0, 1, &s)).unwrap();
        let sco = omsi_scenery::sco::SceneryObject::load(&omsi_cfg::resolve_path(&roots.content, &rel)).unwrap();
        assert_eq!(sco.model.meshes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_polygon_is_a_sheet_facing_up_whichever_way_it_was_clicked() {
        let s = Settings::default();
        let square = [Vec3::new(0.0, 0.0, 1.0), Vec3::new(10.0, 0.0, 1.0), Vec3::new(10.0, 10.0, 1.0), Vec3::new(0.0, 10.0, 1.0)];
        for clockwise in [false, true] {
            let mut pts = square.to_vec();
            if clockwise {
                pts.reverse();
            }
            let src = polygon(&pts, None, 0, &s).unwrap();
            let mesh = &src.model.parts[0].mesh;
            assert_eq!(mesh.faces.len(), 1);
            assert!(mesh.face_normal(0).z > 0.99, "faces up");
            assert!((mesh.face_area(0) - 100.0).abs() < 1e-3);
            assert!(src.abs_height);
        }
        assert!(polygon(&square[..2], None, 0, &s).is_none(), "two corners fill nothing");
        assert!(polygon(&[Vec3::ZERO, Vec3::X, Vec3::X * 2.0], None, 0, &s).is_none(), "a line has no area");
    }

    #[test]
    fn a_thick_polygon_is_a_closed_slab() {
        let s = Settings { poly_thickness: 0.5, ..Default::default() };
        let pts = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0), Vec3::new(8.0, 6.0, 0.0), Vec3::new(0.0, 6.0, 0.0)];
        let src = polygon(&pts, Some("t.png"), 0, &s).unwrap();
        let mesh = &src.model.parts[0].mesh;
        assert_eq!(mesh.faces.len(), 6);
        assert!(mesh.is_closed());
        let up = (0..mesh.faces.len()).filter(|f| mesh.face_normal(*f).z > 0.99).count();
        let down = (0..mesh.faces.len()).filter(|f| mesh.face_normal(*f).z < -0.99).count();
        assert_eq!((up, down), (1, 1));
        let centre = mesh.bounds().map(|(a, b)| (a + b) / 2.0).unwrap();
        for f in 0..mesh.faces.len() {
            let out = mesh.face_center(f) - centre;
            assert!(mesh.face_normal(f).dot(out) > 0.0, "face {f} points out");
        }
        assert_eq!(src.model.materials[0].texture.as_deref(), Some("t.png"));
    }

    #[test]
    fn lines_join_into_one_outline_in_any_order() {
        use glam::DVec3;
        let a = DVec3::new(0.0, 0.0, 0.0);
        let b = DVec3::new(10.0, 0.0, 0.0);
        let c = DVec3::new(10.0, 10.0, 0.0);
        let d = DVec3::new(0.0, 10.0, 0.0);
        // (the second one backwards, the last one first)
        let ring = outline(vec![vec![a, b], vec![c, b], vec![d, c], vec![a, d]], 0.5).unwrap();
        assert_eq!(ring.len(), 5);
        assert!(ring[0].distance(*ring.last().unwrap()) < 0.5);
        // (the first one in the middle of the chain)
        let chain = outline(vec![vec![b, c], vec![a, b], vec![c, d]], 0.5).unwrap();
        assert_eq!(chain.len(), 4);
        assert!(chain[0].distance(a) < 1e-9 && chain[3].distance(d) < 1e-9);
        // (one that does not meet the others)
        assert!(outline(vec![vec![a, b], vec![c, d]], 0.5).is_none());
    }

    #[test]
    fn a_polygon_is_written_as_an_object_openomsi_reads() {
        let dir = std::env::temp_dir().join(format!("om-polygon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let roots = crate::install::Roots { omsi: dir.join("omsi"), content: dir.join("content") };
        let pts = [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0), Vec3::new(-5.0, 5.0, 0.0)];
        let rel = crate::editable::new_rel(&roots, "M", "Polygon");
        crate::editable::save(&roots, &rel, &polygon(&pts, None, 0, &Settings { poly_thickness: 0.3, ..Default::default() }).unwrap()).unwrap();
        let sco = omsi_scenery::sco::SceneryObject::load(&omsi_cfg::resolve_path(&roots.content, &rel)).unwrap();
        assert_eq!(sco.model.meshes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
