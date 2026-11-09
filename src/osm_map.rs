//! The map as OpenStreetMap draws it, made from its data as a vector picture: streets with
//! their names, houses, water, green and the stops with a sign, laid on a tile like a
//! satellite picture. Sharp at any size (it is drawn at the picture's own resolution), and
//! nothing is built from it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use ab_glyph::{Font, FontRef};
use glam::DVec2;
use image::RgbImage;
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

use crate::doc::{tile_origin, TileKey};
use crate::geo::Georef;

type LatLon = (f64, f64);

/// What the data of a place holds that the picture shows.
#[derive(Default)]
pub struct Data {
    elems: Vec<Elem>,
}

struct Elem {
    tags: HashMap<String, String>,
    geom: Geom,
}

enum Geom {
    Node(LatLon),
    Way(Vec<LatLon>),
    /// A relation's ways with their roles (`outer`, `inner`).
    Rel(Vec<(bool, Vec<LatLon>)>),
}

impl Elem {
    fn tag(&self, k: &str) -> &str {
        self.tags.get(k).map(|s| s.as_str()).unwrap_or("")
    }
}

/// The Overpass query for everything the picture shows within a box (south, west, north, east).
pub fn query(bbox: [f64; 4]) -> String {
    let b = format!("{:.6},{:.6},{:.6},{:.6}", bbox[0], bbox[1], bbox[2], bbox[3]);
    format!(
        "[out:json][timeout:90];(\
         way[\"highway\"]({b});\
         way[\"railway\"~\"^(rail|tram|light_rail)$\"]({b});\
         way[\"building\"]({b});\
         way[\"natural\"~\"^(water|wood|scrub|grassland|wetland|heath)$\"]({b});\
         way[\"waterway\"~\"^(river|stream|canal|riverbank|drain|ditch)$\"]({b});\
         way[\"landuse\"~\"^(forest|residential|commercial|retail|industrial|grass|meadow|village_green|recreation_ground|cemetery|orchard|allotments|farmland|farmyard|reservoir|basin)$\"]({b});\
         way[\"leisure\"~\"^(park|garden|pitch|playground|sports_centre|recreation_ground|nature_reserve)$\"]({b});\
         way[\"amenity\"=\"parking\"]({b});\
         rel[\"natural\"~\"^(water|wood)$\"]({b});\
         rel[\"landuse\"~\"^(forest|residential|grass|meadow)$\"]({b});\
         rel[\"leisure\"~\"^(park|garden)$\"]({b});\
         rel[\"building\"]({b});\
         rel[\"waterway\"=\"riverbank\"]({b});\
         node[\"highway\"=\"bus_stop\"]({b});\
         node[\"public_transport\"=\"platform\"][\"bus\"=\"yes\"]({b});\
         node[\"railway\"=\"tram_stop\"]({b});\
         );out geom;"
    )
}

fn points(g: Option<&serde_json::Value>) -> Vec<LatLon> {
    g.and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect()
}

pub fn parse(body: &str) -> Result<Data, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut elems = Vec::new();
    for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
        let tags: HashMap<String, String> = e.get("tags").and_then(|t| t.as_object()).into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
        let geom = match e.get("type").and_then(|t| t.as_str()) {
            Some("node") => match (e.get("lat").and_then(|x| x.as_f64()), e.get("lon").and_then(|x| x.as_f64())) {
                (Some(la), Some(lo)) => Geom::Node((la, lo)),
                _ => continue,
            },
            Some("way") => {
                let p = points(e.get("geometry"));
                if p.len() < 2 {
                    continue;
                }
                Geom::Way(p)
            }
            Some("relation") => {
                let members: Vec<(bool, Vec<LatLon>)> = e
                    .get("members")
                    .and_then(|m| m.as_array())
                    .into_iter()
                    .flatten()
                    .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("way"))
                    .map(|m| (m.get("role").and_then(|r| r.as_str()) != Some("inner"), points(m.get("geometry"))))
                    .filter(|(_, p)| p.len() >= 2)
                    .collect();
                if members.is_empty() {
                    continue;
                }
                Geom::Rel(members)
            }
            _ => continue,
        };
        elems.push(Elem { tags, geom });
    }
    Ok(Data { elems })
}

