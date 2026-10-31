//! The complex junction editor: a junction drawn from above out of small splines (asphalt,
//! road markings, footways, kerbs). Pieces are laid with the line and arc tools along whatever
//! spline types the catalogue has; arms mark where roads join. The drawing is then either put
//! on the map as it is, as ordinary splines (a complex junction), or saved as a custom
//! junction the Roads tool's automatic junctions can use (see `cjunction`).

use std::path::PathBuf;

use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};
use glam::DVec2;

use crate::assets::{Assets, SplineModel};
use crate::cjunction::{self, Arm, LObject, LSeg, Layout, Piece};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::{Category, Kind, Library};
use crate::roads::{self, Seg};
use crate::ui::{check, Action, TEXT_DIM};
use crate::ui_tex::{quad, TexCache};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Select,
    Line,
    Arc,
    /// A smooth path through any number of clicks (complex turns), finished with Enter, a
    /// double click or the button.
    Curve,
    Arm,
    /// Stand a scenery object on the drawing.
    Object,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pick {
    Piece(usize),
    Arm(usize),
    Object(usize),
}

/// What a drawing gesture comes to: a piece, and the connecting piece Auto-Adapt lays before it.
struct Plan {
    connector: Option<Piece>,
    segs: Vec<LSeg>,
}

pub struct ComplexJunctions {
    pub open: bool,
    pub layout: Layout,
    tool: Tool,
    /// The spline type pieces are drawn with, and the catalogue's filter.
    file: Option<String>,
    cat: Option<Category>,
    search: String,
    /// The points of the piece or arm being drawn.
    pending: Vec<DVec2>,
    pick: Option<Pick>,
    /// The view: the layout's point at the canvas's middle, and pixels per metre.
    centre: DVec2,
    scale: f32,
    snap: bool,
    /// Auto-Align: a piece started at the end of another goes on in line with it (a straight
    /// along its way or across it, an arc tangent to it); else lines keep to steps of 15°.
    auto_align: bool,
    /// Auto-Adapt: a road of another width started at a road's end keeps the edge on the
    /// side it bends to and takes a taper to get there.
    auto_adapt: bool,
    /// Auto-Junction: the arms are found from the roads' free ends, as the drawing changes.
    auto_arms: bool,
    /// The view as the junction will look (no grid, handles or guides; roads out of the
    /// arms), and the lanes of the AI drawn over it.
    preview: bool,
    lanes: bool,
    /// The scenery object stood with the Object tool, its search, how it is turned to the
    /// piece it stands on (degrees), and for 'along the piece': the spacing and the offset
    /// across (m, to the right).
    obj_file: Option<String>,
    obj_search: String,
    obj_turn: f64,
    obj_step: f64,
    obj_offset: f64,
    /// How much the layout is turned when put on the map (degrees).
    turn: f64,
    /// The saved layouts (read when the window opens and after a save).
    saved: Option<Vec<Layout>>,
    /// Moving the picked piece: where the cursor was last. Dragging an end of the picked
    /// piece: which (false: its start).
    moving: Option<DVec2>,
    handle: Option<bool>,
    message: Option<(String, bool)>,
    /// The pieces' textures as the canvas shows them.
    pics: TexCache,
    /// The drawing's junctions as drawn: the pieces and build they were made for, the pieces cut back, the plates' triangles.
    /// Junctions the drawing has room for none of (see `Resolved::skipped`).
    skipped: usize,
    plate: Option<(Vec<Piece>, crate::cjunction::AutoBuild, Vec<Piece>, Vec<([DVec2; 3], Option<std::path::PathBuf>, [[f32; 2]; 3])>, Vec<(Seg, i32, i32)>)>,
    /// Steps back and forward; the drawing as it was last frame, and whether it is being
    /// changed (a drag changes it every frame: one step).
    undo: Vec<Layout>,
    redo: Vec<Layout>,
    seen: Layout,
    changing: bool,
    /// Zoom to the whole drawing at the next frame (the first, after opening or a new one).
    fit: bool,
    /// The size of the canvas last frame (for fitting).
    canvas: Vec2,
    /// The painted line being set up, and the content folder it is written into (the window
    /// is given it each frame); `reload` asks for the catalogue to be read again.
    paint: crate::mark_spline::Paint,
    content: Option<PathBuf>,
    reload: Option<String>,
}

impl Default for ComplexJunctions {
    fn default() -> Self {
        let layout = fresh();
        ComplexJunctions { open: false, seen: layout.clone(), layout, tool: Tool::Line, file: None, cat: Some(Category::Roads), search: String::new(), pending: Vec::new(), pick: None, centre: DVec2::ZERO, scale: 12.0, snap: true, auto_align: true, auto_adapt: true, auto_arms: true, obj_file: None, obj_search: String::new(), obj_turn: 0.0, obj_step: 20.0, obj_offset: 4.0, preview: false, lanes: false, turn: 0.0, saved: None, moving: None, handle: None, message: None, pics: TexCache::default(), skipped: 0, plate: None, undo: Vec::new(), redo: Vec::new(), changing: false, fit: true, canvas: Vec2::new(800.0, 500.0), paint: crate::mark_spline::Paint::default(), content: None, reload: None }
    }
}

fn fresh() -> Layout {
    Layout { name: tr("My junction", "Мой перекрёсток").into(), auto: true, ..Default::default() }
}

