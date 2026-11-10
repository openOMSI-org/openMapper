//! Where OMSI 2 and openOMSI live on this computer.
//!
//! openMapper reads the original installation (it ships no content of its own) and writes
//! maps and imported assets into openOMSI's content folder, which the game reads before the
//! installation. The original folder is never written to.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// openOMSI's data folder (`~/.openomsi`): the launcher's settings and, by default, the
/// content folder.
pub fn openomsi_data_dir() -> PathBuf {
    home().join(".openomsi")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The bits of openOMSI's `launcher.json` the editor uses.
#[derive(Debug, Default, Deserialize)]
struct LauncherCfg {
    #[serde(default)]
    root: String,
    #[serde(default)]
    game: String,
}

fn launcher_cfg() -> LauncherCfg {
    std::fs::read_to_string(openomsi_data_dir().join("launcher.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The editor's own settings (`~/.openomsi/openmapper.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Prefs {
    /// The OMSI 2 folder chosen by hand (empty: found by itself).
    #[serde(default)]
    pub omsi_root: String,
    /// "en" or "ru" (empty: the system's language).
    #[serde(default)]
    pub language: String,
    /// Maps opened lately, newest first (global.cfg paths).
    #[serde(default)]
    pub recent: Vec<String>,
    /// Show the welcome hints.
    #[serde(default)]
    pub hints_seen: bool,
    /// The Getting started guide does not open by itself for a new map.
    #[serde(default)]
    pub guide_off: bool,
    /// Catalogue entries marked with a star (relative paths).
    #[serde(default)]
    pub favourites: Vec<String>,
    /// Catalogue entries chosen lately, newest first.
    #[serde(default)]
    pub recent_assets: Vec<String>,
    /// The interface's zoom (0: not chosen, as 1).
    #[serde(default)]
    pub ui_scale: f32,
    /// How the mouse moves the camera: "map" (the default, like map editors and city
    /// builders) or "fly" (like Unity / Unreal: right button looks round, W A S D fly).
    #[serde(default)]
    pub camera: String,
    /// The camera is moved as in OMSI 2's own editor (View > Preset Key Shortcut): the middle button
    /// turns the view, the right button held and moved up and down zooms, a right click on
    /// the ground goes there, the arrows move.
    #[serde(default)]
    pub omsi_controls: bool,
    /// Which keys the editor starts from: "" (as `omsi_controls` says, the default),
    /// "mapper", "omsi" or "blender" (`crate::controls::Preset`).
    #[serde(default)]
    pub keys_set: String,
    /// Only the tiles round the place looked at are loaded, 100 at a time, as in OMSI 2's own
    /// editor (None: not asked yet; the default is off).
    #[serde(default)]
    pub tile_streaming: Option<bool>,
    /// How many tiles one window of streaming holds (0: 100).
    #[serde(default)]
    pub stream_tiles: u32,
    /// How fast and sensitive the camera is (View > Controls).
    #[serde(default)]
    pub cam_tune: crate::camera::Tuning,
    /// The graphics API: "" (the best the system has), "dx12", "vulkan", "metal" or "gl"
    /// (`crate::gpu::Api`). A choice that cannot start falls back to the automatic one.
    #[serde(default)]
    pub graphics_api: String,
    /// Graphics quality: "low", "medium", "high" (empty: high).
    #[serde(default)]
    pub graphics: String,
    /// Imagery sources added by the user and keys for those that need one.
    #[serde(default)]
    pub geo: crate::geo::sources::GeoPrefs,
    /// Where the camera was when each map was left (its folder; the point looked at, yaw,
    /// pitch, distance), newest first: a map opens there again.
    #[serde(default)]
    pub last_views: Vec<(String, [f64; 6])>,
    /// Keyboard commands moved to another key (command id → key as text; empty: no key).
    #[serde(default)]
    pub keys: std::collections::BTreeMap<String, String>,
    /// New versions from GitHub: "ask" (offered; the default), "auto" (installed and the
    /// program restarted by itself) or "off" (not looked for).
    #[serde(default)]
    pub update_mode: String,
    /// The user agreed that test drives in OMSI 2 itself copy the map into its folder.
    #[serde(default)]
    pub omsi2_copy_ok: bool,
    /// What the Test drive window was last set to.
    #[serde(default)]
    pub drive: crate::drive::DriveSettings,
    /// The status bar's snapping switches, as they were left.
    #[serde(default)]
    pub snap: SnapBar,
}

/// The status bar's switches (Auto-Align, Surface, the knife, the ruler, and the rest).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapBar {
    #[serde(default = "default_on")]
    pub auto_align: bool,
    #[serde(default = "default_on")]
    pub auto_adapt: bool,
    #[serde(default = "default_on")]
    pub auto_junction: bool,
    #[serde(default = "default_on")]
    pub auto_switch: bool,
    #[serde(default = "default_on")]
    pub auto_terrain: bool,
    #[serde(default)]
    pub surface: bool,
    #[serde(default)]
    pub knife: bool,
    #[serde(default)]
    pub ruler: bool,
    #[serde(default)]
    pub grid: bool,
    #[serde(default = "default_grid_step")]
    pub grid_step: f64,
    #[serde(default)]
    pub angle: bool,
    #[serde(default = "default_angle_step")]
    pub angle_step: f64,
}

fn default_on() -> bool {
    true
}

fn default_grid_step() -> f64 {
    1.0
}

fn default_angle_step() -> f64 {
    15.0
}

impl Default for SnapBar {
    fn default() -> Self {
        Self { auto_align: true, auto_adapt: true, auto_junction: true, auto_switch: true, auto_terrain: true, surface: false, knife: false, ruler: false, grid: false, grid_step: 1.0, angle: false, angle_step: 15.0 }
    }
}

impl Prefs {
    fn path() -> PathBuf {
        openomsi_data_dir().join("openmapper.json")
    }

    pub fn load() -> Prefs {
        let mut p: Prefs = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // (maps deleted or moved since are not offered)
        p.recent.retain(|m| std::path::Path::new(m).is_file());
        p
    }

    pub fn save(&self) {
        // a screenshot run (checks without a person at the screen) leaves the settings of
        // whoever uses openMapper alone: its maps are not theirs to see among the recent ones
        if std::env::var_os("OPENMAPPER_SHOT").is_some() {
            return;
        }
        let _ = std::fs::create_dir_all(openomsi_data_dir());
        if let Ok(s) = serde_json::to_string_pretty(self) {
            // (all or nothing: a crash while writing must not leave half a file)
            if let Err(e) = crate::doc::safe::write_atomic(&Self::path(), s.as_bytes()) {
                log::warn!("{}: {e}", Self::path().display());
            }
        }
    }

    /// Stars an asset or takes its star away.
    pub fn toggle_favourite(&mut self, rel: &str) {
        let before = self.favourites.len();
        self.favourites.retain(|r| !r.eq_ignore_ascii_case(rel));
        if self.favourites.len() == before {
            self.favourites.push(rel.to_string());
        }
        self.save();
    }

    pub fn push_recent_asset(&mut self, rel: &str) {
        if self.recent_assets.first().map(|r| r.eq_ignore_ascii_case(rel)).unwrap_or(false) {
            return;
        }
        self.recent_assets.retain(|r| !r.eq_ignore_ascii_case(rel));
        self.recent_assets.insert(0, rel.to_string());
        self.recent_assets.truncate(24);
        self.save();
    }

    /// The camera kept for a map folder as it was left.
    pub fn remember_view(&mut self, map_dir: &Path, v: [f64; 6]) {
        let s = map_dir.to_string_lossy().to_string();
        self.last_views.retain(|(d, _)| d != &s);
        self.last_views.insert(0, (s, v));
        self.last_views.truncate(30);
        self.save();
    }

    pub fn last_view(&self, map_dir: &Path) -> Option<[f64; 6]> {
        let s = map_dir.to_string_lossy();
        self.last_views.iter().find(|(d, _)| *d == s).map(|(_, v)| *v)
    }

    pub fn push_recent(&mut self, cfg: &Path) {
        let s = cfg.to_string_lossy().to_string();
        self.recent.retain(|r| r != &s);
        self.recent.insert(0, s);
        self.recent.truncate(8);
        self.save();
    }
}

/// A complete OMSI 2 installation.
pub fn is_omsi_root(p: &Path) -> bool {
    p.is_dir() && omsi_cfg::missing_original_essentials(p).is_empty()
}

/// The OMSI 2 folder: the one chosen in openMapper, the one openOMSI's launcher uses, or
/// one found in the usual places (Steam libraries, next to the program).
pub fn find_omsi_root(prefs: &Prefs) -> Option<PathBuf> {
    let mut first: Vec<PathBuf> = Vec::new();
    if let Some(r) = std::env::var_os("OMSI_ROOT") {
        first.push(PathBuf::from(r));
    }
    if !prefs.omsi_root.is_empty() {
        first.push(PathBuf::from(&prefs.omsi_root));
    }
    let l = launcher_cfg();
    if !l.root.is_empty() {
        first.push(PathBuf::from(&l.root));
    }
    for c in &first {
        for g in omsi_cfg::install_search::root_guesses(c) {
            if is_omsi_root(&g) {
                return Some(g);
            }
        }
    }
    omsi_cfg::find_original_install(&first).filter(|p| is_omsi_root(p))
}

/// openOMSI's content folder: `$OMSI_CONTENT`, the `content` folder beside the game binary
/// openOMSI's launcher starts (when it can be written), else `~/.openomsi/content`.
pub fn content_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OMSI_CONTENT") {
        return PathBuf::from(d);
    }
    let l = launcher_cfg();
    if !l.game.is_empty() {
        if let Some(dir) = Path::new(&l.game).parent() {
            let beside = if dir.ends_with("Contents/MacOS") {
                dir.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).map(Path::to_path_buf)
            } else {
                Some(dir.to_path_buf())
            };
            if let Some(b) = beside {
                let cand = omsi_cfg::content_folder_of(&b);
                if cand.is_dir() && !omsi_cfg::is_programs_folder(&cand) && omsi_cfg::is_writable(&cand) {
                    return cand;
                }
            }
        }
    }
    openomsi_data_dir().join("content")
}

