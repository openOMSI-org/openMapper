//! The map's name and description in other languages: `global_ENG.dsc`, `global_FRA.dsc`…
//! beside `global.cfg` (whose own name and description are the ones shown where a language
//! has no file; OMSI 2's are German). openOMSI's launcher shows the one of its language,
//! else the English one. Written as OMSI 2's are: UTF-16 with a byte order mark.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use egui::{Color32, RichText};

use crate::doc::safe::Op;
use crate::i18n::tr;

/// The languages offered (OMSI's codes) and their names.
pub const LANGS: &[(&str, &str)] = &[("ENG", "English"), ("DEU", "Deutsch"), ("FRA", "Français"), ("RUS", "Русский"), ("UKR", "Українська"), ("POL", "Polski"), ("CZE", "Čeština"), ("ITA", "Italiano"), ("ESP", "Español"), ("NLD", "Nederlands"), ("HUN", "Magyar")];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Descriptions {
    /// Language code: (name, description).
    pub langs: BTreeMap<String, (String, String)>,
    pub dirty: bool,
    /// Counts each change, so a save that is still writing can tell a later edit from the one it copied.
    pub edit: u64,
    changed: BTreeSet<String>,
    removed: BTreeSet<String>,
}

fn file_of(code: &str) -> String {
    format!("global_{code}.dsc")
}

/// A `.dsc` file's name and description (as openOMSI's launcher reads them).
pub fn parse(text: &str) -> (String, String) {
    let mut name = Vec::new();
    let mut desc: Vec<&str> = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = if t.eq_ignore_ascii_case("[name]") || t.eq_ignore_ascii_case("[friendlyname]") {
                "name"
            } else if t.eq_ignore_ascii_case("[description]") {
                "description"
            } else {
                ""
            };
            continue;
        }
        match section {
            "name" if !t.is_empty() => name.push(t.to_string()),
            "name" => section = "",
            "description" => desc.push(line.trim_end()),
            _ => {}
        }
    }
    (name.join(" "), desc.join("\n").trim().to_string())
}

pub fn text(name: &str, description: &str) -> String {
    format!("\n[friendlyname]\n{}\n\n[description]\n{}\n[end]\n", name.trim(), description.trim())
}

/// UTF-16 with a byte order mark and CR LF.
fn utf16(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in text.replace("\r\n", "\n").replace('\n', "\r\n").encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

impl Descriptions {
    pub fn load(dir: &Path) -> Descriptions {
        let mut d = Descriptions::default();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let l = n.to_lowercase();
            if let Some(code) = l.strip_prefix("global_").and_then(|r| r.strip_suffix(".dsc")) {
                if let Ok(b) = std::fs::read(e.path()) {
                    d.langs.insert(code.to_uppercase(), parse(&omsi_cfg::decode_text(&b)));
                }
            }
        }
        d
    }

    pub fn set(&mut self, code: &str, name: String, description: String) {
        self.langs.insert(code.to_string(), (name, description));
        self.changed.insert(code.to_string());
        self.removed.remove(code);
        self.mark();
    }

    pub fn remove(&mut self, code: &str) {
        if self.langs.remove(code).is_some() {
            self.removed.insert(code.to_string());
            self.changed.remove(code);
            self.mark();
        }
    }

    /// Put back as it was (undo, redo) in place of `replaced`.
    pub fn restored(&mut self, replaced: &Descriptions) {
        self.changed = self.langs.keys().cloned().collect();
        // (and those still to be taken away, of either state: removed and not saved since)
        let pending = std::mem::take(&mut self.removed);
        self.removed = replaced.langs.keys().chain(&pending).chain(&replaced.removed).filter(|k| !self.langs.contains_key(*k)).cloned().collect();
        self.mark();
    }

    /// What saving writes and takes away in the map folder.
    pub fn plan(&self, map_dir: &Path) -> Vec<Op> {
        if !self.dirty {
            return Vec::new();
        }
        let at = |code: &str| {
            let want = file_of(code).to_lowercase();
            std::fs::read_dir(map_dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase() == want).unwrap_or(false))).unwrap_or_else(|| map_dir.join(file_of(code)))
        };
        let mut ops: Vec<Op> = self.removed.iter().map(|c| Op::Remove(at(c))).collect();
        for c in &self.changed {
            if let Some((n, d)) = self.langs.get(c) {
                ops.push(Op::Write(at(c), utf16(&text(n, d))));
            }
        }
        ops
    }

    fn mark(&mut self) {
        self.dirty = true;
        self.edit = crate::doc::next_rev();
    }

    pub fn saved(&mut self) {
        self.changed.clear();
        self.removed.clear();
        self.dirty = false;
    }
}

