//! What can be done to whole objects as meshes: make one editable (a copy of its own),
//! separate its parts into objects, join objects into one, and booleans (union,
//! difference, intersection, slice) between any objects, as Blender's Join and Boolean or
//! Roblox's solid modelling do. Each is one undo step of the map; the objects' files are new
//! ones, so nothing that was there before is changed.

use glam::{DVec3, Mat4, Vec3};
use omsi_map::MapObject;
use openmapper_mesh::csg::BoolOp;
use openmapper_mesh::{Mesh, Model, Part};

use crate::doc::{tile_of, tile_origin, Sel, TileKey};
use crate::editable::{self, Source};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::install::Roots;
use crate::library::Library;

/// A boolean as the interface offers it: the three of CSG and the slice, which keeps both
/// what is outside the cutters and what is inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bool {
    Union,
    Difference,
    Intersection,
    Slice,
}

impl Bool {
    pub const ALL: [Bool; 4] = [Bool::Union, Bool::Difference, Bool::Intersection, Bool::Slice];

    pub fn label(self) -> &'static str {
        match self {
            Bool::Union => tr("Union", "Объединение"),
            Bool::Difference => tr("Difference (cut out)", "Вычитание (вырезать)"),
            Bool::Intersection => tr("Intersection", "Пересечение"),
            Bool::Slice => tr("Slice (keep both pieces)", "Разрезать (оставить обе части)"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Bool::Union => tr("One object of all the selected, their insides gone", "Один объект из всех выделенных, без внутренних стенок"),
            Bool::Difference => tr("The other selected objects cut out of the one shown in the properties", "Остальные выделенные объекты вырезаются из того, что показан в свойствах"),
            Bool::Intersection => tr("Only what all the selected share", "Только общая часть всех выделенных"),
            Bool::Slice => tr("The first object cut by the others into what is outside them and what is inside", "Первый объект разрезается остальными на то, что снаружи, и то, что внутри"),
        }
    }
}

/// An object on the map: where it is and how it stands.
struct Placed {
    sel: Sel,
    k: TileKey,
    obj: MapObject,
    world: DVec3,
    abs: bool,
}

fn placed(ed: &Editor, lib: &Library, sel: Sel) -> Option<Placed> {
    let Sel::Object(k, id) = sel else { return None };
    let doc = ed.doc.as_ref()?;
    let obj = doc.object(k, id)?.clone();
    if obj.parent_id.is_some() {
        return None;
    }
    let abs = lib.find(&obj.file).map(|a| a.abs_height).unwrap_or(false);
    let world = doc.object_world(k, &obj, abs);
    Some(Placed { sel, k, obj, world, abs })
}

