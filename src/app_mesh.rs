//! The app's side of openMapper's own objects: the selected one's source kept at hand, the
//! edit mode's keys, its files written and drawn again after a change, and what needs the
//! content folder or a file dialog (making objects editable, joining, booleans, export,
//! textures and materials, baking, signal heads).

use std::path::{Path, PathBuf};

use glam::{DVec3, Vec2, Vec3};
use openmapper_mesh::edit::{self, Delete, MergeAt, SelectMode, Selection};
use openmapper_mesh::{Material, Mesh};
use winit::keyboard::KeyCode;

use super::App;
use crate::doc::{tile_origin, Sel};
use crate::editable;
use crate::i18n::tr;
use crate::meshedit::{Session, Sizing};
use crate::ui_mesh::{BakeChoice, MeshAction, TexSlot};

impl App {
    /// Each frame: the selected openMapper object's source opened (or let go), where its map
    /// object stands, what the cursor is on in the edit mode, and its files written once a
    /// change has rested.
    pub(super) fn mesh_frame(&mut self) {
        let Some(roots) = self.roots.clone() else { return };
        if let Some(pts) = self.ed.poly_request.take() {
            if let Err(e) = self.make_polygon(&pts) {
                self.ed.toast(e.to_string());
            }
        }
        // the object selected alone, if it is openMapper's
        let want: Option<(String, Sel)> = match (self.ed.selected, self.ed.selection().len()) {
            (Some(s @ Sel::Object(k, id)), 1) => self.ed.doc.as_ref().and_then(|d| d.object(k, id)).map(|o| (o.file.clone(), s)),
            _ => None,
        };
        let want = want.filter(|(rel, _)| {
            if self.mesh_known.as_ref().map(|(r, _)| r.eq_ignore_ascii_case(rel)).unwrap_or(false) {
                self.mesh_known.as_ref().map(|k| k.1).unwrap_or(false)
            } else {
                let e = editable::is_editable(&roots, rel);
                self.mesh_known = Some((rel.clone(), e));
                e
            }
        });
        let keep = match (&self.ed.mesh, &want) {
            (Some(m), Some((rel, s))) => m.rel.eq_ignore_ascii_case(rel) && m.inst == *s,
            (Some(m), None) => m.editing && self.ed.doc.as_ref().map(|d| selected_still(d, m.inst)).unwrap_or(false),
            _ => false,
        };
        if !keep {
            if let Some(mut m) = self.ed.mesh.take() {
                self.write_mesh(&mut m, true);
            }
            if let Some((rel, s)) = want {
                self.ed.mesh = Session::open(&roots, &rel, s);
            }
        }
        let view = self.ed.gizmo_view();
        let Some(m) = self.ed.mesh.as_mut() else { return };
        // where its map object stands (it may have moved, or have gone by an undo)
        let place = self.ed.doc.as_ref().and_then(|d| {
            let Sel::Object(k, id) = m.inst else { return None };
            let (k, o) = crate::selection::object_by_id(d, id).or_else(|| d.object(k, id).map(|o| (k, o)))?;
            Some(editable::placement(d.object_world(k, o, m.src.abs_height), o.rot))
        });
        match place {
            Some(p) => m.place = p,
            None => {
                m.editing = false;
                return;
            }
        }
        // (another tool chosen: the edit mode is left, its changes written)
        if m.editing && self.ed.tool != crate::editor::Tool::Select {
            self.toggle_edit_mode();
            return;
        }
        if m.editing && !self.mouse.over_ui {
            if let Some(view) = view {
                if m.modal_active() {
                    m.update_modal(&view);
                } else if !self.mouse.left {
                    m.hover = m.pick(&view);
                }
            }
        }
        let mut m = self.ed.mesh.take().unwrap();
        self.write_mesh(&mut m, false);
        self.ed.mesh = Some(m);
        if let Some(h) = self.hook_edit.take() {
            self.edit_hook(&h);
        }
    }

    /// OPENMAPPER_EDIT=vertex|edge|face[,all][,extrude|inset|bevel|subdivide|bool]: the edit
    /// mode of the selected object, for checks without a person at the screen.
    fn edit_hook(&mut self, h: &str) {
        self.toggle_edit_mode();
        let Some(m) = self.ed.mesh.as_mut() else { return };
        for w in h.split(',') {
            match w.trim() {
                "vertex" => m.set_mode(SelectMode::Vertex),
                "edge" => m.set_mode(SelectMode::Edge),
                "face" => m.set_mode(SelectMode::Face),
                "all" => m.sel = Selection::all(m.mesh()),
                "top" => {
                    // (the faces looking up)
                    let mesh = m.mesh().clone();
                    let f: Vec<usize> = (0..mesh.faces.len()).filter(|&i| mesh.face_normal(i).z > 0.9).collect();
                    m.mode = SelectMode::Face;
                    m.sel = Selection::from_faces(&mesh, f.iter().copied());
                }
                "extrude" => m.apply(|mesh, s, _| Some(edit::extrude_along_normals(mesh, s, 0.6))),
                "inset" => m.apply(|mesh, s, _| Some(edit::inset(mesh, s, 0.15, 0.0, false))),
                "bevel" => m.apply(|mesh, s, mode| Some(edit::bevel(mesh, s, mode, 0.1, 3))),
                "subdivide" => m.apply(|mesh, s, _| Some(edit::subdivide(mesh, s, 2))),
                "modifier" => {
                    m.checkpoint();
                    let pi = m.part;
                    m.src.model.parts[pi].modifiers.push(openmapper_mesh::Stacked { on: true, modifier: openmapper_mesh::modifiers::Modifier::Subdivision { levels: 2, smooth: true } });
                    m.changed();
                }
                w if w.starts_with("mod:") => {
                    // (any modifier by its name's start, with its usual settings)
                    let want = w[4..].to_lowercase();
                    if let Some(modifier) = openmapper_mesh::modifiers::Modifier::all().into_iter().find(|x| x.name().to_lowercase().starts_with(&want)) {
                        m.checkpoint();
                        let pi = m.part;
                        m.src.model.parts[pi].modifiers.push(openmapper_mesh::Stacked { on: true, modifier });
                        m.changed();
                    }
                }
                "leave" => {
                    self.toggle_edit_mode();
                    return;
                }
                _ => {}
            }
        }
    }

