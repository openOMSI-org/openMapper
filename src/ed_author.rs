//! Laying splines exactly as asked (the MCP authoring tools): a chain of one spline type through
//! given points, tangent-continuous everywhere (biarcs between the points, the headings at the
//! points taken from their neighbours or from the road ends it starts and finishes at), its
//! height following the ground or the ends it is joined to; one undo step.

use glam::{DVec2, DVec3};
use omsi_map::MapSpline;

use crate::doc::{tile_of, tile_origin, Document, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::roads::{self, Seg};

/// Two arcs from `p` heading `hp` to `q` arriving heading `hq`, however tight (a straight when
/// both headings run along the chord).
pub fn biarc_any(p: DVec2, hp: f64, q: DVec2, hq: f64) -> Option<Vec<Seg>> {
    let v = q - p;
    let len = v.length();
    if len < 0.05 {
        return None;
    }
    let chord = roads::heading_of(v);
    let off = |h: f64| (roads::norm_deg(h - chord + 180.0) - 180.0).abs();
    if off(hp) < 0.05 && off(hq) < 0.05 {
        return Some(vec![Seg { start: p, heading: chord, length: len, radius: 0.0 }]);
    }
    let (t1, t2) = (roads::dir(hp), roads::dir(hq));
    let t = t1 + t2;
    let denom = 2.0 * (1.0 - t1.dot(t2));
    let vt = v.dot(t);
    let d = if denom.abs() < 1e-9 {
        let k = v.dot(t2);
        if k.abs() < 1e-9 {
            return None;
        }
        v.dot(v) / (4.0 * k)
    } else {
        (-vt + (vt * vt + denom * v.dot(v)).sqrt()) / denom
    };
    if !(d > 0.0) || !d.is_finite() {
        return None;
    }
    let joint = (p + t1 * d + q - t2 * d) * 0.5;
    let a = roads::arc_to_any(p, hp, joint)?;
    let b = roads::arc_to_any(joint, a.end_heading(), q)?;
    let miss = (roads::norm_deg(b.end_heading() - hq + 180.0) - 180.0).abs();
    (miss <= 1.0).then(|| vec![a, b].into_iter().filter(|s| s.length > 0.01).collect())
}

/// A tangent-continuous chain through `points`: the heading at each inner point halfway between
/// the ways to its neighbours (weighted by the distances), at the ends as given or towards the
/// neighbour. Points closer than 0.3 m to the one before are left out.
pub fn chain_through(points: &[DVec2], h0: Option<f64>, h1: Option<f64>) -> Result<Vec<Seg>, String> {
    let mut p: Vec<DVec2> = Vec::new();
    for q in points {
        if p.last().map(|l| (*l - *q).length() >= 0.3).unwrap_or(true) {
            p.push(*q);
        }
    }
    if p.len() < 2 {
        return Err("at least two points apart are needed".into());
    }
    let n = p.len();
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                roads::heading_of(p[1] - p[0])
            } else if i == n - 1 {
                roads::heading_of(p[n - 1] - p[n - 2])
            } else {
                let (a, b) = (p[i] - p[i - 1], p[i + 1] - p[i]);
                roads::heading_of(a.normalize() * b.length() + b.normalize() * a.length())
            }
        })
        .collect();
    if let Some(x) = h0 {
        h[0] = x;
    }
    if let Some(x) = h1 {
        h[n - 1] = x;
    }
    // (with only the ends' headings given, two points and two headings: one biarc)
    let mut out = Vec::new();
    for i in 0..n - 1 {
        let segs = biarc_any(p[i], h[i], p[i + 1], h[i + 1]).ok_or_else(|| format!("no smooth way from point {i} to point {}: {:.1} m apart with headings {:.0}° and {:.0}°", i + 1, (p[i + 1] - p[i]).length(), h[i], h[i + 1]))?;
        out.extend(segs);
    }
    Ok(out)
}

