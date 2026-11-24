//! What shows the trolleybus wire on the map: a **kit**, a `.cfg` file the map maker writes
//! (by hand, or in the kit's window) naming their own content: the wire spline (`.sli`, made in
//! Blender or anywhere, its wires at their height in its profile) and the mast, frog and section
//! insulator objects (`.sco`), with how far apart masts stand and how far a mast is from its
//! wire. openMapper makes no models of its own: it lays the files the kit names.
//!
//! A kit is the map's (`<map>\trolley\<name>.cfg`) or shared by every map
//! (`<content>\openMapper\trolley\<name>.cfg`).
//!
//! Laying wire over a road puts a copy of each road piece of the kit's wire spline on the map,
//! moved sideways so the wire hangs over the lane, and masts beside it every so many metres;
//! wire over a junction's lanes (or drawn by hand) is laid as short straight pieces. What a
//! span laid is kept with it (`Span::laid`) and goes with it; the frogs and insulators are put
//! again wherever the wire's are after every change (`Wire::marks`). All of it with the wire's
//! file in one undo step.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DVec2, DVec3};
use omsi_map::{MapObject, MapSpline};

use crate::doc::{tile_of, tile_origin, Document};
use crate::editor::Editor;
use crate::history::History;
use crate::i18n::tr;
use crate::install::Roots;
use crate::trolley::{RoadLane, Span, Wire};

// ------------------------------------------------------------------------------ the kit

#[derive(Debug, Clone, PartialEq)]
pub struct KitDef {
    pub name: String,
    /// The wire spline (`Splines\…\x.sli`): laid along the wire.
    pub wire: String,
    /// The objects (`Sceneryobjects\…\x.sco`); empty: none put.
    pub mast: String,
    pub frog: String,
    pub insulator: String,
    /// Metres between masts along a road.
    pub step: f64,
    /// Metres from a mast's foot to the wire it holds, across the road.
    pub reach: f64,
    /// Degrees a mast is turned more (clockwise from above): 0 when its model's arm reaches
    /// along its -y, 90 along its -x, -90 along its +x.
    pub turn: f64,
}

impl Default for KitDef {
    fn default() -> KitDef {
        KitDef { name: String::new(), wire: String::new(), mast: String::new(), frog: String::new(), insulator: String::new(), step: 30.0, reach: 2.5, turn: 0.0 }
    }
}

