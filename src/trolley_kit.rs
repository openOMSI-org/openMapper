//! What shows the trolleybus wire on the map, made by openMapper itself as ordinary OMSI
//! content (no other wire or mast models needed): a wire spline (`Splines\openMapper\Trolley`,
//! two thin wires high over a road's lane, as trolleybus maps for OMSI 2 draw theirs, so
//! openOMSI also reads them as the wire), masts with an arm and a hanger over the wire, and
//! the frogs and section insulators where the wire has them (`Sceneryobjects\openMapper\Trolley`).
//!
//! Laying wire over a road puts a copy of each road piece of the wire type on the map and masts
//! beside it every so many metres; wire over a junction's lanes (or drawn by hand) is laid as
//! short straight wire pieces. What a span laid is kept with it (`Span::laid`) and goes with
//! it; the frogs and insulators are put again wherever the wire's are after every change. All
//! of it with the wire's file in one undo step.

use glam::{DVec2, DVec3, Mat4, Vec3};
use omsi_map::{MapObject, MapSpline};
use openmapper_mesh::primitives::Primitive;
use openmapper_mesh::{Material, Mesh, Model, Part};

use crate::doc::{tile_of, tile_origin, Document};
use crate::editable::Source;
use crate::editor::Editor;
use crate::history::History;
use crate::i18n::tr;
use crate::install::Roots;
use crate::trolley::{RoadLane, Span, Wire};

/// The folders of what openMapper makes for the wire.
const SPLINES: &str = "Splines\\openMapper\\Trolley";
const OBJECTS: &str = "Sceneryobjects\\openMapper\\Trolley";
/// A wire piece of a junction lane or a hand-drawn span hangs this far over its spline line.
const PIECE_HEIGHT: f64 = 5.0;

/// How the wire is shown on the map when it is laid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kit {
    /// Wire splines laid along each span.
    pub wires: bool,
    /// Masts beside the road (spans over roads).
    pub masts: bool,
    /// Metres between masts.
    pub mast_step: f64,
}

impl Default for Kit {
    fn default() -> Kit {
        Kit { wires: true, masts: true, mast_step: 30.0 }
    }
}

