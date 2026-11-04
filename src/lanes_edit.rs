//! A road's own lanes made editable. A road type's AI lanes (its `[path]` blocks) belong to
//! the road type: OMSI has no way to take one away or bend it on a single road. So the
//! road is given a copy of its type without lanes (the same look), and each lane becomes an
//! AI path spline of its own, along the same line and the same way, which the Logic tool
//! edits like any path drawn by hand.

use std::path::Path;

use omsi_map::MapSpline;

use crate::assets::Assets;
use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::install::Roots;

/// The text of a road type without its `[path]` blocks.
pub fn strip_paths(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in text.lines().map(str::trim_end) {
        let t = line.trim();
        if t.eq_ignore_ascii_case("[path]") {
            skipping = true;
            continue;
        }
        if skipping {
            // (a block's values run to the next blank line or the next keyword)
            if t.is_empty() || t.starts_with('[') {
                skipping = false;
            } else {
                continue;
            }
        }
        out.push(line);
    }
    out.join("\n") + "\n"
}

/// The textures a road type's text names.
fn textures_of(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.iter().enumerate().filter(|(_, l)| l.eq_ignore_ascii_case("[texture]")).filter_map(|(i, _)| lines.get(i + 1).filter(|n| !n.is_empty()).map(|n| n.to_string())).collect()
}

/// The copy of a road type without lanes, written into the content folder with the textures
/// it names; its relative path. The same copy is used again the next time.
pub fn lane_free_copy(roots: &Roots, rel: &str) -> anyhow::Result<String> {
    let from = [&roots.content, &roots.omsi].iter().map(|r| omsi_cfg::resolve_path(r, rel)).find(|p| p.is_file()).ok_or_else(|| anyhow::anyhow!("{rel} is not installed"))?;
    let text = omsi_cfg::decode_text(&std::fs::read(&from)?);
    let stem = Path::new(&rel.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "road".into());
    let dir = roots.content.join("Splines").join("openMapper");
    let tex_dir = dir.join("texture");
    std::fs::create_dir_all(&tex_dir)?;
    // (the textures it names, found as the game finds them, brought along)
    let src_dir = from.parent().map(Path::to_path_buf).unwrap_or_default();
    let dirs = crate::assets::texture_dirs(&roots.omsi, &roots.content, &src_dir);
    let refs: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
    let global = [omsi_cfg::resolve_path(&roots.content, "Texture"), omsi_cfg::resolve_path(&roots.omsi, "Texture")];
    for name in textures_of(&text) {
        let Some(found) = omsi_texture::find_texture(&name, &refs) else { continue };
        if found.parent().map(|p| global.iter().any(|g| g == p)).unwrap_or(false) {
            continue;
        }
        let Some(file) = found.file_name() else { continue };
        if found.parent() != Some(tex_dir.as_path()) {
            let _ = std::fs::copy(&found, tex_dir.join(file));
            for ext in ["cfg", "surf"] {
                let side = std::path::PathBuf::from(format!("{}.{ext}", found.display()));
                if side.is_file() {
                    let _ = std::fs::copy(&side, tex_dir.join(format!("{}.{ext}", file.to_string_lossy())));
                }
            }
        }
    }
    let name = format!("{stem}_no_lanes");
    let path = dir.join(format!("{name}.sli"));
    if !path.is_file() {
        crate::doc::safe::write_atomic(&path, &omsi_cfg::encode_text(&strip_paths(&text).replace('\n', "\r\n")))?;
    }
    Ok(format!("Splines\\openMapper\\{name}.sli"))
}

/// The invisible rail lane the Logic tool draws for the AI's trains, written into the content
/// folder when it is first wanted (the installation has no such spline of its own to count on).
/// Its relative path.
pub fn rail_path_spline(roots: &Roots) -> anyhow::Result<String> {
    const RAIL_PATH: &str = "Splines\\openMapper\\invis_rail.sli";
    let path = omsi_cfg::resolve_path(&roots.content, RAIL_PATH);
    if !path.is_file() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = "File created with openMapper\n\n[onlyeditor]\n\n[path]\n2\n0\n0\n1.435\n2\n";
        crate::doc::safe::write_atomic(&path, &omsi_cfg::encode_text(&text.replace('\n', "\r\n")))?;
    }
    Ok(RAIL_PATH.to_string())
}

/// One lane's spline: the road's piece offset sideways by `x` (m), run the way the lane
/// goes (`reverse`: from the road's end back).
fn lane_spline(s: &MapSpline, k: TileKey, x: f64, z: f64, reverse: bool, file: &str, id: i64) -> Option<(TileKey, MapSpline)> {
    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
    let mirror = if s.mirror { -1.0 } else { 1.0 };
    let x = x * mirror;
    // (an arc offset is an arc: its radius changes by the offset, its angle stays)
    let (radius, length) = if s.radius.abs() > 1e-6 {
        let r = s.radius - x;
        if r.abs() < 1.0 || r.signum() != s.radius.signum() {
            return None;
        }
        (r, s.length * r / s.radius)
    } else {
        (0.0, s.length)
    };
    let at = if reverse { s.length } else { 0.0 };
    let p = c.offset_point(at, x, z);
    let heading = crate::roads::norm_deg(c.heading_at(at) + if reverse { 180.0 } else { 0.0 });
    let key = tile_of(p.truncate());
    let org = tile_origin(key);
    let (grad_start, grad_end, delta_h) = if reverse { (-s.grad_end, -s.grad_start, s.delta_h.map(|d| -d)) } else { (s.grad_start, s.grad_end, s.delta_h) };
    Some((
        key,
        MapSpline {
            file: file.to_string(),
            id,
            pos: [p.x - org.x, p.y - org.y, p.z],
            heading,
            length,
            radius: if reverse { -radius } else { radius },
            grad_start,
            grad_end,
            delta_h,
            ..Default::default()
        },
    ))
}