fn f(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Where the objects stand, said in the kit's file for whoever writes it by hand.
const HELP: &str = "# A trolleybus wire kit of openMapper (Map > Trolleybus wire): your own content, laid where\n\
# the wire runs. Files are paths in the content (or OMSI 2) folder; leave one empty for none.\n\
# [wire]       the wire spline (.sli): its profile puts the wires at their height over the road\n\
# [mast]       an object (.sco): its origin at its foot, its arm reaching along its -y to the wire\n\
# [frog]       an object hung where the wire branches: its origin on the wire, y along it\n\
# [insulator]  an object hung where two sections meet: its origin on the wire, y along it\n\
# [masts]      metres between masts, metres from a mast's foot to its wire, and degrees the\n\
#              mast is turned (0: its arm along -y, 90: along -x, -90: along +x)\n\n";

impl KitDef {
    /// The kit as its `.cfg` file.
    pub fn text(&self) -> String {
        let mut s = String::from(HELP);
        let _ = writeln!(s, "[name]\n{}\n", self.name.replace(['\r', '\n'], " "));
        for (key, v) in [("wire", &self.wire), ("mast", &self.mast), ("frog", &self.frog), ("insulator", &self.insulator)] {
            let _ = writeln!(s, "[{key}]\n{}\n", v.trim());
        }
        let _ = writeln!(s, "[masts]\n{}\n{}\n{}", f(self.step), f(self.reach), f(self.turn));
        s
    }

    /// A kit read from its `.cfg` (what it does not understand is left out; a block with no
    /// line after it is empty).
    pub fn parse(text: &str) -> KitDef {
        let mut k = KitDef::default();
        let lines: Vec<&str> = text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).collect();
        let mut i = 0;
        // the lines of a block up to the next block or blank line
        let body = |i: usize| -> Vec<&str> { lines[i + 1..].iter().take_while(|l| !l.is_empty() && !l.starts_with('[')).copied().collect() };
        while i < lines.len() {
            let key = lines[i].to_ascii_lowercase();
            let b = body(i);
            let first = b.first().map(|x| x.to_string()).unwrap_or_default();
            match key.as_str() {
                "[name]" => k.name = first,
                "[wire]" => k.wire = first,
                "[mast]" => k.mast = first,
                "[frog]" => k.frog = first,
                "[insulator]" => k.insulator = first,
                "[masts]" => {
                    k.step = b.first().and_then(|l| l.parse().ok()).unwrap_or(30.0);
                    k.reach = b.get(1).and_then(|l| l.parse().ok()).unwrap_or(2.5);
                    k.turn = b.get(2).and_then(|l| l.parse().ok()).unwrap_or(0.0);
                }
                _ => {}
            }
            i += 1;
        }
        k
    }

    /// The wire spline's wires: their middle across it, how far apart, how high (None: the
    /// file is not there, or its profile is no pair of wires).
    pub fn profile(&self, roots: &Roots) -> Option<(f64, f64, f64)> {
        let def = omsi_scenery::sli::Spline::load(&roots.resolve(self.wire.trim())).ok()?;
        crate::trolley::wire_profile(&def)
    }

    /// What is wrong with it: files named that are not there.
    pub fn problems(&self, roots: &Roots) -> Vec<String> {
        let mut out = Vec::new();
        for (what, v) in [(tr("the wire spline", "сплайн провода"), &self.wire), (tr("the mast", "опора"), &self.mast), (tr("the frog", "развилка"), &self.frog), (tr("the insulator", "изолятор"), &self.insulator)] {
            if !v.trim().is_empty() && !roots.resolve(v.trim()).is_file() {
                out.push(format!("{what}: {} {}", v.trim(), tr("is not there", "не найден")));
            }
        }
        if self.wire.trim().is_empty() {
            out.push(tr("no wire spline: nothing of the wire is shown on the map", "нет сплайна провода: сам провод на карте не показывается").into());
        } else if roots.resolve(self.wire.trim()).is_file() && self.profile(roots).is_none() {
            out.push(tr("the wire spline's profile is no pair of thin wires over 3 m: it is laid as it is, and openOMSI will not take it for the wire without trolley.txt", "профиль сплайна провода — не пара тонких проводов выше 3 м: он кладётся как есть, и openOMSI без trolley.txt не примет его за провод").into());
        }
        out
    }
}

/// Where a kit is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// In the map's folder: this map only.
    Map,
    /// In the content folder: every map.
    Shared,
}

/// A kit on disk: `map:<stem>` or `shared:<stem>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitRef {
    pub scope: Scope,
    pub stem: String,
}

impl KitRef {
    pub fn id(&self) -> String {
        format!("{}:{}", if self.scope == Scope::Map { "map" } else { "shared" }, self.stem)
    }

    pub fn of_id(id: &str) -> Option<KitRef> {
        let (s, stem) = id.split_once(':')?;
        let scope = match s {
            "map" => Scope::Map,
            "shared" => Scope::Shared,
            _ => return None,
        };
        Some(KitRef { scope, stem: stem.to_string() })
    }

    fn dir(scope: Scope, roots: &Roots, map_dir: &Path) -> PathBuf {
        match scope {
            Scope::Map => map_dir.join("trolley"),
            Scope::Shared => roots.content.join("openMapper").join("trolley"),
        }
    }

    pub fn path(&self, roots: &Roots, map_dir: &Path) -> PathBuf {
        KitRef::dir(self.scope, roots, map_dir).join(format!("{}.cfg", self.stem))
    }
}

/// A file name for a kit's name.
pub fn stem_of(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.trim_matches('_').is_empty() { "kit".into() } else { s }
}

/// Every kit the map can use: its own, then the shared ones.
pub fn list(roots: &Roots, map_dir: &Path) -> Vec<(KitRef, KitDef)> {
    let mut out = Vec::new();
    for scope in [Scope::Map, Scope::Shared] {
        let mut here: Vec<(KitRef, KitDef)> = std::fs::read_dir(KitRef::dir(scope, roots, map_dir))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "cfg").then_some(())?;
                let stem = p.file_stem()?.to_string_lossy().to_string();
                let text = omsi_cfg::decode_text(&std::fs::read(&p).ok()?);
                Some((KitRef { scope, stem }, KitDef::parse(&text)))
            })
            .collect();
        here.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()));
        out.extend(here);
    }
    out
}