/// Open pieces joined end to end into closed rings (a multipolygon's outline comes in pieces).
fn rings(pieces: Vec<Vec<LatLon>>) -> Vec<Vec<LatLon>> {
    let mut out = Vec::new();
    let mut left: Vec<Vec<LatLon>> = pieces;
    while let Some(mut cur) = left.pop() {
        loop {
            if cur.len() > 3 && cur.first() == cur.last() {
                break;
            }
            let end = *cur.last().unwrap();
            let Some(i) = left.iter().position(|p| p.first() == Some(&end) || p.last() == Some(&end)) else { break };
            let mut next = left.swap_remove(i);
            if next.first() != Some(&end) {
                next.reverse();
            }
            cur.extend(next.into_iter().skip(1));
        }
        if cur.len() > 3 && cur.first() == cur.last() {
            out.push(cur);
        }
    }
    out
}

fn rgb(h: u32) -> Color {
    Color::from_rgba8((h >> 16) as u8, (h >> 8) as u8, h as u8, 255)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn poly(pts: &[DVec2], close: bool) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(p.x as f32, p.y as f32);
        } else {
            pb.line_to(p.x as f32, p.y as f32);
        }
    }
    if close {
        pb.close();
    }
    pb.finish()
}

/// How an area is coloured, and the order it is drawn in (low first).
fn area_style(e: &Elem) -> Option<(u8, u32, Option<u32>)> {
    let (lu, nat, lei) = (e.tag("landuse"), e.tag("natural"), e.tag("leisure"));
    Some(match () {
        _ if matches!(nat, "water") || e.tag("waterway") == "riverbank" || matches!(lu, "reservoir" | "basin") => (4, 0xaad3df, None),
        _ if e.tags.contains_key("building") => (5, 0xd9d0c9, Some(0xbfb2a8)),
        _ if matches!(lu, "forest") || matches!(nat, "wood") => (2, 0xadd19e, None),
        _ if matches!(lu, "grass" | "meadow" | "village_green" | "recreation_ground" | "cemetery" | "orchard" | "allotments") || matches!(lei, "park" | "garden" | "recreation_ground" | "nature_reserve" | "playground") || matches!(nat, "scrub" | "grassland" | "heath" | "wetland") => (1, 0xcdebb0, None),
        _ if matches!(lei, "pitch" | "sports_centre") => (3, 0xaae0cb, Some(0x8ccfb3)),
        _ if e.tag("amenity") == "parking" => (3, 0xeeeeee, Some(0xd6d6d6)),
        _ if matches!(lu, "residential") => (0, 0xe0dfdf, None),
        _ if matches!(lu, "commercial" | "retail") => (0, 0xf2dad9, None),
        _ if matches!(lu, "industrial") => (0, 0xebdbe8, None),
        _ if matches!(lu, "farmland" | "farmyard") => (0, 0xeef0d5, None),
        _ => return None,
    })
}

/// A road class: (rank, fill, casing, width in metres, name's size in metres), None: not drawn.
fn road_style(h: &str) -> Option<(u8, u32, Option<u32>, f64, f64)> {
    Some(match h {
        "motorway" | "motorway_link" | "trunk" | "trunk_link" => (9, 0xe990a0, Some(0xdc2a67), 12.0, 3.4),
        "primary" | "primary_link" => (8, 0xfcd6a4, Some(0xc09a5a), 10.0, 3.2),
        "secondary" | "secondary_link" => (7, 0xf7fabf, Some(0xb3b36f), 9.0, 3.0),
        "tertiary" | "tertiary_link" => (6, 0xffffff, Some(0x9d9d9d), 8.0, 2.9),
        "residential" | "unclassified" | "living_street" => (5, 0xffffff, Some(0xb5b5b5), 6.0, 2.6),
        "service" => (4, 0xffffff, Some(0xbdbdbd), 3.2, 0.0),
        "pedestrian" => (3, 0xdddde8, Some(0x9a9aa8), 4.5, 2.4),
        _ => return None,
    })
}