/// Make the lanes of these road pieces editable: each gets the lane-free copy of its type
/// (`free` maps a type to its copy) and an AI path spline for every street and footway lane.
/// A piece with rails is left as it is. Gives back how many pieces were changed.
pub fn detach(doc: &mut Document, assets: &mut Assets, pieces: &[(TileKey, i64)], free: &std::collections::HashMap<String, String>, car: &str, walk: &str) -> usize {
    let mut changed = 0;
    for (k, id) in pieces {
        let Some(s) = doc.spline(*k, *id).cloned() else { continue };
        let Some(model) = assets.spline(&s.file) else { continue };
        let Some(copy) = free.get(&s.file.to_lowercase()) else { continue };
        if model.def.paths.is_empty() || model.is_ai_path() || model.def.paths.iter().any(|p| p.kind == 2) {
            continue;
        }
        let mut made: Vec<(TileKey, MapSpline)> = Vec::new();
        for p in &model.def.paths {
            let file = if p.kind == 1 { walk } else { car };
            let (x, z) = (p.start[0] as f64, p.start[2] as f64);
            // (the lane's own direction, turned round on a mirrored road)
            let dir = match (p.direction, s.mirror) {
                (0, true) => 1,
                (1, true) => 0,
                (d, _) => d,
            };
            // (a path type that carries traffic both ways takes a two-way lane as one spline)
            let both = assets.spline(file).and_then(|m| m.def.paths.first().map(|q| q.direction == 2)).unwrap_or(false);
            let ways: &[bool] = match dir {
                _ if both => &[false],
                0 => &[false],
                1 => &[true],
                _ => &[false, true],
            };
            for &reverse in ways {
                let new_id = doc.new_id();
                if let Some(m) = lane_spline(&s, *k, x, z, reverse, file, new_id) {
                    made.push(m);
                }
            }
        }
        for (key, sp) in made {
            if let Some(t) = doc.tiles.get_mut(&key) {
                t.tile.splines.push(sp);
                t.touch_content();
            }
        }
        if let Some(t) = doc.tiles.get_mut(k) {
            if let Some(sp) = t.tile.splines.iter_mut().find(|q| q.id == *id) {
                sp.file = copy.clone();
                // (the rules were for the lanes by number: they no longer apply)
                sp.rules.clear();
            }
            t.touch_content();
        }
        changed += 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rail lane's file reads back as an AI path that is a track, made once.
    #[test]
    fn the_rail_lane_is_an_invisible_track() {
        let roots = crate::doc::tests::scratch("rail-lane");
        let rel = rail_path_spline(&roots).unwrap();
        assert_eq!(rail_path_spline(&roots).unwrap(), rel);
        let mut assets = Assets::new(roots.omsi.clone(), roots.content.clone());
        let m = assets.spline_wait(&rel, 5000).expect("it loads");
        assert!(m.is_ai_path() && m.is_track());
        assert_eq!((m.def.paths[0].kind, m.def.paths[0].direction), (2, 2));
        let _ = std::fs::remove_dir_all(&roots.content);
    }

    #[test]
    fn the_lanes_go_and_the_looks_stay() {
        let text = "[texture]\na.bmp\n\n[profile]\n0\n\n[path]\n1\n-5.384\n0.250\n1.176\n2\n\n[path]\n0\n2.000\n0.100\n3.600\n0\n";
        let out = strip_paths(text);
        assert!(out.contains("[texture]") && out.contains("[profile]"));
        assert!(!out.contains("[path]") && !out.contains("5.384"));
        assert_eq!(textures_of(text), vec!["a.bmp".to_string()]);
    }

    #[test]
    fn a_lane_beside_an_arc_is_an_arc_of_the_same_angle() {
        let s = MapSpline { id: 1, pos: [100.0, 100.0, 0.0], heading: 0.0, length: 50.0, radius: 100.0, ..Default::default() };
        let (_, lane) = lane_spline(&s, (0, 0), 2.0, 0.0, false, "x.sli", 9).unwrap();
        // (on the inside of a right-hand bend: smaller radius, shorter)
        assert!((lane.radius - 98.0).abs() < 1e-9 && (lane.length - 49.0).abs() < 1e-9);
        let c = omsi_geometry::SplineCurve::from_map(&s, tile_origin((0, 0)));
        let l = omsi_geometry::SplineCurve::from_map(&lane, tile_origin((0, 0)));
        let want = c.offset_point(s.length, 2.0, 0.0);
        assert!((l.end_point() - want).truncate().length() < 0.01, "{:?} {:?}", l.end_point(), want);
        // run backwards: it starts at the end, heading round
        let (_, back) = lane_spline(&s, (0, 0), 2.0, 0.0, true, "x.sli", 10).unwrap();
        let b = omsi_geometry::SplineCurve::from_map(&back, tile_origin((0, 0)));
        assert!((b.point_at(0.0) - want).truncate().length() < 0.01);
        assert!((b.end_point() - c.offset_point(0.0, 2.0, 0.0)).truncate().length() < 0.01, "{:?}", b.end_point());
    }
}
