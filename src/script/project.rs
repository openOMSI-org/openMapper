//! A map's scripts as a project: the plugin folder `plugins/map_<folder>/` of the content
//! folder, which openOMSI loads as a folder plugin (`main.lua` and the modules it requires)
//! and the Development Tools' compiler builds into one `.oop`:
//!
//! ```text
//! plugins/map_<folder>/
//!   openomsi-plugin.toml   the plugin's details and permissions (for an .oop)
//!   main.lua               made by openMapper: starts the scripts in order, on this map only
//!   openmapper/map.lua     made by openMapper: `map.on`, `map.every`... for this map
//!   scripts/*.lua          the map's scripts: each starts when the map is played
//!   modules/**/*.lua       code the scripts share: require("modules.name")
//!   data/                  files the scripts read
//!   .openmapper.json       the scripts' order and which are switched off
//! ```
//!
//! One script failing to start leaves the others running, as separate scripts in an engine's
//! script service do. A Rust plugin of the map is a project of its own beside it
//! (`plugins/map_<folder>_rs/`), built into `plugins/map_<folder>_rs.oop`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

pub const SCRIPTS: &str = "scripts";
pub const MODULES: &str = "modules";
const SETTINGS: &str = ".openmapper.json";
const MANIFEST: &str = "openomsi-plugin.toml";
/// The line that tells openMapper's own files from the author's.
const MADE: &str = "-- Made by openMapper";

/// What a file of the project is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// `scripts/*.lua`: runs when the map is played.
    Script,
    /// `modules/**/*.lua`: required by the scripts.
    Module,
    /// openMapper's own (`main.lua`, `openmapper/`): written again on every save.
    Made,
    /// The plugin's details.
    Manifest,
    /// Anything else (data, pictures, sounds).
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// From the project's folder, with `/`.
    pub rel: String,
    pub kind: Kind,
    /// A script that is switched off is not started (kept, and still checked).
    pub on: bool,
}

impl File {
    /// `modules.util` for `modules/util.lua` (what `require` takes); the file name otherwise.
    pub fn require_name(&self) -> String {
        let s = self.rel.strip_suffix(".lua").unwrap_or(&self.rel);
        s.strip_suffix("/init").unwrap_or(s).replace('/', ".")
    }

    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }

    pub fn is_lua(&self) -> bool {
        self.rel.ends_with(".lua")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Settings {
    /// The scripts in the order they start (file names in `scripts/`).
    #[serde(default)]
    order: Vec<String>,
    /// Scripts switched off.
    #[serde(default)]
    off: BTreeSet<String>,
}

/// A map's script project.
pub struct Project {
    pub dir: PathBuf,
    pub map_folder: String,
}

/// The plugin's folder name: openOMSI takes letters, digits, `-` and `_`.
pub fn plugin_name(map_folder: &str) -> String {
    let s: String = map_folder.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("map_{s}")
}

/// A name a script or module file can have (`Traffic by hour` → `traffic_by_hour`).
pub fn file_stem(name: &str) -> String {
    let s: String = name.trim().trim_end_matches(".lua").chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "script".into() } else { s }
}

/// A Lua string literal of `s`.
fn lua_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\r', '\n'], " "))
}

fn read(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| omsi_cfg::decode_text(&b).replace("\r\n", "\n"))
}

fn write(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    crate::doc::safe::write_atomic(path, text.as_bytes())?;
    Ok(())
}

impl Project {
    pub fn of(content: &Path, map_folder: &str) -> Project {
        Project { dir: content.join("plugins").join(plugin_name(map_folder)), map_folder: map_folder.to_string() }
    }