pub fn load(roots: &Roots, map_dir: &Path, id: &str) -> Option<KitDef> {
    let r = KitRef::of_id(id)?;
    Some(KitDef::parse(&omsi_cfg::decode_text(&std::fs::read(r.path(roots, map_dir)).ok()?)))
}

pub fn save(roots: &Roots, map_dir: &Path, r: &KitRef, k: &KitDef) -> anyhow::Result<()> {
    let p = r.path(roots, map_dir);
    std::fs::create_dir_all(p.parent().unwrap_or(map_dir))?;
    crate::doc::safe::write_atomic(&p, k.text().replace('\n', "\r\n").as_bytes())?;
    crate::hot::own_change();
    Ok(())
}

/// A kit moved between the map and every map (its file: the map's wire names it again).
pub fn move_to(roots: &Roots, map_dir: &Path, r: &KitRef, to: Scope) -> anyhow::Result<KitRef> {
    let new = KitRef { scope: to, stem: r.stem.clone() };
    let (a, b) = (r.path(roots, map_dir), new.path(roots, map_dir));
    anyhow::ensure!(!b.exists(), "{}", tr("a kit of that name is there already", "комплект с таким именем уже есть"));
    std::fs::create_dir_all(b.parent().unwrap_or(map_dir))?;
    std::fs::rename(&a, &b).or_else(|_| std::fs::copy(&a, &b).and_then(|_| std::fs::remove_file(&a)))?;
    Ok(new)
}

// ------------------------------------------------------------------------- on the map

fn put_object(doc: &mut Document, hist: &mut History, file: &str, at: DVec3, heading: f64) -> i64 {
    let id = doc.new_id();
    let k = tile_of(at.truncate());
    if !doc.tiles.contains_key(&k) {
        return id;
    }
    hist.touch_tile(doc, k);
    let local = at.truncate() - tile_origin(k);
    let above = at.z - doc.ground(at.truncate());
    let t = doc.tiles.get_mut(&k).unwrap();
    t.tile.objects.push(MapObject { file: file.to_string(), id, pos: [local.x, local.y, above], rot: [crate::roads::norm_deg(heading), 0.0, 0.0], ..Default::default() });
    t.touch_content();
    id
}

fn put_spline(doc: &mut Document, hist: &mut History, s: MapSpline, at: DVec2) -> i64 {
    let k = tile_of(at);
    if !doc.tiles.contains_key(&k) {
        return s.id;
    }
    hist.touch_tile(doc, k);
    let id = s.id;
    let t = doc.tiles.get_mut(&k).unwrap();
    t.tile.splines.push(s);
    t.touch_content();
    id
}

