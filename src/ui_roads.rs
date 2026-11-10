//! The Road types window: every road type with how its junctions are made, and the maker
//! of new road types (lanes, widths, footways, textures), written as ordinary OMSI splines.

use std::path::{Path, PathBuf};

use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};

use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::library::{Category, Kind, Library};
use crate::road_maker::{MarkStyle, Marking, RoadSpec};
use crate::ui::Action;
use crate::ui_tex::{quad, TexCache};

#[derive(Default)]
pub struct RoadTypes {
    pub open: bool,
    search: String,
    picked: Option<String>,
    /// The maker's road, when it is open.
    maker: Option<RoadSpec>,
    /// Pictures that can be a roadway or a footway (found once).
    textures: Option<Vec<PathBuf>>,
    tex_search: [String; 3],
    /// The road's textures as the previews show them.
    pics: TexCache,
    error: Option<String>,
    /// The drawing board for the markings.
    canvas: crate::ui_markpaint::MarkCanvas,
}

/// The pictures of the spline texture folders: what roads are painted with.
fn spline_textures(roots: &Roots) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots.all() {
        let Some(splines) = std::fs::read_dir(root).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("splines")).unwrap_or(false))) else { continue };
        let mut stack = vec![splines];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    // (not the seasons' copies)
                    let n = e.file_name().to_string_lossy().to_lowercase();
                    if !n.starts_with("winter") && !n.starts_with("spring") && !n.starts_with("fall") {
                        stack.push(p);
                    }
                } else if p.extension().map(|x| ["bmp", "dds", "tga", "png", "jpg"].contains(&x.to_string_lossy().to_lowercase().as_str())).unwrap_or(false) {
                    out.push(p);
                }
            }
        }
    }
    out.sort_by_key(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default());
    out.dedup_by_key(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default());
    out
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
}

