//! Labels drawn over a `shot`: what the picture alone does not tell (which spline is which,
//! where a road ends with nothing joined, what the map check finds, points of one's own).

use glam::DVec3;
use tiny_skia::{FillRule, PathBuilder, Pixmap, Transform};

use super::render::{paint, stroke};
use super::{Args, Server};
use crate::camera::Camera;
use crate::doc::tile_origin;
use crate::library::Category;

/// Draw the asked layers over `img`; how many labels and marks were drawn.
pub(crate) fn draw(s: &mut Server, a: &Args, cam: &Camera, img: &mut image::RgbaImage, layers: &[String]) -> usize {
    // (points of one's own read first: the map and the models are lent below)
    let marks: Vec<(glam::DVec2, Option<f64>, Option<String>, (u8, u8, u8))> = a
        .arr("marks")
        .map(|v| {
            v.iter()
                .filter_map(|m| {
                    let at = s.point(m.get("at")?).ok()?;
                    let c = m.get("colour").or(m.get("color")).and_then(|v| v.as_array()).filter(|v| v.len() >= 3).map(|v| (v[0].as_u64().unwrap_or(0) as u8, v[1].as_u64().unwrap_or(0) as u8, v[2].as_u64().unwrap_or(0) as u8)).unwrap_or((255, 0, 255));
                    Some((at, m.get("z").and_then(|v| v.as_f64()), m.get("text").and_then(|v| v.as_str()).map(str::to_string), c))
                })
                .collect()
        })
        .unwrap_or_default();
    let assets = &mut s.assets;
    let lib = &s.lib;
    let (w, h) = (img.width(), img.height());
    let Some(mut pm) = Pixmap::from_vec(std::mem::take(img).into_raw(), tiny_skia::IntSize::from_wh(w, h).unwrap()) else { return 0 };
    let on = |l: &str| layers.iter().any(|x| x == l || x == "all");
    let proj = |p: DVec3| cam.project(p, w as f32, h as f32).filter(|(x, y)| *x >= -20.0 && *y >= -20.0 && *x <= w as f32 + 20.0 && *y <= h as f32 + 20.0);
    let id = Transform::identity();
    let mut n = 0usize;
    let dot = |pm: &mut Pixmap, x: f32, y: f32, r: f32, c: (u8, u8, u8)| {
        if let Some(p) = PathBuilder::from_circle(x, y, r) {
            pm.fill_path(&p, &paint(c.0, c.1, c.2, 235), FillRule::Winding, id, None);
            pm.stroke_path(&p, &paint(255, 255, 255, 220), &stroke(1.2, None), id, None);
        }
    };
    let Some(d) = s.ed.doc.as_ref() else { return 0 };
    let technical = a.bool("technical").unwrap_or(false);
    let mut labels: Vec<(f32, f32, String, (u8, u8, u8))> = Vec::new();
    if on("ids") {
        for (key, t) in &d.tiles {
            let org = tile_origin(*key);
            for sp in t.tile.splines.iter().filter(|x| !x.deleted) {
                let cat = lib.find(&sp.file).map(|x| x.category).unwrap_or(Category::Roads);
                if !technical && !matches!(cat, Category::Roads | Category::Tracks | Category::Paths) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(sp, org);
                if let Some((x, y)) = proj(c.point_at(sp.length * 0.5) + DVec3::Z * 0.3) {
                    let ink = match cat {
                        Category::Paths => (200, 150, 40),
                        Category::Tracks => (200, 80, 200),
                        _ => (90, 160, 255),
                    };
                    labels.push((x + 3.0, y - 3.0, sp.id.to_string(), ink));
                }
            }
        }
    }
    if on("ends") || on("links") {
        use crate::road_ends::EndState;
        for e in crate::road_ends::road_ends(d, lib, assets, technical) {
            let Some((x, y)) = proj(e.pos + DVec3::Z * 0.2) else { continue };
            let c = match e.state {
                EndState::Free if on("ends") => (230, 20, 20),
                EndState::Touching if on("ends") => (255, 140, 0),
                EndState::Plate if on("links") => (40, 120, 255),
                EndState::Linked if on("links") => (20, 170, 40),
                _ => continue,
            };
            dot(&mut pm, x, y, if matches!(e.state, EndState::Free | EndState::Touching) { 4.5 } else { 2.5 }, c);
            n += 1;
        }
    }
    if on("grid") {
        // world lines every `grid` metres (10 by default), labelled at the picture's left and top
        let step = a.f64("grid").unwrap_or(10.0).max(1.0);
        let c = cam.smooth_target_now();
        let reach = cam.smooth_distance_now() * 1.5 + 50.0;
        let z = c.z;
        let (x0, x1) = (((c.x - reach) / step).floor() as i64, ((c.x + reach) / step).ceil() as i64);
        let (y0, y1) = (((c.y - reach) / step).floor() as i64, ((c.y + reach) / step).ceil() as i64);
        let ink = paint(255, 255, 255, 110);
        let line = |pm: &mut Pixmap, a: DVec3, b: DVec3| -> Option<(f32, f32, f32, f32)> {
            let mut pb = PathBuilder::new();
            let mut first = None;
            let mut last = None;
            for k in 0..=40 {
                let q = a + (b - a) * (k as f64 / 40.0);
                if let Some((x, y)) = cam.project(q, w as f32, h as f32) {
                    if first.is_none() {
                        pb.move_to(x, y);
                        first = Some((x, y));
                    } else {
                        pb.line_to(x, y);
                    }
                    last = Some((x, y));
                }
            }
            let path = pb.finish()?;
            pm.stroke_path(&path, &ink, &stroke(1.0, None), id, None);
            let (f, l) = (first?, last?);
            Some((f.0, f.1, l.0, l.1))
        };
        for i in x0..=x1 {
            let x = i as f64 * step;
            if let Some((fx, fy, lx, ly)) = line(&mut pm, DVec3::new(x, y0 as f64 * step, z), DVec3::new(x, y1 as f64 * step, z)) {
                // (labelled where the line crosses the top edge, or its upper end)
                let (tx, ty) = if fy < ly { (fx, fy) } else { (lx, ly) };
                labels.push((tx.clamp(2.0, w as f32 - 40.0) + 2.0, ty.max(12.0) + 2.0, format!("x{}", x), (230, 230, 230)));
            }
        }
        for j in y0..=y1 {
            let y = j as f64 * step;
            if let Some((fx, fy, lx, ly)) = line(&mut pm, DVec3::new(x0 as f64 * step, y, z), DVec3::new(x1 as f64 * step, y, z)) {
                let (tx, ty) = if fx < lx { (fx, fy) } else { (lx, ly) };
                labels.push((tx.max(2.0) + 2.0, ty.clamp(12.0, h as f32 - 4.0) - 2.0, format!("y{}", y), (230, 230, 230)));
            }
        }
    }
    if on("street") {
        for i in crate::street_check::check(d, lib, assets, None) {
            let Some((x, y)) = proj(DVec3::new(i.at.x, i.at.y, d.ground(i.at) + 0.3)) else { continue };
            let c = match i.kind {
                "object_on_road" | "path_into_road" | "link_kink" => (230, 30, 30),
                "road_end_free" => (230, 120, 0),
                _ => (200, 0, 200),
            };
            if let Some(r) = tiny_skia::Rect::from_xywh(x - 6.0, y - 6.0, 12.0, 12.0) {
                pm.stroke_path(&PathBuilder::from_rect(r), &paint(c.0, c.1, c.2, 255), &stroke(2.5, None), id, None);
            }
            labels.push((x + 8.0, y + 4.0, i.kind.to_string(), c));
            n += 1;
        }
    }
    if on("issues") {
        for i in crate::check::issues(d, lib) {
            let Some(at) = i.at else { continue };
            let Some((x, y)) = proj(DVec3::new(at.x, at.y, d.ground(at) + 0.3)) else { continue };
            let c = match i.severity {
                crate::check::Severity::Error => (220, 0, 0),
                crate::check::Severity::Warning => (230, 120, 0),
                _ => (0, 110, 200),
            };
            if let Some(r) = tiny_skia::Rect::from_xywh(x - 5.0, y - 5.0, 10.0, 10.0) {
                pm.stroke_path(&PathBuilder::from_rect(r), &paint(c.0, c.1, c.2, 255), &stroke(2.5, None), id, None);
            }
            labels.push((x + 7.0, y + 4.0, format!("{:?}", i.kind), c));
            n += 1;
        }
    }
    // points of one's own: marks [{at: [x, y], text, colour}]
    for (at, z, t, c) in marks {
        let z = z.unwrap_or_else(|| d.ground(at) + 0.3);
        let Some((x, y)) = proj(DVec3::new(at.x, at.y, z)) else { continue };
        dot(&mut pm, x, y, 5.0, c);
        if let Some(t) = t {
            labels.push((x + 7.0, y + 4.0, t, c));
        }
        n += 1;
    }
    // (labels last, over every mark; too many would be a smear)
    if labels.len() <= 900 {
        for (x, y, t, c) in &labels {
            label(&mut pm, t, *x, *y, *c);
        }
    }
    n += labels.len();
    // tiny-skia keeps premultiplied colour: the picture is opaque, so it is the same
    *img = image::RgbaImage::from_raw(w, h, pm.take()).unwrap_or_default();
    n
}

/// A label on a dark plate, in the colour of what it names (its left end at x, baseline y).
fn label(pm: &mut Pixmap, s: &str, x: f32, y: f32, ink: (u8, u8, u8)) {
    let Some(f) = crate::osm_map::font() else { return };
    let Some((path, _)) = crate::osm_map::text_path(f, s, 12.0) else { return };
    let b = path.bounds();
    if let Some(r) = tiny_skia::Rect::from_xywh(x + b.left() - 2.0, y + b.top() - 2.0, b.width() + 4.0, b.height() + 4.0) {
        pm.fill_rect(r, &paint(15, 15, 15, 190), Transform::identity(), None);
    }
    // (light enough to read on the dark plate)
    let lift = |c: u8| (c as u16 + (255 - c as u16) * 11 / 20) as u8;
    pm.fill_path(&path, &paint(lift(ink.0), lift(ink.1), lift(ink.2), 255), FillRule::Winding, Transform::from_translate(x, y), None);
}
