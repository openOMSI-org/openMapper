//! The interface of the editing tools: the box dragged out to select, the snapping switches
//! in the status bar, the properties of several things selected at once and a road's
//! heights at its ends.

use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use glam::{DVec2, DVec3};

use crate::doc::{Sel, TileKey};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::ui::{check, prop_row, section, ACCENT, TEXT_DIM};

/// The selection box over the view.
pub fn paint_box(ctx: &egui::Context, ed: &Editor) {
    let Some([a, b]) = ed.select_box else { return };
    let r = egui::Rect::from_two_pos(egui::pos2(a.0, a.1), egui::pos2(b.0, b.1));
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("select box")));
    p.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(232, 122, 50, 28));
    p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
}

/// The names of the bus stops and start points near the camera, over them in the view.
pub fn paint_names(ctx: &egui::Context, ed: &Editor, lib: &Library, cam: &crate::camera::Camera, view: egui::Rect) {
    let Some(doc) = ed.doc.as_ref() else { return };
    if !ed.show_names || !view.is_positive() {
        return;
    }
    let eye = cam.eye();
    const REACH: f64 = 450.0;
    let mut names: Vec<(f64, DVec3, String, bool)> = Vec::new();
    for (k, t) in &doc.tiles {
        // (only tiles near the camera)
        let mid = crate::doc::tile_origin(*k) + DVec2::splat(crate::doc::tile_size() * 0.5);
        if (mid - eye.truncate()).length() > REACH + crate::doc::tile_size() {
            continue;
        }
        for o in &t.tile.objects {
            let Some(a) = lib.find(&o.file) else { continue };
            let name = if a.is_bus_stop {
                o.extra.first().map(|n| n.replace('@', " ")).unwrap_or_default()
            } else if a.is_entry_point {
                doc.global.entry_points.iter().find(|e| e.object_id == o.id).map(|e| e.name.clone()).unwrap_or_default()
            } else {
                continue;
            };
            if name.trim().is_empty() {
                continue;
            }
            let p = doc.object_world(*k, o, a.abs_height) + DVec3::new(0.0, 0.0, 3.5);
            let d = (p - eye).length();
            if d < REACH {
                names.push((d, p, name, a.is_bus_stop));
            }
        }
    }
    names.sort_by(|a, b| a.0.total_cmp(&b.0));
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("names"))).with_clip_rect(view);
    let font = egui::FontId::proportional(12.0);
    // (nearest first; one that would lie over a nearer one is left out)
    let mut taken: Vec<egui::Rect> = Vec::new();
    for (d, p, name, stop) in names.into_iter().take(80) {
        let Some((x, y)) = crate::selection::project(cam, p, view.width(), view.height()) else { continue };
        let at = view.min + Vec2::new(x, y);
        if !view.contains(at) {
            continue;
        }
        // (fainter further off)
        let alpha = (255.0 * (1.0 - (d / REACH) as f32 * 0.7)) as u8;
        let galley = painter.layout_no_wrap(name, font.clone(), Color32::from_rgba_unmultiplied(255, 255, 255, alpha));
        let r = egui::Rect::from_center_size(at - Vec2::new(0.0, galley.size().y * 0.5), galley.size() + Vec2::new(10.0, 4.0));
        if taken.iter().any(|t| t.intersects(r)) {
            continue;
        }
        taken.push(r);
        let fill = if stop { Color32::from_rgba_unmultiplied(30, 70, 120, alpha / 4 * 3) } else { Color32::from_rgba_unmultiplied(120, 60, 20, alpha / 4 * 3) };
        painter.rect_filled(r, CornerRadius::same(3), fill);
        painter.galley(r.min + Vec2::new(5.0, 2.0), galley, Color32::WHITE);
    }
}

/// The number of each path (lane) of a road or object over the view, as its path rules name
/// them: at the lane's middle, with the way traffic goes.
pub fn paint_path_numbers(ctx: &egui::Context, lanes: &[crate::lanes::Lane], cam: &crate::camera::Camera, view: egui::Rect) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("path numbers"))).with_clip_rect(view);
    let font = egui::FontId::proportional(11.0);
    let mut taken: Vec<egui::Rect> = Vec::new();
    for l in lanes.iter().filter(|l| l.pts.len() >= 2) {
        let mid = l.pts.len() / 2;
        let (a, b) = (l.pts[mid - 1], l.pts[mid]);
        let Some((x, y)) = crate::selection::project(cam, (a + b) * 0.5 + DVec3::Z * 0.3, view.width(), view.height()) else { continue };
        let at = view.min + Vec2::new(x, y);
        if !view.contains(at) {
            continue;
        }
        let way = match l.direction {
            2 => "↔",
            _ => "",
        };
        let fill = match l.kind {
            1 => Color32::from_rgb(60, 110, 160),
            2 => Color32::from_rgb(120, 80, 150),
            _ => Color32::from_rgb(170, 120, 30),
        };
        let galley = painter.layout_no_wrap(format!("#{}{way}", l.index), font.clone(), Color32::WHITE);
        let mut r = egui::Rect::from_center_size(at, galley.size() + Vec2::new(8.0, 2.0));
        // (lanes side by side: their labels one under another rather than on top)
        while taken.iter().any(|t| t.intersects(r)) {
            r = r.translate(Vec2::new(0.0, r.height() + 1.0));
        }
        taken.push(r);
        painter.rect_filled(r, CornerRadius::same(3), fill);
        painter.galley(r.min + Vec2::new(4.0, 1.0), galley, Color32::WHITE);
    }
}

/// What the ruler measures, by its line in the view.
pub fn paint_ruler(ctx: &egui::Context, ed: &Editor, cam: &crate::camera::Camera, view: egui::Rect) {
    let Some((a, b)) = ed.ruler_line() else { return };
    let d = b - a;
    let flat = d.truncate().length();
    let grade = if flat > 0.01 { format!("  {:+.1} %", d.z / flat * 100.0) } else { String::new() };
    let Some((x, y)) = crate::selection::project(cam, (a + b) * 0.5 + DVec3::Z * 1.0, view.width(), view.height()) else { return };
    let at = view.min + Vec2::new(x, y);
    if !view.contains(at) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("ruler"))).with_clip_rect(view);
    let galley = painter.layout_no_wrap(format!("{flat:.2} m  Δh {:+.2} m{grade}", d.z), egui::FontId::monospace(12.0), Color32::BLACK);
    let r = egui::Rect::from_center_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), Color32::from_rgb(255, 216, 51));
    painter.galley(r.min + Vec2::new(5.0, 2.0), galley, Color32::BLACK);
}

pub fn knife_help() -> &'static str {
    tr("Knife (K): a click on a road cuts it there, as often as you like; the pieces' lengths shown at the cursor, on whole grid steps with grid snapping on · right-click or Esc: off", "Нож (K): клик по дороге разрезает её в этом месте, сколько угодно раз; длины кусков видны у курсора, с привязкой к сетке — по целым шагам сетки · правый клик или Esc: выключить")
}

/// The two pieces the knife would leave, by its line in the view.
pub fn paint_cut(ctx: &egui::Context, ed: &Editor, cam: &crate::camera::Camera, view: egui::Rect) {
    let Some(c) = ed.cut_preview() else { return };
    let Some((x, y)) = crate::selection::project(cam, c.pos + DVec3::Z * 1.0, view.width(), view.height()) else { return };
    let at = view.min + Vec2::new(x, y - 22.0);
    if !view.contains(at) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("knife"))).with_clip_rect(view);
    let galley = painter.layout_no_wrap(format!("{:.2} m | {:.2} m", c.at, c.length - c.at), egui::FontId::monospace(12.0), Color32::WHITE);
    let r = egui::Rect::from_center_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), if c.fits { Color32::from_rgb(200, 55, 45) } else { Color32::from_gray(90) });
    painter.galley(r.min + Vec2::new(5.0, 2.0), galley, Color32::WHITE);
}

/// How many of the status bar's snapping units there are (see `snap_unit`).
const SNAP_UNITS: usize = 9;

/// The width a snapping unit takes in the status bar (its toggles are as wide as their text).
fn snap_unit_width(ui: &egui::Ui, i: usize) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    let toggle = |t: &str| {
        let galley = egui::WidgetText::from(RichText::new(t).small()).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
        18.0 + 6.0 + galley.size().x + gap
    };
    match i {
        0 => 46.0 + gap + toggle(tr("Angle", "Угол")),
        1 => 56.0 + gap + toggle(tr("Grid", "Сетка")),
        2 => toggle(tr("Knife", "Нож")),
        3 => toggle(tr("Ruler", "Линейка")),
        4 => toggle(tr("Auto-Terrain", "Авто-рельеф")),
        5 => toggle(tr("Auto-Switch", "Авто-стрелки")),
        6 => toggle(tr("Auto-Junction", "Авто-перекрёсток")),
        7 => toggle(tr("Auto-Adapt", "Авто-переход")),
        _ => toggle(tr("Auto-Align", "Авто-выравнивание")),
    }
}