/// A spline end: where it is and the heading continuing out of the spline there.
pub fn end_of(doc: &Document, e: (TileKey, i64, bool)) -> Option<(DVec3, f64)> {
    let s = doc.spline(e.0, e.1)?;
    let c = omsi_geometry::SplineCurve::from_map(s, tile_origin(e.0));
    Some(if e.2 { (c.end_point(), c.heading_at(s.length)) } else { (c.point_at(0.0), roads::norm_deg(c.heading_at(0.0) + 180.0)) })
}

impl Editor {
    /// A chain of `file` along `segs` (world), `lift` over the ground; joined to the spline end
    /// `from` at its start and to `to` at its end (their heights kept). Gives back its pieces.
    pub fn lay_chain(&mut self, file: &str, segs: &[Seg], from: Option<(TileKey, i64, bool)>, to: Option<(TileKey, i64, bool)>, lift: f64) -> Result<Vec<(TileKey, i64)>, String> {
        let Some(doc) = self.doc.as_ref() else { return Err("no map".into()) };
        check_room(doc, segs)?;
        self.history.begin(tr("Splines laid", "Сплайны проложены"));
        let doc = self.doc.as_mut().unwrap();
        // (and the tiles of the ends it is linked to: their links change too)
        let keys: Vec<TileKey> = segs.iter().flat_map(|s| [tile_of(s.start), tile_of(s.end())]).chain(from.map(|e| e.0)).chain(to.map(|e| e.0)).collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        self.history.touch_global(doc);
        let made = lay_chain_in(doc, file, segs, from, to, lift);
        self.history.commit();
        made
    }
}

/// Whether every piece lies on the map.
fn check_room(doc: &Document, segs: &[Seg]) -> Result<(), String> {
    if segs.is_empty() {
        return Err("nothing to lay".into());
    }
    for s in segs {
        if !doc.tiles.contains_key(&tile_of(s.start)) || !doc.tiles.contains_key(&tile_of(s.end())) {
            return Err(tr("The spline reaches past the map: add tiles there first", "Сплайн выходит за карту: сначала добавьте тайлы").into());
        }
    }
    Ok(())
}