    /// Write the session's object if it changed (at once with `now`), and have its model read
    /// and drawn again.
    pub(super) fn write_mesh(&mut self, m: &mut Session, now: bool) {
        let Some(roots) = self.roots.clone() else { return };
        // (what follows the ground follows it where this object stands)
        let place = m.place;
        let doc = self.ed.doc.as_ref();
        let base = doc.map(|d| d.ground(place.w_axis.truncate().truncate())).unwrap_or(0.0);
        let ground = |p: Vec2| -> f32 {
            let w = place.transform_point3(glam::DVec3::new(p.x as f64, p.y as f64, 0.0));
            doc.map(|d| (d.ground(w.truncate()) - base) as f32).unwrap_or(0.0)
        };
        let follows = m.src.model.parts.iter().any(|p| p.modifiers.iter().any(|s| s.on && matches!(s.modifier, openmapper_mesh::modifiers::Modifier::Ground { .. })));
        let written = m.write_if_due(&roots, now, if follows { Some(&ground) } else { None });
        match written {
            Ok(true) => self.forget_object(&m.rel),
            Ok(false) => {}
            Err(e) => {
                m.dirty = false;
                self.ed.error(format!("{}: {e}", tr("Could not write the object", "Не удалось записать объект")));
            }
        }
    }

    /// An object's files were written anew: its model and pictures are read again.
    pub(super) fn forget_object(&mut self, rel: &str) {
        let Some(roots) = &self.roots else { return };
        let (dir, _) = editable::folder_of(roots, rel);
        crate::hot::own_change();
        if let Some(a) = &mut self.assets {
            a.forget_object(rel, &dir);
        }
        if let Some(g) = &mut self.gfx {
            g.renderer.forget_model(rel, &dir);
        }
    }

    /// Into the edit mode of the selected object (made an openMapper object of its own first
    /// if it is not one), or out of it.
    pub(super) fn toggle_edit_mode(&mut self) {
        if let Some(m) = self.ed.mesh.as_mut().filter(|m| m.editing) {
            m.cancel();
            m.editing = false;
            m.changed();
            let mut m = self.ed.mesh.take().unwrap();
            self.write_mesh(&mut m, true);
            self.ed.mesh = Some(m);
            return;
        }
        let Some(sel @ Sel::Object(..)) = self.ed.selected else {
            self.ed.toast(tr("Select an object to edit its mesh", "Выделите объект, чтобы редактировать его меш"));
            return;
        };
        if self.ed.mesh.is_none() {
            let Some(roots) = self.roots.clone() else { return };
            match crate::meshops::make_editable(&mut self.ed, &roots, &self.lib, sel) {
                Ok(rel) => {
                    self.mesh_known = Some((rel.clone(), true));
                    self.ed.mesh = Session::open(&roots, &rel, sel);
                    self.ed.toast(tr("A copy of its own was made: the original type is unchanged", "Создана собственная копия: исходный тип не изменён"));
                }
                Err(e) => {
                    self.ed.error(format!("{}: {e}", tr("Cannot edit this object", "Этот объект нельзя редактировать")));
                    return;
                }
            }
        }
        if let Some(m) = self.ed.mesh.as_mut() {
            m.editing = true;
            m.part = m.part.min(m.src.model.parts.len().saturating_sub(1));
        }
        self.ed.set_tool(crate::editor::Tool::Select);
        self.ed.xform = crate::editor::Xform::Select;
    }

    /// A key in the edit mode, Blender's where the view's own keys allow: whether it was
    /// taken.
    pub(super) fn mesh_key(&mut self, code: KeyCode) -> bool {
        let (ctrl, shift, alt) = (self.keys.ctrl, self.keys.shift, self.keys.alt);
        let view = self.ed.gizmo_view();
        let Some(m) = self.ed.mesh.as_mut().filter(|m| m.editing) else { return false };
        match code {
            KeyCode::Tab => {
                self.toggle_edit_mode();
                return true;
            }
            KeyCode::Escape => {
                if m.modal_active() {
                    m.cancel();
                } else if m.dragging() {
                    m.cancel_drag();
                    self.ed.stop_drag();
                } else {
                    self.toggle_edit_mode();
                }
                return true;
            }
            KeyCode::Enter | KeyCode::NumpadEnter if m.modal_active() => m.confirm(),
            KeyCode::KeyZ if alt && !ctrl => m.xray = !m.xray,
            KeyCode::KeyZ if ctrl && shift => m.redo(),
            KeyCode::KeyZ if ctrl => m.undo(),
            KeyCode::KeyY if ctrl => m.redo(),
            KeyCode::Digit1 if !ctrl => m.set_mode(SelectMode::Vertex),
            KeyCode::Digit2 if !ctrl => m.set_mode(SelectMode::Edge),
            KeyCode::Digit3 if !ctrl => m.set_mode(SelectMode::Face),
            KeyCode::KeyA if alt => m.sel = Selection::default(),
            KeyCode::KeyA if !ctrl => {
                let all = Selection::all(m.mesh());
                m.sel = if m.sel.verts.len() == all.verts.len() { Selection::default() } else { all };
            }
            KeyCode::KeyI if ctrl => {
                let s = edit::invert(m.mesh(), &m.sel, m.mode);
                m.sel = s;
            }
            KeyCode::KeyL if ctrl => {
                let s = edit::select_linked(m.mesh(), &m.sel);
                m.sel = s;
            }
            KeyCode::KeyL => {
                // (the piece under the cursor)
                if let (Some(h), Some(_)) = (m.hover, &view) {
                    let mut one = Selection::default();
                    let mesh = m.mesh().clone();
                    match h {
                        crate::meshedit::Hit::Vert(v) => one.toggle_vert(&mesh, v),
                        crate::meshedit::Hit::Edge(e) => one.toggle_edge(&mesh, e),
                        crate::meshedit::Hit::Face(f) => one.toggle_face(&mesh, f),
                    }
                    let linked = edit::select_linked(&mesh, &one);
                    let mut v = m.sel.vertices();
                    v.extend(linked.vertices());
                    m.sel = if m.mode == SelectMode::Face {
                        let mut f: Vec<usize> = m.sel.faces.iter().copied().collect();
                        f.extend(linked.faces.iter().copied());
                        Selection::from_faces(&mesh, f.iter().copied())
                    } else {
                        Selection::from_verts(&mesh, v.iter().copied())
                    };
                }
            }
            KeyCode::KeyE if !ctrl => {
                if let Some(v) = &view {
                    m.start(Sizing::Extrude, v);
                }
            }
            KeyCode::KeyI => {
                if let Some(v) = &view {
                    m.start(Sizing::Inset, v);
                }
            }
            KeyCode::KeyB if ctrl => {
                if let Some(v) = &view {
                    m.start(Sizing::Bevel, v);
                }
            }
            KeyCode::KeyR if ctrl => {
                // (across the ring through the edge under the cursor)
                let e = match m.hover {
                    Some(crate::meshedit::Hit::Edge(e)) => Some(e),
                    _ => {
                        let mode = m.mode;
                        m.mode = SelectMode::Edge;
                        let h = view.as_ref().and_then(|v| m.pick(v));
                        m.mode = mode;
                        match h {
                            Some(crate::meshedit::Hit::Edge(e)) => Some(e),
                            _ => None,
                        }
                    }
                };
                match (e, &view) {
                    (Some(e), Some(v)) => m.start(Sizing::LoopCut(e), v),
                    _ => self.ed.toast(tr("Point at an edge to cut across its ring of faces", "Наведите на ребро, чтобы разрезать кольцо граней поперёк")),
                }
            }
            KeyCode::KeyF if !ctrl => m.apply(|mesh, s, _| Some(edit::fill(mesh, s))),
            KeyCode::KeyM if !ctrl => m.apply(|mesh, s, _| Some(edit::merge(mesh, s, MergeAt::Center))),
            KeyCode::KeyJ if alt => m.apply(|mesh, s, _| Some(edit::tris_to_quads(mesh, s, 40.0))),
            KeyCode::KeyJ if !ctrl => m.apply(|mesh, s, _| Some(edit::connect(mesh, s))),
            KeyCode::KeyT if ctrl => m.apply(|mesh, s, _| Some(edit::triangulate(mesh, s))),
            KeyCode::KeyN if shift => m.apply(|mesh, s, _| {
                edit::recalculate_outside(mesh, s);
                None
            }),
            KeyCode::KeyN if alt => m.apply(|mesh, s, _| {
                edit::flip_normals(mesh, s);
                None
            }),
            KeyCode::KeyD if shift => m.apply(|mesh, s, _| Some(edit::duplicate(mesh, s))),
            KeyCode::KeyY if !ctrl => m.apply(|mesh, s, _| Some(edit::split(mesh, s))),
            KeyCode::KeyP if !ctrl => {
                self.mesh_action(MeshAction::SeparateSelected);
            }
            KeyCode::Delete | KeyCode::Backspace | KeyCode::KeyX if !ctrl => {
                let what = match m.mode {
                    SelectMode::Vertex => Delete::Vertices,
                    SelectMode::Edge => Delete::Edges,
                    SelectMode::Face => Delete::Faces,
                };
                m.apply(|mesh, s, _| Some(edit::delete(mesh, s, what)));
            }
            KeyCode::KeyV if !ctrl => self.ed.xform = crate::editor::Xform::Move,
            KeyCode::KeyC if !ctrl => self.ed.xform = crate::editor::Xform::Rotate,
            KeyCode::KeyS if alt => self.ed.xform = crate::editor::Xform::Scale,
            KeyCode::KeyX if ctrl => return false,
            _ => return false,
        }
        true
    }

