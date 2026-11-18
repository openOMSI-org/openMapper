//! Updates from GitHub: the newest release is looked for at start; depending on the setting
//! it is offered (install, not now, or never again), or installed and the program restarted
//! by itself. The packages are the ones the release workflow publishes; the download is
//! checked against the release's SHA256SUMS before the program's file is replaced.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use egui::RichText;
use sha2::{Digest, Sha256};

use crate::i18n::tr;
use crate::install::Prefs;
use crate::theme::pal;

const LATEST: &str = "https://api.github.com/repos/openOMSI-org/openMapper/releases/latest";
const PAGE: &str = "https://github.com/openOMSI-org/openMapper/releases/latest";

/// What to do about new versions (`Prefs::update_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Offer each new version (the default).
    Ask,
    /// Install it and restart, without asking.
    Auto,
    /// Never look.
    Off,
}

impl Mode {
    pub fn from_code(s: &str) -> Mode {
        match s {
            "auto" => Mode::Auto,
            "off" => Mode::Off,
            _ => Mode::Ask,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Auto => "auto",
            Mode::Off => "off",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub notes: String,
    asset: String,
    url: String,
    sums_url: Option<String>,
}

enum State {
    Idle,
    Checking(Receiver<Result<Option<Release>, String>>),
    Available(Release),
    Installing(Release, Receiver<Result<(), String>>),
    /// Installed; the restart waits for the map to be saved.
    Installed(String),
    UpToDate,
    Failed(String),
}

pub struct Updater {
    state: State,
    started: Option<std::time::Instant>,
    /// The offer was answered "not now" (until the next start).
    declined: bool,
    /// The Updates window (settings and "check now").
    pub window: bool,
    manual: bool,
}

impl Updater {
    pub fn new() -> Updater {
        Updater { state: State::Idle, started: Some(std::time::Instant::now()), declined: false, window: false, manual: false }
    }

    /// Whether this run looks for updates by itself: not a development build, not a
    /// scripted run.
    fn automatic() -> bool {
        std::env::var_os("OPENMAPPER_SHOT").is_none() && std::env::var_os("OPENMAPPER_NO_UPDATE").is_none() && (!cfg!(debug_assertions) || std::env::var_os("OPENMAPPER_UPDATE_CHECK").is_some())
    }

    fn check(&mut self) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(latest());
        });
        self.state = State::Checking(rx);
    }

    fn install(&mut self, r: Release) {
        let (tx, rx) = channel();
        let job = r.clone();
        std::thread::spawn(move || {
            let _ = tx.send(download_and_replace(&job));
        });
        // (a failure of an install is shown, whatever started it)
        self.manual = true;
        self.state = State::Installing(r, rx);
    }

    /// One frame: the look at start, the offer or the window. True when the program should
    /// be restarted now (the new one is in place and nothing is unsaved).
    pub fn frame(&mut self, ctx: &egui::Context, prefs: &mut Prefs, unsaved: bool) -> bool {
        let mode = Mode::from_code(&prefs.update_mode);
        if let Some(t) = self.started {
            if t.elapsed() > Duration::from_secs(4) {
                self.started = None;
                if mode != Mode::Off && Self::automatic() {
                    self.check();
                }
            } else {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        let mut next = None;
        match &mut self.state {
            State::Checking(rx) => {
                ctx.request_repaint_after(Duration::from_millis(300));
                if let Ok(r) = rx.try_recv() {
                    next = Some(match r {
                        Ok(Some(rel)) => State::Available(rel),
                        Ok(None) => State::UpToDate,
                        // (a failed look at start is not worth a message)
                        Err(e) => State::Failed(e),
                    });
                }
            }
            State::Installing(rel, rx) => {
                ctx.request_repaint_after(Duration::from_millis(300));
                if let Ok(r) = rx.try_recv() {
                    next = Some(match r {
                        Ok(()) => State::Installed(rel.version.clone()),
                        Err(e) => State::Failed(e),
                    });
                }
            }
            _ => {}
        }
        if let Some(n) = next {
            self.state = n;
        }
        // automatic mode: a new version is installed without asking
        if mode == Mode::Auto {
            if let State::Available(r) = &self.state {
                let r = r.clone();
                self.install(r);
            }
        }
        if let State::Installed(_) = self.state {
            if !unsaved {
                return true;
            }
        }
        self.ui(ctx, prefs, mode, unsaved);
        false
    }

    fn ui(&mut self, ctx: &egui::Context, prefs: &mut Prefs, mode: Mode, unsaved: bool) {
        let offer = matches!(self.state, State::Available(_)) && mode == Mode::Ask && !self.declined;
        let busy = matches!(self.state, State::Installing(..)) || matches!(self.state, State::Installed(_));
        let failed_shown = matches!(self.state, State::Failed(_)) && self.manual && !self.window;
        if !(offer || self.window || busy || failed_shown) {
            return;
        }
        let mut install = None;
        let mut close = false;
        let mut set_mode = mode;
        let mut check = false;
        let mut open = true;
        egui::Window::new(tr("Updates", "Обновления")).open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(format!("{} {}", tr("This version:", "Эта версия:"), env!("CARGO_PKG_VERSION")));
            match &self.state {
                State::Available(r) => {
                    ui.label(RichText::new(format!("{} {}", tr("New version:", "Новая версия:"), r.version)).strong());
                    if !r.notes.is_empty() {
                        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                            crate::markdown::show(ui, r.notes.trim());
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button(tr("Install and restart", "Установить и перезапустить")).clicked() {
                            install = Some(r.clone());
                        }
                        if ui.button(tr("Not now", "Не сейчас")).clicked() {
                            close = true;
                        }
                        ui.hyperlink_to(tr("What changed", "Что нового"), PAGE);
                    });
                }
                State::Installing(r, _) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("{} {}…", tr("Downloading and installing", "Скачивание и установка"), r.version));
                    });
                }
                State::Installed(v) => {
                    ui.label(format!("{} {v}", tr("Installed version", "Установлена версия")));
                    if unsaved {
                        ui.label(RichText::new(tr("openMapper restarts when you save the map.", "openMapper перезапустится, когда вы сохраните карту.")).color(pal().text_dim));
                    }
                }
                State::Checking(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Looking for a new version…", "Поиск новой версии…"));
                    });
                }
                State::UpToDate => {
                    ui.label(tr("This is the newest version.", "Это самая новая версия."));
                }
                State::Failed(e) => {
                    ui.colored_label(pal().error, e);
                    ui.hyperlink_to(tr("Download it from GitHub by hand", "Скачать вручную с GitHub"), PAGE);
                }
                State::Idle => {}
            }
            if self.window {
                ui.separator();
                ui.label(tr("When there is a new version:", "Когда есть новая версия:"));
                ui.radio_value(&mut set_mode, Mode::Ask, tr("Offer it to me", "Предлагать установить"));
                ui.radio_value(&mut set_mode, Mode::Auto, tr("Install it and restart by itself", "Ставить и перезапускаться самому"));
                ui.radio_value(&mut set_mode, Mode::Off, tr("Never look for updates", "Не проверять обновления"));
                if ui.add_enabled(!matches!(self.state, State::Checking(_) | State::Installing(..) | State::Installed(_)), egui::Button::new(tr("Check now", "Проверить сейчас"))).clicked() {
                    check = true;
                }
            } else if offer {
                ui.separator();
                if ui.button(tr("Never offer updates again", "Больше не предлагать обновления")).clicked() {
                    set_mode = Mode::Off;
                }
            }
        });
        if set_mode != mode {
            prefs.update_mode = set_mode.code().to_string();
            prefs.save();
        }
        if check {
            self.manual = true;
            self.check();
        }
        if let Some(r) = install {
            self.install(r);
        }
        if !open || close {
            if offer {
                self.declined = true;
            }
            self.window = false;
            if matches!(self.state, State::Failed(_) | State::UpToDate) {
                self.state = State::Idle;
            }
            self.manual = false;
        }
        if set_mode == Mode::Off && offer {
            self.declined = true;
        }
    }
}