/// [`Editor::lay_chain`] within an edit already begun (no undo step of its own).
pub fn lay_chain_in(doc: &mut Document, file: &str, segs: &[Seg], from: Option<(TileKey, i64, bool)>, to: Option<(TileKey, i64, bool)>, lift: f64) -> Result<Vec<(TileKey, i64)>, String> {
    check_room(doc, segs)?;
    {
        let z_from = from.and_then(|e| end_of(doc, e)).map(|e| e.0.z);
        let z_to = to.and_then(|e| end_of(doc, e)).map(|e| e.0.z);
        let ids: Vec<i64> = segs.iter().map(|_| doc.new_id()).collect();
        let total: f64 = segs.iter().map(|s| s.length).sum();
        // heights: the ground (+ lift) along it; from a joined end's height, eased onto the
        // ground over the first and last 20 m
        let mut along = 0.0;
        let height = |doc: &Document, p: DVec2, at: f64| -> f64 {
            // (a short piece between two ends: straight from the one's height to the other's;
            // eased onto the ground from both ends at once it began off its own end's height)
            if let (Some(zf), Some(zt)) = (z_from, z_to) {
                if total < 40.0 {
                    return zf + (zt - zf) * (at / total.max(0.01)).clamp(0.0, 1.0);
                }
            }
            let mut z = doc.ground(p) + lift;
            if let Some(zf) = z_from {
                let k = (1.0 - at / 20.0).clamp(0.0, 1.0);
                z = z * (1.0 - k) + zf * k;
            }
            if let Some(zt) = z_to {
                let k = (1.0 - (total - at) / 20.0).clamp(0.0, 1.0);
                z = z * (1.0 - k) + zt * k;
            }
            z
        };
        let mut made = Vec::new();
        for (i, s) in segs.iter().enumerate() {
            let (z0, z1) = (height(doc, s.start, along), height(doc, s.end(), along + s.length));
            let k = tile_of(s.start);
            let org = tile_origin(k);
            let g = (z1 - z0) / s.length.max(0.01) * 100.0;
            let sp = MapSpline {
                file: file.to_string(),
                id: ids[i],
                prev_id: if i > 0 { ids[i - 1] } else { 0 },
                next_id: ids.get(i + 1).copied().unwrap_or(0),
                pos: [s.start.x - org.x, s.start.y - org.y, z0],
                heading: s.heading,
                length: s.length,
                radius: s.radius,
                grad_start: g,
                grad_end: g,
                // (a [spline_h]: its height ends exactly that much higher, and a cut keeps it so)
                delta_h: Some(z1 - z0),
                is_h: true,
                tex_offset: along,
                map_chain_offset: Some(along),
                ..Default::default()
            };
            along += s.length;
            let t = doc.tiles.get_mut(&k).unwrap();
            t.tile.splines.push(sp);
            t.touch_content();
            made.push((k, ids[i]));
        }
        if let (Some(e), Some(first)) = (from, made.first()) {
            crate::check::apply_link(doc, e, (first.0, first.1, false));
        }
        if let (Some(e), Some(last)) = (to, made.last()) {
            crate::check::apply_link(doc, (last.0, last.1, true), e);
        }
        Ok(made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_through_points_is_tangent_continuous_and_meets_its_end_headings() {
        let pts = [DVec2::new(0.0, 0.0), DVec2::new(20.0, 15.0), DVec2::new(45.0, 18.0), DVec2::new(60.0, 0.0)];
        let segs = chain_through(&pts, Some(90.0), Some(180.0)).unwrap();
        assert!((segs[0].heading - 90.0).abs() < 1e-6);
        assert!((roads::norm_deg(segs.last().unwrap().end_heading() - 180.0 + 180.0) - 180.0).abs() < 1.0);
        for w in segs.windows(2) {
            assert!((w[0].end() - w[1].start).length() < 1e-6, "{w:?}");
            assert!((roads::norm_deg(w[0].end_heading() - w[1].heading + 180.0) - 180.0).abs() < 1.0, "{w:?}");
        }
        assert!((segs.last().unwrap().end() - pts[3]).length() < 1e-6);
        // a straight run is one straight
        let s = chain_through(&[DVec2::ZERO, DVec2::new(0.0, 10.0), DVec2::new(0.0, 30.0)], None, None).unwrap();
        assert!(s.iter().all(|g| g.radius == 0.0), "{s:?}");
    }
}

/// Whether `p` lies inside the closed ring `poly` (even-odd).
pub fn inside_ring(poly: &[DVec2], p: DVec2) -> bool {
    let mut c = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
        j = i;
    }
    c
}

fn ring_dist(poly: &[DVec2], p: DVec2) -> f64 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-12)).clamp(0.0, 1.0);
            (a + (b - a) * t - p).length()
        })
        .fold(f64::MAX, f64::min)
}

impl Editor {
    /// Ground layer `layer` laid over the areas `polys` (closed rings, world), at full
    /// strength inside, fading over `feather` metres outside their edge; the layers above it
    /// taken away there so it shows (`erase`: the layer taken away instead). One undo step.
    /// Gives back the texels changed.
    pub fn paint_areas(&mut self, layer: usize, polys: &[Vec<DVec2>], feather: f64, erase: bool) -> Result<usize, String> {
        let Some(doc) = self.doc.as_mut() else { return Err("no map".into()) };
        self.history.begin(tr("Ground painted over areas", "Земля закрашена по площадям"));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
        for k in keys {
            self.history.touch_tile(doc, k);
        }
        let r = paint_areas_in(doc, layer, polys, feather, erase);
        if r.is_ok() {
            self.history.commit();
        } else {
            self.history.rollback(doc);
        }
        r
    }
}