/// A footpath, a track, a cycleway: thin and dashed.
fn path_style(h: &str) -> Option<u32> {
    match h {
        "footway" | "path" | "steps" | "bridleway" => Some(0xf8746b),
        "cycleway" => Some(0x3e63e6),
        "track" => Some(0x9c7a3c),
        _ => None,
    }
}

pub(crate) fn font() -> Option<&'static FontRef<'static>> {
    static F: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    F.get_or_init(|| {
        // (the interface's own font: it has Cyrillic as well as Latin)
        let defs: &'static egui::FontDefinitions = Box::leak(Box::new(egui::FontDefinitions::default()));
        let order = ["Ubuntu-Light", "NotoSans-Regular"];
        let data = order.iter().find_map(|n| defs.font_data.get(*n)).or_else(|| defs.font_data.values().next())?;
        FontRef::try_from_slice(&data.font).ok()
    })
    .as_ref()
}

/// A text as an outline at a size (px): its path with the baseline at y = 0, and its width.
pub(crate) fn text_path(f: &FontRef, s: &str, px: f32) -> Option<(Path, f32)> {
    let upm = f.units_per_em()?;
    let k = px / upm;
    let mut pb = PathBuilder::new();
    let mut pen = 0.0f32;
    for c in s.chars() {
        let id = f.glyph_id(c);
        if let Some(o) = f.outline(id) {
            let mut last: Option<(f32, f32)> = None;
            let at = |p: ab_glyph::Point| (pen + p.x * k, -p.y * k);
            for curve in &o.curves {
                let (start, end) = match *curve {
                    ab_glyph::OutlineCurve::Line(a, b) => (a, b),
                    ab_glyph::OutlineCurve::Quad(a, _, c) => (a, c),
                    ab_glyph::OutlineCurve::Cubic(a, _, _, d) => (a, d),
                };
                let s0 = at(start);
                if last != Some(s0) {
                    if last.is_some() {
                        pb.close();
                    }
                    pb.move_to(s0.0, s0.1);
                }
                match *curve {
                    ab_glyph::OutlineCurve::Line(_, b) => {
                        let b = at(b);
                        pb.line_to(b.0, b.1);
                    }
                    ab_glyph::OutlineCurve::Quad(_, b, c) => {
                        let (b, c) = (at(b), at(c));
                        pb.quad_to(b.0, b.1, c.0, c.1);
                    }
                    ab_glyph::OutlineCurve::Cubic(_, b, c, d) => {
                        let (b, c, d) = (at(b), at(c), at(d));
                        pb.cubic_to(b.0, b.1, c.0, c.1, d.0, d.1);
                    }
                }
                last = Some(at(end));
            }
            if last.is_some() {
                pb.close();
            }
        }
        pen += f.h_advance_unscaled(id) * k;
    }
    Some((pb.finish()?, pen))
}

struct Label {
    path: Path,
    at: Transform,
    px: f32,
    ink: Color,
}

/// Where labels are put, so that none lies on another.
#[derive(Default)]
struct Taken {
    boxes: Vec<[f32; 4]>,
    names: Vec<(String, DVec2)>,
}

impl Taken {
    fn free(&self, b: [f32; 4]) -> bool {
        self.boxes.iter().all(|t| b[2] < t[0] || b[0] > t[2] || b[3] < t[1] || b[1] > t[3])
    }
}

