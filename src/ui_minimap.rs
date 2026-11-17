//! The overview map in the corner of the view: the tiles, the roads and where the camera
//! is. A click or drag on it moves the camera there.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use glam::DVec2;

use crate::doc::{tile_origin, tile_size, Document};
use crate::i18n::tr;
use crate::library::{Category, Library};
use crate::theme::pal;

/// What the camera shows, for the marker.
#[derive(Clone, Copy)]
pub struct CamView {
    pub yaw: f64,
    pub distance: f64,
    /// Where the eye is over the ground.
    pub eye: DVec2,
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    Road,
    Track,
    Path,
    Other,
}

/// The map's lines in world metres, built again only when a tile changes.
#[derive(Default)]
pub struct Minimap {
    pub open: bool,
    pub large: bool,
    key: u64,
    tiles: Vec<DVec2>,
    /// Satellite or own pictures laid on tiles: a tile's corner, a small copy, its opacity
    /// (`ui_geo::GeoUi::minimap_tiles`).
    pub imagery: Vec<(DVec2, egui::TextureId, f32)>,
    lines: Vec<(Line, Vec<DVec2>)>,
    /// The lines drawn into a small picture, once (the whole map is tens of thousands of
    /// splines: as shapes they were tessellated again in every frame) and the key it was made
    /// for.
    raster: Option<(u64, egui::TextureHandle)>,
    /// How many times the lines have changed: the picture is made again only then, not when
    /// just the map's key changed (an object moved, but no road did).
    lines_rev: u64,
    min: DVec2,
    max: DVec2,
    /// When the lines were built: a thumbnail of a map of hundreds of tiles is not made
    /// again on every frame of a drag (a thing moving changes its tile, and with it the key).
    built: Option<std::time::Instant>,
}

fn kind_of(lib: &Library, file: &str) -> Option<Line> {
    match lib.find(file).map(|a| a.category) {
        Some(Category::Roads) => Some(Line::Road),
        Some(Category::Tracks) => Some(Line::Track),
        Some(Category::Paths) => Some(Line::Path),
        // AI-only splines and helpers would only clutter it
        Some(Category::Technical) => None,
        _ => Some(Line::Other),
    }
}

/// A dot of the lines' width, at a place in the picture (clipped: a line off the map's edge
/// costs nothing).
fn dot(img: &mut egui::ColorImage, x: f32, y: f32, r: f32, c: Color32) {
    let (w, h) = (img.size[0] as i32, img.size[1] as i32);
    let (cx, cy) = (x.round() as i32, y.round() as i32);
    let ri = r.ceil() as i32;
    for dy in -ri..=ri {
        let py = cy + dy;
        if py < 0 || py >= h {
            continue;
        }
        for dx in -ri..=ri {
            let pxi = cx + dx;
            if pxi < 0 || pxi >= w {
                continue;
            }
            if ((dx * dx + dy * dy) as f32) > r * r + 0.25 {
                continue;
            }
            img.pixels[py as usize * img.size[0] + pxi as usize] = c;
        }
    }
}

/// A line between two places in the picture, a dot each pixel of it (a stroke's own way, without
/// the tessellator's): the later lines lie over the earlier ones, as the shapes did.
fn line_into(img: &mut egui::ColorImage, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: Color32) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        dot(img, x0 + dx * t, y0 + dy * t, r, c);
    }
}

impl Minimap {
    pub fn new() -> Minimap {
        Minimap { open: true, ..Default::default() }
    }

