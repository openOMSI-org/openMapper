//! The menu a right click in the view opens (the Select tool, and Game logic on one of its
//! things): copy, paste there, duplicate, delete, group and ungroup, and the like.

use egui::{Color32, RichText};
use glam::DVec3;

use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::ui::Action;

/// Where the menu is open: its corner on the screen and the ground point clicked.
#[derive(Clone, Copy)]
pub struct ViewMenu {
    pub at: egui::Pos2,
    pub ground: Option<DVec3>,
}

fn item(ui: &mut egui::Ui, icon: Icon, text: &str, keys: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(icons::labelled(icon, text)).shortcut_text(keys).min_size(egui::vec2(230.0, 0.0))).clicked()
}

/// The menu, while it is open. It closes on a choice, a click elsewhere or Esc.
pub fn show(ctx: &egui::Context, menu: &mut Option<ViewMenu>, ed: &mut Editor, lib: &Library, actions: &mut Vec<Action>) {
    let Some(m) = *menu else { return };
    let sels = ed.selection();
    let n = sels.len();
    let group = ed.selected_group();
    let mut done = false;
    let r = egui::Area::new(egui::Id::new("view menu")).order(egui::Order::Foreground).fixed_pos(m.at).constrain(true).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(230.0);
            // what it is about
            let title = match (group, n) {
                (Some(g), _) => format!("{} «{}» · {n}", tr("Group", "Группа"), ed.groups()[g].name),
                (None, 0) => tr("Nothing selected", "Ничего не выделено").to_string(),
                (None, 1) => {
                    let file = ed.doc.as_ref().and_then(|d| match sels[0] {
                        crate::doc::Sel::Object(k, id) => crate::selection::any_object(d, k, id).map(|o| o.file.clone()),
                        crate::doc::Sel::Spline(k, id) => d.spline(k, id).map(|s| s.file.clone()),
                        crate::doc::Sel::Attached(k, id) => d.attachment(k, id).map(|a| a.file.clone()),
                    });
                    file.map(|f| lib.find(&f).map(|a| a.name.clone()).unwrap_or_else(|| f.rsplit('\\').next().unwrap_or(&f).to_string())).unwrap_or_default()
                }
                (None, _) => format!("{} {n}", tr("Selected:", "Выбрано:")),
            };
            ui.label(RichText::new(title).strong().color(Color32::from_gray(230)));
            ui.separator();
            let any = n > 0;
            if item(ui, Icon::Copy, tr("Copy", "Копировать"), "Ctrl+C", any) {
                actions.push(Action::Copy);
                done = true;
            }
            if item(ui, Icon::Copy, tr("Cut", "Вырезать"), "Ctrl+X", any) {
                actions.push(Action::Cut);
                done = true;
            }
            if item(ui, Icon::Paste, tr("Paste here", "Вставить сюда"), "Ctrl+V", ed.has_clip()) {
                actions.push(Action::PasteAt(m.ground));
                done = true;
            }
            if item(ui, Icon::Add, tr("Duplicate", "Дублировать"), "Ctrl+D", any) {
                actions.push(Action::Duplicate);
                done = true;
            }
            if item(ui, Icon::Delete, tr("Delete", "Удалить"), "Del", any) {
                ed.delete_selected();
                done = true;
            }
            // a road under the click: cut there; one piece selected: joined to the next again
            let road = m.ground.and_then(|g| ed.road_at(g.truncate()));
            let joinable = match (n, sels.first()) {
                (1, Some(crate::doc::Sel::Spline(k, id))) => ed.doc.as_ref().and_then(|d| d.spline(*k, *id)).filter(|s| s.next_id != 0).map(|_| (*k, *id)),
                _ => None,
            };
            if road.is_some() || joinable.is_some() {
                ui.separator();
            }
            if road.is_some() && item(ui, Icon::Road, tr("Cut the road here", "Разрезать дорогу здесь"), "", true) {
                if let Some(g) = m.ground {
                    ed.cut_road_at(g.truncate());
                }
                done = true;
            }
            if let Some((k, id)) = joinable {
                if item(ui, Icon::Route, tr("Join with the next piece", "Соединить со следующим куском"), "", true) {
                    ed.join_with_next(k, id);
                    done = true;
                }
            }
            ui.separator();
            if item(ui, Icon::Layers, tr("Group", "Сгруппировать"), "Ctrl+G", n >= 2 && group.is_none()) {
                ed.group_selection();
                done = true;
            }
            if item(ui, Icon::Layers, tr("Ungroup", "Разгруппировать"), "Ctrl+Shift+G", ed.selection_grouped()) {
                ed.ungroup_selection();
                done = true;
            }
            if item(ui, Icon::SelectAll, tr("Select all like it", "Выбрать все такие"), "", n == 1) {
                let c = ed.select_alike(sels[0]);
                ed.toast(format!("{} {c}", tr("Selected:", "Выбрано:")));
                done = true;
            }
            if item(ui, Icon::Focus, tr("Go to", "Перейти"), "F", any) {
                actions.push(Action::FocusSelection);
                done = true;
            }
            let one_object = matches!(ed.selected, Some(crate::doc::Sel::Object(..))) && sels.len() == 1;
            if item(ui, Icon::Route, tr("Crossing editor…", "Редактор перекрёстка…"), "", one_object) {
                actions.push(Action::CrossingPaths);
                done = true;
            }
            let splines = sels.iter().any(|s| matches!(s, crate::doc::Sel::Spline(..)));
            if item(ui, Icon::Add, tr("From the selected splines", "По выделенным сплайнам"), "P", splines) {
                actions.push(Action::Mesh(crate::ui_mesh::MeshAction::PolygonFromSplines));
                done = true;
            }
            if item(ui, Icon::Add, tr("Draw a polygon", "Нарисовать многоугольник"), "Shift+P", true) {
                actions.push(Action::Mesh(crate::ui_mesh::MeshAction::PolygonDraw));
                done = true;
            }
            if item(ui, Icon::Import, tr("Export…", "Экспорт…"), "", any) {
                actions.push(Action::Mesh(crate::ui_mesh::MeshAction::ExportModel));
                done = true;
            }
            if item(ui, Icon::Save, tr("Keep for other maps…", "Сохранить для других карт…"), "", any) {
                actions.push(Action::SavedGroups);
                done = true;
            }
            ui.separator();
            if item(ui, Icon::SelectAll, tr("Select all", "Выделить всё"), "Ctrl+A", true) {
                actions.push(Action::SelectAll);
                done = true;
            }
            if item(ui, Icon::Select, tr("Deselect", "Снять выделение"), "Esc", any) {
                ed.select(None);
                done = true;
            }
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !r.response.contains_pointer();
    if done || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        *menu = None;
    }
}