/// One snapping unit: a toggle, or a step and its toggle.
fn snap_unit(ui: &mut egui::Ui, ed: &mut Editor, i: usize) {
    match i {
        0 => {
            // (laid out right to left: the toggle, then the step)
            check(ui, &mut ed.snap_angle, RichText::new(tr("Angle", "Угол")).small()).on_hover_text(tr("Snap headings to the angle step", "Привязка направления к шагу угла"));
            egui::ComboBox::from_id_salt("angle step").width(46.0).selected_text(RichText::new(format!("{}°", ed.angle_step)).small()).show_ui(ui, |ui| {
                for s in [1.0, 5.0, 10.0, 15.0, 22.5, 30.0, 45.0, 90.0] {
                    ui.selectable_value(&mut ed.angle_step, s, format!("{s}°"));
                }
            }).response.on_hover_text(tr("The angle step: what snapping comes to and what R turns by", "Шаг угла: к чему привязывается направление и на сколько поворачивает R"));
        }
        1 => {
            check(ui, &mut ed.snap_grid, RichText::new(tr("Grid", "Сетка")).small()).on_hover_text(tr("Snap positions to the grid", "Привязка положения к сетке"));
            egui::ComboBox::from_id_salt("grid step").width(56.0).selected_text(RichText::new(format!("{} m", ed.grid_step)).small()).show_ui(ui, |ui| {
                for s in [0.5, 1.0, 2.0, 5.0, 10.0] {
                    ui.selectable_value(&mut ed.grid_step, s, format!("{s} m"));
                }
            });
        }
        2 => {
            let mut knife = ed.cutting;
            if check(ui, &mut knife, RichText::new(tr("Knife", "Нож")).small()).on_hover_text(knife_help()).changed() {
                ed.toggle_cutting();
            }
        }
        3 => {
            let mut on = ed.ruler.is_some();
            if check(ui, &mut on, RichText::new(tr("Ruler", "Линейка")).small()).on_hover_text(tr("Click two points to measure the distance and gradient between them (U) · right-click: off", "Клик по двум точкам: расстояние и уклон между ними (U) · правый клик: выключить")).changed() {
                ed.toggle_ruler();
            }
        }
        4 => {
            check(ui, &mut ed.auto_terrain, RichText::new(tr("Auto-Terrain", "Авто-рельеф")).small()).on_hover_text(tr("What you build follows the ground: roads, paths and tracks take its height (off: they run level from where they start). Only what is built from now on; nothing already there changes", "Новое подстраивается под рельеф: дороги, пути и рельсы берут его высоту (выкл.: идут ровно от точки начала). Только то, что строится дальше; уже построенное не меняется"));
        }
        5 => {
            check(ui, &mut ed.auto_switches, RichText::new(tr("Auto-Switch", "Авто-стрелки")).small()).on_hover_text(tr("Branching a track off another makes a switch there", "Ответвление пути от другого создаёт стрелку"));
        }
        6 => {
            check(ui, &mut ed.auto_junctions, RichText::new(tr("Auto-Junction", "Авто-перекрёсток")).small()).on_hover_text(tr("Make junctions where roads meet", "Делать перекрёстки, где дороги встречаются"));
        }
        7 => {
            check(ui, &mut ed.auto_adapt, RichText::new(tr("Auto-Adapt", "Авто-переход")).small()).on_hover_text(tr("Where roads of different width meet, a plate tapers one into the other", "Где встречаются дороги разной ширины, между ними плавный переход"));
        }
        _ => {
            check(ui, &mut ed.align_to_road, RichText::new(tr("Auto-Align", "Авто-выравнивание")).small()).on_hover_text(tr("Line objects up with the nearest road (Shift: put it freely)", "Выравнивать объекты по ближайшей дороге (Shift: ставить свободно)"));
        }
    }
}

/// What the snapping controls of the status bar take when all of them are shown.
pub fn snap_controls_width(ui: &egui::Ui) -> f32 {
    (0..SNAP_UNITS).map(|i| snap_unit_width(ui, i)).sum::<f32>() + ui.spacing().item_spacing.x * 2.0
}

/// Grid and angle snapping and the Auto-... switches, in the status bar. When `room` is not
/// enough for all of them the last ones go into a menu behind a "..." button instead of
/// dropping out of the bar one after another as the window narrows.
pub fn snap_controls(ui: &mut egui::Ui, ed: &mut Editor, room: f32) {
    ui.spacing_mut().interact_size.y = 16.0;
    let more_w = 28.0 + ui.spacing().item_spacing.x;
    let full = snap_controls_width(ui);
    // (how many units fit; when not all, the "more" button takes some room too)
    let mut shown = SNAP_UNITS;
    if full > room {
        let mut left = room - more_w;
        shown = 0;
        for i in 0..SNAP_UNITS {
            let w = snap_unit_width(ui, i);
            if w > left && shown >= 2 {
                break;
            }
            left -= w;
            shown += 1;
        }
    }
    // (laid out right to left: the first units stay on the right)
    for i in 0..2.min(shown) {
        snap_unit(ui, ed, i);
    }
    if shown > 2 {
        ui.separator();
        // the ruler's measurement, when there is room for it
        if shown == SNAP_UNITS {
            if let Some((a, b)) = ed.ruler_line() {
                let d = b - a;
                let flat = d.truncate().length();
                let grade = if flat > 0.01 { format!("{:+.1} %", d.z / flat * 100.0) } else { "-".into() };
                ui.label(RichText::new(format!("{:.2} m · Δh {:+.2} m · {grade}", flat, d.z)).small().monospace().color(Color32::from_rgb(255, 216, 51)));
            }
        }
        for i in 2..shown {
            snap_unit(ui, ed, i);
        }
    }
    if shown < SNAP_UNITS {
        let r = ui.add(egui::Button::new(RichText::new("···").strong()).min_size(Vec2::new(28.0, 16.0)));
        let r = r.on_hover_text(tr("More switches: the window is too narrow for all of them", "Ещё переключатели: окно слишком узкое для всех"));
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(170.0);
            // (a menu lays out top down: the units in the order of the bar)
            for i in shown..SNAP_UNITS {
                snap_unit(ui, ed, i);
            }
        });
    }
}

