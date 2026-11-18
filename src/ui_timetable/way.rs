//! A trip's way picked by hand (#122, #71): the paths clicked in the view one after the
//! other as in OMSI 2's editor, before the first stop and after the last as well, and where
//! two paths of the way do not join (the game's bus loses its way there).

use super::*;
use crate::timetable::way;
use crate::theme::pal;

/// How near a click must be to a path to pick it (m).
const PATH_REACH: f64 = 8.0;

/// The lanes of the trip's saved track (None: it has none, or a path of it is not on the map).
pub(super) fn saved_lanes(net: &Network, doc: &Document, name: &str) -> Option<Vec<usize>> {
    let track = doc.timetable.track(name)?;
    way::lanes_of_track(net, &track.entries, &doc.order).into_iter().collect()
}

/// The way of a trip along `lanes`, its stops found on it, kept as the trip's (picked by
/// hand): its track, and the station links where every stop lies on it.
pub(super) fn keep_by_hand(net: &Network, files: &mut Files, doc: &mut Document, name: &str, lanes: &[usize]) -> Route {
    let stations = doc.timetable.trip(name).map(|t| t.stations.clone()).unwrap_or_default();
    let pos: Vec<Option<DVec3>> = stations.iter().map(|id| timetable::stop_world(doc, files, *id)).collect();
    let r = way::along(net, lanes, &pos, doc.global.left_hand_traffic);
    let links = if r.stops.len() == stations.len() && stations.len() >= 2 { r.links(net, &doc.order, &stations) } else { Vec::new() };
    doc.timetable.set_by_hand(name, Some((r.track(net, &doc.order), links)));
    r
}

impl TimetableUi {
    /// The lanes the trip drives now: picked by hand, else as found.
    fn way_lanes(&self, doc: &Document, name: &str) -> Vec<usize> {
        let Some(net) = &self.net else { return Vec::new() };
        if doc.timetable.by_hand(name) {
            return saved_lanes(net, doc, name).unwrap_or_default();
        }
        self.shown.as_ref().filter(|s| s.0.eq_ignore_ascii_case(name) && s.3.ok()).map(|s| s.3.lanes.clone()).unwrap_or_default()
    }

    /// The trip's way set to `lanes`, picked by hand from now on.
    fn set_way(&mut self, doc: &mut Document, name: &str, lanes: &[usize]) {
        let Some(net) = &self.net else { return };
        let r = keep_by_hand(net, &mut self.files, doc, name, lanes);
        if r.problems.is_empty() {
            self.problems.remove(&lower(name));
        } else {
            self.problems.insert(lower(name), r.problems);
        }
        self.routed.remove(&lower(name));
        self.shown = None;
    }

    /// While the paths are being picked, a click in the view puts the path under it into the
    /// way, or takes it out (see [`way::with_lane`]).
    pub(super) fn click_path(&mut self, ed: &mut Editor) {
        let Some((_, at)) = ed.picked.take() else { return };
        let (Some(p), Some(name)) = (at, self.trip.clone()) else { return };
        let Some(doc) = ed.doc.as_mut() else { return };
        let Some(net) = &self.net else { return };
        let ways = self.ways.get(&lower(&name)).copied().or_else(|| self.shown.as_ref().filter(|s| s.0 == name).and_then(|s| s.3.ways)).unwrap_or(Ways::Road);
        let Some(l) = way::lane_at(net, p, PATH_REACH, ways, doc.global.left_hand_traffic) else {
            ed.toasts.push(crate::editor::Toast { text: tr("No path there: click a lane of a road (View > AI lanes shows them)", "Здесь нет пути: кликните по полосе дороги (Вид > Полосы ИИ показывает их)").into(), at: std::time::Instant::now(), error: true });
            return;
        };
        let lanes = self.way_lanes(doc, &name);
        let new = way::with_lane(net, &lanes, l);
        self.set_way(doc, &name, &new);
    }

