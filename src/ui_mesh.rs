//! The interface of openMapper's own objects: the shapes to start from, and (in the
//! properties) their parts, modifiers, materials and lamps.

use openmapper_mesh::primitives::{Primitive, ALL};

use crate::i18n::tr;
use crate::ui::Action;

pub fn primitive_label(p: &Primitive) -> &'static str {
    match p {
        Primitive::Cube { .. } => tr("Cube", "Куб"),
        Primitive::Plane { .. } => tr("Plane", "Плоскость"),
        Primitive::Grid { .. } => tr("Grid", "Сетка"),
        Primitive::Circle { .. } => tr("Circle", "Круг"),
        Primitive::Triangle { .. } => tr("Triangle", "Треугольник"),
        Primitive::Prism { .. } => tr("Triangular prism", "Треугольная призма"),
        Primitive::Wedge { .. } => tr("Wedge", "Клин"),
        Primitive::CornerWedge { .. } => tr("Corner wedge", "Угловой клин"),
        Primitive::Pyramid { .. } => tr("Pyramid", "Пирамида"),
        Primitive::UvSphere { .. } => tr("Sphere", "Сфера"),
        Primitive::IcoSphere { .. } => tr("Icosphere", "Икосфера"),
        Primitive::Cylinder { .. } => tr("Cylinder", "Цилиндр"),
        Primitive::Cone { .. } => tr("Cone", "Конус"),
        Primitive::Torus { .. } => tr("Torus", "Тор"),
        Primitive::Tube { .. } => tr("Tube", "Труба"),
        Primitive::Capsule { .. } => tr("Capsule", "Капсула"),
        Primitive::Stairs { .. } => tr("Stairs", "Лестница"),
        Primitive::Arch { .. } => tr("Arch", "Арка"),
    }
}

/// The shapes as buttons; a click makes one to put on the map.
pub fn primitives_grid(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (i, p) in ALL.iter().enumerate() {
            if ui.add_sized([96.0, 24.0], egui::Button::new(primitive_label(p))).on_hover_text(tr("Make one and put it on the map with a click (each one can then be edited, scaled, cut and joined)", "Создать и поставить на карту кликом (потом его можно редактировать, масштабировать, резать и объединять)")).clicked() {
                actions.push(Action::CreatePrimitive(i));
            }
        }
    });
}

/// The shapes as a menu.
pub fn primitives_menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    for (i, p) in ALL.iter().enumerate() {
        if ui.button(primitive_label(p)).clicked() {
            actions.push(Action::CreatePrimitive(i));
            ui.close();
        }
    }
}

// ------------------------------------------------------------------------------ actions

/// A texture of a material: its colour picture, or one of the maps beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexSlot {
    Color,
    Normal,
    Orm,
    Night,
    Light,
}

impl TexSlot {
    pub fn suffix(self) -> &'static str {
        match self {
            TexSlot::Color => "",
            TexSlot::Normal => "_nn",
            TexSlot::Orm => "_orm",
            TexSlot::Night => "_night",
            TexSlot::Light => "_lm",
        }
    }
}

/// What a bake makes: a picture `size` square, with ambient occlusion and the lamps' light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeChoice {
    pub size: u32,
    pub ao: bool,
    pub ao_distance: f32,
    pub light: bool,
}

impl Default for BakeChoice {
    fn default() -> BakeChoice {
        BakeChoice { size: 1024, ao: true, ao_distance: 1.5, light: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshAction {
    /// Into the edit mode of the selected object, or out of it (Tab).
    ToggleEdit,
    MakeSingle,
    SeparateParts,
    Join,
    Boolean(crate::meshops::Bool, bool),
    /// The selected faces made an object of their own (P).
    SeparateSelected,
    Export,
    /// From the view's right-click menu: any object or road to O3D, OBJ, FBX, GLB or X.
    ExportModel,
    ExportTexture(usize),
    PickTexture(usize, TexSlot),
    ImportMaterial,
    Bake(BakeChoice),
    NewSignal(Vec<crate::editable::Lamp>),
    NewLamp,
    /// A picture or repeating texture on a panel, a light source, a sound source.
    NewPicture(crate::makers::Settings),
    NewLight(crate::makers::Settings),
    NewSound(crate::makers::Settings),
    /// The selected roads and splines made an openMapper object of their shape (in place of
    /// them with `true`), to edit as a mesh.
    SplinesToObject(bool),
    /// A polygon: click its corners on the map, or fill the outline the selected splines make.
    PolygonDraw,
    PolygonFromSplines,
}

use crate::editable::{Lamp, Light, LightKind, When};
use crate::editor::Editor;
use crate::meshedit::{Session, Sizing};
use egui::{Color32, RichText};
use openmapper_mesh::edit::{self, Delete, MergeAt, SelectMode};
use openmapper_mesh::modifiers::{CastShape, DeformMode, Modifier};
use openmapper_mesh::{Axis, Material, Stacked};

const DIM: Color32 = Color32::from_rgb(140, 143, 150);

/// A change made in the panel: kept for undo when it begins, the files written after it.
fn touched(m: &mut Session, r: &egui::Response) {
    if r.drag_started() || (r.changed() && !r.dragged()) {
        m.checkpoint();
    }
    if r.changed() {
        m.changed();
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong().color(Color32::WHITE));
    ui.add_space(2.0);
}

pub fn modifier_label(m: &Modifier) -> &'static str {
    match m {
        Modifier::Array { .. } => tr("Array", "Массив"),
        Modifier::Mirror { .. } => tr("Mirror", "Зеркало"),
        Modifier::Solidify { .. } => tr("Solidify", "Толщина"),
        Modifier::Bevel { .. } => tr("Bevel", "Фаска"),
        Modifier::Subdivision { .. } => tr("Subdivision surface", "Подразделение поверхности"),
        Modifier::Decimate { .. } => tr("Decimate", "Упрощение"),
        Modifier::Triangulate => tr("Triangulate", "Триангуляция"),
        Modifier::Smooth { .. } => tr("Smooth", "Сглаживание"),
        Modifier::Displace { .. } => tr("Displace", "Смещение (шум)"),
        Modifier::Wireframe { .. } => tr("Wireframe", "Каркас"),
        Modifier::Screw { .. } => tr("Screw", "Винт"),
        Modifier::SimpleDeform { .. } => tr("Simple deform", "Простая деформация"),
        Modifier::Cast { .. } => tr("Cast", "Отливка (к форме)"),
        Modifier::Weld { .. } => tr("Weld", "Сварка вершин"),
        Modifier::EdgeSplit { .. } => tr("Edge split", "Разделение рёбер"),
        Modifier::Boolean { .. } => tr("Boolean", "Булева операция"),
        Modifier::Ground { .. } => tr("Follow the ground", "По рельефу"),
    }
}

fn axis_combo(ui: &mut egui::Ui, id: egui::Id, a: &mut Axis) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(format!("{a:?}")).width(50.0).show_ui(ui, |ui| {
        for x in [Axis::X, Axis::Y, Axis::Z] {
            changed |= ui.selectable_value(a, x, format!("{x:?}")).changed();
        }
    });
    changed
}

fn num(ui: &mut egui::Ui, label: &str, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>, suffix: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(DIM));
        ui.add(egui::DragValue::new(v).speed(speed).range(range).max_decimals(3).suffix(suffix))
    })
    .inner
}

fn count(ui: &mut egui::Ui, label: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(DIM));
        ui.add(egui::DragValue::new(v).speed(0.1).range(range))
    })
    .inner
}

fn vec3(ui: &mut egui::Ui, label: &str, v: &mut glam::Vec3, speed: f64) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(DIM));
        let a = ui.add(egui::DragValue::new(&mut v.x).speed(speed).max_decimals(3).prefix("x "));
        let b = ui.add(egui::DragValue::new(&mut v.y).speed(speed).max_decimals(3).prefix("y "));
        let c = ui.add(egui::DragValue::new(&mut v.z).speed(speed).max_decimals(3).prefix("z "));
        a.union(b).union(c)
    })
    .inner
}