fn cm(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

// ------------------------------------------------------------------------- the files

/// The wire spline: two thin wires `spacing` apart, `x` right of the spline's line and
/// `height` over it. Written once; its relative path.
pub fn wire_spline(content: &std::path::Path, x: f64, height: f64, spacing: f64) -> anyhow::Result<String> {
    let stem = format!("Wire_x{}_h{}_s{}", cm(x), cm(height), cm(spacing));
    let rel = format!("{SPLINES}\\{stem}.sli");
    let path = omsi_cfg::resolve_path(content, &rel);
    if path.is_file() {
        return Ok(rel);
    }
    let dir = path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(dir.join("texture"))?;
    let tex = dir.join("texture").join("trolley_wire.tga");
    if !tex.is_file() {
        image::RgbaImage::from_pixel(4, 4, image::Rgba([46, 42, 38, 255])).save(&tex)?;
    }
    // (each wire a narrow V: seen from below and the side, thin enough that openOMSI takes the
    // pair for a wire when the map has no trolley.txt)
    let mut s = format!("File created with openMapper\n\n[friendlyname]\nTrolleybus wire {:+.2} m, {:.2} m up\n\n[texture]\ntrolley_wire.tga\n\n", x, height);
    for side in [-0.5, 0.5] {
        let c = x + side * spacing;
        s += "[profile]\n0\n\n";
        for (dx, dz, u) in [(-0.012, 0.0, 0.0), (0.0, -0.012, 0.5), (0.012, 0.0, 1.0)] {
            s += &format!("[profilepnt]\n{:.3}\n{:.3}\n{u}\n1\n\n", c + dx, height + dz);
        }
    }
    crate::doc::safe::write_atomic(&path, &omsi_cfg::encode_text(&s.replace('\n', "\r\n")))?;
    Ok(rel)
}

fn shape(p: Primitive, m: Mat4, material: u32) -> Mesh {
    let mut mesh = p.build();
    mesh.transform(m);
    for f in &mut mesh.faces {
        f.material = material;
        f.smooth = p.round();
    }
    mesh
}

fn source(name: &str, parts: Vec<(Mesh, u32)>, materials: Vec<Material>) -> Source {
    let mut body = Mesh::default();
    for (m, _) in &parts {
        body.append(m, 0);
    }
    Source { name: name.into(), model: Model { parts: vec![Part::new("Body", body)], materials }, ..Default::default() }
}

fn steel() -> Material {
    Material::colored("Steel", [0.40, 0.42, 0.44, 1.0])
}

fn dark() -> Material {
    Material::colored("Fittings", [0.13, 0.13, 0.13, 1.0])
}

/// A mast: a pole with an arm reaching `arm` metres toward -y, the wire's hanger at its end
/// `height` metres up (the pair across the arm).
pub fn mast(arm: f64, height: f64, spacing: f64) -> Source {
    let (arm, h, sp) = (arm as f32, height as f32, spacing as f32);
    let pi2 = std::f32::consts::FRAC_PI_2;
    source(
        "Trolleybus mast",
        vec![
            (shape(Primitive::Cone { radius_bottom: 0.14, radius_top: 0.09, height: h + 0.8, segments: 12 }, Mat4::IDENTITY, 0), 0),
            (shape(Primitive::Cylinder { radius: 0.045, height: arm + 0.2, segments: 8 }, Mat4::from_translation(Vec3::new(0.0, 0.1, h + 0.4)) * Mat4::from_rotation_x(pi2), 0), 0),
            (shape(Primitive::Cube { size: Vec3::new(0.03, 0.03, 0.35) }, Mat4::from_translation(Vec3::new(0.0, -arm, h + 0.22)), 1), 1),
            (shape(Primitive::Cube { size: Vec3::new(0.08, sp + 0.2, 0.05) }, Mat4::from_translation(Vec3::new(0.0, -arm, h + 0.05)), 1), 1),
        ],
        vec![steel(), dark()],
    )
}

/// A frog: the housing where a wire branches, along y.
pub fn frog(spacing: f64) -> Source {
    let sp = spacing as f32;
    source(
        "Trolleybus frog",
        vec![
            (shape(Primitive::Cube { size: Vec3::new(sp + 0.3, 0.9, 0.1) }, Mat4::from_translation(Vec3::new(0.0, 0.0, 0.1)), 0), 0),
            (shape(Primitive::Cube { size: Vec3::new(0.1, 1.4, 0.06) }, Mat4::from_translation(Vec3::new(0.0, 0.0, 0.18)), 0), 0),
        ],
        vec![dark()],
    )
}

/// A section insulator: a light bar in each wire, along y.
pub fn insulator(spacing: f64) -> Source {
    let sp = spacing as f32;
    source(
        "Trolleybus section insulator",
        vec![
            (shape(Primitive::Cube { size: Vec3::new(0.07, 1.0, 0.07) }, Mat4::from_translation(Vec3::new(-sp * 0.5, 0.0, 0.04)), 0), 0),
            (shape(Primitive::Cube { size: Vec3::new(0.07, 1.0, 0.07) }, Mat4::from_translation(Vec3::new(sp * 0.5, 0.0, 0.04)), 0), 0),
            (shape(Primitive::Cube { size: Vec3::new(sp + 0.2, 0.06, 0.04) }, Mat4::from_translation(Vec3::new(0.0, 0.0, 0.12)), 1), 1),
        ],
        vec![Material::colored("Insulator", [0.88, 0.85, 0.76, 1.0]), dark()],
    )
}

/// An object of the kit written once under `stem`; its relative path.
fn object(roots: &Roots, stem: &str, make: impl FnOnce() -> Source) -> Option<String> {
    let rel = format!("{OBJECTS}\\{stem}\\{stem}.sco");
    if !omsi_cfg::resolve_path(&roots.content, &rel).is_file() {
        if let Err(e) = crate::editable::save(roots, &rel, &make()) {
            log::warn!("{rel}: {e}");
            return None;
        }
    }
    Some(rel)
}

/// Whether a file is one the kit made (the map's wire, its masts, frogs and insulators).
pub fn is_kit(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    // (with the folder's backslash: a map called Trolley… has its junctions beside it)
    [SPLINES, OBJECTS].iter().any(|d| f.starts_with(&format!("{}\\", d.to_ascii_lowercase())))
}

/// A frog's or an insulator's object (put again after every change of the wire).
fn is_mark(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    let base = OBJECTS.to_ascii_lowercase();
    f.starts_with(&format!("{base}\\frog_")) || f.starts_with(&format!("{base}\\insulator_"))
}

/// Wire pieces and masts of the kit on the map that no span of the wire names (the wire's
/// file taken away by hand, or the span deleted outside openMapper): where they stand.
pub fn orphans(doc: &Document) -> Vec<(crate::doc::Sel, DVec2)> {
    let named: std::collections::HashSet<i64> = doc.traffic.trolley.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).collect();
    let mut out = Vec::new();
    for (k, t) in &doc.tiles {
        let org = tile_origin(*k);
        for o in t.tile.objects.iter().filter(|o| is_kit(&o.file) && !is_mark(&o.file) && !named.contains(&o.id)) {
            out.push((crate::doc::Sel::Object(*k, o.id), org + DVec2::new(o.pos[0], o.pos[1])));
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted && is_kit(&s.file) && !named.contains(&s.id)) {
            out.push((crate::doc::Sel::Spline(*k, s.id), org + DVec2::new(s.pos[0], s.pos[1])));
        }
    }
    out
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

/// The wire splines and masts of a new span, their IDCodes kept in it.
fn lay_span(doc: &mut Document, hist: &mut History, roots: &Roots, assets: &mut crate::assets::Assets, span: &mut Span, road: Option<RoadLane>, kit: Kit, spacing: f64) {
    match road {
        Some(r) => {
            let Some(sp) = doc.spline(r.tile, r.id).cloned() else { return };
            if kit.wires {
                if let Ok(file) = wire_spline(&roots.content, r.x, r.height, spacing) {
                    let id = doc.new_id();
                    // (a copy of the road piece: the wire follows its bends, gradients and cant)
                    let w = MapSpline { file, id, prev_id: 0, next_id: 0, mirror: false, terrain_align: None, terrain_align_flag: false, rules: Vec::new(), ..sp.clone() };
                    span.laid.push(put_spline(doc, hist, w, tile_origin(r.tile) + DVec2::new(sp.pos[0], sp.pos[1])));
                }
            }
            if kit.masts && kit.mast_step >= 5.0 {
                let half = assets.spline(&sp.file).map(|m| m.half_width as f64).unwrap_or(4.0);
                let side = if r.x < 0.0 { -1.0 } else { 1.0 };
                let edge = side * (half + 0.6);
                let arm = (edge - r.x).abs();
                let Some(file) = object(roots, &format!("Mast_a{}_h{}_s{}", cm((arm * 4.0).round() / 4.0), cm(r.height), cm(spacing)), || mast((arm * 4.0).round() / 4.0, r.height, spacing)) else { return };
                let curve = omsi_geometry::SplineCurve::from_map(&sp, tile_origin(r.tile));
                let mut s = kit.mast_step * 0.5;
                while s < sp.length {
                    let foot = curve.offset_point(s, edge, 0.0);
                    let over = curve.offset_point(s, r.x, 0.0);
                    let foot = DVec3::new(foot.x, foot.y, doc.ground(foot.truncate()));
                    // (its arm, toward its -y, reaches over the wire)
                    let heading = crate::roads::heading_of(-(over - foot).truncate());
                    span.laid.push(put_object(doc, hist, &file, foot, heading));
                    s += kit.mast_step;
                }
            }
        }
        None if kit.wires => {
            let Ok(file) = wire_spline(&roots.content, 0.0, PIECE_HEIGHT, spacing) else { return };
            let pts = simplified(&span.points, 0.05);
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let flat = (b - a).truncate().length();
                if flat < 0.05 {
                    continue;
                }
                let k = tile_of(a.truncate());
                let local = a.truncate() - tile_origin(k);
                let grad = (b.z - a.z) / flat * 100.0;
                let id = doc.new_id();
                let s = MapSpline { file: file.clone(), id, pos: [local.x, local.y, a.z - PIECE_HEIGHT], heading: crate::roads::heading_of((b - a).truncate()), length: flat, grad_start: grad, grad_end: grad, ..Default::default() };
                span.laid.push(put_spline(doc, hist, s, a.truncate()));
            }
        }
        None => {}
    }
}