/// The right panel with more than one thing selected.
pub fn group_properties(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library, actions: &mut Vec<crate::ui::Action>) {
    let sels = ed.selection();
    let objects = sels.iter().filter(|s| !matches!(s, Sel::Spline(..))).count();
    let splines = sels.len() - objects;
    section(ui, tr("Selection", "Выделение"));
    egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.label(RichText::new(format!("{} {}", tr("Selected:", "Выбрано:"), sels.len())).strong().color(Color32::WHITE));
        ui.label(RichText::new(format!("{} {objects} · {} {splines}", tr("objects", "объектов"), tr("splines", "сплайнов"))).small().color(TEXT_DIM));
        ui.add_space(6.0);
        // a group: its name, and undoing it; else making one
        match ed.selected_group() {
            Some(g) => {
                let mut name = ed.groups()[g].name.clone();
                ui.horizontal(|ui| {
                    ui.label(icons::labelled(Icon::Layers, tr("Group", "Группа")));
                    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)).changed() {
                        ed.rename_group(g, &name);
                    }
                    if ui.button(tr("Ungroup", "Разгруппировать")).on_hover_text("Ctrl+Shift+G").clicked() {
                        ed.ungroup_selection();
                    }
                });
                ui.label(RichText::new(tr("A click on one of it selects all of it, Ctrl + click one alone", "Клик по любому её объекту выделяет всю группу, Ctrl + клик — один объект")).small().color(TEXT_DIM));
            }
            None => {
                if ui.button(icons::labelled(Icon::Layers, tr("Group", "Сгруппировать"))).on_hover_text(tr("Keep these together: a click on one selects all, a drag moves all (Ctrl+G)", "Держать вместе: клик по одному выделяет все, перетаскивание двигает все (Ctrl+G)")).clicked() {
                    ed.group_selection();
                }
            }
        }
        ui.add_space(6.0);
        // (the numbers are kept between frames until applied)
        let id = egui::Id::new("group numbers");
        let mut v: [f64; 4] = ui.data_mut(|d| *d.get_temp_mut_or_default(id));
        let mut act: Option<u8> = None;
        egui::Grid::new("group").num_columns(3).spacing(Vec2::new(6.0, 5.0)).show(ui, |ui| {
            ui.label(RichText::new(tr("Move", "Сдвиг")).color(TEXT_DIM));
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(&mut v[0]).speed(0.1).max_decimals(2).prefix("X ").suffix(" m").min_decimals(0));
                ui.add(egui::DragValue::new(&mut v[1]).speed(0.1).max_decimals(2).prefix("Y ").suffix(" m").min_decimals(0));
            });
            if ui.button(tr("Apply", "Применить")).clicked() {
                act = Some(0);
            }
            ui.end_row();
            ui.label(RichText::new(tr("Height", "Высота")).color(TEXT_DIM));
            ui.add(egui::DragValue::new(&mut v[2]).speed(0.05).max_decimals(2).prefix("Z ").suffix(" m"));
            if ui.button(tr("Apply", "Применить")).clicked() {
                act = Some(1);
            }
            ui.end_row();
            ui.label(RichText::new(tr("Turn", "Поворот")).color(TEXT_DIM));
            ui.add(egui::DragValue::new(&mut v[3]).speed(1.0).range(-360.0..=360.0).suffix("°"));
            if ui.button(tr("Apply", "Применить")).clicked() {
                act = Some(2);
            }
            ui.end_row();
        });
        ui.data_mut(|d| d.insert_temp(id, v));
        match act {
            Some(0) => ed.move_selection_by(DVec2::new(v[0], v[1])),
            Some(1) => ed.raise_selection(v[2]),
            Some(2) => ed.rotate(v[3]),
            _ => {}
        }
        if objects > 0 {
            ui.add_space(4.0);
            if ui.button(icons::labelled(Icon::Flatten, tr("Level the ground under them", "Выровнять землю под ними"))).on_hover_text(tr("Each one's outline made level with its foot, blending into the ground 3 m round it", "Под каждым — ровно на уровне его основания, с плавным переходом 3 м вокруг")).clicked() {
                ed.level_request = true;
            }
            if let Some(ot) = ed.object_type.clone() {
                let n = lib.find(&ot).map(|a| a.name.clone()).unwrap_or_else(|| ot.clone());
                ui.add_space(4.0);
                if ui.button(icons::labelled(Icon::Swap, &format!("{} «{}»", tr("Make every object", "Сделать все объекты"), n))).on_hover_text(tr("The object chosen in the Objects tool's catalogue", "Объект, выбранный в каталоге инструмента «Объекты»")).clicked() {
                    ed.replace_selected_objects(&ot);
                }
            }
        }
        if objects > 1 {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("Random headings", "Случайные повороты")).on_hover_text(tr("Every selected object turned its own random way (trees, bushes, rocks)", "Каждый выбранный объект повёрнут случайно (деревья, кусты, камни)")).clicked() {
                    ed.turn_objects(true);
                }
                if ui.button(tr("Along the road", "Вдоль дороги")).on_hover_text(tr("Each turned the way the nearest road runs beside it", "Каждый повёрнут по ходу ближайшей дороги рядом")).clicked() {
                    let n = ed.align_to_roads();
                    ed.toast(format!("{} {n}", tr("Turned:", "Повёрнуто:")));
                }
                if ui.add_enabled(matches!(ed.selected, Some(Sel::Object(..))), egui::Button::new(tr("All like this one", "Все как этот"))).on_hover_text(tr("Every selected object takes the heading of the one in the properties", "Все выбранные объекты получают поворот того, что в свойствах")).clicked() {
                    ed.turn_objects(false);
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("Lean with the ground", "Наклонить по земле")).on_hover_text(lean_help()).clicked() {
                    ed.lean_objects(true);
                }
                if ui.button(tr("Stand upright", "Поставить ровно")).clicked() {
                    ed.lean_objects(false);
                }
            });
        }
        if splines > 0 {
            if let Some(rt) = ed.road_type.clone() {
                let n = lib.find(&rt).map(|a| a.name.clone()).unwrap_or_else(|| rt.clone());
                ui.add_space(4.0);
                if ui.button(icons::labelled(Icon::Swap, &format!("{} «{}»", tr("Make every road", "Сделать все дороги"), n))).on_hover_text(tr("The road type chosen in the Roads tool's catalogue", "Тип дороги, выбранный в каталоге инструмента «Дороги»")).clicked() {
                    ed.retype_splines(&rt);
                }
            }
        }
        if splines > 0 {
            ui.add_space(4.0);
            if ui.button(tr("Lay them on the ground", "Положить их на землю")).on_hover_text(lay_help()).clicked() {
                ed.lay_selected_on_ground();
            }
            if ui.button(tr("Reverse their direction", "Развернуть их направление")).on_hover_text(reverse_help()).clicked() {
                ed.reverse_selected();
            }
        }
        if splines > 1 {
            ui.add_space(4.0);
            if ui.button(tr("Even slope along them", "Ровный уклон вдоль них")).on_hover_text(tr("One chain of roads: from the height at one end to that at the other, evenly (a bridge, a ramp)", "Одна цепочка дорог: от высоты на одном конце до высоты на другом, равномерно (мост, пандус)")).clicked() {
                ed.even_selected_chain();
            }
        }
        crate::ui_profile::profile_section(ui, ed, actions);
        paint_along_section(ui, ed, lib);
        if splines > 0 && !ed.editing_paths() {
            ui.add_space(4.0);
            if ui.button(icons::labelled(Icon::Route, tr("Edit its AI lanes", "Править полосы ИИ"))).on_hover_text(tr("Its lanes become AI paths of their own (the same lines, the same way) that the Logic tool's Edit existing bends, moves and deletes. The road keeps its look.", "Полосы становятся отдельными путями ИИ (те же линии, то же направление): их можно двигать, гнуть и удалять в Логике > Пути ИИ > Править готовые. Вид дороги не меняется.")).clicked() {
                actions.push(crate::ui::Action::DetachLanes);
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button(icons::labelled(Icon::Copy, tr("Copy", "Копировать"))).clicked() {
                ed.copy(lib);
            }
            if ui.button(tr("Duplicate", "Дублировать")).clicked() {
                ed.duplicate(lib);
            }
            if ui.button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked() {
                ed.delete_selected();
            }
            if ui.button(tr("Deselect", "Снять выделение")).clicked() {
                ed.select(None);
            }
        });
    });
    array_section(ui, ed, lib);
    parallel_section(ui, ed, lib);
    rows_section(ui, ed, lib);
}

/// What was changed at a road's ends this frame.
#[derive(Default)]
pub struct EndsEdit {
    /// (at the end rather than the start, new height, new gradient)
    change: Option<(bool, Option<f64>, Option<f64>)>,
    /// Both gradients to this.
    even: Option<f64>,
    /// (at the end rather than the start, new cant)
    cant: Option<(bool, f64)>,
}

/// A road's heights and gradients at its two ends.
pub fn spline_ends(ui: &mut egui::Ui, doc: &crate::doc::Document, k: TileKey, id: i64) -> EndsEdit {
    let mut out = EndsEdit::default();
    let Some(s) = doc.spline(k, id) else { return out };
    let c = omsi_geometry::SplineCurve::from_map(s, crate::doc::tile_origin(k));
    let (mut z0, mut z1) = (c.point_at(0.0).z, c.end_point().z);
    let (mut g0, mut g1) = (c.slope_at(0.0) * 100.0, c.slope_at(s.length) * 100.0);
    let (mut c0, mut c1) = (s.cant_start, s.cant_end);
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Ends (joined roads follow)", "Концы (соседние дороги следуют)")).strong());
    egui::Grid::new("spline ends").num_columns(4).spacing(Vec2::new(6.0, 5.0)).show(ui, |ui| {
        ui.label("");
        ui.label(RichText::new(tr("height", "высота")).small().color(TEXT_DIM));
        ui.label(RichText::new(tr("gradient", "уклон")).small().color(TEXT_DIM));
        ui.label(RichText::new(tr("cant", "вираж")).small().color(TEXT_DIM)).on_hover_text(tr("Banking across the road, %: positive lowers its right side (into a right bend)", "Поперечный наклон, %: плюс опускает правую сторону (в правый поворот)"));
        ui.end_row();
        for (at_end, z, g, cant, name) in [(false, &mut z0, &mut g0, &mut c0, tr("Start", "Начало")), (true, &mut z1, &mut g1, &mut c1, tr("End", "Конец"))] {
            prop_row(ui, name, |ui| {
                if ui.add(egui::DragValue::new(z).speed(0.02).max_decimals(2).suffix(" m")).changed() {
                    out.change = Some((at_end, Some(*z), None));
                }
                let lim = g.abs().max(40.0);
                if ui.add(egui::DragValue::new(g).speed(0.05).range(-lim..=lim).max_decimals(2).suffix(" %")).changed() {
                    out.change = Some((at_end, None, Some(*g)));
                }
                if ui.add(egui::DragValue::new(cant).speed(0.05).range(-100.0..=100.0).max_decimals(2).suffix(" %")).changed() {
                    out.cant = Some((at_end, *cant));
                }
            });
        }
    });
    if ui.button(tr("Even slope end to end", "Ровный уклон от начала до конца")).on_hover_text(tr("Both gradients to the straight climb between the ends' heights", "Оба уклона: по прямой между высотами концов")).clicked() {
        out.even = Some((z1 - z0) / s.length.max(0.1) * 100.0);
    }
    out
}

/// Put what `spline_ends` changed into the map (a dragged number is one step).
pub fn apply_ends(ed: &mut Editor, ctx: &egui::Context, k: TileKey, id: i64, e: EndsEdit) {
    if let Some(g) = e.even {
        ed.set_spline_end(k, id, false, None, Some(g));
        ed.set_spline_end(k, id, true, None, Some(g));
    }
    if let Some((at_end, c)) = e.cant {
        ed.set_spline_cant(k, id, at_end, c);
        return;
    }
    match e.change {
        Some((at_end, z, g)) => ed.set_spline_end(k, id, at_end, z, g),
        None => ed.end_panel_step_if_idle(ctx),
    }
}

/// The rule kinds OMSI 2's editor offers for a path, with whether they take a value.
pub(crate) const RULES: [(&str, bool); 6] = [("speedlimit", true), ("trafficdensity", true), ("priority", true), ("trucks", true), ("no_cars", false), ("bus", false)];