/// A modifier's settings; the response of what was changed.
fn modifier_ui(ui: &mut egui::Ui, id: usize, m: &mut Modifier, parts: &[String]) -> Option<egui::Response> {
    let mut rs: Vec<egui::Response> = Vec::new();
    let flag = |ui: &mut egui::Ui, v: &mut bool, label: &str, rs: &mut Vec<egui::Response>| rs.push(ui.checkbox(v, label));
    match m {
        Modifier::Array { count: c, relative, constant, merge, merge_distance } => {
            rs.push(count(ui, tr("Count", "Количество"), c, 1..=1000));
            rs.push(vec3(ui, tr("Relative", "Относительно"), relative, 0.01));
            rs.push(vec3(ui, tr("Offset", "Сдвиг"), constant, 0.01));
            flag(ui, merge, tr("Merge touching ends", "Сварить стыки"), &mut rs);
            if *merge {
                rs.push(num(ui, tr("Distance", "Расстояние"), merge_distance, 0.001, 0.0..=1.0, " m"));
            }
        }
        Modifier::Mirror { x, y, z, merge, merge_distance, bisect } => {
            ui.horizontal(|ui| {
                flag(ui, x, "X", &mut rs);
                flag(ui, y, "Y", &mut rs);
                flag(ui, z, "Z", &mut rs);
            });
            flag(ui, bisect, tr("Cut away the other side first", "Сначала отрезать другую сторону"), &mut rs);
            flag(ui, merge, tr("Weld at the mirror", "Сварить на оси"), &mut rs);
            if *merge {
                rs.push(num(ui, tr("Distance", "Расстояние"), merge_distance, 0.001, 0.0..=1.0, " m"));
            }
        }
        Modifier::Solidify { thickness, offset, rim } => {
            rs.push(num(ui, tr("Thickness", "Толщина"), thickness, 0.005, -10.0..=10.0, " m"));
            rs.push(num(ui, tr("Offset", "Смещение"), offset, 0.01, -1.0..=1.0, ""));
            flag(ui, rim, tr("Close the rim", "Закрыть край"), &mut rs);
        }
        Modifier::Bevel { width, segments, angle_deg } => {
            rs.push(num(ui, tr("Width", "Ширина"), width, 0.002, 0.0..=10.0, " m"));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 1..=16));
            rs.push(num(ui, tr("Angle", "Угол"), angle_deg, 0.5, 0.0..=180.0, "°"));
        }
        Modifier::Subdivision { levels, smooth } => {
            rs.push(count(ui, tr("Levels", "Уровни"), levels, 0..=4));
            flag(ui, smooth, tr("Smooth (Catmull-Clark)", "Сгладить (Catmull-Clark)"), &mut rs);
        }
        Modifier::Decimate { ratio } => rs.push(num(ui, tr("Ratio", "Доля"), ratio, 0.01, 0.01..=1.0, "")),
        Modifier::Triangulate => {}
        Modifier::Smooth { factor, repeat } => {
            rs.push(num(ui, tr("Factor", "Сила"), factor, 0.01, -2.0..=2.0, ""));
            rs.push(count(ui, tr("Repeat", "Повторы"), repeat, 0..=50));
        }
        Modifier::Displace { strength, scale, seed, along_normal } => {
            rs.push(num(ui, tr("Strength", "Сила"), strength, 0.005, -10.0..=10.0, " m"));
            rs.push(num(ui, tr("Size", "Размер"), scale, 0.01, 0.01..=100.0, " m"));
            rs.push(count(ui, tr("Seed", "Зерно"), seed, 0..=9999));
            flag(ui, along_normal, tr("Along the normals", "Вдоль нормалей"), &mut rs);
        }
        Modifier::Wireframe { thickness } => rs.push(num(ui, tr("Thickness", "Толщина"), thickness, 0.002, 0.001..=5.0, " m")),
        Modifier::Screw { angle_deg, steps, height, axis } => {
            rs.push(num(ui, tr("Angle", "Угол"), angle_deg, 1.0, -3600.0..=3600.0, "°"));
            rs.push(count(ui, tr("Steps", "Шаги"), steps, 1..=1024));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, -100.0..=100.0, " m"));
            let r = ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Axis", "Ось")).color(DIM));
                axis_combo(ui, egui::Id::new(("screw", id)), axis)
            });
            if r.inner {
                rs.push(r.response.clone());
                let mut r2 = r.response;
                r2.mark_changed();
                rs.push(r2);
            }
        }
        Modifier::SimpleDeform { mode, amount, axis } => {
            let mut changed = false;
            ui.horizontal(|ui| {
                for (md, label) in [(DeformMode::Twist, tr("Twist", "Скрутка")), (DeformMode::Bend, tr("Bend", "Изгиб")), (DeformMode::Taper, tr("Taper", "Сужение")), (DeformMode::Stretch, tr("Stretch", "Растяжение"))] {
                    changed |= ui.selectable_value(mode, md, label).changed();
                }
            });
            let unit = if matches!(mode, DeformMode::Twist | DeformMode::Bend) { "°" } else { "" };
            rs.push(num(ui, tr("Amount", "Величина"), amount, 0.5, -3600.0..=3600.0, unit));
            changed |= ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Axis", "Ось")).color(DIM));
                axis_combo(ui, egui::Id::new(("deform", id)), axis)
            })
            .inner;
            if changed {
                let mut r = ui.label("");
                r.mark_changed();
                rs.push(r);
            }
        }
        Modifier::Cast { shape, factor } => {
            let mut changed = false;
            ui.horizontal(|ui| {
                for (sh, label) in [(CastShape::Sphere, tr("Sphere", "Сфера")), (CastShape::Cylinder, tr("Cylinder", "Цилиндр")), (CastShape::Cuboid, tr("Box", "Куб"))] {
                    changed |= ui.selectable_value(shape, sh, label).changed();
                }
            });
            rs.push(num(ui, tr("Factor", "Сила"), factor, 0.01, -2.0..=2.0, ""));
            if changed {
                let mut r = ui.label("");
                r.mark_changed();
                rs.push(r);
            }
        }
        Modifier::Weld { distance } => rs.push(num(ui, tr("Distance", "Расстояние"), distance, 0.001, 0.0..=10.0, " m")),
        Modifier::EdgeSplit { angle_deg } => rs.push(num(ui, tr("Angle", "Угол"), angle_deg, 0.5, 0.0..=180.0, "°")),
        Modifier::Boolean { op, operand } => {
            let mut changed = false;
            ui.horizontal(|ui| {
                use openmapper_mesh::csg::BoolOp;
                for (o, label) in [(BoolOp::Difference, tr("Difference", "Вычитание")), (BoolOp::Union, tr("Union", "Объединение")), (BoolOp::Intersection, tr("Intersection", "Пересечение"))] {
                    changed |= ui.selectable_value(op, o, label).changed();
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("With the part", "С частью")).color(DIM));
                egui::ComboBox::from_id_salt(("bool", id)).selected_text(if operand.is_empty() { "—" } else { operand.as_str() }).show_ui(ui, |ui| {
                    for p in parts {
                        changed |= ui.selectable_value(operand, p.clone(), p).changed();
                    }
                });
            });
            ui.label(RichText::new(tr("Tip: hide the cutting part (its eye in the list) so only the cut shows", "Совет: скройте режущую часть (глаз в списке), чтобы был виден только вырез")).small().color(DIM));
            if changed {
                let mut r = ui.label("");
                r.mark_changed();
                rs.push(r);
            }
        }
        Modifier::Ground { offset } => {
            rs.push(num(ui, tr("Above the ground", "Над землёй"), offset, 0.01, -10.0..=10.0, " m"));
            ui.label(RichText::new(tr("Every vertex follows the terrain under it (in the editor's view and the written files at the place it stands)", "Каждая вершина следует рельефу под ней")).small().color(DIM));
        }
    }
    rs.into_iter().reduce(|a, b| a.union(b))
}

// ------------------------------------------------------------------------- the panels