/// The description in the map's properties: a language picker above it (the map's own,
/// in `global.cfg`, or one of the `.dsc` files).
pub fn editor(ui: &mut egui::Ui, doc: &mut crate::doc::Document, history: &mut crate::history::History) {
    let before = doc.descriptions.clone();
    let id = ui.id().with("dsc language");
    let mut lang: String = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    let label = |code: &str| if code.is_empty() { tr("The map's own (global.cfg)", "Своё карты (global.cfg)").to_string() } else { LANGS.iter().find(|l| l.0 == code).map(|l| format!("{} ({code})", l.1)).unwrap_or_else(|| code.to_string()) };
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Description", "Описание")).color(Color32::from_rgb(140, 143, 150)));
        egui::ComboBox::from_id_salt("dsc lang").selected_text(label(&lang)).width(170.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut lang, String::new(), label(""));
            for (code, _) in LANGS {
                let has = doc.descriptions.langs.contains_key(*code);
                let text = format!("{}{}", label(code), if has { "  •" } else { "" });
                ui.selectable_value(&mut lang, code.to_string(), text);
            }
        });
    });
    if lang.is_empty() {
        ui.add(egui::TextEdit::multiline(&mut doc.global.description).desired_rows(4).desired_width(f32::INFINITY));
    } else {
        let (mut name, mut desc) = doc.descriptions.langs.get(&lang).cloned().unwrap_or_else(|| (doc.global.friendly_name.clone(), String::new()));
        let had = doc.descriptions.langs.contains_key(&lang);
        let a = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr("Name in this language", "Название на этом языке")).desired_width(f32::INFINITY));
        let b = ui.add(egui::TextEdit::multiline(&mut desc).hint_text(tr("Description in this language", "Описание на этом языке")).desired_rows(4).desired_width(f32::INFINITY));
        if a.changed() || b.changed() {
            doc.descriptions.set(&lang, name, desc);
        }
        if had && ui.small_button(tr("Remove this language", "Убрать этот язык")).clicked() {
            doc.descriptions.remove(&lang);
        }
        if !had {
            ui.label(RichText::new(tr("Not written yet: it is saved as soon as you type.", "Пока нет: сохранится, как только вы начнёте писать.")).small().color(Color32::from_rgb(140, 143, 150)));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, lang));
    if doc.descriptions != before {
        history.push_snapshot(tr("Edit description", "Правка описания"), crate::history::Snapshot { descriptions: Some(before), ..Default::default() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grundorf's files read; written and read back the same (and as openOMSI's launcher
    /// parses them).
    #[test]
    fn descriptions_round_trip() {
        let mut d = Descriptions::default();
        d.set("ENG", "Ring town".into(), "A ring road.\nTwo stops.".into());
        d.set("RUS", "Кольцо".into(), "Кольцевая дорога.".into());
        let dir = std::env::temp_dir().join(format!("openmapper-dsc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("global_fra.dsc"), b"old").unwrap();
        d.remove("FRA");
        d.removed.insert("FRA".into());
        crate::doc::safe::commit(&d.plan(&dir)).unwrap();
        d.saved();
        let back = Descriptions::load(&dir);
        assert_eq!(back.langs, d.langs);
        assert!(!dir.join("global_fra.dsc").exists());
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(root) = crate::install::find_omsi_root(&crate::install::Prefs::default()) {
            let g = Descriptions::load(&root.join("maps").join("Grundorf"));
            assert_eq!(g.langs["ENG"].0, "Grundorf (Tutorial)");
            assert!(g.langs["ENG"].1.starts_with("Grundorf is a fictional village"));
        }
    }

    /// A language removed, then another edit, then that edit undone: the removed one's file
    /// still goes on the next save.
    #[test]
    fn an_undo_keeps_a_language_removed_before() {
        let dir = std::env::temp_dir().join(format!("openmapper-dsc-undo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("global_FRA.dsc"), utf16(&text("Ville", "x"))).unwrap();
        let loaded = Descriptions::load(&dir);
        let mut removed = loaded.clone();
        removed.remove("FRA");
        let mut edited = removed.clone();
        edited.set("ENG", "Town".into(), "y".into());
        let mut undone = removed.clone();
        undone.restored(&edited);
        crate::doc::safe::commit(&undone.plan(&dir)).unwrap();
        assert!(Descriptions::load(&dir).langs.is_empty(), "{:?}", Descriptions::load(&dir).langs);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