/// [`Editor::paint_areas`] within an edit already begun.
pub fn paint_areas_in(doc: &mut Document, layer: usize, polys: &[Vec<DVec2>], feather: f64, erase: bool) -> Result<usize, String> {
    doc.sync_mask_slots();
    let layers = doc.global.ground_textures.len();
    if layer >= layers || layer == 0 && !erase {
        return Err(format!("layer {layer}: the map has layers 1 to {} to paint", layers.saturating_sub(1)));
    }
    let ts = crate::doc::tile_size();
    let boxes: Vec<(DVec2, DVec2)> = polys.iter().map(|p| (p.iter().fold(DVec2::splat(f64::MAX), |a, b| a.min(*b)) - DVec2::splat(feather), p.iter().fold(DVec2::splat(f64::MIN), |a, b| a.max(*b)) + DVec2::splat(feather))).collect();
    let keys: Vec<TileKey> = doc.tiles.keys().copied().filter(|k| { let o = crate::doc::tile_origin(*k); boxes.iter().any(|(lo, hi)| lo.x < o.x + ts && hi.x > o.x && lo.y < o.y + ts && hi.y > o.y) }).collect();
    let mut changed = 0usize;
    let sizes: Vec<usize> = doc.global.ground_textures.iter().map(crate::doc::mask_size).collect();
    for key in keys {
        let org = crate::doc::tile_origin(key);
        let t = doc.tiles.get_mut(&key).unwrap();
        let mut touched = false;
        for li in 1..layers {
            let up = li == layer && !erase;
            let down = (li == layer && erase) || (li > layer && !erase);
            if !up && !down {
                continue;
            }
            let size = sizes[li];
            let texel = ts / size as f64;
            let mask = t.masks[li].get_or_insert_with(|| vec![0u8; size * size]);
            if mask.len() != size * size {
                *mask = vec![0u8; size * size];
            }
            for (pi, poly) in polys.iter().enumerate() {
                let (lo, hi) = boxes[pi];
                let x0 = (((lo.x - org.x) / texel).floor().max(0.0)) as usize;
                let x1 = (((hi.x - org.x) / texel).ceil().min(size as f64)).max(0.0) as usize;
                let y0 = (((lo.y - org.y) / texel).floor().max(0.0)) as usize;
                let y1 = (((hi.y - org.y) / texel).ceil().min(size as f64)).max(0.0) as usize;
                for yi in y0..y1 {
                    for xi in x0..x1 {
                        let p = DVec2::new(org.x + (xi as f64 + 0.5) * texel, org.y + (yi as f64 + 0.5) * texel);
                        let w = if inside_ring(poly, p) { 1.0 } else if feather > 0.0 { (1.0 - ring_dist(poly, p) / feather).max(0.0) } else { 0.0 };
                        if w <= 0.0 {
                            continue;
                        }
                        let row = size - 1 - yi;
                        let a = &mut mask[row * size + xi];
                        let v = *a as f64;
                        let nv = if up { v.max(255.0 * w) } else { v * (1.0 - w) };
                        let nv = nv.round().clamp(0.0, 255.0) as u8;
                        if nv != *a {
                            *a = nv;
                            changed += 1;
                            touched = true;
                        }
                    }
                }
            }
        }
        if touched {
            t.touch_masks();
        }
    }
    Ok(changed)
}

/// What the track beds were made from: tracks, terrain samples changed, ground painted.
#[derive(Debug, Default, serde::Serialize)]
pub struct BedReport {
    pub tracks: usize,
    pub samples: usize,
    pub texels: usize,
}