pub(crate) fn rule_name(kind: &str) -> &'static str {
    match kind {
        "speedlimit" => tr("Speed limit, km/h", "Ограничение скорости, км/ч"),
        "trafficdensity" => tr("Traffic density ×", "Плотность трафика ×"),
        "priority" => tr("Priority", "Приоритет"),
        "trucks" => tr("Trucks (0: none)", "Грузовики (0: нет)"),
        "no_cars" => tr("No cars", "Без легковых"),
        "bus" => tr("Buses only", "Только автобусы"),
        _ => tr("Other", "Другое"),
    }
}

/// `bus` only opens a path to buses; it is `no_cars` that shuts it to the cars. "Buses only"
/// therefore needs both rules on the path, as OMSI 2's editor writes them.
pub(crate) fn close_path_to_cars(rules: &mut Vec<omsi_map::MapRule>, path: i32) {
    if !rules.iter().any(|r| r.path_index == path && r.kind == "no_cars" && !r.kill) {
        rules.push(omsi_map::MapRule { path_index: path, kind: "no_cars".into(), value: 0.0, extra: 0.0, kill: false });
    }
}

/// The rules of a road's (or an object's) paths, as OMSI 2's editor sets them: a speed
/// limit, the traffic density, priority, trucks, cars and buses, each for one path.
pub fn rules_section(ui: &mut egui::Ui, ed: &mut Editor, sel: Sel) {
    let Some(doc) = ed.doc.as_ref() else { return };
    let rules = match sel {
        Sel::Object(k, id) => crate::selection::any_object(doc, k, id).map(|o| o.rules.clone()),
        Sel::Spline(k, id) => doc.spline(k, id).map(|s| s.rules.clone()),
        Sel::Attached(k, id) => doc.attachment(k, id).map(|a| a.rules.clone()),
    };
    let Some(mut rules) = rules else { return };
    let mut changed = false;
    let mut numbered = false;
    egui::Frame::new().inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Path rules", "Правила путей"), rules.iter().filter(|r| !r.kill).count())).strong()).id_salt("rules").default_open((1..=6).contains(&rules.iter().filter(|r| !r.kill).count())).show(ui, |ui| {
            ui.label(RichText::new(tr("For the AI and the player on one path (lane) of it, counted from 0: the view shows their numbers", "Для ИИ и игрока на одном пути (полосе), считая от 0: их номера видны в окне вида")).small().color(TEXT_DIM));
            numbered = true;
            let mut remove = None;
            let mut close_to_cars = None;
            for (i, r) in rules.iter_mut().enumerate().filter(|(_, r)| !r.kill) {
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut r.path_index).range(0..=64).prefix("#")).on_hover_text(tr("Path (lane) number", "Номер пути (полосы)")).changed();
                    let before = r.kind.clone();
                    egui::ComboBox::from_id_salt(("rule", i)).selected_text(rule_name(&r.kind)).width(104.0).show_ui(ui, |ui| {
                        for (k, _) in RULES {
                            ui.selectable_value(&mut r.kind, k.to_string(), rule_name(k));
                        }
                    });
                    if r.kind != before {
                        changed = true;
                        if r.kind == "bus" {
                            close_to_cars = Some(r.path_index);
                        }
                        r.value = match r.kind.as_str() {
                            "speedlimit" => 50.0,
                            "trafficdensity" => 1.0,
                            _ => 0.0,
                        };
                    }
                    if RULES.iter().any(|(k, v)| *k == r.kind && *v) || !RULES.iter().any(|(k, _)| *k == r.kind) {
                        changed |= ui.add(egui::DragValue::new(&mut r.value).speed(0.1).max_decimals(2)).changed();
                    }
                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                rules.remove(i);
                changed = true;
            }
            if let Some(path) = close_to_cars {
                close_path_to_cars(&mut rules, path);
            }
            if ui.button(icons::labelled(Icon::Add, tr("Add a rule", "Добавить правило"))).clicked() {
                rules.push(omsi_map::MapRule { path_index: 0, kind: "speedlimit".into(), value: 50.0, extra: 0.0, kill: false });
                changed = true;
            }
        });
    });
    ed.path_numbers = numbered.then_some(sel);
    if changed {
        ed.set_rules(sel, rules);
    } else {
        ed.end_panel_step_if_idle(ui.ctx());
    }
}

/// What hangs on an object's attachment points (signs on a post, a timetable on a stop),
/// and the catalogue's object hung on one, as OMSI 2's editor does it.
pub fn attach_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library, k: TileKey, id: i64, points: usize) {
    let Some(doc) = ed.doc.as_ref() else { return };
    let hung: Vec<(i64, usize, String)> = doc.tiles.get(&k).map(|t| t.tile.attach_objects.iter().filter(|a| a.parent_id == Some(id)).map(|a| (a.id, a.attach_index, a.file.clone())).collect()).unwrap_or_default();
    let key = egui::Id::new("attach point");
    let mut point: usize = ui.data_mut(|d| *d.get_temp_mut_or_default(key));
    point = point.min(points - 1);
    let mut go: Option<Sel> = None;
    let mut add = false;
    egui::Frame::new().inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        egui::CollapsingHeader::new(RichText::new(format!("{} ({})", tr("Hanging on it", "Висит на нём"), hung.len())).strong()).id_salt("attach").show(ui, |ui| {
            ui.label(RichText::new(format!("{} {points}", tr("Points to hang objects on:", "Точек для крепления:"))).small().color(TEXT_DIM));
            for (aid, at, file) in &hung {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{at}")).color(TEXT_DIM));
                    let n = lib.find(file).map(|a| a.name.clone()).unwrap_or_else(|| file.rsplit('\\').next().unwrap_or(file).to_string());
                    if ui.link(n).on_hover_text(tr("Select it", "Выбрать его")).clicked() {
                        go = Some(Sel::Object(k, *aid));
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut point).range(0..=points - 1).prefix("#"));
                let what = ed.object_type.as_ref().map(|o| lib.find(o).map(|a| a.name.clone()).unwrap_or_else(|| o.clone()));
                let label = match &what {
                    Some(n) => format!("{} «{n}»", tr("Hang", "Повесить")),
                    None => tr("Hang", "Повесить").to_string(),
                };
                add = ui.add_enabled(what.is_some(), egui::Button::new(icons::labelled(Icon::Add, &label)).truncate()).on_disabled_hover_text(tr("Choose an object in the Objects catalogue first", "Сначала выберите объект в каталоге «Объекты»")).clicked();
            });
        });
    });
    ui.data_mut(|d| d.insert_temp(key, point));
    if let Some(s) = go {
        ed.select(Some(s));
    } else if add {
        if let Some(f) = ed.object_type.clone() {
            ed.attach_object(k, id, point, &f);
        }
    }
}

/// Copies of the selection in a row: how many, the step between them and a turn each.
pub fn array_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library) {
    if ed.selection().is_empty() {
        return;
    }
    let key = egui::Id::new("array numbers");
    // (count, step x, step y, turn, rows, row step x, row step y: kept between frames)
    let mut v: [f64; 7] = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || [5.0, 10.0, 0.0, 0.0, 1.0, 0.0, 10.0]));
    let mut go = false;
    egui::Frame::new().inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| egui::CollapsingHeader::new(RichText::new(tr("Copies in a row", "Копии в ряд")).strong()).id_salt("array").show(ui, |ui| {
        ui.label(RichText::new(tr("A fence, a row of houses, an avenue: the selection copied again and again, a step on each time", "Забор, ряд домов, аллея: выделение копируется снова и снова, каждый раз на шаг дальше")).small().color(TEXT_DIM));
        egui::Grid::new("array").num_columns(2).spacing(Vec2::new(6.0, 5.0)).show(ui, |ui| {
            prop_row(ui, tr("Copies", "Копий"), |ui| {
                ui.add(egui::DragValue::new(&mut v[0]).speed(0.2).range(1.0..=200.0).max_decimals(0));
            });
            prop_row(ui, tr("Step", "Шаг"), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut v[1]).speed(0.1).prefix("X ").suffix(" m"));
                    ui.add(egui::DragValue::new(&mut v[2]).speed(0.1).prefix("Y ").suffix(" m"));
                });
            });
            prop_row(ui, tr("Turn each", "Поворот каждой"), |ui| {
                ui.add(egui::DragValue::new(&mut v[3]).speed(0.5).range(-180.0..=180.0).suffix("°"));
            });
            prop_row(ui, tr("Rows", "Рядов"), |ui| {
                ui.add(egui::DragValue::new(&mut v[4]).speed(0.1).range(1.0..=50.0).max_decimals(0)).on_hover_text(tr("More than one: a grid (an orchard, a car park), each row a row step on", "Больше одного: сетка (сад, стоянка), каждый ряд на шаг рядов дальше"));
            });
            if v[4] >= 2.0 {
                prop_row(ui, tr("Row step", "Шаг рядов"), |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut v[5]).speed(0.1).prefix("X ").suffix(" m"));
                        ui.add(egui::DragValue::new(&mut v[6]).speed(0.1).prefix("Y ").suffix(" m"));
                    });
                });
            }
        });
        let total = (v[0] as usize + 1) * v[4].max(1.0) as usize - 1;
        go = ui.button(icons::labelled(Icon::Copy, &format!("{} {}", tr("Make", "Сделать"), total))).clicked();
    }));
    ui.data_mut(|d| d.insert_temp(key, v));
    if go {
        ed.array(lib, v[0] as usize, DVec2::new(v[1], v[2]), v[3], v[4] as usize, DVec2::new(v[5], v[6]));
    }
}