/// The points of a line without those that lie on the straight between their neighbours.
fn simplified(pts: &[DVec3], tol: f64) -> Vec<DVec3> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let ab = b - a;
    let far = (1..pts.len() - 1)
        .map(|i| {
            let t = if ab.length_squared() > 1e-9 { ((pts[i] - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            (i, (a + ab * t).distance(pts[i]))
        })
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    if far.1 <= tol {
        return vec![a, b];
    }
    let mut left = simplified(&pts[..=far.0], tol);
    left.pop();
    left.extend(simplified(&pts[far.0..], tol));
    left
}

/// Whether a file is the kit's wire spline or mast.
fn of_kit(k: &KitDef, file: &str) -> bool {
    [&k.wire, &k.mast].iter().any(|f| !f.trim().is_empty() && f.trim().eq_ignore_ascii_case(file.trim()))
}

/// A copy of a road piece `off` metres to its right (an arc keeps its centre, as
/// `parallel::copy` makes them): its tile, the copy, where it starts. None where the bend is
/// too tight for it.
fn sideways(s: &MapSpline, org: DVec2, off: f64) -> Option<(MapSpline, DVec2)> {
    let h = s.heading.to_radians();
    let right = DVec2::new(h.cos(), -h.sin());
    let start = org + DVec2::new(s.pos[0], s.pos[1]) + right * off;
    let (radius, length) = if s.radius == 0.0 {
        (0.0, s.length)
    } else {
        let r = s.radius - off;
        if r == 0.0 || r.signum() != s.radius.signum() {
            return None;
        }
        (r, s.length * r / s.radius)
    };
    let scale = s.length / length.max(0.01);
    let no = tile_origin(tile_of(start));
    let c = MapSpline { pos: [start.x - no.x, start.y - no.y, s.pos[2]], radius, length, grad_start: s.grad_start * scale, grad_end: s.grad_end * scale, ..s.clone() };
    Some((c, start))
}

/// How new wire is shown with a kit: its wire splines, its masts.
#[derive(Clone, Copy)]
pub struct Use<'a> {
    pub kit: &'a KitDef,
    pub wires: bool,
    pub masts: bool,
}

/// The wire splines and masts of a new span, their IDCodes kept in it.
fn lay_span(doc: &mut Document, hist: &mut History, roots: &Roots, span: &mut Span, road: Option<RoadLane>, u: Use) {
    let k = u.kit;
    let wire = k.wire.trim().to_string();
    // (the wire spline's wires hang `mid` right of its line and `height` over it)
    let (mid, _, height) = k.profile(roots).unwrap_or((0.0, 0.0, 0.0));
    match road {
        Some(r) => {
            let Some(sp) = doc.spline(r.tile, r.id).cloned() else { return };
            if u.wires && !wire.is_empty() {
                // (a copy of the road piece: the wire follows its bends, gradients and cant)
                let base = MapSpline { file: wire.clone(), id: doc.new_id(), prev_id: 0, next_id: 0, mirror: false, terrain_align: None, terrain_align_flag: false, rules: Vec::new(), ..sp.clone() };
                if let Some((w, at)) = sideways(&base, tile_origin(r.tile), r.x - mid) {
                    span.laid.push(put_spline(doc, hist, w, at));
                }
            }
            if u.masts && !k.mast.trim().is_empty() && k.step >= 3.0 {
                let side = if r.x < 0.0 { -1.0 } else { 1.0 };
                let foot_x = r.x + side * k.reach;
                let curve = omsi_geometry::SplineCurve::from_map(&sp, tile_origin(r.tile));
                let mut s = k.step * 0.5;
                while s < sp.length {
                    let foot = curve.offset_point(s, foot_x, 0.0);
                    let over = curve.offset_point(s, r.x, 0.0);
                    let foot = DVec3::new(foot.x, foot.y, doc.ground(foot.truncate()));
                    // (its arm, toward its -y, reaches over the wire; the kit turns models made
                    // the other way)
                    let heading = crate::roads::heading_of(-(over - foot).truncate()) + k.turn;
                    span.laid.push(put_object(doc, hist, k.mast.trim(), foot, heading));
                    s += k.step;
                }
            }
        }
        None if u.wires && !wire.is_empty() => {
            let pts = simplified(&span.points, 0.05);
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let flat = (b - a).truncate().length();
                if flat < 0.05 {
                    continue;
                }
                let h = crate::roads::heading_of((b - a).truncate());
                let hr = h.to_radians();
                // (the spline's line where its wires hang over the span's points)
                let start = a.truncate() - DVec2::new(hr.cos(), -hr.sin()) * mid;
                let t = tile_of(start);
                let local = start - tile_origin(t);
                let grad = (b.z - a.z) / flat * 100.0;
                let s = MapSpline { file: wire.clone(), id: doc.new_id(), pos: [local.x, local.y, a.z - height], heading: h, length: flat, grad_start: grad, grad_end: grad, ..Default::default() };
                span.laid.push(put_spline(doc, hist, s, start));
            }
        }
        None => {}
    }
}

/// Take away objects and splines by IDCode (what a span laid, the marks put).
fn remove(doc: &mut Document, hist: &mut History, ids: &[i64]) {
    for id in ids {
        if let Some((k, _)) = doc.find_spline(*id) {
            hist.touch_tile(doc, k);
            doc.delete_spline(k, *id);
            continue;
        }
        let k = doc.tiles.iter().find(|(_, t)| t.tile.objects.iter().any(|o| o.id == *id)).map(|(k, _)| *k);
        if let Some(k) = k {
            hist.touch_tile(doc, k);
            doc.delete_object(k, *id);
        }
    }
}