    /// The Rust plugin's project folder, beside this one.
    pub fn rust_dir(&self) -> PathBuf {
        self.dir.with_file_name(format!("{}_rs", plugin_name(&self.map_folder)))
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    pub fn exists(&self) -> bool {
        self.dir.join("main.lua").is_file()
    }

    fn settings(&self) -> Settings {
        read(&self.dir.join(SETTINGS)).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save_settings(&self, s: &Settings) -> anyhow::Result<()> {
        write(&self.dir.join(SETTINGS), &serde_json::to_string_pretty(s)?)
    }

    /// Every file of the project: the scripts in their order first, then the modules, then
    /// the rest by name.
    pub fn files(&self) -> Vec<File> {
        let mut out = Vec::new();
        let mut stack = vec![self.dir.clone()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || matches!(name.as_str(), "dist" | "target") {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let rel = p.strip_prefix(&self.dir).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                let kind = if rel == MANIFEST {
                    Kind::Manifest
                } else if rel == "main.lua" || rel.starts_with("openmapper/") {
                    Kind::Made
                } else if rel.ends_with(".lua") && rel.starts_with("scripts/") && !rel["scripts/".len()..].contains('/') {
                    Kind::Script
                } else if rel.ends_with(".lua") && rel.starts_with("modules/") {
                    Kind::Module
                } else {
                    Kind::Data
                };
                out.push(File { rel, kind, on: true });
            }
        }
        let s = self.settings();
        for f in out.iter_mut().filter(|f| f.kind == Kind::Script) {
            f.on = !s.off.contains(f.name());
        }
        let place = |f: &File| s.order.iter().position(|n| n == f.name()).unwrap_or(usize::MAX);
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| place(a).cmp(&place(b))).then_with(|| a.rel.cmp(&b.rel)));
        out
    }

    pub fn scripts(&self) -> Vec<File> {
        self.files().into_iter().filter(|f| f.kind == Kind::Script).collect()
    }

    /// Make the folder a project: the map's old one-file script (openMapper 0.1.24 and
    /// before: `main.lua` with a header) becomes `scripts/main.lua`, reading `on` and `every`
    /// as it did. Nothing happens to a project already made.
    pub fn create(&self, friendly: &str) -> anyhow::Result<()> {
        let main = self.dir.join("main.lua");
        if let Some(old) = read(&main).filter(|t| !t.starts_with(MADE)) {
            const MARK: &str = "-- ===== your script below =====";
            let body = match old.find(MARK) {
                Some(i) => old[i + MARK.len()..].trim_start_matches('\n').to_string(),
                None => old,
            };
            let text = format!("-- The map's script of earlier versions, as it was.\nlocal on, every, here, MAP = map.on, map.every, map.here, map.folder\n\n{body}");
            write(&self.path("scripts/main.lua"), &text)?;
        }
        if !self.dir.join(MANIFEST).is_file() {
            write(&self.dir.join(MANIFEST), &manifest(&self.map_folder, friendly))?;
        }
        std::fs::create_dir_all(self.path(SCRIPTS))?;
        std::fs::create_dir_all(self.path(MODULES))?;
        self.write_made(friendly)
    }

    /// openMapper's own files, from the scripts as they are now.
    pub fn write_made(&self, friendly: &str) -> anyhow::Result<()> {
        write(&self.path("openmapper/map.lua"), &map_module(&self.map_folder, friendly))?;
        let names: Vec<String> = self.scripts().into_iter().filter(|f| f.on).map(|f| f.require_name()).collect();
        write(&self.dir.join("main.lua"), &loader(friendly, &names))
    }

    /// A new script (in `scripts/`) or module (in `modules/`), with `text`; its file.
    pub fn add(&self, kind: Kind, name: &str, text: &str, friendly: &str) -> anyhow::Result<File> {
        let folder = if kind == Kind::Module { MODULES } else { SCRIPTS };
        let stem = file_stem(name);
        let mut rel = format!("{folder}/{stem}.lua");
        let mut n = 2;
        while self.path(&rel).exists() {
            rel = format!("{folder}/{stem}_{n}.lua");
            n += 1;
        }
        write(&self.path(&rel), text)?;
        if kind == Kind::Script {
            let mut s = self.settings();
            s.order.retain(|o| self.path(&format!("{SCRIPTS}/{o}")).exists());
            s.order.push(rel.rsplit('/').next().unwrap_or_default().to_string());
            self.save_settings(&s)?;
        }
        self.write_made(friendly)?;
        Ok(File { rel, kind, on: true })
    }

    pub fn save_file(&self, rel: &str, text: &str) -> anyhow::Result<()> {
        write(&self.path(rel), text)?;
        crate::hot::own_change();
        Ok(())
    }

    pub fn read_file(&self, rel: &str) -> Option<String> {
        read(&self.path(rel))
    }