pub(crate) fn lean_help() -> &'static str {
    tr("Tilted to the ground under it, the way it slopes forward and to the side (a fence, a bench or a shed on a hillside)", "Наклон по земле под ним, вперёд и вбок (забор, скамейка или сарай на склоне)")
}

pub(crate) fn lay_help() -> &'static str {
    tr("Heights along the ground again, smoothed so it climbs a hill rather than every bump (after shaping the ground or laying a height map over it); where a road not selected joins, its height is kept", "Высоты снова по земле, сглаженно: дорога поднимается на холм, а не на каждую кочку (после правки рельефа или наложения карты высот); где примыкает невыделенная дорога, её высота сохраняется")
}

pub(crate) fn reverse_help() -> &'static str {
    tr("Each starts where it ended: the traffic on it goes the other way (a one-way street, a track's direction), the road itself stays where it is", "Каждый начинается там, где кончался: движение по нему идёт в обратную сторону (улица с односторонним движением, направление пути), сама дорога остаётся на месте")
}

/// A copy of the selected road(s) or track(s) alongside, at a distance.
pub fn parallel_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library) {
    let sels = ed.selection();
    let Some(Sel::Spline(k, id)) = sels.iter().find(|s| matches!(s, Sel::Spline(..))).copied() else { return };
    egui::Frame::new().inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| parallel_inner(ui, ed, lib, k, id));
}

fn parallel_inner(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library, k: TileKey, id: i64) {
    ui.label(RichText::new(tr("Parallel copy", "Параллельная копия")).strong()).on_hover_text(tr("A second track or the other carriageway: the selected splines copied alongside, bends kept exact, linked as they are", "Второй путь или встречная половина: выбранные сплайны копируются рядом, повороты точно, связи как у оригинала"));
    // (the distance is kept between frames; at first the road's own width)
    let key = egui::Id::new("parallel distance");
    let width = ed.doc.as_ref().and_then(|d| d.spline(k, id)).and_then(|s| lib.find(&s.file)).map(|a| (a.half_width as f64 * 2.0 * 2.0).round() / 2.0).unwrap_or(4.0).max(3.0);
    let mut dist: f64 = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || width));
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut dist).speed(0.05).range(0.5..=200.0).max_decimals(2).suffix(" m"));
        if ui.button(tr("To the left", "Слева")).clicked() {
            ed.parallel_copy(-dist);
        }
        if ui.button(tr("To the right", "Справа")).clicked() {
            ed.parallel_copy(dist);
        }
    });
    ui.label(RichText::new(tr("Left and right of the way the road runs (its arrows with AI lanes shown)", "Слева и справа по ходу дороги (её стрелки при показе полос ИИ)")).small().color(TEXT_DIM));
    ui.data_mut(|d| d.insert_temp(key, dist));
    // a track's overhead wire: a wire spline laid on it, piece for piece
    // (whether the road copied is a track, read in its tile: one look-up, not one for every
    // spline of a selection of thousands - that was most of a frame)
    let track = ed.doc.as_ref().and_then(|d| d.spline(k, id)).and_then(|x| lib.find(&x.file)).map(|a| a.category == crate::library::Category::Tracks).unwrap_or(false);
    if !track {
        return;
    }
    let wires: Vec<&crate::library::Asset> = lib.assets.iter().filter(|a| a.kind == crate::library::Kind::Spline && ["fahrdraht", "oberleitung", "fahrleitung", "ohl", "wire", "catenary", "contact"].iter().any(|w| a.rel.to_lowercase().contains(w))).collect();
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Overhead wire", "Контактный провод")).strong()).on_hover_text(tr("For trams and electric trains: a wire spline laid on the selected track, piece for piece and linked (its height is the wire spline's own); the masts go in with Objects along the road", "Для трамваев и электричек: сплайн провода поверх выбранного пути, кусок за куском и со связями (высота — своя у сплайна провода); мачты ставятся «Объектами вдоль дороги»"));
    if wires.is_empty() {
        ui.label(RichText::new(tr("No wire spline installed (one whose name says Fahrdraht, Oberleitung or wire)", "Нет сплайна провода (с Fahrdraht, Oberleitung или wire в имени)")).small().color(TEXT_DIM));
        return;
    }
    let wkey = egui::Id::new("wire type");
    let mut chosen: String = ui.data_mut(|d| d.get_temp::<String>(wkey)).filter(|c| wires.iter().any(|w| w.rel == *c)).unwrap_or_else(|| wires[0].rel.clone());
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("wire pick").selected_text(wires.iter().find(|w| w.rel == chosen).map(|w| w.name.clone()).unwrap_or_default()).width(150.0).show_ui(ui, |ui| {
            for w in &wires {
                ui.selectable_value(&mut chosen, w.rel.clone(), &w.name).on_hover_text(&w.rel);
            }
        });
        if ui.button(icons::labelled(Icon::Add, tr("Lay it", "Проложить"))).clicked() {
            let n = ed.wire_along(&chosen);
            ed.toast(format!("{} {n}", tr("Wire pieces laid:", "Кусков провода:")));
        }
    });
    ui.data_mut(|d| d.insert_temp(wkey, chosen));
}

/// The ground painted along the selected roads: a layer under them or beside them.
pub fn paint_along_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library) {
    if !ed.selection().iter().any(|s| matches!(s, Sel::Spline(..))) || ed.editing_paths() {
        return;
    }
    let Some(doc) = ed.doc.as_ref() else { return };
    let names: Vec<String> = doc.global.ground_textures.iter().map(|g| std::path::Path::new(&g.texture.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()).collect();
    egui::CollapsingHeader::new(RichText::new(tr("Paint the ground along it", "Покрасить землю вдоль")).strong()).id_salt("paint along").show(ui, |ui| {
        if names.len() < 2 {
            ui.label(RichText::new(tr("Add a ground texture in the Paint tool first: the base ground is under everything", "Сначала добавьте текстуру земли в инструменте «Покраска»: основа лежит под всем")).small().color(TEXT_DIM));
            return;
        }
        let a = &mut ed.paint_along;
        a.layer = a.layer.clamp(1, names.len() - 1);
        egui::Grid::new("paint along grid").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
            ui.label(RichText::new(tr("Layer", "Слой")).color(TEXT_DIM));
            egui::ComboBox::from_id_salt("paint along layer").selected_text(&names[a.layer]).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate().skip(1) {
                    ui.selectable_value(&mut a.layer, i, n);
                }
            });
            ui.end_row();
            ui.label(RichText::new(tr("Where", "Где")).color(TEXT_DIM));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut a.under, false, tr("beside it", "рядом"));
                ui.selectable_value(&mut a.under, true, tr("under it too", "и под ней"));
            });
            ui.end_row();
            ui.label(RichText::new(tr("Out from its edge", "От края")).color(TEXT_DIM));
            ui.add(egui::DragValue::new(&mut a.width).range(0.0..=40.0).speed(0.1).suffix(" m"));
            ui.end_row();
            ui.label(RichText::new(tr("Soft edge", "Мягкий край")).color(TEXT_DIM));
            ui.add(egui::DragValue::new(&mut a.soft).range(0.0..=20.0).speed(0.1).suffix(" m"));
            ui.end_row();
        });
        if ui.button(icons::labelled(Icon::Brush, tr("Paint", "Покрасить"))).on_hover_text(tr("A gravel verge, dirt by a track, grass along a path: the brush of the Paint tool touches it up afterwards", "Гравийная обочина, грунт у путей, трава вдоль дорожки: кисть «Покраски» поправит потом")).clicked() {
            ed.paint_along_selected(lib);
        }
    });
}