    fn rebuild(&mut self, doc: &Document, lib: &Library, moving: bool) {
        let key = doc.tiles.iter().fold(doc.tiles.len() as u64, |h, (k, t)| {
            h.wrapping_mul(31).wrapping_add(t.content_rev).wrapping_add((k.0 as u64) << 20 ^ k.1 as u64)
        }) ^ (lib.assets.len() as u64) << 40;
        if key == self.key && !self.tiles.is_empty() {
            return;
        }
        // (while things are being moved the thumbnail is left as it was, and made again once
        // they are let go: every frame of a move of thousands drew all of the map anew)
        if moving && !self.tiles.is_empty() {
            return;
        }
        // (while something moves - a grab, a drag, models still coming in - the map changes
        // with every frame; the thumbnail is made again a few times a second, not sixty)
        if let Some(at) = self.built {
            if at.elapsed() < std::time::Duration::from_millis(300) {
                return;
            }
        }
        self.built = Some(std::time::Instant::now());
        self.key = key;
        self.tiles.clear();
        let mut lines: Vec<(Line, Vec<DVec2>)> = Vec::new();
        let ts = tile_size();
        let (mut min, mut max) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for (&k, t) in &doc.tiles {
            let o = tile_origin(k);
            self.tiles.push(o);
            min = min.min(o);
            max = max.max(o + DVec2::splat(ts));
            for s in &t.tile.splines {
                let Some(kind) = kind_of(lib, &s.file) else { continue };
                let c = omsi_geometry::SplineCurve::from_map(s, o);
                // straight ones need their ends only; bends a point every ~8 m
                let n = if s.radius == 0.0 { 1 } else { (s.length / 8.0).ceil().clamp(2.0, 32.0) as usize };
                let pts = (0..=n).map(|i| c.point_at(s.length * i as f64 / n as f64).truncate()).collect();
                lines.push((kind, pts));
            }
        }
        // junction plates (openMapper's and stock crossing pieces: objects roads snap to),
        // drawn as their roads carried on to the middle
        let mut ends: Vec<(DVec2, f64, Line)> = Vec::new();
        for (&k, t) in &doc.tiles {
            let o = tile_origin(k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                let Some(kind @ (Line::Road | Line::Path)) = kind_of(lib, &s.file) else { continue };
                let c = omsi_geometry::SplineCurve::from_map(s, o);
                if s.prev_id == 0 {
                    ends.push((c.point_at(0.0).truncate(), c.heading_at(0.0) + 180.0, kind));
                }
                if s.next_id == 0 {
                    ends.push((c.end_point().truncate(), c.heading_at(s.length), kind));
                }
            }
        }
        for (&k, t) in &doc.tiles {
            let org = tile_origin(k);
            for ob in &t.tile.objects {
                let plate = crate::editor::is_own_junction(&ob.file) || lib.find(&ob.file).map(|a| !a.helpers.is_empty()).unwrap_or(false);
                if !plate {
                    continue;
                }
                let c = org + DVec2::new(ob.pos[0], ob.pos[1]);
                for (p, h, kind) in &ends {
                    let to = c - *p;
                    let d = to.length();
                    // (an end that runs into the plate: near it and heading for its middle)
                    let off = ((to.x.atan2(to.y).to_degrees() - h + 540.0).rem_euclid(360.0) - 180.0).abs();
                    if d < 35.0 && (d < 1.0 || off < 45.0) {
                        lines.push((*kind, vec![*p, c]));
                    }
                }
            }
        }
        // roads on top
        lines.sort_by_key(|(k, _)| match k {
            Line::Other => 0,
            Line::Path => 1,
            Line::Track => 2,
            Line::Road => 3,
        });
        // (the picture is drawn again only when a line of it changed: a thing moving about
        // changes its tile, and that would else redraw the whole thumbnail on every frame)
        if lines != self.lines {
            self.lines = lines;
            self.lines_rev = self.lines_rev.wrapping_add(1);
        }
        self.min = min;
        self.max = max;
    }