    /// A file renamed within its folder; its new path.
    pub fn rename(&self, rel: &str, new_name: &str, friendly: &str) -> anyhow::Result<String> {
        let (dir, old) = rel.rsplit_once('/').unwrap_or(("", rel));
        let ext = old.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default();
        let stem = if ext == ".lua" { file_stem(new_name) } else { new_name.trim().trim_end_matches(&ext).to_string() };
        anyhow::ensure!(!stem.is_empty(), "{}", tr("a name is needed", "нужно имя"));
        let new_rel = if dir.is_empty() { format!("{stem}{ext}") } else { format!("{dir}/{stem}{ext}") };
        anyhow::ensure!(!self.path(&new_rel).exists(), "{}", tr("a file of that name is there already", "файл с таким именем уже есть"));
        std::fs::rename(self.path(rel), self.path(&new_rel))?;
        let mut s = self.settings();
        let new_name = new_rel.rsplit('/').next().unwrap_or_default().to_string();
        for o in s.order.iter_mut().filter(|o| o.as_str() == old) {
            *o = new_name.clone();
        }
        if s.off.remove(old) {
            s.off.insert(new_name);
        }
        self.save_settings(&s)?;
        self.write_made(friendly)?;
        Ok(new_rel)
    }

    /// A file taken out of the project: into the content folder's `openMapper\bin`, as the
    /// catalogue deletes, so nothing is lost by a slip.
    pub fn delete(&self, content: &Path, rel: &str, friendly: &str) -> anyhow::Result<()> {
        let bin = content.join("openMapper").join("bin").join("scripts").join(plugin_name(&self.map_folder));
        let to = bin.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        let _ = std::fs::remove_file(&to);
        std::fs::rename(self.path(rel), &to).or_else(|_| std::fs::copy(self.path(rel), &to).and_then(|_| std::fs::remove_file(self.path(rel))))?;
        self.write_made(friendly)
    }

    pub fn set_on(&self, name: &str, on: bool, friendly: &str) -> anyhow::Result<()> {
        let mut s = self.settings();
        if on {
            s.off.remove(name);
        } else {
            s.off.insert(name.to_string());
        }
        self.save_settings(&s)?;
        self.write_made(friendly)
    }

    /// A script moved `by` places in the order the scripts start.
    pub fn move_script(&self, name: &str, by: i32, friendly: &str) -> anyhow::Result<()> {
        let mut s = self.settings();
        s.order = self.scripts().into_iter().map(|f| f.name().to_string()).collect();
        let Some(i) = s.order.iter().position(|n| n == name) else { return Ok(()) };
        let j = (i as i32 + by).clamp(0, s.order.len() as i32 - 1) as usize;
        let n = s.order.remove(i);
        s.order.insert(j, n);
        self.save_settings(&s)?;
        self.write_made(friendly)
    }

    /// The modules a script can require, as `require` names them.
    pub fn module_names(&self) -> Vec<String> {
        self.files().into_iter().filter(|f| f.kind == Kind::Module).map(|f| f.require_name()).collect()
    }

    /// Every Lua file's text, for a test run or the checks (`rel` → text).
    pub fn sources(&self) -> Vec<(String, String)> {
        self.files().into_iter().filter(|f| f.is_lua()).filter_map(|f| Some((f.rel.clone(), self.read_file(&f.rel)?))).collect()
    }
}

/// `openmapper/map.lua`: what a script of this map calls instead of `omsi.on` and the like,
/// so it hears only what happens while this map is played.
fn map_module(map_folder: &str, friendly: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{MADE} for the map {}: written again on every save.", friendly.replace(['\r', '\n'], " "));
    let _ = writeln!(s, "-- `map` is what the map's scripts use instead of omsi.on, omsi.every... so that they");
    let _ = writeln!(s, "-- work only while this map is played (the plugin is loaded on every map).");
    let _ = writeln!(s, "local map = {{ folder = {}, name = {} }}", lua_string(map_folder), lua_string(friendly));
    s.push_str(
        r#"
--- Whether the game is on this map now.
function map.here() return omsi.map.name() == map.folder end

--- omsi.on, on this map only.
function map.on(event, fn)
  return omsi.on(event, function(...) if map.here() then return fn(...) end end)
end

--- omsi.every, on this map only.
function map.every(seconds, fn)
  return omsi.every(seconds, function(...) if map.here() then return fn(...) end end)
end

--- omsi.after, on this map only.
function map.after(seconds, fn)
  return omsi.after(seconds, function(...) if map.here() then return fn(...) end end)
end

--- omsi.watch, on this map only.
function map.watch(kind, name, fn)
  if fn == nil then kind, name, fn = "var", kind, name end
  return omsi.watch(kind, name, function(...) if map.here() then return fn(...) end end)
end

--- A line in the game's log, tagged with the script that wrote it.
function map.log(...) omsi.log(...) end

return map
"#,
    );
    s
}