/// Take away objects and splines by IDCode (what a span had laid).
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

/// The frogs and insulators put again where the wire has them now.
fn put_marks(doc: &mut Document, hist: &mut History, roots: Option<&Roots>) {
    let keys: Vec<_> = doc.tiles.iter().filter(|(_, t)| t.tile.objects.iter().any(|o| is_mark(&o.file))).map(|(k, _)| *k).collect();
    for k in keys {
        hist.touch_tile(doc, k);
        let t = doc.tiles.get_mut(&k).unwrap();
        t.tile.objects.retain(|o| !is_mark(&o.file));
        t.touch_content();
    }
    let (Some(roots), Some(w)) = (roots, doc.traffic.trolley.clone()) else { return };
    let js = crate::trolley::joints(&w);
    // (the way the wire runs through a joint: along its first span there)
    let heading = |j: &crate::trolley::Joint| {
        let (i, far) = j.ends[0];
        let p = &w.spans[i].points;
        let d = if far { p[p.len() - 1] - p[p.len() - 2] } else { p[1] - p[0] };
        crate::roads::heading_of(d.truncate())
    };
    let sp = w.spacing;
    let frog_file = object(roots, &format!("Frog_s{}", cm(sp)), || frog(sp));
    let ins_file = object(roots, &format!("Insulator_s{}", cm(sp)), || insulator(sp));
    for j in js.iter().filter(|j| j.ends.len() == 3) {
        if let Some(f) = &frog_file {
            put_object(doc, hist, f, j.pos, heading(j));
        }
    }
    let breaks = crate::trolley::section_breaks(&w, &js);
    for (p, _, _) in breaks {
        if let (Some(f), Some(j)) = (&ins_file, js.iter().find(|j| j.pos.distance(p) < 0.01)) {
            put_object(doc, hist, f, p, heading(j));
        }
    }
}