/// OMSI 2's own program (`Omsi.exe`) in an installation, whatever the case of its name.
pub fn omsi2_exe(root: &Path) -> Option<PathBuf> {
    std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).find(|p| p.is_file() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case("omsi.exe")).unwrap_or(false))
}

/// The openOMSI game to test maps with: of every copy found (beside openMapper, the one
/// openOMSI's launcher names and the program beside it, the macOS app), the newest build -
/// an old app left in /Applications must not win over the current one.
pub fn find_game() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["openomsi.exe", "omsi.exe"] } else { &["openomsi", "omsi"] };
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
        }
    }
    let l = launcher_cfg();
    let mut cands: Vec<PathBuf> = Vec::new();
    if !l.game.is_empty() {
        cands.push(PathBuf::from(&l.game));
        if let Some(dir) = Path::new(&l.game).parent() {
            dirs.push(dir.to_path_buf());
        }
    }
    for d in &dirs {
        for n in names {
            cands.push(d.join(n));
        }
    }
    #[cfg(target_os = "macos")]
    {
        cands.push(PathBuf::from("/Applications/openOMSI.app/Contents/MacOS/openomsi"));
        cands.push(home().join("Applications/openOMSI.app/Contents/MacOS/openomsi"));
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = cands
        .into_iter()
        .filter(|p| p.is_file() && p.file_stem().map(|s| s.to_string_lossy() != "openmapper").unwrap_or(false))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    let game = found.into_iter().next().map(|f| f.1);
    if let Some(g) = &game {
        log::info!("openOMSI for test drives: {}", g.display());
    }
    game
}