/// `main.lua`: the map's scripts started in order, each on its own (one that fails is logged
/// and the others go on).
fn loader(friendly: &str, scripts: &[String]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{MADE} (Map > Scripts) for the map {}: it starts the map's scripts.", friendly.replace(['\r', '\n'], " "));
    let _ = writeln!(s, "-- Written again on every save: write in scripts/ and modules/, not here.");
    let _ = writeln!(s, "map = require(\"openmapper.map\")");
    let _ = writeln!(s, "for _, name in ipairs({{ {} }}) do", scripts.iter().map(|n| lua_string(n)).collect::<Vec<_>>().join(", "));
    let _ = writeln!(s, "  local ok, err = pcall(require, name)");
    let _ = writeln!(s, "  if not ok then omsi.error(\"script \" .. name .. \" did not start: \" .. tostring(err)) end");
    let _ = writeln!(s, "end");
    s
}

/// The plugin's details, as the Development Tools write them; an `.oop` built from the
/// project gets its permissions from what the scripts call.
fn manifest(map_folder: &str, friendly: &str) -> String {
    let id: String = plugin_name(map_folder).chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let toml_string = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    format!(
        "# openOMSI plugin project of the map {friendly}, made by openMapper.\n\
         [plugin]\n\
         id = \"org.openmapper.{id}\"\n\
         name = {}\n\
         version = \"0.1.0\"\n\
         kind = \"lua\"\n\
         entry = \"main.lua\"\n\
         description = {}\n\
         min_openomsi = \"0.2.21\"\n\
         permissions = []\n\n\
         [lua]\n\
         obfuscation_level = 2\n",
        toml_string(&format!("{friendly} (map scripts)")),
        toml_string(&format!("The scripts of the map {friendly}")),
    )
}

/// Ready-made scripts to start from.
pub struct Template {
    pub title: &'static str,
    pub name: &'static str,
    pub text: &'static str,
}

pub fn templates() -> Vec<Template> {
    vec![
        Template { title: tr("Empty script", "Пустой скрипт"), name: "script", text: "-- What should happen on this map. `map.on`, `map.every`, `map.after` work while it is played.\n-- map.on(\"stop_arrive\", function(name, id, number, delay) end)\n" },
        Template { title: tr("Empty module", "Пустой модуль"), name: "module", text: "-- Code the map's scripts share: local m = require(\"modules.<name>\")\nlocal M = {}\n\nfunction M.hello(who)\n  return \"Hello, \" .. who\nend\n\nreturn M\n" },
        Template { title: tr("A greeting when the bus is taken", "Приветствие, когда садишься в автобус"), name: "greeting", text: "map.on(\"vehicle\", function(name)\n  if name then\n    omsi.message(\"Welcome! You drive the \" .. name .. \" on \" .. map.name, 6)\n  end\nend)\n" },
        Template { title: tr("Announce every stop", "Объявлять каждую остановку"), name: "announce_stops", text: "-- the stop's name and how late (or early) the bus is\nmap.on(\"stop_arrive\", function(name, id, number, delay)\n  omsi.ui.toast(string.format(\"%d. %s, %s\", number, name, omsi.fmt.delay(delay)))\nend)\n" },
        Template { title: tr("Traffic by the hour", "Движение по часам"), name: "traffic_by_hour", text: "-- busy in the rush hours, quiet at night\nlocal function density(h)\n  if h >= 7 and h < 9 or h >= 16 and h < 18 then return 300 end\n  if h >= 22 or h < 5 then return 40 end\n  return 150\nend\n\nmap.on(\"hour\", function(h)\n  omsi.traffic.set_density(density(h))\nend)\n" },
        Template { title: tr("Weather at a time", "Погода в заданное время"), name: "evening_clouds", text: "-- the sky closes at six in the evening\nmap.on(\"hour\", function(h)\n  if h == 18 then\n    omsi.weather.set({ clouds = \"Overcast 1\" })\n  end\nend)\n" },
        Template { title: tr("Speed limit warning", "Предупреждение о скорости"), name: "speed_warning", text: "-- once a second: over the lane's speed limit\nmap.every(1, function()\n  local limit = omsi.map.speed_limit()\n  local kmh = omsi.var(\"Velocity\")\n  if limit and kmh and kmh > limit + 5 then\n    omsi.message(string.format(\"Slow down: %.0f km/h, the limit is %.0f\", kmh, limit), 2)\n  end\nend)\n" },
        Template { title: tr("A counter kept between games", "Счётчик, сохраняемый между играми"), name: "trips_counter", text: "-- how many trips were driven on this map, kept in the plugin's storage\nmap.on(\"trip_done\", function()\n  local n = (omsi.storage.get(\"trips\") or 0) + 1\n  omsi.storage.set(\"trips\", n)\n  omsi.ui.toast(\"Trips on this map: \" .. n)\nend)\n" },
    ]
}