impl Editor {
    /// Track beds in the ground: under every track the ground brought to `top` over the rail
    /// line (the top of its ballast) across `core` metres either side, blending back to the
    /// ground over `blend` metres; never above a road or path running near (its surface less
    /// 8 cm), so the yard's bed is one surface however the tracks run, split and end. The bed
    /// painted with ground layer `layer` (if any) `paint` metres either side. One undo step.
    pub fn track_beds(&mut self, assets: &mut crate::assets::Assets, top: f64, core: f64, blend: f64, layer: Option<usize>, paint: f64) -> BedReport {
        let mut rep = BedReport::default();
        let Some(doc) = self.doc.as_ref() else { return rep };
        // samples every metre: tracks (point, rail-line height), roads and paths (point, height, half width)
        let mut tracks: Vec<(DVec2, f64)> = Vec::new();
        let mut bands: Vec<Vec<DVec2>> = Vec::new();
        let mut ways: Vec<(DVec2, f64, f64)> = Vec::new();
        for (k, t) in &doc.tiles {
            let org = crate::doc::tile_origin(*k);
            for s in t.tile.splines.iter().filter(|s| !s.deleted && s.length > 0.05) {
                let Some(m) = assets.spline_wait(&s.file, 5000) else { continue };
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let n = (s.length.ceil() as usize).max(1);
                let pts: Vec<(DVec2, f64, f64)> = (0..=n).map(|i| { let a = s.length * i as f64 / n as f64; let q = c.point_at(a); (q.truncate(), q.z, c.heading_at(a)) }).collect();
                if m.is_track() {
                    rep.tracks += 1;
                    tracks.extend(pts.iter().map(|(p, z, _)| (*p, *z)));
                    let side = |p: DVec2, h: f64, x: f64| p + DVec2::new(h.to_radians().cos(), -h.to_radians().sin()) * x;
                    let mut ring: Vec<DVec2> = pts.iter().map(|(p, _, h)| side(*p, *h, paint)).collect();
                    ring.extend(pts.iter().rev().map(|(p, _, h)| side(*p, *h, -paint)));
                    bands.push(ring);
                } else if (m.joinable() || !m.def.paths.is_empty()) && !m.is_ai_path() {
                    let hw = m.half_width as f64;
                    ways.extend(pts.iter().map(|(p, z, _)| (*p, *z, hw)));
                }
            }
        }
        if tracks.is_empty() {
            return rep;
        }
        let cell = crate::doc::tile_size() / (omsi_map::TERRAIN_SAMPLES - 1) as f64;
        let n = (omsi_map::TERRAIN_SAMPLES - 1) as i64;
        let reach = core + blend;
        let gkey = |p: DVec2| ((p.x / 10.0).floor() as i64, (p.y / 10.0).floor() as i64);
        let mut tg: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, (p, _)) in tracks.iter().enumerate() {
            tg.entry(gkey(*p)).or_default().push(i);
        }
        let mut wg: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, (p, _, _)) in ways.iter().enumerate() {
            wg.entry(gkey(*p)).or_default().push(i);
        }
        let near = |g: &std::collections::HashMap<(i64, i64), Vec<usize>>, p: DVec2, r: f64| -> Vec<usize> {
            let (cx, cy) = gkey(p);
            let s = (r / 10.0).ceil() as i64;
            let mut v = Vec::new();
            for dx in -s..=s {
                for dy in -s..=s {
                    v.extend(g.get(&(cx.saturating_add(dx), cy.saturating_add(dy))).into_iter().flatten().copied());
                }
            }
            v
        };
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for (p, _) in &tracks {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        lo -= DVec2::splat(reach);
        hi += DVec2::splat(reach);
        // (only the map's own ground: a broken spline of a map file, 10^22 m out, made the
        // walk over the samples between endless)
        if let Some((a, b)) = self.doc.as_ref().map(|d| d.extent()) {
            (lo, hi) = (lo.max(a), hi.min(b));
        }
        self.history.begin(tr("Track beds", "Постель путей"));
        let pts: Vec<DVec2> = tracks.iter().step_by(10).map(|t| t.0).collect();
        self.touch_around(&pts, reach + 6.0);
        let Some(doc) = self.doc.as_mut() else { return rep };
        // (each tile a sample is set on is kept as it is set: the tiles round every so many
        // samples missed some, which undo then left as the edit made them)
        let hist = &mut self.history;
        for gy in (lo.y / cell).floor() as i64..=(hi.y / cell).ceil() as i64 {
            for gx in (lo.x / cell).floor() as i64..=(hi.x / cell).ceil() as i64 {
                let q = DVec2::new(gx as f64 * cell, gy as f64 * cell);
                let Some((d, z)) = near(&tg, q, reach).into_iter().map(|i| ((tracks[i].0 - q).length(), tracks[i].1)).min_by(|a, b| a.0.total_cmp(&b.0)) else { continue };
                if d > reach {
                    continue;
                }
                let Some(old) = crate::editor::world_sample(doc, gx, gy, n) else { continue };
                let target = z + top - 0.03;
                let w = if d <= core { 1.0 } else { 1.0 - ((d - core) / blend).clamp(0.0, 1.0) };
                let w = w * w * (3.0 - 2.0 * w);
                let mut h = old as f64 + (target - old as f64) * w;
                // (never up through a road or a path: a cell's diagonal round it stays under it)
                for i in near(&wg, q, 12.0) {
                    let (p, wz, hw) = ways[i];
                    if (p - q).length() <= hw + cell * std::f64::consts::SQRT_2 {
                        h = h.min(wz - 0.08);
                    }
                }
                if (h - old as f64).abs() > 0.005 {
                    crate::editor::set_world_sample(doc, hist, gx, gy, n, h as f32);
                    rep.samples += 1;
                }
            }
        }
        if let Some(l) = layer {
            // (the paint reaches whichever tiles the beds cross)
            for k in doc.tiles.keys().copied().collect::<Vec<_>>() {
                hist.touch_tile(doc, k);
            }
            rep.texels = paint_areas_in(doc, l, &bands, 1.0, false).unwrap_or(0);
        }
        self.history.commit();
        rep
    }
}

