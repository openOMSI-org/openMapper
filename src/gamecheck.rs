//! Check in openOMSI (Map menu): the saved map run by the game itself, offscreen, with its
//! own map checks (roads with no surface under a lane, roads buried in the ground, steps a
//! bus wheel cannot climb, chained roads apart in height, start points off the road, the
//! trips' ways on the lanes), and its picture of the map taken from the editor's view. What
//! it says is listed, the problems first.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

use egui::RichText;

use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::theme::pal;

/// The game's checks asked for.
const CHECKS: [&str; 6] = ["OMSI_CHECK_ROADS", "OMSI_CHECK_SPLINES", "OMSI_CHECK_TRIPS", "OMSI_CHECK_ENTRIES", "OMSI_CHECK_SPIKES", "OMSI_CHECK_WHEELS"];

/// One line of the game's report: whether it names a problem, and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub problem: bool,
    pub text: String,
}

/// What the run gave.
pub struct Outcome {
    pub lines: Vec<Line>,
    pub picture: Option<PathBuf>,
    pub secs: f32,
    /// The game's own output, for a look when something went wrong.
    pub log: String,
}

/// The first whole number in a text after `after` (or at its start).
fn number_after(t: &str, after: &str) -> Option<u64> {
    let i = if after.is_empty() { 0 } else { t.find(after)? + after.len() };
    let rest = t[i..].trim_start();
    let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    n.parse().ok()
}

/// The report lines of the game's log: its checks' summaries, a trip's gap or a bare point
/// named, and errors.
pub fn summarize(log: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in log.lines() {
        let text = raw.split_once("] ").map(|(_, t)| t).unwrap_or(raw).trim();
        let error = raw.contains(" ERROR ") || raw.starts_with("Error") || raw.contains("panicked");
        let line = if error {
            Some(Line { problem: true, text: text.to_string() })
        } else if text.starts_with("check trips: ") && text.contains(" trips on loaded lanes") {
            let bad = ["gaps", "taken the wrong way round", "lanes not as long as the file says"].iter().any(|w| {
                text.split(';').chain(text.split(',')).any(|part| part.trim().ends_with(w) && number_after(part.trim(), "").map(|n| n > 0).unwrap_or(false))
            });
            Some(Line { problem: bad, text: text.to_string() })
        } else if text.starts_with("check trips: ") && (text.contains(": gap") || text.contains("WRONG WAY")) {
            Some(Line { problem: true, text: text.to_string() })
        } else if text.starts_with("wheel check:") && text.contains("wheel points") {
            // ("3636 wheel points, 0 under a face, 0 off the lane's height ...": the counts after
            // the total)
            let bad = text.split(',').skip(1).any(|part| number_after(part.trim(), "").map(|n| n > 0).unwrap_or(false));
            Some(Line { problem: bad, text: text.to_string() })
        } else if ["road check:", "spline check:", "entry point check:", "buried check:", "ground-cut check:", "ground-over-road check:", "wheel check:", "spike check:"].iter().any(|p| text.starts_with(p)) {
            // ("3 of 804 ...": a problem when the first number is not 0, and is not a tiny share
            // of a great many, as the stock maps have too: 14 of 283163 ground points)
            let n = text.split_once(':').and_then(|(_, r)| number_after(r, "")).unwrap_or(0);
            let of = number_after(text, " of ").unwrap_or(0);
            let tiny = of > 0 && (n as f64) < of as f64 * 0.0005;
            Some(Line { problem: n > 0 && !tiny, text: text.to_string() })
        } else if text.starts_with("bare point at") {
            Some(Line { problem: true, text: text.to_string() })
        } else {
            None
        };
        if let Some(l) = line {
            if !out.contains(&l) {
                out.push(l);
            }
        }
    }
    // (the problems first, in the order the game named them)
    out.sort_by_key(|l| !l.problem);
    out
}

/// The place a report line names ("… at (-156, 96, 0.1) …"), world x, y.
pub fn place_in(text: &str) -> Option<glam::DVec2> {
    let i = text.find("at (")? + 4;
    let j = text[i..].find(')')?;
    let v: Vec<f64> = text[i..i + j].split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() >= 2).then(|| glam::DVec2::new(v[0], v[1]))
}

#[derive(Default)]
pub struct GameCheck {
    /// A place of the report clicked: the camera goes there.
    pub look: Option<glam::DVec2>,
    pub open: bool,
    /// Every tile loaded (else those round the first start point).
    pub whole: bool,
    running: Option<(Receiver<Result<Outcome, String>>, Instant)>,
    outcome: Option<Outcome>,
    error: Option<String>,
    picture: Option<egui::TextureHandle>,
    show_log: bool,
}

