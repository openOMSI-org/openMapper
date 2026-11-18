//! The Trolleybus wire window (Map menu): the overhead wire openOMSI's trolleybuses run under
//! (`trolley.txt`). Wire is laid over the kerb lane of the selected roads, over chosen lanes of
//! a junction, or read off the map's wire splines; the sections, the switches at its frogs
//! (how each is set and where it starts: straight, to the branch or at random) and the
//! insulators between sections are set here, spans and switches picked in the view. What
//! shows the wire on the map comes from a kit of the map maker's (`trolley_kit`), made and
//! changed in its own window.

use egui::{RichText, Vec2};
use glam::DVec3;

use crate::doc::Sel;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::trolley::{self, Initial, Lay, Mode, Section};
use crate::trolley_kit::{KitDef, KitRef, PartDef, Piece, Scope, Shape, PIECES, SHAPES};
use crate::theme::pal;
use crate::ui::{check, prop_row, Action};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    #[default]
    Lay,
    Sections,
    Spans,
    Switches,
}

/// What needs the models of the roads and objects: done after the window, with the assets.
enum Lay2 {
    /// A span drawn by hand: its points (at the wire's height).
    Drawn(Vec<DVec3>),
    Roads(Vec<Sel>),
    Lanes(Vec<usize>),
    WireSplines,
}

#[derive(Default)]
pub struct TrolleyWindow {
    pub open: bool,
    tab: Tab,
    lay: Option<Lay>,
    /// The street lanes of the one object selected (a junction), read once it is selected
    /// (None inside: not read yet).
    lanes: Option<(Sel, Option<Vec<(usize, Vec<DVec3>)>>)>,
    chosen: Vec<usize>,
    /// The kit new wire is shown with (its id); none: the wire for the game only.
    kit: Option<String>,
    /// Leave out the kit's wire splines / masts.
    no_wires: bool,
    no_masts: bool,
    /// The kits the map can use (read when the window opens, again after a change).
    kits: Option<Vec<(KitRef, KitDef)>>,
    edit: Option<KitEdit>,
    /// A kit to open in its window once the map is there (OPENMAPPER_SETTINGS=kit:<id>).
    pending_kit: Option<String>,
}

/// A kit being made or changed in its window.
struct KitEdit {
    r: Option<KitRef>,
    scope: Scope,
    def: KitDef,
    saved: KitDef,
    piece: Piece,
    part: Option<usize>,
}

fn way_name(w: &str) -> &'static str {
    match w {
        "branch" => tr("branch", "ответвление"),
        "left" => tr("left", "налево"),
        "right" => tr("right", "направо"),
        _ => tr("straight", "прямо"),
    }
}