/// The road seen from above, a stretch of it running up the picture: footways and kerbs, the
/// roadway with its lane lines (dashed between lanes of one way, solid at the middle), the
/// middle strip, the pocket, and arrows for the ways the lanes go.
fn plan(ui: &mut egui::Ui, s: &RoadSpec, tex: &mut TexCache) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::from_rgb(28, 30, 33));
    let total = s.total_width().max(1.0);
    let k = ((rect.width() - 30.0) / total).min((rect.height() - 20.0) / 14.0);
    let rw = s.roadway_width();
    let centre = rect.center().x + (s.footway_left() - s.footway_right() - s.pocket) / 2.0 * k;
    let x = |m: f32| centre + m * k;
    let (top, bottom) = (rect.top() + 6.0, rect.bottom() - 18.0);
    let ctx = ui.ctx().clone();
    let (road_t, foot_t) = (tex.get(&ctx, &s.roadway_tex), tex.get(&ctx, &s.footway_tex));
    let med_t = tex.get(&ctx, &s.median_tex);
    // a band across the road from `a` to `b` m, painted with a picture over the u range
    // `u` (v running along the road, `per_m` of it to the metre)
    let strip = |a: f32, b: f32, col: Color32, pic: Option<egui::TextureId>, u: (f32, f32), per_m: f32| {
        let v = (bottom - top) / k * per_m;
        quad(&p, pic, [Pos2::new(x(a), top), Pos2::new(x(b), top), Pos2::new(x(b), bottom), Pos2::new(x(a), bottom)], [Pos2::new(u.0, v), Pos2::new(u.1, v), Pos2::new(u.1, 0.0), Pos2::new(u.0, 0.0)], col);
    };
    let walk = Color32::from_rgb(150, 150, 145);
    strip(-rw / 2.0 - s.footway_left(), -rw / 2.0, walk, foot_t, (0.187, 0.953), 0.2);
    strip(rw / 2.0 + s.pocket, rw / 2.0 + s.pocket + s.footway_right(), walk, foot_t, (0.953, 0.187), 0.2);
    strip(-rw / 2.0, rw / 2.0, Color32::from_rgb(70, 72, 76), road_t, (0.0, 1.0), 0.1);
    if s.pocket > 0.0 {
        let u = 0.5 / (s.lanes_fwd + s.lanes_back).max(1) as f32;
        strip(rw / 2.0, rw / 2.0 + s.pocket, Color32::from_rgb(86, 90, 104), road_t, (u - 0.01, u + 0.01), 0.1);
    }
    // kerbs
    let kerb = |m: f32| {
        if s.kerb > 0.0 {
            p.line_segment([Pos2::new(x(m), top), Pos2::new(x(m), bottom)], Stroke::new(2.0, Color32::from_gray(205)));
        }
    };
    if s.footway_left() > 0.0 {
        kerb(-rw / 2.0);
    }
    if s.footway_right() > 0.0 {
        kerb(rw / 2.0 + s.pocket);
    }
    let mid = s.median_width();
    let two_way = s.lanes_fwd > 0 && s.lanes_back > 0;
    if mid > 0.0 {
        let col = if s.median_tex.as_os_str().is_empty() { Color32::from_rgb(90, 92, 96) } else { Color32::from_rgb(86, 128, 74) };
        let (ml, mr) = s.median_edges();
        strip(ml, mr, col, med_t, (0.0, 1.0), 0.2);
    }
    let dash = |m: f32, col: Color32, solid: bool| {
        let mut y = bottom;
        while y > top {
            let y2 = if solid { top } else { (y - 1.5 * k).max(top) };
            p.line_segment([Pos2::new(x(m), y), Pos2::new(x(m), y2)], Stroke::new(1.5, col));
            if solid {
                break;
            }
            y -= 3.0 * k;
        }
    };
    let white = Color32::from_gray(225);
    let yellow = Color32::from_rgb(232, 190, 60);
    let gap = if two_way { mid / 2.0 } else { 0.0 };
    let arrow = |m: f32, up: bool, col: Color32| {
        // (drawn, not written: a lane's arrow must show whatever fonts there are)
        let dir = if up { -1.0 } else { 1.0 };
        for y in [bottom - 22.0, top + 40.0] {
            p.add(egui::Shape::convex_polygon(vec![Pos2::new(x(m), y + dir * 8.0), Pos2::new(x(m) - 5.0, y - dir * 6.0), Pos2::new(x(m) + 5.0, y - dir * 6.0)], col, Stroke::NONE));
        }
    };
    let lanes = |first: f32, n: u8, up: bool| {
        for i in 0..n {
            let l = first + i as f32 * s.lane_width;
            arrow(l + s.lane_width / 2.0, up, Color32::from_rgb(if up { 232 } else { 110 }, if up { 170 } else { 180 }, if up { 60 } else { 240 }));
            if i > 0 && !s.marked {
                dash(l, white, false);
            }
        }
    };
    let offset = s.centerline();
    if two_way {
        lanes(offset - gap - s.lanes_back as f32 * s.lane_width, s.lanes_back, false);
        lanes(offset + gap, s.lanes_fwd, true);
        if mid <= 0.0 && !s.marked {
            dash(offset - 0.1, yellow, true);
            dash(offset + 0.1, yellow, true);
        }
    } else {
        lanes(-rw / 2.0, s.lanes_fwd + s.lanes_back, s.lanes_fwd > 0);
    }
    if s.marked {
        // the painted lines, as the texture will have them (dashes at their real length)
        for m in &s.markings {
            let col = Color32::from_rgb(m.color[0], m.color[1], m.color[2]);
            let period = s.dash_period(m);
            for (lx, dashed) in m.lines_at() {
                let mut y = bottom;
                while y > top {
                    let y2 = if dashed { (y - m.dash.min(period) * k).max(top) } else { top };
                    p.line_segment([Pos2::new(x(lx), y), Pos2::new(x(lx), y2)], Stroke::new((m.width * k).max(1.0), col));
                    if !dashed {
                        break;
                    }
                    y -= period * k;
                }
            }
        }
    } else if s.pocket > 0.0 {
        dash(rw / 2.0, white, false);
    }
    if s.pocket > 0.0 {
        arrow(rw / 2.0 + s.pocket / 2.0, true, Color32::from_rgb(120, 200, 150));
        p.text(Pos2::new(x(rw / 2.0 + s.pocket / 2.0), (top + bottom) / 2.0), egui::Align2::CENTER_CENTER, "BUS", egui::FontId::proportional(11.0), Color32::from_rgb(120, 200, 150));
    }
    // 5 m scale
    let sy = rect.bottom() - 9.0;
    p.line_segment([Pos2::new(rect.left() + 10.0, sy), Pos2::new(rect.left() + 10.0 + 5.0 * k, sy)], Stroke::new(2.0, Color32::from_gray(170)));
    p.text(Pos2::new(rect.left() + 16.0 + 5.0 * k, sy), egui::Align2::LEFT_CENTER, "5 m", egui::FontId::proportional(11.0), Color32::from_gray(170));
    p.text(Pos2::new(rect.right() - 8.0, sy), egui::Align2::RIGHT_CENTER, format!("{:.1} m", total), egui::FontId::proportional(11.0), Color32::from_gray(170));
}

