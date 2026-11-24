//! Saved views: places of a map to come back to (a busy junction, the depot, a stop),
//! each a camera as it was, kept with the map in `<map>/openMapper/views.json` (the game
//! ignores the folder). `Ctrl+1…9` goes to the first nine, `Ctrl+Shift+1…9` keeps the
//! view there.

use std::path::{Path, PathBuf};

use egui::{RichText, Vec2};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::pal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub name: String,
    pub target: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub distance: f64,
}

impl SavedView {
    fn of(cam: &Camera, name: String) -> SavedView {
        SavedView { name, target: cam.target.to_array(), yaw: cam.yaw, pitch: cam.pitch, distance: cam.distance }
    }

    /// The camera flies there (smoothly: it follows its wanted state).
    fn apply(&self, cam: &mut Camera) {
        cam.target = DVec3::from_array(self.target);
        cam.yaw = self.yaw;
        cam.pitch = self.pitch;
        cam.distance = self.distance;
    }
}

#[derive(Default)]
pub struct Views {
    pub open: bool,
    list: Vec<SavedView>,
    /// The map folder they belong to.
    dir: Option<PathBuf>,
    error: Option<String>,
}

fn file(map_dir: &Path) -> PathBuf {
    crate::geo::dir_of(map_dir).join("views.json")
}

pub fn load(map_dir: &Path) -> Vec<SavedView> {
    std::fs::read_to_string(file(map_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(map_dir: &Path, list: &[SavedView]) -> std::io::Result<()> {
    let p = file(map_dir);
    if list.is_empty() {
        return match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(p.parent().unwrap())?;
    crate::doc::safe::write_atomic(&p, serde_json::to_string_pretty(list).unwrap_or_default().as_bytes())
}

impl Views {
    /// The views of the map open now (read again when another map is opened).
    pub fn follow(&mut self, map_dir: Option<&Path>) {
        if self.dir.as_deref() != map_dir {
            self.dir = map_dir.map(Path::to_path_buf);
            self.list = map_dir.map(load).unwrap_or_default();
            self.error = None;
        }
    }

    fn write(&mut self) {
        if let Some(d) = &self.dir {
            self.error = save(d, &self.list).err().map(|e| format!("{}: {e}", tr("Could not keep the views", "Не удалось сохранить виды")));
        }
    }

    /// The view `n` (0-based) gone to; false when there is none.
    pub fn go(&self, n: usize, cam: &mut Camera) -> bool {
        self.list.get(n).map(|v| v.apply(cam)).is_some()
    }

    /// The camera kept as view `n` (0-based; past the end: added after the last).
    pub fn store(&mut self, n: usize, cam: &Camera) -> usize {
        let name = self.list.get(n).map(|v| v.name.clone()).unwrap_or_else(|| format!("{} {}", tr("View", "Вид"), self.list.len() + 1));
        let v = SavedView::of(cam, name);
        let at = if n < self.list.len() {
            self.list[n] = v;
            n
        } else {
            self.list.push(v);
            self.list.len() - 1
        };
        self.write();
        at
    }

    pub fn window(&mut self, ctx: &egui::Context, cam: &mut Camera) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut changed = false;
        egui::Window::new(icons::labelled(Icon::Bookmark, tr("Saved views", "Сохранённые виды"))).id(egui::Id::new("saved views")).open(&mut open).default_pos([360.0, 140.0]).resizable(false).show(ctx, |ui| {
            ui.label(RichText::new(tr("Places of the map to come back to: Ctrl+1…9 goes to the first nine, Ctrl+Shift+1…9 keeps the view there", "Места карты, к которым удобно возвращаться: Ctrl+1…9 ведёт к первым девяти, Ctrl+Shift+1…9 запоминает вид туда")).small().color(pal().text_dim));
            if ui.button(icons::labelled(Icon::Add, tr("Keep this view", "Запомнить этот вид"))).clicked() {
                let n = self.list.len();
                self.store(n, cam);
            }
            ui.separator();
            if self.list.is_empty() {
                ui.label(RichText::new(tr("None yet", "Пока нет")).italics().color(pal().text_dim));
            }
            let mut remove = None;
            let mut retake = None;
            let mut swap = None;
            let n = self.list.len();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (i, v) in self.list.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let key = if i < 9 { format!("{}", i + 1) } else { " ".into() };
                        ui.label(RichText::new(key).monospace().color(pal().text_dim));
                        if ui.button(icons::labelled(Icon::Focus, tr("Go", "К нему"))).clicked() {
                            v.apply(cam);
                        }
                        changed |= ui.add(egui::TextEdit::singleline(&mut v.name).desired_width(170.0)).lost_focus();
                        ui.spacing_mut().item_spacing = Vec2::new(2.0, 0.0);
                        if icons::small(ui, Icon::Raise, tr("Earlier in the list", "Выше в списке")).clicked() && i > 0 {
                            swap = Some(i - 1);
                        }
                        if icons::small(ui, Icon::Lower, tr("Later in the list", "Ниже в списке")).clicked() && i + 1 < n {
                            swap = Some(i);
                        }
                        if icons::small(ui, Icon::Save, tr("Take the view now instead", "Заменить нынешним видом")).clicked() {
                            retake = Some(i);
                        }
                        if icons::small(ui, Icon::Delete, tr("Forget it", "Забыть")).clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
            if let Some(i) = retake {
                self.store(i, cam);
            }
            if let Some(i) = swap {
                self.list.swap(i, i + 1);
                changed = true;
            }
            if let Some(i) = remove {
                self.list.remove(i);
                changed = true;
            }
            if let Some(e) = &self.error {
                ui.colored_label(pal().error, e);
            }
        });
        self.open = open;
        if changed {
            self.write();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_are_kept_with_the_map_and_gone_to() {
        let dir = std::env::temp_dir().join(format!("openmapper-views-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut v = Views::default();
        v.follow(Some(&dir));
        let mut cam = Camera::default();
        cam.target = DVec3::new(10.0, 20.0, 3.0);
        cam.yaw = 90.0;
        assert_eq!(v.store(5, &cam), 0);
        cam.target = DVec3::new(-40.0, 0.0, 0.0);
        v.store(1, &cam);
        // another map, then this one again: read from its folder
        v.follow(None);
        assert!(!v.go(0, &mut cam));
        v.follow(Some(&dir));
        assert!(v.go(0, &mut cam));
        assert_eq!((cam.target, cam.yaw), (DVec3::new(10.0, 20.0, 3.0), 90.0));
        assert_eq!(load(&dir).len(), 2);
        // retaken keeps its name
        v.list[0].name = "Depot".into();
        cam.target = DVec3::new(-40.0, 0.0, 0.0);
        v.store(0, &cam);
        assert_eq!(load(&dir)[0].name, "Depot");
        assert_eq!(load(&dir)[0].target, [-40.0, 0.0, 0.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