impl TrolleyWindow {
    /// Open the window, and a kit's window on the kit `id` (`map:<name>`, `shared:<name>`).
    pub fn edit_kit(&mut self, id: &str) {
        self.open = true;
        self.kit = Some(id.to_string());
        self.pending_kit = Some(id.to_string());
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: Option<&crate::install::Roots>, mut assets: Option<&mut crate::assets::Assets>, actions: &mut Vec<Action>) {
        ed.wire.show = self.open;
        if !self.open {
            ed.wire.picking = false;
            ed.wire.preview.clear();
            return;
        }
        let Some(doc) = ed.doc.as_ref() else { return };
        if let (Some(id), Some(roots)) = (self.pending_kit.take(), roots) {
            if let (Some(r), Some(k)) = (KitRef::of_id(&id), crate::trolley_kit::load(roots, &doc.dir, &id)) {
                self.edit = Some(KitEdit { scope: r.scope, r: Some(r), saved: k.clone(), def: k, piece: Piece::Mast, part: Some(1) });
            }
        }
        let lay = self.lay.get_or_insert_with(|| Lay { left_hand: doc.global.left_hand_traffic, ..Default::default() });
        let before = doc.traffic.trolley.clone();
        let mut wire = before.clone().unwrap_or_default();
        let mut exists = before.is_some();
        // a click in the view picks a switch (at a frog) or else a span
        // (a span being drawn: a click adds a point, on a span's end if it is near one)
        let lift = |c: DVec3| end_or(&wire, c, lay.height);
        ed.wire.next = ed.wire.drawing.as_ref().and(ed.cursor).filter(|_| ed.cursor_in_view).map(lift);
        if let (Some(d), Some(c)) = (ed.wire.drawing.as_mut(), ed.wire.click) {
            d.push(lift(c));
            ed.wire.click = None;
        }
        if let Some(c) = ed.wire.click.take() {
            let frog = trolley::joints(&wire).into_iter().filter(|j| j.ends.len() == 3 && j.pos.truncate().distance(c.truncate()) < 4.0).find_map(|j| trolley::switch_at(&wire, j.pos));
            if let Some(i) = frog {
                ed.wire.switch = Some(i);
                self.tab = Tab::Switches;
            } else if let Some(i) = trolley::span_near(&wire, c.truncate(), 4.0) {
                ed.wire.span = Some(i);
                self.tab = Tab::Spans;
            }
        }
        if ed.wire.span.is_some_and(|i| i >= wire.spans.len()) {
            ed.wire.span = None;
        }
        if ed.wire.switch.is_some_and(|i| i >= wire.switches.len()) {
            ed.wire.switch = None;
        }
        // (the one object selected: its street lanes to lay the wire over)
        let sel = ed.selection();
        let roads: Vec<Sel> = sel.iter().copied().filter(|s| matches!(s, Sel::Spline(..))).collect();
        let object = match sel.as_slice() {
            [s @ Sel::Object(..)] => Some(*s),
            _ => None,
        };
        if self.lanes.as_ref().map(|l| l.0) != object {
            self.chosen.clear();
            self.lanes = object.map(|o| (o, None));
        }
        let mut lay_req: Option<Lay2> = None;
        let mut remove_all = false;
        let mut preview: Vec<Vec<DVec3>> = Vec::new();
        let mut open = self.open;
        let picking = &mut ed.wire.picking;
        let drawing = &mut ed.wire.drawing;
        let (span_sel, switch_sel) = (&mut ed.wire.span, &mut ed.wire.switch);
        egui::Window::new(icons::labelled(Icon::Bus, tr("Trolleybus wire", "Контактная сеть троллейбуса"))).id(egui::Id::new("trolley wire")).open(&mut open).default_pos([330.0, 90.0]).default_size([460.0, 560.0]).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("The overhead wire openOMSI's trolleybuses run under (trolley.txt beside the map; OMSI 2 ignores it). Without it the game reads the map's wire splines.", "Контактная сеть для троллейбусов openOMSI (trolley.txt рядом с картой; OMSI 2 его не читает). Без него игра берёт провода из сплайнов карты.")).small().color(pal().text_dim));
            if !exists {
                ui.add_space(6.0);
                if ui.button(icons::labelled(Icon::Add, tr("Start a wire for this map", "Начать контактную сеть карты"))).clicked() {
                    exists = true;
                    wire.sections.push(Section::new(1));
                }
                return;
            }
            ui.horizontal(|ui| {
                for (t, name) in [(Tab::Lay, tr("Lay", "Укладка")), (Tab::Sections, tr("Sections", "Секции")), (Tab::Spans, tr("Spans", "Пролёты")), (Tab::Switches, tr("Switches", "Стрелки"))] {
                    if ui.selectable_label(self.tab == t, name).clicked() {
                        self.tab = t;
                    }
                }
                ui.separator();
                let b = egui::Button::new(icons::labelled(Icon::Select, tr("Pick in the view", "Выбор в окне вида"))).selected(*picking);
                if ui.add(b).on_hover_text(tr("A click in the view picks a span or a switch (yellow dot)", "Клик в окне вида выбирает пролёт или стрелку (жёлтая точка)")).clicked() {
                    *picking = !*picking;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| match self.tab {
                Tab::Lay => {
                    egui::Grid::new("trolley lay").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                        prop_row(ui, tr("Height", "Высота"), |ui| {
                            ui.add(egui::DragValue::new(&mut lay.height).range(3.5..=9.0).speed(0.05).suffix(" m")).on_hover_text(tr("The wires over the road surface", "Провода над покрытием дороги"));
                        });
                        prop_row(ui, tr("Lanes", "Полосы"), |ui| {
                            let names = [tr("the kerb lane each way", "крайняя полоса в обе стороны"), tr("the way the road runs", "по направлению дороги"), tr("against the road's way", "против направления дороги")];
                            egui::ComboBox::from_id_salt("trolley ways").selected_text(names[lay.ways as usize]).show_ui(ui, |ui| {
                                for (i, n) in names.iter().enumerate() {
                                    ui.selectable_value(&mut lay.ways, i as u8, *n);
                                }
                            });
                        });
                        prop_row(ui, tr("Section", "Секция"), |ui| section_combo(ui, "lay section", &wire.sections, &mut lay.section));
                        prop_row(ui, tr("Kit", "Комплект"), |ui| {
                            let kits = self.kits.get_or_insert_with(|| roots.map(|r| crate::trolley_kit::list(r, &doc.dir)).unwrap_or_default());
                            let name = |id: &Option<String>| match id.as_ref().and_then(|i| kits.iter().find(|k| &k.0.id() == i)) {
                                Some((r, k)) => format!("{}{}", k.name, if r.scope == Scope::Shared { tr(" (every map)", " (все карты)") } else { "" }),
                                None => tr("none: the wire for the game only", "нет: провод только для игры").to_string(),
                            };
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt("trolley kit").selected_text(name(&self.kit)).width(200.0).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.kit, None, name(&None));
                                        for (r, k) in kits.iter() {
                                            let id = Some(r.id());
                                            let n = format!("{}{}", k.name, if r.scope == Scope::Shared { tr(" (every map)", " (все карты)") } else { "" });
                                            ui.selectable_value(&mut self.kit, id, n);
                                        }
                                    });
                                    if ui.small_button(icons::labelled(Icon::Add, "")).on_hover_text(tr("A new kit", "Новый комплект")).clicked() {
                                        let def = KitDef { name: tr("My wire", "Мой провод").into(), ..KitDef::default() };
                                        self.edit = Some(KitEdit { r: None, scope: Scope::Map, saved: def.clone(), def, piece: Piece::Mast, part: None });
                                    }
                                    let chosen = self.kit.as_ref().and_then(|i| kits.iter().find(|k| &k.0.id() == i)).cloned();
                                    if let Some((r, k)) = chosen {
                                        if ui.small_button(icons::labelled(Icon::Settings, "")).on_hover_text(tr("Change this kit", "Изменить этот комплект")).clicked() {
                                            self.edit = Some(KitEdit { scope: r.scope, r: Some(r), saved: k.clone(), def: k, piece: Piece::Mast, part: None });
                                        }
                                    }
                                });
                                let chosen = self.kit.as_ref().and_then(|i| kits.iter().find(|k| &k.0.id() == i)).map(|k| k.1.clone());
                                match chosen {
                                    Some(k) => {
                                        lay.height = k.height;
                                        let mut wires = !self.no_wires;
                                        check(ui, &mut wires, tr("its wire", "его провод"));
                                        self.no_wires = !wires;
                                        let has_mast = !k.mesh(Piece::Mast).is_empty();
                                        let mut masts = !self.no_masts && has_mast;
                                        ui.add_enabled_ui(has_mast, |ui| check(ui, &mut masts, &format!("{} {} m", tr("its masts, every", "его опоры, через каждые"), k.step))).response.on_disabled_hover_text(tr("The kit has no mast: give it one in the kit's window", "У комплекта нет опоры: задайте её в окне комплекта"));
                                        if has_mast {
                                            self.no_masts = !masts;
                                        }
                                        ui.label(RichText::new(tr("Frogs and section insulators are put where the wire has them, as the kit makes them.", "Стрелки и секционные изоляторы ставятся там, где они есть в сети, такими, как их делает комплект.")).small().color(pal().text_dim));
                                    }
                                    None => {
                                        ui.label(RichText::new(tr("Nothing is laid on the map: make a kit (+) to say what the wire, its masts, frogs and insulators look like.", "На карту ничего не кладётся: сделайте комплект (+), чтобы задать, как выглядят провод, опоры, стрелки и изоляторы.")).small().color(pal().text_dim));
                                    }
                                }
                            });
                        });
                    });
                    ui.add_space(6.0);
                    let n = roads.len();
                    if ui.add_enabled(n > 0, egui::Button::new(icons::labelled(Icon::Road, &format!("{} ({n})", tr("Over the selected roads", "Над выбранными дорогами"))))).on_disabled_hover_text(tr("Select roads in the view first (double-click: a whole road)", "Сначала выберите дороги в окне вида (двойной клик: вся дорога)")).clicked() {
                        lay_req = Some(Lay2::Roads(roads.clone()));
                    }
                    if let Some((_, Some(lanes))) = &self.lanes {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("The selected object's lanes (a junction): tick those the wire goes over. Three spans meeting make a frog.", "Полосы выбранного объекта (перекрёстка): отметьте, над какими пройдёт провод. Три сходящихся пролёта — стрелка.")).small().color(pal().text_dim));
                        if lanes.is_empty() {
                            ui.label(RichText::new(tr("It has no street lanes.", "У него нет полос для машин.")).color(pal().text_dim));
                        }
                        ui.horizontal_wrapped(|ui| {
                            for (idx, pts) in lanes {
                                let mut on = self.chosen.contains(idx);
                                let r = check(ui, &mut on, format!("{} {idx}", tr("lane", "полоса")));
                                if r.changed() {
                                    if on { self.chosen.push(*idx) } else { self.chosen.retain(|x| x != idx) }
                                }
                                if r.hovered() || on {
                                    preview.push(pts.clone());
                                }
                            }
                        });
                        if ui.add_enabled(!self.chosen.is_empty(), egui::Button::new(icons::labelled(Icon::Junction, tr("Over the ticked lanes", "Над отмеченными полосами")))).clicked() {
                            lay_req = Some(Lay2::Lanes(self.chosen.clone()));
                        }
                    } else {
                        ui.label(RichText::new(tr("Select one junction to lay the wire over its lanes.", "Выберите один перекрёсток, чтобы проложить провод над его полосами.")).small().color(pal().text_dim));
                    }
                    ui.add_space(6.0);
                    match drawing.as_ref().map(|d| d.len()) {
                        None => {
                            if ui.button(icons::labelled(Icon::Line, tr("Draw a span by hand", "Нарисовать пролёт вручную"))).on_hover_text(tr("Click its points in the view; started or ended on a span's end it joins it, on a span's middle it branches off there (a frog)", "Кликайте его точки в окне вида; начатый или законченный на конце пролёта присоединяется к нему, на середине — ответвляется (стрелка)")).clicked() {
                                *drawing = Some(Vec::new());
                                *picking = false;
                            }
                        }
                        Some(n) => {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{n} {}", tr("points: click the next in the view", "точек: кликните следующую в окне вида"))).color(pal().accent));
                                if ui.add_enabled(n >= 2, egui::Button::new(tr("Finish", "Готово"))).clicked() {
                                    lay_req = drawing.take().map(Lay2::Drawn);
                                }
                                if ui.button(tr("Cancel", "Отмена")).clicked() {
                                    *drawing = None;
                                }
                            });
                        }
                    }
                    if ui.button(icons::labelled(Icon::Import, tr("Read the map's wire splines", "Взять провода из сплайнов карты"))).on_hover_text(tr("Splines drawn as a pair of wires (trolleybus maps made for OMSI 2): a span along each, as the game reads them", "Сплайны в виде пары проводов (троллейбусные карты для OMSI 2): пролёт вдоль каждого, как их читает игра")).clicked() {
                        lay_req = Some(Lay2::WireSplines);
                    }
                    ui.add_space(10.0);
                    ui.label(RichText::new(tr("Network", "Сеть")).strong());
                    egui::Grid::new("trolley net").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                        prop_row(ui, tr("Voltage", "Напряжение"), |ui| {
                            ui.add(egui::DragValue::new(&mut wire.voltage).range(100.0..=3000.0).speed(5.0).suffix(" V"));
                        });
                        prop_row(ui, tr("Wire spacing", "Между проводами"), |ui| {
                            ui.add(egui::DragValue::new(&mut wire.spacing).range(0.2..=2.0).speed(0.01).suffix(" m"));
                        });
                        prop_row(ui, tr("Roughness", "Неровность"), |ui| {
                            ui.add(egui::DragValue::new(&mut wire.roughness).range(0.0..=0.05).speed(0.0001).max_decimals(4).suffix(" m")).on_hover_text(tr("How uneven the contact wire is: worn or icy wire 0.003 and more", "Неровность провода: изношенный или обледенелый — 0.003 и больше"));
                        });
                    });
                    ui.add_space(10.0);
                    if ui.button(icons::labelled(Icon::Delete, tr("Delete the whole wire", "Удалить всю контактную сеть"))).on_hover_text(tr("trolley.txt goes with the next save; the game reads the map's wire splines again", "trolley.txt будет удалён при сохранении; игра снова возьмёт провода из сплайнов")).clicked() {
                        remove_all = true;
                    }
                }
                Tab::Sections => {
                    ui.label(RichText::new(tr("Stretches fed by one substation. Where two meet there is a dead stretch (an insulator).", "Участки от одной подстанции. Где они встречаются — мёртвый участок (изолятор).")).small().color(pal().text_dim));
                    let mut drop = None;
                    for (i, c) in wire.sections.iter_mut().enumerate() {
                        let used = wire.spans.iter().filter(|s| s.section == c.id).count();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}", c.id)).strong());
                            ui.add(egui::TextEdit::singleline(&mut c.name).hint_text(tr("name", "название")).desired_width(110.0));
                            check(ui, &mut c.live, tr("live", "под током"));
                            ui.add(egui::DragValue::new(&mut c.feeder_ohm).range(0.0..=5.0).speed(0.005).max_decimals(3).suffix(" Ω")).on_hover_text(tr("The feeder's resistance: the voltage drops with the current", "Сопротивление фидера: напряжение падает с током"));
                            ui.add(egui::DragValue::new(&mut c.regen_kw).range(0.0..=2000.0).speed(1.0).suffix(" kW")).on_hover_text(tr("Power the feeder takes back from a braking bus", "Мощность, которую фидер принимает от тормозящего троллейбуса"));
                            ui.label(RichText::new(format!("{used} {}", tr("spans", "пролётов"))).small().color(pal().text_dim));
                            if used == 0 && ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                                drop = Some(i);
                            }
                        });
                    }
                    if let Some(i) = drop {
                        wire.sections.remove(i);
                    }
                    if ui.button(icons::labelled(Icon::Add, tr("Add a section", "Добавить секцию"))).clicked() {
                        let id = wire.sections.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                        wire.sections.push(Section::new(id));
                    }
                    let js = trolley::joints(&wire);
                    let breaks = trolley::section_breaks(&wire, &js);
                    if !breaks.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tr("Insulators (where sections meet)", "Изоляторы (стыки секций)")).strong());
                        for (p, len, stored) in breaks {
                            ui.horizontal(|ui| {
                                let mut l = len;
                                ui.label(format!("{:.0}, {:.0}", p.x, p.y));
                                if ui.add(egui::DragValue::new(&mut l).range(0.0..=20.0).speed(0.05).suffix(tr(" m dead", " м без тока"))).changed() {
                                    match stored {
                                        Some(i) => wire.insulators[i].length = l,
                                        None => wire.insulators.push(trolley::Insulator { pos: p, length: l }),
                                    }
                                }
                                if ui.small_button(icons::labelled(Icon::Search, "")).on_hover_text(tr("Go there", "Перейти")).clicked() {
                                    actions.push(Action::LookAt(p.truncate()));
                                }
                            });
                        }
                    }
                }
                Tab::Spans => {
                    let total: f64 = wire.spans.iter().map(|s| s.length()).sum();
                    ui.label(RichText::new(format!("{} {} · {:.2} km", wire.spans.len(), tr("spans", "пролётов"), total / 1000.0)).color(pal().text_dim));
                    if let Some(i) = *span_sel {
                        let mut drop = false;
                        let sp = &mut wire.spans[i];
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{} {i} · {:.1} m · {} {}", tr("Span", "Пролёт"), sp.length(), sp.points.len(), tr("points", "точек"))).strong().color(pal().accent));
                        egui::Grid::new("trolley span").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                            prop_row(ui, tr("Section", "Секция"), |ui| section_combo(ui, "span section", &wire.sections, &mut sp.section));
                            prop_row(ui, tr("Spacing", "Между проводами"), |ui| {
                                let mut own = sp.spacing.is_some();
                                if check(ui, &mut own, tr("its own", "своё")).changed() {
                                    sp.spacing = own.then_some(wire.spacing);
                                }
                                if let Some(v) = sp.spacing.as_mut() {
                                    ui.add(egui::DragValue::new(v).range(0.2..=2.0).speed(0.01).suffix(" m"));
                                }
                            });
                            prop_row(ui, tr("Height", "Высота"), |ui| {
                                let mut dz = 0.0;
                                if ui.add(egui::DragValue::new(&mut dz).speed(0.02).prefix("± ").suffix(" m")).on_hover_text(tr("Drag: the whole span up or down", "Тяните: весь пролёт вверх или вниз")).changed() {
                                    for p in sp.points.iter_mut() {
                                        p.z += dz;
                                    }
                                }
                            });
                        });
                        ui.horizontal(|ui| {
                            if ui.button(tr("Reverse", "Развернуть")).on_hover_text(tr("The span the other way: the positive wire is on the right of its way", "Пролёт в другую сторону: плюсовой провод справа по его ходу")).clicked() {
                                sp.points.reverse();
                            }
                            if ui.button(icons::labelled(Icon::Search, tr("Go there", "Перейти"))).clicked() {
                                if let Some(p) = sp.points.get(sp.points.len() / 2) {
                                    actions.push(Action::LookAt(p.truncate()));
                                }
                            }
                            if ui.button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked() {
                                drop = true;
                            }
                        });
                        if drop {
                            wire.spans.remove(i);
                            *span_sel = None;
                        }
                    } else {
                        ui.label(RichText::new(tr("Pick a span in the view, or in the list.", "Выберите пролёт в окне вида или в списке.")).small().color(pal().text_dim));
                    }
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("trolley spans").max_height(220.0).show(ui, |ui| {
                        for (i, sp) in wire.spans.iter().enumerate() {
                            if ui.selectable_label(*span_sel == Some(i), format!("{i}: {} {} · {:.0} m", tr("section", "секция"), sp.section, sp.length())).clicked() {
                                *span_sel = Some(i);
                                if let Some(p) = sp.points.get(sp.points.len() / 2) {
                                    actions.push(Action::LookAt(p.truncate()));
                                }
                            }
                        }
                    });
                }
                Tab::Switches => {
                    ui.label(RichText::new(tr("A switch where three spans meet (yellow in the view). Every frog gets one.", "Стрелка там, где сходятся три пролёта (жёлтая точка в окне вида). Она есть у каждой развилки.")).small().color(pal().text_dim));
                    egui::ScrollArea::vertical().id_salt("trolley switches").max_height(160.0).show(ui, |ui| {
                        if wire.switches.is_empty() {
                            ui.label(RichText::new(tr("No frogs yet: lay three spans meeting in one point (a branch).", "Развилок пока нет: проложите три пролёта, сходящихся в одной точке (ответвление).")).color(pal().text_dim));
                        }
                        for (i, s) in wire.switches.iter().enumerate() {
                            let name = if s.name.trim().is_empty() { format!("{} {i}", tr("Switch", "Стрелка")) } else { s.name.clone() };
                            if ui.selectable_label(*switch_sel == Some(i), format!("{name} · {} · {}", s.mode.name(), s.initial.name())).clicked() {
                                *switch_sel = Some(i);
                                actions.push(Action::LookAt(s.pos.truncate()));
                            }
                        }
                    });
                    if let Some(i) = *switch_sel {
                        ui.separator();
                        switch_editor(ui, &mut wire.switches[i]);
                        if ui.button(icons::labelled(Icon::Search, tr("Go there", "Перейти"))).clicked() {
                            actions.push(Action::LookAt(wire.switches[i].pos.truncate()));
                        }
                    }
                }
            });
            let problems = trolley::problems(&wire);
            if !problems.is_empty() {
                ui.separator();
                for (p, text) in problems.iter().take(6) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(text).small().color(pal().warn));
                        if ui.small_button(icons::labelled(Icon::Search, "")).clicked() {
                            actions.push(Action::LookAt(p.truncate()));
                        }
                    });
                }
            }
        });
        self.open = open;
        ed.wire.preview = preview;
        let mut added = Vec::new();
        let kit_def = self.kit.as_ref().and_then(|i| self.kits.as_ref()?.iter().find(|k| &k.0.id() == i)).map(|k| k.1.clone());
        let mut shown = (!self.no_wires, !self.no_masts);
        if let Some(a) = assets.as_deref_mut() {
            let doc = ed.doc.as_ref().unwrap();
            // the lanes of a junction just selected (its model read first)
            if let Some((o, lanes @ None)) = self.lanes.as_mut() {
                if let Sel::Object(k, id) = *o {
                    if let Some(f) = doc.object(k, id).map(|x| x.file.clone()) {
                        let _ = a.object_wait(&f, 2000);
                    }
                }
                *lanes = Some(trolley::object_lanes(doc, a, *o));
            }
            if let Some(req) = lay_req {
                let new = match req {
                    Lay2::Drawn(pts) => vec![(trolley::drawn(&mut wire, pts, lay.section), None)],
                    Lay2::Roads(sel) => {
                        for s in &sel {
                            if let Some(f) = match *s { Sel::Spline(k, id) => doc.spline(k, id).map(|x| x.file.clone()), _ => None } {
                                let _ = a.spline_wait(&f, 2000);
                            }
                        }
                        trolley::over_roads(doc, a, &sel, *lay)
                    }
                    Lay2::Lanes(chosen) => self.lanes.as_ref().and_then(|l| l.1.as_ref()).map(|l| trolley::over_lanes(l, &chosen, *lay)).unwrap_or_default(),
                    Lay2::WireSplines => {
                        // (the map's own wire splines show it already)
                        shown = (false, false);
                        trolley::from_wire_splines(doc, a, lay.section)
                    }
                };
                if kit_def.is_some() {
                    wire.kit = self.kit.clone();
                }
                added = trolley::add_spans(&mut wire, new);
                ed.toast(format!("{} {}", tr("Spans laid:", "Пролётов проложено:"), added.len()));
            }
        }
        let after = if remove_all || !exists { None } else { Some(wire) };
        if after != before {
            let u = kit_def.as_ref().map(|k| crate::trolley_kit::Use { kit: k, wires: shown.0, masts: shown.1 });
            crate::trolley_kit::apply(ed, roots, after, added, u);
        }
        if let (Some(roots), true) = (roots, self.edit.is_some()) {
            self.kit_window(ctx, ed, roots);
        }
    }

    /// The window of a kit: what the wire, its masts, frogs and insulators look like.
    fn kit_window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: &crate::install::Roots) {
        let Some(map_dir) = ed.doc.as_ref().map(|d| d.dir.clone()) else { return };
        let selected_file = match ed.selected {
            Some(Sel::Object(k, id)) => ed.doc.as_ref().and_then(|d| crate::selection::any_object(d, k, id)).map(|o| o.file.clone()),
            _ => None,
        };
        let Some(e) = self.edit.as_mut() else { return };
        let mut open = true;
        let mut done: Option<&str> = None;
        let mut message: Option<String> = None;
        let title = format!("{}: {}", tr("Wire kit", "Комплект контактной сети"), e.def.name);
        egui::Window::new(icons::labelled(Icon::Settings, &title)).id(egui::Id::new("trolley kit window")).open(&mut open).default_size([760.0, 600.0]).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("Everything shown on the map comes from here: openMapper has no wire or mast models of its own. The kit is a .cfg file; change it here or in any text editor.", "Всё, что видно на карте, берётся отсюда: своих моделей провода и опор у openMapper нет. Комплект — это файл .cfg; меняйте его здесь или в любом текстовом редакторе.")).small().color(pal().text_dim));
            egui::Grid::new("kit head").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                kit_row(ui, tr("Name", "Название"), |ui| {
                    ui.add_enabled(e.r.is_none(), egui::TextEdit::singleline(&mut e.def.name).desired_width(240.0)).on_disabled_hover_text(tr("The name is the kit's file: make a copy to rename it", "Название — это файл комплекта: чтобы переименовать, сделайте копию"));
                });
                kit_row(ui, tr("For", "Для"), |ui| {
                    ui.radio_value(&mut e.scope, Scope::Map, tr("this map", "этой карты"));
                    ui.radio_value(&mut e.scope, Scope::Shared, tr("every map", "всех карт"));
                });
                kit_row(ui, tr("Wires", "Провода"), |ui| {
                    ui.add(egui::DragValue::new(&mut e.def.wires).range(1..=8));
                    ui.label(tr("apart", "через"));
                    ui.add(egui::DragValue::new(&mut e.def.spacing).range(0.05..=3.0).speed(0.01).suffix(" m"));
                });
                kit_row(ui, tr("Height", "Высота"), |ui| {
                    ui.add(egui::DragValue::new(&mut e.def.height).range(2.0..=12.0).speed(0.02).suffix(" m")).on_hover_text(tr("Over the road surface", "Над покрытием дороги"));
                    ui.label(tr("thickness", "толщина"));
                    ui.add(egui::DragValue::new(&mut e.def.thickness).range(0.002..=0.1).speed(0.001).fixed_decimals(3).suffix(" m"));
                    ui.color_edit_button_srgb(&mut e.def.colour);
                });
                kit_row(ui, tr("Masts", "Опоры"), |ui| {
                    ui.label(tr("every", "через"));
                    ui.add(egui::DragValue::new(&mut e.def.step).range(5.0..=120.0).speed(0.5).suffix(" m"));
                    ui.label(tr("arm reaching", "кронштейн на"));
                    ui.add(egui::DragValue::new(&mut e.def.reach).range(0.2..=20.0).speed(0.05).suffix(" m")).on_hover_text(tr("A mast stands this far beside its wire, its arm reaching over it", "Опора стоит на таком расстоянии от провода, её кронштейн дотягивается до него"));
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                for p in PIECES {
                    let n = e.def.mesh(p).len();
                    if ui.selectable_label(e.piece == p, format!("{} ({n})", p.label())).clicked() {
                        e.piece = p;
                        e.part = None;
                    }
                }
            });
            ui.label(RichText::new(e.piece.help()).small().color(pal().text_dim));
            let piece = e.piece;
            ui.columns(2, |cols| {
                let ui = &mut cols[0];
                // the parts
                let parts = e.def.mesh_mut(piece);
                egui::ScrollArea::vertical().id_salt("kit parts").max_height(150.0).show(ui, |ui| {
                    if parts.is_empty() {
                        ui.label(RichText::new(tr("No parts: nothing is put on the map for it. Add a part below.", "Нет деталей: на карту для этого ничего не кладётся. Добавьте деталь ниже.")).small().color(pal().text_dim));
                    }
                    for (i, p) in parts.iter().enumerate() {
                        let text = if p.shape == Shape::Object { format!("{}. {}", i + 1, p.file.rsplit('\\').next().unwrap_or("")) } else { format!("{}. {}", i + 1, p.shape.label()) };
                        if ui.selectable_label(e.part == Some(i), text).clicked() {
                            e.part = Some(i);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.menu_button(icons::labelled(Icon::Add, tr("Part", "Деталь")), |ui| {
                        for sh in SHAPES {
                            if ui.button(sh.label()).clicked() {
                                let mut p = PartDef::new(sh);
                                if let (Shape::Object, Some(f)) = (sh, selected_file.as_ref()) {
                                    p.file = f.clone();
                                }
                                parts.push(p);
                                e.part = Some(parts.len() - 1);
                                ui.close();
                            }
                        }
                    });
                    if let Some(i) = e.part.filter(|i| *i < parts.len()) {
                        if ui.small_button(icons::labelled(Icon::Copy, "")).on_hover_text(tr("Duplicate", "Дублировать")).clicked() {
                            let p = parts[i].clone();
                            parts.insert(i + 1, p);
                            e.part = Some(i + 1);
                        }
                        if ui.small_button(icons::labelled(Icon::Delete, "")).on_hover_text(tr("Remove", "Удалить")).clicked() {
                            parts.remove(i);
                            e.part = None;
                        }
                    }
                });
                // the part picked
                if let Some(p) = e.part.and_then(|i| parts.get_mut(i)) {
                    ui.separator();
                    egui::Grid::new("kit part").num_columns(2).spacing(Vec2::new(8.0, 4.0)).show(ui, |ui| {
                        kit_row(ui, tr("Shape", "Форма"), |ui| {
                            egui::ComboBox::from_id_salt("kit shape").selected_text(p.shape.label()).show_ui(ui, |ui| {
                                for sh in SHAPES {
                                    ui.selectable_value(&mut p.shape, sh, sh.label());
                                }
                            });
                        });
                        if p.shape == Shape::Object {
                            kit_row(ui, tr("Object", "Объект"), |ui| {
                                ui.add(egui::TextEdit::singleline(&mut p.file).desired_width(180.0));
                                if let Some(f) = &selected_file {
                                    if ui.small_button(tr("The selected one", "Выбранный")).clicked() {
                                        p.file = f.clone();
                                    }
                                }
                            });
                        }
                        let xyz = |ui: &mut egui::Ui, v: &mut [f32; 3], speed: f64, suffix: &str| {
                            for (c, n) in v.iter_mut().zip(["x", "y", "z"]) {
                                ui.add(egui::DragValue::new(c).speed(speed).prefix(format!("{n} ")).suffix(suffix));
                            }
                        };
                        kit_row(ui, tr("Position", "Положение"), |ui| xyz(ui, &mut p.pos, 0.01, " m"));
                        kit_row(ui, tr("Turn", "Поворот"), |ui| xyz(ui, &mut p.rot, 1.0, "°"));
                        kit_row(ui, tr("Size", "Размер"), |ui| {
                            for (c, n) in p.size.iter_mut().zip(p.shape.size_names()) {
                                if !n.is_empty() {
                                    ui.add(egui::DragValue::new(c).range(0.0..=50.0).speed(0.005).prefix(format!("{n} ")));
                                }
                            }
                        });
                        if p.shape != Shape::Object {
                            kit_row(ui, tr("Colour", "Цвет"), |ui| {
                                ui.color_edit_button_srgb(&mut p.colour);
                            });
                        }
                    });
                }
                // the drawing
                let ui = &mut cols[1];
                kit_drawing(ui, &e.def, piece, e.part);
            });
            ui.separator();
            ui.horizontal(|ui| {
                let dirty = e.def != e.saved || e.r.as_ref().map(|r| r.scope != e.scope).unwrap_or(true);
                if ui.add_enabled(dirty, egui::Button::new(icons::labelled(Icon::Save, tr("Save", "Сохранить")))).clicked() {
                    done = Some("save");
                }
                if ui.add_enabled(e.r.is_some(), egui::Button::new(tr("Save and renew the map's wire", "Сохранить и обновить провод карты"))).on_hover_text(tr("What is laid from this kit is made anew as it now says (a changed reach or step shows on wire laid from now on)", "То, что проложено по этому комплекту, переделывается по нему (изменённые вылет и шаг действуют для новой укладки)")).clicked() {
                    done = Some("renew");
                }
                if ui.add_enabled(dirty, egui::Button::new(tr("Revert", "Вернуть"))).clicked() {
                    e.def = e.saved.clone();
                    e.scope = e.r.as_ref().map(|r| r.scope).unwrap_or(Scope::Map);
                }
                if e.r.is_some() {
                    if ui.button(tr("Copy", "Копия")).on_hover_text(tr("A new kit from this one (to rename it, or change it apart)", "Новый комплект из этого (чтобы переименовать или менять отдельно)")).clicked() {
                        e.r = None;
                        e.def.name = format!("{} {}", e.def.name, tr("copy", "копия"));
                    }
                    if ui.button(tr("Show the file", "Показать файл")).clicked() {
                        done = Some("reveal");
                    }
                }
            });
        });
        let e = self.edit.as_mut().unwrap();
        match done {
            Some("save") | Some("renew") => {
                let r = match &e.r {
                    Some(r) if r.scope != e.scope => crate::trolley_kit::move_to(roots, &map_dir, r, e.scope).map_err(|x| message = Some(x.to_string())).ok(),
                    Some(r) => Some(r.clone()),
                    None => {
                        let r = KitRef { scope: e.scope, stem: crate::trolley_kit::stem_of(&e.def.name) };
                        if r.path(roots, &map_dir).exists() {
                            message = Some(tr("a kit of that name is there already", "комплект с таким именем уже есть").into());
                            None
                        } else {
                            Some(r)
                        }
                    }
                };
                if let Some(r) = r {
                    match crate::trolley_kit::save(roots, &map_dir, &r, &e.def) {
                        Ok(()) => {
                            // (the map's wire names the kit where it now is)
                            if let Some(old) = e.r.as_ref().map(|o| o.id()).filter(|o| *o != r.id()) {
                                if let Some(mut w) = ed.doc.as_ref().and_then(|d| d.traffic.trolley.clone()).filter(|w| w.kit.as_deref() == Some(old.as_str())) {
                                    w.kit = Some(r.id());
                                    trolley::set(ed, Some(roots), Some(w));
                                }
                            }
                            e.saved = e.def.clone();
                            e.r = Some(r.clone());
                            self.kit = Some(r.id());
                            self.kits = None;
                            if done == Some("renew") {
                                let n = crate::trolley_kit::renew(ed, roots, &e.def);
                                ed.toast(format!("{} {n}", tr("Made anew from the kit:", "Переделано по комплекту:")));
                            }
                        }
                        Err(x) => message = Some(x.to_string()),
                    }
                }
            }
            Some("reveal") => {
                if let Some(r) = &e.r {
                    crate::app::reveal(&r.path(roots, &map_dir));
                }
            }
            _ => {}
        }
        if let Some(m) = message {
            ed.toast(m);
        }
        if !open {
            self.edit = None;
        }
    }
}