    /// What the lines' picture was made for: the map as built, where the map lies on the
    /// screen (a resized window, the larger map) and the screen's pixels a point.
    fn raster_key(&self, rect: Rect, ppp: f32) -> u64 {
        let mut h = self.lines_rev.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for v in [rect.min.x, rect.min.y, rect.width(), rect.height(), ppp * 100.0] {
            h = h.rotate_left(7) ^ (v as i32 as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        h
    }

    /// The lines into a picture of the map's rect, each as its own colour and width, a pixel of
    /// the picture a pixel of the screen (a point is two of them on a sharp screen).
    fn raster_lines(&self, rect: Rect, ppp: f32, to_screen: &impl Fn(DVec2) -> Pos2) -> egui::ColorImage {
        let w = ((rect.width() * ppp).ceil() as usize).clamp(1, 1024);
        let h = ((rect.height() * ppp).ceil() as usize).clamp(1, 1024);
        let sx = w as f32 / rect.width().max(1.0);
        let sy = h as f32 / rect.height().max(1.0);
        // (a pixel of the picture, from a place on the screen)
        let px = |p: Pos2| ((p.x - rect.min.x) * sx, (p.y - rect.min.y) * sy);
        let mut img = egui::ColorImage { size: [w, h], pixels: vec![Color32::TRANSPARENT; w * h], source_size: egui::Vec2::new(w as f32, h as f32) };
        for (kind, pts) in &self.lines {
            let (width, c): (f32, Color32) = match kind {
                Line::Road => (1.6, Color32::from_rgb(205, 205, 210)),
                Line::Track => (1.0, Color32::from_rgb(170, 130, 90)),
                Line::Path => (0.8, Color32::from_rgb(120, 125, 120)),
                Line::Other => (0.6, Color32::from_rgb(90, 95, 90)),
            };
            let r = (width * 0.5 * sx.min(sy)).max(0.5);
            let mut prev: Option<(f32, f32)> = None;
            for &p in pts {
                let (x, y) = px(to_screen(p));
                if let Some((x0, y0)) = prev {
                    line_into(&mut img, x0, y0, x, y, r, c);
                }
                prev = Some((x, y));
            }
        }
        img
    }

    /// Draws it in the view's corner; returns where to move the camera to.
    pub fn show(&mut self, ui: &mut egui::Ui, view: Rect, doc: &Document, lib: &Library, cam: CamView, moving: bool) -> Option<DVec2> {
        if !self.open || doc.tiles.is_empty() {
            return None;
        }
        self.rebuild(doc, lib, moving);
        let span = (self.max - self.min).max(DVec2::splat(1.0));
        let side: f32 = if self.large { 360.0 } else { 200.0 };
        let side = side.min(view.width() * 0.45).min(view.height() * 0.6);
        let scale = side as f64 / span.x.max(span.y);
        let size = Vec2::new((span.x * scale) as f32, (span.y * scale) as f32);
        let rect = Rect::from_min_size(Pos2::new(view.right() - size.x - 12.0, view.bottom() - size.y - 12.0), size);
        let to_screen = |p: DVec2| Pos2::new(rect.left() + ((p.x - self.min.x) * scale) as f32, rect.bottom() - ((p.y - self.min.y) * scale) as f32);
        let to_world = |p: Pos2| DVec2::new(self.min.x + (p.x - rect.left()) as f64 / scale, self.min.y + (rect.bottom() - p.y) as f64 / scale);

        let resp = ui.interact(rect.expand(4.0), ui.id().with("minimap"), Sense::click_and_drag());
        let painter = ui.painter_at(rect.expand(6.0));
        painter.rect_filled(rect.expand(4.0), CornerRadius::same(3), pal().overlay);
        let ts = tile_size();
        for &o in &self.tiles {
            let r = Rect::from_two_pos(to_screen(o), to_screen(o + DVec2::splat(ts)));
            painter.rect_filled(r.shrink(0.5), CornerRadius::ZERO, Color32::from_rgb(52, 62, 48));
        }
        for &(o, tex, a) in &self.imagery {
            let r = Rect::from_two_pos(to_screen(o), to_screen(o + DVec2::splat(ts)));
            painter.image(tex, r.shrink(0.5), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha((a * 255.0) as u8));
        }
        // (the whole map's roads, drawn into a small picture once: tens of thousands of
        // splines laid out as shapes are tessellated again in every frame)
        let ppp = ui.ctx().pixels_per_point();
        let raster_key = self.raster_key(rect, ppp);
        if self.raster.as_ref().map(|(k, _)| *k) != Some(raster_key) {
            let img = self.raster_lines(rect, ppp, &to_screen);
            self.raster = Some((raster_key, ui.ctx().load_texture("minimap lines", img, egui::TextureOptions::LINEAR)));
        }
        if let Some((_, tex)) = &self.raster {
            painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        // the camera: where the eye is and which way it looks, the cone as long as the view
        // reaches (it grows with the height)
        let at = to_screen(cam.eye);
        let d = crate::roads::dir(cam.yaw);
        let fwd = Vec2::new(d.x as f32, -d.y as f32);
        let reach = ((cam.distance * 1.2 * scale) as f32).clamp(8.0, side * 0.45);
        let side_v = Vec2::new(-fwd.y, fwd.x) * reach * 0.55;
        let accent = Color32::from_rgb(232, 120, 40);
        painter.add(Shape::convex_polygon(vec![at, at + fwd * reach + side_v, at + fwd * reach - side_v], Color32::from_rgba_unmultiplied(232, 120, 40, 50), Stroke::new(1.0, accent)));
        painter.circle_filled(at, 3.5, accent);
        painter.rect_stroke(rect.expand(4.0), CornerRadius::same(3), Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
        let resp = resp.on_hover_text(tr("Overview: click or drag to move there · double-click: larger / smaller · M: hide", "Обзор: клик или перетаскивание: переместиться · двойной клик: крупнее / мельче · M: скрыть"));
        if resp.double_clicked() {
            self.large = !self.large;
            return None;
        }
        if resp.clicked() || resp.dragged() {
            return resp.interact_pointer_pos().map(to_world);
        }
        None
    }
}
