//! Which vehicles drive a line (`car_use/*.ocu`): the types its tours are given (only these,
//! or these with some probability) and the fleet numbers fixed for single tours, for a span of
//! dates. The AI timetable buses take their vehicles from these; a line with none takes any
//! vehicle of its tours' depot group.

use std::collections::BTreeSet;
use std::path::Path;

use omsi_timetable::CarUse;

use super::{encode, Timetable};
use crate::doc::safe::Op;

/// One `.ocu` file: its name (without the extension) and what it says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CarUseFile {
    pub file: String,
    pub use_: CarUse,
}

/// How a file's types are meant: the only ones, or the preferred with a probability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeMode {
    Only,
    Preferred,
}

impl CarUseFile {
    pub fn mode(&self) -> TypeMode {
        if self.use_.types_prefered.is_some() && self.use_.only_types.is_empty() { TypeMode::Preferred } else { TypeMode::Only }
    }

    /// The vehicle types the file names, whichever way.
    pub fn types(&self) -> Vec<String> {
        match self.mode() {
            TypeMode::Only => self.use_.only_types.clone(),
            TypeMode::Preferred => self.use_.types_prefered.as_ref().map(|t| t.1.clone()).unwrap_or_default(),
        }
    }

    /// Set the types and how they are meant (`chance`: for the preferred, 0..1).
    pub fn set_types(&mut self, mode: TypeMode, types: Vec<String>, chance: f32) {
        match mode {
            TypeMode::Only => {
                self.use_.only_types = types;
                self.use_.types_prefered = None;
            }
            TypeMode::Preferred => {
                self.use_.only_types.clear();
                self.use_.types_prefered = Some((chance.clamp(0.0, 1.0), types));
            }
        }
    }
}

/// The text of a file, as OMSI 2's own are laid out (CR LF).
pub fn text(c: &CarUse) -> String {
    let mut o = String::new();
    o.push_str(&format!("[valid]\n{}\n{}\n\n[line]\n{}\n\n", c.valid.0, c.valid.1, c.line));
    if !c.only_types.is_empty() {
        o.push_str("[onlytypes]\n");
        for t in &c.only_types {
            o.push_str(&format!("{t}\n"));
        }
        o.push_str("[end]\n\n");
    }
    if let Some((f, types)) = &c.types_prefered {
        o.push_str(&format!("[types_prefered]\n{f}\n"));
        for t in types {
            o.push_str(&format!("{t}\n"));
        }
        o.push_str("[end]\n\n");
    }
    if !c.number_tour.is_empty() {
        o.push_str("[number_tour]\n");
        for (n, t) in &c.number_tour {
            o.push_str(&format!("{n}\t{t}\n"));
        }
        o.push_str("[end]\n\n");
    }
    for t in &c.type_tour {
        o.push_str(&format!("[type_tour]\n{t}\n[end]\n\n"));
    }
    o
}

/// Every `.ocu` of `map_dir/car_use`, in the order of their names.
pub fn load_dir(map_dir: &Path) -> Vec<CarUseFile> {
    let dir = omsi_cfg::resolve_path(map_dir, "car_use");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("ocu")).unwrap_or(false)).collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    files.iter().filter_map(|p| Some(CarUseFile { file: p.file_stem()?.to_string_lossy().into_owned(), use_: CarUse::load(p).ok()? })).collect()
}

/// A date as yyyymmdd from its parts (clamped to a real-looking one).
pub fn date(y: i32, m: i32, d: i32) -> i32 {
    y.clamp(1900, 2100) * 10_000 + m.clamp(1, 12) * 100 + d.clamp(1, 31)
}

impl Timetable {
    pub fn touch_car_use(&mut self, file: &str) {
        self.car_changed.insert(file.to_lowercase());
        self.car_removed.remove(&file.to_lowercase());
        self.mark();
    }