/// Both content roots in the order the game searches them: the content folder (mods and the
/// maps made here), then the installation.
#[derive(Clone)]
pub struct Roots {
    pub omsi: PathBuf,
    pub content: PathBuf,
}

impl Roots {
    pub fn new(omsi: PathBuf) -> Roots {
        let content = content_dir();
        let _ = omsi_cfg::ensure_content_layout(&content);
        omsi_cfg::add_content_root(content.clone());
        if let Err(e) = omsi_cfg::publisher::mount_packs(&content) {
            log::warn!("publisher packs: {e}");
        }
        omsi_cfg::add_content_root(omsi.clone());
        Roots { omsi, content }
    }

    /// Content roots, the content folder first.
    pub fn all(&self) -> [&Path; 2] {
        [&self.content, &self.omsi]
    }

    /// A path relative to whichever root has it (the content folder first).
    pub fn resolve(&self, rel: &str) -> PathBuf {
        let c = omsi_cfg::resolve_path(&self.content, rel);
        if omsi_cfg::vfs::exists(&c) {
            return c;
        }
        omsi_cfg::resolve_path(&self.omsi, rel)
    }
}

/// A map found on this computer.
#[derive(Debug, Clone)]
pub struct MapEntry {
    pub name: String,
    pub cfg: PathBuf,
    /// Made or copied by openMapper / installed as a mod (in the content folder).
    pub in_content: bool,
}