/// The properties of an openMapper object (the selected one): its parts, modifiers,
/// materials, lamps, baking and export.
pub fn object_panel(ui: &mut egui::Ui, ed: &mut Editor, bake: &mut BakeChoice, actions: &mut Vec<Action>) {
    let users = ed.mesh.as_ref().map(|m| crate::meshops::users(ed, &m.rel)).unwrap_or(0);
    let Some(m) = ed.mesh.as_mut() else { return };
    heading(ui, tr("Mesh (openMapper object)", "Меш (объект openMapper)"));
    if users > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{} {users} {}", tr("Shared by", "Используется"), tr("objects on the map: changes apply to all", "объектами карты: изменения затронут все"))).small().color(Color32::from_rgb(230, 180, 90)));
            if ui.small_button(tr("Make this one single", "Сделать отдельной копией")).clicked() {
                actions.push(Action::Mesh(MeshAction::MakeSingle));
            }
        });
    }
    ui.horizontal(|ui| {
        let label = if m.editing { tr("Leave the edit mode (Tab)", "Выйти из редактирования (Tab)") } else { tr("Edit the mesh (Tab)", "Редактировать меш (Tab)") };
        if ui.add(egui::Button::new(RichText::new(label).strong()).fill(if m.editing { crate::ui::ACCENT } else { Color32::from_rgb(52, 54, 60) })).clicked() {
            actions.push(Action::Mesh(MeshAction::ToggleEdit));
        }
        if ui.button(tr("Export…", "Экспорт…")).on_hover_text(tr("Write it as OBJ, glTF / GLB, FBX, Collada, USD, STL, PLY, 3MF, OFF, X or O3D", "Записать в OBJ, glTF / GLB, FBX, Collada, USD, STL, PLY, 3MF, OFF, X или O3D")).clicked() {
            actions.push(Action::Mesh(MeshAction::Export));
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Name", "Имя")).color(DIM));
        let mut n = m.src.name.clone();
        let r = ui.text_edit_singleline(&mut n);
        if r.changed() {
            m.src.name = n;
        }
        touched(m, &r);
    });
    ui.horizontal(|ui| {
        let mut abs = m.src.abs_height;
        let r = ui.checkbox(&mut abs, tr("Height above sea level", "Высота над уровнем моря")).on_hover_text(tr("Off: its height is kept above the ground under it", "Выкл.: высота считается от земли под ним"));
        if r.changed() {
            m.src.abs_height = abs;
        }
        touched(m, &r);
        let mut c = m.src.collision;
        let r = ui.checkbox(&mut c, tr("Solid (collides)", "Твёрдый (столкновения)"));
        if r.changed() {
            m.src.collision = c;
        }
        touched(m, &r);
    });

    // parts
    heading(ui, &format!("{} ({})", tr("Parts", "Части"), m.src.model.parts.len()));
    let mut remove = None;
    let mut dup = None;
    let n_parts = m.src.model.parts.len();
    for i in 0..n_parts {
        ui.horizontal(|ui| {
            let active = m.part == i;
            if ui.radio(active, "").on_hover_text(tr("The part edited and given modifiers", "Часть, которую редактируют и которой задают модификаторы")).clicked() {
                m.part = i;
                m.sel = Default::default();
            }
            let mut vis = m.src.model.parts[i].visible;
            let r = ui.checkbox(&mut vis, "").on_hover_text(tr("Shown (and written into the object)", "Показана (и записывается в объект)"));
            if r.changed() {
                m.src.model.parts[i].visible = vis;
            }
            touched(m, &r);
            let mut name = m.src.model.parts[i].name.clone();
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(110.0));
            if r.changed() && !name.trim().is_empty() {
                let old = std::mem::replace(&mut m.src.model.parts[i].name, name.clone());
                for v in &mut m.src.part_vars {
                    if v.part == old {
                        v.part = name.clone();
                    }
                }
            }
            touched(m, &r);
            ui.label(RichText::new(format!("{}", m.src.model.parts[i].mesh.faces.len())).small().color(DIM)).on_hover_text(tr("faces", "граней"));
            if ui.small_button("⧉").on_hover_text(tr("Duplicate the part", "Дублировать часть")).clicked() {
                dup = Some(i);
            }
            if n_parts > 1 && ui.small_button("×").on_hover_text(tr("Delete the part", "Удалить часть")).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = dup {
        m.checkpoint();
        let mut p = m.src.model.parts[i].clone();
        p.name = m.src.model.free_name(&p.name);
        m.src.model.parts.push(p);
        m.changed();
    }
    if let Some(i) = remove {
        m.checkpoint();
        let p = m.src.model.parts.remove(i);
        m.src.part_vars.retain(|v| v.part != p.name);
        m.part = m.part.min(m.src.model.parts.len() - 1);
        m.sel = Default::default();
        m.changed();
    }
    ui.horizontal_wrapped(|ui| {
        ui.menu_button(tr("Add a shape as a part", "Добавить фигуру как часть"), |ui| {
            for p in ALL {
                if ui.button(primitive_label(&p)).clicked() {
                    m.checkpoint();
                    let mut mesh = p.build();
                    if p.round() {
                        for f in &mut mesh.faces {
                            f.smooth = true;
                        }
                    }
                    let mut part = openmapper_mesh::Part::new(&m.src.model.free_name(p.name()), mesh);
                    part.primitive = Some(p);
                    m.src.model.parts.push(part);
                    m.part = m.src.model.parts.len() - 1;
                    m.changed();
                    ui.close();
                }
            }
        });
        if n_parts > 1 && ui.button(tr("Separate parts into objects", "Разделить части на объекты")).on_hover_text(tr("Each part becomes an object of its own, standing where it stood", "Каждая часть становится отдельным объектом на своём месте")).clicked() {
            actions.push(Action::Mesh(MeshAction::SeparateParts));
        }
    });
    // the shape's own settings, while it is not edited
    let pi = m.part.min(m.src.model.parts.len().saturating_sub(1));
    if let Some(mut p) = m.src.model.parts.get(pi).and_then(|p| p.primitive) {
        ui.label(RichText::new(format!("{}: {}", tr("Shape", "Фигура"), primitive_label(&p))).small().color(DIM));
        if let Some(r) = primitive_ui(ui, &mut p) {
            touched(m, &r);
            if r.changed() {
                let round = p.round();
                let part = &mut m.src.model.parts[pi];
                let materials: Vec<u32> = part.mesh.faces.first().map(|f| vec![f.material]).unwrap_or_default();
                part.mesh = p.build();
                for f in &mut part.mesh.faces {
                    f.smooth = round;
                    f.material = materials.first().copied().unwrap_or(0);
                }
                part.primitive = Some(p);
            }
        }
    }

    // modifiers
    heading(ui, tr("Modifiers", "Модификаторы"));
    let names: Vec<String> = m.src.model.parts.iter().map(|p| p.name.clone()).collect();
    let mut action: Option<(usize, u8)> = None;
    let n_mods = m.src.model.parts.get(pi).map(|p| p.modifiers.len()).unwrap_or(0);
    for k in 0..n_mods {
        let mut st = m.src.model.parts[pi].modifiers[k].clone();
        let mut resp = None;
        egui::Frame::new().fill(Color32::from_rgb(40, 42, 47)).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let r = ui.checkbox(&mut st.on, RichText::new(modifier_label(&st.modifier)).strong());
                resp = Some(r);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text(tr("Remove", "Удалить")).clicked() {
                        action = Some((k, 0));
                    }
                    if ui.small_button(tr("Apply", "Применить")).on_hover_text(tr("Make it part of the mesh", "Сделать частью меша")).clicked() {
                        action = Some((k, 1));
                    }
                    if k + 1 < n_mods && ui.small_button(crate::icons::Icon::Down.text(14.0)).clicked() {
                        action = Some((k, 2));
                    }
                    if k > 0 && ui.small_button(crate::icons::Icon::Up.text(14.0)).clicked() {
                        action = Some((k, 3));
                    }
                });
            });
            if let Some(r) = modifier_ui(ui, k, &mut st.modifier, &names) {
                resp = Some(match resp.take() {
                    Some(a) => a.union(r),
                    None => r,
                });
            }
        });
        if let Some(r) = resp {
            touched(m, &r);
            if r.changed() {
                m.src.model.parts[pi].modifiers[k] = st;
            }
        }
    }
    if let Some((k, what)) = action {
        m.checkpoint();
        let part = &mut m.src.model.parts[pi];
        match what {
            0 => {
                part.modifiers.remove(k);
            }
            1 => {
                let st = part.modifiers.remove(k);
                let operand_parts = names.clone();
                let snapshot = m.src.model.clone();
                let operand = |name: &str| snapshot.parts.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| p.mesh.clone());
                let ctx = openmapper_mesh::modifiers::Context { operand: &operand, ground: None };
                let _ = operand_parts;
                let part = &mut m.src.model.parts[pi];
                // (the ones above it are applied with it: what it worked on)
                let mut mesh = part.mesh.clone();
                let above: Vec<Stacked> = part.modifiers.drain(..k).collect();
                for s in above.iter().filter(|s| s.on) {
                    mesh = openmapper_mesh::modifiers::apply(&mesh, &s.modifier, &ctx);
                }
                part.mesh = openmapper_mesh::modifiers::apply(&mesh, &st.modifier, &ctx);
                part.primitive = None;
            }
            2 => part.modifiers.swap(k, k + 1),
            _ => part.modifiers.swap(k, k - 1),
        }
        m.changed();
    }
    ui.menu_button(tr("Add a modifier", "Добавить модификатор"), |ui| {
        for md in Modifier::all() {
            if ui.button(modifier_label(&md)).clicked() {
                m.checkpoint();
                let md = match md {
                    Modifier::Boolean { op, .. } => Modifier::Boolean { op, operand: names.iter().find(|n| **n != names[pi]).cloned().unwrap_or_default() },
                    other => other,
                };
                m.src.model.parts[pi].modifiers.push(Stacked { on: true, modifier: md });
                m.changed();
                ui.close();
            }
        }
    });

    // materials
    heading(ui, tr("Materials", "Материалы"));
    let editing = m.editing && !m.sel.faces.is_empty();
    let n_mats = m.src.model.materials.len();
    let mut remove_mat = None;
    for i in 0..n_mats {
        let mut mat = m.src.model.materials[i].clone();
        let mut resp: Option<egui::Response> = None;
        let mut join = |r: egui::Response| resp = Some(match resp.take() {
            Some(a) => a.union(r),
            None => r,
        });
        egui::CollapsingHeader::new(format!("{}  {}", i + 1, mat.name)).id_salt(("mat", i)).default_open(n_mats <= 2).show(ui, |ui| {
            ui.horizontal(|ui| {
                join(ui.color_edit_button_rgba_unmultiplied(&mut mat.color).on_hover_text(tr("Colour (multiplies the texture)", "Цвет (умножается на текстуру)")));
                join(ui.add(egui::TextEdit::singleline(&mut mat.name).desired_width(120.0)));
                if editing && ui.small_button(tr("Assign to selected faces", "Назначить выделенным граням")).clicked() {
                    m.apply(|mesh, s, _| {
                        edit::set_material(mesh, s, i as u32);
                        None
                    });
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Texture", "Текстура")).color(DIM));
                ui.label(mat.texture.as_deref().unwrap_or("—"));
                if ui.small_button(tr("Choose…", "Выбрать…")).clicked() {
                    actions.push(Action::Mesh(MeshAction::PickTexture(i, TexSlot::Color)));
                }
                if mat.texture.is_some() {
                    if ui.small_button("×").on_hover_text(tr("No texture", "Без текстуры")).clicked() {
                        mat.texture = None;
                        let mut r = ui.label("");
                        r.mark_changed();
                        join(r);
                    }
                    if ui.small_button(tr("Save as…", "Сохранить как…")).on_hover_text(tr("PNG, JPEG, BMP, TGA, TIFF, WebP or DDS", "PNG, JPEG, BMP, TGA, TIFF, WebP или DDS")).clicked() {
                        actions.push(Action::Mesh(MeshAction::ExportTexture(i)));
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Transparency", "Прозрачность")).color(DIM));
                for (v, label) in [(0u8, tr("Opaque", "Нет")), (1, tr("Cut out", "Вырез")), (2, tr("Blended", "Смешивание"))] {
                    join(ui.selectable_value(&mut mat.alpha, v, label));
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Glows", "Свечение")).color(DIM));
                join(ui.color_edit_button_rgb(&mut mat.emissive).on_hover_text(tr("Light of its own: black for none", "Собственный свет: чёрный — без свечения")));
                ui.label(RichText::new(tr("Shine", "Блеск")).color(DIM));
                join(ui.add(egui::DragValue::new(&mut mat.power).speed(0.5).range(1.0..=200.0)));
            });
            ui.horizontal_wrapped(|ui| {
                for (slot, label, now) in [(TexSlot::Normal, tr("Normal map", "Нормали"), mat.normal_map.clone()), (TexSlot::Orm, tr("ORM map", "ORM"), mat.orm_map.clone()), (TexSlot::Night, tr("Night texture", "Ночная"), mat.night_map.clone()), (TexSlot::Light, tr("Light map", "Карта света"), mat.light_map.clone())] {
                    let text = if now.is_some() { format!("{} {label}", crate::icons::Icon::CheckCircle.ch()) } else { label.to_string() };
                    if ui.small_button(text).on_hover_text(now.unwrap_or_else(|| tr("Choose a picture", "Выбрать картинку").to_string())).clicked() {
                        actions.push(Action::Mesh(MeshAction::PickTexture(i, slot)));
                    }
                }
            });
            if n_mats > 1 && ui.small_button(tr("Remove the material", "Удалить материал")).clicked() {
                remove_mat = Some(i);
            }
        });
        if let Some(r) = resp {
            touched(m, &r);
            if r.changed() {
                m.src.model.materials[i] = mat;
            }
        }
    }
    if let Some(i) = remove_mat {
        m.checkpoint();
        m.src.model.materials.remove(i);
        for p in &mut m.src.model.parts {
            for f in &mut p.mesh.faces {
                if f.material as usize >= i && f.material > 0 {
                    f.material -= 1;
                }
            }
        }
        m.changed();
    }
    ui.horizontal(|ui| {
        if ui.button(tr("Add a material", "Добавить материал")).clicked() {
            m.checkpoint();
            let n = m.src.model.materials.len();
            m.src.model.materials.push(Material::colored(&format!("Material{}", n + 1), [0.8, 0.8, 0.8, 1.0]));
            m.changed();
        }
        if ui.button(tr("Import materials…", "Импорт материалов…")).on_hover_text(tr("From an .mtl, a glTF / GLB / FBX, or the pictures of one PBR material (base colour, normal, roughness, metal, AO...)", "Из .mtl, glTF / GLB / FBX или набора картинок PBR-материала (цвет, нормали, шероховатость, металл, AO...)")).clicked() {
            actions.push(Action::Mesh(MeshAction::ImportMaterial));
        }
    });

    // lamps
    heading(ui, tr("Lamps", "Источники света"));
    let mut remove_light = None;
    let n_lights = m.src.lights.len();
    for i in 0..n_lights {
        let mut l = m.src.lights[i].clone();
        let mut resp: Option<egui::Response> = None;
        let mut join = |r: egui::Response| resp = Some(match resp.take() {
            Some(a) => a.union(r),
            None => r,
        });
        egui::CollapsingHeader::new(format!("{} {}", match l.kind { LightKind::Glow => tr("Glow", "Свечение"), LightKind::Ground => tr("Light on the ground", "Свет на земле") }, i + 1)).id_salt(("light", i)).show(ui, |ui| {
            join(vec3(ui, tr("Where", "Где"), &mut l.pos, 0.01));
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Colour", "Цвет")).color(DIM));
                join(ui.color_edit_button_srgb(&mut l.color));
                if l.kind == LightKind::Glow {
                    ui.label(RichText::new(tr("Size", "Размер")).color(DIM));
                    join(ui.add(egui::DragValue::new(&mut l.size).speed(0.01).range(0.01..=20.0).suffix(" m")));
                } else {
                    ui.label(RichText::new(tr("Reach", "Радиус")).color(DIM));
                    join(ui.add(egui::DragValue::new(&mut l.range).speed(0.1).range(0.5..=100.0).suffix(" m")));
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("On", "Горит")).color(DIM));
                let night = l.when == When::Night;
                let always = l.when == When::Always;
                if ui.selectable_label(always, tr("Always", "Всегда")).clicked() {
                    l.when = When::Always;
                    let mut r = ui.label("");
                    r.mark_changed();
                    join(r);
                }
                if ui.selectable_label(night, tr("At night", "Ночью")).clicked() {
                    l.when = When::Night;
                    let mut r = ui.label("");
                    r.mark_changed();
                    join(r);
                }
                let mut var = match &l.when {
                    When::Var(v) => v.clone(),
                    _ => String::new(),
                };
                let r = ui.add(egui::TextEdit::singleline(&mut var).hint_text(tr("variable", "переменная")).desired_width(80.0));
                if r.changed() {
                    l.when = if var.trim().is_empty() { When::Always } else { When::Var(var) };
                }
                join(r);
            });
            if l.kind == LightKind::Glow {
                ui.horizontal(|ui| {
                    let mut cone = l.cone.is_some();
                    let r = ui.checkbox(&mut cone, tr("Seen from one side", "Видна с одной стороны"));
                    if r.changed() {
                        l.cone = if cone { Some((30.0, 70.0)) } else { None };
                    }
                    join(r);
                    join(ui.checkbox(&mut l.star, tr("Star", "Лучи")));
                });
                if l.cone.is_some() {
                    join(vec3(ui, tr("Facing", "Направление"), &mut l.dir, 0.01));
                }
            }
            if ui.small_button(tr("Remove the lamp", "Удалить источник")).clicked() {
                remove_light = Some(i);
            }
        });
        if let Some(r) = resp {
            touched(m, &r);
            if r.changed() {
                m.src.lights[i] = l;
            }
        }
    }
    if let Some(i) = remove_light {
        m.checkpoint();
        m.src.lights.remove(i);
        m.changed();
    }
    ui.horizontal_wrapped(|ui| {
        let top = m.src.model.parts.iter().filter_map(|p| p.mesh.bounds()).fold(0.0f32, |a, b| a.max(b.1.z));
        if ui.button(tr("Add a glow", "Добавить свечение")).on_hover_text(tr("A lamp's bulb, a lit sign: a glow seen at night", "Лампа, светящаяся вывеска: свечение, видное ночью")).clicked() {
            m.checkpoint();
            m.src.lights.push(Light { pos: glam::Vec3::new(0.0, 0.0, top), ..Default::default() });
            m.changed();
        }
        if ui.button(tr("Add light on the ground", "Добавить свет на землю")).on_hover_text(tr("The pool of light a street lamp throws at night", "Пятно света от фонаря ночью")).clicked() {
            m.checkpoint();
            m.src.lights.push(Light { kind: LightKind::Ground, pos: glam::Vec3::new(0.0, 0.0, top), ..Default::default() });
            m.changed();
        }
    });
    if let Some(sig) = &m.src.signal {
        ui.label(RichText::new(format!("{}: {}", tr("Signal head, lamps", "Светофор, секции"), sig.lamps.iter().map(|l| l.label()).collect::<Vec<_>>().join(", "))).small().color(DIM));
        ui.label(RichText::new(tr("Put it within 40 m of a crossing with a light program: it takes its light from it (or link it in its properties)", "Поставьте его ближе 40 м к перекрёстку со светофорной программой: он возьмёт свет от него (или привяжите в свойствах)")).small().color(DIM));
    }

    // baking
    heading(ui, tr("Bake", "Запекание"));
    ui.label(RichText::new(tr("Its textures and colours painted into one picture on a new UV map (one material: fewer draws in the game), with its own shadows and its lamps' night light if chosen", "Текстуры и цвета объекта рисуются в одну картинку на новой UV-развёртке (один материал: меньше вызовов отрисовки в игре), по желанию с собственными тенями и ночным светом ламп")).small().color(DIM));
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Size", "Размер")).color(DIM));
        egui::ComboBox::from_id_salt("bake size").selected_text(format!("{0} × {0}", bake.size)).width(110.0).show_ui(ui, |ui| {
            for s in [256u32, 512, 1024, 2048, 4096] {
                ui.selectable_value(&mut bake.size, s, format!("{s} × {s}"));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut bake.ao, tr("Ambient occlusion", "Затенение (AO)"));
        if bake.ao {
            ui.add(egui::DragValue::new(&mut bake.ao_distance).speed(0.05).range(0.05..=50.0).suffix(" m")).on_hover_text(tr("How far its shadows reach", "Как далеко тянутся тени"));
        }
    });
    ui.checkbox(&mut bake.light, tr("Lamps' light at night (light map)", "Ночной свет ламп (карта света)"));
    if ui.button(tr("Bake", "Запечь")).clicked() {
        actions.push(Action::Mesh(MeshAction::Bake(*bake)));
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(m.can_undo(), egui::Button::new(tr("Undo", "Отменить"))).on_hover_text(tr("The object's last change (Ctrl+Z in the edit mode)", "Последнее изменение объекта (Ctrl+Z в режиме редактирования)")).clicked() {
            m.undo();
        }
        if ui.add_enabled(m.can_redo(), egui::Button::new(tr("Redo", "Повторить"))).clicked() {
            m.redo();
        }
    });
}