impl Editor {
    /// The ground kept under every surface: the upward faces of surface objects (junction plates,
    /// paved areas, crossing decks) and the roads, paths and tracks. Each terrain sample within a
    /// cell's diagonal of a surface (so the ground drawn between samples stays under it too) is
    /// brought down to 6 cm below it where it is higher. One undo step; how many samples moved.
    pub fn ground_under(&mut self, assets: &mut crate::assets::Assets) -> usize {
        let Some(doc) = self.doc.as_ref() else { return 0 };
        assets.load_map_models(doc, 60_000);
        let cell = crate::doc::tile_size() / (omsi_map::TERRAIN_SAMPLES - 1) as f64;
        let n = (omsi_map::TERRAIN_SAMPLES - 1) as i64;
        let reach = cell * std::f64::consts::SQRT_2 + 0.05;
        // surfaces as triangles (world) and as strips along splines (point, z, half width)
        let mut tris: Vec<[DVec3; 3]> = Vec::new();
        let mut strips: Vec<(DVec2, f64, f64)> = Vec::new();
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for o in &t.tile.objects {
                let Some(m) = assets.object(&o.file) else { continue };
                let flat = m.hi.z - m.lo.z < 1.5;
                if !(m.surface || (!m.paths.is_empty() && flat)) {
                    continue;
                }
                let p = org + DVec2::new(o.pos[0], o.pos[1]);
                let world = if m.abs_height { DVec3::new(p.x, p.y, o.pos[2]) } else { DVec3::new(p.x, p.y, doc.ground(p) + o.pos[2]) };
                let pose = crate::place::Pose::of_object(world, o.rot);
                for part in &m.parts {
                    let mesh = &part.mesh;
                    let w: Vec<DVec3> = mesh.positions.iter().map(|v| pose.pos + pose.rot.transform_vector3(*v).as_dvec3()).collect();
                    for f in mesh.indices.chunks_exact(3) {
                        let (a, b, c) = (w[f[0] as usize], w[f[1] as usize], w[f[2] as usize]);
                        let nz = (b - a).cross(c - a).normalize_or_zero().z.abs();
                        if nz > 0.6 {
                            tris.push([a, b, c]);
                        }
                    }
                }
            }
            for s in t.tile.splines.iter().filter(|s| !s.deleted && s.length > 0.05) {
                let Some(m) = assets.spline(&s.file) else { continue };
                // (tracks have their bed: see track_beds)
                if m.is_ai_path() || m.is_track() || !(m.joinable() || !m.def.paths.is_empty()) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let k = (s.length.ceil() as usize).max(1);
                for i in 0..=k {
                    let q = c.point_at(s.length * i as f64 / k as f64);
                    // (off the ground — bridge decks, stairs — nothing is done under it)
                    if q.z - doc.ground(q.truncate()) > 1.5 {
                        continue;
                    }
                    strips.push((q.truncate(), q.z, m.half_width as f64));
                }
            }
        }
        let key = |p: DVec2| ((p.x / 10.0).floor() as i64, (p.y / 10.0).floor() as i64);
        let mut tg: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, t) in tris.iter().enumerate() {
            let lo = t.iter().fold(DVec2::splat(f64::MAX), |a, p| a.min(p.truncate())) - DVec2::splat(reach);
            let hi = t.iter().fold(DVec2::splat(f64::MIN), |a, p| a.max(p.truncate())) + DVec2::splat(reach);
            for x in (lo.x / 10.0).floor() as i64..=(hi.x / 10.0).floor() as i64 {
                for y in (lo.y / 10.0).floor() as i64..=(hi.y / 10.0).floor() as i64 {
                    tg.entry((x, y)).or_default().push(i);
                }
            }
        }
        let mut sg: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, (p, _, _)) in strips.iter().enumerate() {
            sg.entry(key(*p)).or_default().push(i);
        }
        // the lowest a sample may stand: under each surface near it
        let mut cap: std::collections::HashMap<(i64, i64), f64> = Default::default();
        let mut put = |gx: i64, gy: i64, z: f64| {
            let e = cap.entry((gx, gy)).or_insert(f64::MAX);
            *e = e.min(z);
        };
        let tri_z = |t: &[DVec3; 3], q: DVec2| -> (f64, f64) {
            // (distance in plan from the triangle, and its height at the nearest point)
            let (a, b, c) = (t[0], t[1], t[2]);
            let (p0, p1, p2) = (a.truncate(), b.truncate(), c.truncate());
            let den = (p1 - p0).perp_dot(p2 - p0);
            if den.abs() < 1e-9 {
                return (f64::MAX, 0.0);
            }
            let wb = (q - p0).perp_dot(p2 - p0) / den;
            let wc = (p1 - p0).perp_dot(q - p0) / den;
            let wa = 1.0 - wb - wc;
            let (wa2, wb2, wc2) = (wa.clamp(0.0, 1.0), wb.clamp(0.0, 1.0), wc.clamp(0.0, 1.0));
            let s = (wa2 + wb2 + wc2).max(1e-9);
            let near = (p0 * wa2 + p1 * wb2 + p2 * wc2) / s;
            let z = (a.z * wa2 + b.z * wb2 + c.z * wc2) / s;
            if wa >= 0.0 && wb >= 0.0 && wc >= 0.0 { (0.0, a.z * wa + b.z * wb + c.z * wc) } else { ((near - q).length(), z) }
        };
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for t in &tris {
            for p in t {
                lo = lo.min(p.truncate());
                hi = hi.max(p.truncate());
            }
        }
        for (p, _, h) in &strips {
            lo = lo.min(*p - DVec2::splat(*h));
            hi = hi.max(*p + DVec2::splat(*h));
        }
        if lo.x > hi.x {
            return 0;
        }
        lo -= DVec2::splat(reach);
        hi += DVec2::splat(reach);
        // (only the map's own ground: a broken spline of a map file, 10^22 m out, made the
        // walk over the samples between endless)
        if let Some((a, b)) = self.doc.as_ref().map(|d| d.extent()) {
            (lo, hi) = (lo.max(a), hi.min(b));
        }
        for gy in (lo.y / cell).floor() as i64..=(hi.y / cell).ceil() as i64 {
            for gx in (lo.x / cell).floor() as i64..=(hi.x / cell).ceil() as i64 {
                let q = DVec2::new(gx as f64 * cell, gy as f64 * cell);
                for i in tg.get(&key(q)).into_iter().flatten() {
                    let (d, z) = tri_z(&tris[*i], q);
                    if d <= reach {
                        put(gx, gy, z - 0.06);
                    }
                }
                let (cx, cy) = key(q);
                for dx in -2..=2 {
                    for dy in -2..=2 {
                        for i in sg.get(&(cx.saturating_add(dx), cy.saturating_add(dy))).into_iter().flatten() {
                            let (p, z, h) = strips[*i];
                            if (p - q).length() <= h + reach {
                                put(gx, gy, z - 0.06);
                            }
                        }
                    }
                }
            }
        }
        self.history.begin(tr("Ground under surfaces", "Земля под покрытиями"));
        let pts: Vec<DVec2> = cap.keys().step_by(20).map(|(x, y)| DVec2::new(*x as f64 * cell, *y as f64 * cell)).collect();
        self.touch_around(&pts, 400.0);
        let Some(doc) = self.doc.as_mut() else { return 0 };
        // (each tile a sample is set on is kept as it is set: the tiles round every so many
        // samples missed some, which undo then left as the edit made them)
        let hist = &mut self.history;
        let mut moved = 0;
        for ((gx, gy), z) in cap {
            let Some(old) = crate::editor::world_sample(doc, gx, gy, n) else { continue };
            if (old as f64) > z {
                crate::editor::set_world_sample(doc, hist, gx, gy, n, z as f32);
                moved += 1;
            }
        }
        self.history.commit();
        moved
    }
}

