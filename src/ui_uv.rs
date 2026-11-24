//! The UV editor (a window beside the edit mode, as Blender's UV Editor): the faces of the
//! part being edited laid over their texture, the selected ones lit; a click picks a face,
//! a drag moves what is selected, the wheel zooms and the right button pans; buttons turn,
//! flip, fit, pack and even out the islands, and mark seams where they part.

use std::path::PathBuf;

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use glam::{Affine2, Vec2 as GVec2};
use openmapper_mesh::edit::Selection;

use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::pal;

pub struct UvEditor {
    /// The picture shown under the faces, and the file it was read from.
    tex: Option<(PathBuf, egui::TextureHandle)>,
    zoom: f32,
    pan: Vec2,
    /// A drag of the selection going on (its undo step taken).
    dragging: bool,
}

impl Default for UvEditor {
    fn default() -> UvEditor {
        UvEditor { tex: None, zoom: 1.0, pan: Vec2::ZERO, dragging: false }
    }
}

/// The window, while it is open and a mesh is being edited.
pub fn window(ctx: &egui::Context, st: &mut UvEditor, open: &mut bool, ed: &mut Editor, roots: Option<&crate::install::Roots>) {
    let Some(m) = ed.mesh.as_mut().filter(|m| m.editing) else { return };
    if !*open {
        return;
    }
    // the texture of the selected faces (or of the part's first face)
    let mat = m.sel.faces.iter().next().and_then(|f| m.mesh().faces.get(*f)).or_else(|| m.mesh().faces.first()).map(|f| f.material as usize).unwrap_or(0);
    let tex_name = m.src.model.materials.get(mat).and_then(|x| x.texture.clone());
    let path = match (roots, &tex_name) {
        (Some(r), Some(t)) => Some(crate::editable::folder_of(r, &m.rel).0.join("texture").join(t)),
        _ => None,
    };
    if st.tex.as_ref().map(|t| Some(&t.0) != path.as_ref()).unwrap_or(path.is_some()) {
        st.tex = path.as_ref().and_then(|p| {
            let img = crate::models::picture(None, Some(p))?;
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Some((p.clone(), ctx.load_texture(format!("uv {}", p.display()), ci, egui::TextureOptions::LINEAR)))
        });
    }
    let mut keep_open = *open;
    egui::Window::new(icons::labelled(Icon::Texture, tr("UV editor", "UV-редактор"))).id(egui::Id::new("uv editor")).open(&mut keep_open).default_size([520.0, 560.0]).resizable(true).show(ctx, |ui| {
        let faces: Vec<usize> = m.sel.faces.iter().copied().collect();
        ui.horizontal_wrapped(|ui| {
            use openmapper_mesh::uv;
            let op = |m: &mut crate::meshedit::Session, f: &dyn Fn(&mut openmapper_mesh::Mesh, &[usize])| {
                let faces = faces.clone();
                m.apply(|mesh, _, _| {
                    f(mesh, &faces);
                    None
                });
            };
            if ui.button(icons::labelled(Icon::AutoFix, tr("Smart unwrap", "Умная развёртка"))).on_hover_text(tr("Every flat side apart in the picture, no faces on top of one another (all faces if none is selected)", "Каждая плоская сторона отдельно на картинке, грани не лежат друг на друге (все, если ничего не выделено)")).clicked() {
                op(m, &|mesh, f| uv::unwrap(mesh, f, uv::Unwrap::Smart { angle_deg: 66.0 }, 0.01));
            }
            if ui.button(icons::labelled(Icon::Cube, tr("Box unwrap", "Развёртка коробкой"))).on_hover_text(tr("The six sides of a box apart in the picture", "Шесть сторон коробки отдельно на картинке")).clicked() {
                op(m, &|mesh, f| uv::unwrap(mesh, f, uv::Unwrap::Cube { size: 1.0 }, 0.01));
            }
            ui.separator();
            if ui.button(icons::labelled(Icon::RotateLeft, "90°")).clicked() {
                op(m, &|mesh, f| uv::rotate(mesh, f, 90.0));
            }
            if ui.button(icons::labelled(Icon::Rotate, "90°")).clicked() {
                op(m, &|mesh, f| uv::rotate(mesh, f, -90.0));
            }
            if ui.button(icons::labelled(Icon::Swap, tr("Flip", "Отразить"))).on_hover_text(tr("Left to right", "Слева направо")).clicked() {
                op(m, &|mesh, f| uv::flip(mesh, f, true));
            }
            if ui.button(icons::labelled(Icon::SwapVert, tr("Flip", "Отразить"))).on_hover_text(tr("Top to bottom", "Сверху вниз")).clicked() {
                op(m, &|mesh, f| uv::flip(mesh, f, false));
            }
            if ui.button(icons::labelled(Icon::ZoomIn, tr("Scale ×2", "Масштаб ×2"))).clicked() {
                op(m, &|mesh, f| scale_about_centre(mesh, f, 2.0));
            }
            if ui.button(icons::labelled(Icon::ZoomOut, tr("Scale ÷2", "Масштаб ÷2"))).clicked() {
                op(m, &|mesh, f| scale_about_centre(mesh, f, 0.5));
            }
            if ui.button(icons::labelled(Icon::Fit, tr("Fit", "Вписать"))).on_hover_text(tr("Stretched to fill the picture", "Растянуть на всю картинку")).clicked() {
                op(m, &|mesh, f| uv::fit(mesh, f));
            }
            if ui.button(tr("Pack islands", "Упаковать острова")).clicked() {
                op(m, &|mesh, f| uv::pack(mesh, f, 0.01, true));
            }
            if ui.button(tr("Average island scale", "Выровнять масштаб островов")).clicked() {
                op(m, &|mesh, f| uv::average_scale(mesh, f));
            }
            if ui.button(tr("Seams from islands", "Швы по островам")).clicked() {
                m.apply(|mesh, _, _| {
                    uv::seams_from_islands(mesh);
                    None
                });
            }
        });
        ui.label(egui::RichText::new(tr("Click: a face (Shift adds) · drag: move the selected · wheel: zoom · right button: pan", "Клик: грань (Shift — добавить) · протяжка: двигать выделенное · колесо: масштаб · правая кнопка: панорама")).small().color(pal().text_dim));
        let (rect, resp) = ui.allocate_exact_size(ui.available_size().max(Vec2::splat(200.0)), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, pal().view);
        let side = rect.width().min(rect.height()) * 0.9 * st.zoom;
        let origin = rect.center() - Vec2::splat(side * 0.5) + st.pan;
        let to_screen = |uv: GVec2| Pos2::new(origin.x + uv.x * side, origin.y + uv.y * side);
        let from_screen = |p: Pos2| GVec2::new((p.x - origin.x) / side, (p.y - origin.y) / side);
        // the picture once, and its repeats faintly round it
        let unit = Rect::from_min_size(origin, Vec2::splat(side));
        if let Some((_, h)) = &st.tex {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let r = unit.translate(Vec2::new(dx as f32 * side, dy as f32 * side));
                    let tint = if dx == 0 && dy == 0 { Color32::WHITE } else { Color32::from_gray(70) };
                    painter.image(h.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
                }
            }
        } else {
            painter.rect_filled(unit, 0.0, Color32::from_gray(45));
        }
        painter.rect_stroke(unit, 0.0, Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Middle);
        let mesh = m.mesh().clone();
        let selected = &m.sel.faces;
        for (fi, f) in mesh.faces.iter().enumerate() {
            let pts: Vec<Pos2> = f.uvs.iter().map(|uv| to_screen(*uv)).collect();
            let on = selected.contains(&fi);
            if on {
                painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(255, 140, 40, 60), Stroke::NONE));
            }
            let stroke = if on { Stroke::new(1.5, Color32::from_rgb(255, 160, 60)) } else { Stroke::new(1.0, Color32::from_rgba_unmultiplied(230, 230, 235, 140)) };
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        // zoom about the cursor, pan with the right button
        if let Some(hover) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let before = from_screen(hover);
                st.zoom = (st.zoom * (1.0 + scroll * 0.002)).clamp(0.1, 40.0);
                let side2 = rect.width().min(rect.height()) * 0.9 * st.zoom;
                let origin2 = rect.center() - Vec2::splat(side2 * 0.5) + st.pan;
                let after = Pos2::new(origin2.x + before.x * side2, origin2.y + before.y * side2);
                st.pan += hover - after;
            }
        }
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            st.pan += resp.drag_delta();
        }
        // a click on a face picks it; a drag moves the selection
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let uv = from_screen(p);
                let hit = mesh.faces.iter().enumerate().rev().find(|(_, f)| inside(uv, &f.uvs)).map(|(i, _)| i);
                let mut faces: Vec<usize> = if shift { m.sel.faces.iter().copied().collect() } else { Vec::new() };
                if let Some(h) = hit {
                    if let Some(i) = faces.iter().position(|x| *x == h) {
                        faces.remove(i);
                    } else {
                        faces.push(h);
                    }
                }
                m.mode = openmapper_mesh::edit::SelectMode::Face;
                m.sel = Selection::from_faces(m.mesh(), faces.iter().copied());
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) && !m.sel.faces.is_empty() {
            if !st.dragging {
                st.dragging = true;
                m.checkpoint();
            }
            let d = resp.drag_delta() / side;
            let faces: Vec<usize> = m.sel.faces.iter().copied().collect();
            openmapper_mesh::uv::transform(m.mesh_mut(), &faces, Affine2::from_translation(GVec2::new(d.x, d.y)));
            m.changed();
        }
        if resp.drag_stopped() {
            st.dragging = false;
        }
        ui.ctx().request_repaint();
    });
    *open = keep_open;
}

fn inside(p: GVec2, poly: &[GVec2]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
    }
    c
}

fn scale_about_centre(mesh: &mut openmapper_mesh::Mesh, faces: &[usize], k: f32) {
    let (lo, hi) = openmapper_mesh::uv::uv_bounds(mesh, faces);
    let c = (lo + hi) * 0.5;
    openmapper_mesh::uv::transform(mesh, faces, Affine2::from_translation(c) * Affine2::from_scale(GVec2::splat(k)) * Affine2::from_translation(-c));
}