// ---------------------------------------------------------------------------- the network

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().user_agent(crate::geo::fetch::USER_AGENT).timeout_global(Some(Duration::from_secs(300))).build().into()
}

/// The platform's name in the package files.
fn platform() -> Option<(&'static str, &'static str, &'static str)> {
    let os = if cfg!(target_os = "windows") {
        ("windows", "zip")
    } else if cfg!(target_os = "macos") {
        ("macos", "zip")
    } else if cfg!(target_os = "linux") {
        ("linux", "tar.gz")
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return None;
    };
    Some((os.0, arch, os.1))
}

/// "1.2.3" or "v1.2.3" as numbers.
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    s.trim().trim_start_matches('v').split('-').next()?.split('.').map(|p| p.parse().ok()).collect()
}

pub fn newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The release in GitHub's answer, when it is newer than `current` and has a package for
/// this platform.
pub fn pick(body: &str, current: &str, os: &str, arch: &str, ext: &str) -> Result<Option<Release>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).ok_or("no tag in the answer")?;
    if !newer(tag, current) {
        return Ok(None);
    }
    let version = tag.trim_start_matches('v').to_string();
    let wanted = format!("openMapper-{version}-{os}-{arch}.{ext}");
    let assets = v.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let find = |name: &str| assets.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(name)).and_then(|a| a.get("browser_download_url")).and_then(|u| u.as_str()).map(String::from);
    let Some(url) = find(&wanted) else {
        return Err(format!("{} {version}: {wanted}", tr("The release has no package", "В релизе нет пакета")));
    };
    Ok(Some(Release { version, notes: v.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string(), asset: wanted, url, sums_url: find("SHA256SUMS") }))
}

