//! Editing an openMapper object's mesh in the view, as Blender's Edit Mode: vertices, edges
//! or faces picked with a click (Shift adds, Alt picks a loop) or a box, moved, turned and
//! scaled with the handles, and changed by the operations of `openmapper_mesh::edit`
//! (extrude, inset, bevel, loop cut and the rest), the ones with a size following the mouse
//! until a click. The object on the map is hidden meanwhile and drawn here as it is edited;
//! its files are written again shortly after each change.

use std::time::Instant;

use glam::{DMat4, DQuat, DVec2, DVec3, Mat4, Vec3};
use openmapper_mesh::edit::{self, SelectMode, Selection};
use openmapper_mesh::{edge_key, Mesh};

use crate::doc::Sel;
use crate::editable::{self, Source};
use crate::gizmo::{self, View};
use crate::install::Roots;
use crate::render::{FrameExtras, OverlayPoint};

/// What is under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Vert(u32),
    Edge((u32, u32)),
    Face(usize),
}

/// An operation whose size follows the mouse until a click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sizing {
    /// Faces (or edges) pushed out along their normal.
    Extrude,
    /// Faces shrunk into themselves, a frame of new faces round them.
    Inset,
    /// Edges rounded off.
    Bevel,
    /// A cut across the ring of faces through this edge, slid along it.
    LoopCut((u32, u32)),
}

/// The last operation with a size: done again from what was there before when its numbers
/// are changed in the panel (Blender's Adjust Last Operation).
#[derive(Clone, Debug)]
pub struct Last {
    pub kind: Sizing,
    pub value: f32,
    /// Bevel's segments, a loop cut's cuts.
    pub count: u32,
    base: Mesh,
    base_sel: Selection,
}

struct Modal {
    kind: Sizing,
    base: Mesh,
    base_sel: Selection,
    start_px: DVec2,
    /// Where the selection is (world) and which way it faces.
    anchor: DVec3,
    normal: DVec3,
    start_t: f64,
    count: u32,
    value: f32,
}

pub struct Session {
    /// The object's file, and the map object it is edited on.
    pub rel: String,
    pub inst: Sel,
    /// A copy of its own made only so that the mesh could be read, which the map object does
    /// not point at yet: it is handed the copy at the first change, and the copy goes when the
    /// edit mode is left before anything was changed.
    pub made: Option<String>,
    pub src: Source,
    /// The part being edited.
    pub part: usize,
    pub mode: SelectMode,
    pub sel: Selection,
    pub editing: bool,
    /// The files need writing (since `changed_at`).
    pub dirty: bool,
    changed_at: Instant,
    undo: Vec<(Source, Selection)>,
    redo: Vec<(Source, Selection)>,
    modal: Option<Modal>,
    pub last: Option<Last>,
    pub hover: Option<Hit>,
    /// Pick and box-select through the mesh, not only what is seen.
    pub xray: bool,
    /// The object's own textures are shown in the edit view, under the shading tinted over
    /// them with `shade` (0..1: 0 the textures as they are, 1 the plain shading).
    pub tex_preview: bool,
    pub shade: f32,
    /// The object's `texture` folder (for the pictures of its materials).
    pub tex_dir: std::path::PathBuf,
    /// Where the object stands, set each frame.
    pub place: DMat4,
    /// The mesh as a handle drag began.
    drag_base: Option<Mesh>,
    /// Counts changes; the other parts as their modifiers make them, at that count.
    rev: u64,
    others: std::cell::RefCell<Option<(u64, Vec<(usize, Mesh)>)>>,
    /// The edited part in triangles (for picking), at a count and part.
    tris: std::cell::RefCell<Option<(u64, usize, openmapper_mesh::TriMesh)>>,
}

impl Session {
    pub fn open(roots: &Roots, rel: &str, inst: Sel) -> Option<Session> {
        let src = editable::load(roots, rel)?;
        Some(Session {
            rel: rel.to_string(),
            inst,
            made: None,
            src,
            part: 0,
            mode: SelectMode::Vertex,
            sel: Selection::default(),
            editing: false,
            dirty: false,
            changed_at: Instant::now(),
            undo: Vec::new(),
            redo: Vec::new(),
            modal: None,
            last: None,
            hover: None,
            xray: false,
            tex_preview: false,
            shade: 0.5,
            tex_dir: editable::folder_of(roots, rel).0.join("texture"),
            place: DMat4::IDENTITY,
            drag_base: None,
            rev: 0,
            others: Default::default(),
            tris: Default::default(),
        })
    }