    pub(super) fn mesh_action(&mut self, a: MeshAction) {
        let Some(roots) = self.roots.clone() else { return };
        let result: anyhow::Result<()> = (|| {
            match a {
                MeshAction::ToggleEdit => self.toggle_edit_mode(),
                MeshAction::MakeSingle => {
                    if let Some(s) = self.ed.selected {
                        if let Some(mut m) = self.ed.mesh.take() {
                            self.write_mesh(&mut m, true);
                        }
                        crate::meshops::make_single(&mut self.ed, &roots, &self.lib, s)?;
                        self.mesh_known = None;
                    }
                }
                MeshAction::SeparateParts => {
                    if let Some(s) = self.ed.selected {
                        if let Some(mut m) = self.ed.mesh.take() {
                            self.write_mesh(&mut m, true);
                        }
                        let made = crate::meshops::separate_parts(&mut self.ed, &roots, &self.lib, s)?;
                        self.ed.toast(format!("{} {}", tr("Objects made:", "Создано объектов:"), made.len()));
                    }
                }
                MeshAction::Join => {
                    if let Some(mut m) = self.ed.mesh.take() {
                        self.write_mesh(&mut m, true);
                    }
                    crate::meshops::join(&mut self.ed, &roots, &self.lib)?;
                    self.mesh_known = None;
                }
                MeshAction::Boolean(op, keep) => {
                    if let Some(mut m) = self.ed.mesh.take() {
                        self.write_mesh(&mut m, true);
                    }
                    crate::meshops::boolean(&mut self.ed, &roots, &self.lib, op, keep)?;
                    self.mesh_known = None;
                }
                MeshAction::SeparateSelected => self.separate_selected()?,
                MeshAction::Export => self.export_selection(false)?,
                MeshAction::ExportModel => self.export_selection(true)?,
                MeshAction::ExportTexture(i) => self.export_texture(i)?,
                MeshAction::PickTexture(i, slot) => self.pick_texture(i, slot)?,
                MeshAction::ImportMaterial => self.import_material()?,
                MeshAction::Bake(choice) => self.bake(choice)?,
                MeshAction::NewSignal(lamps) => self.new_signal(&lamps)?,
                MeshAction::NewLamp => self.new_lamp()?,
                MeshAction::NewPicture(s) => self.new_picture(&s)?,
                MeshAction::NewLight(s) => self.new_light(&s)?,
                MeshAction::NewSound(s) => self.new_sound(&s)?,
                MeshAction::SplinesToObject(replace) => self.splines_to_object(replace)?,
                MeshAction::PolygonDraw(s) => {
                    self.ed.set_tool(crate::editor::Tool::Objects);
                    self.ed.poly_settings = s;
                    self.ed.poly_pts.clear();
                    self.ed.poly_draw = true;
                    self.ed.toast(tr("Click the corners of the polygon; the first corner or Enter closes it", "Кликайте по углам многоугольника; первый угол или Enter замыкает его"));
                }
                MeshAction::PolygonFromSplines(s) => {
                    self.ed.poly_settings = s;
                    let pts = self.spline_outline()?;
                    self.make_polygon(&pts)?;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.ed.error(e.to_string());
        }
    }

    /// The selected faces of the mesh being edited made an object of their own beside it.
    fn separate_selected(&mut self) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let map = self.map_folder();
        let Some(m) = self.ed.mesh.as_mut().filter(|m| m.editing) else { return Ok(()) };
        if m.sel.faces.is_empty() {
            anyhow::bail!("{}", tr("select the faces to separate", "выделите грани, которые нужно отделить"));
        }
        m.checkpoint();
        let sel = m.sel.clone();
        let piece = edit::separate(m.mesh_mut(), &sel);
        m.sel = Selection::default();
        m.changed();
        let mut one = editable::Source { name: format!("{} (2)", m.src.name), abs_height: m.src.abs_height, ..Default::default() };
        one.model.materials = m.src.model.materials.clone();
        one.model.parts.push(openmapper_mesh::Part::new(&m.src.model.parts[m.part].name, piece));
        editable::merge_materials(&mut one.model);
        let (from, _) = editable::folder_of(&roots, &m.rel);
        let place = m.place;
        let rel = editable::new_rel(&roots, &map, &format!("{}_part", editable::safe_name(&m.src.name)));
        let (dir, _) = editable::folder_of(&roots, &rel);
        copy_missing(&from.join("texture"), &dir.join("texture"));
        editable::save(&roots, &rel, &one)?;
        let (_, rot, pos) = place.to_scale_rotation_translation();
        let r = crate::gizmo::object_rot(rot);
        self.add_map_object(&rel, pos, r);
        self.ed.toast(tr("Separated into an object of its own", "Отделено в отдельный объект"));
        Ok(())
    }

    pub(super) fn map_folder(&self) -> String {
        self.ed.doc.as_ref().and_then(|d| d.dir.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Shared".into())
    }

    pub(super) fn add_map_object_at(&mut self, rel: &str, world: DVec3) -> Option<Sel> {
        self.add_map_object(rel, world, [0.0; 3])
    }

    /// A new map object of `rel` (one undo step).
    fn add_map_object(&mut self, rel: &str, world: DVec3, rot: [f64; 3]) -> Option<Sel> {
        let doc = self.ed.doc.as_mut()?;
        let k = crate::doc::tile_of(world.truncate());
        if !doc.tiles.contains_key(&k) {
            return None;
        }
        self.ed.history.begin(tr("Add object", "Добавление объекта"));
        self.ed.history.touch_tile(doc, k);
        self.ed.history.touch_global(doc);
        let id = doc.new_id();
        let org = tile_origin(k);
        let z = world.z - doc.ground(world.truncate());
        let t = doc.tiles.get_mut(&k)?;
        t.tile.objects.push(omsi_map::MapObject { file: rel.to_string(), id, pos: [world.x - org.x, world.y - org.y, z], rot, ..Default::default() });
        t.touch_content();
        self.ed.history.commit();
        Some(Sel::Object(k, id))
    }

    // ---------------------------------------------------------------------------- export

    /// The selected objects and roads written to a 3D file of the format the name ends in.
    /// `simple`: the formats of the right-click menu (O3D, OBJ, FBX, GLB, X as text or binary)
    /// and a single object written as the model is made, not turned as it stands on the map.
    fn export_selection(&mut self, simple: bool) -> anyhow::Result<()> {
        use openmapper_mesh::export::{ExportObject, Format, Scene};
        let sels = self.ed.selection();
        if sels.is_empty() {
            anyhow::bail!("{}", tr("select what to export (objects, roads)", "выделите, что экспортировать (объекты, дороги)"));
        }
        let formats: Vec<Format> = if simple { vec![Format::O3d, Format::Obj, Format::Fbx, Format::Glb, Format::X] } else { Format::ALL.into_iter().filter(|f| *f != Format::XBinary).collect() };
        // (named after the model when there is one)
        let first_name = self.ed.doc.as_ref().and_then(|d| match sels[0] {
            Sel::Object(k, id) => d.object(k, id).map(|o| o.file.clone()),
            Sel::Spline(k, id) => d.spline(k, id).map(|s| s.file.clone()),
            Sel::Attached(k, id) => d.attachment(k, id).map(|a| a.file.clone()),
        });
        let stem = first_name.and_then(|f| f.rsplit('\\').next().map(|n| n.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| n.to_string()))).filter(|n| !n.is_empty() && sels.len() == 1).unwrap_or_else(|| "export".into());
        let mut dialog = rfd::FileDialog::new().set_title(tr("Export the selection", "Экспорт выделенного")).set_file_name(format!("{stem}.glb"));
        for f in &formats {
            dialog = dialog.add_filter(f.label(), &[f.extension()]);
        }
        let Some(mut path) = dialog.save_file() else { return Ok(()) };
        // (a file name with no extension, as some systems' dialogs give)
        if path.extension().is_none() {
            path.set_extension("glb");
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut format = formats.iter().copied().find(|f| f.extension() == ext).ok_or_else(|| anyhow::anyhow!("{}: .{ext}", tr("unknown format", "неизвестный формат")))?;
        if simple && format == Format::X {
            let binary = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title("openMapper")
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_description(tr("Write the .x file in its binary form? No: as text, which OMSI 2 and openOMSI read.", "Записать файл .x в двоичном виде? Нет: текстом, как читают OMSI 2 и openOMSI."))
                .show();
            if binary == rfd::MessageDialogResult::Yes {
                format = Format::XBinary;
            }
        }
        let single_object = simple && sels.len() == 1 && matches!(sels[0], Sel::Object(..) | Sel::Attached(..));
        let (Some(doc), Some(assets)) = (self.ed.doc.as_ref(), self.assets.as_mut()) else { return Ok(()) };
        // (the selection's middle is the file's origin)
        let centre = crate::selection::centre(doc, &sels).unwrap_or_default();
        let origin = DVec3::new(centre.x, centre.y, doc.ground(centre));
        let mut scene = Scene::default();
        let mut textures: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
        let mut waiting = 0;
        for s in &sels {
            match *s {
                Sel::Object(k, id) => {
                    let Some(o) = doc.object(k, id) else { continue };
                    let Some(model) = assets.object(&o.file) else {
                        waiting += 1;
                        continue;
                    };
                    let world = doc.object_world(k, o, model.abs_height);
                    let place = if single_object { glam::Mat4::IDENTITY } else { editable::placement(world - origin, o.rot).as_mat4() };
                    for part in &model.parts {
                        let mesh = mesh_of(&part.mesh, &part.materials, &mut scene.materials, &mut textures);
                        scene.objects.push(ExportObject { name: o.file.rsplit('\\').next().unwrap_or("object").trim_end_matches(".sco").to_string(), mesh, transform: place });
                    }
                }
                Sel::Attached(k, id) => {
                    let Some(a) = doc.attachment(k, id) else { continue };
                    let Some(model) = assets.object(&a.file) else {
                        waiting += 1;
                        continue;
                    };
                    let Some(first) = crate::selection::attachment_first(doc, k, id) else { continue };
                    let rot = [crate::selection::attachment_world_rot(doc, k, a), a.rot[1], a.rot[2]];
                    let place = if single_object { glam::Mat4::IDENTITY } else { editable::placement(first.pose.pos - origin, rot).as_mat4() };
                    for part in &model.parts {
                        let mesh = mesh_of(&part.mesh, &part.materials, &mut scene.materials, &mut textures);
                        scene.objects.push(ExportObject { name: a.file.rsplit('\\').next().unwrap_or("object").trim_end_matches(".sco").to_string(), mesh, transform: place });
                    }
                }
                Sel::Spline(k, id) => {
                    let Some(sp) = doc.spline(k, id) else { continue };
                    let Some(model) = assets.spline(&sp.file) else {
                        waiting += 1;
                        continue;
                    };
                    // (built in the tile's own frame: moved to the file's origin)
                    let org = tile_origin(k);
                    let Some(md) = crate::render::build_spline(&model, sp, org) else { continue };
                    let mut mesh = mesh_of(&md, &model.materials, &mut scene.materials, &mut textures);
                    mesh.transform(glam::Mat4::from_translation(Vec3::new((org.x - origin.x) as f32, (org.y - origin.y) as f32, -origin.z as f32)));
                    scene.objects.push(ExportObject { name: sp.file.rsplit('\\').next().unwrap_or("spline").trim_end_matches(".sli").to_string(), mesh, transform: glam::Mat4::IDENTITY });
                }
            }
        }
        if waiting > 0 && scene.objects.is_empty() {
            anyhow::bail!("{}", tr("the models are still loading: try again in a moment", "модели ещё загружаются: попробуйте через мгновение"));
        }
        // (glTF, USD and 3MF take only PNG and JPEG: the others go in as PNG)
        let needs_png = matches!(format, Format::Gltf | Format::Glb | Format::Usda | Format::ThreeMf);
        let tmp = std::env::temp_dir().join(format!("openmapper-export-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        if needs_png {
            for m in &mut scene.materials {
                if let Some(t) = m.texture.clone() {
                    let e = Path::new(&t).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if !matches!(e.as_str(), "png" | "jpg" | "jpeg") {
                        if let Some(img) = textures.get(&t).and_then(|p| crate::models::picture(None, Some(p))) {
                            let png = format!("{}.png", Path::new(&t).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                            if img.save(tmp.join(&png)).is_ok() {
                                textures.insert(png.clone(), tmp.join(&png));
                                m.texture = Some(png);
                            }
                        }
                    }
                }
            }
        }
        let written = openmapper_mesh::export::export(&scene, format, &path, &|name| textures.get(name).cloned())?;
        let _ = std::fs::remove_dir_all(&tmp);
        self.ed.toast(format!("{} {} ({} {})", tr("Exported:", "Экспортировано:"), path.display(), written.len(), tr("files", "файлов")));
        Ok(())
    }

    fn export_texture(&mut self, material: usize) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let Some(m) = self.ed.mesh.as_ref() else { return Ok(()) };
        let Some(tex) = m.src.model.materials.get(material).and_then(|m| m.texture.clone()) else {
            anyhow::bail!("{}", tr("this material has no texture", "у этого материала нет текстуры"));
        };
        let (dir, _) = editable::folder_of(&roots, &m.rel);
        let src = dir.join("texture").join(&tex);
        let img = crate::models::picture(None, Some(&src)).ok_or_else(|| anyhow::anyhow!("{}: {tex}", tr("cannot read", "не читается")))?;
        let stem = Path::new(&tex).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "texture".into());
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("Export the texture", "Экспорт текстуры"))
            .set_file_name(format!("{stem}.png"))
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("BMP", &["bmp"])
            .add_filter("TGA", &["tga"])
            .add_filter("TIFF", &["tif", "tiff"])
            .add_filter("WebP", &["webp"])
            .add_filter("DDS", &["dds"])
            .save_file()
        else {
            return Ok(());
        };
        save_picture(&img, &path)?;
        self.ed.toast(format!("{} {}", tr("Texture saved:", "Текстура сохранена:"), path.display()));
        Ok(())
    }

    // ------------------------------------------------------------- textures and materials

    /// A picture chosen for a material's texture (or one of its maps): copied into the
    /// object's folder, as PNG if OMSI does not read its format.
    fn pick_texture(&mut self, material: usize, slot: TexSlot) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let Some(m) = self.ed.mesh.as_mut() else { return Ok(()) };
        let Some(file) = rfd::FileDialog::new().set_title(tr("Choose a picture", "Выберите картинку")).add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "jpeg", "bmp", "tga", "dds", "tif", "tiff", "webp"]).pick_file() else { return Ok(()) };
        let (dir, stem) = editable::folder_of(&roots, &m.rel);
        let name = bring_picture(&file, &dir.join("texture"), &format!("{stem}_{material}{}", slot.suffix()))?;
        m.checkpoint();
        if let Some(mat) = m.src.model.materials.get_mut(material) {
            match slot {
                TexSlot::Color => {
                    mat.texture = Some(name.clone());
                    // (a texture shows as it is: white lets it through unchanged)
                    mat.color = [1.0, 1.0, 1.0, mat.color[3]];
                    if mat.alpha == 0 {
                        mat.alpha = crate::models::picture(None, Some(&dir.join("texture").join(&name))).map(|i| crate::models::alpha_of(&i)).unwrap_or(0);
                    }
                }
                TexSlot::Normal => {
                    let base = mat.texture.clone().ok_or_else(|| anyhow::anyhow!("{}", tr("give the material a texture first: its maps go beside it", "сначала задайте материалу текстуру: карты лежат рядом с ней")))?;
                    let target = format!("{}_nn.png", Path::new(&base).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                    std::fs::rename(dir.join("texture").join(&name), dir.join("texture").join(&target))?;
                    mat.normal_map = Some(target);
                }
                TexSlot::Orm => {
                    let base = mat.texture.clone().ok_or_else(|| anyhow::anyhow!("{}", tr("give the material a texture first: its maps go beside it", "сначала задайте материалу текстуру: карты лежат рядом с ней")))?;
                    let target = format!("{}_orm.png", Path::new(&base).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                    std::fs::rename(dir.join("texture").join(&name), dir.join("texture").join(&target))?;
                    mat.orm_map = Some(target);
                }
                TexSlot::Night => mat.night_map = Some(name),
                TexSlot::Light => mat.light_map = Some(name),
            }
        }
        m.changed();
        let rel = m.rel.clone();
        self.forget_object(&rel);
        Ok(())
    }

    /// Materials brought in from another tool: a Wavefront `.mtl`, the materials of a glTF /
    /// GLB / FBX, or a set of PBR pictures (base colour, normal, roughness, metalness,
    /// occlusion, emission) chosen together.
    fn import_material(&mut self) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let Some(m) = self.ed.mesh.as_mut() else { return Ok(()) };
        let Some(files) = rfd::FileDialog::new()
            .set_title(tr("Import materials (an .mtl, a model, or the pictures of one PBR material)", "Импорт материалов (.mtl, модель или картинки одного PBR-материала)"))
            .add_filter(tr("Materials and pictures", "Материалы и картинки"), &["mtl", "gltf", "glb", "fbx", "obj", "png", "jpg", "jpeg", "bmp", "tga", "dds", "tif", "tiff", "webp"])
            .pick_files()
        else {
            return Ok(());
        };
        let (dir, stem) = editable::folder_of(&roots, &m.rel);
        let tex_dir = dir.join("texture");
        std::fs::create_dir_all(&tex_dir)?;
        let new = crate::import::materials_from(&files, &tex_dir, &stem)?;
        if new.is_empty() {
            anyhow::bail!("{}", tr("no materials found there", "материалов не найдено"));
        }
        m.checkpoint();
        let n = new.len();
        m.src.model.materials.extend(new);
        m.changed();
        let rel = m.rel.clone();
        self.forget_object(&rel);
        self.ed.toast(format!("{} {n}", tr("Materials added:", "Добавлено материалов:")));
        Ok(())
    }