/// What a spline type is drawn as: its colour.
fn colour(cat: Option<Category>, alpha: u8) -> Color32 {
    let (r, g, b) = match cat {
        Some(Category::Roads) => (74, 76, 82),
        Some(Category::Tracks) => (120, 100, 80),
        Some(Category::Paths) => (176, 176, 170),
        Some(Category::Decoration) => (196, 166, 110),
        _ => (110, 170, 200),
    };
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

fn is_road(lib: &Library, file: &str) -> bool {
    lib.find(file).map(|a| a.category == Category::Roads).unwrap_or(false)
}

fn half_of(lib: &Library, file: &str) -> f64 {
    lib.find(file).map(|a| a.half_width as f64).unwrap_or(1.5)
}

/// The end `dir` of a line drawn from `a` to `b`, turned onto the way the end it grows
/// from goes on (`tangent`: straight on, or across it) or onto a step of 15°, when close.
fn align_end(a: DVec2, b: DVec2, tangent: Option<f64>) -> DVec2 {
    let v = b - a;
    let d = v.length();
    if d < 0.01 {
        return b;
    }
    let h = roads::heading_of(v);
    let near = |c: f64| ((h - c + 180.0).rem_euclid(360.0) - 180.0).abs();
    let mut best: Option<(f64, f64)> = None;
    if let Some(t) = tangent {
        for c in [t, t + 90.0, t - 90.0] {
            if near(c) < 10.0 {
                best = Some((near(c), c));
            }
        }
    }
    if best.is_none() {
        let c = (h / 15.0).round() * 15.0;
        if near(c) < 3.0 {
            best = Some((near(c), c));
        }
    }
    match best {
        Some((_, c)) => a + roads::dir(c) * d,
        None => b,
    }
}

impl ComplexJunctions {
    /// The lanes drawn over the drawing, or not.
    pub fn show_lanes(&mut self, on: bool) {
        self.lanes = on;
    }

    pub fn show_preview(&mut self, on: bool) {
        self.preview = on;
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, content: Option<&PathBuf>, assets: Option<&mut Assets>, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        if self.saved.is_none() {
            self.saved = Some(content.map(|c| Layout::load_all(c)).unwrap_or_default());
        }
        let mut open = self.open;
        self.content = content.cloned();
        let mut assets = assets;
        let screen = ctx.content_rect().size();
        let size = Vec2::new((screen.x * 0.9).clamp(900.0, 1700.0), (screen.y * 0.88).clamp(600.0, 1100.0));
        egui::Window::new(icons::labelled(Icon::Junction, tr("Complex junction editor", "Редактор сложных перекрёстков"))).open(&mut open).default_size(size).min_size(Vec2::new(760.0, 520.0)).resizable(true).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                egui::ScrollArea::vertical().id_salt("cj side").auto_shrink([false, false]).max_width(268.0).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(256.0);
                        self.side(ui, lib, &mut assets);
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    self.toolbar(ui);
                    self.canvas(ui, lib, &mut assets);
                    ui.add_space(4.0);
                    self.bottom(ui, ed, content, actions);
                });
            });
        });
        self.open = open;
        if let Some(m) = self.reload.take() {
            actions.push(Action::ReloadContent(m));
        }
        self.track_changes(lib);
    }

    /// One step in the history for a burst of changes (a drag is one), and the arms found
    /// again when Auto-Junction is on.
    fn track_changes(&mut self, lib: &Library) {
        if self.auto_arms && self.layout != self.seen {
            self.layout.sync_arms(&|f| is_road(lib, f), &|f| half_of(lib, f));
            self.rebuild();
            if let Some(Pick::Arm(i)) = self.pick {
                if i >= self.layout.arms.len() {
                    self.pick = None;
                }
            }
        }
        if self.layout != self.seen {
            if !self.changing {
                self.undo.push(self.seen.clone());
                if self.undo.len() > 200 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.changing = true;
            self.seen = self.layout.clone();
        } else {
            self.changing = false;
        }
    }

    /// The turns, corners and straight-on pieces built again from the arms; a picked piece
    /// keeps its place in the list.
    fn rebuild(&mut self) {
        let map = self.layout.sync_built();
        if let Some(Pick::Piece(i)) = self.pick {
            self.pick = map.get(i).copied().flatten().map(Pick::Piece);
        }
    }

    fn undo_step(&mut self) {
        if let Some(l) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.layout, l));
            self.seen = self.layout.clone();
            self.fix_pick();
        }
    }

    fn redo_step(&mut self) {
        if let Some(l) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.layout, l));
            self.seen = self.layout.clone();
            self.fix_pick();
        }
    }

    fn fix_pick(&mut self) {
        self.pending.clear();
        self.pick = match self.pick {
            Some(Pick::Piece(i)) if i < self.layout.pieces.len() => self.pick,
            Some(Pick::Arm(i)) if i < self.layout.arms.len() => self.pick,
            _ => None,
        };
    }

    /// A new, empty drawing (or a saved one opened).
    fn load(&mut self, l: Layout) {
        self.layout = l;
        self.seen = self.layout.clone();
        self.undo.clear();
        self.redo.clear();
        self.pending.clear();
        self.pick = None;
        self.fit = true;
    }

    /// The road piece end at `p` (within 0.3 m): the point, the heading out of the piece
    /// there and the road's half width.
    fn end_at(&self, p: DVec2, lib: &Library) -> Option<(DVec2, f64, f64)> {
        let mut best: Option<(f64, (DVec2, f64, f64))> = None;
        for piece in self.layout.pieces.iter().filter(|q| is_road(lib, &q.file) && !q.auto) {
            let Some((a, ha, b, hb)) = piece.ends() else { continue };
            for (pt, h) in [(a, roads::norm_deg(ha + 180.0)), (b, hb)] {
                let d = (pt - p).length();
                if d < 0.3 && best.map(|x| d < x.0).unwrap_or(true) {
                    best = Some((d, (pt, h, half_of(lib, &piece.file))));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// What drawing through `pts` with the tool makes: a piece (and, with Auto-Adapt, the
    /// taper before it).
    fn plan(&self, pts: &[DVec2], lib: &Library) -> Option<Plan> {
        let file = self.file.as_ref()?;
        let mut pts = pts.to_vec();
        if pts.len() < 2 {
            return None;
        }
        let from = if self.auto_align || self.auto_adapt { self.end_at(pts[0], lib) } else { None };
        let start_h = from.map(|e| e.1).filter(|_| self.auto_align);
        let mut connector = None;
        if let (true, Some((pt, out, hw0)), true) = (self.auto_adapt, from, is_road(lib, file)) {
            let step = hw0 - half_of(lib, file);
            let lateral = roads::right(out).dot(*pts.last()? - pt);
            if step.abs() >= 0.25 && lateral.abs() >= 1.0 {
                // (the new road keeps the old one's edge on the side it bends to)
                let shift = step * lateral.signum();
                let taper = (step.abs() * 5.0).clamp(6.0, 20.0);
                let q = pt + roads::dir(out) * taper + roads::right(out) * shift;
                if let Some(v) = roads::biarc(pt, out, q, out) {
                    connector = Some(Piece { file: file.clone(), segs: v.iter().map(LSeg::of).collect(), lift: 0.0, hidden: false, auto: false });
                    pts[0] = q;
                }
            }
        }
        let segs: Vec<roads::Seg> = match self.tool {
            Tool::Line => {
                let end = if self.auto_align { align_end(pts[0], pts[1], start_h) } else { pts[1] };
                roads::straight(pts[0], end).into_iter().collect()
            }
            Tool::Arc if pts.len() >= 3 => cjunction::arc3(pts[0], pts[1], pts[2]).into_iter().collect(),
            Tool::Arc => match start_h {
                Some(h) => cjunction::tangent_arc(pts[0], h, pts[1]).into_iter().collect(),
                None => roads::straight(pts[0], pts[1]).into_iter().collect(),
            },
            Tool::Curve => cjunction::curve_through(&pts, start_h, None),
            _ => return None,
        };
        if segs.is_empty() {
            return None;
        }
        Some(Plan { connector, segs: segs.iter().map(LSeg::of).collect() })
    }

    /// The clicks that finish a piece with the tool, given the points so far.
    fn clicks_needed(&self, lib: &Library) -> Option<usize> {
        match self.tool {
            Tool::Line => Some(2),
            Tool::Arc => Some(if self.pending.first().map(|p| self.auto_align && self.end_at(*p, lib).is_some()).unwrap_or(false) { 2 } else { 3 }),
            _ => None,
        }
    }

    /// The pending points laid as a piece.
    fn commit(&mut self, lib: &Library) {
        let pts = std::mem::take(&mut self.pending);
        let Some(file) = self.file.clone() else { return };
        match self.plan(&pts, lib) {
            Some(plan) => {
                // markings and footways lie a little over asphalt
                let lift = if is_road(lib, &file) { 0.0 } else { 0.02 };
                if let Some(mut c) = plan.connector {
                    c.lift = lift;
                    self.layout.pieces.push(c);
                }
                self.layout.pieces.push(Piece { file, segs: plan.segs, lift, hidden: false, auto: false });
                self.pick = Some(Pick::Piece(self.layout.pieces.len() - 1));
            }
            None => self.message = Some((tr("Too short: the points are too close", "Слишком коротко: точки слишком близко").into(), true)),
        }
    }

    // ------------------------------------------------------------ the left side

    /// The left side: the layout's name, the tools, the catalogue of spline types, and the
    /// picked piece or arm.
    fn side(&mut self, ui: &mut egui::Ui, lib: &Library, assets: &mut Option<&mut Assets>) {
        ui.horizontal(|ui| {
            ui.label(tr("Name", "Название"));
            ui.add(egui::TextEdit::singleline(&mut self.layout.name).desired_width(120.0));
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("cj saved").selected_text(tr("Open a saved one…", "Открыть сохранённый…")).width(150.0).show_ui(ui, |ui| {
                let list = self.saved.clone().unwrap_or_default();
                for l in list {
                    if ui.selectable_label(false, &l.name).clicked() {
                        self.load(l);
                    }
                }
            });
            if ui.button(tr("New", "Новый")).clicked() {
                self.load(fresh());
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Draw with", "Рисовать")).strong());
        ui.horizontal_wrapped(|ui| {
            for (t, icon, label, hint) in [
                (Tool::Select, Icon::Select, tr("Select", "Выбор"), tr("Pick a piece or an arm; drag it to move it, drag the dots at the ends of the picked piece to change it; Delete removes it", "Выбрать деталь или въезд; перетаскивание двигает, точки на концах выбранной детали меняют её; Delete удаляет")),
                (Tool::Line, Icon::Line, tr("Line", "Линия"), tr("Click the start, then the end", "Клик в начало, затем в конец")),
                (Tool::Arc, Icon::Arc, tr("Arc", "Дуга"), tr("Click the start, a point on the arc, then the end. Started at the end of a piece with Auto-Align on, two clicks make an arc tangent to it", "Клик в начало, в точку дуги, затем в конец. От конца детали при включённом Авто-выравнивании хватает двух кликов: дуга касается её")),
                (Tool::Curve, Icon::Road, tr("Curve", "Кривая"), tr("A smooth path through every point clicked, for complex turns: Enter, a double click or the button below ends it", "Плавная линия через все точки, для сложных поворотов: Enter, двойной клик или кнопка ниже заканчивают")),
                (Tool::Object, Icon::Building, tr("Object", "Объект"), tr("Stand a scenery object (a lamp, a shelter, a bin): on a piece it is turned along it, with Auto-Align on", "Поставить объект (фонарь, остановку, урну): на детали он поворачивается вдоль неё при включённом Авто-выравнивании")),
                (Tool::Arm, Icon::Arm, tr("Arm", "Въезд"), tr("Where a road comes in: click the place, then a point out along the road", "Где входит дорога: клик в место, затем в точку вдоль дороги наружу")),
            ] {
                if ui.selectable_label(self.tool == t, icons::labelled(icon, label)).on_hover_text(hint).clicked() {
                    self.tool = t;
                    self.pending.clear();
                }
            }
        });
        if self.tool == Tool::Curve && self.pending.len() >= 2 && ui.button(tr("Finish the curve", "Закончить кривую")).clicked() {
            self.commit(lib);
        }
        check(ui, &mut self.snap, tr("Snap to the grid and to ends", "Привязка к сетке и концам"));
        check(ui, &mut self.auto_align, tr("Auto-Align", "Авто-выравнивание")).on_hover_text(tr("A piece started at the end of another goes on in line with it: a line along its way or across it, an arc or a curve tangent to it. Elsewhere lines keep to steps of 15°", "Деталь от конца другой продолжает её: линия вдоль или поперёк, дуга и кривая касательно. В остальных местах линии держатся шагов в 15°"));
        check(ui, &mut self.auto_adapt, tr("Auto-Adapt", "Авто-переход")).on_hover_text(tr("A road of another width started at the end of a road keeps its edge on the side it bends to, and a taper is laid to get there", "Дорога другой ширины от конца дороги сохраняет её край со стороны поворота, переход укладывается сам"));
        if check(ui, &mut self.auto_arms, tr("Auto-Junction", "Авто-перекрёсток")).on_hover_text(tr("The arms are found by themselves: every free end of a road is where a road comes in. Arms put by hand stay as they are", "Въезды находятся сами: каждый свободный конец дороги — место, где входит дорога. Въезды, поставленные вручную, остаются")).changed() && self.auto_arms {
            self.layout.sync_arms(&|f| is_road(lib, f), &|f| half_of(lib, f));
            self.rebuild();
        }
        if self.auto_arms {
            ui.indent("cj built", |ui| {
                let b = &mut self.layout.build;
                check(ui, &mut b.plate, tr("Plate with kerbs and footways", "Плита с бордюрами и тротуарами")).on_hover_text(tr("Junctions as the Roads tool makes them: where roads cross, one ends on another or several end at one place, they are cut back and a plate is laid with rounded kerbs, footways and lanes through it", "Перекрёстки, как их делает инструмент «Дороги»: где дороги пересекаются, одна упирается в другую или несколько сходятся в одном месте, они обрезаются и кладётся плита со скруглёнными бордюрами, тротуарами и полосами через неё"));
                check(ui, &mut b.turns, tr("Turns", "Повороты")).on_hover_text(tr("A turn of the arm's road type between each two neighbouring arms, so traffic has lanes through the junction", "Поворот типа дороги въезда между каждыми двумя соседними въездами: у транспорта есть полосы через перекрёсток"));
                check(ui, &mut b.corners, tr("Rounded corners", "Скруглённые углы")).on_hover_text(tr("Where the roads' middle lines meet, the turn is an arc of this radius (a turn is still made where it does not fit)", "Где осевые дорог пересекаются, поворот делается дугой этого радиуса (где она не помещается, остаётся обычный поворот)"));
                ui.add_enabled_ui(b.corners, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Corner radius", "Радиус угла"));
                        ui.add(egui::DragValue::new(&mut b.radius).range(2.0..=60.0).speed(0.2).suffix(" m"));
                    });
                });
                check(ui, &mut b.through, tr("Straight on", "Прямо")).on_hover_text(tr("A piece straight on between arms that face each other", "Деталь прямо между въездами, смотрящими друг на друга"));
            });
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new(tr("Undo", "Отменить"))).on_hover_text("Ctrl+Z").clicked() {
                self.undo_step();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new(tr("Redo", "Вернуть"))).on_hover_text("Ctrl+Y").clicked() {
                self.redo_step();
            }
        });
        ui.add_space(4.0);
        // (a road to begin with: what is drawn is seen at once)
        if self.file.is_none() {
            self.file = lib.filter(Kind::Spline, Some(Category::Roads), "").next().map(|a| a.rel.clone());
        }
        if self.tool == Tool::Object {
            ui.label(RichText::new(tr("Object", "Объект")).strong());
            ui.add(egui::TextEdit::singleline(&mut self.obj_search).hint_text(tr("Search", "Поиск")).desired_width(f32::INFINITY));
            egui::ScrollArea::vertical().id_salt("cj objects").max_height(190.0).show(ui, |ui| {
                for a in lib.filter(Kind::Object, None, &self.obj_search).filter(|a| a.category != Category::Technical).take(150) {
                    let on = self.obj_file.as_deref().map(|f| f.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
                    if ui.selectable_label(on, &a.name).on_hover_text(&a.rel).clicked() {
                        self.obj_file = Some(a.rel.clone());
                    }
                }
            });
            let name = self.obj_file.as_deref().and_then(|f| lib.find(f)).map(|a| a.name.clone()).unwrap_or_else(|| tr("nothing chosen", "ничего не выбрано").to_string());
            ui.label(RichText::new(format!("{} {name}", tr("Standing:", "Ставится:"))).small().color(TEXT_DIM));
            ui.horizontal(|ui| {
                ui.label(tr("Turn", "Поворот"));
                ui.add(egui::DragValue::new(&mut self.obj_turn).speed(1.0).suffix("°")).on_hover_text(tr("Added to the way the piece goes where it stands on one (180° faces it the other way)", "Добавляется к направлению детали, на которой он стоит (180° — в другую сторону)"));
                ui.label(tr("Offset", "Сдвиг"));
                ui.add(egui::DragValue::new(&mut self.obj_offset).speed(0.1).suffix(" m")).on_hover_text(tr("Across the piece to the right, for 'Along the piece'", "Поперёк детали вправо, для «Вдоль детали»"));
                ui.label(tr("Every", "Через"));
                ui.add(egui::DragValue::new(&mut self.obj_step).speed(0.5).range(2.0..=200.0).suffix(" m"));
            });
        } else {
            self.paint_section(ui);
            ui.label(RichText::new(tr("Spline type", "Тип сплайна")).strong());
            egui::ComboBox::from_id_salt("cj cat").selected_text(self.cat.map(|c| c.label()).unwrap_or(tr("Everything", "Всё"))).width(ui.available_width()).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.cat, None, tr("Everything", "Всё"));
                for c in [Category::Roads, Category::Paths, Category::Decoration, Category::Tracks] {
                    ui.selectable_value(&mut self.cat, Some(c), c.label());
                }
            });
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("Search", "Поиск")).desired_width(f32::INFINITY));
            egui::ScrollArea::vertical().id_salt("cj list").max_height(170.0).show(ui, |ui| {
                for a in lib.filter(Kind::Spline, self.cat, &self.search).filter(|a| a.category != Category::Technical).take(150) {
                    let on = self.file.as_deref().map(|f| f.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
                    if ui.selectable_label(on, &a.name).on_hover_text(&a.rel).clicked() {
                        self.file = Some(a.rel.clone());
                    }
                }
            });
            let name = self.file.as_deref().and_then(|f| lib.find(f)).map(|a| a.name.clone()).unwrap_or_else(|| tr("nothing chosen", "ничего не выбрано").to_string());
            ui.label(RichText::new(format!("{} {name}", tr("Drawing with:", "Рисуется:"))).small().color(TEXT_DIM));
            // the chosen type as it looks: a stretch of it from above, as it will be drawn
            if let Some(file) = self.file.clone() {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 3.0, Color32::from_rgb(34, 38, 34));
                let hw = half_of(lib, &file).max(0.5);
                let sc = ((rect.height() - 8.0) as f64 / (hw * 2.0)).min(14.0) as f32;
                let to = move |p: DVec2| Pos2::new(rect.center().x + p.x as f32 * sc, rect.center().y - p.y as f32 * sc);
                let len = (rect.width() / sc) as f64 + 2.0;
                let sample = Piece { file, segs: vec![LSeg { x: -len / 2.0, y: 0.0, heading: 90.0, radius: 0.0, length: len }], lift: 0.0, hidden: false, auto: false };
                let ctx = ui.ctx().clone();
                draw_one(&mut self.pics, &painter, &to, &sample, lib, assets, &ctx, sc, false, 255, false);
            }
        }
        ui.add_space(6.0);
        let chosen = self.file.clone();
        match self.pick {
            Some(Pick::Piece(i)) if i < self.layout.pieces.len() => self.piece_panel(ui, lib, i, chosen),
            Some(Pick::Arm(i)) if i < self.layout.arms.len() => {
                ui.label(RichText::new(tr("Arm", "Въезд")).strong());
                let mut delete = false;
                let a = &mut self.layout.arms[i];
                ui.horizontal(|ui| {
                    ui.label("x");
                    ui.add(egui::DragValue::new(&mut a.x).speed(0.05).suffix(" m"));
                    ui.label("y");
                    ui.add(egui::DragValue::new(&mut a.y).speed(0.05).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Faces", "Курс"));
                    ui.add(egui::DragValue::new(&mut a.out).speed(1.0).range(0.0..=360.0).suffix("°"));
                });
                if a.auto {
                    ui.label(RichText::new(tr("found from a road's free end", "найден по свободному концу дороги")).small().color(TEXT_DIM));
                }
                ui.label(RichText::new(if a.road.is_empty() { tr("any road type", "любой тип дороги").to_string() } else { lib.find(&a.road).map(|r| r.name.clone()).unwrap_or_else(|| a.road.clone()) }).small());
                ui.horizontal(|ui| {
                    if ui.button(tr("Road: the type chosen", "Дорога: выбранный тип")).clicked() {
                        if let Some(f) = chosen.clone() {
                            a.road = f;
                            a.auto = false;
                        }
                    }
                    if ui.button(tr("Any", "Любая")).clicked() {
                        a.road.clear();
                        a.auto = false;
                    }
                });
                delete = ui.button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked() || delete;
                if delete {
                    self.layout.arms.remove(i);
                    self.pick = None;
                }
            }
            _ => {}
        }
        if let Some(Pick::Object(i)) = self.pick.filter(|p| matches!(p, Pick::Object(i) if *i < self.layout.objects.len())) {
            ui.label(RichText::new(tr("Object", "Объект")).strong());
            let o = &mut self.layout.objects[i];
            ui.label(RichText::new(lib.find(&o.file).map(|a| a.name.clone()).unwrap_or_else(|| o.file.clone())).small());
            ui.horizontal(|ui| {
                ui.label("x");
                ui.add(egui::DragValue::new(&mut o.x).speed(0.05).suffix(" m"));
                ui.label("y");
                ui.add(egui::DragValue::new(&mut o.y).speed(0.05).suffix(" m"));
            });
            ui.horizontal(|ui| {
                ui.label(tr("Heading", "Курс"));
                ui.add(egui::DragValue::new(&mut o.heading).speed(1.0).suffix("°"));
                ui.label(tr("Height", "Высота"));
                ui.add(egui::DragValue::new(&mut o.z).speed(0.02).suffix(" m"));
            });
            if ui.small_button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked() {
                self.layout.objects.remove(i);
                self.pick = None;
            }
        }
        ui.add_space(6.0);
        self.layers(ui, lib);
        ui.add_space(6.0);
        ui.label(RichText::new(tr("The whole drawing", "Весь рисунок")).strong());
        ui.horizontal_wrapped(|ui| {
            if ui.small_button(tr("Mirror", "Зеркало")).on_hover_text(tr("Left and right swapped", "Лево и право меняются местами")).clicked() {
                self.layout.transform(true, 0.0);
            }
            if ui.small_button(tr("Turn 90°", "Повернуть на 90°")).clicked() {
                self.layout.transform(false, 90.0);
            }
            if ui.small_button(tr("Find the arms", "Найти въезды")).on_hover_text(tr("The free ends of roads become arms", "Свободные концы дорог становятся въездами")).clicked() {
                let was = self.auto_arms;
                self.layout.sync_arms(&|f| is_road(lib, f), &|f| half_of(lib, f));
                self.rebuild();
                self.auto_arms = was;
            }
            if ui.small_button(tr("Clear", "Очистить")).clicked() {
                self.layout.pieces.clear();
                self.layout.arms.clear();
                self.pick = None;
                self.pending.clear();
            }
        });
    }

    /// Painted lines: the line's look is set here, written as a spline type of its own and
    /// chosen to draw with; it is laid with the Line, Arc and Curve tools like any other.
    fn paint_section(&mut self, ui: &mut egui::Ui) {
        use crate::mark_spline::{Paint, PaintStyle};
        egui::CollapsingHeader::new(RichText::new(tr("Painted line", "Нарисованная линия")).strong()).id_salt("cj paint").default_open(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, hint, preset) in [
                    (tr("Stop line", "Стоп-линия"), tr("A wide solid line across a lane: draw it across the road", "Широкая сплошная линия поперёк полосы: рисуйте её поперёк дороги"), Paint::stop_line()),
                    (tr("Crossing", "Переход"), tr("Zebra bars: draw it across the road, the bars lie along it", "Зебра: рисуйте поперёк дороги, полосы лежат вдоль неё"), Paint::crossing()),
                    (tr("Centre line", "Осевая"), tr("A double yellow line", "Двойная жёлтая"), Paint { style: PaintStyle::Double, ..Paint::default().yellow() }),
                    (tr("Lane line", "Между полос"), tr("A white dashed line", "Белый пунктир"), Paint { style: PaintStyle::Dashed, ..Default::default() }),
                ] {
                    if ui.small_button(label).on_hover_text(hint).clicked() {
                        self.paint = preset;
                    }
                }
            });
            let p = &mut self.paint;
            egui::ComboBox::from_id_salt("cj paint style").selected_text(match p.style {
                PaintStyle::Solid => tr("Solid", "Сплошная"),
                PaintStyle::Dashed => tr("Dashed", "Пунктир"),
                PaintStyle::Double => tr("Double", "Двойная"),
                PaintStyle::SolidDashed => tr("Solid and dashed", "Сплошная и пунктир"),
                PaintStyle::Zebra => tr("Crossing (bars)", "Переход (зебра)"),
            }).width(ui.available_width()).show_ui(ui, |ui| {
                for (s, label) in [(PaintStyle::Solid, tr("Solid", "Сплошная")), (PaintStyle::Dashed, tr("Dashed", "Пунктир")), (PaintStyle::Double, tr("Double", "Двойная")), (PaintStyle::SolidDashed, tr("Solid and dashed", "Сплошная и пунктир")), (PaintStyle::Zebra, tr("Crossing (bars)", "Переход (зебра)"))] {
                    ui.selectable_value(&mut p.style, s, label);
                }
            });
            let zebra = p.style == PaintStyle::Zebra;
            let dashes = matches!(p.style, PaintStyle::Dashed | PaintStyle::SolidDashed | PaintStyle::Zebra);
            ui.horizontal(|ui| {
                ui.label(if zebra { tr("Reaches", "Глубина") } else { tr("Width", "Ширина") });
                ui.add(egui::DragValue::new(&mut p.width).speed(0.01).range(0.05..=6.0).suffix(" m")).on_hover_text(tr("A line's width (each line of a double one); a crossing's reach along the road", "Ширина линии (каждой у двойной); у перехода — как далеко он тянется вдоль дороги"));
                ui.label(tr("Colour", "Цвет"));
                ui.color_edit_button_srgb(&mut p.color);
            });
            if dashes {
                ui.horizontal(|ui| {
                    ui.label(if zebra { tr("Bar", "Полоса") } else { tr("Dash", "Штрих") });
                    ui.add(egui::DragValue::new(&mut p.dash).speed(0.05).range(0.1..=20.0).suffix(" m"));
                    ui.label(tr("Gap", "Промежуток"));
                    ui.add(egui::DragValue::new(&mut p.gap).speed(0.05).range(0.1..=30.0).suffix(" m"));
                });
            }
            let paint = *p;
            if ui.add_enabled(self.content.is_some(), egui::Button::new(tr("Draw with this line", "Рисовать этой линией"))).on_hover_text(tr("It is made as a spline type of the content folder (Splines\\openMapper\\marks) and chosen: draw it with Line, Arc or Curve, over the asphalt", "Создаётся как тип сплайна в папке контента (Splines\\openMapper\\marks) и выбирается: рисуйте им Линией, Дугой или Кривой поверх асфальта")).clicked() {
                if let Some(c) = self.content.clone() {
                    match paint.write(&c) {
                        Ok(rel) => {
                            self.file = Some(rel);
                            self.cat = Some(Category::Decoration);
                            if self.tool == Tool::Select || self.tool == Tool::Object || self.tool == Tool::Arm {
                                self.tool = Tool::Line;
                            }
                            self.reload = Some(tr("Painted line made: draw with it in the junction editor", "Линия создана: рисуйте ею в редакторе перекрёстков").to_string());
                            self.message = None;
                        }
                        Err(e) => self.message = Some((e.to_string(), true)),
                    }
                }
            }
        });
        ui.add_space(4.0);
    }

    /// The layers: every piece (the topmost first), the arms and the objects, to pick, hide
    /// and put in order.
    fn layers(&mut self, ui: &mut egui::Ui, lib: &Library) {
        ui.label(RichText::new(tr("Layers", "Слои")).strong());
        let mut swap: Option<(usize, usize)> = None;
        egui::ScrollArea::vertical().id_salt("cj layers").max_height(170.0).show(ui, |ui| {
            let n = self.layout.pieces.len();
            for i in (0..n).rev() {
                if self.layout.pieces[i].auto {
                    let name = lib.find(&self.layout.pieces[i].file).map(|a| a.name.clone()).unwrap_or_else(|| self.layout.pieces[i].file.clone());
                    ui.label(RichText::new(format!("{} · {name}", tr("auto", "авто"))).small().color(TEXT_DIM)).on_hover_text(tr("Built by Auto-Junction from the arms", "Построено Авто-перекрёстком из въездов"));
                    continue;
                }
                ui.horizontal(|ui| {
                    let mut shown = !self.layout.pieces[i].hidden;
                    if ui.checkbox(&mut shown, "").on_hover_text(tr("Shown: a hidden layer is not put on the map or written", "Виден: скрытый слой не ставится на карту и не записывается")).changed() {
                        self.layout.pieces[i].hidden = !shown;
                    }
                    let name = lib.find(&self.layout.pieces[i].file).map(|a| a.name.clone()).unwrap_or_else(|| self.layout.pieces[i].file.clone());
                    let len: f64 = self.layout.pieces[i].segs.iter().map(|s| s.length).sum();
                    if ui.selectable_label(self.pick == Some(Pick::Piece(i)), format!("{name} · {len:.0} m")).clicked() {
                        self.pick = Some(Pick::Piece(i));
                    }
                    if i + 1 < n && !self.layout.pieces[i + 1].auto && ui.small_button("^").on_hover_text(tr("Higher", "Выше")).clicked() {
                        swap = Some((i, i + 1));
                    }
                    if i > 0 && ui.small_button("v").on_hover_text(tr("Lower", "Ниже")).clicked() {
                        swap = Some((i, i - 1));
                    }
                });
            }
            for (i, a) in self.layout.arms.iter().enumerate() {
                if ui.selectable_label(self.pick == Some(Pick::Arm(i)), format!("{} {} · {:.0}°", tr("Arm", "Въезд"), i + 1, a.out)).clicked() {
                    self.pick = Some(Pick::Arm(i));
                }
            }
            for (i, o) in self.layout.objects.iter().enumerate() {
                let name = lib.find(&o.file).map(|a| a.name.clone()).unwrap_or_else(|| o.file.clone());
                if ui.selectable_label(self.pick == Some(Pick::Object(i)), name).clicked() {
                    self.pick = Some(Pick::Object(i));
                }
            }
        });
        if let Some((a, b)) = swap {
            self.layout.pieces.swap(a, b);
            self.pick = Some(Pick::Piece(b));
        }
    }

    /// Objects every `obj_step` m along piece `i`, `obj_offset` m to its right, each turned
    /// along it.
    fn objects_along(&mut self, i: usize) {
        let (Some(file), Some(piece)) = (self.obj_file.clone(), self.layout.pieces.get(i)) else { return };
        let mut made = Vec::new();
        let step = self.obj_step.max(2.0);
        let mut chain = 0.0;
        let mut next = step / 2.0;
        for s in &piece.segs {
            let seg = s.seg();
            while next <= chain + s.length {
                let d = next - chain;
                let h = seg.heading_at(d);
                let p = seg.point(d) + roads::right(h) * self.obj_offset;
                made.push(LObject { file: file.clone(), x: p.x, y: p.y, z: piece.lift, heading: roads::norm_deg(h + self.obj_turn) });
                next += step;
            }
            chain += s.length;
        }
        self.layout.objects.extend(made);
    }

    /// The picked piece: where it lies, its numbers and what can be done with it.
    fn piece_panel(&mut self, ui: &mut egui::Ui, lib: &Library, i: usize, chosen: Option<String>) {
        ui.label(RichText::new(tr("Piece", "Деталь")).strong());
        let (mut delete, mut lower, mut higher, mut dup, mut mirror, mut quarter, mut reverse) = (false, false, false, false, false, false, false);
        let p = &mut self.layout.pieces[i];
        ui.label(RichText::new(lib.find(&p.file).map(|a| a.name.clone()).unwrap_or_else(|| p.file.clone())).small());
        let len: f64 = p.segs.iter().map(|s| s.length).sum();
        ui.label(RichText::new(format!("{len:.1} m · {} {}", p.segs.len(), tr("segments", "участков"))).small().color(TEXT_DIM));
        if let [s] = p.segs.as_mut_slice() {
            ui.horizontal(|ui| {
                ui.label("x");
                ui.add(egui::DragValue::new(&mut s.x).speed(0.05).suffix(" m"));
                ui.label("y");
                ui.add(egui::DragValue::new(&mut s.y).speed(0.05).suffix(" m"));
            });
            ui.horizontal(|ui| {
                ui.label(tr("Heading", "Курс"));
                ui.add(egui::DragValue::new(&mut s.heading).speed(0.5).suffix("°"));
                ui.label(tr("Length", "Длина"));
                ui.add(egui::DragValue::new(&mut s.length).speed(0.05).range(0.5..=2000.0).suffix(" m"));
            });
            ui.horizontal(|ui| {
                ui.label(tr("Radius", "Радиус"));
                ui.add(egui::DragValue::new(&mut s.radius).speed(0.1).range(-4000.0..=4000.0).suffix(" m")).on_hover_text(tr("0: a straight; positive: turns right; negative: turns left", "0 — прямая; больше нуля — вправо; меньше — влево"));
            });
            s.heading = roads::norm_deg(s.heading);
        }
        ui.horizontal(|ui| {
            ui.label(tr("Lift", "Высота"));
            ui.add(egui::DragValue::new(&mut p.lift).speed(0.005).range(-0.5..=2.0).max_decimals(3).suffix(" m")).on_hover_text(tr("Above the plate: markings lie a little over the asphalt they are painted on", "Над плитой: разметка лежит чуть выше асфальта, на котором нарисована"));
        });
        if ui.button(tr("Type: the one chosen", "Тип: выбранный")).clicked() {
            if let Some(f) = chosen {
                p.file = f;
            }
        }
        ui.horizontal_wrapped(|ui| {
            dup = ui.small_button(tr("Copy", "Копия")).on_hover_text("Ctrl+D").clicked();
            mirror = ui.small_button(tr("Mirror", "Зеркало")).clicked();
            quarter = ui.small_button(tr("Turn 90°", "Повернуть на 90°")).clicked();
            reverse = ui.small_button(tr("Reverse", "Развернуть")).on_hover_text(tr("Runs the other way: the piece's start and end swap", "Идёт в другую сторону: начало и конец меняются")).clicked();
            lower = i > 0 && ui.small_button(tr("Lower", "Ниже")).on_hover_text(tr("Under the one before it", "Под предыдущую")).clicked();
            higher = i + 1 < self.layout.pieces.len() && ui.small_button(tr("Higher", "Выше")).on_hover_text(tr("Over the one after it", "Над следующей")).clicked();
            delete = ui.small_button(icons::labelled(Icon::Delete, tr("Delete", "Удалить"))).clicked();
        });
        if ui.add_enabled(self.obj_file.is_some(), egui::Button::new(tr("Objects along it", "Объекты вдоль неё")).small()).on_hover_text(tr("The object chosen with the Object tool stood along this piece at the spacing and offset set there", "Объект, выбранный инструментом «Объект», вдоль этой детали с заданными там шагом и сдвигом")).clicked() {
            self.objects_along(i);
        }
        let p = &mut self.layout.pieces[i];
        let about = p.segs.first().map(|s| DVec2::new(s.x, s.y)).unwrap_or(DVec2::ZERO);
        if mirror || quarter {
            for s in &mut p.segs {
                *s = s.transformed(mirror, if quarter { 90.0 } else { 0.0 }, about);
            }
        }
        if reverse {
            *p = p.reversed();
        }
        if dup {
            self.duplicate(i);
        } else if delete {
            self.layout.pieces.remove(i);
            self.pick = None;
        } else if lower {
            self.layout.pieces.swap(i, i - 1);
            self.pick = Some(Pick::Piece(i - 1));
        } else if higher {
            self.layout.pieces.swap(i, i + 1);
            self.pick = Some(Pick::Piece(i + 1));
        }
    }

    /// A copy of piece `i`, a metre aside, picked.
    fn duplicate(&mut self, i: usize) {
        let Some(mut c) = self.layout.pieces.get(i).cloned() else { return };
        for s in &mut c.segs {
            s.x += 1.0;
            s.y -= 1.0;
        }
        self.layout.pieces.push(c);
        self.pick = Some(Pick::Piece(self.layout.pieces.len() - 1));
    }

    // ------------------------------------------------------------ the view

    /// Above the drawing: edit or preview, the lanes, the zoom.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.preview, false, tr("Edit", "Правка"));
            ui.selectable_value(&mut self.preview, true, tr("Preview", "Просмотр")).on_hover_text(tr("The junction as it will be: the roads running out of the arms, no grid or handles", "Перекрёсток таким, каким будет: дороги от въездов, без сетки и точек"));
            check(ui, &mut self.lanes, tr("Lanes", "Полосы")).on_hover_text(tr("The lanes of the AI over the drawing, with the way they go", "Полосы движения ИИ поверх рисунка, с направлением"));
            ui.separator();
            if ui.button("−").on_hover_text(tr("Zoom out (-)", "Отдалить (-)")).clicked() {
                self.zoom_by(1.0 / 1.35);
            }
            if ui.button("+").on_hover_text(tr("Zoom in (+)", "Приблизить (+)")).clicked() {
                self.zoom_by(1.35);
            }
            if ui.button(tr("Fit", "Вписать")).on_hover_text(tr("The whole drawing in view (F)", "Весь рисунок в окне (F)")).clicked() {
                self.fit = true;
            }
            ui.label(RichText::new(format!("{:.0}%", self.scale / 12.0 * 100.0)).small().color(TEXT_DIM));
        });
    }

    fn zoom_by(&mut self, f: f32) {
        self.scale = (self.scale * f).clamp(0.4, 400.0);
    }

    /// The view that shows the whole drawing (or the 40 m round the origin of an empty one).
    fn fit_view(&mut self) {
        let (lo, hi) = self.layout.bounds().unwrap_or((DVec2::splat(-20.0), DVec2::splat(20.0)));
        let size = (hi - lo).max(DVec2::splat(10.0)) * 1.2 + DVec2::splat(8.0);
        self.centre = (lo + hi) / 2.0;
        let s = (self.canvas.x as f64 / size.x).min(self.canvas.y as f64 / size.y) as f32;
        self.scale = s.clamp(0.4, 60.0);
    }

    /// The drawing, seen from above.
    fn canvas(&mut self, ui: &mut egui::Ui, lib: &Library, assets: &mut Option<&mut Assets>) {
        let size = Vec2::new(ui.available_width().max(300.0), (ui.available_height() - 100.0).max(300.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        self.canvas = rect.size();
        if self.fit {
            self.fit = false;
            self.fit_view();
        }
        // zoom about the cursor: the wheel, a pinch
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let pinch = ui.input(|i| i.zoom_delta());
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            let factor = (1.0 + scroll * 0.0025).max(0.2) * pinch;
            if factor != 1.0 {
                if let Some(h) = resp.hover_pos() {
                    let (scale, centre) = (self.scale, self.centre);
                    let before = DVec2::new(centre.x + (h.x - rect.center().x) as f64 / scale as f64, centre.y - (h.y - rect.center().y) as f64 / scale as f64);
                    self.zoom_by(factor);
                    let after = DVec2::new(centre.x + (h.x - rect.center().x) as f64 / self.scale as f64, centre.y - (h.y - rect.center().y) as f64 / self.scale as f64);
                    self.centre += before - after;
                }
            }
            let keys = ui.input(|i| (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals), i.key_pressed(egui::Key::Minus), i.key_pressed(egui::Key::F), i.modifiers));
            if keys.0 {
                self.zoom_by(1.35);
            }
            if keys.1 {
                self.zoom_by(1.0 / 1.35);
            }
            if keys.2 && !keys.3.command {
                self.fit = true;
            }
            let ctrl = keys.3.command;
            let (z, y, d) = ui.input(|i| (i.key_pressed(egui::Key::Z), i.key_pressed(egui::Key::Y), i.key_pressed(egui::Key::D)));
            if ctrl && z && !keys.3.shift {
                self.undo_step();
            } else if ctrl && (y || (z && keys.3.shift)) {
                self.redo_step();
            } else if ctrl && d {
                if let Some(Pick::Piece(i)) = self.pick {
                    self.duplicate(i);
                }
            }
        }
        // pan: right or middle button, or the left one on empty ground with the Select tool
        let pan_empty = self.tool == Tool::Select && self.pick.is_none() && self.handle.is_none();
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) || (pan_empty && self.moving.is_none() && resp.dragged_by(egui::PointerButton::Primary)) {
            let d = resp.drag_delta();
            self.centre -= DVec2::new(d.x as f64, -d.y as f64) / self.scale as f64;
        }
        let (scale, centre) = (self.scale, self.centre);
        let to_screen = move |p: DVec2| Pos2::new(rect.center().x + ((p.x - centre.x) * scale as f64) as f32, rect.center().y - ((p.y - centre.y) * scale as f64) as f32);
        let from_screen = move |q: Pos2| DVec2::new(centre.x + (q.x - rect.center().x) as f64 / scale as f64, centre.y - (q.y - rect.center().y) as f64 / scale as f64);
        let painter = ui.painter_at(rect);
        let preview = self.preview;
        painter.rect_filled(rect, 3.0, if preview { Color32::from_rgb(78, 96, 70) } else { Color32::from_rgb(34, 38, 34) });
        // the grid: metres up close, tens far away
        if !preview {
            let step = if scale >= 14.0 { 1.0 } else if scale >= 4.0 { 5.0 } else if scale >= 1.2 { 10.0 } else { 50.0 };
            let (lo, hi) = (from_screen(rect.left_bottom()), from_screen(rect.right_top()));
            let line = |a: DVec2, b: DVec2, zero: bool, major: bool| painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(if zero { 1.5 } else { 1.0 }, Color32::from_white_alpha(if zero { 60 } else if major { 26 } else { 12 })));
            let mut x = (lo.x / step).floor() * step;
            while x <= hi.x {
                line(DVec2::new(x, lo.y), DVec2::new(x, hi.y), x == 0.0, (x / (step * 5.0)).fract().abs() < 1e-6);
                x += step;
            }
            let mut y = (lo.y / step).floor() * step;
            while y <= hi.y {
                line(DVec2::new(lo.x, y), DVec2::new(hi.x, y), y == 0.0, (y / (step * 5.0)).fract().abs() < 1e-6);
                y += step;
            }
        }
        let ctx = ui.ctx().clone();
        // in the preview, the roads run on out of the arms
        if preview {
            let fallback = self.layout.pieces.iter().find(|p| is_road(lib, &p.file)).map(|p| p.file.clone());
            for a in &self.layout.arms {
                let Some(file) = (!a.road.is_empty()).then(|| a.road.clone()).or_else(|| fallback.clone()) else { continue };
                let stub = Piece { file: file.clone(), segs: vec![LSeg { x: a.x, y: a.y, heading: a.out, radius: 0.0, length: 40.0 }], lift: 0.0, hidden: false, auto: false };
                draw_one(&mut self.pics, &painter, &to_screen, &stub, lib, assets, &ctx, scale, false, 150, false);
            }
        }
        // the junctions as the Roads tool makes them (made again only when the drawing changes)
        let stale = self.plate.as_ref().map(|p| p.0 != self.layout.pieces || p.1 != self.layout.build).unwrap_or(true);
        if stale {
            // (the roadway's asphalt is cut as it is for the map, into a folder of its own, so that the drawing shows it)
            let preview_dir = std::env::temp_dir().join("openmapper-junction-preview");
            let _ = std::fs::create_dir_all(&preview_dir);
            let resolved = self.layout.resolve(&|f| is_road(lib, f), &|f| half_of(lib, f));
            self.skipped = resolved.skipped;
            let plates: Option<Vec<_>> = resolved.plates.iter().map(|arms| assets.as_deref_mut().and_then(|a| crate::junction::layout_plate(a, arms, Some(&preview_dir), None)).map(|p| (p.flat_textured(), p.lanes))).collect();
            // (a road type not loaded yet: made again next frame)
            self.plate = plates.map(|pl| {
                let lanes = pl.iter().flat_map(|p| p.1.iter().copied()).collect();
                (self.layout.pieces.clone(), self.layout.build, resolved.pieces, pl.into_iter().flat_map(|p| p.0).collect(), lanes)
            });
        }
        let shown: Vec<Piece> = self.plate.as_ref().map(|p| p.2.clone()).unwrap_or_else(|| self.layout.pieces.clone());
        if let Some((_, _, _, tris, _)) = &self.plate {
            // (the roadway flat, the kerbs and footways with their pictures: one mesh each, with
            // no outline round the triangles, which made the fans at the corners bristle)
            let mut flat = egui::epaint::Mesh::default();
            let mut by_tex: Vec<(egui::TextureId, egui::epaint::Mesh)> = Vec::new();
            for (t, tex, uv) in tris {
                let id = tex.as_ref().and_then(|f| self.pics.get(&ctx, f));
                let m = match id {
                    Some(id) => match by_tex.iter().position(|(i, _)| *i == id) {
                        Some(k) => &mut by_tex[k].1,
                        None => {
                            by_tex.push((id, egui::epaint::Mesh::with_texture(id)));
                            &mut by_tex.last_mut().unwrap().1
                        }
                    },
                    None => &mut flat,
                };
                let col = if id.is_some() { Color32::WHITE } else { Color32::from_rgb(74, 76, 82) };
                let base = m.vertices.len() as u32;
                for (p, uv) in t.iter().zip(uv) {
                    m.vertices.push(egui::epaint::Vertex { pos: to_screen(*p), uv: if id.is_some() { Pos2::new(uv[0], uv[1]) } else { egui::epaint::WHITE_UV }, color: col });
                }
                m.indices.extend([base, base + 1, base + 2]);
            }
            painter.add(egui::Shape::mesh(flat));
            for (_, m) in by_tex {
                painter.add(egui::Shape::mesh(m));
            }
        }
        // the pieces, first drawn lowest
        // (the roads as cut back for the junctions; the picked piece shows whole, faint, over them)
        for p in shown.iter().filter(|p| !p.hidden) {
            draw_one(&mut self.pics, &painter, &to_screen, p, lib, assets, &ctx, scale, false, 235, false);
        }
        if !preview {
            for (i, p) in self.layout.pieces.iter().enumerate().filter(|(_, p)| !p.hidden) {
                let picked = self.pick == Some(Pick::Piece(i));
                draw_one(&mut self.pics, &painter, &to_screen, p, lib, assets, &ctx, scale, picked, if picked { 90 } else { 0 }, true);
            }
        }
        if self.lanes {
            // (the lanes as they will be: along the roads as cut back, and through the plates)
            for p in shown.iter().filter(|p| !p.hidden) {
                if let Some(m) = assets.as_deref_mut().and_then(|a| a.spline(&p.file)) {
                    draw_lanes(&painter, &to_screen, p, &m, scale);
                }
            }
            for (seg, kind, dir) in self.plate.iter().flat_map(|p| p.4.iter()) {
                draw_lane_line(&painter, &to_screen, seg, *kind, *dir, scale);
            }
        }
        for (i, o) in self.layout.objects.iter().enumerate() {
            let c = to_screen(DVec2::new(o.x, o.y));
            let picked = !preview && self.pick == Some(Pick::Object(i));
            let col = if picked { Color32::from_rgb(255, 160, 60) } else { Color32::from_rgb(120, 220, 170) };
            let (f, r) = (roads::dir(o.heading), roads::right(o.heading));
            let m = (5.0_f64).max(0.5 * scale as f64) / scale as f64;
            let pt = |a: f64, b: f64| to_screen(DVec2::new(o.x, o.y) + f * a + r * b);
            painter.add(egui::Shape::convex_polygon(vec![pt(m, 0.0), pt(-m * 0.6, m * 0.7), pt(-m * 0.6, -m * 0.7)], col, Stroke::new(1.0, Color32::BLACK)));
            if !preview {
                painter.text(c + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, lib.find(&o.file).map(|a| a.name.as_str()).unwrap_or("?"), egui::FontId::proportional(9.5), Color32::from_gray(200));
            }
        }
        if !preview {
            for (i, a) in self.layout.arms.iter().enumerate() {
                draw_arm(&painter, &to_screen, a, self.pick == Some(Pick::Arm(i)), scale);
            }
            // the handles of the picked piece
            if let (Some(Pick::Piece(i)), Tool::Select) = (self.pick, self.tool) {
                if let Some(piece) = self.layout.pieces.get(i).filter(|p| p.segs.len() == 1) {
                    if let Some((a, _, b, _)) = piece.ends() {
                        for q in [a, b] {
                            painter.circle_filled(to_screen(q), 5.5, Color32::from_rgb(255, 160, 60));
                            painter.circle_stroke(to_screen(q), 5.5, Stroke::new(1.0, Color32::BLACK));
                        }
                    }
                }
            }
            // the place it is put on the map by
            let o = to_screen(DVec2::ZERO);
            painter.circle_stroke(o, 5.0, Stroke::new(1.5, Color32::from_rgb(255, 160, 60)));
            painter.text(o + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, tr("origin", "начало"), egui::FontId::proportional(10.0), Color32::from_rgb(255, 160, 60));
        }
        // a scale bar: 5 m, or 50 far away
        let bar = if scale >= 3.0 { 5.0 } else { 50.0 };
        let sy = rect.bottom() - 10.0;
        painter.line_segment([Pos2::new(rect.left() + 12.0, sy), Pos2::new(rect.left() + 12.0 + bar as f32 * scale, sy)], Stroke::new(2.0, Color32::from_gray(190)));
        painter.text(Pos2::new(rect.left() + 18.0 + bar as f32 * scale, sy), egui::Align2::LEFT_CENTER, format!("{bar} m"), egui::FontId::proportional(11.0), Color32::from_gray(190));

        if !preview {
            self.interact(ui, &resp, &painter, rect, lib, assets, &to_screen, &from_screen);
        }
        let hint = if preview {
            tr("Preview: the junction as it will look. Wheel zooms, right or middle drag pans", "Просмотр: перекрёсток таким, каким будет. Колесо — масштаб, правая/средняя кнопка — сдвиг")
        } else {
            match self.tool {
                Tool::Select => tr("Click a piece or an arm to pick it, drag it to move it", "Клик по детали или въезду выбирает, перетаскивание двигает"),
                Tool::Line => tr("Click the start of the piece, then its end", "Клик в начало детали, затем в её конец"),
                Tool::Arc => tr("Click the start, a point the arc passes, then the end", "Клик в начало, в точку дуги, затем в конец"),
                Tool::Curve => tr("Click the points the curve passes; Enter or a double click ends it", "Кликайте точки, через которые пройдёт кривая; Enter или двойной клик заканчивает"),
                Tool::Arm => tr("Click where a road comes in, then a point out along the road", "Клик, где входит дорога, затем точка вдоль дороги наружу"),
                Tool::Object => tr("Click where the object stands; on a piece it faces along it", "Клик там, где встанет объект; на детали он смотрит вдоль неё"),
            }
        };
        painter.text(rect.left_top() + Vec2::new(8.0, 6.0), egui::Align2::LEFT_TOP, hint, egui::FontId::proportional(11.5), Color32::from_gray(if preview { 235 } else { 200 }));
        painter.text(rect.left_top() + Vec2::new(8.0, 22.0), egui::Align2::LEFT_TOP, tr("Wheel zooms, right or middle drag pans, F fits, Ctrl+Z undoes, Esc cancels", "Колесо — масштаб, правая/средняя кнопка — сдвиг, F — вписать, Ctrl+Z — отмена, Esc — сброс"), egui::FontId::proportional(10.5), Color32::from_gray(if preview { 220 } else { 140 }));
    }

    /// Clicks, keys, dragging and what follows the cursor while drawing.
    #[allow(clippy::too_many_arguments)]
    fn interact(&mut self, ui: &mut egui::Ui, resp: &egui::Response, painter: &egui::Painter, rect: egui::Rect, lib: &Library, assets: &mut Option<&mut Assets>, to_screen: &dyn Fn(DVec2) -> Pos2, from_screen: &dyn Fn(Pos2) -> DVec2) {
        let scale = self.scale;
        // where the cursor is in the drawing, snapped to a piece's or arm's end close by, else
        // to the grid
        let cursor = resp.hover_pos().map(|h| {
            let raw = from_screen(h);
            if !self.snap {
                return raw;
            }
            let near = 9.0 / scale as f64;
            let mut best: Option<(f64, DVec2)> = None;
            let ends = self.layout.pieces.iter().flat_map(|p| p.segs.iter()).flat_map(|s| [s.seg().point(0.0), s.seg().end()]).chain(self.layout.arms.iter().map(|a| DVec2::new(a.x, a.y)));
            for e in ends {
                let d = (e - raw).length();
                if d < near && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, e));
                }
            }
            best.map(|b| b.1).unwrap_or_else(|| {
                let g = if scale >= 14.0 { 0.5 } else if scale >= 4.0 { 1.0 } else { 5.0 };
                DVec2::new((raw.x / g).round() * g, (raw.y / g).round() * g)
            })
        });
        // what is being drawn follows the cursor
        if let Some(c) = cursor {
            let mut pts = self.pending.clone();
            pts.push(c);
            if self.file.is_some() && matches!(self.tool, Tool::Line | Tool::Arc | Tool::Curve) {
                if let Some(plan) = self.plan(&pts, lib) {
                    let file = self.file.clone().unwrap_or_default();
                    let ctx = ui.ctx().clone();
                    for piece in plan.connector.into_iter().chain(std::iter::once(Piece { file: file.clone(), segs: plan.segs, lift: 0.0, hidden: false, auto: false })) {
                        // (as it will look, a little faded, with its outline)
                        draw_one(&mut self.pics, painter, to_screen, &piece, lib, assets, &ctx, scale, true, 170, true);
                    }
                }
            }
            if self.tool == Tool::Object && self.obj_file.is_some() {
                let (p, h) = self.object_spot(c, lib);
                let (f, r) = (roads::dir(h), roads::right(h));
                let m = 8.0 / scale as f64;
                let pt = |a: f64, b: f64| to_screen(p + f * a + r * b);
                painter.add(egui::Shape::convex_polygon(vec![pt(m, 0.0), pt(-m * 0.6, m * 0.7), pt(-m * 0.6, -m * 0.7)], Color32::from_rgba_unmultiplied(255, 200, 90, 170), Stroke::new(1.0, Color32::BLACK)));
            }
            if self.tool == Tool::Arm && self.pending.len() == 1 {
                let out = roads::heading_of(c - self.pending[0]);
                draw_arm(painter, to_screen, &Arm { x: self.pending[0].x, y: self.pending[0].y, out, ..Default::default() }, true, scale);
            }
            painter.circle_stroke(to_screen(c), 4.0, Stroke::new(1.0, Color32::from_rgb(255, 200, 90)));
            for p in &self.pending {
                painter.circle_filled(to_screen(*p), 3.0, Color32::from_rgb(255, 200, 90));
            }
            painter.text(rect.right_top() + Vec2::new(-8.0, 6.0), egui::Align2::RIGHT_TOP, format!("{:.1}, {:.1} m", c.x, c.y), egui::FontId::proportional(11.0), Color32::from_gray(190));
        }

        // keys and clicks
        let (esc, enter) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
        if esc || (resp.clicked_by(egui::PointerButton::Secondary) && !self.pending.is_empty()) {
            self.pending.clear();
        }
        if (enter || resp.double_clicked_by(egui::PointerButton::Primary)) && self.tool == Tool::Curve && resp.hovered() {
            // (a double click has put its point twice: the same place twice counts once)
            if self.pending.len() >= 2 {
                self.commit(lib);
            }
        }
        let delete = resp.hovered() && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        if delete {
            match self.pick.take() {
                Some(Pick::Piece(i)) if i < self.layout.pieces.len() => {
                    self.layout.pieces.remove(i);
                }
                Some(Pick::Arm(i)) if i < self.layout.arms.len() => {
                    self.layout.arms.remove(i);
                }
                Some(Pick::Object(i)) if i < self.layout.objects.len() => {
                    self.layout.objects.remove(i);
                }
                _ => {}
            }
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            if let Some(c) = cursor {
                self.click(c, resp.hover_pos().map(from_screen).unwrap_or(c), 8.0 / scale as f64, lib);
            }
        }
        // moving the picked piece or arm, or an end of the picked piece
        if self.tool == Tool::Select {
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(h) = resp.interact_pointer_pos() {
                    let raw = from_screen(h);
                    let tol = 8.0 / scale as f64;
                    self.handle = None;
                    if let Some(Pick::Piece(i)) = self.pick {
                        if let Some((a, _, b, _)) = self.layout.pieces.get(i).filter(|p| p.segs.len() == 1).and_then(|p| p.ends()) {
                            if (a - raw).length() < tol * 1.2 {
                                self.handle = Some(false);
                            } else if (b - raw).length() < tol * 1.2 {
                                self.handle = Some(true);
                            }
                        }
                    }
                    if self.handle.is_none() {
                        self.pick = self.hit(raw, tol, lib).or(self.pick);
                        self.moving = self.pick.map(|_| raw);
                    }
                }
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                if let (Some(end), Some(c), Some(Pick::Piece(i))) = (self.handle, cursor, self.pick) {
                    self.drag_end(i, end, c);
                } else if let (Some(from), Some(h)) = (self.moving, resp.interact_pointer_pos()) {
                    let now = from_screen(h);
                    let d = now - from;
                    self.moving = Some(now);
                    match self.pick {
                        Some(Pick::Piece(i)) if i < self.layout.pieces.len() => {
                            for s in &mut self.layout.pieces[i].segs {
                                s.x += d.x;
                                s.y += d.y;
                            }
                        }
                        Some(Pick::Object(i)) if i < self.layout.objects.len() => {
                            self.layout.objects[i].x += d.x;
                            self.layout.objects[i].y += d.y;
                        }
                        Some(Pick::Arm(i)) if i < self.layout.arms.len() => {
                            self.layout.arms[i].x += d.x;
                            self.layout.arms[i].y += d.y;
                            self.layout.arms[i].auto = false;
                        }
                        _ => {}
                    }
                }
            }
            if resp.drag_stopped() {
                self.moving = None;
                self.handle = None;
            }
        }
    }

    /// An end of the single-segment piece `i` dragged to `to`: a straight stays one, an arc
    /// keeps passing the point it passed in the middle.
    fn drag_end(&mut self, i: usize, end: bool, to: DVec2) {
        let Some(piece) = self.layout.pieces.get_mut(i) else { return };
        let [s] = piece.segs.as_mut_slice() else { return };
        let seg = s.seg();
        let (a, b, mid) = (seg.start, seg.end(), seg.point(seg.length / 2.0));
        let (na, nb) = if end { (a, to) } else { (to, b) };
        let made = if s.radius == 0.0 { roads::straight(na, nb) } else { cjunction::arc3(na, mid, nb) };
        if let Some(m) = made {
            *s = LSeg::of(&m);
        }
    }

    /// The piece or arm at `p` (within `tol` m).
    fn hit(&self, p: DVec2, tol: f64, lib: &Library) -> Option<Pick> {
        for (i, o) in self.layout.objects.iter().enumerate() {
            if (DVec2::new(o.x, o.y) - p).length() < tol * 1.6 {
                return Some(Pick::Object(i));
            }
        }
        for (i, a) in self.layout.arms.iter().enumerate() {
            if (DVec2::new(a.x, a.y) - p).length() < tol * 1.6 {
                return Some(Pick::Arm(i));
            }
        }
        // (the topmost piece under the point wins; of those, the nearest middle)
        let mut best: Option<(f64, usize)> = None;
        for (i, piece) in self.layout.pieces.iter().enumerate().filter(|(_, p)| !p.hidden && !p.auto) {
            let hw = half_of(lib, &piece.file).max(tol);
            let d = piece.distance_to(p);
            if d < hw && best.map(|b| d < b.0 || (d - b.0).abs() < 0.01).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|b| Pick::Piece(b.1))
    }

    /// Where an object clicked at `at` stands and which way it faces: on a piece (within its
    /// width) with Auto-Align it faces along it, turned by `obj_turn`.
    fn object_spot(&self, at: DVec2, lib: &Library) -> (DVec2, f64) {
        if self.auto_align {
            let mut best: Option<(f64, f64)> = None;
            for piece in self.layout.pieces.iter().filter(|p| !p.hidden) {
                let reach = half_of(lib, &piece.file) + 1.0;
                for s in &piece.segs {
                    let seg = s.seg();
                    let n = (s.length / 0.4).ceil().max(2.0) as usize;
                    for k in 0..=n {
                        let d = s.length * k as f64 / n as f64;
                        let dist = (seg.point(d) - at).length();
                        if dist < reach && best.map(|b| dist < b.0).unwrap_or(true) {
                            best = Some((dist, seg.heading_at(d)));
                        }
                    }
                }
            }
            if let Some((_, h)) = best {
                return (at, roads::norm_deg(h + self.obj_turn));
            }
        }
        (at, roads::norm_deg(self.obj_turn))
    }

    fn click(&mut self, at: DVec2, raw: DVec2, tol: f64, lib: &Library) {
        self.message = None;
        match self.tool {
            Tool::Select => self.pick = self.hit(raw, tol, lib),
            Tool::Line | Tool::Arc | Tool::Curve => {
                if self.file.is_none() {
                    self.message = Some((tr("Pick a spline type on the left first", "Сначала выберите тип сплайна слева").into(), true));
                    return;
                }
                if self.pending.last().map(|p| (*p - at).length() < 0.2).unwrap_or(false) {
                    return;
                }
                self.pending.push(at);
                if self.clicks_needed(lib) == Some(self.pending.len()) {
                    self.commit(lib);
                }
            }
            Tool::Object => {
                let Some(file) = self.obj_file.clone() else {
                    self.message = Some((tr("Pick an object on the left first", "Сначала выберите объект слева").into(), true));
                    return;
                };
                let (p, h) = self.object_spot(at, lib);
                self.layout.objects.push(LObject { file, x: p.x, y: p.y, z: 0.0, heading: h });
                self.pick = Some(Pick::Object(self.layout.objects.len() - 1));
            }
            Tool::Arm => {
                if self.pending.is_empty() {
                    self.pending.push(at);
                } else {
                    let from = self.pending.remove(0);
                    if (at - from).length() < 0.5 {
                        return;
                    }
                    let road = self.file.clone().filter(|f| is_road(lib, f)).unwrap_or_default();
                    self.layout.arms.push(Arm { x: from.x, y: from.y, out: roads::heading_of(at - from), road, auto: false });
                    self.pick = Some(Pick::Arm(self.layout.arms.len() - 1));
                }
            }
        }
    }

    /// Under the drawing: what it can be done with.
    fn bottom(&mut self, ui: &mut egui::Ui, ed: &mut Editor, content: Option<&PathBuf>, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {} · {} {}", self.layout.pieces.len(), tr("pieces", "деталей"), self.layout.arms.len(), tr("arms", "въездов"))).small().color(TEXT_DIM));
            if let Some((m, bad)) = &self.message {
                ui.colored_label(if *bad { Color32::from_rgb(240, 120, 100) } else { Color32::from_rgb(120, 200, 150) }, m);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Turn", "Поворот"));
            ui.add(egui::DragValue::new(&mut self.turn).speed(1.0).range(-360.0..=360.0).suffix("°"));
            let can = ed.doc.is_some() && !self.layout.pieces.is_empty();
            if ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::Add, tr("Put on the map", "Поставить на карту")))).on_hover_text(tr("Its pieces become ordinary splines at the middle of the view, to move and change like any other. Nothing about them is automatic", "Детали становятся обычными сплайнами в центре вида, их двигают и меняют как любые другие. Ничего автоматического")).clicked() {
                actions.push(Action::PlaceComplex(Box::new(self.layout.clone()), self.turn));
            }
            ui.separator();
            check(ui, &mut self.layout.auto, tr("Use in automatic junctions", "Использовать в авто-перекрёстках")).on_hover_text(tr("Where the Roads tool makes a junction by itself and the roads meet as the arms do (same number, facing about the same ways, of the same road types), this one is put there", "Где инструмент «Дороги» делает перекрёсток сам и дороги сходятся как въезды (то же число, примерно те же направления, те же типы дорог), ставится этот"));
            let can = content.is_some() && !self.layout.pieces.is_empty() && self.layout.arms.len() >= 3;
            if ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::Junction, tr("Save as a custom junction", "Сохранить как свой перекрёсток")))).on_hover_text(tr("Written as a junction piece of the content folder (it shows in the catalogue under Junctions) with lanes for the AI; needs three arms or more", "Записывается как перекрёсток в папке контента (виден в каталоге «Перекрёстки») с полосами для ИИ; нужно три въезда и больше")).clicked() {
                actions.push(Action::SaveCustomJunction(Box::new(self.layout.clone())));
            }
            if ui.add_enabled(content.is_some() && !self.layout.pieces.is_empty(), egui::Button::new(icons::labelled(Icon::Cube, tr("Save as an object", "Сохранить как объект")))).on_hover_text(tr("The whole drawing as one scenery object of the catalogue (its meshes, and lanes for the AI along its pieces), to place anywhere like any object; arms are not needed", "Весь рисунок как один объект каталога (меши и полосы для ИИ вдоль деталей), ставится где угодно как обычный объект; въезды не нужны")).clicked() {
                actions.push(Action::SaveComplexObject(Box::new(self.layout.clone())));
            }
            if ui.add_enabled(content.is_some() && !self.layout.pieces.is_empty(), egui::Button::new(tr("Save the drawing", "Сохранить рисунок"))).clicked() {
                if let Some(c) = content {
                    match self.layout.save(c) {
                        Ok(_) => {
                            self.saved = None;
                            self.message = Some((tr("Saved", "Сохранено").into(), false));
                        }
                        Err(e) => self.message = Some((e.to_string(), true)),
                    }
                }
            }
        });
        if self.skipped > 0 {
            ui.label(RichText::new(tr("A junction was not made: its roads are too short or too close together. Lengthen them or move the junctions apart.", "Перекрёсток не построен: дороги слишком короткие или стоят слишком близко. Удлините их или раздвиньте перекрёстки.")).small().color(Color32::from_rgb(235, 150, 80)));
        }
        ui.label(RichText::new(tr("A custom junction is cut back to its arms where roads meet it; draw the roads' own types at the arms so they join cleanly.", "Свой перекрёсток обрезает дороги по въездам; рисуйте у въездов те типы дорог, что подойдут.")).small().color(TEXT_DIM));
    }

    /// After a custom junction was written: the layout as saved.
    pub fn saved_as(&mut self, rel: String) {
        self.layout.baked = Some(rel);
        self.saved = None;
        self.message = Some((tr("Saved as a custom junction", "Сохранён как свой перекрёсток").into(), false));
    }

    pub fn succeeded(&mut self, what: String) {
        self.message = Some((what, false));
    }

    pub fn failed(&mut self, why: String) {
        self.message = Some((why, true));
    }
}

