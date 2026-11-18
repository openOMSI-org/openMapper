//! The open map kept in step with its files: every two seconds its folder is looked at, and
//! what another program (OMSI 2's editor, another tool) wrote there is read in again. Where
//! the map has changes here not saved yet, the question is asked: the files' or these.
//! A copy of an installation's map also looks at the installation's, which OMSI 2's editor
//! writes, and offers what changed there.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::doc::sync::{self, Changes, Stamp};
use crate::doc::Document;
use crate::i18n::tr;
use crate::install::Roots;
use crate::theme::pal;

/// How often the folder is looked at.
const EVERY: Duration = Duration::from_secs(2);

struct Watched {
    /// The folder the map is read from.
    dir: PathBuf,
    /// Its files as the map has them (None: to be looked at first).
    base: Option<Stamp>,
    /// A change seen once: it is taken when the next look finds the files the same (a program
    /// writes many files one after another).
    settling: Option<Stamp>,
    /// The global settings as read or saved (see `sync::conflicts`).
    global_base: String,
    /// The installation's map this one is the copy of, and its change seen once.
    twin: Option<(PathBuf, Option<Stamp>)>,
}

struct Scanned {
    generation: u64,
    stamp: Stamp,
    twin: Option<Stamp>,
}

/// A question waiting for the user.
pub enum Ask {
    /// Changed outside and here too.
    Conflict(Changes),
    /// The installation's map changed: these files.
    Twin { twin: PathBuf, files: Vec<String>, stamp: Stamp },
}

#[derive(Default)]
pub struct MapSync {
    watched: Option<Watched>,
    scan: Option<Receiver<Scanned>>,
    /// Counts up whenever what is on disk becomes the map's own (read, saved): a look begun
    /// before is not wanted.
    generation: u64,
    last: Option<Instant>,
    pub ask: Option<Ask>,
}

/// What an answer did.
pub enum Answered {
    Reloaded(Changes),
    Took(usize),
    Failed(String),
    Nothing,
}

impl MapSync {
    /// The map was just read or saved here: its files are as it has them.
    pub fn rebase(&mut self, doc: Option<&Document>) {
        self.generation += 1;
        self.scan = None;
        if let Some(w) = &mut self.watched {
            w.base = None;
            w.settling = None;
            if let Some(doc) = doc {
                w.global_base = crate::doc::global_text(doc);
            }
        }
    }