    /// The trip's way: found along the roads or picked by hand, its paths, and where it breaks.
    pub(super) fn way_section(&mut self, ui: &mut egui::Ui, doc: &mut Document, name: &str, actions: &mut Vec<Action>) {
        let Some(net) = &self.net else { return };
        let hand = doc.timetable.by_hand(name);
        let lanes = self.way_lanes(doc, name);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let label = if self.way_edit { tr("Done picking paths", "Готово") } else { tr("Pick the paths on the map", "Выбрать пути на карте") };
            if ui.add(egui::Button::selectable(self.way_edit, icons::labelled(Icon::Route, label))).on_hover_text(tr("Choose the way yourself, path by path, as in OMSI 2's editor", "Выберите путь сами, путь за путём, как в редакторе OMSI 2")).clicked() {
                self.way_edit = !self.way_edit;
                self.picking = false;
            }
            if hand && ui.button(icons::labelled(Icon::Route, tr("Find the way automatically", "Найти путь автоматически"))).on_hover_text(tr("Forget the paths picked: the way is found along the roads again", "Забыть выбранные пути: путь снова найдётся по дорогам")).clicked() {
                doc.timetable.set_by_hand(name, None);
                self.way_edit = false;
                self.shown = None;
            }
        });
        if hand {
            ui.label(RichText::new(tr("The paths were picked by hand: they are kept when the stops change.", "Пути выбраны вручную: они сохраняются при изменении остановок.")).small().color(pal().text_dim));
        }
        if self.way_edit {
            ui.label(RichText::new(tr("Click paths in the view. One after the end or before the start is added, with the paths to it; one beside the way takes the place of the part it goes round (another branch at a junction); a path of the way is taken out. The way may run on before the first stop and after the last.", "Кликайте пути на карте. Путь после конца или перед началом добавляется вместе с путями до него; путь в стороне заменяет участок, который он объезжает (другая ветка на перекрёстке); путь из маршрута убирается. Путь может идти и до первой остановки, и после последней.")).small().color(pal().accent));
        }
        // where the way breaks
        let gaps = way::gaps(net, &lanes);
        for &(k, m) in &gaps {
            ui.horizontal(|ui| {
                let text = format!("{} {} ({:.1} m): {}", tr("The way breaks after path", "Путь прерывается после пути"), k + 1, m, tr("the paths do not join, the bus's GPS loses its way here in the game", "пути не соединены, GPS автобуса в игре здесь теряет маршрут"));
                ui.label(coloured(icons::labelled(Icon::Warning, &text), pal().warn));
                if small_button(ui, Icon::Focus, tr("Show on the map", "Показать на карте")) {
                    actions.push(Action::LookAt(net.lanes[lanes[k]].pts.last().unwrap().truncate()));
                }
            });
        }
        if lanes.is_empty() {
            return;
        }
        let mut remove: Option<Vec<usize>> = None;
        let shown = self.shown.as_ref().filter(|s| s.0 == name).map(|s| s.3.clone());
        egui::CollapsingHeader::new(icons::labelled(Icon::Route, &format!("{} ({})", tr("Paths of the way", "Пути маршрута"), lanes.len()))).id_salt(("tt way paths", lower(name))).default_open(false).show(ui, |ui| {
            // (where the stops are, to trim the way before the first and after the last)
            let stop_at: Vec<usize> = shown.as_ref().filter(|r| r.lanes == lanes).map(|r| r.stops.iter().map(|s| s.0).collect()).unwrap_or_default();
            ui.horizontal(|ui| {
                if let (Some(&a), Some(&b)) = (stop_at.first(), stop_at.last()) {
                    if ui.add_enabled(a > 0, egui::Button::new(icons::labelled(Icon::Cut, tr("Cut before the first stop", "Убрать до первой остановки")))).clicked() {
                        remove = Some(lanes[a..].to_vec());
                    }
                    if ui.add_enabled(b + 1 < lanes.len(), egui::Button::new(icons::labelled(Icon::Cut, tr("Cut after the last stop", "Убрать после последней остановки")))).clicked() {
                        remove = Some(lanes[..=b].to_vec());
                    }
                }
            });
            egui::ScrollArea::vertical().id_salt(("tt way list", lower(name))).max_height(260.0).show(ui, |ui| {
                for (k, &l) in lanes.iter().enumerate() {
                    let lane = &net.lanes[l];
                    ui.horizontal(|ui| {
                        let what = if lane.spline { tr("spline", "сплайн") } else { tr("object", "объект") };
                        let stop = stop_at.contains(&k);
                        let text = format!("{:>3}. {} {} · {} {} · {:.0} m", k + 1, what, lane.key.id, tr("path", "путь"), lane.key.path, lane.length());
                        let broken = gaps.iter().any(|g| g.0 == k);
                        ui.label(RichText::new(text).monospace().color(if broken { pal().warn } else { pal().text }));
                        if stop {
                            ui.label(Icon::Bus.text(14.0).color(pal().accent)).on_hover_text(tr("A stop on this path", "Остановка на этом пути"));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if small_button(ui, Icon::Delete, tr("Take out", "Убрать")) {
                                let mut v = lanes.clone();
                                v.remove(k);
                                remove = Some(v);
                            }
                            if small_button(ui, Icon::Focus, tr("Show on the map", "Показать на карте")) {
                                actions.push(Action::LookAt(lane.point_at(lane.length() * 0.5).truncate()));
                            }
                        });
                    });
                }
            });
        });
        if let Some(v) = remove {
            self.set_way(doc, name, &v);
        }
    }
}
