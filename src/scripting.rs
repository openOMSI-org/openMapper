//! OMSI 2's object scripts (`.osc`): what a scenery object runs every frame, checked with
//! openOMSI's own compiler, so what is reported is what the game would say; and lights
//! switched by a script. The map's Lua scripts for openOMSI are `crate::script`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::editable::{self, Source, When};
use crate::i18n::tr;

/// A problem in a script: the line (1-based, 0 when it belongs to no line) and what is wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub line: usize,
    pub message: String,
}

// ------------------------------------------------------------------------------------ OMSI 2

/// The variables a script reads or writes, as `(X.L.name)` / `(X.$.name)`: the object's own
/// numbers and its texts (system variables and constants are not the object's).
pub fn used_vars(osc: &str) -> (Vec<String>, Vec<String>) {
    let (mut nums, mut strs) = (Vec::new(), Vec::new());
    for line in osc.lines() {
        // (the rest of a line after ' is a comment)
        let code = line.split('\'').next().unwrap_or("");
        let mut rest = code;
        while let Some(i) = rest.find('(') {
            rest = &rest[i + 1..];
            let Some(j) = rest.find(')') else { break };
            let inner = &rest[..j];
            let mut parts = inner.splitn(3, '.');
            if let (Some(a), Some(b), Some(name)) = (parts.next(), parts.next(), parts.next()) {
                let name = name.trim();
                if matches!(a, "L" | "S") && !name.is_empty() && !name.contains(char::is_whitespace) {
                    let list = match b {
                        "L" => Some(&mut nums),
                        "$" => Some(&mut strs),
                        _ => None,
                    };
                    if let Some(list) = list {
                        if !list.iter().any(|x: &String| x.eq_ignore_ascii_case(name)) {
                            list.push(name.to_string());
                        }
                    }
                }
            }
            rest = &rest[j + 1..];
        }
    }
    (nums, strs)
}

/// An object's scripts as its `.sco` lists them: the script files, the lists of its variables
/// and of its texts, the constant files (each as written, relative to the object's folder).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScoScripts {
    pub scripts: Vec<String>,
    pub varlists: Vec<String>,
    pub strlists: Vec<String>,
    pub consts: Vec<String>,
}

pub fn sco_scripts(sco: &str) -> ScoScripts {
    let lines = omsi_cfg::split_lines(sco);
    let mut out = ScoScripts::default();
    let mut i = 0;
    while i < lines.len() {
        let target = match omsi_cfg::keyword_of(&lines[i]).map(str::to_ascii_lowercase).as_deref() {
            Some("script") => Some(&mut out.scripts),
            Some("varnamelist") => Some(&mut out.varlists),
            Some("stringvarnamelist") => Some(&mut out.strlists),
            Some("constfile") => Some(&mut out.consts),
            _ => None,
        };
        i += 1;
        if let Some(list) = target {
            let n: usize = lines.get(i).and_then(|l| l.trim().parse().ok()).unwrap_or(0);
            i += 1;
            for _ in 0..n {
                if let Some(l) = lines.get(i).filter(|l| omsi_cfg::keyword_of(l).is_none()) {
                    let l = l.trim();
                    if !l.is_empty() {
                        list.push(l.to_string());
                    }
                    i += 1;
                }
            }
        }
    }
    out
}