/// A piece drawn with its textures once its model has loaded.
#[allow(clippy::too_many_arguments)]
fn draw_one(pics: &mut TexCache, painter: &egui::Painter, to_screen: &dyn Fn(DVec2) -> Pos2, p: &Piece, lib: &Library, assets: &mut Option<&mut Assets>, ctx: &egui::Context, scale: f32, picked: bool, alpha: u8, guides: bool) {
    let hw = half_of(lib, &p.file);
    let cat = lib.find(&p.file).map(|a| a.category);
    // (its model, once loaded, gives its textures and where each surface lies)
    let model = assets.as_deref_mut().and_then(|a| a.spline(&p.file));
    let look = model.as_deref().map(|m| {
        let ids: Vec<Option<egui::TextureId>> = m.materials.iter().map(|mat| mat.texture.as_deref().and_then(|t| pics.get(ctx, t))).collect();
        (m, ids)
    });
    draw_piece(painter, to_screen, p, hw.max(1.2 / scale as f64), colour(cat, alpha), picked, scale, look.as_ref().map(|(m, ids)| (*m, ids.as_slice())), guides);
}

/// A piece as a band of its type's width along its segments.
fn draw_piece(painter: &egui::Painter, to_screen: &dyn Fn(DVec2) -> Pos2, p: &Piece, hw: f64, col: Color32, picked: bool, scale: f32, look: Option<(&SplineModel, &[Option<egui::TextureId>])>, guides: bool) {
    let textured = look.filter(|(m, _)| m.def.profiles.iter().any(|q| q.points.len() >= 2));
    let mut mesh = egui::epaint::Mesh::default();
    let mut centre_line: Vec<Pos2> = Vec::new();
    for s in &p.segs {
        let seg = s.seg();
        let n = ((s.length * scale as f64 / 6.0).ceil() as usize).clamp(2, 400);
        let mut prev: Option<(Pos2, Pos2)> = None;
        for k in 0..=n {
            let d = s.length * k as f64 / n as f64;
            let (c, h) = (seg.point(d), seg.heading_at(d));
            let r = roads::right(h);
            let (a, b) = (to_screen(c - r * hw), to_screen(c + r * hw));
            centre_line.push(to_screen(c));
            if let Some((pa, pb)) = prev {
                let i = mesh.vertices.len() as u32;
                for q in [pa, pb, a, b] {
                    mesh.vertices.push(egui::epaint::Vertex { pos: q, uv: egui::epaint::WHITE_UV, color: col });
                }
                mesh.indices.extend([i, i + 1, i + 2, i + 1, i + 3, i + 2]);
            }
            prev = Some((a, b));
        }
    }
    if col.a() == 0 {
        // (only its guides)
    } else if let Some((model, ids)) = textured {
        // each surface of the spline's profile seen from above, painted with its texture
        let mut chain = 0.0;
        for s in &p.segs {
            let seg = s.seg();
            let n = ((s.length * scale as f64 / 6.0).ceil() as usize).clamp(2, 400);
            for prof in &model.def.profiles {
                let tex = ids.get(prof.texture).copied().flatten();
                for w in prof.points.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    if (b.x - a.x).abs() < 0.02 {
                        continue;
                    }
                    let vs = if a.v_scale.abs() > 1e-4 { a.v_scale as f64 } else { 0.1 };
                    for k in 0..n {
                        let (d0, d1) = (s.length * k as f64 / n as f64, s.length * (k + 1) as f64 / n as f64);
                        let at = |d: f64, x: f32| {
                            let r = roads::right(seg.heading_at(d));
                            to_screen(seg.point(d) + r * x as f64)
                        };
                        let (v0, v1) = (((chain + d0) * vs) as f32, ((chain + d1) * vs) as f32);
                        quad(painter, tex, [at(d0, a.x), at(d0, b.x), at(d1, b.x), at(d1, a.x)], [Pos2::new(a.u, v0), Pos2::new(b.u, v0), Pos2::new(b.u, v1), Pos2::new(a.u, v1)], col);
                    }
                }
            }
            chain += s.length;
        }
    } else {
        painter.add(egui::Shape::mesh(mesh));
    }
    if !guides {
        return;
    }
    painter.add(egui::Shape::line(centre_line.clone(), Stroke::new(if picked { 2.0 } else { 1.0 }, if picked { Color32::from_rgb(255, 160, 60) } else { Color32::from_white_alpha(120) })));
    // its way
    if let Some(s) = p.segs.first() {
        let seg = s.seg();
        let (c, h) = (seg.point(s.length / 2.0), seg.heading_at(s.length / 2.0));
        let (f, r) = (roads::dir(h), roads::right(h));
        let m = 4.0 / scale as f64;
        let tip = to_screen(c + f * m);
        painter.add(egui::Shape::convex_polygon(vec![tip, to_screen(c - f * m * 0.4 + r * m * 0.6), to_screen(c - f * m * 0.4 - r * m * 0.6)], Color32::from_white_alpha(150), Stroke::NONE));
    }
}

