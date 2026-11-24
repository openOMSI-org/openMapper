//! A map's scripts built into one `.oop` with the Development Tools' compiler (`oopc`): the Lua
//! project obfuscated and bundled with its modules, or the map's Rust plugin compiled to
//! WebAssembly by `cargo`. The file carries the permissions the code needs and, when a key is
//! given, its author's signature; the sources never go into it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use oopc::manifest::Manifest;

use crate::i18n::tr;

pub enum Msg {
    Line(String),
    Done(Result<PathBuf, String>),
}

/// The permissions the project's Lua needs written into its manifest (a plain `.lua` plugin has
/// them all; an `.oop` only those it declares). What was set.
pub fn match_permissions(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let needed = oopc::lint::needed_permissions(dir);
    let path = dir.join("openomsi-plugin.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let list = needed.keys().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ");
    let mut out = String::new();
    let mut done = false;
    for line in text.lines() {
        if !done && line.trim_start().starts_with("permissions") && line.contains('=') {
            out.push_str(&format!("permissions = [{list}]\n"));
            done = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !done {
        out = out.replacen("[plugin]\n", &format!("[plugin]\npermissions = [{list}]\n"), 1);
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(needed.into_iter().collect())
}

/// Build the project in `dir` into `dir/dist/<id>.oop`, signed with the key file when given.
fn build(dir: &Path, key: Option<&Path>, log: &(dyn Fn(&str) + Sync)) -> Result<PathBuf, String> {
    let manifest = Manifest::load(dir)?;
    if manifest.plugin.kind == oopc::manifest::ProjectKind::Lua {
        for (perm, by) in match_permissions(dir)? {
            log(&format!("{} {perm} ({by})", tr("needs", "нужно разрешение")));
        }
    }
    let manifest = Manifest::load(dir)?;
    let header = manifest.to_header();
    header.validate().map_err(|e| e.to_string())?;
    let built = oopc::build::compile_with(dir, &manifest, &oopc::build::BuildOptions { cargo: None, log: Some(log) })?;
    for n in &built.notes {
        log(n);
    }
    let mut builder = oop_format::OopBuilder::new(header.clone()).files(built.files);
    let signing = match key {
        Some(k) => Some(oop_format::decode_secret_key(&std::fs::read_to_string(k).map_err(|e| format!("{}: {e}", k.display()))?).map_err(|e| e.to_string())?),
        None => None,
    };
    if let Some(k) = &signing {
        builder = builder.sign(k);
    }
    let bytes = builder.write().map_err(|e| e.to_string())?;
    let out = dir.join("dist").join(format!("{}.oop", header.id));
    std::fs::create_dir_all(out.parent().unwrap_or(dir)).map_err(|e| e.to_string())?;
    std::fs::write(&out, &bytes).map_err(|e| e.to_string())?;
    log(&format!("{} {} ({} KiB)", tr("built", "собрано"), out.display(), bytes.len().div_ceil(1024)));
    match &signing {
        Some(k) => log(&format!("{} {}", tr("signed by", "подписано"), oop_format::fingerprint(&k.verifying_key()))),
        None => log(tr("not signed: the game loads it with a warning that its author is not known", "без подписи: игра загрузит его с предупреждением, что автор неизвестен")),
    }
    Ok(out)
}

/// The build on a thread of its own (a Rust one runs `cargo` for a while): its lines as they come.
pub fn spawn(dir: PathBuf, key: Option<PathBuf>, wake: egui::Context) -> Receiver<Msg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let lines = std::sync::Mutex::new(tx.clone());
        let log = |l: &str| {
            if let Ok(t) = lines.lock() {
                let _ = t.send(Msg::Line(l.to_string()));
            }
            wake.request_repaint();
        };
        let r = build(&dir, key.as_deref(), &log);
        let _ = tx.send(Msg::Done(r));
        wake.request_repaint();
    });
    rx
}

/// The map's Rust plugin: a new project from the Development Tools' template, beside the Lua one.
pub fn create_rust(dir: &Path, map_folder: &str, friendly: &str) -> Result<(), String> {
    let id: String = format!("{}_rs", crate::script::project::plugin_name(map_folder)).chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    oopc::templates::create_with(dir, &oopc::templates::NewProject { name: format!("{friendly} (Rust)"), id: format!("org.openmapper.{id}"), author: "openMapper".into(), rust: true, template: oopc::templates::Template::Hello })?;
    // (the plugin is loaded on every map: it acts on this one only)
    let lib = dir.join("src").join("lib.rs");
    let text = std::fs::read_to_string(&lib).map_err(|e| e.to_string())?;
    let note = format!("// The Rust plugin of the map {friendly} (openMapper: Map > Scripts). Build it there, or with\n// `oopc build`; the game loads plugins/{}.oop. It runs on every map: check\n// `omsi::api::map_name()` against \"{map_folder}\" before acting.\n\n", dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    std::fs::write(&lib, format!("{note}{text}")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::project::{Kind, Project};

    #[test]
    fn a_map_project_builds_into_an_oop_with_the_permissions_it_needs() {
        let content = std::env::temp_dir().join(format!("om-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&content);
        let p = Project::of(&content, "Build Town");
        p.create("Build Town").unwrap();
        p.add(Kind::Script, "toast", "map.on(\"hour\", function(h) omsi.ui.toast(\"It is \" .. h) end)\n", "Build Town").unwrap();
        p.add(Kind::Module, "util", "return {}\n", "Build Town").unwrap();
        let lines = std::sync::Mutex::new(Vec::new());
        let out = build(&p.dir, None, &|l: &str| lines.lock().unwrap().push(l.to_string())).unwrap();
        let oop = oop_format::Oop::read(&std::fs::read(&out).unwrap()).unwrap();
        assert!(oop.header().permissions.iter().any(|x| x == "ui"), "{:?}", oop.header().permissions);
        assert!(!oop.entry().is_empty());
        // (the sources are not in it)
        assert!(!String::from_utf8_lossy(oop.entry()).contains("It is "));
    }
}