/// A Lua script for objects (`[lua_script]` of a `.sco`).
pub const OBJECT_TEMPLATE: &str = "-- A script of scenery objects: one runs for every object of the type within range.\nlocal state = {}\n\nomsi.on(\"object_init\", function(id, type, x, y, z, heading)\n  state[id] = { t = 0 }\nend)\n\nomsi.on(\"object_frame\", function(id, dt)\n  local s = state[id]\n  if not s then return end\n  s.t = s.t + dt\nend)\n\nomsi.on(\"object_click\", function(id, event)\nend)\n\nomsi.on(\"object_gone\", function(id)\n  state[id] = nil\nend)\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("om-script-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_old_one_file_script_becomes_the_first_script_of_the_project() {
        let content = temp("migrate");
        let p = Project::of(&content, "My Town");
        std::fs::create_dir_all(&p.dir).unwrap();
        std::fs::write(p.dir.join("main.lua"), "-- header\nlocal MAP = \"My Town\"\n-- ===== your script below =====\non(\"hour\", function(h) end)\n").unwrap();
        p.create("My Town").unwrap();
        let s = p.read_file("scripts/main.lua").unwrap();
        assert!(s.contains("local on, every, here, MAP = map.on, map.every, map.here, map.folder"));
        assert!(s.ends_with("on(\"hour\", function(h) end)\n"));
        let main = p.read_file("main.lua").unwrap();
        assert!(main.starts_with(MADE) && main.contains("\"scripts.main\""));
        // (made again, nothing of the author's is lost)
        p.create("My Town").unwrap();
        assert_eq!(p.read_file("scripts/main.lua").unwrap(), s);
    }

    #[test]
    fn scripts_start_in_their_order_and_one_switched_off_does_not() {
        let content = temp("order");
        let p = Project::of(&content, "t");
        p.create("T").unwrap();
        let a = p.add(Kind::Script, "First one", "", "T").unwrap();
        let b = p.add(Kind::Script, "second", "", "T").unwrap();
        let m = p.add(Kind::Module, "Util", "return {}", "T").unwrap();
        assert_eq!(a.rel, "scripts/first_one.lua");
        assert_eq!(m.require_name(), "modules.util");
        assert!(p.read_file("main.lua").unwrap().contains("{ \"scripts.first_one\", \"scripts.second\" }"));
        p.move_script("second.lua", -1, "T").unwrap();
        assert!(p.read_file("main.lua").unwrap().contains("{ \"scripts.second\", \"scripts.first_one\" }"));
        p.set_on("first_one.lua", false, "T").unwrap();
        assert!(p.read_file("main.lua").unwrap().contains("{ \"scripts.second\" }"));
        let names: Vec<(String, bool)> = p.scripts().into_iter().map(|f| (f.rel, f.on)).collect();
        assert_eq!(names, vec![(b.rel.clone(), true), (a.rel.clone(), false)]);
        let r = p.rename(&b.rel, "Renamed", "T").unwrap();
        assert_eq!(r, "scripts/renamed.lua");
        assert!(p.read_file("main.lua").unwrap().contains("\"scripts.renamed\""));
        p.delete(&content, &r, "T").unwrap();
        assert!(!p.path(&r).exists());
        assert!(content.join("openMapper/bin/scripts/map_t/scripts/renamed.lua").is_file());
        assert!(p.read_file("main.lua").unwrap().contains("ipairs({  })"));
    }

    #[test]
    fn the_projects_lua_reads_as_lua() {
        let content = temp("parse");
        let p = Project::of(&content, "Ä \"x\"");
        p.create("Ä \"x\"").unwrap();
        p.add(Kind::Script, "a", "", "Ä \"x\"").unwrap();
        for (rel, text) in p.sources() {
            assert!(full_moon::parse(&text).is_ok(), "{rel}: {text}");
        }
        for t in templates() {
            assert!(full_moon::parse(t.text).is_ok(), "{}", t.title);
        }
        assert!(full_moon::parse(OBJECT_TEMPLATE).is_ok());
        let m = oopc::manifest::Manifest::load(&p.dir).unwrap();
        assert_eq!(m.plugin.id, "org.openmapper.map____x_");
        m.to_header().validate().unwrap();
    }
}