/// A shape's settings; the response of what changed.
fn primitive_ui(ui: &mut egui::Ui, p: &mut Primitive) -> Option<egui::Response> {
    let mut rs: Vec<egui::Response> = Vec::new();
    let m = " m";
    match p {
        Primitive::Cube { size } | Primitive::Wedge { size } | Primitive::CornerWedge { size } => rs.push(vec3(ui, tr("Size", "Размер"), size, 0.01)),
        Primitive::Plane { size } => {
            rs.push(num(ui, tr("Width", "Ширина"), &mut size.x, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Depth", "Глубина"), &mut size.y, 0.01, 0.01..=1000.0, m));
        }
        Primitive::Grid { size, x_div, y_div } => {
            rs.push(num(ui, tr("Width", "Ширина"), &mut size.x, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Depth", "Глубина"), &mut size.y, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Divisions X", "Деления X"), x_div, 1..=256));
            rs.push(count(ui, tr("Divisions Y", "Деления Y"), y_div, 1..=256));
        }
        Primitive::Circle { radius, segments } => {
            rs.push(num(ui, tr("Radius", "Радиус"), radius, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 3..=256));
        }
        Primitive::Triangle { width, height } => {
            rs.push(num(ui, tr("Width", "Ширина"), width, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
        }
        Primitive::Prism { width, depth, height } | Primitive::Pyramid { width, depth, height } => {
            rs.push(num(ui, tr("Width", "Ширина"), width, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Depth", "Глубина"), depth, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
        }
        Primitive::UvSphere { radius, segments, rings } => {
            rs.push(num(ui, tr("Radius", "Радиус"), radius, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 3..=256));
            rs.push(count(ui, tr("Rings", "Кольца"), rings, 2..=128));
        }
        Primitive::IcoSphere { radius, subdivisions } => {
            rs.push(num(ui, tr("Radius", "Радиус"), radius, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Subdivisions", "Подразделения"), subdivisions, 0..=6));
        }
        Primitive::Cylinder { radius, height, segments } => {
            rs.push(num(ui, tr("Radius", "Радиус"), radius, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 3..=256));
        }
        Primitive::Cone { radius_bottom, radius_top, height, segments } => {
            rs.push(num(ui, tr("Bottom radius", "Нижний радиус"), radius_bottom, 0.01, 0.0..=1000.0, m));
            rs.push(num(ui, tr("Top radius", "Верхний радиус"), radius_top, 0.01, 0.0..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 3..=256));
        }
        Primitive::Torus { major, minor, major_segments, minor_segments } => {
            rs.push(num(ui, tr("Radius", "Радиус"), major, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Tube radius", "Радиус трубки"), minor, 0.01, 0.005..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), major_segments, 3..=256));
            rs.push(count(ui, tr("Tube segments", "Сегменты трубки"), minor_segments, 3..=128));
        }
        Primitive::Tube { outer, inner, height, segments } => {
            rs.push(num(ui, tr("Outer radius", "Внешний радиус"), outer, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Inner radius", "Внутренний радиус"), inner, 0.01, 0.0..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 3..=256));
        }
        Primitive::Capsule { radius, height, segments, rings } => {
            rs.push(num(ui, tr("Radius", "Радиус"), radius, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 3..=256));
            rs.push(count(ui, tr("Rings", "Кольца"), rings, 1..=64));
        }
        Primitive::Stairs { width, depth, height, steps } => {
            rs.push(num(ui, tr("Width", "Ширина"), width, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Depth", "Глубина"), depth, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Steps", "Ступени"), steps, 1..=200));
        }
        Primitive::Arch { width, height, depth, thickness, segments } => {
            rs.push(num(ui, tr("Opening", "Проём"), width, 0.01, 0.05..=1000.0, m));
            rs.push(num(ui, tr("Height", "Высота"), height, 0.01, 0.05..=1000.0, m));
            rs.push(num(ui, tr("Depth", "Глубина"), depth, 0.01, 0.01..=1000.0, m));
            rs.push(num(ui, tr("Wall", "Стенка"), thickness, 0.01, 0.01..=1000.0, m));
            rs.push(count(ui, tr("Segments", "Сегменты"), segments, 2..=128));
        }
    }
    rs.into_iter().reduce(|a, b| a.union(b))
}

/// For an object that is not openMapper's: what can be done to it as a mesh.
pub fn plain_object_panel(ui: &mut egui::Ui, parts: usize, actions: &mut Vec<Action>) {
    heading(ui, tr("Mesh", "Меш"));
    ui.horizontal_wrapped(|ui| {
        if ui.button(RichText::new(tr("Edit the mesh (Tab)", "Редактировать меш (Tab)")).strong()).on_hover_text(tr("A copy of this object's type of its own is made for it, with its meshes, materials and lamps to edit; the original stays as it is", "Для объекта делается собственная копия типа с мешами, материалами и лампами для редактирования; оригинал не меняется")).clicked() {
            actions.push(Action::Mesh(MeshAction::ToggleEdit));
        }
        if parts > 1 && ui.button(tr("Separate parts into objects", "Разделить части на объекты")).clicked() {
            actions.push(Action::Mesh(MeshAction::SeparateParts));
        }
        if ui.button(tr("Export…", "Экспорт…")).clicked() {
            actions.push(Action::Mesh(MeshAction::Export));
        }
    });
}

/// For a selected road or spline: its shape as a mesh.
pub fn spline_panel(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    heading(ui, tr("As a mesh", "Как меш"));
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Copy as an object", "Копия как объект")).on_hover_text(tr("An openMapper object of its shape beside it, to edit as a mesh (vertices, edges, faces)", "Объект openMapper той же формы, чтобы править его как меш (вершины, рёбра, грани)")).clicked() {
            actions.push(Action::Mesh(MeshAction::SplinesToObject(false)));
        }
        if ui.button(tr("Replace by an object", "Заменить объектом")).on_hover_text(tr("The same, the spline taken away (its lanes go with it)", "То же, а сплайн удаляется (вместе с полосами)")).clicked() {
            actions.push(Action::Mesh(MeshAction::SplinesToObject(true)));
        }
        if ui.button(tr("Polygon from it", "Многоугольник по нему")).on_hover_text(tr("Fill the outline the selected roads and splines make when they join end to end (kerbs round an island, for instance)", "Заполнить контур, который выделенные дороги и сплайны образуют, смыкаясь концами (например, бордюры вокруг островка)")).clicked() {
            actions.push(Action::Mesh(MeshAction::PolygonFromSplines));
        }
        if ui.button(tr("Export…", "Экспорт…")).clicked() {
            actions.push(Action::Mesh(MeshAction::Export));
        }
    });
}

/// For several selected objects: join them, booleans, export.
pub fn many_panel(ui: &mut egui::Ui, keep: &mut bool, objects: usize, splines: usize, actions: &mut Vec<Action>) {
    if splines > 0 {
        heading(ui, tr("Splines as a mesh", "Сплайны как меш"));
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Copy as one object", "Копия как один объект")).clicked() {
                actions.push(Action::Mesh(MeshAction::SplinesToObject(false)));
            }
            if ui.button(tr("Replace by one object", "Заменить одним объектом")).clicked() {
                actions.push(Action::Mesh(MeshAction::SplinesToObject(true)));
            }
            if ui.button(tr("Polygon from them", "Многоугольник по ним")).on_hover_text(tr("Fill the outline the selected roads and splines make when they join end to end (kerbs round an island, for instance)", "Заполнить контур, который выделенные дороги и сплайны образуют, смыкаясь концами (например, бордюры вокруг островка)")).clicked() {
                actions.push(Action::Mesh(MeshAction::PolygonFromSplines));
            }
        });
    }
    if objects < 2 {
        if ui.button(tr("Export the selection…", "Экспорт выделенного…")).clicked() {
            actions.push(Action::Mesh(MeshAction::Export));
        }
        return;
    }
    heading(ui, tr("As meshes", "Как меши"));
    if ui.button(tr("Join into one object (Ctrl+J)", "Объединить в один объект (Ctrl+J)")).on_hover_text(tr("All their parts in the one shown in the properties, the others gone", "Все их части — в объекте из свойств, остальные удаляются")).clicked() {
        actions.push(Action::Mesh(MeshAction::Join));
    }
    ui.label(RichText::new(tr("Solid modelling (CSG): the object shown in the properties with the others", "Твердотельное моделирование (CSG): объект из свойств с остальными")).small().color(DIM));
    ui.horizontal_wrapped(|ui| {
        for op in crate::meshops::Bool::ALL {
            if ui.button(op.label()).on_hover_text(op.hint()).clicked() {
                actions.push(Action::Mesh(MeshAction::Boolean(op, *keep)));
            }
        }
    });
    ui.checkbox(keep, tr("Keep the cutting objects", "Оставить режущие объекты"));
    if ui.button(tr("Export the selection…", "Экспорт выделенного…")).clicked() {
        actions.push(Action::Mesh(MeshAction::Export));
    }
}

/// The edit mode's tools (the left panel while it is on).
pub fn edit_panel(ui: &mut egui::Ui, ed: &mut Editor, uv_open: &mut bool, pics: &mut crate::ui_tex::TexCache, actions: &mut Vec<Action>) {
    let Some(m) = ed.mesh.as_mut().filter(|m| m.editing) else { return };
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0;
        for (mode, label, key) in [(SelectMode::Vertex, tr("Vertices", "Вершины"), "1"), (SelectMode::Edge, tr("Edges", "Рёбра"), "2"), (SelectMode::Face, tr("Faces", "Грани"), "3")] {
            if ui.add_sized([w, 24.0], egui::Button::new(label).selected(m.mode == mode)).on_hover_text(key).clicked() {
                m.set_mode(mode);
            }
        }
    });
    if m.src.model.parts.len() > 1 {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Part", "Часть")).color(DIM));
            let mut pi = m.part;
            egui::ComboBox::from_id_salt("edit part").selected_text(m.src.model.parts[pi].name.clone()).show_ui(ui, |ui| {
                for (i, p) in m.src.model.parts.iter().enumerate() {
                    ui.selectable_value(&mut pi, i, &p.name);
                }
            });
            if pi != m.part {
                m.part = pi;
                m.sel = Default::default();
            }
        });
    }
    let mesh = m.mesh();
    ui.label(RichText::new(format!("{} {} · {} {} · {} {}", mesh.positions.len(), tr("vertices", "вершин"), mesh.edges().len(), tr("edges", "рёбер"), mesh.faces.len(), tr("faces", "граней"))).small().color(DIM));
    ui.label(RichText::new(format!("{}: {} / {} / {}", tr("Selected", "Выделено"), m.sel.verts.len(), m.sel.edges.len(), m.sel.faces.len())).small().color(DIM));
    egui::CollapsingHeader::new(RichText::new(tr("Display", "Показ")).strong().color(Color32::WHITE)).id_salt("tab display").default_open(true).show(ui, |ui| {
        ui.checkbox(&mut m.xray, tr("X-ray (Alt+Z): see and pick through the mesh", "Рентген (Alt+Z): видеть и выбирать сквозь меш"));
        ui.checkbox(&mut m.tex_preview, tr("Show the textures", "Показывать текстуры")).on_hover_text(tr("The object's own pictures under the editing shading", "Картинки самого объекта под рабочей заливкой"));
        if m.tex_preview {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Shading opacity", "Непрозрачность заливки")).color(DIM));
                let mut pct = m.shade * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix(" %").integer()).changed() {
                    m.shade = pct / 100.0;
                }
            });
            ui.label(RichText::new(tr("0 %: the textures as they are; 100 %: the plain shading. The picture is renewed a moment after each change.", "0 %: текстуры как есть; 100 %: простая заливка. Картинка обновляется через мгновение после правки.")).small().color(DIM));
        }
    });

    let view = ed.gizmo_view();
    let Some(m) = ed.mesh.as_mut() else { return };
    egui::CollapsingHeader::new(RichText::new(tr("Select", "Выделение")).strong().color(Color32::WHITE)).id_salt("tab select").default_open(true).show(ui, |ui| {
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("All (A)", "Всё (A)")).clicked() {
            m.sel = edit::Selection::all(m.mesh());
        }
        if ui.button(tr("None (Alt+A)", "Ничего (Alt+A)")).clicked() {
            m.sel = Default::default();
        }
        if ui.button(tr("Invert (Ctrl+I)", "Инвертировать (Ctrl+I)")).clicked() {
            m.sel = edit::invert(m.mesh(), &m.sel, m.mode);
        }
        if ui.button(tr("Linked (Ctrl+L)", "Связанное (Ctrl+L)")).clicked() {
            m.sel = edit::select_linked(m.mesh(), &m.sel);
        }
        if ui.button(tr("More", "Больше")).clicked() {
            m.sel = edit::select_more(m.mesh(), &m.sel);
        }
        if ui.button(tr("Less", "Меньше")).clicked() {
            m.sel = edit::select_less(m.mesh(), &m.sel);
        }
    });
    ui.label(RichText::new(tr("Click, Shift + click to add, drag on nothing for a box, a second click or Alt + click for a loop", "Клик, Shift + клик — добавить, протяжка по пустому — рамка, второй клик или Alt + клик — петля")).small().color(DIM));
    });

    egui::CollapsingHeader::new(RichText::new(tr("Change", "Изменить")).strong().color(Color32::WHITE)).id_salt("tab change").default_open(true).show(ui, |ui| {
    let start = |m: &mut Session, k: Sizing| {
        if let Some(v) = &view {
            m.start(k, v);
        }
    };
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Extrude (E)", "Выдавить (E)")).clicked() {
            start(m, Sizing::Extrude);
        }
        if ui.button(tr("Inset (I)", "Вставка (I)")).clicked() {
            start(m, Sizing::Inset);
        }
        if ui.button(tr("Bevel (Ctrl+B)", "Фаска (Ctrl+B)")).clicked() {
            start(m, Sizing::Bevel);
        }
        if ui.button(tr("Loop cut (Ctrl+R)", "Разрез петлёй (Ctrl+R)")).on_hover_text(tr("Point at an edge and press Ctrl+R", "Наведите на ребро и нажмите Ctrl+R")).clicked() {
            if let Some(e) = m.sel.edges.iter().next().copied() {
                start(m, Sizing::LoopCut(e));
            }
        }
        if ui.button(tr("Subdivide", "Подразделить")).clicked() {
            m.apply(|mesh, s, _| Some(edit::subdivide(mesh, s, 1)));
        }
        if ui.button(tr("Fill (F)", "Заполнить (F)")).clicked() {
            m.apply(|mesh, s, _| Some(edit::fill(mesh, s)));
        }
        if ui.button(tr("Bridge loops", "Соединить петли")).clicked() {
            m.apply(|mesh, s, _| Some(edit::bridge_loops(mesh, s)));
        }
        if ui.button(tr("Connect (J)", "Соединить вершины (J)")).clicked() {
            m.apply(|mesh, s, _| Some(edit::connect(mesh, s)));
        }
        if ui.button(tr("Poke", "Веер из центра")).clicked() {
            m.apply(|mesh, s, _| Some(edit::poke(mesh, s)));
        }
        if ui.button(tr("Duplicate (Shift+D)", "Дублировать (Shift+D)")).clicked() {
            m.apply(|mesh, s, _| Some(edit::duplicate(mesh, s)));
        }
        if ui.button(tr("Split (Y)", "Отделить (Y)")).clicked() {
            m.apply(|mesh, s, _| Some(edit::split(mesh, s)));
        }
        if ui.button(tr("Separate into an object (P)", "В отдельный объект (P)")).clicked() {
            actions.push(Action::Mesh(MeshAction::SeparateSelected));
        }
        if ui.button(tr("Spin 360°", "Вращение 360°")).on_hover_text(tr("The selection turned round the object's up axis through its middle, in 12 steps", "Выделенное вращается вокруг вертикальной оси объекта, 12 шагов")).clicked() {
            m.apply(|mesh, s, _| Some(edit::spin(mesh, s, glam::Vec3::Z, glam::Vec3::ZERO, 360.0, 12)));
        }
        if ui.button(tr("Smooth vertices", "Сгладить вершины")).clicked() {
            m.apply(|mesh, s, _| {
                edit::smooth_vertices(mesh, s, 0.5, 1);
                None
            });
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.menu_button(tr("Merge (M)", "Слить (M)"), |ui| {
            for (at, label) in [(MergeAt::Center, tr("At the middle", "В центре")), (MergeAt::First, tr("At the first", "К первой")), (MergeAt::Last, tr("At the last", "К последней")), (MergeAt::Collapse, tr("Collapse each piece", "Стянуть каждый кусок"))] {
                if ui.button(label).clicked() {
                    m.apply(|mesh, s, _| Some(edit::merge(mesh, s, at)));
                    ui.close();
                }
            }
            if ui.button(tr("By distance (1 mm)", "По расстоянию (1 мм)")).clicked() {
                m.apply(|mesh, s, _| {
                    edit::merge_by_distance(mesh, s, 0.001);
                    None
                });
                ui.close();
            }
        });
        ui.menu_button(tr("Delete (X)", "Удалить (X)"), |ui| {
            for (what, label) in [(Delete::Vertices, tr("Vertices", "Вершины")), (Delete::Edges, tr("Edges", "Рёбра")), (Delete::Faces, tr("Faces", "Грани")), (Delete::OnlyFaces, tr("Only faces", "Только грани"))] {
                if ui.button(label).clicked() {
                    m.apply(|mesh, s, _| Some(edit::delete(mesh, s, what)));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(tr("Dissolve vertices", "Растворить вершины")).clicked() {
                m.apply(|mesh, s, _| Some(edit::dissolve_vertices(mesh, s)));
                ui.close();
            }
            if ui.button(tr("Dissolve edges", "Растворить рёбра")).clicked() {
                m.apply(|mesh, s, _| Some(edit::dissolve_edges(mesh, s)));
                ui.close();
            }
            if ui.button(tr("Dissolve faces", "Растворить грани")).clicked() {
                m.apply(|mesh, s, _| Some(edit::dissolve_faces(mesh, s)));
                ui.close();
            }
        });
        ui.menu_button(tr("Mirror", "Отразить"), |ui| {
            for a in [Axis::X, Axis::Y, Axis::Z] {
                if ui.button(format!("{a:?}")).clicked() {
                    let c = edit::selection_center(m.mesh(), &m.sel).unwrap_or_default();
                    m.apply(|mesh, s, _| {
                        edit::mirror(mesh, s, a, c);
                        None
                    });
                    ui.close();
                }
            }
        });
        ui.menu_button(tr("Faces", "Грани"), |ui| {
            if ui.button(tr("Triangulate (Ctrl+T)", "Триангулировать (Ctrl+T)")).clicked() {
                m.apply(|mesh, s, _| Some(edit::triangulate(mesh, s)));
                ui.close();
            }
            if ui.button(tr("Triangles to quads (Alt+J)", "Треугольники в четырёхугольники (Alt+J)")).clicked() {
                m.apply(|mesh, s, _| Some(edit::tris_to_quads(mesh, s, 40.0)));
                ui.close();
            }
            if ui.button(tr("Shade smooth", "Сглаженное затенение")).clicked() {
                m.apply(|mesh, s, _| {
                    edit::set_smooth(mesh, s, true);
                    None
                });
                ui.close();
            }
            if ui.button(tr("Shade flat", "Плоское затенение")).clicked() {
                m.apply(|mesh, s, _| {
                    edit::set_smooth(mesh, s, false);
                    None
                });
                ui.close();
            }
            if ui.button(tr("Flip normals (Alt+N)", "Развернуть нормали (Alt+N)")).clicked() {
                m.apply(|mesh, s, _| {
                    edit::flip_normals(mesh, s);
                    None
                });
                ui.close();
            }
            if ui.button(tr("Normals outward (Shift+N)", "Нормали наружу (Shift+N)")).clicked() {
                m.apply(|mesh, s, _| {
                    edit::recalculate_outside(mesh, s);
                    None
                });
                ui.close();
            }
        });
        ui.menu_button(tr("Edges", "Рёбра"), |ui| {
            for (label, seam, on) in [(tr("Mark seam", "Пометить шов"), true, true), (tr("Clear seam", "Снять шов"), true, false), (tr("Mark sharp", "Пометить острым"), false, true), (tr("Clear sharp", "Снять острое"), false, false)] {
                if ui.button(label).clicked() {
                    m.apply(|mesh, s, _| {
                        if seam {
                            edit::mark_seam(mesh, s, on);
                        } else {
                            edit::mark_sharp(mesh, s, on);
                        }
                        None
                    });
                    ui.close();
                }
            }
        });
    });
    });

    // the last sized operation, to change after
    if let Some(l) = m.last.clone() {
        heading(ui, tr("Adjust the last operation", "Настроить последнюю операцию"));
        let (mut v, mut c) = (l.value, l.count);
        let label = match l.kind {
            Sizing::Extrude => tr("Distance", "Расстояние"),
            Sizing::Inset => tr("Thickness", "Толщина"),
            Sizing::Bevel => tr("Width", "Ширина"),
            Sizing::LoopCut(_) => tr("Slide", "Сдвиг"),
        };
        let a = ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(DIM));
            ui.add(egui::DragValue::new(&mut v).speed(0.005).max_decimals(3))
        });
        let b = match l.kind {
            Sizing::Bevel => Some(count(ui, tr("Segments", "Сегменты"), &mut c, 1..=64)),
            Sizing::LoopCut(_) => Some(count(ui, tr("Cuts", "Разрезы"), &mut c, 1..=32)),
            _ => None,
        };
        if a.inner.changed() || b.map(|r| r.changed()).unwrap_or(false) {
            m.redo_last(v, c);
        }
    }

    egui::CollapsingHeader::new(RichText::new(tr("UV map", "UV-развёртка")).strong().color(Color32::WHITE)).id_salt("tab uv").default_open(false).show(ui, |ui| {
    ui.horizontal_wrapped(|ui| {
        use openmapper_mesh::uv::Unwrap;
        let faces: Vec<usize> = m.sel.faces.iter().copied().collect();
        let unwrap = |m: &mut Session, how: Unwrap| {
            let f = faces.clone();
            m.apply(move |mesh, _, _| {
                openmapper_mesh::uv::unwrap(mesh, &f, how, 0.01);
                None
            });
        };
        ui.menu_button(tr("Unwrap", "Развернуть"), |ui| {
            let dir = view.as_ref().map(|v| v.cam.axes().0.as_vec3()).unwrap_or(glam::Vec3::NEG_Z);
            for (how, label) in [
                (Unwrap::Conformal, tr("By seams (conformal)", "По швам (конформно)")),
                (Unwrap::Smart { angle_deg: 66.0 }, tr("Smart project", "Умная проекция")),
                (Unwrap::Cube { size: 1.0 }, tr("Cube projection (metres)", "Кубическая проекция (в метрах)")),
                (Unwrap::Cylinder { axis: Axis::Z, size: 1.0 }, tr("Cylinder projection", "Цилиндрическая проекция")),
                (Unwrap::Sphere, tr("Sphere projection", "Сферическая проекция")),
                (Unwrap::Planar { direction: dir, size: 1.0 }, tr("Project from the view", "Проекция из вида")),
                (Unwrap::LightmapPack, tr("Lightmap pack", "Упаковка для карты света")),
            ] {
                if ui.button(label).clicked() {
                    unwrap(m, how);
                    ui.close();
                }
            }
        });
        if ui.button(if *uv_open { tr("Close the UV editor", "Закрыть UV-редактор") } else { tr("UV editor", "UV-редактор") }).clicked() {
            *uv_open = !*uv_open;
        }
    });
    ui.label(RichText::new(tr("Faces of the selection (all, with none selected); mark seams on edges first for an unwrap by seams", "Грани выделения (все, если ничего не выделено); для развёртки по швам сначала пометьте швы на рёбрах")).small().color(DIM));
    });

    if !m.src.model.materials.is_empty() {
        egui::CollapsingHeader::new(RichText::new(tr("Materials", "Материалы")).strong().color(Color32::WHITE)).id_salt("tab materials").default_open(true).show(ui, |ui| {
            // how many of the selected faces each material has
            let mut used = vec![0usize; m.src.model.materials.len()];
            for &f in &m.sel.faces {
                if let Some(face) = m.mesh().faces.get(f) {
                    if let Some(u) = used.get_mut(face.material as usize) {
                        *u += 1;
                    }
                }
            }
            let any = !m.sel.faces.is_empty();
            let mats: Vec<(String, Option<String>, [f32; 4])> = m.src.model.materials.iter().map(|x| (x.name.clone(), x.texture.clone(), x.color)).collect();
            for (i, (name, tex, color)) in mats.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
                    match tex.as_ref().and_then(|t| pics.get(ui.ctx(), &m.tex_dir.join(t))) {
                        Some(id) => {
                            egui::Image::new((id, rect.size())).paint_at(ui, rect);
                        }
                        None => {
                            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
                            ui.painter().rect_filled(rect, egui::CornerRadius::same(3), Color32::from_rgb(c(color[0]), c(color[1]), c(color[2])));
                        }
                    }
                    ui.painter().rect_stroke(rect, egui::CornerRadius::same(3), egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
                    let label = if used[i] > 0 { format!("{name}  ({})", used[i]) } else { name.clone() };
                    let r = ui.add_enabled(any, egui::Button::new(label).selected(used[i] > 0 && used[i] == m.sel.faces.len()));
                    if r.on_hover_text(tr("Give this material to the selected faces", "Назначить этот материал выделенным граням")).clicked() {
                        m.apply(|mesh, s, _| {
                            edit::set_material(mesh, s, i as u32);
                            None
                        });
                    }
                });
            }
            if !any {
                ui.label(RichText::new(tr("Select faces to give them a material.", "Выделите грани, чтобы назначить им материал.")).small().color(DIM));
            }
        });
    }
    ui.add_space(10.0);
    if ui.button(RichText::new(tr("Leave the edit mode (Tab)", "Выйти из редактирования (Tab)")).strong()).clicked() {
        actions.push(Action::Mesh(MeshAction::ToggleEdit));
    }
}