/// The map drawn: `to_px` gives a place's pixel, `ppm` the pixels to a metre.
pub fn draw(data: &Data, size: u32, to_px: &dyn Fn(LatLon) -> DVec2, ppm: f64, cancel: &AtomicBool) -> Result<RgbImage, String> {
    let mut pm = Pixmap::new(size, size).ok_or("picture too big")?;
    pm.fill(rgb(0xf2efe9));
    let px = |v: &[LatLon]| -> Vec<DVec2> { v.iter().map(|p| to_px(*p)).collect() };
    let id = Transform::identity();
    let stroke = |w: f64, cap: LineCap, dash: Option<StrokeDash>| Stroke { width: w as f32, line_cap: cap, line_join: LineJoin::Round, dash, ..Default::default() };

    // areas: land, green, water, houses
    let mut areas: Vec<(u8, u32, Option<u32>, Path)> = Vec::new();
    for e in &data.elems {
        let Some((order, fill, edge)) = area_style(e) else { continue };
        if e.tag("highway") != "" {
            continue;
        }
        let rs: Vec<Vec<LatLon>> = match &e.geom {
            Geom::Way(w) if w.len() > 3 && w.first() == w.last() => vec![w.clone()],
            Geom::Rel(m) => rings(m.iter().map(|(_, p)| p.clone()).collect()),
            _ => continue,
        };
        let mut pb = PathBuilder::new();
        for r in &rs {
            for (i, p) in px(r).iter().enumerate() {
                if i == 0 {
                    pb.move_to(p.x as f32, p.y as f32);
                } else {
                    pb.line_to(p.x as f32, p.y as f32);
                }
            }
            pb.close();
        }
        if let Some(p) = pb.finish() {
            areas.push((order, fill, edge, p));
        }
    }
    areas.sort_by_key(|a| a.0);
    for (_, fill, edge, p) in &areas {
        pm.fill_path(p, &paint(rgb(*fill)), FillRule::EvenOdd, id, None);
        if let Some(c) = edge {
            pm.stroke_path(p, &paint(rgb(*c)), &stroke((0.3 * ppm).max(1.0), LineCap::Butt, None), id, None);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }

    // waterways
    for e in &data.elems {
        let Geom::Way(w) = &e.geom else { continue };
        let width = match e.tag("waterway") {
            "river" => 6.0,
            "canal" => 5.0,
            "stream" | "drain" | "ditch" => 1.6,
            _ => continue,
        };
        if let Some(p) = poly(&px(w), false) {
            pm.stroke_path(&p, &paint(rgb(0xaad3df)), &stroke(width * ppm, LineCap::Round, None), id, None);
        }
    }

    // railways
    for e in &data.elems {
        let Geom::Way(w) = &e.geom else { continue };
        if e.tag("railway").is_empty() || e.tag("tunnel") == "yes" {
            continue;
        }
        let Some(p) = poly(&px(w), false) else { continue };
        let tram = e.tag("railway") != "rail";
        pm.stroke_path(&p, &paint(rgb(if tram { 0x999999 } else { 0x808080 })), &stroke((if tram { 1.0 } else { 1.7 }) * ppm, LineCap::Butt, None), id, None);
        if !tram {
            let dash = StrokeDash::new(vec![(4.0 * ppm) as f32, (4.0 * ppm) as f32], 0.0);
            pm.stroke_path(&p, &paint(rgb(0xffffff)), &stroke(0.8 * ppm, LineCap::Butt, dash), id, None);
        }
    }

    // footpaths, dashed
    for e in &data.elems {
        let Geom::Way(w) = &e.geom else { continue };
        let Some(c) = path_style(e.tag("highway")) else { continue };
        let Some(p) = poly(&px(w), false) else { continue };
        let dash = StrokeDash::new(vec![(1.6 * ppm) as f32, (1.0 * ppm) as f32], 0.0);
        pm.stroke_path(&p, &paint(rgb(c)), &stroke((0.8 * ppm).max(1.5), LineCap::Butt, dash), id, None);
    }

    // roads, the minor ones first, each class its casing and then its fill (a bridge over a road)
    let mut roads: Vec<(i8, u8, &Elem, (u8, u32, Option<u32>, f64, f64))> = Vec::new();
    for e in &data.elems {
        let Geom::Way(_) = &e.geom else { continue };
        let Some(s) = road_style(e.tag("highway")) else { continue };
        if e.tag("tunnel") == "yes" || e.tag("area") == "yes" {
            continue;
        }
        let layer = e.tag("layer").parse::<i8>().unwrap_or(if e.tag("bridge") == "yes" { 1 } else { 0 });
        roads.push((layer, s.0, e, s));
    }
    roads.sort_by_key(|r| (r.0, r.1));
    let mut i = 0;
    while i < roads.len() {
        let key = (roads[i].0, roads[i].1);
        let mut j = i;
        while j < roads.len() && (roads[j].0, roads[j].1) == key {
            j += 1;
        }
        for pass in 0..2 {
            for r in &roads[i..j] {
                let Geom::Way(w) = &r.2.geom else { continue };
                let Some(p) = poly(&px(w), false) else { continue };
                let (_, fill, casing, width, _) = r.3;
                if pass == 0 {
                    if let Some(c) = casing {
                        pm.stroke_path(&p, &paint(rgb(c)), &stroke(width * ppm + (1.2 * ppm).max(2.0), LineCap::Round, None), id, None);
                    }
                } else {
                    pm.stroke_path(&p, &paint(rgb(fill)), &stroke(width * ppm, LineCap::Round, None), id, None);
                }
            }
        }
        i = j;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }

    // labels: the stops first, then the streets round them
    let lim = size as f32;
    let mut taken = Taken::default();
    let mut labels: Vec<Label> = Vec::new();
    let mut icons: Vec<(DVec2, bool)> = Vec::new();
    for e in &data.elems {
        let Geom::Node(n) = e.geom else { continue };
        let tram = e.tag("railway") == "tram_stop";
        let c = to_px(n);
        let r = 1.9 * ppm;
        if c.x < -r || c.y < -r || c.x > lim as f64 + r || c.y > lim as f64 + r {
            continue;
        }
        icons.push((c, tram));
        taken.boxes.push([(c.x - r) as f32, (c.y - r) as f32, (c.x + r) as f32, (c.y + r) as f32]);
        let name = e.tag("name");
        if let (Some(f), false) = (font(), name.is_empty()) {
            let size_px = (2.5 * ppm) as f32;
            if let Some((path, w)) = text_path(f, name, size_px) {
                let (x, y) = (c.x as f32 + (r * 1.3) as f32, c.y as f32 + size_px * 0.35);
                let b = [x, y - size_px * 0.8, x + w, y + size_px * 0.2];
                if b[0] >= 0.0 && b[2] <= lim && b[1] >= 0.0 && b[3] <= lim && taken.free(b) {
                    taken.boxes.push(b);
                    labels.push(Label { path, at: Transform::from_translate(x, y), px: size_px, ink: rgb(0x0b4a9a) });
                }
            }
        }
    }
    if let Some(f) = font() {
        let mut named: Vec<(u8, &Elem, f64)> = Vec::new();
        for e in &data.elems {
            let Geom::Way(_) = &e.geom else { continue };
            let Some(s) = road_style(e.tag("highway")) else { continue };
            if s.4 > 0.0 && !e.tag("name").is_empty() && e.tag("tunnel") != "yes" {
                named.push((s.0, e, s.4));
            }
        }
        // (the big roads get their names first)
        named.sort_by_key(|n| std::cmp::Reverse(n.0));
        for (_, e, name_m) in named {
            let Geom::Way(w) = &e.geom else { continue };
            let pts = px(w);
            let size_px = (name_m * ppm) as f32;
            let Some((path, tw)) = text_path(f, e.tag("name"), size_px) else { continue };
            let mut start = 0usize;
            let mut placed = 0;
            while start + 1 < pts.len() && placed < 3 {
                // a stretch of the road as long as the name, going straight on
                let mut len = 0.0;
                let mut end = start;
                while end + 1 < pts.len() && len < tw as f64 * 1.15 {
                    len += (pts[end + 1] - pts[end]).length();
                    end += 1;
                }
                if len < tw as f64 * 1.15 {
                    break;
                }
                let (a, b) = (pts[start], pts[end]);
                let chord = (b - a).length();
                let straight = chord > len * 0.96;
                let mid = (a + b) * 0.5;
                let m = (tw * 0.6 + size_px) as f64;
                let inside = mid.x > m && mid.y > m && mid.x < lim as f64 - m && mid.y < lim as f64 - m;
                if straight && inside {
                    let mut ang = (b.y - a.y).atan2(b.x - a.x);
                    if ang > std::f64::consts::FRAC_PI_2 {
                        ang -= std::f64::consts::PI;
                    } else if ang < -std::f64::consts::FRAC_PI_2 {
                        ang += std::f64::consts::PI;
                    }
                    let (hw, hh) = (tw as f64 * 0.5 + 2.0, size_px as f64 * 0.5 + 2.0);
                    let (ex, ey) = (hw * ang.cos().abs() + hh * ang.sin().abs(), hw * ang.sin().abs() + hh * ang.cos().abs());
                    let bx = [(mid.x - ex) as f32, (mid.y - ey) as f32, (mid.x + ex) as f32, (mid.y + ey) as f32];
                    let name = e.tag("name").to_string();
                    let near = taken.names.iter().any(|(n, p)| *n == name && (*p - mid).length() < 60.0 * ppm);
                    if taken.free(bx) && !near {
                        taken.boxes.push(bx);
                        taken.names.push((name, mid));
                        let at = Transform::from_translate(mid.x as f32, mid.y as f32).pre_concat(Transform::from_rotate(ang.to_degrees() as f32)).pre_translate(-tw * 0.5, size_px * 0.35);
                        labels.push(Label { path: path.clone(), at, px: size_px, ink: rgb(0x303030) });
                        placed += 1;
                        // (the next one a few names further on)
                        let skip = tw as f64 * 3.0;
                        let mut run = 0.0;
                        while end + 1 < pts.len() && run < skip {
                            run += (pts[end + 1] - pts[end]).length();
                            end += 1;
                        }
                        start = end;
                        continue;
                    }
                }
                start += 1;
            }
        }
    }

    // the stops' signs, then the names on top of everything
    for (c, tram) in &icons {
        let r = 1.9 * ppm;
        let mut pb = PathBuilder::new();
        pb.push_circle(c.x as f32, c.y as f32, r as f32);
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &paint(rgb(0xffffff)), FillRule::Winding, id, None);
        }
        let mut pb = PathBuilder::new();
        pb.push_circle(c.x as f32, c.y as f32, (r * 0.84) as f32);
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &paint(rgb(if *tram { 0xd0021b } else { 0x1d6fc4 })), FillRule::Winding, id, None);
        }
        // (a bus or a tram seen from the front: a white body with a coloured windscreen)
        let (cx, cy, u) = (c.x as f32, c.y as f32, r as f32);
        let blue = rgb(if *tram { 0xd0021b } else { 0x1d6fc4 });
        if let Some(body) = tiny_skia::Rect::from_xywh(cx - 0.46 * u, cy - 0.5 * u, 0.92 * u, 0.95 * u) {
            pm.fill_path(&PathBuilder::from_rect(body), &paint(rgb(0xffffff)), FillRule::Winding, id, None);
        }
        if let Some(glass) = tiny_skia::Rect::from_xywh(cx - 0.34 * u, cy - 0.38 * u, 0.68 * u, 0.36 * u) {
            pm.fill_path(&PathBuilder::from_rect(glass), &paint(blue), FillRule::Winding, id, None);
        }
    }
    let halo = paint(rgb(0xffffff));
    for l in &labels {
        pm.stroke_path(&l.path, &halo, &Stroke { width: l.px * 0.28, line_join: LineJoin::Round, ..Default::default() }, l.at, None);
    }
    for l in &labels {
        pm.fill_path(&l.path, &paint(l.ink), FillRule::Winding, l.at, None);
    }

    let mut img = RgbImage::new(size, size);
    for (o, p) in img.pixels_mut().zip(pm.data().chunks_exact(4)) {
        o.0 = [p[0], p[1], p[2]];
    }
    Ok(img)
}