/// A row of the kit's window: its name, then what it holds side by side.
fn kit_row(ui: &mut egui::Ui, name: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(name).color(pal().text_dim));
    ui.horizontal(add);
    ui.end_row();
}

/// A kit's piece drawn from the side, the front and above, with the wire where it hangs.
fn kit_drawing(ui: &mut egui::Ui, k: &KitDef, piece: Piece, picked: Option<usize>) {
    let p = pal();
    // the parts' edges, the picked one brighter
    let mut lines: Vec<(glam::Vec3, glam::Vec3, bool)> = Vec::new();
    for (i, part) in k.mesh(piece).iter().enumerate() {
        let Some(m) = part.mesh() else { continue };
        for f in &m.faces {
            for w in 0..f.verts.len() {
                let (a, b) = (f.verts[w], f.verts[(w + 1) % f.verts.len()]);
                if a < b {
                    lines.push((m.positions[a as usize], m.positions[b as usize], picked == Some(i)));
                }
            }
        }
    }
    // the wire: across the mast's arm, or along the frog and insulator
    let n = k.wires.max(1) as f32;
    let wires: Vec<glam::Vec3> = (0..k.wires.max(1)).map(|i| {
        let off = (i as f32 - (n - 1.0) * 0.5) * k.spacing as f32;
        if piece == Piece::Mast { glam::Vec3::new(0.0, -k.reach as f32 + off, k.height as f32) } else { glam::Vec3::new(off, 0.0, 0.0) }
    }).collect();
    let views: [(&str, fn(glam::Vec3) -> egui::Vec2); 3] = [
        (tr("From the side", "Сбоку"), |v| egui::vec2(v.y, -v.z)),
        (tr("From the front", "Спереди"), |v| egui::vec2(v.x, -v.z)),
        (tr("From above", "Сверху"), |v| egui::vec2(v.x, -v.y)),
    ];
    // (the three side by side as wide as there is room for)
    let w = ((ui.available_width() - 16.0) / 3.0).clamp(100.0, 260.0);
    ui.horizontal(|ui| {
        for (title, f) in views {
            ui.vertical(|ui| {
                ui.label(RichText::new(title).small());
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, w), egui::Sense::hover());
                let paint = ui.painter_at(rect);
                paint.rect_filled(rect, 3.0, p.canvas);
                let mut lo = egui::vec2(f32::MAX, f32::MAX);
                let mut hi = egui::vec2(f32::MIN, f32::MIN);
                for q in lines.iter().flat_map(|l| [l.0, l.1]).chain(wires.iter().copied()).chain([glam::Vec3::ZERO]) {
                    let v = f(q);
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                let size = (hi - lo).max(egui::vec2(0.5, 0.5));
                let scale = (rect.width() - 20.0) / size.x.max(size.y);
                let mid = (lo + hi) * 0.5;
                let at = |q: glam::Vec3| rect.center() + (f(q) - mid) * scale;
                // the ground or the wire's line, and the origin
                paint.line_segment([egui::pos2(rect.left(), at(glam::Vec3::ZERO).y), egui::pos2(rect.right(), at(glam::Vec3::ZERO).y)], egui::Stroke::new(1.0, p.grid_major));
                paint.circle_stroke(at(glam::Vec3::ZERO), 3.0, egui::Stroke::new(1.0, p.ink_dim));
                for (a, b, hot) in &lines {
                    paint.line_segment([at(*a), at(*b)], egui::Stroke::new(if *hot { 1.4 } else { 0.8 }, if *hot { p.accent } else { p.ink }));
                }
                for q in &wires {
                    paint.circle_filled(at(*q), 3.0, egui::Color32::from_rgb(k.colour[0], k.colour[1], k.colour[2]).gamma_multiply(1.0));
                    paint.circle_stroke(at(*q), 4.0, egui::Stroke::new(1.0, p.warn));
                }
            });
        }
    });
    ui.label(RichText::new(tr("Orange rings: the wires. Objects of the content are drawn on the map, not here.", "Оранжевые кольца: провода. Объекты контента видны на карте, не здесь.")).small().color(pal().text_dim));
}

