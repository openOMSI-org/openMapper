//! The ground formed once, from everything that lies on it (`form_ground`): roads, paths,
//! junction plates and paved areas, track beds (tracks and switches). One pass over the terrain
//! samples, each taking its height from the surfaces near it at once, so no later pass undoes
//! an earlier one; how far it moved the ground is measured, and over a limit it is reported
//! (a pit or a cliff is a fault of what lies there, not something to dig).

use glam::{DVec2, DVec3};
use serde::Serialize;

use crate::doc::tile_origin;
use crate::editor::Editor;
use crate::i18n::tr;

/// What `form_ground` did, and what it found wrong.
#[derive(Default, Serialize)]
pub struct FormReport {
    /// Samples moved (more than 5 mm).
    pub samples: usize,
    pub raised: usize,
    pub lowered: usize,
    /// The largest rise and fall (m).
    pub max_raise: f64,
    pub max_lower: f64,
    /// Samples moved more than the limit: (world x, y, metres moved), worst first.
    pub over_limit: Vec<[f64; 3]>,
    /// Things lying where the map has no ground (no tile there): (what, world x, y).
    pub off_terrain: Vec<(String, f64, f64)>,
    /// Ground texels painted as the tracks' ballast.
    pub texels: usize,
}

/// Settings of `form_ground`.
pub struct FormSettings {
    /// The bed's top over a track's line (the ballast between its sleepers lies at 0.64).
    pub bed_top: f64,
    /// The bed's half width either side of a track's line.
    pub bed_half: f64,
    /// Metres over which the ground goes back to its own height round what was laid.
    pub blend: f64,
    /// The most a sample may move before it is reported.
    pub limit: f64,
    /// Ground layer painted under the tracks (the bed's width + `paint_extra`), if any.
    pub layer: Option<usize>,
    pub paint_extra: f64,
    /// Measure only: the ground is not changed.
    pub dry_run: bool,
    /// Only the ground in this box (world, min and max) is formed (what lies outside still counts,
    /// so the edge meets it); `None`: the whole map.
    pub region: Option<(DVec2, DVec2)>,
}

/// A point of something that sets the ground: where (plan), the ground wanted there, and the
/// highest the ground may stand there (so it is not seen through it).
#[derive(Clone, Copy)]
struct Want {
    p: DVec2,
    z: f64,
    cap: f64,
}