/// The south, west, north and east edges of some tiles together (a little wider).
pub fn bbox(geo: &Georef, tiles: &[TileKey], ts: f64) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for &k in tiles {
        let o = tile_origin(k);
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.0), (0.5, 1.0), (0.0, 0.5), (1.0, 0.5)] {
            let (la, lo) = geo.lat_lon(o + DVec2::new(x * ts, y * ts));
            b = [b[0].min(la), b[1].min(lo), b[2].max(la), b[3].max(lo)];
        }
    }
    let (dl, dn) = ((b[2] - b[0]) * 0.02, (b[3] - b[1]) * 0.02);
    [b[0] - dl, b[1] - dn, b[2] + dl, b[3] + dn]
}

/// What OpenStreetMap has within a box: one request for all the tiles of a job (kept a week
/// in the cache), not one for each.
pub fn fetch(bbox: [f64; 4]) -> Result<Data, String> {
    parse(&crate::geo::fetch::overpass(&query(bbox))?)
}

/// The picture of map tile `k`, `size` pixels a side, row 0 its north edge.
pub fn build(geo: &Georef, k: TileKey, ts: f64, size: u32, data: &Data, cancel: &AtomicBool) -> Result<RgbImage, String> {
    let o = tile_origin(k);
    let to_px = |p: LatLon| {
        let w = geo.world(p.0, p.1) - o;
        DVec2::new(w.x / ts * size as f64, (1.0 - w.y / ts) * size as f64)
    };
    draw(data, size, &to_px, size as f64 / ts, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"elements":[
      {"type":"way","tags":{"highway":"residential","name":"Ленина улица"},"geometry":[{"lat":0.0,"lon":0.0},{"lat":0.0,"lon":0.002}]},
      {"type":"way","tags":{"building":"yes"},"geometry":[{"lat":0.0003,"lon":0.0003},{"lat":0.0003,"lon":0.0006},{"lat":0.0006,"lon":0.0006},{"lat":0.0006,"lon":0.0003},{"lat":0.0003,"lon":0.0003}]},
      {"type":"way","tags":{"natural":"water"},"geometry":[{"lat":-0.0008,"lon":0.0},{"lat":-0.0008,"lon":0.0008},{"lat":-0.0004,"lon":0.0008},{"lat":-0.0004,"lon":0.0},{"lat":-0.0008,"lon":0.0}]},
      {"type":"node","lat":0.0002,"lon":0.0012,"tags":{"highway":"bus_stop","name":"Центр"}}]}"#;

    fn pic() -> RgbImage {
        let data = parse(BODY).unwrap();
        // 0.002° of longitude is about 222 m: 1 px = about 0.4 m at 600 px
        let to_px = |p: LatLon| DVec2::new(p.1 / 0.002 * 600.0, 300.0 - p.0 / 0.002 * 600.0);
        draw(&data, 600, &to_px, 600.0 / 222.0, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn the_map_is_drawn_with_its_roads_houses_water_and_stops() {
        let img = pic();
        let at = |x: u32, y: u32| img.get_pixel(x, y).0;
        // the land, a road (white), a house, a pond, a stop's blue sign
        assert_eq!(at(10, 100), [0xf2, 0xef, 0xe9]);
        assert_eq!(at(100, 300), [0xff, 0xff, 0xff]);
        assert_eq!(at(135, 165), [0xd9, 0xd0, 0xc9]);
        assert_eq!(at(100, 450), [0xaa, 0xd3, 0xdf]);
        let blue = (350..370).any(|x| (230..250).any(|y| at(x, y)[2] > 150 && at(x, y)[0] < 80));
        assert!(blue, "no sign of the stop");
    }

    #[test]
    fn the_streets_name_is_written_in_dark_ink_on_the_road() {
        let img = pic();
        let dark = (0..600).filter(|x| (270..330).any(|y| img.get_pixel(*x, y).0.iter().all(|c| *c < 160))).count();
        assert!(dark > 10, "{dark} dark columns");
    }
}