/// Objects along the selected road(s): the rows on a road selected alone, and new rows of the
/// object chosen in the catalogue.
pub fn rows_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library) {
    let sels = ed.selection();
    if !sels.iter().any(|s| matches!(s, Sel::Spline(..))) {
        return;
    }
    section(ui, tr("Objects along the road", "Объекты вдоль дороги"));
    egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
        let name_of = |f: &str| lib.find(f).map(|a| a.name.clone()).unwrap_or_else(|| f.rsplit('\\').next().unwrap_or(f).to_string());
        // the rows already on a road selected alone
        if let (Some((k, id)), Some(doc)) = (ed.single_spline(), ed.doc.as_ref()) {
            let rows: Vec<(usize, omsi_map::SplineAttachment)> = crate::rows::rows_on(doc, k, id).into_iter().filter_map(|r| doc.tiles.get(&k).and_then(|t| t.tile.spline_attachments.get(r)).map(|a| (r, a.clone()))).collect();
            let mut edit: Option<(usize, omsi_map::SplineAttachment)> = None;
            let mut remove: Option<usize> = None;
            let new_type = ed.object_type.clone();
            for (r, a) in &rows {
                let mut b = a.clone();
                crate::ui::long_label(ui, &name_of(&a.file), egui::TextStyle::Body, Color32::WHITE);
                egui::Grid::new(("row", *r)).num_columns(2).spacing(Vec2::new(6.0, 4.0)).show(ui, |ui| {
                    prop_row(ui, tr("Every", "Шаг"), |ui| {
                        let mut i = b.interval;
                        if ui.add(egui::DragValue::new(&mut i).speed(0.1).range(0.5..=500.0).suffix(" m")).changed() {
                            crate::rows::set_interval(&mut b, i);
                        }
                    });
                    prop_row(ui, tr("Beside", "Сбоку"), |ui| {
                        ui.add(egui::DragValue::new(&mut b.offset[0]).speed(0.05).max_decimals(2).suffix(" m"));
                    });
                    prop_row(ui, tr("Height", "Высота"), |ui| {
                        ui.add(egui::DragValue::new(&mut b.offset[1]).speed(0.02).max_decimals(2).suffix(" m"));
                    });
                    prop_row(ui, tr("Turn", "Поворот"), |ui| {
                        ui.add(egui::DragValue::new(&mut b.rot[0]).speed(1.0).max_decimals(2).suffix("°"));
                    });
                    prop_row(ui, "", |ui| {
                        check(ui, &mut b.tilt, tr("Lean with the road", "Наклон вместе с дорогой")).on_hover_text(tr("Each object leans with the road's gradient and banking (bays and arrows on the road); off, it stands upright", "Каждый объект наклоняется по уклону и виражу дороги (карманы и стрелки на дороге); выключено — стоит прямо"));
                    });
                    // (the texts every object of the row shows: a street name, a line number)
                    prop_row(ui, tr("Texts", "Тексты"), |ui| {
                        ui.vertical(|ui| {
                            let mut drop = None;
                            for (i, t) in b.strings.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(t).desired_width(120.0));
                                    if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                                        drop = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = drop {
                                b.strings.remove(i);
                            }
                            if ui.small_button(icons::labelled(Icon::Add, tr("Text", "Текст"))).on_hover_text(tr("A text the row's objects show (@ starts a new line)", "Текст, который показывают объекты ряда (@ — новая строка)")).clicked() {
                                b.strings.push(String::new());
                            }
                        });
                    });
                });
                ui.horizontal(|ui| {
                    if let Some(t) = &new_type {
                        if !t.eq_ignore_ascii_case(&a.file) && ui.button(icons::labelled(Icon::Swap, &format!("{} «{}»", tr("Replace with", "Заменить на"), crate::ui::short(&name_of(t), 24)))).clicked() {
                            b.file = t.clone();
                        }
                    }
                    if ui.button(icons::labelled(Icon::Delete, tr("Remove", "Убрать"))).clicked() {
                        remove = Some(*r);
                    }
                });
                if b != *a {
                    edit = Some((*r, b));
                }
                ui.add_space(4.0);
            }
            if let Some(r) = remove {
                ed.remove_row(k, r);
            } else if let Some((r, b)) = edit {
                ed.edit_row(k, r, |a| *a = b);
            } else {
                ed.end_panel_step_if_idle(ui.ctx());
            }
            if !rows.is_empty() {
                ui.separator();
            }
        }
        // a new row
        match &ed.object_type {
            Some(t) => ui.label(format!("{} {}", tr("Object:", "Объект:"), crate::ui::short(&name_of(t), 40))),
            None => ui.label(RichText::new(tr("Pick an object (a lamp, a tree, a bollard) in the Objects tool's catalogue", "Выберите объект (фонарь, дерево, столбик) в каталоге инструмента «Объекты»")).small().color(TEXT_DIM)),
        };
        let p = &mut ed.row_params;
        egui::Grid::new("new row").num_columns(2).spacing(Vec2::new(6.0, 4.0)).show(ui, |ui| {
            prop_row(ui, tr("Every", "Шаг"), |ui| {
                ui.add(egui::DragValue::new(&mut p.interval).speed(0.1).range(0.5..=500.0).suffix(" m"));
            });
            prop_row(ui, tr("Beside", "Сбоку"), |ui| {
                ui.add(egui::DragValue::new(&mut p.offset).speed(0.05).suffix(" m")).on_hover_text(tr("Right of the road's direction; negative: left", "Справа по направлению дороги; меньше нуля: слева"));
            });
            prop_row(ui, tr("Free at start / end", "Отступ в начале / конце"), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut p.margin_start).speed(0.1).range(0.0..=1000.0).suffix(" m"));
                    ui.add(egui::DragValue::new(&mut p.margin_end).speed(0.1).range(0.0..=1000.0).suffix(" m"));
                });
            });
            prop_row(ui, tr("Height", "Высота"), |ui| {
                ui.add(egui::DragValue::new(&mut p.height).speed(0.02).suffix(" m"));
            });
            prop_row(ui, tr("Turn", "Поворот"), |ui| {
                ui.add(egui::DragValue::new(&mut p.turn).speed(1.0).range(-360.0..=360.0).suffix("°"));
            });
        });
        check(ui, &mut p.both, tr("Both sides", "С обеих сторон"));
        check(ui, &mut p.tilt, tr("Lean with the road", "Наклон вместе с дорогой"));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(ed.object_type.is_some(), egui::Button::new(icons::labelled(Icon::Add, tr("Place along the road", "Расставить вдоль дороги")))).clicked() {
                ed.place_rows();
            }
            if ed.selection().len() > 1 && ui.button(tr("Remove all rows", "Убрать все ряды")).clicked() {
                ed.remove_rows_of_selection();
            }
        });
    });
}

/// The Roads tool's options for roads that keep their height.
pub fn road_options(ui: &mut egui::Ui, ed: &mut Editor) {
    let r = check(ui, &mut ed.roundabout, tr("Roundabout: click the centre, then the ring", "Кольцо: клик в центр, потом по кольцу"));
    r.on_hover_text(tr("A one-way ring of the chosen road type (pick a one-way road); roads inside are cut back to it and joined with junctions", "Одностороннее кольцо выбранного типа дороги (выберите одностороннюю); дороги внутри обрезаются по кольцу и соединяются перекрёстками"));
    let r = check(ui, &mut ed.keep_height, tr("Keep the height (bridge, embankment)", "Держать высоту (мост, насыпь)"));
    r.on_hover_text(tr("An even gradient from end to end, the ground left as it is; the rise to the end can be typed while drawing. Pillars: select the road and put them along it as objects.", "Ровный уклон от начала до конца, земля не меняется; подъём к концу можно ввести при рисовании. Опоры: выберите дорогу и расставьте их вдоль неё как объекты."));
}

/// The Roads tool's bus pocket tool: with the box ticked a click on a road lays a pocket
/// there, again and again, until it is switched off.
pub fn pocket_options(ui: &mut egui::Ui, ed: &mut Editor) {
    let r = check(ui, &mut ed.bus_pocket, icons::labelled(Icon::Road, tr("Bus Pocket Tool", "Инструмент «Автобусный карман»")));
    r.on_hover_text(tr("Tick it, then press on a road: a bus pocket is laid there, on the kerb the road's own traffic keeps to. It stays on, pocket after pocket, until you untick it", "Включите и нажимайте на дорогу: там появится автобусный карман, со стороны бордюра, к которому прижимается движение дороги. Инструмент работает, пока вы его не выключите"));
    if !ed.bus_pocket {
        return;
    }
    egui::Frame::new().inner_margin(Margin { left: 18, ..Default::default() }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(tr("Full width over", "Полная ширина на"));
            ui.add(egui::DragValue::new(&mut ed.pocket_len).range(4.0..=80.0).speed(0.5).suffix(" m"));
        });
        ui.horizontal(|ui| {
            ui.label(tr("Bay width", "Ширина кармана"));
            if ed.pocket_width <= 0.0 {
                ed.pocket_width = 3.0;
            }
            ui.add(egui::DragValue::new(&mut ed.pocket_width).range(1.5..=6.0).speed(0.05).suffix(" m")).on_hover_text(tr("How much wider the road is along the bay, on the kerb its traffic keeps to; the other side stays as it is", "На сколько дорога шире у кармана, со стороны бордюра её полосы движения; другая сторона не меняется"));
        });
        check(ui, &mut ed.pocket_metres, tr("Tapers in metres", "Переходы в метрах")).on_hover_text(tr("Give the length of each taper in metres instead of a ratio", "Задавать длину каждого перехода в метрах, а не отношением"));
        ui.horizontal(|ui| {
            ui.label(tr("Taper in", "Вход"));
            if ed.pocket_metres {
                ui.add(egui::DragValue::new(&mut ed.pocket_in_m).range(12.0..=60.0).speed(0.5).suffix(" m"));
            } else {
                ui.add(egui::DragValue::new(&mut ed.pocket_taper_in).range(1.0..=12.0).speed(0.1).suffix(" :1")).on_hover_text(tr("Metres of road the bay takes to open per metre it widens (more: gentler)", "Сколько метров дороги уходит на раскрытие кармана на метр ширины (больше — положе)"));
            }
            if ed.pocket_alike {
                ed.pocket_taper_out = ed.pocket_taper_in;
                ed.pocket_out_m = ed.pocket_in_m;
            }
            ui.add_enabled_ui(!ed.pocket_alike, |ui| {
                ui.label(tr("out", "выход"));
                if ed.pocket_metres {
                    ui.add(egui::DragValue::new(&mut ed.pocket_out_m).range(12.0..=60.0).speed(0.5).suffix(" m"));
                } else {
                    ui.add(egui::DragValue::new(&mut ed.pocket_taper_out).range(1.0..=12.0).speed(0.1).suffix(" :1"));
                }
            });
        });
        check(ui, &mut ed.pocket_alike, tr("Both ends alike", "Оба конца одинаковые"));
        check(ui, &mut ed.pocket_fit, tr("Slide the bay to where it fits", "Сдвигать карман туда, где он помещается")).on_hover_text(tr("Near the end of a road or a junction there is no room for the taper at both ends: the bay is moved along the road to where there is", "У конца дороги или перекрёстка нет места для перехода с обеих сторон: карман сдвигается вдоль дороги туда, где место есть"));
        ui.label(RichText::new(tr("A bay always gets a taper at both ends; where one cannot be laid nothing is changed.", "Карман всегда получает переход с обеих сторон; если один уложить нельзя, ничего не меняется.")).small().color(TEXT_DIM));
        ui.label(RichText::new(tr("The stretch is cut out of the road and given the bay along its kerb, its ends tapering to the road's width. The road's own type is widened: nothing to pick.", "Участок вырезается из дороги и получает карман вдоль бордюра, концы сужаются до ширины дороги. Тип дороги расширяется сам: выбирать ничего не нужно.")).small().color(TEXT_DIM));
    });
}