fn section_combo(ui: &mut egui::Ui, id: &str, sections: &[Section], value: &mut i32) {
    let name = |c: &Section| if c.name.trim().is_empty() { format!("{}", c.id) } else { format!("{} · {}", c.id, c.name) };
    let shown = sections.iter().find(|c| c.id == *value).map(name).unwrap_or_else(|| value.to_string());
    egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
        for c in sections {
            ui.selectable_value(value, c.id, name(c));
        }
    });
}

fn switch_editor(ui: &mut egui::Ui, s: &mut trolley::Switch) {
    egui::Grid::new("trolley switch").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
        prop_row(ui, tr("Name", "Название"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.name).hint_text(tr("a display shows the switch of its name", "табло показывает стрелку с этим названием")).desired_width(200.0));
        });
        prop_row(ui, tr("Set by", "Управление"), |ui| {
            egui::ComboBox::from_id_salt("switch mode").selected_text(s.mode.name()).show_ui(ui, |ui| {
                for m in Mode::ALL {
                    ui.selectable_value(&mut s.mode, m, m.name());
                }
            });
        });
        prop_row(ui, tr("At the start", "В начале"), |ui| {
            egui::ComboBox::from_id_salt("switch initial").selected_text(s.initial.name()).show_ui(ui, |ui| {
                for m in Initial::ALL {
                    ui.selectable_value(&mut s.initial, m, m.name());
                }
            })
            .response
            .on_hover_text(tr("Where it stands when the game reads the wire (at random: drawn once a run; straight in an online game)", "Положение, когда игра читает сеть (случайно: выбирается раз за запуск; в сетевой игре — прямо)"));
        });
        match s.mode {
            Mode::Power | Mode::Indicator => {
                prop_row(ui, tr("Contact plate", "Контакт"), |ui| {
                    ui.add(egui::DragValue::new(&mut s.trigger).range(0.5..=60.0).speed(0.1).suffix(tr(" m before the frog", " м до развилки")));
                });
                if s.mode == Mode::Power {
                    prop_row(ui, tr("Power on from", "Ток от"), |ui| {
                        ui.add(egui::DragValue::new(&mut s.sense).range(0.0..=500.0).speed(0.5).suffix(" A"));
                    });
                }
                prop_row(ui, tr("Invert", "Наоборот"), |ui| {
                    check(ui, &mut s.invert, tr("the other way round", "наоборот"));
                });
            }
            Mode::Radio => {
                prop_row(ui, tr("Channel", "Канал"), |ui| {
                    let mut own = s.channel.is_some();
                    if check(ui, &mut own, "").changed() {
                        s.channel = own.then_some(1);
                    }
                    if let Some(c) = s.channel.as_mut() {
                        ui.add(egui::DragValue::new(c).range(0..=999));
                    } else {
                        ui.label(RichText::new(tr("any", "любой")).color(pal().text_dim));
                    }
                });
                prop_row(ui, tr("Range", "Дальность"), |ui| {
                    ui.add(egui::DragValue::new(&mut s.range).range(1.0..=1000.0).speed(1.0).suffix(" m"));
                });
                prop_row(ui, tr("Route codes", "Коды маршрутов"), |ui| {
                    ui.vertical(|ui| {
                        let mut drop = None;
                        for (i, (code, way)) in s.routes.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(code).range(0..=99999));
                                egui::ComboBox::from_id_salt(("route way", i)).selected_text(way_name(way)).width(90.0).show_ui(ui, |ui| {
                                    for w in trolley::WAYS {
                                        if ui.selectable_label(way == w, way_name(w)).clicked() {
                                            *way = w.to_string();
                                        }
                                    }
                                });
                                if ui.small_button(icons::labelled(Icon::Delete, "")).clicked() {
                                    drop = Some(i);
                                }
                            });
                        }
                        if let Some(i) = drop {
                            s.routes.remove(i);
                        }
                        if ui.small_button(icons::labelled(Icon::Add, tr("A route code", "Код маршрута"))).clicked() {
                            s.routes.push((0, "branch".into()));
                        }
                    });
                });
            }
            Mode::Fixed => {}
        }
    });
}

/// Where a click puts the next point of a span drawn by hand: on a span's end near it, else
/// the wire's height over the ground or road clicked.
fn end_or(w: &trolley::Wire, c: DVec3, height: f64) -> DVec3 {
    trolley::end_near(w, c.truncate()).unwrap_or(c + DVec3::Z * height)
}