/// The spline seen from above or across it (the choice is kept while the maker is open).
fn preview(ui: &mut egui::Ui, s: &RoadSpec, tex: &mut TexCache) {
    let key = egui::Id::new("road maker view");
    let mut top: bool = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || true));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut top, true, icons::labelled(Icon::Map, tr("Top view", "Вид сверху")));
        ui.selectable_value(&mut top, false, icons::labelled(Icon::SideView, tr("Side view", "Вид сбоку")));
    });
    ui.data_mut(|d| d.insert_temp(key, top));
    if top { plan(ui, s, tex) } else { section(ui, s, tex) }
}

/// The road's cross-section, drawn as one looks along it.
fn section(ui: &mut egui::Ui, s: &RoadSpec, tex: &mut TexCache) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 100.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::from_rgb(28, 30, 33));
    let total = s.total_width().max(1.0);
    let k = (rect.width() - 20.0) / total;
    let rw = s.roadway_width();
    // (the road's middle is the origin; the footways and the pocket make the picture lean to
    // the wider side)
    let centre = rect.center().x + (s.footway_left() - s.footway_right() - s.pocket) / 2.0 * k;
    let x = |m: f32| centre + m * k;
    let base = rect.bottom() - 26.0;
    let lift = (s.kerb * k * 4.0).min(16.0);
    let ctx = ui.ctx().clone();
    let (road_t, foot_t, med_t) = (tex.get(&ctx, &s.roadway_tex), tex.get(&ctx, &s.footway_tex), tex.get(&ctx, &s.median_tex));
    // a surface's top seen end on, painted with its picture over the u range `u`
    let band = |a: f32, b: f32, y0: f32, col: Color32, pic: Option<egui::TextureId>, u: (f32, f32)| {
        quad(&p, pic, [Pos2::new(x(a), y0), Pos2::new(x(b), y0), Pos2::new(x(b), base), Pos2::new(x(a), base)], [Pos2::new(u.0, 0.0), Pos2::new(u.1, 0.0), Pos2::new(u.1, 0.4), Pos2::new(u.0, 0.4)], col);
    };
    // footways
    let walk = Color32::from_rgb(150, 150, 145);
    if s.footway_left() > 0.0 {
        band(-rw / 2.0 - s.footway_left(), -rw / 2.0, base - lift - 8.0, walk, foot_t, (0.187, 0.953));
    }
    if s.footway_right() > 0.0 {
        band(rw / 2.0 + s.pocket, rw / 2.0 + s.pocket + s.footway_right(), base - lift - 8.0, walk, foot_t, (0.953, 0.187));
    }
    // the roadway, lanes, the middle
    band(-rw / 2.0, rw / 2.0, base - 8.0, Color32::from_rgb(70, 72, 76), road_t, (0.0, 1.0));
    if s.pocket > 0.0 {
        let u = 0.5 / (s.lanes_fwd + s.lanes_back).max(1) as f32;
        band(rw / 2.0, rw / 2.0 + s.pocket, base - 8.0, Color32::from_rgb(86, 90, 104), road_t, (u - 0.01, u + 0.01));
    }
    let two_way = s.lanes_fwd > 0 && s.lanes_back > 0;
    let mid = s.median_width();
    if mid > 0.0 {
        let own = !s.median_tex.as_os_str().is_empty();
        let col = if own { Color32::from_rgb(86, 128, 74) } else { Color32::from_rgb(90, 92, 96) };
        let up = if s.median_raised { lift } else { 0.0 };
        let (ml, mr) = s.median_edges();
        band(ml, mr, base - 8.0 - up, col, if own { med_t } else { road_t }, (0.0, 1.0));
    }
    let gap = if two_way { mid / 2.0 } else { 0.0 };
    let lane = |left: f32, fwd: bool| {
        let c = x(left + s.lane_width / 2.0);
        let y = base - 18.0;
        let col = if fwd { Color32::from_rgb(232, 170, 60) } else { Color32::from_rgb(110, 180, 240) };
        p.text(Pos2::new(c, y), egui::Align2::CENTER_CENTER, if fwd { Icon::Up.ch() } else { Icon::Down.ch() }, Icon::font(14.0), col);
        p.line_segment([Pos2::new(x(left), base - 8.0), Pos2::new(x(left), base)], Stroke::new(1.0, Color32::from_gray(200)));
    };
    let offset = s.centerline();
    if two_way {
        for i in 0..s.lanes_back {
            lane(offset - gap - (s.lanes_back - i) as f32 * s.lane_width, false);
        }
        for i in 0..s.lanes_fwd {
            lane(offset + gap + i as f32 * s.lane_width, true);
        }
    } else {
        for i in 0..(s.lanes_fwd + s.lanes_back) {
            lane(-rw / 2.0 + i as f32 * s.lane_width, s.lanes_fwd > 0);
        }
    }
    p.text(Pos2::new(rect.center().x, rect.bottom() - 10.0), egui::Align2::CENTER_CENTER, format!("{:.1} m · {} {:.1} m", total, tr("roadway", "проезжая часть"), rw), egui::FontId::proportional(11.0), Color32::from_gray(170));
}