/// The lanes a spline type has for the AI along a piece: a line at each lane's offset, an
/// arrow in the way it goes (green: vehicles, orange: people).
fn draw_lanes(painter: &egui::Painter, to_screen: &dyn Fn(DVec2) -> Pos2, piece: &Piece, model: &SplineModel, scale: f32) {
    for path in &model.def.paths {
        let col = if path.kind == 0 { Color32::from_rgb(110, 230, 150) } else { Color32::from_rgb(255, 180, 80) };
        let x = path.start[0] as f64;
        for s in &piece.segs {
            let seg = s.seg();
            let n = ((s.length * scale as f64 / 8.0).ceil() as usize).clamp(2, 300);
            let pts: Vec<Pos2> = (0..=n)
                .map(|k| {
                    let d = s.length * k as f64 / n as f64;
                    to_screen(seg.point(d) + roads::right(seg.heading_at(d)) * x)
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
            // an arrow at the middle (a lane that runs both ways has none)
            if path.direction != 2 {
                let d = s.length / 2.0;
                let h = seg.heading_at(d) + if path.direction == 1 { 180.0 } else { 0.0 };
                let c = seg.point(d) + roads::right(seg.heading_at(d)) * x;
                let (f, r) = (roads::dir(h), roads::right(h));
                let m = 4.0 / scale as f64;
                painter.add(egui::Shape::convex_polygon(vec![to_screen(c + f * m), to_screen(c - f * m * 0.6 + r * m * 0.6), to_screen(c - f * m * 0.6 - r * m * 0.6)], col, Stroke::NONE));
            }
        }
    }
}

/// One lane of a plate: a line along `seg` with an arrow in the way it goes (green: vehicles,
/// orange: people).
fn draw_lane_line(painter: &egui::Painter, to_screen: &dyn Fn(DVec2) -> Pos2, seg: &Seg, kind: i32, direction: i32, scale: f32) {
    let col = if kind == 0 { Color32::from_rgb(110, 230, 150) } else { Color32::from_rgb(255, 180, 80) };
    let n = ((seg.length * scale as f64 / 8.0).ceil() as usize).clamp(2, 300);
    let pts: Vec<Pos2> = (0..=n).map(|k| to_screen(seg.point(seg.length * k as f64 / n as f64))).collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
    if direction != 2 {
        let d = seg.length / 2.0;
        let h = seg.heading_at(d) + if direction == 1 { 180.0 } else { 0.0 };
        let c = seg.point(d);
        let (f, r) = (roads::dir(h), roads::right(h));
        let m = 4.0 / scale as f64;
        painter.add(egui::Shape::convex_polygon(vec![to_screen(c + f * m), to_screen(c - f * m * 0.6 + r * m * 0.6), to_screen(c - f * m * 0.6 - r * m * 0.6)], col, Stroke::NONE));
    }
}

/// An arm: a dot where the road joins and an arrow out along the road.
fn draw_arm(painter: &egui::Painter, to_screen: &dyn Fn(DVec2) -> Pos2, a: &Arm, picked: bool, scale: f32) {
    let col = if picked { Color32::from_rgb(255, 160, 60) } else { Color32::from_rgb(110, 200, 255) };
    let p = DVec2::new(a.x, a.y);
    let tip = p + roads::dir(a.out) * (28.0 / scale as f64);
    painter.line_segment([to_screen(p), to_screen(tip)], Stroke::new(2.5, col));
    let (f, r) = (roads::dir(a.out), roads::right(a.out));
    let m = 6.0 / scale as f64;
    painter.add(egui::Shape::convex_polygon(vec![to_screen(tip + f * m), to_screen(tip - f * m * 0.2 + r * m * 0.7), to_screen(tip - f * m * 0.2 - r * m * 0.7)], col, Stroke::NONE));
    painter.circle_filled(to_screen(p), 5.0, col);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> ComplexJunctions {
        ComplexJunctions { file: Some("Splines\\a.sli".into()), ..Default::default() }
    }

    #[test]
    fn two_clicks_lay_a_line_and_three_an_arc() {
        let lib = Library::default();
        let mut cj = editor();
        cj.tool = Tool::Line;
        cj.click(DVec2::new(0.0, 0.0), DVec2::ZERO, 0.5, &lib);
        assert!(cj.layout.pieces.is_empty() && cj.pending.len() == 1);
        cj.click(DVec2::new(0.0, 10.0), DVec2::ZERO, 0.5, &lib);
        assert_eq!(cj.layout.pieces.len(), 1);
        let s = cj.layout.pieces[0].segs[0];
        assert!((s.length - 10.0).abs() < 1e-9 && s.heading == 0.0 && s.radius == 0.0);
        assert_eq!(cj.pick, Some(Pick::Piece(0)));
        // an arc: start, a point on it, end
        cj.tool = Tool::Arc;
        for p in [DVec2::new(0.0, 20.0), DVec2::new(14.142135, 14.142135), DVec2::new(20.0, 0.0)] {
            cj.click(p, p, 0.5, &lib);
        }
        assert_eq!(cj.layout.pieces.len(), 2);
        assert!((cj.layout.pieces[1].segs[0].radius - 20.0).abs() < 1e-3);
    }

    #[test]
    fn nothing_is_drawn_without_a_spline_type_and_an_arm_takes_two_clicks() {
        let lib = Library::default();
        let mut cj = ComplexJunctions::default();
        cj.tool = Tool::Line;
        cj.click(DVec2::ZERO, DVec2::ZERO, 0.5, &lib);
        assert!(cj.pending.is_empty() && cj.message.is_some());
        cj.tool = Tool::Arm;
        cj.click(DVec2::new(5.0, 5.0), DVec2::ZERO, 0.5, &lib);
        cj.click(DVec2::new(5.0, 15.0), DVec2::ZERO, 0.5, &lib);
        assert_eq!(cj.layout.arms.len(), 1);
        let a = &cj.layout.arms[0];
        assert_eq!((a.x, a.y, a.out), (5.0, 5.0, 0.0), "faces north, the way the second click lies");
    }

    #[test]
    fn a_piece_is_found_where_it_lies() {
        let lib = Library::default();
        let mut cj = editor();
        cj.layout.pieces.push(Piece { file: "Splines\\a.sli".into(), segs: vec![LSeg { x: 0.0, y: 0.0, heading: 90.0, radius: 0.0, length: 10.0 }], lift: 0.0, hidden: false, auto: false });
        cj.layout.arms.push(Arm { x: 10.0, y: 0.0, out: 90.0, ..Default::default() });
        assert_eq!(cj.hit(DVec2::new(4.0, 0.5), 0.5, &lib), Some(Pick::Piece(0)));
        assert_eq!(cj.hit(DVec2::new(10.0, 0.2), 0.5, &lib), Some(Pick::Arm(0)), "an arm before the piece under it");
        assert_eq!(cj.hit(DVec2::new(4.0, 9.0), 0.5, &lib), None);
    }

    #[test]
    fn auto_align_goes_on_in_line_and_a_curve_is_one_piece_through_its_clicks() {
        let lib = crate::library::roads_for_tests(&[("Splines\\a.sli", 3.0)]);
        let mut cj = editor();
        cj.tool = Tool::Line;
        cj.click(DVec2::ZERO, DVec2::ZERO, 0.5, &lib);
        cj.click(DVec2::new(0.0, 10.0), DVec2::ZERO, 0.5, &lib);
        // from its end, a click a few degrees off the way it goes lands on the line
        cj.click(DVec2::new(0.0, 10.0), DVec2::ZERO, 0.5, &lib);
        cj.click(DVec2::new(1.5, 30.0), DVec2::ZERO, 0.5, &lib);
        let s = cj.layout.pieces[1].segs[0];
        assert!(s.heading.abs() < 1e-6 || (s.heading - 360.0).abs() < 1e-6, "{}", s.heading);
        // a tangent arc takes two clicks from that end
        cj.tool = Tool::Arc;
        cj.click(DVec2::new(0.0, 30.0), DVec2::ZERO, 0.5, &lib);
        cj.click(DVec2::new(10.0, 40.0), DVec2::ZERO, 0.5, &lib);
        assert_eq!(cj.layout.pieces.len(), 3);
        assert!((cj.layout.pieces[2].segs[0].radius - 10.0).abs() < 1e-6);
        // a curve through four points: one piece, ended by hand
        cj.tool = Tool::Curve;
        for p in [DVec2::new(30.0, 0.0), DVec2::new(40.0, 10.0), DVec2::new(30.0, 20.0), DVec2::new(20.0, 30.0)] {
            cj.click(p, p, 0.5, &lib);
        }
        assert_eq!(cj.layout.pieces.len(), 3);
        cj.commit(&lib);
        assert_eq!(cj.layout.pieces.len(), 4);
        assert!(cj.layout.pieces[3].segs.len() >= 3);
    }

    #[test]
    fn a_change_is_one_step_back_and_forward() {
        let lib = Library::default();
        let mut cj = editor();
        cj.tool = Tool::Line;
        cj.click(DVec2::ZERO, DVec2::ZERO, 0.5, &lib);
        cj.click(DVec2::new(0.0, 10.0), DVec2::ZERO, 0.5, &lib);
        cj.track_changes(&lib);
        cj.track_changes(&lib);
        assert_eq!(cj.undo.len(), 1);
        cj.undo_step();
        assert!(cj.layout.pieces.is_empty());
        cj.redo_step();
        assert_eq!(cj.layout.pieces.len(), 1);
    }
}