    // ---------------------------------------------------------------------------- baking

    /// The object's look baked into one picture on a new UV map: its textures and colours,
    /// with the shadows of its own shape (ambient occlusion) if chosen, and its lamps' light
    /// at night as a light map. Every face then takes that one material: fewer draw calls in
    /// the game, and the shading painted in.
    fn bake(&mut self, choice: BakeChoice) -> anyhow::Result<()> {
        use openmapper_mesh::bake::{self, BakeKind, Image, PointLight};
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let Some(m) = self.ed.mesh.as_mut() else { return Ok(()) };
        let (dir, stem) = editable::folder_of(&roots, &m.rel);
        let tex_dir = dir.join("texture");
        let materials = m.src.model.materials.clone();
        let load = |name: &str| -> Option<Image> {
            let img = crate::models::picture(None, Some(&tex_dir.join(name)))?;
            Some(Image::from_rgba8(img.width(), img.height(), img.as_raw()))
        };
        m.checkpoint();
        let size = choice.size;
        let mut joined = Mesh::default();
        let mut owners: Vec<(usize, usize)> = Vec::new();
        for (pi, part) in m.src.model.parts.iter().enumerate() {
            for fi in 0..part.mesh.faces.len() {
                owners.push((pi, fi));
            }
            joined.append(&part.mesh, 0);
        }
        if joined.is_empty() {
            anyhow::bail!("{}", tr("nothing to bake", "нечего запекать"));
        }
        let target = bake::lightmap_target(&joined, 2.0 / size as f32);
        let mut color = bake::bake(&joined, &target, &materials, &load, &BakeKind::Color, size, 4);
        if choice.ao {
            let ao = bake::bake(&joined, &target, &materials, &load, &BakeKind::AmbientOcclusion { samples: 24, distance: choice.ao_distance }, size, 4);
            for (c, a) in color.pixels.iter_mut().zip(&ao.pixels) {
                for k in 0..3 {
                    c[k] *= 0.25 + 0.75 * a[0];
                }
            }
        }
        let atlas = format!("{stem}_baked.png");
        image::RgbaImage::from_raw(size, size, color.to_rgba8()).ok_or_else(|| anyhow::anyhow!("bake"))?.save(tex_dir.join(&atlas))?;
        let alpha = materials.iter().map(|m| m.alpha).max().unwrap_or(0);
        let mut mat = Material { name: "Baked".into(), color: [1.0, 1.0, 1.0, 1.0], texture: Some(atlas.clone()), alpha, ..Default::default() };
        if choice.light {
            let place = m.place;
            let lights: Vec<PointLight> = m
                .src
                .lights
                .iter()
                .map(|l| PointLight { position: l.pos, color: l.color.map(|c| c as f32 / 255.0), radius: if l.kind == editable::LightKind::Ground { l.range } else { (l.size * 8.0).max(2.0) } })
                .collect();
            let _ = place;
            if lights.is_empty() {
                anyhow::bail!("{}", tr("the object has no lamps to bake the night light of", "у объекта нет ламп, свет которых можно запечь"));
            }
            let light = bake::bake(&joined, &target, &materials, &load, &BakeKind::Light { lights, ambient: 0.0 }, size, 4);
            let lm = format!("{stem}_baked_lm.png");
            image::RgbaImage::from_raw(size, size, light.to_rgba8()).ok_or_else(|| anyhow::anyhow!("bake"))?.save(tex_dir.join(&lm))?;
            mat.light_map = Some(lm);
        }
        // the new UVs and the one material on every face
        let mut k = 0;
        for (pi, part) in m.src.model.parts.iter_mut().enumerate() {
            for f in part.mesh.faces.iter_mut() {
                if let Some(t) = target.get(k) {
                    f.uvs = t.clone();
                }
                f.material = 0;
                debug_assert_eq!(owners[k].0, pi);
                k += 1;
            }
            part.primitive = None;
        }
        m.src.model.materials = vec![mat];
        m.changed();
        let rel = m.rel.clone();
        self.forget_object(&rel);
        self.ed.toast(format!("{} {atlas}", tr("Baked into", "Запечено в")));
        Ok(())
    }