/// The map's wire made `after` in one undo step, with what shows it: the new spans (their
/// places in `after`, and the road lane each runs over) get their wire splines and masts as
/// `kit` says, what spans no longer there had laid goes, the frogs and insulators stand where
/// the wire's are.
pub fn apply(ed: &mut Editor, roots: Option<&Roots>, assets: Option<&mut crate::assets::Assets>, mut after: Option<Wire>, new: Vec<(usize, Option<RoadLane>)>, kit: Option<Kit>) {
    let Some(doc) = ed.doc.as_mut() else { return };
    let hist = &mut ed.history;
    hist.begin(tr("Trolleybus wire", "Контактная сеть"));
    hist.touch_traffic(doc);
    hist.touch_global(doc);
    let kept: std::collections::HashSet<i64> = after.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).collect();
    let gone: Vec<i64> = doc.traffic.trolley.iter().flat_map(|w| w.spans.iter().flat_map(|s| s.laid.iter().copied())).filter(|id| !kept.contains(id)).collect();
    remove(doc, hist, &gone);
    if let (Some(w), Some(roots), Some(assets), Some(kit)) = (after.as_mut(), roots, assets, kit) {
        let spacing = w.spacing;
        for (i, road) in new {
            if let Some(span) = w.spans.get_mut(i) {
                lay_span(doc, hist, roots, assets, span, road, kit, spacing);
            }
        }
        omsi_cfg::content_changed();
    }
    doc.traffic.trolley = after;
    doc.traffic.touch(crate::traffic::File::Trolley);
    put_marks(doc, hist, roots);
    hist.commit();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_keeps_only_its_corners() {
        let pts: Vec<DVec3> = [(0.0, 0.0), (5.0, 0.01), (10.0, 0.0), (10.0, 10.0)].iter().map(|p| DVec3::new(p.0, p.1, 6.0)).collect();
        assert_eq!(simplified(&pts, 0.05), vec![pts[0], pts[2], pts[3]]);
    }

    /// The wire spline is the pair openOMSI reads as a wire, where it was asked for.
    #[test]
    fn the_wire_spline_reads_as_a_pair_of_wires() {
        let content = crate::doc::tests::scratch("trolley_kit").content;
        let rel = wire_spline(&content, 1.6, 5.6, 0.6).unwrap();
        let def = omsi_scenery::sli::Spline::load(&omsi_cfg::resolve_path(&content, &rel)).unwrap();
        let (mid, spacing, h) = crate::trolley::wire_profile(&def).unwrap();
        assert!((mid - 1.6).abs() < 1e-3 && (spacing - 0.6).abs() < 1e-3 && (h - 5.596).abs() < 0.01, "{mid} {spacing} {h}");
        assert!(is_kit(&rel) && !is_mark(&rel));
        assert!(!is_kit("Sceneryobjects\\openMapper\\TrolleyTest\\junction_1.sco"));
    }

    /// Laid over a road: the wire's copy of the road and masts beside it, the frog where three
    /// spans meet, all gone again with one undo.
    #[test]
    fn the_wire_is_shown_and_undone_in_one_step() {
        let roots = crate::doc::tests::scratch("trolley_kit_lay");
        let mut assets = crate::assets::Assets::new(roots.omsi.clone(), roots.content.clone());
        let mut ed = Editor::default();
        let mut doc = crate::selection::tests::flat_map(1);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.splines.push(MapSpline { file: "Splines\\road.sli".into(), id: 500, pos: [10.0, 50.0, 0.0], heading: 90.0, length: 100.0, ..Default::default() });
        ed.doc = Some(doc);
        let at = |x: f64, y: f64| DVec3::new(x, y, 5.6);
        let spans = vec![
            (Span { section: 1, points: vec![at(10.0, 51.6), at(110.0, 51.6)], ..Default::default() }, Some(RoadLane { tile: (0, 0), id: 500, x: 1.6, height: 5.6 })),
            (Span { section: 1, points: vec![at(110.0, 51.6), at(150.0, 51.6)], ..Default::default() }, None),
            (Span { section: 2, points: vec![at(110.0, 51.6), at(130.0, 80.0)], ..Default::default() }, None),
        ];
        let mut w = crate::trolley::or_new(None);
        let added = crate::trolley::add_spans(&mut w, spans);
        apply(&mut ed, Some(&roots), Some(&mut assets), Some(w), added, Some(Kit::default()));
        let doc = ed.doc.as_ref().unwrap();
        let t = &doc.tiles[&(0, 0)].tile;
        let wires = t.splines.iter().filter(|s| is_kit(&s.file)).count();
        let masts = t.objects.iter().filter(|o| o.file.contains("Mast_")).count();
        let frogs = t.objects.iter().filter(|o| o.file.contains("Frog_")).count();
        let insulators = t.objects.iter().filter(|o| o.file.contains("Insulator_")).count();
        assert_eq!((wires, masts, frogs, insulators), (3, 3, 1, 1));
        let w = doc.traffic.trolley.as_ref().unwrap();
        assert_eq!(w.spans[0].laid.len(), 4);
        assert!(crate::trolley::parse(&crate::trolley::text(w)).spans[0].laid == w.spans[0].laid);
        assert!(omsi_cfg::resolve_path(&roots.content, "Sceneryobjects\\openMapper\\Trolley\\Frog_s60\\Frog_s60.sco").is_file());
        // a span deleted takes what it laid with it
        let mut less = w.clone();
        less.spans.remove(0);
        crate::trolley::set(&mut ed, Some(&roots), Some(less));
        let t = &ed.doc.as_ref().unwrap().tiles[&(0, 0)].tile;
        assert_eq!(t.objects.iter().filter(|o| o.file.contains("Mast_")).count(), 0);
        assert_eq!(t.splines.iter().filter(|s| is_kit(&s.file)).count(), 2);
        ed.undo();
        ed.undo();
        let doc = ed.doc.as_ref().unwrap();
        assert!(doc.traffic.trolley.is_none());
        assert!(!doc.tiles[&(0, 0)].tile.objects.iter().any(|o| is_kit(&o.file)) && !doc.tiles[&(0, 0)].tile.splines.iter().any(|s| is_kit(&s.file)));
    }
}