#[cfg(test)]
mod short_joint_tests {
    use super::*;

    /// A chain laid on from a road whose spline starts on another tile is undone whole: that
    /// road's link to it was changed with its tile not kept, and kept pointing at a spline
    /// that no longer was.
    #[test]
    fn a_chain_laid_on_from_a_road_on_another_tile_is_undone_whole() {
        let mut doc = crate::selection::tests::flat_map(2);
        let ts = crate::doc::tile_size();
        // (from tile 0, 0 across into tile 1, 0)
        crate::selection::tests::add_spline(&mut doc, (0, 0), 1, ts - 20.0, 100.0, 90.0, 40.0, 0, 0);
        let (end, heading) = end_of(&doc, ((0, 0), 1, true)).unwrap();
        assert_eq!(tile_of(end.truncate()), (1, 0));
        let way = DVec2::new(heading.to_radians().sin(), heading.to_radians().cos());
        let seg = crate::roads::straight(end.truncate(), end.truncate() + way * 30.0).unwrap();
        assert_eq!(tile_of(seg.end()), (1, 0), "the chain lies on tile 1, 0 alone");
        let mut ed = Editor::default();
        ed.open(doc);
        let made = ed.lay_chain("Splines\\road.sli", &[seg], Some(((0, 0), 1, true)), None, 0.0).unwrap();
        let next = |ed: &Editor| ed.doc.as_ref().unwrap().spline((0, 0), 1).unwrap().next_id;
        assert_eq!(next(&ed), made[0].1, "linked");
        ed.undo();
        assert_eq!(next(&ed), 0, "the link undone with the chain");
    }