    /// A new, empty assignment for `line` valid for all years (its file name made unique).
    pub fn add_car_use(&mut self, line: &str) -> usize {
        let base: String = line.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let base = format!("{}_{}", crate::i18n::tr("Vehicles", "Транспорт"), base);
        let mut file = base.clone();
        let mut n = 2;
        while self.car_use.iter().any(|c| c.file.eq_ignore_ascii_case(&file)) {
            file = format!("{base}_{n}");
            n += 1;
        }
        self.car_use.push(CarUseFile { file: file.clone(), use_: CarUse { valid: (19000101, 20991231), line: line.to_string(), ..Default::default() } });
        self.touch_car_use(&file);
        self.car_use.len() - 1
    }

    pub fn remove_car_use(&mut self, i: usize) {
        let c = self.car_use.remove(i);
        self.car_changed.remove(&c.file.to_lowercase());
        self.car_removed.insert(c.file.to_lowercase());
        self.mark();
    }

    /// Files of a line that was renamed follow it; those of one that was deleted go.
    pub fn car_use_follow(&mut self, old: &str, new: Option<&str>) {
        let mut changed = Vec::new();
        for c in self.car_use.iter_mut().filter(|c| c.use_.line.trim().eq_ignore_ascii_case(old.trim())) {
            if let Some(n) = new {
                c.use_.line = n.to_string();
                changed.push(c.file.clone());
            }
        }
        for f in changed {
            self.touch_car_use(&f);
        }
        if new.is_none() {
            let gone: Vec<usize> = self.car_use.iter().enumerate().filter(|(_, c)| c.use_.line.trim().eq_ignore_ascii_case(old.trim())).map(|(i, _)| i).collect();
            for i in gone.into_iter().rev() {
                self.remove_car_use(i);
            }
        }
    }
}

/// What saving writes and takes away of `car_use` in `map_dir`.
pub fn plan(tt: &Timetable, map_dir: &Path) -> Vec<Op> {
    let mut ops = Vec::new();
    let dir = map_dir.join("car_use");
    let at = |name: &str| {
        std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase() == format!("{}.ocu", name.to_lowercase())).unwrap_or(false))).unwrap_or_else(|| dir.join(format!("{name}.ocu")))
    };
    for f in &tt.car_removed {
        let p = at(f);
        if p.is_file() {
            ops.push(Op::Remove(p));
        }
    }
    for c in tt.car_use.iter().filter(|c| tt.car_changed.contains(&c.file.to_lowercase())) {
        ops.push(Op::Write(at(&c.file), encode(&text(&c.use_))));
    }
    ops
}

pub fn saved(tt: &mut Timetable) {
    tt.car_changed = BTreeSet::new();
    tt.car_removed = BTreeSet::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stock_file_reads_and_writes_the_same() {
        let dir = std::env::temp_dir().join(format!("om-caruse-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("car_use")).unwrap();
        let src = "[valid]\r\n19890105\r\n19890105\r\n\r\n[line]\r\n92\r\n\r\n[number_tour]\r\n3010\tMo-Fr 1\r\n3018\tMo-Fr 2\r\n[end]\r\n\r\n[type_tour]\r\nvehicles\\MAN_SD200\\MAN_SD77.bus\tMo-Fr 7\r\n[end]\r\n\r\n";
        std::fs::write(dir.join("car_use/A_92.ocu"), src).unwrap();
        let files = load_dir(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file, "A_92");
        assert_eq!(files[0].use_.number_tour.len(), 2);
        assert_eq!(text(&files[0].use_).replace('\n', "\r\n"), src);
        let mut f = files[0].clone();
        f.set_types(TypeMode::Preferred, vec!["vehicles\\a\\b.bus".into()], 0.9);
        let again = {
            std::fs::write(dir.join("car_use/B.ocu"), encode(&text(&f.use_))).unwrap();
            load_dir(&dir).into_iter().find(|c| c.file == "B").unwrap()
        };
        assert_eq!(again.mode(), TypeMode::Preferred);
        assert_eq!(again.use_.types_prefered.as_ref().map(|t| t.0), Some(0.9));
        let _ = std::fs::remove_dir_all(dir);
    }
}
