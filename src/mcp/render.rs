//! `render`: the map from above as a picture, for an agent to see what is where and what an
//! edit did. The ground shaded by its height, water, roads at their widths, houses at their
//! footprints, trees, stops with their names, start points, a grid with coordinates.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use glam::DVec2;
use serde_json::json;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

use super::{number, p2, Args, Out, Reply, Server};
use crate::doc::{tile_of, tile_origin, tile_size, Sel};
use crate::library::Category;

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn stroke(w: f32, dash: Option<StrokeDash>) -> Stroke {
    Stroke { width: w.max(0.5), line_cap: LineCap::Round, line_join: LineJoin::Round, dash, ..Default::default() }
}

/// A label with a light halo, its left end at (x, y) (the baseline).
fn text(pm: &mut Pixmap, s: &str, x: f32, y: f32, px: f32, ink: (u8, u8, u8)) {
    let Some(f) = crate::osm_map::font() else { return };
    let Some((path, _)) = crate::osm_map::text_path(f, s, px) else { return };
    let t = Transform::from_translate(x, y);
    pm.stroke_path(&path, &paint(255, 255, 255, 220), &stroke(px * 0.22, None), t, None);
    pm.fill_path(&path, &paint(ink.0, ink.1, ink.2, 255), FillRule::Winding, t, None);
}

/// A tidy step for grid lines: 1, 2 or 5 times a power of ten, about `want` metres.
fn nice(want: f64) -> f64 {
    let p = 10f64.powf(want.max(1e-3).log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * p).find(|s| *s >= want).unwrap_or(10.0 * p)
}