fn latest() -> Result<Option<Release>, String> {
    let (os, arch, ext) = platform().ok_or_else(|| tr("No automatic updates for this system.", "Для этой системы нет автообновления.").to_string())?;
    let mut r = agent().get(LATEST).header("Accept", "application/vnd.github+json").call().map_err(net)?;
    let body = r.body_mut().read_to_string().map_err(net)?;
    pick(&body, env!("CARGO_PKG_VERSION"), os, arch, ext)
}

fn net(e: ureq::Error) -> String {
    format!("{} ({e})", tr("No connection to GitHub: are you online?", "Нет связи с GitHub: есть ли интернет?"))
}

fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut r = agent().get(url).call().map_err(net)?;
    r.body_mut().with_config().limit(limit).read_to_vec().map_err(net)
}

/// The hash a SHA256SUMS file gives for a file name.
pub fn hash_in(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (h, f) = l.split_once(char::is_whitespace)?;
        (f.trim().trim_start_matches('*') == name).then(|| h.to_lowercase())
    })
}

fn download_and_replace(r: &Release) -> Result<(), String> {
    let bytes = fetch(&r.url, 400 << 20)?;
    // the package must be the one the release lists: no sums, no update
    let sums_url = r.sums_url.as_ref().ok_or_else(|| tr("The release has no SHA256SUMS: not installed.", "В релизе нет SHA256SUMS: не устанавливаю.").to_string())?;
    let sums = String::from_utf8_lossy(&fetch(sums_url, 1 << 20)?).to_string();
    let want = hash_in(&sums, &r.asset).ok_or_else(|| tr("The package is not in SHA256SUMS: not installed.", "Пакета нет в SHA256SUMS: не устанавливаю.").to_string())?;
    let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        return Err(tr("The download does not match its checksum: not installed.", "Скачанное не совпадает с контрольной суммой: не устанавливаю.").to_string());
    }
    let program = extract_program(&r.asset, &bytes)?;
    replace_self(&program).map_err(|e| format!("{}: {e}", tr("Could not replace the program", "Не удалось заменить программу")))
}

