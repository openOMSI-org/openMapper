//! Groups (Edit menu): a selection kept under a name, to put down again in any map, as a
//! stop with its shelter and signs or a crossing with its lights. Each is a tile file of its
//! own in `<content>/openMapper/groups`, laid round its middle.

use std::path::PathBuf;

use egui::RichText;

use crate::clipboard::Clip;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::library::Library;
use crate::theme::pal;

struct Entry {
    name: String,
    path: PathBuf,
    count: usize,
}

#[derive(Default)]
pub struct Groups {
    pub open: bool,
    name: String,
    list: Option<Vec<Entry>>,
}

fn dir() -> PathBuf {
    crate::install::content_dir().join("openMapper").join("groups")
}

/// A name as a file name (what a file system forbids made `_`).
fn file_name(name: &str) -> String {
    let n: String = name.trim().chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    format!("{}.map", n.trim_matches('.'))
}

fn read_list() -> Vec<Entry> {
    let mut v: Vec<Entry> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("map")).unwrap_or(false))
        .filter_map(|p| {
            let t = omsi_map::Tile::load(&p).ok()?;
            let count = t.objects.len() + t.splines.len();
            Some(Entry { name: p.file_stem()?.to_string_lossy().to_string(), path: p, count })
        })
        .collect();
    v.sort_by_key(|e| e.name.to_lowercase());
    v
}

impl Groups {
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, lib: &Library) {
        if !self.open {
            return;
        }
        let entries = self.list.get_or_insert_with(read_list);
        let abs = |f: &str| lib.find(f).map(|a| a.abs_height).unwrap_or(false);
        let mut open = self.open;
        let mut reread = false;
        egui::Window::new(icons::labelled(Icon::Objects, tr("Groups for other maps", "Группы для других карт"))).id(egui::Id::new("groups")).open(&mut open).default_pos([360.0, 120.0]).default_size([340.0, 420.0]).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(tr("A selection kept under a name, to put down again in any map (a stop with its shelter and signs, a crossing with its lights)", "Выделение, сохранённое под именем, чтобы ставить его снова на любой карте (остановка с павильоном и знаками, перекрёсток со светофорами)")).small().color(pal().text_dim));
            ui.add_space(4.0);
            let n = ed.selection().len();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(tr("Name", "Название")).desired_width(170.0));
                let can = n > 0 && !self.name.trim().is_empty() && ed.doc.is_some();
                let r = ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::Save, tr("Keep the selection", "Сохранить выделение"))));
                if r.clicked() {
                    if let Some(doc) = &ed.doc {
                        let clip = crate::clipboard::copy(doc, &ed.selection(), abs);
                        let path = dir().join(file_name(&self.name));
                        let bytes = crate::doc::save::utf16(&crate::doc::save::tile_text(&clip.to_tile()));
                        let done = std::fs::create_dir_all(dir()).and_then(|_| crate::doc::safe::write_atomic(&path, &bytes));
                        match done {
                            Ok(()) => {
                                ed.toast(format!("{} «{}»: {}", tr("Group kept", "Группа сохранена"), self.name.trim(), clip.len()));
                                self.name.clear();
                                reread = true;
                            }
                            Err(e) => ed.error(format!("{}: {e}", path.display())),
                        }
                    }
                }
            });
            if n == 0 {
                ui.label(RichText::new(tr("Select what to keep in the view first.", "Сначала выделите в окне вида то, что сохранить.")).small().color(pal().text_dim));
            }
            ui.separator();
            if entries.is_empty() {
                ui.label(RichText::new(tr("No groups yet.", "Групп пока нет.")).color(pal().text_dim));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for e in entries.iter() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&e.name).color(pal().text_strong));
                        ui.label(RichText::new(format!("{} {}", e.count, if e.count == 1 { tr("thing", "шт.") } else { tr("things", "шт.") })).small().color(pal().text_dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(icons::labelled(Icon::Delete, "")).on_hover_text(tr("Delete the group (to openMapper's bin)", "Удалить группу (в корзину openMapper)")).clicked() {
                                let bin = crate::install::content_dir().join("openMapper").join("bin").join("groups");
                                let to = bin.join(e.path.file_name().unwrap_or_default());
                                if std::fs::create_dir_all(&bin).and_then(|_| std::fs::rename(&e.path, &to)).is_ok() {
                                    reread = true;
                                }
                            }
                            if ui.small_button(icons::labelled(Icon::Paste, tr("Use", "Взять"))).on_hover_text(tr("Into the clipboard: Ctrl+V puts it down at the cursor, as often as wanted", "В буфер обмена: Ctrl+V ставит её у курсора, сколько угодно раз")).clicked() {
                                match omsi_map::Tile::load(&e.path) {
                                    Ok(t) => {
                                        let clip: Clip = Clip::from_tile(&t, abs);
                                        ed.clip = Some(clip);
                                        ed.toast(format!("«{}» {}", e.name, tr("is in the clipboard: Ctrl+V puts it at the cursor", "в буфере обмена: Ctrl+V ставит её у курсора")));
                                    }
                                    Err(err) => ed.error(format!("{}: {err}", e.path.display())),
                                }
                            }
                        });
                    });
                }
            });
        });
        self.open = open;
        if reread || !open {
            self.list = None;
        }
    }
}