    /// The selected splines' shape as one openMapper object standing at the first one's
    /// start, its textures copied beside it; the splines taken away with `replace`.
    fn splines_to_object(&mut self, replace: bool) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let sels: Vec<(crate::doc::TileKey, i64)> = self.ed.selection().iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((*k, *id)) } else { None }).collect();
        if sels.is_empty() {
            anyhow::bail!("{}", tr("select a road or spline first", "сначала выделите дорогу или сплайн"));
        }
        let (Some(doc), Some(assets)) = (self.ed.doc.as_ref(), self.assets.as_mut()) else { return Ok(()) };
        let (k0, id0) = sels[0];
        let Some(first) = doc.spline(k0, id0) else { return Ok(()) };
        let o0 = tile_origin(k0);
        let origin = DVec3::new(o0.x + first.pos[0], o0.y + first.pos[1], first.pos[2]);
        let mut src = editable::Source { name: first.file.rsplit('\\').next().unwrap_or("Spline").trim_end_matches(".sli").to_string(), abs_height: true, ..Default::default() };
        let mut textures: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
        for (k, id) in &sels {
            let Some(sp) = doc.spline(*k, *id) else { continue };
            let Some(model) = assets.spline(&sp.file) else {
                anyhow::bail!("{}", tr("the road's type is still loading: try again in a moment", "тип дороги ещё загружается: попробуйте через мгновение"));
            };
            let org = tile_origin(*k);
            let Some(md) = crate::render::build_spline(&model, sp, org) else { continue };
            let mut mesh = mesh_of(&md, &model.materials, &mut src.model.materials, &mut textures);
            mesh.transform(glam::Mat4::from_translation(Vec3::new((org.x - origin.x) as f32, (org.y - origin.y) as f32, -origin.z as f32)));
            let name = src.model.free_name(&src.name.clone());
            src.model.parts.push(openmapper_mesh::Part::new(&name, mesh));
        }
        if src.model.parts.is_empty() {
            anyhow::bail!("{}", tr("these splines have no shape to see", "у этих сплайнов нет видимой формы"));
        }
        let rel = editable::new_rel(&roots, &self.map_folder(), &editable::safe_name(&src.name));
        let (dir, _) = editable::folder_of(&roots, &rel);
        std::fs::create_dir_all(dir.join("texture"))?;
        for (name, path) in &textures {
            let _ = std::fs::copy(path, dir.join("texture").join(name));
        }
        editable::save(&roots, &rel, &src)?;
        // (its height is the splines', not the ground's: it is made with an absolute one)
        let added = self.add_map_object_abs(&rel, origin);
        if replace {
            if let Some(doc) = self.ed.doc.as_mut() {
                self.ed.history.begin(tr("Replace by an object", "Замена объектом"));
                for (k, id) in &sels {
                    crate::selection::touch_spline(&mut self.ed.history, doc, *k, *id);
                    doc.delete_spline(*k, *id);
                }
                self.ed.history.commit();
            }
        }
        if let Some(s) = added {
            self.ed.select(Some(s));
        }
        self.lib.start_scan(&roots);
        self.ed.toast(tr("Made into an object: Tab edits its mesh", "Превращено в объект: Tab — редактировать меш"));
        Ok(())
    }

    /// The outline the selected splines make end to end, along their centre lines.
    fn spline_outline(&self) -> anyhow::Result<Vec<DVec3>> {
        let Some(doc) = self.ed.doc.as_ref() else { return Ok(Vec::new()) };
        let mut lines = Vec::new();
        for sel in self.ed.selection() {
            let Sel::Spline(k, id) = sel else { continue };
            let Some(sp) = doc.spline(k, id) else { continue };
            let c = omsi_geometry::SplineCurve::from_map(sp, tile_origin(k));
            let n = if c.radius == 0.0 { 1 } else { (c.length / 2.0).ceil().clamp(1.0, 500.0) as usize };
            lines.push((0..=n).map(|i| c.point_at(c.length * i as f64 / n as f64)).collect());
        }
        if lines.is_empty() {
            anyhow::bail!("{}", tr("select the roads or splines that make the outline first", "сначала выделите дороги или сплайны, образующие контур"));
        }
        crate::makers::outline(lines, 1.5).ok_or_else(|| anyhow::anyhow!("{}", tr("the selected splines do not make one outline: they have to join end to end", "выделенные сплайны не образуют один контур: они должны смыкаться концами")))
    }

    /// A polygon object over `pts` (world corners, with their heights), by the maker's
    /// settings, its texture copied beside it; selected when made.
    fn make_polygon(&mut self, pts: &[DVec3]) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let s = self.ed.poly_settings.clone();
        if pts.len() < 3 {
            anyhow::bail!("{}", tr("A polygon needs three corners", "Для многоугольника нужны три угла"));
        }
        let (mut lo, mut hi) = (pts[0], pts[0]);
        for p in pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let origin = DVec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, (lo.z + hi.z) / 2.0);
        let rel_pts: Vec<Vec3> = pts.iter().map(|p| (*p - origin).as_vec3()).collect();
        let rel = editable::new_rel(&roots, &self.map_folder(), "Polygon");
        let (dir, stem) = editable::folder_of(&roots, &rel);
        let tex = match &s.poly_texture {
            Some(file) => {
                let img = crate::models::picture(None, Some(file)).ok_or_else(|| anyhow::anyhow!("{}: {}", tr("cannot read the picture", "картинка не читается"), file.display()))?;
                Some((bring_picture(file, &dir.join("texture"), &stem)?, crate::models::alpha_of(&img)))
            }
            None => None,
        };
        let Some(src) = crate::makers::polygon(&rel_pts, tex.as_ref().map(|t| t.0.as_str()), tex.as_ref().map(|t| t.1).unwrap_or(0), &s) else {
            anyhow::bail!("{}", tr("The corners make no area", "Из этих углов не получается площади"));
        };
        let Some(added) = self.add_map_object_abs_pending(&roots, &rel, &src, origin)? else {
            anyhow::bail!("{}", tr("The polygon lies outside the map", "Многоугольник вне карты"));
        };
        self.lib.start_scan(&roots);
        self.ed.select(Some(added));
        self.ed.toast(tr("Polygon made: Tab edits its mesh", "Многоугольник создан: Tab — редактировать меш"));
        Ok(())
    }

    /// The object saved and put on the map at `world`, if the map has a tile there.
    fn add_map_object_abs_pending(&mut self, roots: &crate::install::Roots, rel: &str, src: &editable::Source, world: DVec3) -> anyhow::Result<Option<Sel>> {
        let k = crate::doc::tile_of(world.truncate());
        if !self.ed.doc.as_ref().map_or(false, |d| d.tiles.contains_key(&k)) {
            return Ok(None);
        }
        editable::save(roots, rel, src)?;
        Ok(self.add_map_object_abs(rel, world))
    }

    fn add_map_object_abs(&mut self, rel: &str, world: DVec3) -> Option<Sel> {
        let doc = self.ed.doc.as_mut()?;
        let k = crate::doc::tile_of(world.truncate());
        if !doc.tiles.contains_key(&k) {
            return None;
        }
        self.ed.history.begin(tr("Add object", "Добавление объекта"));
        self.ed.history.touch_tile(doc, k);
        let id = doc.new_id();
        let org = tile_origin(k);
        let t = doc.tiles.get_mut(&k)?;
        t.tile.objects.push(omsi_map::MapObject { file: rel.to_string(), id, pos: [world.x - org.x, world.y - org.y, world.z], rot: [0.0; 3], ..Default::default() });
        t.touch_content();
        self.ed.history.commit();
        Some(Sel::Object(k, id))
    }

    // ------------------------------------------------------------------- lamps and signals

    /// A signal head of these lamps, from top to bottom: a housing with a visor over each
    /// lamp, the lenses lit by the script from the crossing's phase (it links to a crossing
    /// like OMSI 2's own heads). Put on the map with the next click.
    fn new_signal(&mut self, lamps: &[editable::Lamp]) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let src = crate::signal_maker::signal_head(lamps);
        let rel = editable::new_rel(&roots, &self.map_folder(), "SignalHead");
        editable::save(&roots, &rel, &src)?;
        self.lib.start_scan(&roots);
        self.ed.set_tool(crate::editor::Tool::Objects);
        self.ed.object_type = Some(rel);
        self.ed.toast(tr("Signal head made: click where it stands (near a crossing with a light program it links to it)", "Светофор создан: кликните, где он стоит (рядом с перекрёстком со светофорной программой он к ней привяжется)"));
        Ok(())
    }

    /// A light a script of its own switches, put on the map with the next click.
    pub(super) fn new_scripted_light(&mut self, b: crate::scripting::Behaviour, color: [u8; 3], range: f32) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let rel = crate::scripting::make_light(&roots, &self.map_folder(), b, color, range)?;
        self.lib.start_scan(&roots);
        self.ed.set_tool(crate::editor::Tool::Objects);
        self.ed.object_type = Some(rel);
        self.ed.toast(tr("Light made: click where it goes; its script is in Map > Scripts", "Свет создан: кликните, куда его поставить; его скрипт: Карта > Скрипты"));
        Ok(())
    }

    /// A picture or a texture chosen from a file, made a panel object put on the map with the
    /// next click.
    fn new_picture(&mut self, s: &crate::makers::Settings) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let Some(file) = rfd::FileDialog::new().set_title(tr("Choose a picture or a texture", "Выберите картинку или текстуру")).add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "jpeg", "bmp", "tga", "dds", "tif", "tiff", "webp"]).pick_file() else { return Ok(()) };
        let img = crate::models::picture(None, Some(&file)).ok_or_else(|| anyhow::anyhow!("{}: {}", tr("cannot read the picture", "картинка не читается"), file.display()))?;
        let rel = editable::new_rel(&roots, &self.map_folder(), "Picture");
        let (dir, stem) = editable::folder_of(&roots, &rel);
        let name = bring_picture(&file, &dir.join("texture"), &stem)?;
        let src = crate::makers::picture(&name, img.width() as f32 / img.height().max(1) as f32, crate::models::alpha_of(&img), s);
        editable::save(&roots, &rel, &src)?;
        self.lib.start_scan(&roots);
        self.ed.set_tool(crate::editor::Tool::Objects);
        self.ed.object_type = Some(rel);
        self.ed.toast(tr("Picture made: click where it goes (it can be moved, turned and scaled after)", "Картинка создана: кликните, куда её поставить (потом её можно двигать, поворачивать и масштабировать)"));
        Ok(())
    }

    /// A light source: a lamp that glows and lights the ground, put on the map with the next click.
    fn new_light(&mut self, s: &crate::makers::Settings) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let rel = editable::new_rel(&roots, &self.map_folder(), "Light");
        editable::save(&roots, &rel, &crate::makers::light_source(s))?;
        self.lib.start_scan(&roots);
        self.ed.set_tool(crate::editor::Tool::Objects);
        self.ed.object_type = Some(rel);
        self.ed.toast(tr("Light made: click where it shines", "Свет создан: кликните, где он светит"));
        Ok(())
    }

    /// A sound source from a sound file, put on the map with the next click.
    fn new_sound(&mut self, s: &crate::makers::Settings) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let Some(wav) = rfd::FileDialog::new().set_title(tr("Choose a sound", "Выберите звук")).add_filter(tr("Sounds", "Звуки"), &["wav"]).pick_file() else { return Ok(()) };
        let rel = editable::new_rel(&roots, &self.map_folder(), "Sound");
        let (dir, stem) = editable::folder_of(&roots, &rel);
        editable::save(&roots, &rel, &crate::makers::sound_source(&stem))?;
        crate::makers::write_sound(&dir, &stem, &wav, s)?;
        self.lib.start_scan(&roots);
        self.ed.set_tool(crate::editor::Tool::Objects);
        self.ed.object_type = Some(rel);
        self.ed.toast(tr("Sound source made: click where it plays", "Источник звука создан: кликните, где он звучит"));
        Ok(())
    }

    /// A street lamp: a pole, an arm and a lamp head that glows and lights the ground at night.
    fn new_lamp(&mut self) -> anyhow::Result<()> {
        let Some(roots) = self.roots.clone() else { return Ok(()) };
        let src = crate::signal_maker::street_lamp();
        let rel = editable::new_rel(&roots, &self.map_folder(), "StreetLamp");
        editable::save(&roots, &rel, &src)?;
        self.lib.start_scan(&roots);
        self.ed.set_tool(crate::editor::Tool::Objects);
        self.ed.object_type = Some(rel);
        self.ed.toast(tr("Street lamp made: click where it stands", "Фонарь создан: кликните, где он стоит"));
        Ok(())
    }
}