/// The program's file out of the package.
fn extract_program(asset: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let exe = if cfg!(windows) { "openmapper.exe" } else { "openmapper" };
    if asset.ends_with(".zip") {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| e.to_string())?;
            let name = f.name().map_err(|e| e.to_string())?.replace('\\', "/");
            let ok = name.rsplit('/').next() == Some(exe) && (!cfg!(target_os = "macos") || name.ends_with("Contents/MacOS/openmapper"));
            if f.is_file() && ok {
                let mut out = Vec::new();
                f.read_to_end(&mut out).map_err(|e| e.to_string())?;
                return Ok(out);
            }
        }
        return Err(tr("The package has no program in it.", "В пакете нет программы.").to_string());
    }
    // a tar.gz (Linux): unpacked by the system's tar into a folder of its own
    let dir = std::env::temp_dir().join(format!("openmapper-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tarball = dir.join("package.tar.gz");
    std::fs::write(&tarball, bytes).map_err(|e| e.to_string())?;
    let st = std::process::Command::new("tar").arg("-xzf").arg(&tarball).arg("-C").arg(&dir).status().map_err(|e| format!("tar: {e}"))?;
    let found = if st.success() { find_file(&dir, exe) } else { None };
    let out = found.map(std::fs::read);
    let _ = std::fs::remove_dir_all(&dir);
    match out {
        Some(Ok(b)) => Ok(b),
        Some(Err(e)) => Err(e.to_string()),
        None => Err(tr("The package has no program in it.", "В пакете нет программы.").to_string()),
    }
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if p.file_name().map(|n| n == name).unwrap_or(false) {
            return Some(p);
        }
    }
    None
}

/// Puts the new program where the running one is. The running file may be renamed (Windows)
/// or replaced (elsewhere) while it runs; the old one is removed at the next start.
fn replace_self(program: &[u8]) -> std::io::Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let new = exe.with_extension("new");
    std::fs::write(&new, program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    // (Windows lets a running program be renamed, not overwritten; elsewhere the new file
    // simply takes the old one's place and the running program keeps its own copy)
    #[cfg(windows)]
    {
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        if let Err(e) = std::fs::rename(&exe, &old).and_then(|_| std::fs::rename(&new, &exe)) {
            if !exe.exists() && old.exists() {
                let _ = std::fs::rename(&old, &exe);
            }
            let _ = std::fs::remove_file(&new);
            return Err(e);
        }
    }
    #[cfg(not(windows))]
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    #[cfg(target_os = "macos")]
    {
        // (the app's signature covered the old program: signed again, ad hoc, like the package)
        if let Some(app) = exe.ancestors().find(|p| p.extension().map(|e| e == "app").unwrap_or(false)) {
            let _ = std::process::Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(app).status();
        }
    }
    Ok(())
}

/// Removes what an update left beside the program (called at start).
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
        let _ = std::fs::remove_file(exe.with_extension("new"));
    }
}

/// Starts the program again (with the map it was given, if any) and ends this one.
pub fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let mut c = std::process::Command::new(exe);
        c.args(std::env::args_os().skip(1));
        let _ = c.spawn();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v0.1.1", "0.1.0"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v0.10.0", "0.9.0"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("v0.0.9", "0.1.0"));
        assert!(!newer("nonsense", "0.1.0"));
    }

    #[test]
    fn release_for_a_platform() {
        let body = r#"{"tag_name":"v0.1.1","body":"notes","assets":[
            {"name":"openMapper-0.1.1-windows-x64.zip","browser_download_url":"https://x/w.zip"},
            {"name":"SHA256SUMS","browser_download_url":"https://x/sums"}]}"#;
        let r = pick(body, "0.1.0", "windows", "x64", "zip").unwrap().unwrap();
        assert_eq!(r.version, "0.1.1");
        assert_eq!(r.url, "https://x/w.zip");
        assert_eq!(r.sums_url.as_deref(), Some("https://x/sums"));
        assert!(pick(body, "0.1.1", "windows", "x64", "zip").unwrap().is_none());
        assert!(pick(body, "0.1.0", "linux", "x64", "tar.gz").is_err());
    }

    #[test]
    fn sums() {
        let s = "AB12  openMapper-0.1.1-windows-x64.zip\ncd34 *other.zip\n";
        assert_eq!(hash_in(s, "openMapper-0.1.1-windows-x64.zip").as_deref(), Some("ab12"));
        assert_eq!(hash_in(s, "other.zip").as_deref(), Some("cd34"));
        assert_eq!(hash_in(s, "none"), None);
    }
}
