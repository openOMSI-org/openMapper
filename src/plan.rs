//! A plan of the map as an SVG picture (File > Export a plan of the map): its tiles, water,
//! roads, tracks and footpaths seen from above, the bus lines' ways in colour with a key,
//! the stops with their names, a scale bar and north. To share with players or print.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DVec2, DVec3};

use crate::doc::{tile_origin, tile_size, Document};
use crate::i18n::tr;
use crate::library::{Category, Library};

/// The colours of the lines, in turn.
const LINE_COLOURS: [&str; 10] = ["#d7263d", "#1b78c8", "#2e933c", "#f08a24", "#7b2cbf", "#0f7173", "#c2185b", "#5c6bc0", "#8d6e63", "#b8a200"];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A polyline as an SVG path (`flip`: world to picture).
fn path_d(pts: &[DVec2], flip: &impl Fn(DVec2) -> (f64, f64)) -> String {
    let mut d = String::new();
    for (i, p) in pts.iter().enumerate() {
        let (x, y) = flip(*p);
        let _ = write!(d, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { " L" });
    }
    d
}

/// The map's plan; `roots`: where the stops' models are found (the lines' ways go by them).
pub fn svg(doc: &Document, lib: &Library, roots: &[PathBuf]) -> String {
    let ts = tile_size();
    let (lo, hi) = doc.tiles.keys().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(a, b), k| (a.min(tile_origin(*k)), b.max(tile_origin(*k) + DVec2::splat(ts))));
    if doc.tiles.is_empty() {
        return String::new();
    }
    let margin = 40.0;
    let (w, h) = (hi.x - lo.x + 2.0 * margin, hi.y - lo.y + 2.0 * margin);
    // (north up: the picture's y runs down)
    let flip = |p: DVec2| (p.x - lo.x + margin, hi.y - p.y + margin);
    let font = (w.max(h) / 160.0).clamp(5.0, 40.0);
    let mut s = String::new();
    // (some 2400 pixels along its longer side when opened as a picture)
    let px = 2400.0 / w.max(h);
    let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0} {h:.0}" width="{:.0}" height="{:.0}" font-family="Helvetica, Arial, sans-serif">"#, w * px, h * px);
    let _ = writeln!(s, r##"<rect width="100%" height="100%" fill="#f7f6f2"/>"##);
    // the tiles
    s.push_str(r##"<g fill="#e7ecdc" stroke="#d6dccb" stroke-width="0.5">"##);
    for k in doc.tiles.keys() {
        let (x, y) = flip(tile_origin(*k) + DVec2::new(0.0, ts));
        let _ = write!(s, r#"<rect x="{x:.1}" y="{y:.1}" width="{ts:.1}" height="{ts:.1}"/>"#);
    }
    s.push_str("</g>\n");
    // water: where a tile's ground lies below its water, in 5 m cells
    s.push_str(r##"<g fill="#a9cfe8">"##);
    let cell = 5.0;
    let n = (ts / cell).round() as usize;
    for (k, t) in &doc.tiles {
        let Some(wl) = t.water else { continue };
        let level = (wl[0] + wl[1] + wl[2] + wl[3]) / 4.0;
        let org = tile_origin(*k);
        for j in 0..n {
            let mut run: Option<usize> = None;
            for i in 0..=n {
                let wet = i < n && t.height_at((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell) < level;
                match (wet, run) {
                    (true, None) => run = Some(i),
                    (false, Some(a)) => {
                        let (x, y) = flip(org + DVec2::new(a as f64 * cell, (j + 1) as f64 * cell));
                        let _ = write!(s, r#"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}"/>"#, (i - a) as f64 * cell, cell + 0.2);
                        run = None;
                    }
                    _ => {}
                }
            }
        }
    }
    s.push_str("</g>\n");
    // the splines by their kind
    let mut roads: Vec<(Vec<DVec2>, f64)> = Vec::new();
    let mut tracks: Vec<Vec<DVec2>> = Vec::new();
    let mut paths: Vec<Vec<DVec2>> = Vec::new();
    let mut lines_of = |pts: Vec<DVec2>, file: &str| match lib.find(file).map(|a| (a.category, a.half_width)) {
        Some((Category::Roads, hw)) => roads.push((pts, (hw as f64).max(2.0))),
        Some((Category::Tracks, _)) => tracks.push(pts),
        Some((Category::Paths, _)) => paths.push(pts),
        _ => {}
    };
    for (k, t) in &doc.tiles {
        for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
            let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(*k));
            let m = (sp.length / 4.0).ceil().max(1.0) as usize;
            lines_of((0..=m).map(|i| c.point_at(sp.length * i as f64 / m as f64).truncate()).collect(), &sp.file);
        }
    }
    let _ = writeln!(s, r##"<g fill="none" stroke="#c8b98f" stroke-width="1.2" stroke-linecap="round">{}</g>"##, paths.iter().map(|p| format!(r#"<path d="{}"/>"#, path_d(p, &flip))).collect::<String>());
    // (each road its own width: an outline under, the road over it)
    let mut outline = String::new();
    let mut fill = String::new();
    for (p, hw) in &roads {
        let d = path_d(p, &flip);
        let _ = write!(outline, r#"<path d="{d}" stroke-width="{:.1}"/>"#, hw * 2.0 + 1.6);
        let _ = write!(fill, r#"<path d="{d}" stroke-width="{:.1}"/>"#, hw * 2.0);
    }
    let _ = writeln!(s, r##"<g fill="none" stroke="#a7a59d" stroke-linecap="round" stroke-linejoin="round">{outline}</g>"##);
    let _ = writeln!(s, r##"<g fill="none" stroke="#ffffff" stroke-linecap="round" stroke-linejoin="round">{fill}</g>"##);
    let _ = writeln!(s, r##"<g fill="none" stroke="#5a5a5a" stroke-width="1.6" stroke-dasharray="6 3">{}</g>"##, tracks.iter().map(|p| format!(r#"<path d="{}"/>"#, path_d(p, &flip))).collect::<String>());
    // the lines' ways (each trip's way along the roads, as the Lines window finds it)
    let tt = &doc.timetable;
    let mut key: Vec<(String, &str)> = Vec::new();
    if !tt.trips.is_empty() {
        let mut files = crate::timetable::network::Files::new(roots.to_vec());
        let net = crate::timetable::network::Network::build(doc, &mut files);
        let mut ways = String::new();
        for (li, line) in tt.lines.iter().enumerate() {
            let colour = LINE_COLOURS[li % LINE_COLOURS.len()];
            let mut drawn = false;
            for trip in tt.trips.iter().filter(|t| t.line.eq_ignore_ascii_case(&line.name) && t.stations.len() >= 2) {
                let pos: Vec<Option<DVec3>> = trip.stations.iter().map(|id| crate::timetable::stop_world(doc, &mut files, *id)).collect();
                let r = crate::timetable::route::find(&net, &pos, doc.global.left_hand_traffic, crate::timetable::route::ways_of(&doc.timetable, &trip.name, &net, &doc.order));
                let pts: Vec<DVec2> = r.polyline(&net).iter().map(|p| p.truncate()).collect();
                if pts.len() >= 2 {
                    let _ = write!(ways, r#"<path d="{}" stroke="{colour}"/>"#, path_d(&pts, &flip));
                    drawn = true;
                }
            }
            if drawn {
                key.push((line.name.clone(), colour));
            }
        }
        let _ = writeln!(s, r#"<g fill="none" stroke-width="{:.1}" stroke-opacity="0.75" stroke-linecap="round" stroke-linejoin="round">{ways}</g>"#, (font * 0.45).max(2.5));
    }
    // the stops, each name once over its stops (the two sides of a road)
    let stops: Vec<(String, DVec2)> = doc.tiles.iter().flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.flag == 7 && o.extra.len() >= 4).map(move |o| (o.extra[0].clone(), tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1])))).collect();
    let mut groups: Vec<(String, Vec<DVec2>)> = Vec::new();
    for (name, p) in &stops {
        match groups.iter_mut().find(|g| g.0 == *name && g.1.iter().any(|q| (*q - *p).length() < 80.0)) {
            Some(g) => g.1.push(*p),
            None => groups.push((name.clone(), vec![*p])),
        }
    }
    let r = (font * 0.35).max(2.0);
    s.push_str(r##"<g fill="#ffffff" stroke="#222222" stroke-width="0.8">"##);
    for (_, p) in &stops {
        let (x, y) = flip(*p);
        let _ = write!(s, r#"<circle cx="{x:.1}" cy="{y:.1}" r="{r:.1}"/>"#);
    }
    s.push_str("</g>\n");
    let _ = write!(s, r##"<g font-size="{font:.1}" fill="#222222" stroke="#f7f6f2" stroke-width="{:.1}" paint-order="stroke" text-anchor="middle">"##, font * 0.25);
    for (name, ps) in &groups {
        let c = ps.iter().copied().sum::<DVec2>() / ps.len() as f64;
        let (x, y) = flip(c);
        let _ = write!(s, r#"<text x="{x:.1}" y="{:.1}">{}</text>"#, y - r - font * 0.4, esc(name));
    }
    s.push_str("</g>\n");
    // the title, the lines' key, the scale and north
    let name = if doc.global.friendly_name.is_empty() { doc.global.name.clone() } else { doc.global.friendly_name.clone() };
    let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" font-size="{:.1}" font-weight="bold" fill="#222222">{}</text>"##, margin * 0.5, margin * 0.5 + font * 1.4, font * 1.6, esc(&name));
    for (i, (n, c)) in key.iter().enumerate() {
        let y = margin * 0.5 + font * (3.2 + 1.5 * i as f64);
        let _ = writeln!(s, r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{:.1}" fill="{c}"/><text x="{:.1}" y="{:.1}" font-size="{font:.1}" font-weight="bold" fill="#ffffff" text-anchor="middle">{}</text>"##, margin * 0.5, y - font, font * 2.6, font * 1.25, font * 0.25, margin * 0.5 + font * 1.3, y, esc(n));
    }
    // (a round length about a fifth of the picture)
    let bar = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0].into_iter().rfind(|b| *b <= (w - 2.0 * margin) / 5.0).unwrap_or(10.0);
    let (bx, by) = (w - margin * 0.5 - bar, h - margin * 0.5);
    let label = if bar >= 1000.0 { format!("{} km", bar / 1000.0) } else { format!("{bar} m") };
    let _ = writeln!(s, r##"<g stroke="#222222" stroke-width="{:.1}"><line x1="{bx:.1}" y1="{by:.1}" x2="{:.1}" y2="{by:.1}"/><line x1="{bx:.1}" y1="{:.1}" x2="{bx:.1}" y2="{by:.1}"/><line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{by:.1}"/></g><text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="#222222" text-anchor="middle">{label}</text>"##, font * 0.15, bx + bar, by - font * 0.4, bx + bar, by - font * 0.4, bx + bar, bx + bar / 2.0, by - font * 0.6);
    let (nx, ny) = (w - margin * 0.5 - font, margin * 0.5 + font * 2.5);
    let _ = writeln!(s, r##"<path d="M{nx:.1},{:.1} L{:.1},{:.1} L{nx:.1},{:.1} L{:.1},{:.1} Z" fill="#222222"/><text x="{nx:.1}" y="{:.1}" font-size="{font:.1}" font-weight="bold" fill="#222222" text-anchor="middle">N</text>"##, ny - font * 1.6, nx + font * 0.6, ny, ny - font * 0.4, nx - font * 0.6, ny, ny - font * 1.9);
    let _ = writeln!(s, r##"<text x="{:.1}" y="{:.1}" font-size="{:.1}" fill="#8a8a8a">{}</text>"##, margin * 0.5, h - margin * 0.5, font * 0.7, esc(tr("Made with openMapper", "Сделано в openMapper")));
    s.push_str("</svg>\n");
    s
}

/// `openmapper --plan path/to/global.cfg out.svg` (no window).
pub fn cli(cfg: &Path, out: &Path) -> i32 {
    let prefs = crate::install::Prefs::load();
    let Some(omsi) = crate::install::find_omsi_root(&prefs) else {
        eprintln!("no OMSI 2 installation found");
        return 2;
    };
    let roots = crate::install::Roots::new(omsi);
    let mut lib = Library::default();
    lib.start_scan(&roots);
    let doc = match Document::load(&roots, cfg, |_| {}) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", cfg.display());
            return 2;
        }
    };
    while lib.scanning {
        lib.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    match std::fs::write(out, svg(&doc, &lib, &omsi_cfg::content_roots())) {
        Ok(()) => {
            println!("{}", out.display());
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", out.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::flat_map;

    #[test]
    fn a_plan_names_each_pair_of_stops_once() {
        let mut doc = flat_map(2);
        doc.global.friendly_name = "Town & Country".into();
        let stop = |id: i64, x: f64| omsi_map::MapObject { file: "stop.sco".into(), id, pos: [x, 100.0, 0.0], flag: 7, extra: vec!["Rathaus <Mitte>".into(), "0".into(), "0".into(), "10".into()], ..Default::default() };
        let t = doc.tiles.get_mut(&(0, 0)).unwrap();
        t.tile.objects.extend([stop(1, 100.0), stop(2, 112.0)]);
        doc.tiles.get_mut(&(1, 0)).unwrap().water = Some([5.0; 4]);
        let s = svg(&doc, &Library::default(), &[]);
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert_eq!(s.matches("<circle").count(), 2);
        assert_eq!(s.matches("Rathaus &lt;Mitte&gt;").count(), 1);
        assert!(s.contains("Town &amp; Country"));
        // (the flat ground lies under the water all over its tile)
        assert!(s.contains("#a9cfe8"));
        assert!(s.matches("<rect").count() > 60);
    }
}