/// The frogs and insulators put again where the wire has them now, as its kit names them;
/// their IDCodes kept in the wire.
fn put_marks(doc: &mut Document, hist: &mut History, roots: Option<&Roots>) {
    let Some(mut w) = doc.traffic.trolley.clone() else { return };
    remove(doc, hist, &w.marks);
    w.marks.clear();
    if let Some(kit) = roots.and_then(|r| w.kit.as_deref().and_then(|id| load(r, &doc.dir, id))) {
        let js = crate::trolley::joints(&w);
        // (the way the wire runs through a joint: along its first span there)
        let heading = |j: &crate::trolley::Joint| {
            let (i, far) = j.ends[0];
            let p = &w.spans[i].points;
            let d = if far { p[p.len() - 1] - p[p.len() - 2] } else { p[1] - p[0] };
            crate::roads::heading_of(d.truncate())
        };
        let mut ids = Vec::new();
        if !kit.frog.trim().is_empty() {
            for j in js.iter().filter(|j| j.ends.len() == 3) {
                ids.push(put_object(doc, hist, kit.frog.trim(), j.pos, heading(j)));
            }
        }
        if !kit.insulator.trim().is_empty() {
            for (p, _, _) in crate::trolley::section_breaks(&w, &js) {
                if let Some(j) = js.iter().find(|j| j.pos.distance(p) < 0.01) {
                    ids.push(put_object(doc, hist, kit.insulator.trim(), p, heading(j)));
                }
            }
        }
        w.marks = ids;
    }
    doc.traffic.trolley = Some(w);
}

/// The map's wire made `after` in one undo step, with what shows it: the new spans (their
/// places in `after`, and the road lane each runs over) get the kit's wire splines and masts,
/// what spans no longer there had laid goes, the frogs and insulators stand where the wire's
/// are.
pub fn apply(ed: &mut Editor, roots: Option<&Roots>, mut after: Option<Wire>, new: Vec<(usize, Option<RoadLane>)>, u: Option<Use>) {
    let Some(doc) = ed.doc.as_mut() else { return };
    let hist = &mut ed.history;
    hist.begin(tr("Trolleybus wire", "Контактная сеть"));
    hist.touch_traffic(doc);
    hist.touch_global(doc);
    let kept: std::collections::HashSet<i64> = after.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).collect();
    let gone: Vec<i64> = doc.traffic.trolley.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).filter(|id| !kept.contains(id)).collect();
    remove(doc, hist, &gone);
    // (the marks put last time go with the wire they stood on)
    if let (Some(a), Some(b)) = (after.as_mut(), doc.traffic.trolley.as_ref()) {
        a.marks = b.marks.clone();
    } else if after.is_none() {
        let old = doc.traffic.trolley.as_ref().map(|w| w.marks.clone()).unwrap_or_default();
        remove(doc, hist, &old);
    }
    if let (Some(w), Some(roots), Some(u)) = (after.as_mut(), roots, u) {
        for (i, road) in new {
            if let Some(span) = w.spans.get_mut(i) {
                lay_span(doc, hist, roots, span, road, u);
            }
        }
    }
    doc.traffic.trolley = after;
    put_marks(doc, hist, roots);
    doc.traffic.touch(crate::traffic::File::Trolley);
    hist.commit();
}