impl Editor {
    /// The ground formed under everything on it, in one pass (see the module's words).
    ///
    /// Everything that lies on the ground is turned into points a metre apart: roads, paths,
    /// plates and paved areas (flat surface objects, under 1.5 m from their lowest to their
    /// highest face: a building is not a surface whatever its render type), each wanting the
    /// ground 6 cm under it; tracks and switches, wanting their bed. (1) A sample with such points
    /// round it takes the lowest height wanted there, up or down; one near them goes smoothly
    /// back to its own height over `blend`. (2) Then the ground drawn between the samples is
    /// kept under every point: where it would show through, the four samples round it are
    /// lowered just enough (no trench beyond the cell it is in).
    pub fn form_ground(&mut self, assets: &mut crate::assets::Assets, set: &FormSettings) -> FormReport {
        let mut rep = FormReport::default();
        let Some(doc) = self.doc.as_ref() else { return rep };
        assets.load_map_models(doc, 60_000);
        let cell = crate::doc::tile_size() / (omsi_map::TERRAIN_SAMPLES - 1) as f64;
        let n = (omsi_map::TERRAIN_SAMPLES - 1) as i64;
        let mut want: Vec<Want> = Vec::new();
        let mut bands: Vec<Vec<DVec2>> = Vec::new();
        let off = |what: &str, p: DVec2, rep: &mut FormReport| {
            if rep.off_terrain.len() < 400 && !rep.off_terrain.iter().any(|o| o.0 == what && (DVec2::new(o.1, o.2) - p).length() < 10.0) {
                rep.off_terrain.push((what.to_string(), (p.x * 10.0).round() / 10.0, (p.y * 10.0).round() / 10.0));
            }
        };
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for o in &t.tile.objects {
                let Some(m) = assets.object(&o.file) else { continue };
                let rail = m.helpers.iter().any(|(_, s)| assets.spline(s).map(|x| x.is_track()).unwrap_or(false));
                let flat = m.hi.z - m.lo.z < 1.5;
                let surface = (m.surface || !m.paths.is_empty()) && flat && !m.technical;
                if !rail && !surface {
                    continue;
                }
                let p = org + DVec2::new(o.pos[0], o.pos[1]);
                let Some(g) = doc.height_at(p) else {
                    off(&o.file, p, &mut rep);
                    continue;
                };
                let world = if m.abs_height { DVec3::new(p.x, p.y, o.pos[2]) } else { DVec3::new(p.x, p.y, g as f64 + o.pos[2]) };
                let pose = crate::place::Pose::of_object(world, o.rot);
                for part in &m.parts {
                    let mesh = &part.mesh;
                    let w: Vec<DVec3> = mesh.positions.iter().map(|v| pose.pos + pose.rot.transform_vector3(*v).as_dvec3()).collect();
                    for f in mesh.indices.chunks_exact(3) {
                        let (a, b, c) = (w[f[0] as usize], w[f[1] as usize], w[f[2] as usize]);
                        let nz = (b - a).cross(c - a).normalize_or_zero().z;
                        if rail {
                            // (a switch: its footprint is bed, at the height of its rail line)
                            for q in fill(a, b, c, 1.0) {
                                want.push(Want { p: q.truncate(), z: world.z + set.bed_top, cap: world.z + set.bed_top + 0.02 });
                            }
                        } else if nz > 0.6 {
                            for q in fill(a, b, c, 1.0) {
                                if doc.height_at(q.truncate()).is_none() {
                                    off(&o.file, q.truncate(), &mut rep);
                                }
                                want.push(Want { p: q.truncate(), z: q.z - 0.06, cap: q.z - 0.03 });
                            }
                        }
                    }
                }
                if rail {
                    let ring = footprint(&m, &pose, set.paint_extra);
                    if ring.len() >= 3 {
                        bands.push(ring);
                    }
                }
            }
            for s in t.tile.splines.iter().filter(|s| !s.deleted && s.length > 0.05) {
                let Some(m) = assets.spline(&s.file) else { continue };
                let track = m.is_track();
                // (a fence or a wall stands up from its line: not a surface, whatever its base strip)
                if m.is_ai_path() || !track && (!(m.joinable() || !m.def.paths.is_empty()) || m.top() > 0.5) {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(s, org);
                let k = (s.length.ceil() as usize).max(1);
                // (a structure — a bridge deck, a ramp, stairs — stands on its own: either end more
                // than 1.5 m over the ground, nothing under any of it is shaped)
                let over = |q: DVec3| doc.height_at(q.truncate()).map(|g| q.z - g as f64).unwrap_or(0.0);
                if !track && (over(c.point_at(0.0)) > 1.5 || over(c.end_point()) > 1.5) {
                    continue;
                }
                // (across: the drawn profile's top at each 0.5 m; a track's bed out to `bed_half`)
                let (x0, x1) = profile_span(&m.def);
                let (x0, x1) = if track { (x0.min(-set.bed_half), x1.max(set.bed_half)) } else { (x0, x1) };
                let across_n = ((x1 - x0) / 0.5).ceil().max(1.0) as usize;
                let mut side_a = Vec::new();
                let mut side_b = Vec::new();
                for i in 0..=k {
                    let a = s.length * i as f64 / k as f64;
                    let q = c.point_at(a);
                    let Some(g) = doc.height_at(q.truncate()) else {
                        off(&s.file, q.truncate(), &mut rep);
                        continue;
                    };
                    // (off the ground — bridge decks, stairs — nothing is done under it)
                    if q.z - g as f64 > 1.5 {
                        continue;
                    }
                    let h = c.heading_at(a).to_radians();
                    let across = DVec2::new(h.cos(), -h.sin());
                    for j in 0..=across_n {
                        let x = x0 + (x1 - x0) * j as f64 / across_n as f64;
                        let p = q.truncate() + across * x;
                        let top = profile_top(&m.def, if s.mirror { -x } else { x });
                        if track {
                            let cap = top.map(|t| q.z + t - 0.01).unwrap_or(f64::MAX).max(q.z + set.bed_top);
                            want.push(Want { p, z: q.z + set.bed_top, cap: cap.min(q.z + set.bed_top + 0.04) });
                        } else if let Some(t) = top {
                            want.push(Want { p, z: q.z + t - 0.05, cap: q.z + t - 0.02 });
                        }
                    }
                    if track {
                        side_a.push(q.truncate() + across * (set.bed_half + set.paint_extra));
                        side_b.push(q.truncate() - across * (set.bed_half + set.paint_extra));
                    }
                }
                if side_a.len() >= 2 {
                    side_a.extend(side_b.into_iter().rev());
                    bands.push(side_a);
                }
            }
        }
        if want.is_empty() {
            return rep;
        }
        // hashed by 10 m cells
        let key = |p: DVec2| ((p.x / 10.0).floor() as i64, (p.y / 10.0).floor() as i64);
        let mut gw: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, w) in want.iter().enumerate() {
            gw.entry(key(w.p)).or_default().push(i);
        }
        let near = |p: DVec2, r: f64| -> Vec<usize> {
            let (cx, cy) = key(p);
            let s = (r / 10.0).ceil() as i64;
            let mut v = Vec::new();
            for dx in -s..=s {
                for dy in -s..=s {
                    v.extend(gw.get(&(cx + dx, cy + dy)).into_iter().flatten().copied());
                }
            }
            v
        };
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for w in &want {
            lo = lo.min(w.p);
            hi = hi.max(w.p);
        }
        lo -= DVec2::splat(set.blend + cell);
        hi += DVec2::splat(set.blend + cell);
        if let Some((a, b)) = set.region {
            lo = lo.max(a);
            hi = hi.min(b);
            if lo.x > hi.x || lo.y > hi.y {
                return rep;
            }
            bands.retain(|r| r.iter().any(|p| p.x >= a.x && p.y >= a.y && p.x <= b.x && p.y <= b.y));
        }
        // (1) wanted heights, and the way back round them
        let mut hmap: std::collections::HashMap<(i64, i64), (f64, f64)> = Default::default();
        const COVER: f64 = 0.75;
        for gy in (lo.y / cell).floor() as i64..=(hi.y / cell).ceil() as i64 {
            for gx in (lo.x / cell).floor() as i64..=(hi.x / cell).ceil() as i64 {
                let q = DVec2::new(gx as f64 * cell, gy as f64 * cell);
                let Some(old) = crate::editor::world_sample(doc, gx, gy, n) else { continue };
                let old = old as f64;
                let mut inside: Option<f64> = None;
                let mut nearest: Option<(f64, f64)> = None;
                for i in near(q, set.blend + COVER) {
                    let w = want[i];
                    let d = ((w.p - q).length() - COVER).max(0.0);
                    if d == 0.0 {
                        inside = Some(inside.map(|z: f64| z.min(w.z)).unwrap_or(w.z));
                    }
                    if nearest.map(|n| d < n.0 || d == n.0 && w.z < n.1).unwrap_or(true) {
                        nearest = Some((d, w.z));
                    }
                }
                let h = match (inside, nearest) {
                    (Some(z), _) => z,
                    (None, Some((d, z))) if d < set.blend => {
                        let w = 1.0 - d / set.blend;
                        let w = w * w * (3.0 - 2.0 * w);
                        old + (z - old) * w
                    }
                    _ => old,
                };
                hmap.insert((gx, gy), (old, h));
            }
        }
        // (2) the ground between the samples kept under every point (the higher of a cell's two
        // ways to be cut in triangles, so either way the game cuts it holds)
        for _ in 0..8 {
            let mut changed = false;
            for w in &want {
                let (fx, fy) = (w.p.x / cell, w.p.y / cell);
                let (gx, gy) = (fx.floor() as i64, fy.floor() as i64);
                let (u, v) = (fx - gx as f64, fy - gy as f64);
                let corners = [(gx, gy), (gx + 1, gy), (gx, gy + 1), (gx + 1, gy + 1)];
                let hs: Vec<f64> = corners.iter().map(|c| hmap.get(c).map(|x| x.1).unwrap_or(f64::NAN)).collect();
                if hs.iter().any(|h| h.is_nan()) {
                    continue;
                }
                let (h00, h10, h01, h11) = (hs[0], hs[1], hs[2], hs[3]);
                // diagonal 00-11
                let a = if u >= v { h00 + (h10 - h00) * u + (h11 - h10) * v } else { h00 + (h01 - h00) * v + (h11 - h01) * u };
                // diagonal 10-01
                let b = if u + v <= 1.0 { h00 + (h10 - h00) * u + (h01 - h00) * v } else { h11 + (h01 - h11) * (1.0 - u) + (h10 - h11) * (1.0 - v) };
                let g = a.max(b);
                if g > w.cap + 1e-4 {
                    let e = g - w.cap;
                    for c in corners {
                        if let Some(x) = hmap.get_mut(&c) {
                            if x.1 > w.cap - 0.02 {
                                x.1 = (x.1 - e).max(w.cap - 0.02).min(x.1);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut moves: Vec<(i64, i64, f32)> = Vec::new();
        for ((gx, gy), (old, h)) in &hmap {
            if (h - old).abs() <= 0.005 {
                continue;
            }
            moves.push((*gx, *gy, *h as f32));
            let dz = h - old;
            rep.samples += 1;
            if dz > 0.0 {
                rep.raised += 1;
                rep.max_raise = rep.max_raise.max(dz);
            } else {
                rep.lowered += 1;
                rep.max_lower = rep.max_lower.max(-dz);
            }
            if dz.abs() > set.limit {
                rep.over_limit.push([*gx as f64 * cell, *gy as f64 * cell, (dz * 100.0).round() / 100.0]);
            }
        }
        rep.over_limit.sort_by(|a, b| b[2].abs().total_cmp(&a[2].abs()));
        rep.over_limit.truncate(300);
        rep.max_raise = (rep.max_raise * 100.0).round() / 100.0;
        rep.max_lower = (rep.max_lower * 100.0).round() / 100.0;
        if set.dry_run {
            return rep;
        }
        self.history.begin(tr("Form the ground", "Рельеф по покрытиям"));
        let pts: Vec<DVec2> = moves.iter().step_by(20).map(|(x, y, _)| DVec2::new(*x as f64 * cell, *y as f64 * cell)).collect();
        self.touch_around(&pts, 400.0);
        let Some(doc) = self.doc.as_mut() else { return rep };
        // (each tile a sample is set on is kept as it is set: the tiles round every so many
        // samples missed some, which undo then left as the edit made them)
        let hist = &mut self.history;
        for (gx, gy, h) in moves {
            crate::editor::set_world_sample(doc, hist, gx, gy, n, h);
        }
        if let Some(l) = set.layer {
            // (the paint reaches whichever tiles the surfaces cross)
            for k in doc.tiles.keys().copied().collect::<Vec<_>>() {
                hist.touch_tile(doc, k);
            }
            rep.texels = crate::ed_author::paint_areas_in(doc, l, &bands, 0.5, false).unwrap_or(0);
        }
        self.history.commit();
        rep
    }
}

/// How far a spline's drawn profile reaches either side (m, right positive).
fn profile_span(def: &omsi_scenery::Spline) -> (f64, f64) {
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for q in def.profiles.iter().flat_map(|p| p.points.iter()) {
        lo = lo.min(q.x as f64);
        hi = hi.max(q.x as f64);
    }
    if lo > hi { (0.0, 0.0) } else { (lo, hi) }
}

/// The top of a spline's drawn profile `x` metres right of its line (the highest of its ribbons
/// there; a wall's face, near vertical, does not count), if anything is drawn there.
fn profile_top(def: &omsi_scenery::Spline, x: f64) -> Option<f64> {
    let mut top: Option<f64> = None;
    for p in &def.profiles {
        for w in p.points.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let (ax, bx) = (a.x as f64, b.x as f64);
            let (l, r) = (ax.min(bx), ax.max(bx));
            if r - l < 0.02 || x < l - 1e-6 || x > r + 1e-6 {
                continue;
            }
            let t = (x - ax) / (bx - ax);
            let z = a.z as f64 + (b.z as f64 - a.z as f64) * t;
            top = Some(top.map(|v: f64| v.max(z)).unwrap_or(z));
        }
    }
    top
}

/// Points over a triangle about `step` apart (its corners included).
fn fill(a: DVec3, b: DVec3, c: DVec3, step: f64) -> Vec<DVec3> {
    let l = (b - a).length().max((c - a).length()).max((c - b).length());
    let k = ((l / step).ceil() as usize).clamp(1, 200);
    let mut v = Vec::with_capacity((k + 1) * (k + 2) / 2);
    for i in 0..=k {
        for j in 0..=(k - i) {
            let (u, w) = (i as f64 / k as f64, j as f64 / k as f64);
            v.push(a + (b - a) * u + (c - a) * w);
        }
    }
    v
}

/// An object's plan outline (its box, turned and placed), grown by `grow` metres.
fn footprint(m: &crate::assets::ObjectModel, pose: &crate::place::Pose, grow: f64) -> Vec<DVec2> {
    let (lo, hi) = (m.lo.as_dvec3(), m.hi.as_dvec3());
    let mut pts: Vec<DVec2> = Vec::new();
    for (x, y, z) in [(lo.x, lo.y, lo.z), (hi.x, lo.y, lo.z), (hi.x, hi.y, lo.z), (lo.x, hi.y, lo.z), (lo.x, lo.y, hi.z), (hi.x, lo.y, hi.z), (hi.x, hi.y, hi.z), (lo.x, hi.y, hi.z)] {
        pts.push((pose.pos + pose.rot.transform_vector3(glam::Vec3::new(x as f32, y as f32, z as f32)).as_dvec3()).truncate());
    }
    // (the convex hull of the box's corners in plan, grown)
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (*a - *b).length() < 1e-6);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<DVec2> = Vec::new();
    for p in pts.iter().chain(pts.iter().rev().skip(1)) {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0 {
            hull.pop();
        }
        hull.push(*p);
    }
    hull.pop();
    let c = hull.iter().fold(DVec2::ZERO, |a, p| a + *p) / hull.len() as f64;
    hull.iter().map(|p| *p + (*p - c).normalize_or_zero() * grow).collect()
}