/// Whether the map object is still there (an undo may have taken it away).
fn selected_still(doc: &crate::doc::Document, s: Sel) -> bool {
    let Sel::Object(_, id) = s else { return false };
    crate::selection::object_by_id(doc, id).is_some()
}

fn copy_missing(from: &Path, to: &Path) {
    let Ok(rd) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for e in rd.flatten() {
        let dst = to.join(e.file_name());
        if e.path().is_file() && !dst.exists() {
            let _ = std::fs::copy(e.path(), dst);
        }
    }
}

/// A picture copied into `dir` under `stem` (as PNG when OMSI does not read its format); the
/// file name it has there.
pub fn bring_picture(file: &Path, dir: &Path, stem: &str) -> anyhow::Result<String> {
    std::fs::create_dir_all(dir)?;
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if omsi_texture::EXTENSIONS.contains(&ext.as_str()) {
        let name = format!("{stem}.{ext}");
        std::fs::copy(file, dir.join(&name))?;
        return Ok(name);
    }
    let img = crate::models::picture(None, Some(file)).ok_or_else(|| anyhow::anyhow!("{}: {}", tr("cannot read the picture", "картинка не читается"), file.display()))?;
    let name = format!("{stem}.png");
    img.save(dir.join(&name))?;
    Ok(name)
}