/// What is on the map from a kit as it was (`old`) made as it is now (`new`): its wire splines
/// and masts take the new files, the frogs and insulators are put again. How many changed.
pub fn renew(ed: &mut Editor, roots: &Roots, old: &KitDef, new: &KitDef) -> usize {
    let Some(doc) = ed.doc.as_mut() else { return 0 };
    let laid: std::collections::HashSet<i64> = doc.traffic.trolley.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).collect();
    let mut n = 0;
    ed.history.begin(tr("Renew the wire from its kit", "Обновить провод по комплекту"));
    ed.history.touch_traffic(doc);
    let keys: Vec<_> = doc.tiles.keys().copied().collect();
    for k in keys {
        let t = &doc.tiles[&k];
        let ours = |id: i64, file: &str| laid.contains(&id) && of_kit(old, file);
        if !t.tile.splines.iter().any(|s| ours(s.id, &s.file)) && !t.tile.objects.iter().any(|o| ours(o.id, &o.file)) {
            continue;
        }
        ed.history.touch_tile(doc, k);
        let t = doc.tiles.get_mut(&k).unwrap();
        for s in t.tile.splines.iter_mut().filter(|s| laid.contains(&s.id) && s.file.eq_ignore_ascii_case(old.wire.trim())) {
            if !new.wire.trim().is_empty() && s.file != new.wire.trim() {
                s.file = new.wire.trim().to_string();
                n += 1;
            }
        }
        for o in t.tile.objects.iter_mut().filter(|o| laid.contains(&o.id) && o.file.eq_ignore_ascii_case(old.mast.trim())) {
            if !new.mast.trim().is_empty() && o.file != new.mast.trim() {
                o.file = new.mast.trim().to_string();
                n += 1;
            }
        }
        t.touch_content();
    }
    put_marks(doc, &mut ed.history, Some(roots));
    doc.traffic.touch(crate::traffic::File::Trolley);
    ed.history.commit();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> KitDef {
        KitDef { name: "Test net".into(), wire: "Splines\\Net\\wire.sli".into(), mast: "Sceneryobjects\\Net\\mast.sco".into(), frog: "Sceneryobjects\\Net\\frog.sco".into(), insulator: String::new(), step: 30.0, reach: 2.5, turn: -90.0 }
    }

    /// The map maker's own wire spline: a pair of wires 0.6 m apart, 5.6 m up, `mid` across.
    fn content(roots: &Roots, mid: f64) {
        let dir = roots.content.join("Splines").join("Net");
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = String::from("[texture]\r\nw.tga\r\n\r\n");
        for c in [mid - 0.3, mid + 0.3] {
            s += "[profile]\r\n0\r\n\r\n";
            for (dx, z) in [(-0.006, 5.6), (0.006, 5.6)] {
                s += &format!("[profilepnt]\r\n{:.3}\r\n{z}\r\n0\r\n1\r\n\r\n", c + dx);
            }
        }
        std::fs::write(dir.join("wire.sli"), s).unwrap();
        let obj = roots.content.join("Sceneryobjects").join("Net");
        std::fs::create_dir_all(&obj).unwrap();
        for o in ["mast.sco", "frog.sco"] {
            std::fs::write(obj.join(o), "[mesh]\r\nx.o3d\r\n").unwrap();
        }
    }

    #[test]
    fn a_kit_reads_back_as_it_was_written_and_as_written_by_hand() {
        let k = kit();
        let text = k.text();
        assert_eq!(KitDef::parse(&text), k, "{text}");
        let hand = "[name]\nMy net\n\n[wire]\nSplines\\A\\w.sli  # mine\n\n[mast]\nSceneryobjects\\A\\m.sco\n\n[frog]\n\n[masts]\n40\n3\n";
        let h = KitDef::parse(hand);
        assert_eq!((h.name.as_str(), h.wire.as_str(), h.frog.as_str(), h.insulator.as_str(), h.step, h.reach, h.turn), ("My net", "Splines\\A\\w.sli", "", "", 40.0, 3.0, 0.0));
    }

    #[test]
    fn a_line_keeps_only_its_corners() {
        let pts: Vec<DVec3> = [(0.0, 0.0), (5.0, 0.01), (10.0, 0.0), (10.0, 10.0)].iter().map(|p| DVec3::new(p.0, p.1, 6.0)).collect();
        assert_eq!(simplified(&pts, 0.05), vec![pts[0], pts[2], pts[3]]);
    }

    #[test]
    fn kits_are_the_maps_or_every_maps() {
        let roots = crate::doc::tests::scratch("trolley_kits");
        let map = roots.content.join("maps").join("M");
        let r = KitRef { scope: Scope::Map, stem: stem_of("Test net") };
        save(&roots, &map, &r, &kit()).unwrap();
        assert_eq!(list(&roots, &map).len(), 1);
        let shared = move_to(&roots, &map, &r, Scope::Shared).unwrap();
        assert_eq!(shared.id(), "shared:Test_net");
        assert!(roots.content.join("openMapper/trolley/Test_net.cfg").is_file());
        assert_eq!(load(&roots, &map, &shared.id()), Some(kit()));
        assert_eq!(list(&roots, &roots.content.join("maps").join("Other"))[0].0, shared);
        // (files named that are not there are named)
        assert_eq!(kit().problems(&roots).len(), 3);
    }

    /// Laid over a road: the map maker's wire spline moved over the lane and their masts
    /// beside it, their frog where three spans meet (the kit names no insulator: none), all
    /// gone again with one undo; a kit changed renews what is laid.
    #[test]
    fn the_wire_is_shown_with_the_kits_files_and_undone_in_one_step() {
        let roots = crate::doc::tests::scratch("trolley_kit_lay");
        content(&roots, 0.4);
        let mut ed = Editor::default();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: "Splines\\road.sli".into(), id: 500, pos: [10.0, 50.0, 0.0], heading: 90.0, length: 100.0, ..Default::default() });
        let k = kit();
        assert!(k.problems(&roots).is_empty(), "{:?}", k.problems(&roots));
        assert_eq!(k.profile(&roots).map(|p| (p.0 * 10.0).round() / 10.0), Some(0.4));
        let r = KitRef { scope: Scope::Map, stem: stem_of(&k.name) };
        save(&roots, &doc.dir, &r, &k).unwrap();
        ed.doc = Some(doc);
        let at = |x: f64, y: f64| DVec3::new(x, y, 5.6);
        let spans = vec![
            (Span { section: 1, points: vec![at(10.0, 48.4), at(110.0, 48.4)], ..Default::default() }, Some(RoadLane { tile: (0, 0), id: 500, x: 1.6, height: 5.6 })),
            (Span { section: 1, points: vec![at(110.0, 48.4), at(150.0, 48.4)], ..Default::default() }, None),
            (Span { section: 2, points: vec![at(110.0, 48.4), at(130.0, 80.0)], ..Default::default() }, None),
        ];
        let mut w = crate::trolley::or_new(None);
        w.kit = Some(r.id());
        let added = crate::trolley::add_spans(&mut w, spans);
        apply(&mut ed, Some(&roots), Some(w), added, Some(Use { kit: &k, wires: true, masts: true }));
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        let count = |f: &str| t.objects.iter().filter(|o| o.file == f).count();
        let wires: Vec<&MapSpline> = t.splines.iter().filter(|s| s.file == k.wire).collect();
        assert_eq!((wires.len(), count(&k.mast), count(&k.frog)), (3, 3, 1));
        // (the road's copy moved right by the lane's 1.6 m less the spline's own 0.4 m:
        // heading 90 is east, its right is south)
        assert!((wires[0].pos[1] - 48.8).abs() < 1e-6 && (wires[0].pos[0] - 10.0).abs() < 1e-6, "{:?}", wires[0].pos);
        let w = doc.traffic.trolley.as_ref().unwrap();
        assert_eq!(w.marks.len(), 1);
        let back = crate::trolley::parse(&crate::trolley::text(w));
        assert_eq!((back.kit.clone(), back.marks.clone()), (w.kit.clone(), w.marks.clone()));
        // a span deleted takes what it laid; the frog goes with the branch
        let mut less = w.clone();
        less.spans.remove(2);
        crate::trolley::set(&mut ed, Some(&roots), Some(less));
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        assert_eq!(t.objects.iter().filter(|o| o.file == k.frog).count(), 0);
        ed.undo();
        // a kit changed: the laid masts take its new mast
        let mut k2 = k.clone();
        k2.mast = "Sceneryobjects\\Net\\frog.sco".into();
        assert_eq!(renew(&mut ed, &roots, &k, &k2), 3);
        ed.undo();
        ed.undo();
        let doc = ed.doc.as_ref().unwrap();
        assert!(doc.traffic.trolley.is_none());
        let t = &doc.tiles[&(0, 0)].tile;
        assert!(t.objects.is_empty() && t.splines.len() == 1, "{:?}", t.objects);
    }
}