    pub fn mesh(&self) -> &Mesh {
        &self.src.model.parts[self.part.min(self.src.model.parts.len().saturating_sub(1))].mesh
    }

    pub fn mesh_mut(&mut self) -> &mut Mesh {
        let i = self.part.min(self.src.model.parts.len().saturating_sub(1));
        // (an edited part is no longer the shape it was made as)
        self.src.model.parts[i].primitive = None;
        &mut self.src.model.parts[i].mesh
    }

    /// Keep things as they are before a change, for undo.
    pub fn checkpoint(&mut self) {
        self.undo.push((self.src.clone(), self.sel.clone()));
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn changed(&mut self) {
        self.rev += 1;
        self.dirty = true;
        self.changed_at = Instant::now();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some((s, sel)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.src, s), std::mem::replace(&mut self.sel, sel)));
            self.after_undo();
        }
    }

    pub fn redo(&mut self) {
        if let Some((s, sel)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.src, s), std::mem::replace(&mut self.sel, sel)));
            self.after_undo();
        }
    }

    fn after_undo(&mut self) {
        self.part = self.part.min(self.src.model.parts.len().saturating_sub(1));
        self.last = None;
        self.modal = None;
        self.changed();
    }

    /// Write the files once a change has rested a moment (or at once with `now`): whether
    /// they were written.
    pub fn write_if_due(&mut self, roots: &Roots, now: bool, ground: Option<&dyn Fn(glam::Vec2) -> f32>) -> anyhow::Result<bool> {
        if !self.dirty || self.modal.is_some() || self.drag_base.is_some() || (!now && self.changed_at.elapsed().as_millis() < 350) {
            return Ok(false);
        }
        editable::save_on(roots, &self.rel, &self.src, ground)?;
        self.dirty = false;
        Ok(true)
    }

    pub fn set_mode(&mut self, mode: SelectMode) {
        self.mode = mode;
        // (the selection is kept: what is fully selected in the new mode stays selected)
        let verts: Vec<u32> = self.sel.vertices();
        self.sel = match mode {
            SelectMode::Vertex => Selection::from_verts(self.mesh(), verts.iter().copied()),
            SelectMode::Edge => Selection::from_verts(self.mesh(), verts.iter().copied()),
            SelectMode::Face => {
                let faces: Vec<usize> = self.sel.faces.iter().copied().collect();
                Selection::from_faces(self.mesh(), faces.iter().copied())
            }
        };
    }

    // ------------------------------------------------------------------------- the view

    fn world(&self, p: Vec3) -> DVec3 {
        self.place.transform_point3(p.as_dvec3())
    }

    fn rotation(&self) -> DQuat {
        let (_, r, _) = self.place.to_scale_rotation_translation();
        r
    }

    /// What is under the cursor in the selection mode: the nearest vertex or edge on the
    /// screen within a few points, the face the cursor's ray meets first. Only what is seen
    /// counts, unless x-ray is on.
    /// The edited part's triangles, made again only after a change.
    fn triangles(&self) -> std::cell::Ref<'_, openmapper_mesh::TriMesh> {
        let stale = self.tris.borrow().as_ref().map(|t| t.0 != self.rev || t.1 != self.part).unwrap_or(true);
        if stale {
            *self.tris.borrow_mut() = Some((self.rev, self.part, self.mesh().triangles(180.0)));
        }
        std::cell::Ref::map(self.tris.borrow(), |t| &t.as_ref().unwrap().2)
    }

    pub fn pick(&self, view: &View) -> Option<Hit> {
        let mesh = self.mesh();
        let (o, d) = view.ray();
        let tris = self.triangles();
        let local_o = self.place.inverse().transform_point3(o);
        let local_d = self.place.inverse().transform_vector3(d).normalize();
        let first_hit = ray_tris(&tris, local_o, local_d);
        let seen = |p: Vec3| -> bool {
            if self.xray {
                return true;
            }
            let pw = self.place.transform_point3(p.as_dvec3());
            let eye = view.cam.eye();
            let lo = self.place.inverse().transform_point3(eye);
            let to = p.as_dvec3() - lo;
            let dist = to.length();
            match ray_tris(&tris, lo, to / dist.max(1e-9)) {
                Some((t, _)) => t > dist - 0.01 * dist.max(1.0) - view.metres_per_point(pw) * 2.0,
                None => true,
            }
        };
        let px = |p: Vec3| view_px(view, self.world(p));
        match self.mode {
            SelectMode::Vertex => {
                let mut best: Option<(f64, u32)> = None;
                for (i, p) in mesh.positions.iter().enumerate() {
                    let Some(s) = px(*p) else { continue };
                    let dd = (s - view.pointer).length();
                    if dd < 12.0 && best.map(|b| dd < b.0).unwrap_or(true) && seen(*p) {
                        best = Some((dd, i as u32));
                    }
                }
                best.map(|b| Hit::Vert(b.1))
            }
            SelectMode::Edge => {
                let mut best: Option<(f64, (u32, u32))> = None;
                for (a, b) in mesh.edges() {
                    let (pa, pb) = (mesh.positions[a as usize], mesh.positions[b as usize]);
                    let (Some(sa), Some(sb)) = (px(pa), px(pb)) else { continue };
                    let dd = seg_dist(view.pointer, sa, sb);
                    if dd < 10.0 && best.map(|x| dd < x.0).unwrap_or(true) && seen((pa + pb) * 0.5) {
                        best = Some((dd, (a, b)));
                    }
                }
                best.map(|b| Hit::Edge(b.1))
            }
            SelectMode::Face if self.xray => {
                // (x-ray: the face whose middle is nearest the cursor, near or far side; with
                // none near, the first face under the cursor as ever)
                let near = (0..mesh.faces.len())
                    .filter_map(|fi| px(mesh.face_center(fi)).map(|s| ((s - view.pointer).length(), fi)))
                    .filter(|(dd, _)| *dd < 14.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                near.map(|(_, f)| Hit::Face(f)).or(first_hit.map(|(_, f)| Hit::Face(f)))
            }
            SelectMode::Face => first_hit.map(|(_, f)| Hit::Face(f)),
        }
    }

    /// A click on `hit` (nothing: all let go unless Shift); Shift adds or takes away, Alt
    /// takes the edge loop through it.
    pub fn click(&mut self, hit: Option<Hit>, shift: bool, alt: bool) {
        let mesh = self.mesh().clone();
        let Some(hit) = hit else {
            if !shift {
                self.sel = Selection::default();
            }
            return;
        };
        if alt {
            let e = match hit {
                Hit::Edge(e) => Some(e),
                Hit::Vert(v) => mesh.edges().into_iter().find(|e| e.0 == v || e.1 == v),
                Hit::Face(f) => mesh.faces.get(f).and_then(|f| f.edges().next()).map(|(a, b)| edge_key(a, b)),
            };
            if let Some(e) = e {
                let mut l = edit::edge_loop(&mesh, e);
                if self.mode == SelectMode::Face {
                    l = edit::edge_ring(&mesh, e);
                    let faces: Vec<usize> = l.faces.iter().copied().collect();
                    l = Selection::from_faces(&mesh, faces.iter().copied());
                }
                if shift {
                    let mut v = self.sel.vertices();
                    v.extend(l.vertices());
                    self.sel = Selection::from_verts(&mesh, v.iter().copied());
                } else {
                    self.sel = l;
                }
            }
            return;
        }
        if !shift {
            self.sel = Selection::default();
        }
        match hit {
            Hit::Vert(v) => self.sel.toggle_vert(&mesh, v),
            Hit::Edge(e) => self.sel.toggle_edge(&mesh, e),
            Hit::Face(f) => self.sel.toggle_face(&mesh, f),
        }
    }

    /// Everything of the mode inside a box on the screen (view points) selected, added to
    /// the selection with `add`.
    pub fn box_select(&mut self, view: &View, a: DVec2, b: DVec2, add: bool) {
        let (lo, hi) = (a.min(b), a.max(b));
        let inside = |p: DVec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
        let mesh = self.mesh().clone();
        let picked: Vec<u32> = {
            let tris = self.triangles();
            let eye = self.place.inverse().transform_point3(view.cam.eye());
            let xray = self.xray;
            let seen = |p: Vec3| -> bool {
                if xray {
                    return true;
                }
                let to = p.as_dvec3() - eye;
                let dist = to.length();
                ray_tris(&tris, eye, to / dist.max(1e-9)).map(|(t, _)| t > dist * 0.995 - 0.01).unwrap_or(true)
            };
            mesh.positions.iter().enumerate().filter(|(_, p)| view_px(view, self.world(**p)).map(inside).unwrap_or(false) && seen(**p)).map(|(i, _)| i as u32).collect()
        };
        let mut verts = if add { self.sel.vertices() } else { Vec::new() };
        verts.extend(picked);
        self.sel = match self.mode {
            SelectMode::Face => {
                let set: std::collections::HashSet<u32> = verts.iter().copied().collect();
                let mut faces: Vec<usize> = if add { self.sel.faces.iter().copied().collect() } else { Vec::new() };
                faces.extend(mesh.faces.iter().enumerate().filter(|(_, f)| f.verts.iter().all(|v| set.contains(v))).map(|(i, _)| i));
                Selection::from_faces(&mesh, faces.iter().copied())
            }
            _ => Selection::from_verts(&mesh, verts.iter().copied()),
        };
    }

    // ---------------------------------------------------------------------- the handles

    /// The handles at the middle of the selection, along the object's own axes.
    pub fn gizmo(&self, mode: gizmo::Mode) -> Option<gizmo::Gizmo> {
        if self.sel.is_empty() || self.modal.is_some() {
            return None;
        }
        let c = edit::selection_center(self.mesh(), &self.sel)?;
        Some(gizmo::Gizmo { mode, pivot: self.world(c), axes: glam::DMat3::from_quat(self.rotation()), allowed: [true; 3], cap: f64::INFINITY })
    }

    pub fn begin_drag(&mut self) {
        self.checkpoint();
        self.drag_base = Some(self.mesh().clone());
    }

    /// A handle dragged: the selected vertices moved, turned or scaled from where they were.
    pub fn drag(&mut self, change: gizmo::Change, pivot: DVec3) {
        let Some(base) = self.drag_base.clone() else { return };
        let inv = self.rotation().inverse();
        let c = self.place.inverse().transform_point3(pivot).as_vec3();
        let m = match change {
            gizmo::Change::Move(d) => Mat4::from_translation((inv * d).as_vec3()),
            gizmo::Change::Turn(axis, deg) => {
                let a = (inv * axis).as_vec3().normalize();
                Mat4::from_translation(c) * Mat4::from_axis_angle(a, (deg as f32).to_radians()) * Mat4::from_translation(-c)
            }
            gizmo::Change::Scale(s) => Mat4::from_translation(c) * Mat4::from_scale(s.as_vec3()) * Mat4::from_translation(-c),
        };
        let sel = self.sel.clone();
        let mesh = self.mesh_mut();
        *mesh = base;
        edit::transform_selection(mesh, &sel, m);
    }

    pub fn end_drag(&mut self) {
        if self.drag_base.take().is_some() {
            self.last = None;
            self.changed();
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag_base.is_some()
    }

    pub fn cancel_drag(&mut self) {
        if let Some(base) = self.drag_base.take() {
            *self.mesh_mut() = base;
            self.undo.pop();
        }
    }

    // --------------------------------------------------------------- sized operations

    pub fn modal_active(&self) -> bool {
        self.modal.is_some()
    }

    /// The value an operation with a size has come to, for the status bar.
    pub fn modal_value(&self) -> Option<(Sizing, f32)> {
        self.modal.as_ref().map(|m| (m.kind, m.value))
    }

    /// Begin an operation whose size follows the mouse (a click ends it).
    pub fn start(&mut self, kind: Sizing, view: &View) {
        let mesh = self.mesh().clone();
        let local_c = match kind {
            Sizing::LoopCut(e) => (mesh.positions[e.0 as usize] + mesh.positions[e.1 as usize]) * 0.5,
            _ => match edit::selection_center(&mesh, &self.sel) {
                Some(c) => c,
                None => return,
            },
        };
        let anchor = self.world(local_c);
        let normal = self.rotation() * edit::selection_normal(&mesh, &self.sel).as_dvec3();
        let normal = if normal.length_squared() < 1e-9 { DVec3::Z } else { normal.normalize() };
        self.checkpoint();
        let count = match kind {
            Sizing::Bevel => self.last.as_ref().filter(|l| l.kind == Sizing::Bevel).map(|l| l.count).unwrap_or(1),
            _ => 1,
        };
        let start_t = line_param(view, anchor, normal).unwrap_or(0.0);
        self.modal = Some(Modal { kind, base: mesh, base_sel: self.sel.clone(), start_px: view.pointer, anchor, normal, start_t, count, value: 0.0 });
        self.update_modal(view);
    }

    /// The operation done again for where the mouse is now.
    pub fn update_modal(&mut self, view: &View) {
        let Some(m) = &self.modal else { return };
        let mpp = view.metres_per_point(m.anchor);
        let c = view_px(view, m.anchor).unwrap_or(m.start_px);
        let _grown = ((view.pointer - c).length() - (m.start_px - c).length()) * mpp;
        // (an inset or a bevel grows with how far the pointer has gone from where it began, whichever
        // way: started from a panel's button, the way in to the mesh is not a negative size)
        let moved = (view.pointer - m.start_px).length() * mpp;
        let value = match m.kind {
            Sizing::Extrude => line_param(view, m.anchor, m.normal).map(|t| t - m.start_t).unwrap_or(0.0) as f32,
            Sizing::Inset | Sizing::Bevel => moved as f32,
            Sizing::LoopCut(e) => {
                // (along the edge on the screen: -1 at one end, 1 at the other)
                let mesh = &m.base;
                let (pa, pb) = (self.world(mesh.positions[e.0 as usize]), self.world(mesh.positions[e.1 as usize]));
                match (view_px(view, pa), view_px(view, pb)) {
                    (Some(a), Some(b)) => {
                        let ab = b - a;
                        (((view.pointer - (a + b) * 0.5).dot(ab) / ab.length_squared().max(1e-6)) * 2.0).clamp(-0.95, 0.95) as f32
                    }
                    _ => 0.0,
                }
            }
        };
        let (kind, count, base, base_sel) = (m.kind, m.count, m.base.clone(), m.base_sel.clone());
        if let Some(m) = &mut self.modal {
            m.value = value;
        }
        self.apply_sized(kind, value, count, &base, &base_sel);
    }

    fn apply_sized(&mut self, kind: Sizing, value: f32, count: u32, base: &Mesh, base_sel: &Selection) {
        let mode = self.mode;
        let mut mesh = base.clone();
        let sel = match kind {
            Sizing::Extrude => edit::extrude_along_normals(&mut mesh, base_sel, value),
            Sizing::Inset => edit::inset(&mut mesh, base_sel, value.max(0.0), 0.0, false),
            Sizing::Bevel => edit::bevel(&mut mesh, base_sel, mode, value.max(0.0), count.max(1)),
            Sizing::LoopCut(e) => edit::loop_cut(&mut mesh, e, count.max(1), if count <= 1 { value } else { 0.0 }),
        };
        *self.mesh_mut() = mesh;
        self.sel = sel;
    }

    /// Change the cuts or segments of the operation going on (the mouse wheel).
    pub fn modal_count(&mut self, delta: i32) {
        if let Some(m) = &mut self.modal {
            m.count = (m.count as i32 + delta).clamp(1, 32) as u32;
        }
    }

    pub fn confirm(&mut self) {
        if let Some(m) = self.modal.take() {
            self.last = Some(Last { kind: m.kind, value: m.value, count: m.count, base: m.base, base_sel: m.base_sel });
            self.changed();
        }
    }

    pub fn cancel(&mut self) {
        if let Some(m) = self.modal.take() {
            *self.mesh_mut() = m.base;
            self.sel = m.base_sel;
            self.undo.pop();
        }
    }

    /// The last sized operation done again with new numbers.
    pub fn redo_last(&mut self, value: f32, count: u32) {
        let Some(l) = self.last.clone() else { return };
        self.apply_sized(l.kind, value, count, &l.base, &l.base_sel);
        if let Some(l) = &mut self.last {
            l.value = value;
            l.count = count;
        }
        self.changed();
    }

    /// An operation on the selection, as one undo step.
    pub fn apply(&mut self, f: impl FnOnce(&mut Mesh, &Selection, SelectMode) -> Option<Selection>) {
        self.checkpoint();
        let sel = self.sel.clone();
        let mode = self.mode;
        let new = f(self.mesh_mut(), &sel, mode);
        if let Some(s) = new {
            self.sel = s;
        }
        // (vertices renumbered: what was selected may name nothing now)
        let n = self.mesh().positions.len() as u32;
        let nf = self.mesh().faces.len();
        self.sel.verts.retain(|v| *v < n);
        self.sel.edges.retain(|e| e.0 < n && e.1 < n);
        self.sel.faces.retain(|f| *f < nf);
        self.last = None;
        self.changed();
    }

    // ------------------------------------------------------------------------- drawing

    /// The mesh as it is being edited: every part solid (the edited one lit, the selected
    /// faces tinted), its edges and, in vertex mode, its vertices; seams red, sharp edges cyan.
    pub fn extras(&self, view: &View, x: &mut FrameExtras) {
        let eye = view.cam.eye();
        let light = Vec3::new(0.35, -0.45, 0.82).normalize();
        let sel_faces = &self.sel.faces;
        // (the other parts as their modifiers make them; the edited one as it is)
        let others = {
            let mut c = self.others.borrow_mut();
            if c.as_ref().map(|x| x.0 != self.rev).unwrap_or(true) {
                *c = Some((self.rev, if self.src.model.parts.len() > 1 { self.src.model.evaluated(None) } else { Vec::new() }));
            }
            c.as_ref().map(|x| x.1.clone()).unwrap_or_default()
        };
        for (pi, part) in self.src.model.parts.iter().enumerate() {
            if !part.visible {
                continue;
            }
            let active = pi == self.part;
            let mesh = if active { part.mesh.clone() } else { others.iter().find(|(i, _)| *i == pi).map(|(_, m)| m.clone()).unwrap_or_default() };
            for (fi, _) in mesh.faces.iter().enumerate() {
                let n = mesh.face_normal(fi);
                let lit = 0.45 + 0.55 * n.dot(light).max(0.0);
                let base = if !active {
                    [0.55, 0.56, 0.58]
                } else if sel_faces.contains(&fi) && self.mode == SelectMode::Face {
                    [1.0, 0.58, 0.2]
                } else {
                    let m = self.src.model.materials.get(mesh.faces[fi].material as usize).map(|m| m.color).unwrap_or([0.8; 4]);
                    [0.35 + m[0] * 0.4, 0.35 + m[1] * 0.4, 0.35 + m[2] * 0.4]
                };
                // (x-ray: every face see-through, the picked ones a little less, so the far side
                // of the mesh shows through the near)
                let picked_face = active && sel_faces.contains(&fi);
                let alpha = if self.xray {
                    if picked_face { 0.6 } else if active { 0.22 } else { 0.12 }
                } else if self.tex_preview {
                    // (the textures show through the shading; the picked faces always show)
                    if picked_face { self.shade.max(0.45) } else { self.shade }
                } else {
                    1.0
                };
                if alpha <= 0.004 {
                    continue;
                }
                // (over the object's own textures the shading lies a hair toward the eye)
                let hair = |p: DVec3| if self.tex_preview && !self.xray { p + (eye - p).normalize_or_zero() * view.metres_per_point(p) * 1.5 } else { p };
                let col = [base[0] * lit, base[1] * lit, base[2] * lit, alpha];
                let dst = if self.xray { &mut x.overlay } else { &mut x.solid };
                for t in mesh.triangulate_face(fi) {
                    for k in t {
                        dst.push(OverlayPoint { pos: hair(self.world(mesh.positions[mesh.faces[fi].verts[k] as usize])), color: col });
                    }
                }
            }
        }
        let mesh = self.mesh();
        // edges, lifted a little toward the eye so they are not hidden in their faces
        let lift = |p: DVec3, by: f64| p + (eye - p).normalize_or_zero() * by;
        let sel_edges = &self.sel.edges;
        // (x-ray: edges and points are drawn over everything, not hidden behind the faces)
        let marks = if self.xray { &mut x.overlay } else { &mut x.overlay_depth };
        let hover_edge = match self.hover {
            Some(Hit::Edge(e)) => Some(e),
            _ => None,
        };
        for (a, b) in mesh.edges() {
            let (pa, pb) = (self.world(mesh.positions[a as usize]), self.world(mesh.positions[b as usize]));
            let mpp = view.metres_per_point((pa + pb) * 0.5);
            let k = (a, b);
            let color = if hover_edge == Some(k) {
                [1.0, 1.0, 1.0, 1.0]
            } else if sel_edges.contains(&k) {
                [1.0, 0.62, 0.15, 1.0]
            } else if mesh.is_seam(a, b) {
                [0.95, 0.2, 0.2, 1.0]
            } else if mesh.is_sharp(a, b) {
                [0.2, 0.85, 0.95, 1.0]
            } else {
                [0.05, 0.05, 0.06, 0.9]
            };
            let w = if sel_edges.contains(&k) || hover_edge == Some(k) { 2.4 } else { 1.4 };
            gizmo::ribbon(marks, eye, lift(pa, mpp * 2.0), lift(pb, mpp * 2.0), mpp * w, color);
        }
        match self.mode {
            SelectMode::Vertex => {
                for (i, p) in mesh.positions.iter().enumerate() {
                    let w = self.world(*p);
                    let mpp = view.metres_per_point(w);
                    let on = self.sel.verts.contains(&(i as u32));
                    let hot = self.hover == Some(Hit::Vert(i as u32));
                    let color = if hot { [1.0, 1.0, 1.0, 1.0] } else if on { [1.0, 0.62, 0.15, 1.0] } else { [0.02, 0.02, 0.02, 1.0] };
                    square(marks, &view.cam, lift(w, mpp * 3.0), mpp * if on || hot { 6.0 } else { 4.5 }, color);
                }
            }
            SelectMode::Face => {
                for fi in 0..mesh.faces.len() {
                    let w = self.world(mesh.face_center(fi));
                    let mpp = view.metres_per_point(w);
                    let on = self.sel.faces.contains(&fi);
                    let hot = self.hover == Some(Hit::Face(fi));
                    let color = if hot { [1.0, 1.0, 1.0, 1.0] } else if on { [1.0, 0.62, 0.15, 1.0] } else { [0.1, 0.1, 0.1, 0.9] };
                    square(marks, &view.cam, lift(w, mpp * 3.0), mpp * 4.0, color);
                }
            }
            SelectMode::Edge => {}
        }
        // the loop a cut would make, while choosing where
        if let Some(Modal { kind: Sizing::LoopCut(_), .. }) = &self.modal {
            for &(a, b) in &self.sel.edges {
                let (pa, pb) = (self.world(mesh.positions[a as usize]), self.world(mesh.positions[b as usize]));
                let mpp = view.metres_per_point(pa);
                gizmo::ribbon(&mut x.overlay, eye, pa, pb, mpp * 2.5, [1.0, 0.85, 0.2, 1.0]);
            }
        }
    }
}

/// A small square facing the eye.
fn square(out: &mut Vec<OverlayPoint>, cam: &crate::camera::Camera, c: DVec3, size: f64, color: [f32; 4]) {
    let (_, r, u) = cam.axes();
    let (r, u) = (r * size * 0.5, u * size * 0.5);
    let q = [c - r - u, c + r - u, c + r + u, c - r + u];
    for t in [[0, 1, 2], [0, 2, 3]] {
        for k in t {
            out.push(OverlayPoint { pos: q[k], color });
        }
    }
}

fn view_px(view: &View, p: DVec3) -> Option<DVec2> {
    crate::selection::project(&view.cam, p, view.w, view.h).map(|(x, y)| DVec2::new(x as f64, y as f64))
}

fn seg_dist(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Where along the line through `p` along `dir` the cursor's ray passes nearest (metres).
fn line_param(view: &View, p: DVec3, dir: DVec3) -> Option<f64> {
    let (o, d) = view.ray();
    if d.dot(dir).abs() > 0.995 {
        return None;
    }
    let w = o - p;
    let (b, dd, e) = (d.dot(dir), d.dot(w), dir.dot(w));
    Some((e - b * dd) / (1.0 - b * b))
}

/// The first triangle a ray (in the mesh's frame) meets: how far, and its face.
fn ray_tris(t: &openmapper_mesh::TriMesh, o: DVec3, d: DVec3) -> Option<(f64, usize)> {
    let mut best: Option<(f64, usize)> = None;
    for (i, c) in t.indices.chunks(3).enumerate() {
        let (a, b, cc) = (t.positions[c[0] as usize].as_dvec3(), t.positions[c[1] as usize].as_dvec3(), t.positions[c[2] as usize].as_dvec3());
        let (e1, e2) = (b - a, cc - a);
        let pv = d.cross(e2);
        let det = e1.dot(pv);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let tv = o - a;
        let u = tv.dot(pv) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = tv.cross(e1);
        let v = d.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let dist = e2.dot(q) * inv;
        if dist > 1e-6 && best.map(|b| dist < b.0).unwrap_or(true) {
            best = Some((dist, t.tri_face[i] as usize));
        }
    }
    best
}