fn map_name(ed: &Editor) -> String {
    ed.doc.as_ref().and_then(|d| d.dir.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Shared".into())
}

/// A new object's name and folder for a copy of `rel`.
fn new_for(roots: &Roots, ed: &Editor, rel: &str, suffix: &str) -> (String, std::path::PathBuf) {
    let stem = rel.rsplit('\\').next().and_then(|f| f.strip_suffix(".sco").or_else(|| f.rsplit_once('.').map(|x| x.0))).unwrap_or("Object");
    let stem = crate::scaled::base_of(stem).0;
    let r = editable::new_rel(roots, &map_name(ed), &format!("{stem}{suffix}"));
    let (dir, _) = editable::folder_of(roots, &r);
    (r, dir)
}

/// Point this map object at another file (one undo step must be open).
fn set_file(ed: &mut Editor, k: TileKey, id: i64, rel: &str) {
    let Some(doc) = ed.doc.as_mut() else { return };
    ed.history.touch_tile(doc, k);
    if let Some(o) = doc.object_mut(k, id) {
        o.file = rel.to_string();
    }
    if let Some(t) = doc.tiles.get_mut(&k) {
        t.touch_content();
    }
}

/// A new map object of `rel` standing at `world`, turned by `rot` (one undo step open).
fn add_object(ed: &mut Editor, rel: &str, world: DVec3, rot: [f64; 3], abs: bool) -> Option<Sel> {
    let doc = ed.doc.as_mut()?;
    let k = tile_of(world.truncate());
    if !doc.tiles.contains_key(&k) {
        return None;
    }
    ed.history.touch_tile(doc, k);
    ed.history.touch_global(doc);
    let id = doc.new_id();
    let org = tile_origin(k);
    let z = if abs { world.z } else { world.z - doc.ground(world.truncate()) };
    let t = doc.tiles.get_mut(&k)?;
    t.tile.objects.push(MapObject { file: rel.to_string(), id, pos: [world.x - org.x, world.y - org.y, z], rot, ..Default::default() });
    t.touch_content();
    Some(Sel::Object(k, id))
}

fn delete_object(ed: &mut Editor, k: TileKey, id: i64) {
    let Some(doc) = ed.doc.as_mut() else { return };
    ed.history.touch_tile(doc, k);
    ed.history.touch_global(doc);
    doc.delete_object(k, id);
}

/// The selected object's editable source: the file it has when it is an openMapper object
/// already, or a copy of its type made beside the map's own objects. The map object keeps the
/// file it had: [`point_at`] is what hands it the copy, so a Tab pressed by mistake can be
/// taken back with [`discard_copy`] and leave neither a copy nor a changed map.
pub fn copy_for_edit(ed: &mut Editor, roots: &Roots, lib: &Library, sel: Sel) -> anyhow::Result<String> {
    let p = placed(ed, lib, sel).ok_or_else(|| anyhow::anyhow!("{}", tr("select an object first", "сначала выделите объект")))?;
    if editable::is_editable(roots, &p.obj.file) {
        return Ok(p.obj.file);
    }
    let (rel, dir) = new_for(roots, ed, &p.obj.file, "");
    let src = editable::source_of(roots, &p.obj.file, &dir)?;
    editable::save(roots, &rel, &src)?;
    // (the catalogue need not be read again, and the models need not be dropped, for a copy
    // made only to be edited: a Tab pressed by mistake should cost nothing at all)
    crate::hot::own_change();
    Ok(rel)
}

/// The map object pointed at `rel`, one of openMapper's own (one undo step).
pub fn point_at(ed: &mut Editor, sel: Sel, rel: &str) {
    let Sel::Object(k, id) = sel else { return };
    ed.history.begin(tr("Make editable", "Сделать редактируемым"));
    set_file(ed, k, id, rel);
    ed.history.commit();
}

/// A copy made only to read an object's mesh, and left as it was: its folder goes. Nothing
/// points at it, so nothing else changes.
pub fn discard_copy(roots: &Roots, rel: &str) {
    let (dir, _) = editable::folder_of(roots, rel);
    let _ = std::fs::remove_dir_all(dir);
    // (its going is not the catalogue's news either)
    crate::hot::own_change();
}

/// The selected object made an openMapper object of its own (a copy of its type): what its
/// mesh, materials and lamps are can then be edited. Its file, or the new one.
pub fn make_editable(ed: &mut Editor, roots: &Roots, lib: &Library, sel: Sel) -> anyhow::Result<String> {
    let was = placed(ed, lib, sel).map(|p| p.obj.file);
    let rel = copy_for_edit(ed, roots, lib, sel)?;
    if !was.as_deref().map(|w| w.eq_ignore_ascii_case(&rel)).unwrap_or(false) {
        point_at(ed, sel, &rel);
    }
    Ok(rel)
}

/// Its own copy for this object, when other objects on the map share its type (an edit would
/// change them all).
pub fn make_single(ed: &mut Editor, roots: &Roots, lib: &Library, sel: Sel) -> anyhow::Result<String> {
    let p = placed(ed, lib, sel).ok_or_else(|| anyhow::anyhow!("{}", tr("select an object first", "сначала выделите объект")))?;
    let (rel, dir) = new_for(roots, ed, &p.obj.file, "");
    let src = editable::source_of(roots, &p.obj.file, &dir)?;
    editable::save(roots, &rel, &src)?;
    ed.history.begin(tr("Make single", "Отдельная копия"));
    set_file(ed, p.k, p.obj.id, &rel);
    ed.history.commit();
    Ok(rel)
}

/// How many objects on the map are of type `rel`.
pub fn users(ed: &Editor, rel: &str) -> usize {
    ed.doc.as_ref().map(|d| d.tiles.values().flat_map(|t| t.tile.objects.iter().chain(t.tile.attach_objects.iter())).filter(|o| o.file.eq_ignore_ascii_case(rel)).count()).unwrap_or(0)
}

/// A part moved so that its base's middle is at its origin; how far it was moved.
fn centre_part(part: &mut Part) -> Vec3 {
    let Some((lo, hi)) = part.mesh.bounds() else { return Vec3::ZERO };
    let c = Vec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, lo.z);
    part.mesh.transform(Mat4::from_translation(-c));
    c
}

