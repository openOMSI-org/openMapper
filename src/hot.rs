//! Hot reload: a background thread watches the object, spline and texture folders of the
//! content folder (often) and of the installation (now and then) and says when files there
//! came, went or changed, so the catalogue and the models follow without a restart.
//!
//! Changes the editor makes itself (it writes objects, textures and splines too) are not
//! reported: [`own_change`] marks them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Folders below a root that hold what the editor shows.
const WATCHED: [&str; 3] = ["Sceneryobjects", "Splines", "Texture"];

/// How long the files must stay as they are before a change is reported (a copy of a big
/// folder is one change, not hundreds).
const SETTLE: Duration = Duration::from_millis(900);

/// How long after an own write a change may still be probed and taken as that write's. The
/// content folder is looked at every 1.5 s, so a write there is probed at most that much
/// later; two settle periods cover the probe and a slow filesystem clock.
const OWN_GRACE: Duration = Duration::from_millis(1800);

/// The installation is looked at every fourth tick (6 s), so its own writes are probed much
/// later; the ordering test still decides, and this only keeps an install-root save from
/// being mistaken for somebody else's change.
const INSTALL_GRACE: Duration = Duration::from_millis(7000);

static OWN: Mutex<Option<Instant>> = Mutex::new(None);

/// The editor wrote content files just now: the watcher takes them as already known. Also
/// tells openOMSI's file cache.
pub fn own_change() {
    omsi_cfg::content_changed();
    *OWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

fn own_at() -> Option<Instant> {
    *OWN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a change probed at `probe`, the last clean snapshot being `last_ok`, is the
/// editor's own write. The write has to be newer than that snapshot (so it happened after
/// the last time the folder was known to be as the editor left it), and it has to be the
/// only thing that changed: an external write landing after ours is probed later than the
/// grace, and is reported. The settle loop's own length is not part of the test, so a large
/// file written slowly can no longer outlast it.
fn is_own_write(own: Option<Instant>, last_ok: Instant, probe: Instant, grace: Duration) -> bool {
    own.is_some_and(|t| t > last_ok && probe <= t + grace)
}

pub struct Watcher {
    rx: Receiver<usize>,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    /// Watch `content` (every 1.5 s) and `omsi` (every 6 s).
    pub fn start(content: PathBuf, omsi: PathBuf) -> Watcher {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let spawned = std::thread::Builder::new().name("hot-reload".into()).spawn(move || {
            let mut prints = [fingerprint(&content), fingerprint(&omsi)];
            // (when each root was last seen unchanged: a write made after this is one the
            // editor did itself)
            let mut last_ok = [Instant::now(), Instant::now()];
            let mut tick = 0u32;
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1500));
                tick = tick.wrapping_add(1);
                let roots: [(&Path, bool); 2] = [(&content, true), (&omsi, tick % 4 == 0)];
                let mut changed = false;
                for (i, (root, due)) in roots.into_iter().enumerate() {
                    if !due {
                        continue;
                    }
                    let now = fingerprint(root);
                    if now == prints[i] {
                        last_ok[i] = Instant::now();
                        continue;
                    }
                    // (when the change was first seen: the settle loop below can take a while,
                    // and how long it takes must not decide whether this was our own write)
                    let probe = Instant::now();
                    // (wait until it has stopped changing)
                    let mut last = now;
                    loop {
                        std::thread::sleep(SETTLE);
                        let again = fingerprint(root);
                        if again == last || flag.load(Ordering::Relaxed) {
                            last = again;
                            break;
                        }
                        last = again;
                    }
                    prints[i] = last;
                    // (the settled snapshot is a clean one from now on: a change after this is
                    // somebody else's)
                    let own = own_at();
                    let grace = if i == 0 { OWN_GRACE } else { INSTALL_GRACE };
                    let mine = is_own_write(own, last_ok[i], probe, grace);
                    last_ok[i] = Instant::now();
                    if mine {
                        log::debug!("hot reload: {} changed by the editor itself", root.display());
                    } else {
                        changed = true;
                    }
                }
                if changed && tx.send(tick as usize).is_err() {
                    break;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("hot reload: could not start the watcher: {e}");
        }
        Watcher { rx, stop }
    }

    /// Whether files changed since the last call.
    pub fn poll(&self) -> bool {
        let mut any = false;
        while self.rx.try_recv().is_ok() {
            any = true;
        }
        any
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A number that changes when any file below the watched folders of `root` is added,
/// removed, resized or written to.
fn fingerprint(root: &Path) -> u64 {
    let mut h = 0u64;
    for name in WATCHED {
        let Some(dir) = child(root, name) else { continue };
        visit(&dir, &mut h, 0);
    }
    h
}

fn child(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

fn visit(dir: &Path, acc: &mut u64, depth: u32) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let p = e.path();
        if meta.is_dir() {
            visit(&p, acc, depth + 1);
            continue;
        }
        // (files being written have a temporary name of ours or a download's; the editor's own
        // backups and the crossing writer's trial file are bookkeeping, not content)
        let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "part" | "tmp" | "crdownload" | "bak") || name.ends_with(".crossing-check.sco") {
            continue;
        }
        let mut one = DefaultHasher::new();
        p.hash(&mut one);
        meta.len().hash(&mut one);
        meta.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis()).hash(&mut one);
        // (a sum: the order the folder lists in does not matter)
        *acc = acc.wrapping_add(one.finish());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_changes_the_fingerprint() {
        let dir = std::env::temp_dir().join(format!("openmapper-hot-{}", std::process::id()));
        let objs = dir.join("sceneryobjects").join("My");
        std::fs::create_dir_all(&objs).unwrap();
        let before = fingerprint(&dir);
        assert_eq!(before, fingerprint(&dir));
        std::fs::write(objs.join("house.sco"), "x").unwrap();
        let added = fingerprint(&dir);
        assert_ne!(before, added);
        std::fs::write(objs.join("house.sco"), "xy").unwrap();
        assert_ne!(added, fingerprint(&dir));
        std::fs::write(objs.join("house.part"), "partial").unwrap();
        assert_eq!(fingerprint(&dir), fingerprint(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The editor's own `.bak` and the crossing writer's trial file must not keep the watcher
    /// busy: they are bookkeeping beside the content, not content.
    #[test]
    fn a_backup_and_a_crossing_trial_do_not_change_the_fingerprint() {
        let dir = std::env::temp_dir().join(format!("openmapper-hot-bak-{}", std::process::id()));
        let objs = dir.join("sceneryobjects").join("My");
        std::fs::create_dir_all(&objs).unwrap();
        let sco = objs.join("Kreuzung.sco");
        std::fs::write(&sco, "object").unwrap();
        let before = fingerprint(&dir);
        std::fs::write(objs.join("Kreuzung.sco.bak"), "old").unwrap();
        assert_eq!(fingerprint(&dir), before);
        std::fs::write(objs.join("Kreuzung.crossing-check.sco"), "trial").unwrap();
        assert_eq!(fingerprint(&dir), before);
        // (the real file still counts)
        std::fs::write(&sco, "object, changed").unwrap();
        assert_ne!(fingerprint(&dir), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The own-write test is one of ordering, not of how long the settle loop took: a probe
    /// of a write made after the last clean snapshot is ours even when settling ran long,
    /// while a probe later than the grace (something else changed too) is reported.
    #[test]
    fn an_own_write_is_judged_by_its_order() {
        let last_ok = Instant::now();
        let own = last_ok + Duration::from_millis(100);
        // (probed by the next poll, well before any settle loop)
        assert!(is_own_write(Some(own), last_ok, own + Duration::from_millis(1500), OWN_GRACE));
        // (an external write after ours is probed past the grace)
        assert!(!is_own_write(Some(own), last_ok, own + OWN_GRACE + Duration::from_millis(1), OWN_GRACE));
        // (a write from before the clean snapshot is not ours)
        assert!(!is_own_write(Some(last_ok - Duration::from_millis(10)), last_ok, last_ok, OWN_GRACE));
        assert!(!is_own_write(None, last_ok, last_ok, OWN_GRACE));
        // (the installation is polled every 6 s: its grace covers that, and no more)
        let inst = Duration::from_millis(6000);
        assert!(is_own_write(Some(own), last_ok, own + inst, INSTALL_GRACE));
        assert!(!is_own_write(Some(own), last_ok, own + INSTALL_GRACE + Duration::from_millis(1), INSTALL_GRACE));
    }
}