/// Numbers for the road being drawn, in the view's corner: what is typed is kept to, what is
/// left empty follows the cursor (its value shown greyed).
pub fn road_entry(ctx: &egui::Context, ed: &mut Editor, view: egui::Rect, actions: &mut Vec<crate::ui::Action>) {
    if !ed.road_start_active() || ed.drawing_paths() || !view.is_positive() {
        return;
    }
    let facts = ed.road_plan_facts();
    egui::Area::new(egui::Id::new("road entry")).fixed_pos(view.left_bottom() + Vec2::new(10.0, -46.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::new().fill(Color32::from_rgba_unmultiplied(27, 28, 30, 235)).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(8, 5)).stroke(Stroke::new(1.0, Color32::from_gray(60))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let shown = |v: Option<f64>, unit: &str| v.map(|v| format!("{v:.1}{unit}")).unwrap_or_default();
                let (len, rad, head, rise) = match facts {
                    Some((l, r, h, z)) => (Some(l), Some(r.abs()), Some(h), Some(z)),
                    None => (None, None, None, None),
                };
                let mut fields: Vec<(usize, &str, String)> = vec![
                    (0, tr("Length", "Длина"), shown(len, " m")),
                    (1, tr("Radius", "Радиус"), if rad == Some(0.0) { tr("straight", "прямая").to_string() } else { shown(rad, " m") }),
                    (2, tr("Heading", "Курс"), shown(head, "°")),
                ];
                if ed.keep_height {
                    fields.push((3, tr("Rise", "Подъём"), shown(rise, " m")));
                }
                let mut enter = false;
                for (i, name, hint) in fields {
                    ui.label(RichText::new(name).small().color(TEXT_DIM));
                    let r = ui.add(egui::TextEdit::singleline(&mut ed.typed_text[i]).desired_width(58.0).hint_text(hint));
                    if r.changed() {
                        ed.replan = true;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        enter = true;
                    }
                }
                if ed.typed_text.iter().any(|t| !t.is_empty()) && ui.small_button(tr("Clear", "Сброс")).clicked() {
                    ed.typed_text = Default::default();
                    ed.replan = true;
                }
                ui.label(RichText::new(tr("Shift: straight · Enter: build", "Shift: прямая · Enter: построить")).small().color(TEXT_DIM));
                if enter {
                    actions.push(crate::ui::Action::BuildRoad);
                }
            });
        });
    });
}

/// The lights and crossings of one of openMapper's junctions (it is made again with them).
pub fn junction_section(ui: &mut egui::Ui, ed: &mut Editor, k: TileKey, id: i64) {
    let Some(doc) = ed.doc.as_ref() else { return };
    let Some(o) = doc.object(k, id) else { return };
    if !crate::editor::is_own_junction(&o.file) {
        return;
    }
    let file = o.file.clone();
    let arms = ed.junction_arms(k, id);
    let opts = match ed.junction_options.get(&id) {
        Some(o) => o.clone(),
        None => {
            let o = ed.content_dir.as_ref().map(|c| crate::junction::Options::read(&omsi_cfg::resolve_path(c, &file))).unwrap_or_default();
            ed.junction_options.insert(id, o.clone());
            o
        }
    };
    section(ui, tr("Junction", "Перекрёсток"));
    let mut new = opts.clone();
    let mut apply = false;
    egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
        if check(ui, &mut new.lights, tr("Traffic lights", "Светофоры")).changed() {
            apply = true;
        }
        if new.lights {
            if arms.len() < 3 {
                ui.label(RichText::new(tr("Lights need three roads or more", "Светофорам нужно не меньше трёх дорог")).small().color(TEXT_DIM));
            }
            // (the seconds are applied with the button: the plate is not made again while dragging)
            let tid = egui::Id::new(("junction greens", id));
            let mut g: (f64, f64) = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(tid, || (opts.main_green, opts.side_green)));
            egui::Grid::new(("greens", id)).num_columns(2).spacing(Vec2::new(6.0, 4.0)).show(ui, |ui| {
                prop_row(ui, tr("Main road green", "Зелёный главной"), |ui| {
                    ui.add(egui::DragValue::new(&mut g.0).speed(0.5).range(5.0..=120.0).suffix(" s"));
                });
                prop_row(ui, tr("Side roads green", "Зелёный второстепенных"), |ui| {
                    ui.add(egui::DragValue::new(&mut g.1).speed(0.5).range(5.0..=120.0).suffix(" s"));
                });
            });
            ui.data_mut(|d| d.insert_temp(tid, g));
            if (g.0 - opts.main_green).abs() > 1e-6 || (g.1 - opts.side_green).abs() > 1e-6 {
                if ui.button(tr("Apply", "Применить")).clicked() {
                    new.main_green = g.0;
                    new.side_green = g.1;
                    apply = true;
                }
            }
            ui.label(RichText::new(tr("The main road is the pair of roads most in line; red-yellow 2 s, yellow 3 s, 2 s all red between", "Главная: пара дорог, лежащих ближе всего к одной линии; красный с жёлтым 2 с, жёлтый 3 с, между фазами 2 с всем красный")).small().color(TEXT_DIM));
        }
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Zebra crossings", "Пешеходные переходы")).strong());
        for (i, out) in arms.iter().enumerate() {
            let mut on = new.crossing_at(*out);
            if check(ui, &mut on, format!("{} {} ({:.0}°)", tr("Road", "Дорога"), i + 1, out)).changed() {
                new.crossings.retain(|h| (crate::roads::norm_deg(h - out + 180.0) - 180.0).abs() >= 20.0);
                if on {
                    new.crossings.push(*out);
                }
                apply = true;
            }
        }
    });
    let rebuild = ui.button(icons::labelled(Icon::Junction, tr("Build it again", "Построить заново"))).on_hover_text(tr("Makes the plate, its kerbs and lanes again for the roads as they are now", "Заново строит площадку, бордюры и полосы под дороги, как они сейчас")).clicked();
    if (apply && new != opts) || rebuild {
        ed.set_junction_options(id, new);
    }
}