impl RoadTypes {
    /// The window open on a new road type in the maker (a check hook).
    /// The maker's road with the standard markings painted (for checks).
    pub fn mark_maker(&mut self) {
        if let Some(s) = &mut self.maker {
            s.marked = true;
            s.markings = s.standard_markings();
        }
    }

    /// The drawing board open on the maker's road with a few strokes drawn (for checks).
    pub fn mark_canvas_sample(&mut self) {
        if let Some(s) = &mut self.maker {
            let (cw, len) = (s.canvas_width(), s.mark_period());
            let mut h = crate::road_maker::Hand::default();
            h.stroke(cw, len, (1.0, 1.0), (3.0, 8.0), 0.1, crate::road_maker::YELLOW, false);
            h.fill(cw, len, &crate::road_maker::arrow_polygon((cw * 0.75, 5.0), false), crate::road_maker::WHITE, false);
            h.stroke(cw, len, (cw - 0.5, 9.0), (cw - 0.5, 11.5), 0.15, crate::road_maker::WHITE, false);
            s.hand = Some(std::sync::Arc::new(h));
            self.canvas.open = true;
        }
    }

    pub fn open_maker(&mut self, pocket: bool) {
        self.open = true;
        self.maker = Some(RoadSpec { name: tr("My road", "Моя дорога").into(), pocket: if pocket { 3.0 } else { 0.0 }, ..Default::default() });
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, roots: Option<&Roots>, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let Some(roots) = roots else { return };
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Road, tr("Road types", "Типы дорог"))).open(&mut open).default_size(Vec2::new(760.0, 520.0)).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                // the list
                ui.vertical(|ui| {
                    ui.set_width(260.0);
                    if ui.button(icons::labelled(Icon::Add, tr("Make a new road type…", "Создать тип дороги…"))).clicked() {
                        self.maker = Some(RoadSpec { name: tr("My road", "Моя дорога").into(), ..Default::default() });
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("Search", "Поиск")));
                    egui::ScrollArea::vertical().id_salt("road types").max_height(440.0).show(ui, |ui| {
                        for cat in [Category::Roads, Category::Tracks] {
                            for a in lib.filter(Kind::Spline, Some(cat), &self.search) {
                                let on = self.picked.as_deref().map(|p| p.eq_ignore_ascii_case(&a.rel)).unwrap_or(false);
                                let mut text = a.name.clone();
                                if a.from_content {
                                    text.push_str(tr(" · own", " · своя"));
                                }
                                if ui.selectable_label(on, text).on_hover_text(&a.rel).clicked() {
                                    self.picked = Some(a.rel.clone());
                                    self.maker = None;
                                }
                            }
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let mut cancel = false;
                    if let Some(spec) = &mut self.maker {
                        Self::maker_ui(ui, spec, &mut self.textures, &mut self.tex_search, roots, &mut self.pics, &mut self.canvas.open);
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button(icons::labelled(Icon::CheckCircle, tr("Create the road type", "Создать тип дороги"))).clicked() {
                                match spec.write(&roots.content) {
                                    Ok(rel) => {
                                        if spec.pocket > 0.0 {
                                            ed.pocket_type = Some(rel.clone());
                                        }
                                        ed.road_type = Some(rel.clone());
                                        ed.set_tool(crate::editor::Tool::Roads);
                                        self.picked = Some(rel);
                                        self.error = None;
                                        actions.push(Action::ReloadContent(tr("Road type made: the Roads tool builds with it", "Тип дороги создан: инструмент «Дороги» строит им").to_string()));
                                    }
                                    Err(e) => self.error = Some(e.to_string()),
                                }
                            }
                            if ui.button(tr("Cancel", "Отмена")).clicked() {
                                cancel = true;
                            }
                        });
                    } else if let Some(rel) = self.picked.clone() {
                        let name = lib.find(&rel).map(|a| a.name.clone()).unwrap_or_else(|| rel.clone());
                        ui.label(RichText::new(name).strong().color(Color32::WHITE));
                        crate::ui::long_label(ui, &rel, egui::TextStyle::Small, Color32::from_gray(140));
                        ui.add_space(6.0);
                        if ui.button(icons::labelled(Icon::Road, tr("Build with it", "Строить им"))).clicked() {
                            ed.road_type = Some(rel.clone());
                            ed.set_tool(crate::editor::Tool::Roads);
                        }
                        let path = roots.resolve(&rel);
                        if let Some(spec) = RoadSpec::of(&path) {
                            if ui.button(icons::labelled(Icon::Settings, tr("Change it in the road maker", "Изменить в конструкторе"))).clicked() {
                                self.maker = Some(spec);
                            }
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new(tr("Junction pieces for it", "Перекрёстки для неё")).strong());
                        let (pieces, fits) = lib.junction_pieces(&rel);
                        let preferred = ed.road_styles.piece(&rel).map(str::to_string);
                        let mut pick = preferred.clone();
                        ui.horizontal(|ui| {
                            let shown = preferred.as_deref().and_then(|p| lib.find(p)).map(|a| a.name.clone()).unwrap_or_else(|| tr("made by openMapper", "делает openMapper").to_string());
                            egui::ComboBox::from_id_salt("preferred piece").width(260.0).height(300.0).selected_text(shown).show_ui(ui, |ui| {
                                if ui.selectable_label(preferred.is_none(), tr("made by openMapper", "делает openMapper")).clicked() {
                                    pick = None;
                                }
                                for a in pieces.iter().take(300) {
                                    if ui.selectable_label(preferred.as_deref().map(|p| p.eq_ignore_ascii_case(&a.rel)).unwrap_or(false), format!("{}  ·  {}", a.name, a.dir)).clicked() {
                                        pick = Some(a.rel.clone());
                                    }
                                }
                            });
                            if let Some(p) = preferred.as_deref() {
                                if ui.button(icons::labelled(Icon::Junction, tr("Place it", "Поставить"))).on_hover_text(tr("Choose it in the catalogue to place on the map", "Выбрать в каталоге для установки на карту")).clicked() {
                                    actions.push(Action::PlaceAsset(p.to_string()));
                                }
                            }
                        });
                        ui.label(RichText::new(if pieces.is_empty() {
                            tr("The installation has no junction pieces with places for roads.", "В установке нет перекрёстков с местами для дорог.")
                        } else if fits {
                            tr("Pieces with a place made for this road type. The one picked is offered first when a junction is put on this road's end (select the road: Junction on an end).", "Перекрёстки с местом под этот тип дороги. Выбранный предлагается первым, когда перекрёсток ставят на конец такой дороги (выделите дорогу: «Перекрёсток на конце дороги»).")
                        } else {
                            tr("No piece is made for this road type; the list holds every piece with places for roads.", "Под этот тип дороги перекрёстков нет; в списке все перекрёстки с местами для дорог.")
                        }).small().color(Color32::from_gray(140)));
                        if pick != preferred {
                            ed.road_styles.set_piece(&rel, pick);
                            if let Err(e) = ed.road_styles.save(&roots.content) {
                                self.error = Some(e.to_string());
                            }
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new(tr("Its junctions", "Его перекрёстки")).strong());
                        let mut st = ed.road_styles.get(&rel);
                        let before = st;
                        crate::ui::check(ui, &mut st.junctions, tr("Make a junction where it meets another road (else they only cross)", "Делать перекрёсток, где она встречает другую дорогу (иначе просто пересекаются)"));
                        ui.horizontal(|ui| {
                            ui.label(tr("Corners", "Углы"));
                            ui.add(egui::Slider::new(&mut st.corner, 0.5..=3.0).text(tr("× rounder", "× круглее")));
                        });
                        crate::ui::check(ui, &mut st.lights, tr("Traffic lights by themselves", "Светофоры сами"));
                        crate::ui::check(ui, &mut st.crossings, tr("Zebra crossings on every arm", "Пешеходные переходы на всех въездах"));
                        ui.label(RichText::new(tr("Used for the junctions made from now on (where several road types meet, the one asking more wins). A junction already made keeps its own settings: select it to change them.", "Применяется к новым перекрёсткам (где встречаются разные дороги, берётся большее). Готовый перекрёсток хранит свои настройки: выделите его, чтобы изменить.")).small().color(Color32::from_gray(140)));
                        if st != before {
                            ed.road_styles.set(&rel, st);
                            if let Err(e) = ed.road_styles.save(&roots.content) {
                                self.error = Some(e.to_string());
                            }
                        }
                    } else {
                        ui.label(tr("Pick a road type on the left, or make a new one.", "Выберите тип дороги слева или создайте новый."));
                    }
                    if let Some(e) = &self.error {
                        ui.colored_label(Color32::from_rgb(240, 120, 100), e);
                    }
                    if cancel {
                        self.maker = None;
                    }
                });
            });
        });
        match &mut self.maker {
            Some(spec) => self.canvas.window(ctx, spec),
            None => self.canvas.open = false,
        }
        self.open = open;
    }

    fn maker_ui(ui: &mut egui::Ui, s: &mut RoadSpec, textures: &mut Option<Vec<PathBuf>>, search: &mut [String; 3], roots: &Roots, tex: &mut TexCache, canvas: &mut bool) {
        ui.label(RichText::new(tr("Road maker", "Конструктор дороги")).strong().color(Color32::WHITE));
        ui.horizontal(|ui| {
            ui.label(tr("Start from", "Взять за основу"));
            egui::ComboBox::from_id_salt("road preset").selected_text(tr("a ready-made road…", "готовую дорогу…")).show_ui(ui, |ui| {
                for (label, p) in crate::road_maker::presets() {
                    if ui.selectable_label(false, label).clicked() {
                        // (the name and the pictures stay: they are the user's)
                        *s = RoadSpec { name: std::mem::take(&mut s.name), roadway_tex: std::mem::take(&mut s.roadway_tex), footway_tex: std::mem::take(&mut s.footway_tex), median_tex: std::mem::take(&mut s.median_tex), ..p };
                    }
                }
            });
        });
        egui::Grid::new("road maker").num_columns(2).spacing(Vec2::new(10.0, 5.0)).show(ui, |ui| {
            ui.label(tr("Name", "Название"));
            ui.text_edit_singleline(&mut s.name);
            ui.end_row();
            ui.label(tr("Lanes", "Полосы"));
            ui.horizontal(|ui| {
                ui.label(Icon::Up.text(15.0)).on_hover_text(tr("Lanes the way the road is drawn", "Полосы по ходу дороги"));
                ui.add(egui::DragValue::new(&mut s.lanes_fwd).range(0..=5));
                ui.label(Icon::Down.text(15.0)).on_hover_text(tr("Lanes against it", "Полосы против хода"));
                ui.add(egui::DragValue::new(&mut s.lanes_back).range(0..=5));
                ui.label(RichText::new(tr("(0 against: one-way)", "(0 против: одностороннее)")).small().color(Color32::from_gray(140)));
            });
            ui.end_row();
            ui.label(tr("Lane width", "Ширина полосы"));
            ui.add(egui::Slider::new(&mut s.lane_width, 2.5..=5.0).suffix(" m"));
            ui.end_row();
            ui.label(tr("Middle strip", "Разделитель"));
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut s.median, 0.0..=12.0).suffix(" m"));
                ui.checkbox(&mut s.median_raised, tr("raised", "приподнят")).on_hover_text(tr("As high as the footways, with a kerb along it", "Вровень с тротуарами, с бордюром вдоль"));
            });
            ui.end_row();
            let mut apart = s.footway_right.is_some();
            ui.label(tr("Footways", "Тротуары"));
            ui.vertical(|ui| {
                ui.add(egui::Slider::new(&mut s.footway, 0.0..=8.0).suffix(if apart { tr(" m left", " м слева") } else { " m" }));
                if ui.checkbox(&mut apart, tr("a different one on the right", "справа другой")).changed() {
                    s.footway_right = if apart { Some(s.footway) } else { None };
                }
                if let Some(r) = &mut s.footway_right {
                    ui.add(egui::Slider::new(r, 0.0..=8.0).suffix(tr(" m right", " м справа")));
                }
            });
            ui.end_row();
            ui.label(tr("Kerb", "Бордюр"));
            ui.add(egui::Slider::new(&mut s.kerb, 0.0..=0.3).suffix(" m"));
            ui.end_row();
            ui.label(tr("Bus pocket", "Автобусный карман"));
            ui.add(egui::Slider::new(&mut s.pocket, 0.0..=4.0).suffix(" m")).on_hover_text(tr("A lane for buses to stop in, right of the roadway, before the footway. Lay a road type made so where a bay is wanted with the Bus pocket tool of the Roads tool", "Полоса для остановки автобусов справа от проезжей части, перед тротуаром. Дорогой такого типа делают карман инструментом «Автобусный карман» в «Дорогах»"));
            ui.end_row();
        });
        preview(ui, s, tex);
        let list = textures.get_or_insert_with(|| spline_textures(roots));
        let has_median = s.median_width() > 0.0;
        for (i, (label, hint)) in [(tr("Roadway texture", "Текстура проезжей части"), "asph"), (tr("Footway texture", "Текстура тротуара"), "side"), (tr("Middle strip texture", "Текстура разделителя"), "gras")].into_iter().enumerate() {
            if i == 2 && !has_median {
                continue;
            }
            let cur = match i { 0 => &mut s.roadway_tex, 1 => &mut s.footway_tex, _ => &mut s.median_tex };
            // (a sensible one to start with: the stock asphalt or footway; the strip may keep
            // the roadway's own)
            if cur.as_os_str().is_empty() && i < 2 {
                let want = if i == 0 { "str_asphdrk_2line.bmp" } else { "str_side1.bmp" };
                if let Some(p) = list.iter().find(|p| file_name(p).eq_ignore_ascii_case(want)).or_else(|| list.iter().find(|p| file_name(p).to_lowercase().contains(hint))) {
                    *cur = p.clone();
                }
            }
            ui.horizontal(|ui| {
                ui.label(label);
                egui::ComboBox::from_id_salt(("road tex", i)).width(260.0).selected_text(if cur.as_os_str().is_empty() { tr("like the roadway", "как у проезжей части").to_string() } else { file_name(cur) }).show_ui(ui, |ui| {
                    if i == 2 && ui.selectable_label(cur.as_os_str().is_empty(), tr("like the roadway", "как у проезжей части")).clicked() {
                        cur.clear();
                    }
                    ui.add(egui::TextEdit::singleline(&mut search[i]).hint_text(tr("Search", "Поиск")));
                    let q = search[i].to_lowercase();
                    for p in list.iter().filter(|p| q.is_empty() || file_name(p).to_lowercase().contains(&q)).take(300) {
                        if ui.selectable_label(p == cur, file_name(p)).clicked() {
                            *cur = p.clone();
                        }
                    }
                });
                if ui.small_button(tr("Own…", "Своя…")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "bmp", "tga", "dds"]).pick_file() {
                        *cur = f;
                    }
                }
            });
        }
        egui::CollapsingHeader::new(RichText::new(tr("Road markings", "Дорожная разметка")).strong().color(Color32::WHITE)).id_salt("road markings").default_open(s.marked).show(ui, |ui| {
            ui.checkbox(&mut s.marked, tr("Paint my own markings", "Нарисовать свою разметку")).on_hover_text(tr("The maker paints the lines onto the roadway texture: pick a plain asphalt texture, or its own lines show too", "Конструктор рисует линии на текстуре проезжей части: берите гладкий асфальт, иначе проступят и его собственные линии"));
            if !s.marked {
                return;
            }
            ui.horizontal(|ui| {
                if ui.button(tr("Standard for these lanes", "Типовая для этих полос")).clicked() {
                    s.markings = s.standard_markings();
                }
                if ui.button(tr("Add a line", "Добавить линию")).clicked() {
                    s.markings.push(Marking::default());
                }
                if ui.button(tr("Clear", "Очистить")).clicked() {
                    s.markings.clear();
                }
            });
            ui.horizontal(|ui| {
                if ui.button(icons::labelled(Icon::Paint, tr("Draw by hand…", "Рисовать вручную…"))).on_hover_text(tr("A drawing board with a pen, lines, arrows and an eraser: what you draw goes onto the roadway", "Холст с кистью, линиями, стрелками и ластиком: нарисованное ложится на проезжую часть")).clicked() {
                    *canvas = true;
                    s.marked = true;
                }
                if s.hand.as_ref().map(|h| !h.is_empty()).unwrap_or(false) {
                    ui.label(RichText::new(tr("a drawing is on the road", "на дороге есть рисунок")).small().color(Color32::from_gray(150)));
                }
            });
            let half = s.roadway_width() / 2.0;
            let mut remove = None;
            for (i, m) in s.markings.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::DragValue::new(&mut m.x).speed(0.05).range(-half..=half).fixed_decimals(2).suffix(" m")).on_hover_text(tr("Across the roadway from its middle (right of the road's way is plus)", "Поперёк проезжей части от середины (справа по ходу дороги: плюс)"));
                    egui::ComboBox::from_id_salt(("mark style", i)).width(110.0).selected_text(match m.style {
                        MarkStyle::Solid => tr("Solid", "Сплошная"),
                        MarkStyle::Dashed => tr("Dashed", "Прерывистая"),
                        MarkStyle::Double => tr("Double solid", "Двойная сплошная"),
                        MarkStyle::SolidDashed => tr("Solid and dashed", "Сплошная и прерывистая"),
                    }).show_ui(ui, |ui| {
                        for (st, label) in [(MarkStyle::Solid, tr("Solid", "Сплошная")), (MarkStyle::Dashed, tr("Dashed", "Прерывистая")), (MarkStyle::Double, tr("Double solid", "Двойная сплошная")), (MarkStyle::SolidDashed, tr("Solid and dashed", "Сплошная и прерывистая"))] {
                            ui.selectable_value(&mut m.style, st, label);
                        }
                    });
                    ui.color_edit_button_srgb(&mut m.color);
                    ui.add(egui::DragValue::new(&mut m.width).speed(0.01).range(0.05..=0.6).fixed_decimals(2).suffix(" m")).on_hover_text(tr("The line's width", "Ширина линии"));
                    if matches!(m.style, MarkStyle::Dashed | MarkStyle::SolidDashed) {
                        ui.add(egui::DragValue::new(&mut m.dash).speed(0.1).range(0.5..=12.0).fixed_decimals(1).suffix(" m")).on_hover_text(tr("Dash length", "Длина штриха"));
                        ui.add(egui::DragValue::new(&mut m.gap).speed(0.1).range(0.5..=20.0).fixed_decimals(1).suffix(" m")).on_hover_text(tr("Gap between dashes", "Промежуток между штрихами"));
                    }
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                s.markings.remove(i);
            }
        });
        ui.label(RichText::new(tr("The roadway texture is laid once across all its lanes (as OMSI 2's lane textures are painted: pick one with as many lanes); the footway texture's right end is the kerb.", "Текстура проезжей части ложится один раз поперёк всех полос (как нарисованы полосные текстуры OMSI 2: берите с тем же числом полос); правый край текстуры тротуара: бордюр.")).small().color(Color32::from_gray(140)));
    }
}