/// A picture saved in the format its name ends in (DDS as uncompressed RGBA).
pub fn save_picture(img: &image::RgbaImage, path: &Path) -> anyhow::Result<()> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "dds" => std::fs::write(path, crate::doc::dds::rgba(img.width(), img.height(), img.as_raw()))?,
        "jpg" | "jpeg" | "bmp" => image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().save(path)?,
        _ => img.save(path)?,
    }
    Ok(())
}

/// A drawn mesh (of an object's part or a road) as a mesh to write, its materials added to
/// `materials` (textures by file name, their files kept in `textures`).
pub(crate) fn mesh_of(md: &omsi_geometry::MeshData, mats: &[crate::assets::Material], materials: &mut Vec<Material>, textures: &mut std::collections::HashMap<String, PathBuf>) -> Mesh {
    let mut slot_mat = Vec::with_capacity(mats.len());
    for m in mats {
        let tex = m.texture.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string());
        if let (Some(t), Some(p)) = (&tex, &m.texture) {
            textures.entry(t.clone()).or_insert_with(|| p.clone());
        }
        let alpha = match m.alpha {
            crate::assets::Alpha::Opaque => 0,
            crate::assets::Alpha::Test => 1,
            crate::assets::Alpha::Blend => 2,
        };
        let mat = Material { name: tex.clone().unwrap_or_else(|| format!("Material{}", materials.len())), color: if tex.is_some() { [1.0, 1.0, 1.0, m.color[3]] } else { m.color }, texture: tex, alpha, ..Default::default() };
        let i = materials.iter().position(|x| x.texture == mat.texture && x.color == mat.color && x.alpha == mat.alpha).unwrap_or_else(|| {
            materials.push(mat);
            materials.len() - 1
        });
        slot_mat.push(i as u32);
    }
    let mut mesh = Mesh { positions: md.positions.clone(), ..Default::default() };
    for &(first, count, slot) in &md.ranges {
        let material = slot_mat.get(slot as usize).copied().unwrap_or(0);
        for t in md.indices[first as usize..(first + count) as usize].chunks(3) {
            if t.len() < 3 {
                continue;
            }
            let idx = [t[0] as usize, t[1] as usize, t[2] as usize];
            if idx.iter().any(|&i| i >= md.positions.len()) {
                continue;
            }
            let uv = |i: usize| md.uvs.get(i).copied().unwrap_or(Vec2::ZERO);
            let mut f = openmapper_mesh::Face::new(t.to_vec(), idx.iter().map(|&i| uv(i)).collect(), material);
            // (the drawn triangles wind as their file has them: their normals say which side is out)
            let p: Vec<Vec3> = idx.iter().map(|&i| md.positions[i]).collect();
            let n: Vec3 = idx.iter().filter_map(|&i| md.normals.get(i)).sum();
            if (p[1] - p[0]).cross(p[2] - p[0]).dot(n) < 0.0 {
                f.reverse();
            }
            f.smooth = true;
            mesh.faces.push(f);
        }
    }
    mesh.weld(1e-5);
    mesh
}