/// What an object takes its state from and what takes its state from it (`[varparent]`),
/// as OMSI 2's editor links them: a signal head shows one light of its crossing's program,
/// a stop's display its stop. The parent is picked with a click in the view.
pub fn link_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library, k: TileKey, id: i64, info: Option<&crate::ui::ObjInfo>, parent_lights: Option<&(String, Vec<String>)>) {
    // (what is linked to what is read from the whole map once and kept while it does not
    // change: this panel is drawn on every frame of a drag, and walking a map of a hundred
    // thousand things for it was most of the frame)
    let parent_id = ed.doc.as_ref().and_then(|d| crate::selection::any_object(d, k, id)).and_then(|o| o.var_parent);
    let (link_parent, linked) = ed.links_of(id, parent_id);
    let Some(doc) = ed.doc.as_ref() else { return };
    let Some(me) = crate::selection::any_object(doc, k, id) else { return };
    let name_of = |file: &str| lib.find(file).map(|a| a.name.clone()).unwrap_or_else(|| file.rsplit('\\').next().unwrap_or(file).to_string());
    let parent = parent_id.map(|pid| {
        let found = link_parent.and_then(|(pk, _)| crate::selection::any_object(doc, pk, pid).map(|o| (pk, name_of(&o.file), o.file.clone())));
        (pid, found)
    });
    let light: Option<usize> = me.extra.first().and_then(|l| l.trim().parse().ok());
    let head = info.map(|i| i.light_head).unwrap_or(false);
    let own_lights: &[String] = info.map(|i| i.lights.as_slice()).unwrap_or(&[]);
    let children: Vec<(TileKey, i64, String, Option<usize>)> = linked.into_iter().filter_map(|(ck, cid)| crate::selection::any_object(doc, ck, cid).map(|o| (ck, cid, name_of(&o.file), o.extra.first().and_then(|l| l.trim().parse().ok())))).collect();
    let picking = ed.link_pick == Some((k, id));
    let mut go: Option<Sel> = None;
    let mut set_parent: Option<Option<i64>> = None;
    let mut set_light: Option<usize> = None;
    let mut pick = false;
    let count = parent.is_some() as usize + children.len();
    egui::Frame::new().inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        egui::CollapsingHeader::new(RichText::new(format!("{} ({count})", tr("Linked objects", "Связанные объекты"))).strong()).id_salt("varparent").default_open(head || count > 0).show(ui, |ui| {
            ui.label(RichText::new(tr("An object can take its state from another ([varparent]): a signal head shows a light of its crossing's program, a stop's display its stop.", "Объект может брать состояние от другого ([varparent]): светофор показывает сигнал программы своего перекрёстка, табло остановки — свою остановку.")).small().color(TEXT_DIM));
            match &parent {
                Some((pid, Some((pk, name, _)))) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(tr("Takes its state from", "Берёт состояние от")).color(TEXT_DIM));
                        if ui.link(name).on_hover_text(format!("{} {pid}", tr("Select it: IDCode", "Выбрать его: IDCode"))).clicked() {
                            go = Some(Sel::Object(*pk, *pid));
                        }
                    });
                }
                Some((pid, None)) => {
                    ui.label(RichText::new(format!("{} {pid}", tr("Takes its state from an object that is gone: IDCode", "Берёт состояние от исчезнувшего объекта: IDCode"))).color(Color32::from_rgb(230, 170, 100)));
                }
                None => {
                    ui.label(RichText::new(tr("Takes its state from nothing", "Ни от чего не берёт состояние")).color(TEXT_DIM));
                }
            }
            // a signal head: which light of its crossing's program it shows
            if head {
                match (parent_lights, &parent) {
                    (Some((_, lights)), Some(_)) if !lights.is_empty() => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr("Light", "Сигнал")).color(TEXT_DIM));
                            let shown = |i: usize| match lights.get(i) {
                                Some(n) if !n.trim().is_empty() => format!("{i}: {n}"),
                                _ => format!("{i}"),
                            };
                            egui::ComboBox::from_id_salt("light index").selected_text(light.map(shown).unwrap_or_else(|| "-".into())).width(160.0).show_ui(ui, |ui| {
                                for i in 0..lights.len() {
                                    if ui.selectable_label(light == Some(i), shown(i)).clicked() && light != Some(i) {
                                        set_light = Some(i);
                                    }
                                }
                            });
                        });
                        if light.map(|l| l >= lights.len()).unwrap_or(true) {
                            ui.label(RichText::new(tr("Its light is not one of the crossing's: it stays dark", "Его сигнала нет у перекрёстка: он не горит")).small().color(Color32::from_rgb(230, 170, 100)));
                        }
                    }
                    (Some(_), Some(_)) => {
                        ui.label(RichText::new(tr("That object runs no traffic lights: link the head to a crossing with a light program", "Тот объект не управляет светофорами: свяжите головку с перекрёстком, где есть программа")).small().color(Color32::from_rgb(230, 170, 100)));
                    }
                    (_, None) => {
                        ui.label(RichText::new(tr("A signal head: link it to its crossing to have it light", "Головка светофора: свяжите её с перекрёстком, чтобы она горела")).small().color(Color32::from_rgb(230, 170, 100)));
                    }
                    _ => {}
                }
            }
            ui.horizontal_wrapped(|ui| {
                let label = if picking { tr("Click the object in the view… (Esc)", "Щёлкните объект в виде… (Esc)") } else if parent.is_some() { tr("Link to another…", "Связать с другим…") } else { tr("Link to an object…", "Связать с объектом…") };
                if ui.add(egui::Button::new(icons::labelled(Icon::Traffic, label)).selected(picking)).on_hover_text(tr("Then click the object it is to take its state from (a crossing for a signal head)", "Затем щёлкните объект, от которого брать состояние (перекрёсток для светофора)")).clicked() {
                    pick = true;
                }
                if parent.is_some() && ui.button(icons::labelled(Icon::Delete, tr("Unlink", "Отвязать"))).clicked() {
                    set_parent = Some(None);
                }
            });
            if !children.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{} {}", tr("Taking their state from it:", "Берут состояние от него:"), children.len())).color(TEXT_DIM));
                for (ck, cid, name, l) in children.iter().take(40) {
                    ui.horizontal(|ui| {
                        if ui.link(name).on_hover_text(format!("{} {cid}", tr("Select it: IDCode", "Выбрать его: IDCode"))).clicked() {
                            go = Some(Sel::Object(*ck, *cid));
                        }
                        if let (Some(l), false) = (l, own_lights.is_empty()) {
                            let n = own_lights.get(*l).map(|n| n.as_str()).unwrap_or("?");
                            ui.label(RichText::new(format!("{} {l}: {n}", tr("light", "сигнал"))).small().color(TEXT_DIM));
                        }
                    });
                }
                if children.len() > 40 {
                    ui.label(RichText::new(format!("… {}", children.len() - 40)).color(TEXT_DIM));
                }
            }
        });
    });
    if let Some(s) = go {
        ed.select(Some(s));
    } else if let Some(p) = set_parent {
        ed.set_var_parent(k, id, p);
    } else if let Some(l) = set_light {
        ed.set_light_index(k, id, l);
    } else if pick && picking {
        ed.link_pick = None;
    } else if pick {
        ed.link_pick = Some((k, id));
        ed.toast(tr("Click the object it takes its state from", "Щёлкните объект, от которого брать состояние"));
    }
}

/// A road's free ends: put a junction piece onto one (its helper made for this road type meets
/// the end), the piece picked from those that fit, the road type's preferred one first.
pub fn end_junction_section(ui: &mut egui::Ui, ed: &mut Editor, lib: &Library, actions: &mut Vec<crate::ui::Action>) {
    let Some((k, id)) = ed.single_spline() else { return };
    let Some(s) = ed.doc.as_ref().and_then(|d| d.tiles.get(&k)).and_then(|t| t.tile.splines.iter().find(|s| s.id == id && !s.deleted)) else { return };
    let (free_start, free_end, file) = (s.prev_id == 0, s.next_id == 0, s.file.clone());
    if !free_start && !free_end {
        return;
    }
    section(ui, tr("Junction on an end", "Перекрёсток на конце дороги"));
    egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
        let (pieces, fits) = lib.junction_pieces(&file);
        if pieces.is_empty() {
            ui.label(RichText::new(tr("The installation has no junction pieces with places for roads.", "В установке нет перекрёстков с местами для дорог.")).small().color(TEXT_DIM));
            return;
        }
        let preferred = ed.road_styles.piece(&file).map(str::to_string);
        let cur = ed.junction_piece.clone().filter(|p| pieces.iter().any(|a| a.rel.eq_ignore_ascii_case(p))).or_else(|| preferred.filter(|p| pieces.iter().any(|a| a.rel.eq_ignore_ascii_case(p)))).or_else(|| pieces.first().map(|a| a.rel.clone()));
        let mut pick = cur.clone();
        let name = cur.as_deref().and_then(|c| lib.find(c)).map(|a| a.name.clone()).unwrap_or_default();
        egui::ComboBox::from_id_salt("junction piece").selected_text(name).width(ui.available_width() - 8.0).height(300.0).show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut ed.junction_search).hint_text(tr("Search", "Поиск")));
            let q = ed.junction_search.to_lowercase();
            for a in pieces.iter().filter(|a| q.is_empty() || a.haystack.contains(&q)).take(200) {
                let on = cur.as_deref().map(|c| c.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
                if ui.selectable_label(on, format!("{}  ·  {}", a.name, a.dir)).clicked() {
                    pick = Some(a.rel.clone());
                }
            }
        });
        ed.junction_piece = pick.clone();
        if !fits {
            ui.label(RichText::new(tr("None of the pieces is made for this road type: its first place is used.", "Ни один перекрёсток не сделан под этот тип дороги: берётся его первое место.")).small().color(TEXT_DIM));
        }
        let Some(piece) = pick else { return };
        ui.horizontal(|ui| {
            if free_start && ui.button(tr("At the start", "У начала")).clicked() {
                actions.push(crate::ui::Action::JunctionAtEnd(k, id, false, piece.clone()));
            }
            if free_end && ui.button(tr("At the end", "У конца")).clicked() {
                actions.push(crate::ui::Action::JunctionAtEnd(k, id, true, piece.clone()));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buses_only_shuts_the_path_to_cars_once() {
        let mut rules = vec![omsi_map::MapRule { path_index: 1, kind: "bus".into(), value: 0.0, extra: 0.0, kill: false }];
        close_path_to_cars(&mut rules, 1);
        close_path_to_cars(&mut rules, 1);
        assert_eq!(rules.iter().filter(|r| r.kind == "no_cars" && r.path_index == 1).count(), 1);
        close_path_to_cars(&mut rules, 2);
        assert!(rules.iter().any(|r| r.kind == "no_cars" && r.path_index == 2));
    }
}