    /// Look at the folder now and then. `busy`: a save is writing or an edit is under way
    /// (nothing is read in then). Some: parts of the map were read in again.
    pub fn tick(&mut self, doc: Option<&mut Document>, roots: Option<&Roots>, busy: bool) -> Option<Changes> {
        let (Some(doc), Some(roots)) = (doc, roots) else {
            self.watched = None;
            self.ask = None;
            return None;
        };
        let Some(dir) = sync::read_dir(doc) else {
            self.watched = None;
            return None;
        };
        if self.watched.as_ref().map(|w| w.dir != dir).unwrap_or(true) {
            // the first save of an installation's map: its copy remembers the installation's
            // as it was read
            if let Some(old) = &self.watched {
                if let (Some(base), Some(twin)) = (&old.base, sync::twin_of(roots, &doc.dir)) {
                    if same_dir(&twin, &old.dir) && sync::twin_seen(&doc.dir).is_none() {
                        sync::set_twin_seen(&doc.dir, base);
                    }
                }
            }
            let twin = if doc.source.is_none() { sync::twin_of(roots, &doc.dir) } else { None };
            self.watched = Some(Watched { dir, base: None, settling: None, global_base: crate::doc::global_text(doc), twin: twin.map(|t| (t, None)) });
            self.generation += 1;
            self.scan = None;
            self.ask = None;
        }
        if let Some(rx) = &self.scan {
            match rx.try_recv() {
                Ok(s) => {
                    self.scan = None;
                    if s.generation == self.generation && !busy {
                        return self.scanned(doc, s);
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.scan = None,
            }
        }
        if self.scan.is_none() && !busy && self.ask.is_none() && self.last.map(|t| t.elapsed() >= EVERY).unwrap_or(true) {
            self.last = Some(Instant::now());
            let w = self.watched.as_ref()?;
            let (dir, twin, generation) = (w.dir.clone(), w.twin.as_ref().map(|t| t.0.clone()), self.generation);
            let (tx, rx) = std::sync::mpsc::channel();
            // (a big map has thousands of files: they are looked at off the window's thread)
            let spawned = std::thread::Builder::new().name("map-sync".into()).spawn(move || {
                let _ = tx.send(Scanned { generation, stamp: sync::stamp(&dir), twin: twin.map(|t| sync::stamp(&t)) });
            });
            if spawned.is_ok() {
                self.scan = Some(rx);
            }
        }
        None
    }

    fn scanned(&mut self, doc: &mut Document, s: Scanned) -> Option<Changes> {
        let w = self.watched.as_mut()?;
        let mut out = None;
        match &w.base {
            None => w.base = Some(s.stamp),
            Some(base) if *base == s.stamp => w.settling = None,
            Some(base) if w.settling.as_ref() == Some(&s.stamp) => {
                let files = sync::changed_files(base, &s.stamp);
                log::info!("{}: changed outside openMapper: {}", w.dir.display(), files.join(", "));
                w.base = Some(s.stamp);
                w.settling = None;
                let changes = sync::classify(doc, &files);
                if !changes.is_empty() {
                    let mine = sync::conflicts(doc, &changes, &w.global_base);
                    let done = sync::reload(doc, &w.dir, &changes.without(&mine), false);
                    if done.global {
                        w.global_base = crate::doc::global_text(doc);
                    }
                    if !mine.is_empty() {
                        self.ask = Some(Ask::Conflict(mine));
                    }
                    out = (!done.is_empty()).then_some(done);
                }
            }
            Some(_) => w.settling = Some(s.stamp),
        }
        if let (Some((twin, settling)), Some(now)) = (&mut w.twin, s.twin) {
            let files = sync::twin_changes(&doc.dir, &now);
            if files.is_empty() {
                *settling = None;
            } else if settling.as_ref() == Some(&now) {
                *settling = None;
                if self.ask.is_none() {
                    self.ask = Some(Ask::Twin { twin: twin.clone(), files, stamp: now });
                }
            } else {
                *settling = Some(now);
            }
        }
        out
    }

    /// The question answered: true takes the files' (reload from disk, the installation's
    /// changes brought over), false keeps the map as it is here.
    pub fn answer(&mut self, doc: Option<&mut Document>, yes: bool) -> Answered {
        let (Some(ask), Some(doc)) = (self.ask.take(), doc) else { return Answered::Nothing };
        match ask {
            Ask::Conflict(changes) => {
                if !yes {
                    return Answered::Nothing;
                }
                let Some(w) = self.watched.as_mut() else { return Answered::Nothing };
                let done = sync::reload(doc, &w.dir, &changes, true);
                if done.global {
                    w.global_base = crate::doc::global_text(doc);
                }
                Answered::Reloaded(done)
            }
            Ask::Twin { twin, files, stamp } => {
                sync::set_twin_seen(&doc.dir, &stamp);
                if !yes {
                    return Answered::Nothing;
                }
                // (the copy's files change: the next looks read them in, or ask)
                self.last = None;
                match sync::take_from_twin(&twin, &doc.dir, &files) {
                    Ok(n) => Answered::Took(n),
                    Err(e) => Answered::Failed(e.to_string()),
                }
            }
        }
    }
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.canonicalize().ok().zip(b.canonicalize().ok()).map(|(a, b)| a == b).unwrap_or(a == b)
}

/// The question, while there is one: Some(true) the files', Some(false) the map's here.
pub fn window(ctx: &egui::Context, sync: &MapSync) -> Option<bool> {
    let ask = sync.ask.as_ref()?;
    let mut answer = None;
    let (title, text, note, yes, no) = match ask {
        Ask::Conflict(c) => (
            tr("The map changed on disk", "Карта изменилась на диске"),
            format!("{} {}.", tr("Another program changed the map's files where it has changes here not saved yet:", "Другая программа изменила файлы карты там, где здесь есть несохранённые изменения:"), c.describe()),
            tr("The other changes made on disk were read in already.", "Остальные изменения с диска уже загружены."),
            tr("Reload from disk", "Загрузить с диска"),
            tr("Keep mine", "Оставить мои"),
        ),
        Ask::Twin { files, .. } => (
            tr("The map changed in OMSI 2's folder", "Карта изменилась в папке OMSI 2"),
            format!("{} ({} {}).", tr("The installation's own copy of this map was changed after openMapper's copy in the content folder was saved, by OMSI 2's editor say", "Копия этой карты в установке OMSI 2 изменена после сохранения копии openMapper в папке контента, например редактором OMSI 2"), files.len(), tr("files", "файлов")),
            tr("Bringing the changes over replaces those files of openMapper's copy; the files replaced are kept in a backup (File > Earlier versions).", "Перенос заменит эти файлы в копии openMapper; заменённые файлы сохраняются в резервной копии (Файл > Прошлые версии)."),
            tr("Bring the changes over", "Перенести изменения"),
            tr("Keep openMapper's copy", "Оставить копию openMapper"),
        ),
    };
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(text);
        ui.label(egui::RichText::new(note).small().color(pal().text_dim));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(yes).clicked() {
                answer = Some(true);
            }
            if ui.button(no).clicked() {
                answer = Some(false);
            }
        });
    });
    answer
}