/// Each part of the selected object an object of its own, standing where it stood (its
/// origin at the middle of its base). The objects made.
pub fn separate_parts(ed: &mut Editor, roots: &Roots, lib: &Library, sel: Sel) -> anyhow::Result<Vec<Sel>> {
    let p = placed(ed, lib, sel).ok_or_else(|| anyhow::anyhow!("{}", tr("select an object first", "сначала выделите объект")))?;
    let (_, tmp) = new_for(roots, ed, &p.obj.file, "");
    let src = editable::source_of(roots, &p.obj.file, &tmp)?;
    if src.model.parts.len() < 2 {
        anyhow::bail!("{}", tr("the object has only one part", "у объекта только одна часть"));
    }
    let place = editable::placement(p.world, p.obj.rot);
    let mut made = Vec::new();
    ed.history.begin(tr("Separate parts", "Разделить на части"));
    for (i, part) in src.model.parts.iter().enumerate() {
        let mut one = Source { name: part.name.clone(), abs_height: src.abs_height, collision: src.collision, ..Default::default() };
        let mut part = part.clone();
        let shift = centre_part(&mut part);
        let (mats, remap) = used_materials(&src.model, &part.mesh);
        for f in &mut part.mesh.faces {
            f.material = remap[f.material as usize];
        }
        one.model = Model { parts: vec![part.clone()], materials: mats };
        one.part_vars = src.part_vars.iter().filter(|v| v.part == part.name).cloned().collect();
        // (the lamps stay with the first part)
        if i == 0 {
            one.lights = src.lights.iter().map(|l| editable::Light { pos: l.pos - shift, ..l.clone() }).collect();
            one.signal = src.signal.clone();
            one.extra = src.extra.clone();
        }
        let (rel, dir) = new_for(roots, ed, &p.obj.file, &format!("_{}", editable::safe_name(&part.name)));
        if dir != tmp {
            copy_dir(&tmp.join("texture"), &dir.join("texture"));
            copy_dir(&tmp.join("script"), &dir.join("script"));
        }
        editable::save(roots, &rel, &one)?;
        let world = place.transform_point3(shift.as_dvec3());
        let s = if i == 0 {
            // (the first takes the object's place on the map)
            let Some(doc) = ed.doc.as_mut() else { break };
            ed.history.touch_tile(doc, p.k);
            if let Some(o) = doc.object_mut(p.k, p.obj.id) {
                o.file = rel.clone();
            }
            crate::selection::place_object_at(doc, p.k, p.obj.id, world.truncate());
            if let Some((k, _)) = crate::selection::object_by_id(doc, p.obj.id) {
                let ground = doc.ground(world.truncate());
                if let Some(o) = doc.object_mut(k, p.obj.id) {
                    o.pos[2] = if p.abs { world.z } else { world.z - ground };
                }
                if let Some(t) = doc.tiles.get_mut(&k) {
                    t.touch_content();
                }
                Some(Sel::Object(k, p.obj.id))
            } else {
                None
            }
        } else {
            add_object(ed, &rel, world, p.obj.rot, p.abs)
        };
        made.extend(s);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    ed.history.commit();
    ed.set_selection(made.clone());
    Ok(made)
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for e in rd.flatten() {
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dst);
        } else if !dst.exists() {
            let _ = std::fs::copy(e.path(), &dst);
        }
    }
}

/// The materials a mesh uses, and where each of the model's went.
fn used_materials(model: &Model, mesh: &Mesh) -> (Vec<openmapper_mesh::Material>, Vec<u32>) {
    let mut remap = vec![0u32; model.materials.len().max(1)];
    let mut out = Vec::new();
    for (i, m) in model.materials.iter().enumerate() {
        if mesh.faces.iter().any(|f| f.material as usize == i) {
            remap[i] = out.len() as u32;
            out.push(m.clone());
        }
    }
    if out.is_empty() {
        out.push(openmapper_mesh::Material::default());
    }
    (out, remap)
}