impl GameCheck {
    /// Run the game on the saved map `folder` (`cam`: the editor's eye x, y, z, yaw, pitch
    /// and field of view, for the picture).
    pub fn start(&mut self, game: &Path, roots: &Roots, folder: &str, cam: [f64; 6], tiles: usize) {
        if self.running.is_some() {
            return;
        }
        if !self.open {
            self.whole = tiles <= 30;
        }
        self.open = true;
        self.error = None;
        let pic = std::env::temp_dir().join(format!("openmapper-gamecheck-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&pic);
        let mut cmd = std::process::Command::new(game);
        cmd.arg("--root").arg(&roots.omsi).arg("--map").arg(format!("maps/{folder}/global.cfg")).arg("--schedule").arg("--view").arg("free");
        cmd.arg("--cam").arg(format!("{:.1},{:.1},{:.1},{:.1},{:.1},{:.0}", cam[0], cam[1], cam[2], cam[3], cam[4], cam[5]));
        if self.whole {
            cmd.arg("--all");
        }
        cmd.arg("--offscreen").arg(&pic);
        if let Some(dir) = game.parent() {
            cmd.current_dir(dir);
        }
        cmd.env("OMSI_CONTENT", &roots.content);
        for c in CHECKS {
            cmd.env(c, "1");
        }
        let (tx, rx) = channel();
        let t0 = Instant::now();
        std::thread::spawn(move || {
            let r = cmd.output().map_err(|e| e.to_string()).map(|o| {
                let log = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                let mut lines = summarize(&log);
                if !o.status.success() {
                    lines.insert(0, Line { problem: true, text: format!("{} ({})", tr("openOMSI stopped with an error", "openOMSI завершилась с ошибкой"), o.status) });
                }
                Outcome { lines, picture: pic.is_file().then_some(pic), secs: t0.elapsed().as_secs_f32(), log }
            });
            let _ = tx.send(r);
        });
        self.running = Some((rx, t0));
        self.outcome = None;
        self.picture = None;
    }

    /// The window; true when the check is to be run again (the map saved first). `at_edge`:
    /// whether a place lies at the map's edge or a road's dead end (nothing to mend).
    pub fn window(&mut self, ctx: &egui::Context, at_edge: &dyn Fn(glam::DVec2) -> bool) -> bool {
        if let Some((rx, _)) = &self.running {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok(o) => self.outcome = Some(o),
                    Err(e) => self.error = Some(e),
                }
                self.running = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        if !self.open {
            return false;
        }
        // its picture, once
        if let (None, Some(p)) = (&self.picture, self.outcome.as_ref().and_then(|o| o.picture.clone())) {
            if let Ok(img) = image::open(&p) {
                let img = img.thumbnail(800, 450).to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                self.picture = Some(ctx.load_texture("openomsi check", ci, egui::TextureOptions::LINEAR));
            }
        }
        let mut again = false;
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Verified, tr("Check in openOMSI", "Проверка в openOMSI"))).id(egui::Id::new("game check")).open(&mut open).default_pos([330.0, 90.0]).default_width(440.0).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("The saved map run by the game itself, unseen, with its own checks; the picture is the game's, from your view", "Сохранённая карта, запущенная самой игрой без окна, с её собственными проверками; картинка — игры, с вашего вида")).small().color(pal().text_dim));
            ui.horizontal(|ui| {
                crate::ui::check(ui, &mut self.whole, tr("Every tile (slower on a big map)", "Все тайлы (дольше на большой карте)"));
                if ui.add_enabled(self.running.is_none(), egui::Button::new(icons::labelled(Icon::Refresh, tr("Run again", "Запустить снова")))).clicked() {
                    again = true;
                }
            });
            ui.separator();
            if let Some((_, t0)) = &self.running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{} {:.0} s", tr("openOMSI is loading the map…", "openOMSI загружает карту…"), t0.elapsed().as_secs_f32()));
                });
            }
            if let Some(e) = &self.error {
                ui.colored_label(pal().error, format!("openOMSI: {e}"));
            }
            if let Some(o) = &self.outcome {
                let problems = o.lines.iter().filter(|l| l.problem && !place_in(&l.text).map(at_edge).unwrap_or(false)).count();
                let head = if problems == 0 { tr("Nothing wrong found", "Проблем не найдено").to_string() } else { format!("{} {problems}", tr("Problems found:", "Найдено проблем:")) };
                ui.label(RichText::new(format!("{head} · {:.0} s", o.secs)).strong().color(if problems == 0 { pal().ok } else { pal().warn }));
                if let Some(t) = &self.picture {
                    let w = ui.available_width().min(560.0);
                    let size = t.size_vec2() * (w / t.size_vec2().x);
                    ui.image((t.id(), size));
                }
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    let mut edge = 0;
                    for l in &o.lines {
                        let place = place_in(&l.text);
                        let on_edge = place.map(at_edge).unwrap_or(false);
                        edge += usize::from(on_edge);
                        let c = if l.problem && !on_edge { pal().warn } else { if on_edge { pal().text_dim } else { pal().text } };
                        match place {
                            Some(p) => {
                                if ui.add(egui::Label::new(RichText::new(&l.text).small().color(c).underline()).sense(egui::Sense::click())).on_hover_text(tr("Go there", "Перейти туда")).clicked() {
                                    self.look = Some(p);
                                }
                            }
                            None => {
                                ui.label(RichText::new(&l.text).small().color(c));
                            }
                        }
                    }
                    if edge > 0 {
                        ui.label(RichText::new(format!("{edge} {}", tr("of the places named lie at the map's edge or a road's dead end (greyed): the game measures a lane's last point on the road's very edge, and a bus at the end stands over it; nothing to mend", "из названных мест лежат на краю карты или в тупике дороги (серые): игра меряет последнюю точку полосы на самом краю дороги, а автобус в конце стоит над ним; чинить нечего"))).small().italics().color(pal().text_dim));
                    }
                    if o.lines.is_empty() {
                        ui.label(RichText::new(tr("The game said nothing about the map", "Игра ничего не сказала о карте")).small().color(pal().text_dim));
                    }
                });
                ui.checkbox(&mut self.show_log, tr("The game's whole output", "Весь вывод игры"));
                if self.show_log {
                    egui::ScrollArea::vertical().id_salt("game log").max_height(200.0).show(ui, |ui| {
                        ui.label(RichText::new(&o.log).monospace().size(10.0));
                    });
                }
            }
        });
        self.open = open;
        again
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_report_is_read_its_problems_first() {
        let log = "\
[2026-10-14T01:45:55Z INFO  omsi_scene::cut_terrain] ground-cut check: 0 of 75293 covered ground points would be cut away with nothing under them (0.00 %)
[2026-10-14T01:45:56Z INFO  openomsi_game::offscreen::checks] entry point check: 0 of 1 entry points have no road surface under them
[2026-10-14T01:45:56Z INFO  openomsi_game::offscreen::checks] road check: 1 of 804 points along the driving lanes have no road surface under them (0.1%)
[2026-10-14T01:45:56Z INFO  openomsi_game::offscreen::checks]   bare point at (-156, 96, 0.1), nearest surface NaN m
[2026-10-14T01:45:56Z INFO  omsi_sim::timetable_run::sim::route] check trips: 2 trips on loaded lanes (0 not wholly loaded): 20 joints, 20 joined, 0 lane changes, 0 gaps, 0 taken the wrong way round; 3 lanes not as long as the file says
[2026-10-14T01:45:56Z INFO  openomsi_game::offscreen::checks] wheel check: 0 faces a bus wheel cannot climb along 4 km of driving lanes: 0 steps of 0.38..0.52 m (climbed before), 0 higher
[2026-10-14T01:45:56Z INFO  openomsi_game::offscreen::picture] rendered 43 instances in 438.7 ms";
        let l = summarize(log);
        assert_eq!(l.len(), 6);
        assert_eq!(l.iter().filter(|x| x.problem).count(), 3);
        assert!(l[0].text.starts_with("road check: 1 of 804"));
        assert!(l.iter().any(|x| x.problem && x.text.contains("lanes not as long")));
        assert!(l.iter().any(|x| !x.problem && x.text.starts_with("wheel check: 0")));
        assert!(!summarize("[x INFO  y] wheel check: 3636 wheel points, 0 under a face (a wall to the tyre), 0 off the lane's height by more than 30 cm")[0].problem);
        assert!(summarize("[x INFO  y] wheel check: 3636 wheel points, 2 under a face (a wall to the tyre), 0 off the lane's height by more than 30 cm")[0].problem);
        assert!(!summarize("[x INFO  y] ground-cut check: 14 of 283163 covered ground points would be cut away with nothing under them (0.00 %)")[0].problem);
        assert_eq!(place_in("bare point at (-156, 96, 0.1), nearest surface NaN m"), Some(glam::DVec2::new(-156.0, 96.0)));
        assert_eq!(place_in("road check: 1 of 804 points"), None);
        let fine = "[x INFO  y] check trips: 2 trips on loaded lanes (0 not wholly loaded): 21 joints, 21 joined, 0 lane changes, 0 gaps, 0 taken the wrong way round; 0 lanes not as long as the file says";
        assert!(!summarize(fine)[0].problem);
    }
}