impl Server {
    pub fn render(&mut self, a: Args) -> Reply {
        self.wait_lib();
        let d = self.doc()?;
        let ts = tile_size();
        // the part of the map shown
        let (mut lo, mut hi) = match (a.get("center").or(a.get("at")), a.get("min"), a.get("max")) {
            (_, Some(l), Some(h)) => {
                let (l, h) = (self.point(l)?, self.point(h)?);
                (l.min(h), l.max(h))
            }
            (Some(c), _, _) => {
                let c = self.point(c)?;
                let r = a.f64("radius").unwrap_or(150.0).max(5.0);
                (c - DVec2::splat(r), c + DVec2::splat(r))
            }
            _ => {
                if d.tiles.is_empty() {
                    return Err("the map has no tiles".into());
                }
                let mut l = DVec2::splat(f64::MAX);
                let mut h = DVec2::splat(f64::MIN);
                for k in d.tiles.keys() {
                    l = l.min(tile_origin(*k));
                    h = h.max(tile_origin(*k) + DVec2::splat(ts));
                }
                (l, h)
            }
        };
        if (hi - lo).min_element() < 1.0 {
            hi = lo + DVec2::splat(1.0).max(hi - lo);
            lo = lo.min(hi);
        }
        let size = a.u32_or("size", 768).clamp(256, 2048) as f64;
        let span = hi - lo;
        let k = size / span.max_element();
        let (w, h) = ((span.x * k).round().max(16.0) as u32, (span.y * k).round().max(16.0) as u32);
        let to_px = |p: DVec2| ((p.x - lo.x) * k, (hi.y - p.y) * k);
        let at_px = |x: f64, y: f64| DVec2::new(lo.x + x / k, hi.y - y / k);

        // the ground: shaded by height, water over it, nothing off the map
        let cell = if w * h > 1_200_000 { 2 } else { 1 };
        let (gw, gh) = (w.div_ceil(cell) as usize + 1, h.div_ceil(cell) as usize + 1);
        let mut hs: Vec<Option<f32>> = Vec::with_capacity(gw * gh);
        for j in 0..gh {
            for i in 0..gw {
                hs.push(d.height_at(at_px((i as u32 * cell) as f64, (j as u32 * cell) as f64)));
            }
        }
        let (mut zmin, mut zmax) = (f32::MAX, f32::MIN);
        for z in hs.iter().flatten() {
            zmin = zmin.min(*z);
            zmax = zmax.max(*z);
        }
        let zr = (zmax - zmin).max(1.0);
        let mpp = (cell as f64 / k) as f32;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let mut water_of: HashMap<(i32, i32), Option<f32>> = HashMap::new();
        for y in 0..h {
            for x in 0..w {
                let (i, j) = ((x / cell) as usize, (y / cell) as usize);
                let px = (y * w + x) as usize * 4;
                let c = match hs[j * gw + i] {
                    None => [58u8, 60, 66],
                    Some(z) => {
                        let zx = hs[j * gw + (i + 1).min(gw - 1)].unwrap_or(z);
                        let zy = hs[(j + 1).min(gh - 1) * gw + i].unwrap_or(z);
                        // (light from the north-west)
                        let (dx, dy) = ((zx - z) / mpp, (z - zy) / mpp);
                        let shade = (1.0 - 0.9 * (dx - dy) / (1.0 + (dx * dx + dy * dy).sqrt()) * 0.5).clamp(0.55, 1.25);
                        let t = (z - zmin) / zr;
                        let base = [0.74 + 0.10 * t, 0.80 - 0.04 * t, 0.62 - 0.02 * t];
                        let p = at_px(x as f64, y as f64);
                        let key = tile_of(p);
                        let level = *water_of.entry(key).or_insert_with(|| d.tiles.get(&key).and_then(|t| t.water).map(|w| w.iter().sum::<f32>() / 4.0));
                        match level {
                            Some(l) if l > z => {
                                let deep = ((l - z) / 4.0).clamp(0.0, 1.0);
                                [(90.0 - 40.0 * deep) as u8, (150.0 - 40.0 * deep) as u8, (205.0 - 30.0 * deep) as u8]
                            }
                            _ => [(base[0] * shade * 255.0).min(255.0) as u8, (base[1] * shade * 255.0).min(255.0) as u8, (base[2] * shade * 255.0).min(255.0) as u8],
                        }
                    }
                };
                buf[px..px + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let mut pm = Pixmap::new(w, h).ok_or("picture too big")?;
        pm.data_mut().copy_from_slice(&buf);
        let id = Transform::identity();
        let line = |pts: &[(f64, f64)]| -> Option<tiny_skia::Path> {
            let mut pb = PathBuilder::new();
            for (i, p) in pts.iter().enumerate() {
                if i == 0 {
                    pb.move_to(p.0 as f32, p.1 as f32);
                } else {
                    pb.line_to(p.0 as f32, p.1 as f32);
                }
            }
            pb.finish()
        };
        let margin = 60.0;
        let in_view = |p: DVec2, reach: f64| p.x > lo.x - reach - margin && p.y > lo.y - reach - margin && p.x < hi.x + reach + margin && p.y < hi.y + reach + margin;

        // tile edges and the grid
        let grid = a.0.get("grid").and_then(|g| g.as_bool()).unwrap_or(true);
        if grid {
            let step = nice(span.max_element() / 8.0);
            let mut x = (lo.x / step).ceil() * step;
            while x < hi.x {
                let (px, _) = to_px(DVec2::new(x, lo.y));
                if let Some(p) = line(&[(px, 0.0), (px, h as f64)]) {
                    pm.stroke_path(&p, &paint(255, 255, 255, 70), &stroke(1.0, None), id, None);
                }
                text(&mut pm, &format!("{x:.0}"), px as f32 + 2.0, 12.0, 11.0, (40, 40, 40));
                x += step;
            }
            let mut y = (lo.y / step).ceil() * step;
            while y < hi.y {
                let (_, py) = to_px(DVec2::new(lo.x, y));
                if let Some(p) = line(&[(0.0, py), (w as f64, py)]) {
                    pm.stroke_path(&p, &paint(255, 255, 255, 70), &stroke(1.0, None), id, None);
                }
                text(&mut pm, &format!("{y:.0}"), 2.0, py as f32 - 2.0, 11.0, (40, 40, 40));
                y += step;
            }
        }
        for key in d.tiles.keys() {
            let o = tile_origin(*key);
            if !in_view(o + DVec2::splat(ts * 0.5), ts) {
                continue;
            }
            let (a0, b0) = (to_px(o), to_px(o + DVec2::splat(ts)));
            if let Some(r) = Rect::from_ltrb(a0.0 as f32, b0.1 as f32, b0.0 as f32, a0.1 as f32) {
                let p = PathBuilder::from_rect(r);
                pm.stroke_path(&p, &paint(30, 30, 30, 110), &stroke(1.2, Some(StrokeDash::new(vec![6.0, 4.0], 0.0).unwrap())), id, None);
            }
        }

        // the models of the objects in view, for their footprints (a short wait at most)
        let selected: HashSet<i64> = self.ed.selection().into_iter().map(|s| match s {
            Sel::Object(_, i) | Sel::Spline(_, i) | Sel::Attached(_, i) => i,
        }).collect();
        let mut files: HashSet<String> = HashSet::new();
        for (key, t) in &d.tiles {
            let org = tile_origin(*key);
            if !in_view(org + DVec2::splat(ts * 0.5), ts) {
                continue;
            }
            for o in &t.tile.objects {
                if files.len() < 400 && in_view(org + DVec2::new(o.pos[0], o.pos[1]), 0.0) {
                    files.insert(o.file.to_lowercase());
                }
            }
        }
        let start = Instant::now();
        for f in &files {
            let _ = self.assets.object(f);
        }
        while self.assets.pending > 0 && start.elapsed() < Duration::from_secs(4) {
            self.assets.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut models: HashMap<String, std::sync::Arc<crate::assets::ObjectModel>> = HashMap::new();
        for f in &files {
            if let Some(m) = self.assets.object(f) {
                models.insert(f.clone(), m);
            }
        }
        let d = self.ed.doc.as_ref().ok_or("no map")?;

        // splines: walls and fences, footpaths, roads, tracks, then AI paths
        let lanes = a.0.get("lanes").and_then(|g| g.as_bool()).unwrap_or(true);
        let mut runs: Vec<(u8, Vec<(f64, f64)>, f64, Category, bool, Option<DVec2>, i64)> = Vec::new();
        for (key, t) in &d.tiles {
            let org = tile_origin(*key);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                if !in_view(org + DVec2::new(s.pos[0], s.pos[1]), s.length) {
                    continue;
                }
                let asset = self.lib.find(&s.file);
                // (a file the editor made in this session is not in the catalogue yet: its model tells)
                let made = if asset.is_none() { self.assets.spline(&s.file) } else { None };
                let cat = asset.map(|a| a.category).or(made.as_ref().map(|m| if m.is_track() { Category::Tracks } else { Category::Roads })).unwrap_or(Category::Decoration);
                if cat == Category::Technical && !lanes {
                    continue;
                }
                let width = asset.map(|a| a.half_width).or(made.as_ref().map(|m| m.half_width)).map(|h| h as f64 * 2.0).unwrap_or(1.0);
                let curve = omsi_geometry::SplineCurve::from_map(s, org);
                let n = ((s.length * k / 3.0).ceil() as usize).clamp(2, 400);
                let pts: Vec<(f64, f64)> = (0..=n).map(|i| to_px(curve.point_at(s.length * i as f64 / n as f64).truncate())).collect();
                let order = match cat {
                    Category::Decoration => 0,
                    Category::Paths => 1,
                    Category::Roads => 2,
                    Category::Tracks => 3,
                    _ => 4,
                };
                let mid = curve.point_at(s.length * 0.5).truncate();
                runs.push((order, pts, width, cat, asset.is_none() && made.is_none(), Some(mid), s.id));
            }
        }
        runs.sort_by_key(|r| r.0);
        // (the roads' edges under all of them, so a junction of two reads as one surface)
        for (_, pts, wdt, cat, _, _, _) in &runs {
            if *cat == Category::Roads {
                if let Some(p) = line(pts) {
                    pm.stroke_path(&p, &paint(70, 70, 70, 255), &stroke((wdt * k) as f32 + 2.0, None), id, None);
                }
            }
        }
        for (_, pts, wdt, cat, missing, _, sid) in &runs {
            let Some(p) = line(pts) else { continue };
            let wpx = (wdt * k) as f32;
            match (missing, cat) {
                (true, _) => pm.stroke_path(&p, &paint(230, 40, 40, 255), &stroke(2.0, None), id, None),
                (_, Category::Roads) => pm.stroke_path(&p, &paint(150, 150, 150, 255), &stroke(wpx.max(2.0), None), id, None),
                (_, Category::Tracks) => {
                    pm.stroke_path(&p, &paint(120, 85, 60, 255), &stroke(wpx.max(2.0), None), id, None);
                    pm.stroke_path(&p, &paint(60, 40, 30, 255), &stroke(1.0, StrokeDash::new(vec![3.0, 3.0], 0.0)), id, None);
                }
                (_, Category::Paths) => pm.stroke_path(&p, &paint(222, 200, 160, 255), &stroke(wpx.max(1.5), None), id, None),
                (_, Category::Decoration) => pm.stroke_path(&p, &paint(70, 85, 70, 255), &stroke(wpx.clamp(1.0, 4.0), None), id, None),
                _ => pm.stroke_path(&p, &paint(200, 60, 200, 200), &stroke(1.0, StrokeDash::new(vec![4.0, 3.0], 0.0)), id, None),
            }
            if selected.contains(sid) {
                pm.stroke_path(&p, &paint(255, 220, 0, 160), &stroke(wpx.max(2.0) + 4.0, None), id, None);
            }
        }

        // objects: footprints, trees, stops, start points
        let entries: HashMap<i64, String> = d.global.entry_points.iter().map(|e| (e.object_id, e.name.clone())).collect();
        let mut labels: Vec<(f64, f64, String, (u8, u8, u8), f32)> = Vec::new();
        let ids = a.0.get("ids").and_then(|g| g.as_bool()).unwrap_or(false);
        let mut id_labels: Vec<(f64, f64, String)> = Vec::new();
        for (key, t) in &d.tiles {
            let org = tile_origin(*key);
            if !in_view(org + DVec2::splat(ts * 0.5), ts) {
                continue;
            }
            for o in &t.tile.objects {
                let at = org + DVec2::new(o.pos[0], o.pos[1]);
                if !in_view(at, 0.0) {
                    continue;
                }
                let (px, py) = to_px(at);
                let asset = self.lib.find(&o.file);
                let cat = asset.map(|a| a.category);
                let sel = selected.contains(&o.id);
                if ids {
                    id_labels.push((px, py, o.id.to_string()));
                }
                if entries.contains_key(&o.id) || asset.map(|a| a.is_entry_point).unwrap_or(false) {
                    let f = crate::roads::dir(o.rot[0]);
                    let r = crate::roads::right(o.rot[0]);
                    let s = 7.0;
                    let tip = (px + f.x * s, py - f.y * s);
                    let l = (px - f.x * s * 0.6 - r.x * s * 0.6, py + f.y * s * 0.6 + r.y * s * 0.6);
                    let rr = (px - f.x * s * 0.6 + r.x * s * 0.6, py + f.y * s * 0.6 - r.y * s * 0.6);
                    if let Some(p) = line(&[tip, l, rr, tip]) {
                        pm.fill_path(&p, &paint(220, 30, 30, 255), FillRule::Winding, id, None);
                    }
                    labels.push((px + 8.0, py + 4.0, entries.get(&o.id).cloned().unwrap_or_else(|| "start".into()), (160, 20, 20), 11.0));
                    continue;
                }
                if asset.map(|a| a.is_bus_stop).unwrap_or(false) {
                    if let Some(r) = Rect::from_xywh(px as f32 - 4.0, py as f32 - 4.0, 8.0, 8.0) {
                        pm.fill_rect(r, &paint(30, 90, 220, 255), id, None);
                    }
                    if let Some(n) = o.extra.first().filter(|n| !n.is_empty()) {
                        labels.push((px + 6.0, py - 5.0, n.clone(), (20, 50, 150), 12.0));
                    }
                    continue;
                }
                if cat == Some(Category::Trees) {
                    let r = (2.5 * k).clamp(1.5, 12.0) as f32;
                    if let Some(c) = PathBuilder::from_circle(px as f32, py as f32, r) {
                        pm.fill_path(&c, &paint(40, 110, 45, 220), FillRule::Winding, id, None);
                    }
                    continue;
                }
                let model = models.get(&o.file.to_lowercase());
                let surface = model.map(|m| m.surface).unwrap_or(false) || cat == Some(Category::Junctions);
                let (fill, edge) = match (cat.or(model.map(|_| Category::Objects)), surface) {
                    (_, true) => ((150, 150, 150, 255), (90, 90, 90, 255)),
                    (None, _) => ((230, 40, 40, 255), (120, 0, 0, 255)),
                    _ => ((178, 140, 120, 255), (90, 60, 50, 255)),
                };
                // (the object's frame as the view places it)
                let m = crate::render::object_matrix(glam::DVec3::ZERO, o.rot);
                let world = |v: glam::Vec3| to_px(at + m.transform_point3(v).truncate().as_dvec2());
                let foot = model.map(|m| [m.lo.x, m.lo.y, m.hi.x, m.hi.y]).unwrap_or([-1.0, -1.0, 1.0, 1.0]);
                let big = ((foot[2] - foot[0]) as f64 * k).max((foot[3] - foot[1]) as f64 * k) >= 3.0;
                let mut outline = None;
                if let (true, true, Some(model)) = (big, surface, model) {
                    // a plate on the ground: its own triangles, seen from above
                    let mut pb = PathBuilder::new();
                    for part in &model.parts {
                        let ps = &part.mesh.positions;
                        for t in part.mesh.indices.chunks_exact(3) {
                            let (Some(a0), Some(b0), Some(c0)) = (ps.get(t[0] as usize), ps.get(t[1] as usize), ps.get(t[2] as usize)) else { continue };
                            let (a0, mut b0, mut c0) = (world(*a0), world(*b0), world(*c0));
                            // (all one way round: a face and its back would cancel out)
                            if (b0.0 - a0.0) * (c0.1 - a0.1) - (b0.1 - a0.1) * (c0.0 - a0.0) < 0.0 {
                                std::mem::swap(&mut b0, &mut c0);
                            }
                            pb.move_to(a0.0 as f32, a0.1 as f32);
                            pb.line_to(b0.0 as f32, b0.1 as f32);
                            pb.line_to(c0.0 as f32, c0.1 as f32);
                            pb.close();
                        }
                    }
                    if let Some(p) = pb.finish() {
                        pm.fill_path(&p, &paint(fill.0, fill.1, fill.2, fill.3), FillRule::Winding, id, None);
                        outline = Some(p);
                    }
                } else if big {
                    let corner = |x: f32, y: f32| world(glam::Vec3::new(x, y, 0.0));
                    let pts = [corner(foot[0], foot[1]), corner(foot[2], foot[1]), corner(foot[2], foot[3]), corner(foot[0], foot[3]), corner(foot[0], foot[1])];
                    if let Some(p) = line(&pts) {
                        pm.fill_path(&p, &paint(fill.0, fill.1, fill.2, fill.3), FillRule::Winding, id, None);
                        pm.stroke_path(&p, &paint(edge.0, edge.1, edge.2, edge.3), &stroke(1.0, None), id, None);
                        outline = Some(p);
                    }
                } else if let Some(c) = PathBuilder::from_circle(px as f32, py as f32, 2.0) {
                    pm.fill_path(&c, &paint(edge.0, edge.1, edge.2, 255), FillRule::Winding, id, None);
                    outline = Some(c);
                }
                if let (true, Some(p)) = (sel, outline) {
                    pm.stroke_path(&p, &paint(255, 220, 0, 230), &stroke(3.0, None), id, None);
                }
            }
        }
        if ids {
            for (_, _, _, _, _, mid, sid) in &runs {
                if let Some(m) = mid {
                    let (x, y) = to_px(*m);
                    id_labels.push((x, y, sid.to_string()));
                }
            }
            if id_labels.len() <= 600 {
                for (x, y, s) in &id_labels {
                    text(&mut pm, s, *x as f32 + 3.0, *y as f32 - 3.0, 9.5, (0, 0, 0));
                }
            }
        }
        for (x, y, s, c, px) in &labels {
            text(&mut pm, s, *x as f32, *y as f32, *px, *c);
        }
        // a scale bar
        let bar = nice(span.x / 6.0);
        let bl = (bar * k) as f32;
        if let Some(r) = Rect::from_xywh(10.0, h as f32 - 18.0, bl, 5.0) {
            pm.fill_rect(r, &paint(20, 20, 20, 255), id, None);
        }
        text(&mut pm, &format!("{bar} m"), 14.0 + bl, h as f32 - 12.0, 12.0, (20, 20, 20));

        // as a PNG
        let mut img = image::RgbImage::new(w, h);
        for (o, p) in img.pixels_mut().zip(pm.data().chunks_exact(4)) {
            *o = image::Rgb([p[0], p[1], p[2]]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let meta = json!({
            "min": p2(lo), "max": p2(hi), "pixels": [w, h], "metres_per_pixel": number(1.0 / k),
            "pixel_of_a_point": "px = (x - min.x) / metres_per_pixel, py = (max.y - y) / metres_per_pixel (north is up)",
            "ground_heights": [number(zmin.min(zmax) as f64), number(zmax.max(zmin) as f64)],
            "legend": "grey bands: roads at their width; brown: tracks; beige: footpaths; dark green lines: walls, fences; dashed magenta: AI paths; brown-red boxes: buildings / objects at their footprint; grey boxes: junction plates, surfaces; green dots: trees; blue squares: bus stops (named); red triangles: start points (pointing the way); red: files missing; yellow: the selection; dashed squares: tile edges; blue ground: water.",
        });
        Ok(Out::Image { png, meta })
    }
}

impl Args<'_> {
    pub fn u32_or(&self, k: &str, d: u32) -> u32 {
        self.f64(k).map(|v| v.max(0.0) as u32).unwrap_or(d)
    }
}