/// The makers of objects with lamps, in the Objects tool's Shapes.
pub fn lamp_makers(ui: &mut egui::Ui, lamps: &mut Vec<Lamp>, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.label(RichText::new(tr("With lamps", "Со светом")).small().color(DIM));
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Street lamp", "Фонарь")).on_hover_text(tr("A pole with a lamp that glows and lights the ground at night", "Столб с лампой, светящейся и освещающей землю ночью")).clicked() {
            actions.push(Action::Mesh(MeshAction::NewLamp));
        }
        if ui.button(tr("Signal head", "Светофор")).on_hover_text(tr("A traffic light of the lamps below that a crossing's light program drives", "Светофор из секций ниже, управляемый программой перекрёстка")).clicked() {
            actions.push(Action::Mesh(MeshAction::NewSignal(lamps.clone())));
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Lamps:", "Секции:")).small().color(DIM));
        for l in [Lamp::Red, Lamp::Yellow, Lamp::Green] {
            let mut on = lamps.contains(&l);
            if ui.checkbox(&mut on, l.label()).changed() {
                if on {
                    lamps.push(l);
                    lamps.sort_by_key(|x| *x as u8);
                } else {
                    lamps.retain(|x| *x != l);
                }
            }
        }
    });
}

/// Pictures and patterns, lights and sounds to put on the map, in the Objects tool's Shapes.
pub fn extra_makers(ui: &mut egui::Ui, s: &mut crate::makers::Settings, actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Picture, light, sound", "Картинка, свет, звук")).small().color(DIM));
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Picture / pattern…", "Картинка / паттерн…")).on_hover_text(tr("Choose a picture or a texture: it becomes an object you place, move, turn and scale like any other. Several repeats make a pattern", "Выберите картинку или текстуру: получится объект, который ставится, двигается, поворачивается и масштабируется как любой другой. Несколько повторов дают паттерн")).clicked() {
            actions.push(Action::Mesh(MeshAction::NewPicture(s.clone())));
        }
        if ui.button(tr("Light source", "Источник света")).on_hover_text(tr("A lamp that glows and lights the ground round it", "Лампа, которая светится и освещает землю вокруг")).clicked() {
            actions.push(Action::Mesh(MeshAction::NewLight(s.clone())));
        }
        if ui.button(tr("Sound source…", "Источник звука…")).on_hover_text(tr("Choose a sound file (.wav): it plays where you put the marker, as far as the radius carries; the marker shows in the editor only", "Выберите звуковой файл (.wav): он звучит там, где стоит маркер, на расстояние радиуса; маркер виден только в редакторе")).clicked() {
            actions.push(Action::Mesh(MeshAction::NewSound(s.clone())));
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Polygon", "Многоугольник")).small().color(DIM));
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Draw a polygon", "Нарисовать многоугольник")).on_hover_text(tr("Click its corners on the map; click the first corner or press Enter to close it, Esc takes the last corner back. The inside is filled as an object you can move and edit like any other", "Кликайте по углам на карте; клик по первому углу или Enter замыкает контур, Esc убирает последний угол. Внутренность заполняется объектом, который можно двигать и править как любой другой")).clicked() {
            actions.push(Action::Mesh(MeshAction::PolygonDraw));
        }
        if ui.button(tr("From the selected splines", "По выделенным сплайнам")).on_hover_text(tr("Fill the outline the selected roads and splines make when they join end to end (kerbs round an island, for instance)", "Заполнить контур, который выделенные дороги и сплайны образуют, смыкаясь концами (например, бордюры вокруг островка)")).clicked() {
            actions.push(Action::Mesh(MeshAction::PolygonFromSplines));
        }
    });
    egui::Grid::new("polygon makers").num_columns(2).show(ui, |ui| {
        ui.label(RichText::new(tr("Texture", "Текстура")).small().color(DIM));
        ui.horizontal(|ui| {
            let name = s.poly_texture.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
            if ui.button(name.clone().unwrap_or_else(|| tr("Choose…", "Выбрать…").to_string())).clicked() {
                if let Some(f) = rfd::FileDialog::new().set_title(tr("Choose a picture or a texture", "Выберите картинку или текстуру")).add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "jpeg", "bmp", "tga", "dds", "tif", "tiff", "webp"]).pick_file() {
                    s.poly_texture = Some(f);
                }
            }
            if name.is_some() && ui.small_button("✕").on_hover_text(tr("No texture: a plain colour", "Без текстуры: просто цвет")).clicked() {
                s.poly_texture = None;
            }
        });
        ui.end_row();
        if s.poly_texture.is_none() {
            ui.label(RichText::new(tr("Colour", "Цвет")).small().color(DIM));
            ui.color_edit_button_srgb(&mut s.poly_color);
            ui.end_row();
        }
        ui.label(RichText::new(tr("Texture size, m", "Размер текстуры, м")).small().color(DIM));
        ui.add(egui::DragValue::new(&mut s.poly_scale).range(0.1..=500.0).speed(0.1));
        ui.end_row();
        ui.label(RichText::new(tr("Thickness, m", "Толщина, м")).small().color(DIM));
        ui.add(egui::DragValue::new(&mut s.poly_thickness).range(0.0..=50.0).speed(0.01));
        ui.end_row();
        ui.label(RichText::new(tr("Above the ground, m", "Над землёй, м")).small().color(DIM));
        ui.add(egui::DragValue::new(&mut s.poly_lift).range(-5.0..=5.0).speed(0.01));
        ui.end_row();
    });
    egui::Grid::new("extra makers").num_columns(2).show(ui, |ui| {
        ui.label(RichText::new(tr("Picture width, m", "Ширина картинки, м")).small().color(DIM));
        ui.add(egui::DragValue::new(&mut s.picture_width).range(0.1..=300.0).speed(0.1));
        ui.end_row();
        ui.label(RichText::new(tr("Repeats", "Повторов")).small().color(DIM));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut s.picture_repeat[0]).range(0.01..=200.0).speed(0.1));
            ui.add(egui::DragValue::new(&mut s.picture_repeat[1]).range(0.01..=200.0).speed(0.1));
        });
        ui.end_row();
        ui.label(RichText::new(tr("Picture", "Картинка")).small().color(DIM));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut s.picture_standing, false, tr("on the ground", "на земле"));
            ui.selectable_value(&mut s.picture_standing, true, tr("standing", "стоит"));
        });
        ui.end_row();
        ui.label(RichText::new(tr("Light colour", "Цвет света")).small().color(DIM));
        ui.color_edit_button_srgb(&mut s.light_color);
        ui.end_row();
        ui.label(RichText::new(tr("Light reach, m", "Дальность света, м")).small().color(DIM));
        ui.add(egui::DragValue::new(&mut s.light_range).range(1.0..=200.0));
        ui.end_row();
        ui.label(RichText::new(tr("Light", "Свет")).small().color(DIM));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut s.light_night_only, true, tr("at night", "ночью"));
            ui.selectable_value(&mut s.light_night_only, false, tr("always", "всегда"));
        });
        ui.end_row();
        ui.label(RichText::new(tr("Sound reach, m", "Дальность звука, м")).small().color(DIM));
        ui.add(egui::DragValue::new(&mut s.sound_radius).range(1.0..=1000.0));
        ui.end_row();
        ui.label(RichText::new(tr("Sound volume", "Громкость звука")).small().color(DIM));
        ui.add(egui::Slider::new(&mut s.sound_volume, 0.0..=1.0));
        ui.end_row();
    });
}