fn names_in(text: &str) -> Vec<String> {
    text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

static SCRATCH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Compile `osc` with the variables `vars` and the texts `strvars` declared and the constant
/// files `consts`, as the game would; the problems it finds.
pub fn check_osc(osc: &str, vars: &[String], strvars: &[String], consts: &[PathBuf]) -> Vec<Diag> {
    let dir = std::env::temp_dir().join(format!("openmapper-osc-{}-{}", std::process::id(), SCRATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let script = dir.join("script.osc");
    let (v, s) = (dir.join("vars.txt"), dir.join("strings.txt"));
    let _ = std::fs::write(&script, osc.replace("\r\n", "\n").replace('\n', "\r\n"));
    let _ = std::fs::write(&v, vars.join("\r\n"));
    let _ = std::fs::write(&s, strvars.join("\r\n"));
    let program = omsi_script::compile(&omsi_script::CompileInput { varlists: vec![v], stringvarlists: vec![s], constfiles: consts.to_vec(), scripts: vec![script.clone()], ..Default::default() });
    let _ = std::fs::remove_dir_all(&dir);
    program
        .errors
        .iter()
        .map(|e| Diag { line: if e.file == script { e.line } else { 0 }, message: e.message.clone() })
        .collect()
}

/// The variables and texts an object's script lists name, and its constant files, from the
/// object's folder `dir` (what the check declares).
pub fn declared(dir: &Path, sco: &ScoScripts) -> (Vec<String>, Vec<String>, Vec<PathBuf>) {
    let read = |list: &[String]| -> Vec<String> { list.iter().flat_map(|f| std::fs::read(omsi_cfg::resolve_path(dir, f)).map(|b| names_in(&omsi_cfg::decode_text(&b))).unwrap_or_default()).collect() };
    (read(&sco.varlists), read(&sco.strlists), sco.consts.iter().map(|f| omsi_cfg::resolve_path(dir, f)).collect())
}

/// What a scripted light does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Behaviour {
    /// Lit from one hour of the day to another (a shop's sign).
    Hours { from: f32, to: f32 },
    /// Lit and dark in turn, a whole period in seconds (a beacon).
    Blink { period: f32 },
    /// Lit through the months from one to another, 1 to 12 (Christmas lights).
    Months { from: u8, to: u8 },
}

impl Behaviour {
    pub fn label(self) -> &'static str {
        match self {
            Behaviour::Hours { .. } => tr("Lit during certain hours", "Горит в определённые часы"),
            Behaviour::Blink { .. } => tr("Blinks", "Мигает"),
            Behaviour::Months { .. } => tr("Lit in certain months", "Горит в определённые месяцы"),
        }
    }

    /// The script that sets `lit`.
    pub fn osc(self) -> String {
        let mut s = String::from("' A light made in openMapper: the variable lit says whether it shines.\n' Change the rule below as you like; keep the {frame} ... {end} around it.\n\n{frame}\n");
        match self {
            Behaviour::Hours { from, to } => {
                // (past midnight when the end comes before the start)
                let join = if from <= to { "&&" } else { "||" };
                let _ = write!(s, "\t' minutes since midnight\n\t(L.S.Time) 60 / s0\n\t{} >= l0 {} < {join}\n\t(S.L.lit)\n", (from * 60.0).round(), (to * 60.0).round());
            }
            Behaviour::Blink { period } => {
                let p = period.max(0.2);
                let _ = write!(s, "\t' seconds of play, shining for the first half of every period\n\t(L.S.GetTime) {p} % {} < (S.L.lit)\n", p / 2.0);
            }
            Behaviour::Months { from, to } => {
                let join = if from <= to { "&&" } else { "||" };
                let _ = write!(s, "\t(L.S.Month) s0\n\t{from} >= l0 {to} <= {join}\n\t(S.L.lit)\n");
            }
        }
        s.push_str("{end}\n");
        s
    }
}

/// A light that a script of its own switches: where the files go and what to write.
pub fn scripted_light(color: [u8; 3], range: f32, stem: &str) -> Source {
    let mut src = crate::makers::light_source(&crate::makers::Settings { light_color: color, light_range: range, light_night_only: false, ..Default::default() });
    for l in &mut src.lights {
        l.when = When::Var("lit".into());
    }
    src.name = "Scripted light".into();
    src.extra = format!("[script]\n1\nscript\\{stem}.osc\n\n[varnamelist]\n1\nscript\\{stem}_varlist.txt\n");
    src
}

/// Write the script of a scripted light beside the object (after the object itself is saved).
pub fn write_light_script(dir: &Path, stem: &str, b: Behaviour) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir.join("script"))?;
    std::fs::write(dir.join("script").join(format!("{stem}.osc")), b.osc().replace('\n', "\r\n"))?;
    std::fs::write(dir.join("script").join(format!("{stem}_varlist.txt")), "lit\r\n")?;
    Ok(())
}

/// Save a scripted light as a new object of the map: its relative path.
pub fn make_light(roots: &crate::install::Roots, map: &str, b: Behaviour, color: [u8; 3], range: f32) -> anyhow::Result<String> {
    let rel = editable::new_rel(roots, map, "ScriptedLight");
    let (dir, stem) = editable::folder_of(roots, &rel);
    let src = scripted_light(color, range, &stem);
    editable::save(roots, &rel, &src)?;
    write_light_script(&dir, &stem, b)?;
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_are_found_in_a_script() {
        let (n, s) = used_vars("{frame}\n (L.S.Time) 60 / s0 ' (L.L.nope)\n (S.L.lit)\n (L.L.lit) (S.$.name) (C.L.k)\n{end}");
        assert_eq!(n, vec!["lit"]);
        assert_eq!(s, vec!["name"]);
    }

    #[test]
    fn the_game_compiler_checks_a_script() {
        let ok = Behaviour::Hours { from: 18.0, to: 23.0 }.osc();
        assert!(check_osc(&ok, &["lit".into()], &[], &[]).is_empty(), "{:?}", check_osc(&ok, &["lit".into()], &[], &[]));
        for b in [Behaviour::Blink { period: 2.0 }, Behaviour::Months { from: 12, to: 1 }, Behaviour::Hours { from: 22.0, to: 5.0 }] {
            let d = check_osc(&b.osc(), &["lit".into()], &[], &[]);
            assert!(d.is_empty(), "{b:?}: {d:?}");
        }
        // a variable the object does not list is a problem the game would report
        let d = check_osc("{frame}\n 1 (S.L.unknown)\n{end}\n", &[], &[], &[]);
        assert!(!d.is_empty());
    }

    #[test]
    fn an_object_lists_its_scripts() {
        let s = sco_scripts("[model]\nx.x\n\n[script]\n2\nscript\\a.osc\nscript\\b.osc\n\n[varnamelist]\n1\nscript\\v.txt\n\n[constfile]\n1\nscript\\c.cfg\n");
        assert_eq!(s.scripts, vec!["script\\a.osc", "script\\b.osc"]);
        assert_eq!(s.varlists, vec!["script\\v.txt"]);
        assert_eq!(s.consts, vec!["script\\c.cfg"]);
    }
}
