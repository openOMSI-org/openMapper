//! The window of Map > GPS tracks to trace: files of tracks loaded, each line shown in the
//! view or not, and built into a road of the Roads tool's type (see `trace`).

use egui::RichText;
use glam::DVec2;

use crate::editor::Editor;
use crate::geo::Georef;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::trace::Trace;
use crate::theme::pal;

#[derive(Default)]
pub struct TraceWindow {
    pub open: bool,
    traces: Vec<Trace>,
    error: Option<String>,
    /// A line to build into a road (world points), taken by the app after the interface.
    pub build: Option<(Vec<DVec2>, String)>,
    /// The first line to build into a road of this type once the map is placed.
    build_first: Option<String>,
    /// What the view was last given (the placement and which lines).
    shown_key: Option<(u64, Vec<bool>, usize)>,
}

impl TraceWindow {
    pub fn add_file(&mut self, path: &std::path::Path) {
        match crate::trace::load(path) {
            Ok(t) => {
                self.traces.extend(t);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    /// The first line built into a road of `road` once the map is placed (checks without a
    /// person at the screen).
    pub fn build_first(&mut self, road: String) {
        self.build_first = Some(road);
    }

    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library, georef: Option<Georef>) {
        // the lines shown go to the view in world metres (again only when they change)
        let key = (georef.map(|g| g.stamp()).unwrap_or(0), self.traces.iter().map(|t| t.shown).collect::<Vec<_>>(), self.traces.len());
        if self.shown_key.as_ref() != Some(&key) {
            ed.trace_lines = match georef {
                Some(g) => self.traces.iter().filter(|t| t.shown).map(|t| t.points.iter().map(|(la, lo)| g.world(*la, *lo)).collect()).collect(),
                None => Vec::new(),
            };
            self.shown_key = Some(key);
        }
        if let (Some(g), Some(t), Some(road)) = (georef, self.traces.first(), self.build_first.clone()) {
            self.build = Some((t.points.iter().map(|(la, lo)| g.world(*la, *lo)).collect(), road));
            self.build_first = None;
        }
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Gps, tr("GPS tracks to trace", "GPS-треки для обводки"))).id(egui::Id::new("trace window")).open(&mut open).default_pos([360.0, 120.0]).default_width(340.0).resizable(false).show(ctx, |ui| {
            ui.label(RichText::new(tr("A route recorded on a phone or drawn elsewhere (GPX, KML, GeoJSON), shown in the view in magenta to draw roads along, or built into a road at once", "Маршрут, записанный телефоном или нарисованный в другой программе (GPX, KML, GeoJSON): показан в окне малиновым, чтобы рисовать дороги по нему, или сразу строится дорогой")).small().color(pal().text_dim));
            if georef.is_none() {
                ui.colored_label(pal().warn, tr("Place the map on the Earth first (Map > Satellite and terrain)", "Сначала поместите карту на Землю (Карта > Спутник и рельеф)"));
            }
            if ui.button(icons::labelled(Icon::Open, tr("Load a file…", "Загрузить файл…"))).clicked() {
                if let Some(files) = rfd::FileDialog::new().add_filter("GPX, KML, GeoJSON", &["gpx", "kml", "geojson", "json"]).set_title(tr("GPS tracks to trace", "GPS-треки для обводки")).pick_files() {
                    for f in files {
                        self.add_file(&f);
                    }
                }
            }
            if let Some(e) = &self.error {
                ui.colored_label(pal().error, e);
            }
            ui.separator();
            if self.traces.is_empty() {
                ui.label(RichText::new(tr("None loaded", "Ничего не загружено")).italics().color(pal().text_dim));
            }
            let road = ed.road_type.clone();
            let road_name = road.as_deref().map(|r| lib.find(r).map(|a| a.name.clone()).unwrap_or_else(|| r.to_string()));
            let mut remove = None;
            let mut look = None;
            for (i, t) in self.traces.iter_mut().enumerate() {
                let world: Vec<DVec2> = georef.map(|g| t.points.iter().map(|(la, lo)| g.world(*la, *lo)).collect()).unwrap_or_default();
                let len: f64 = world.windows(2).map(|w| (w[1] - w[0]).length()).sum();
                ui.horizontal(|ui| {
                    crate::ui::check(ui, &mut t.shown, "");
                    ui.label(RichText::new(&t.name).strong());
                    ui.label(RichText::new(format!("{:.2} km", len / 1000.0)).small().color(pal().text_dim));
                    if !world.is_empty() && icons::small(ui, Icon::Focus, tr("Go there", "Перейти туда")).clicked() {
                        look = Some(world[world.len() / 2]);
                    }
                    if icons::small(ui, Icon::Delete, tr("Forget it", "Убрать")).clicked() {
                        remove = Some(i);
                    }
                });
                if !world.is_empty() {
                    let label = match &road_name {
                        Some(n) => format!("{} «{n}»", tr("Build a road along it:", "Построить дорогу по нему:")),
                        None => tr("Choose a road type in the Roads tool to build it", "Выберите тип дороги в инструменте «Дороги», чтобы построить").to_string(),
                    };
                    if ui.add_enabled(road.is_some(), egui::Button::new(icons::labelled(Icon::Road, &label)).small()).on_hover_text(tr("Built as if drawn: corners rounded into arcs, junctions where it meets roads, cut at the map's edge; one undo step", "Строится как нарисованная: углы скругляются дугами, перекрёстки там, где она встречает дороги, обрезается по краю карты; один шаг отмены")).clicked() {
                        if let Some(r) = &road {
                            self.build = Some((world, r.clone()));
                        }
                    }
                }
            }
            if let Some(i) = remove {
                self.traces.remove(i);
            }
            if let Some(p) = look {
                ed.look_request = Some(p);
            }
        });
        self.open = open;
    }
}