/// Every map of both roots (a map in the content folder hides the installation's copy).
pub fn list_maps(roots: &Roots) -> Vec<MapEntry> {
    let mut out: Vec<MapEntry> = Vec::new();
    let content_maps = roots.content.join("maps");
    let install_maps = roots.omsi.join("maps");
    let extra = omsi_cfg::content_dirs("maps");
    let mut dirs = vec![content_maps.clone()];
    dirs.extend(extra.iter().filter(|p| p.starts_with(&roots.content) && **p != content_maps && **p != install_maps).cloned());
    dirs.push(install_maps.clone());
    dirs.extend(extra.into_iter().filter(|p| !p.starts_with(&roots.content) && *p != install_maps));
    let mut seen_dirs = std::collections::HashSet::new();
    for dir in dirs {
        if !seen_dirs.insert(dir.clone()) { continue; }
        let Some(entries) = omsi_cfg::vfs::list_dir(&dir) else { continue };
        for (folder, is_dir) in entries {
            if !is_dir { continue; }
            let p = dir.join(folder);
            let cfg = p.join("global.cfg");
            if !omsi_cfg::vfs::is_file(&cfg) {
                continue;
            }
            let folder = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            if out.iter().any(|m| m.cfg.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().eq_ignore_ascii_case(&folder)).unwrap_or(false)) {
                continue;
            }
            let name = friendly_name(&cfg).unwrap_or_else(|| folder.clone());
            out.push(MapEntry { name, cfg, in_content: dir.starts_with(&roots.content) });
        }
    }
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

fn friendly_name(cfg: &Path) -> Option<String> {
    let f = omsi_cfg::CfgFile::read(cfg).ok()?;
    let mut r = f.reader();
    let mut name = None;
    while let Some(k) = r.next_keyword() {
        match k.as_str() {
            "friendlyname" => return Some(r.str().trim().to_string()).filter(|s| !s.is_empty()),
            "name" => name = Some(r.str().trim().to_string()).filter(|s| !s.is_empty()),
            _ => {}
        }
    }
    name
}

#[cfg(test)]
mod publisher_maps {
    use super::*;

    #[test]
    fn protected_publisher_maps_appear_in_the_map_picker() {
        let root = std::env::temp_dir().join(format!("openmapper-publisher-picker-{}", std::process::id()));
        let content = root.join("content");
        let omsi = root.join("omsi");
        std::fs::create_dir_all(content.join("Packs")).unwrap();
        std::fs::create_dir_all(&omsi).unwrap();
        let signer = omsi_pack::SigningKey::generate(&ring::rand::SystemRandom::new()).unwrap();
        let mut trust = omsi_pack::TrustedPublishers::default();
        trust.insert("fixture", signer.key_id(), signer.public_key());
        trust.save_file(&content.join("Publishers.json")).unwrap();
        let pack = content.join("Packs/protected-map.oopp");
        omsi_pack::create_pack(
            &pack,
            omsi_pack::PackMetadata {
                publisher_id: "fixture".into(), author: "Fixture".into(), pack_id: "map".into(),
                version: "1.0.0".into(), license: "CC0-1.0".into(), redistribution: false,
                key_id: None, dependencies: Vec::new(),
            },
            &[omsi_pack::AssetInput {
                id: "global".into(), path: "maps/Protected/global.cfg".into(),
                source_sha256: "12".repeat(32), bytes: b"[name]\r\nProtected\r\n[friendlyname]\r\nProtected Map\r\n".to_vec(),
            }],
            &signer, None, &std::sync::atomic::AtomicBool::new(false),
        ).unwrap();
        omsi_cfg::add_content_root(content.clone());
        let mounts = omsi_cfg::publisher::mount_packs(&content).unwrap();
        let found = list_maps(&Roots { omsi, content: content.clone() });
        assert!(found.iter().any(|m| m.name == "Protected Map" && m.in_content && omsi_cfg::vfs::is_file(&m.cfg)));
        for mount in mounts { omsi_cfg::vfs::unmount_sealed(&mount); omsi_cfg::remove_content_root(&mount); }
        omsi_cfg::remove_content_root(&content);
        std::fs::remove_dir_all(root).unwrap();
    }
}