/// The selection's objects: the one shown in the properties first.
fn selected_objects(ed: &Editor, lib: &Library) -> Vec<Placed> {
    let mut sels = ed.selection();
    if let Some(i) = ed.selected.and_then(|s| sels.iter().position(|x| *x == s)) {
        sels.swap(0, i);
    }
    sels.into_iter().filter_map(|s| placed(ed, lib, s)).collect()
}

/// All the selected objects joined into the one shown in the properties (Blender's Join):
/// one object of all their parts, the others gone from the map.
pub fn join(ed: &mut Editor, roots: &Roots, lib: &Library) -> anyhow::Result<Sel> {
    let objs = selected_objects(ed, lib);
    if objs.len() < 2 {
        anyhow::bail!("{}", tr("select two or more objects to join", "выделите два или больше объектов, чтобы объединить"));
    }
    let a = &objs[0];
    let (rel, dir) = new_for(roots, ed, &a.obj.file, "");
    let mut src = editable::source_of(roots, &a.obj.file, &dir)?;
    let pa = editable::placement(a.world, a.obj.rot);
    for b in &objs[1..] {
        let other = editable::source_of(roots, &b.obj.file, &dir)?;
        editable::merge_into(&mut src, pa, &other, editable::placement(b.world, b.obj.rot));
    }
    editable::save(roots, &rel, &src)?;
    ed.history.begin(tr("Join objects", "Объединить объекты"));
    set_file(ed, a.k, a.obj.id, &rel);
    for b in &objs[1..] {
        delete_object(ed, b.k, b.obj.id);
    }
    ed.history.commit();
    ed.set_selection(vec![a.sel]);
    Ok(a.sel)
}

/// One mesh of all of a source's parts (modifiers applied).
fn whole(src: &Source) -> Mesh {
    let mut m = Mesh::default();
    for (_, mesh) in src.model.evaluated(None) {
        m.append(&mesh, 0);
    }
    m
}