    #[test]
    fn a_short_piece_between_two_ends_runs_from_the_one_height_to_the_other() {
        let roots = crate::doc::tests::scratch("short-joint");
        let mut doc = crate::doc::tests::sample_map(&roots);
        doc.tiles.retain(|k, _| *k == (0, 0));
        let a = crate::roads::straight(DVec2::new(10.0, 10.0), DVec2::new(10.0, 40.0)).unwrap();
        let b = crate::roads::straight(DVec2::new(10.0, 43.0), DVec2::new(10.0, 80.0)).unwrap();
        let mut ends = Vec::new();
        for (g, z) in [(a, 3.0), (b, 3.6)] {
            let made = lay_chain_in(&mut doc, "Splines\\x.sli", &[g], None, None, 0.0).unwrap();
            let (k, id) = made[0];
            let sp = doc.tiles.get_mut(&k).unwrap().tile.splines.iter_mut().find(|s| s.id == id).unwrap();
            sp.pos[2] = z;
            sp.delta_h = Some(0.0);
            ends.push((k, id));
        }
        let mid = crate::roads::straight(DVec2::new(10.0, 40.0), DVec2::new(10.0, 43.0)).unwrap();
        let made = lay_chain_in(&mut doc, "Splines\\x.sli", &[mid], Some((ends[0].0, ends[0].1, true)), Some((ends[1].0, ends[1].1, false)), 0.0).unwrap();
        let sp = doc.spline(made[0].0, made[0].1).unwrap();
        assert!((sp.pos[2] - 3.0).abs() < 1e-6, "{}", sp.pos[2]);
        assert!((sp.pos[2] + sp.delta_h.unwrap() - 3.6).abs() < 1e-6);
    }
}