/// A boolean of the selected objects: the one shown in the properties against all the
/// others. The result takes its place; the others go unless `keep` (for Slice the inside
/// becomes an object of its own beside it).
pub fn boolean(ed: &mut Editor, roots: &Roots, lib: &Library, op: Bool, keep: bool) -> anyhow::Result<Vec<Sel>> {
    let objs = selected_objects(ed, lib);
    if objs.len() < 2 {
        anyhow::bail!("{}", tr("select two or more objects: the one shown in the properties and what acts on it", "выделите два или больше объектов: тот, что в свойствах, и те, что на него действуют"));
    }
    let a = &objs[0];
    let (rel, dir) = new_for(roots, ed, &a.obj.file, "");
    let src_a = editable::source_of(roots, &a.obj.file, &dir)?;
    let pa = editable::placement(a.world, a.obj.rot);
    // the cutters' meshes in the first one's frame, their materials after its own
    let mut materials = src_a.model.materials.clone();
    let mut cutter = Mesh::default();
    for b in &objs[1..] {
        let other = editable::source_of(roots, &b.obj.file, &dir)?;
        let mut m = whole(&other);
        m.transform((pa.inverse() * editable::placement(b.world, b.obj.rot)).as_mat4());
        let offset = materials.len() as u32;
        materials.extend(other.model.materials.iter().cloned());
        let mut one = Mesh::default();
        one.append(&m, offset);
        // (several cutters: one after the other, so each counts whole)
        cutter = if cutter.is_empty() { one } else { openmapper_mesh::csg::boolean(&cutter, &one, BoolOp::Union) };
    }
    let base = whole(&src_a);
    let (result, inside) = match op {
        Bool::Union => (openmapper_mesh::csg::boolean(&base, &cutter, BoolOp::Union), None),
        Bool::Difference => (openmapper_mesh::csg::boolean(&base, &cutter, BoolOp::Difference), None),
        Bool::Intersection => (openmapper_mesh::csg::boolean(&base, &cutter, BoolOp::Intersection), None),
        Bool::Slice => {
            let (out, inn) = openmapper_mesh::csg::slice(&base, &cutter);
            (out, Some(inn))
        }
    };
    if result.is_empty() && inside.as_ref().map(|m| m.is_empty()).unwrap_or(true) {
        anyhow::bail!("{}", tr("nothing is left: the objects do not overlap that way", "ничего не осталось: объекты так не пересекаются"));
    }
    let make = |mesh: Mesh, name: &str| {
        let mut s = Source { name: name.to_string(), abs_height: src_a.abs_height, collision: src_a.collision, lights: src_a.lights.clone(), ..Default::default() };
        s.model = Model { parts: vec![Part::new(name, mesh)], materials: materials.clone() };
        editable::merge_materials(&mut s.model);
        s
    };
    let name = src_a.name.clone();
    editable::save(roots, &rel, &make(result, &name))?;
    ed.history.begin(op.label());
    set_file(ed, a.k, a.obj.id, &rel);
    let mut out = vec![a.sel];
    if let Some(inn) = inside.filter(|m| !m.is_empty()) {
        let (rel2, dir2) = new_for(roots, ed, &a.obj.file, "_inside");
        copy_dir(&dir.join("texture"), &dir2.join("texture"));
        editable::save(roots, &rel2, &make(inn, &format!("{name} (inside)")))?;
        out.extend(add_object(ed, &rel2, a.world, a.obj.rot, a.abs));
    }
    if !keep {
        for b in &objs[1..] {
            delete_object(ed, b.k, b.obj.id);
        }
    }
    ed.history.commit();
    ed.set_selection(out.clone());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::tests::{add_object, flat_map};

    fn roots(tag: &str) -> (std::path::PathBuf, Roots) {
        let dir = std::env::temp_dir().join(format!("om-meshops-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("content")).unwrap();
        std::fs::create_dir_all(dir.join("omsi")).unwrap();
        (dir.clone(), Roots { omsi: dir.join("omsi"), content: dir.join("content") })
    }

    /// An object of the installation: its `.sco` and its meshes, and no openMapper source
    /// beside it.
    fn an_installed_object(roots: &Roots) -> String {
        let rel = "Sceneryobjects\\Test\\Lamp\\Lamp.sco".to_string();
        let src = crate::editable::primitive_source(openmapper_mesh::primitives::Primitive::Cube { size: Vec3::splat(1.0) });
        crate::editable::save(roots, &rel, &src).unwrap();
        let (dir, _) = crate::editable::folder_of(roots, &rel);
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            if e.path().extension().map(|x| x == "json").unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        assert!(!crate::editable::is_editable(roots, &rel));
        rel
    }

    /// A Tab on an object that is not openMapper's reads a copy of its own, but the map object
    /// keeps the file it had: taken back, nothing is left and nothing has to be undone.
    #[test]
    fn a_copy_for_reading_a_mesh_leaves_the_map_alone_until_it_is_wanted() {
        let (dir, roots) = roots("stray");
        let rel = an_installed_object(&roots);
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects[0].file = rel.clone();
        let mut ed = Editor::default();
        ed.open(doc);
        let lib = Library::default();
        let sel = Sel::Object((0, 0), 1);
        let copy = copy_for_edit(&mut ed, &roots, &lib, sel).unwrap();
        assert!(!copy.eq_ignore_ascii_case(&rel), "a copy of its own: {copy}");
        assert!(crate::editable::is_editable(&roots, &copy));
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().file, rel, "the map still names the object it had");
        assert!(!ed.history.can_undo(), "reading the mesh is not a change of the map");
        // (a Tab and a Tab back: the copy goes, and nothing else does)
        discard_copy(&roots, &copy);
        assert!(!crate::editable::folder_of(&roots, &copy).0.exists(), "no folder left behind");
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().file, rel);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The first change hands the map object the copy, in one step that can be undone.
    #[test]
    fn pointing_at_the_copy_is_one_step_of_the_map() {
        let (dir, roots) = roots("point");
        let rel = an_installed_object(&roots);
        let mut doc = flat_map(1);
        add_object(&mut doc, (0, 0), 1, 100.0, 100.0);
        doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects[0].file = rel.clone();
        let mut ed = Editor::default();
        ed.open(doc);
        let lib = Library::default();
        let sel = Sel::Object((0, 0), 1);
        let copy = copy_for_edit(&mut ed, &roots, &lib, sel).unwrap();
        point_at(&mut ed, sel, &copy);
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().file, copy);
        ed.undo();
        assert_eq!(ed.doc.as_ref().unwrap().object((0, 0), 1).unwrap().file, rel, "one step back");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
