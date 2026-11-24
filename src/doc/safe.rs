//! Saving without losing anything: every file goes to a temporary name first and is renamed
//! over the old one only when all of them were written. Failed installations roll back;
//! regular backups keep the files replaced or removed so a save can also be taken back
//! after a process crash. This is not a crash-atomic multi-file transaction.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use crate::icons::{self, Icon};
use crate::theme::pal;

/// What a save does to one file.
pub enum Op {
    Write(PathBuf, Vec<u8>),
    Remove(PathBuf),
}

impl Op {
    pub fn path(&self) -> &Path {
        match self {
            Op::Write(p, _) | Op::Remove(p) => p,
        }
    }
}

/// How many saves are kept per map.
pub const KEEP: usize = 10;

/// The name a file is written under before it takes the place of the old one.
fn temp_of(p: &Path) -> PathBuf {
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    p.with_file_name(format!(".{name}.openmapper-new"))
}

fn write_synced(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(p)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Write one file so that it is either all old or all new.
pub fn write_atomic(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = temp_of(p);
    if let Err(e) = write_synced(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, p)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Prepare every file, then install them. A filesystem error rolls back the files
/// already replaced or removed; backups remain the recovery path after a process crash.
pub fn commit(ops: &[Op]) -> anyhow::Result<()> {
    super::transaction::commit(ops)
}

/// `rel` (backslashes or slashes) inside `dir` itself, whatever the case of its parts; parts
/// not there are taken as written. Unlike `omsi_cfg::resolve_path` it never leads into
/// another content root: what is written through it lands in `dir` (a map's own copy in
/// the content folder, never the installation's).
pub fn own_path(dir: &Path, rel: &str) -> PathBuf {
    let mut cur = dir.to_path_buf();
    for comp in rel.split(['\\', '/']).filter(|c| !c.is_empty() && *c != ".") {
        let exact = cur.join(comp);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(comp)).unwrap_or(false)));
        cur = found.unwrap_or(exact);
    }
    cur
}

/// A save's time as a folder name (UTC, sorts by time).
pub fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let day = super::civil(secs as i64 / 86_400);
    let t = secs % 86_400;
    format!("{day}-{:02}{:02}{:02}", t / 3600, t / 60 % 60, t % 60)
}

/// Where a map's backups are kept: beside the content folder's maps, never in the map
/// itself (the game would read it).
pub fn backups_of(map_dir: &Path) -> PathBuf {
    let folder = map_dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let content = map_dir.parent().and_then(Path::parent).unwrap_or(map_dir);
    content.join("openMapper").join("backups").join(folder)
}

/// The files a save lists in its backup that it made new (restoring takes them away).
const CREATED: &str = "created-by-the-save.txt";

/// Keep what the operations would replace or remove, and `extra` (folders or files of the
/// map, relative to it, kept whole), in a new backup. None: there was nothing to keep.
pub fn backup(map_dir: &Path, ops: &[Op], extra: &[&str]) -> anyhow::Result<Option<PathBuf>> {
    let root = backups_of(map_dir);
    let mut dir = root.join(stamp());
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{}-{n}", stamp()));
        n += 1;
    }
    let mut created = String::new();
    let mut kept = 0;
    for op in ops {
        let p = op.path();
        let Ok(rel) = p.strip_prefix(map_dir) else { continue };
        if p.is_file() {
            let to = dir.join(rel);
            std::fs::create_dir_all(to.parent().unwrap_or(&dir))?;
            std::fs::copy(p, &to)?;
            kept += 1;
        } else if matches!(op, Op::Write(..)) {
            created.push_str(&rel.to_string_lossy());
            created.push('\n');
        }
    }
    for e in extra {
        let from = map_dir.join(e);
        if from.exists() {
            copy_tree(&from, &dir.join(e))?;
            kept += 1;
        }
    }
    if kept == 0 {
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(None);
    }
    std::fs::write(dir.join(CREATED), created)?;
    prune(&root, KEEP);
    Ok(Some(dir))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_file() {
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(from, to)?;
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        copy_tree(&e.path(), &to.join(e.file_name()))?;
    }
    Ok(())
}

/// A map's backups, newest first.
pub fn backups(map_dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(backups_of(map_dir)).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.join(CREATED).is_file()).collect()).unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// Keep only the newest `keep`.
fn prune(root: &Path, keep: usize) {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    v.sort();
    while v.len() > keep {
        let old = v.remove(0);
        if let Err(e) = std::fs::remove_dir_all(&old) {
            log::warn!("{}: {e}", old.display());
        }
    }
}

/// Put the map back as it was before the save that made `to` (and every later save): the
/// newest backups are laid back one after another, then they go.
pub fn restore(map_dir: &Path, to: &Path) -> anyhow::Result<usize> {
    let all = backups(map_dir);
    let Some(upto) = all.iter().position(|b| b == to) else { anyhow::bail!("{}: not a backup of {}", to.display(), map_dir.display()) };
    let mut n = 0;
    for b in &all[..=upto] {
        // the files the save made new go; the kept ones come back
        let created = std::fs::read_to_string(b.join(CREATED)).unwrap_or_default();
        let mut ops: Vec<Op> = created.lines().filter(|l| !l.trim().is_empty()).map(|l| Op::Remove(map_dir.join(l))).collect();
        collect(b, b, map_dir, &mut ops)?;
        commit(&ops)?;
        std::fs::remove_dir_all(b)?;
        n += 1;
    }
    Ok(n)
}

fn collect(root: &Path, dir: &Path, map_dir: &Path, ops: &mut Vec<Op>) -> anyhow::Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(root, &p, map_dir, ops)?;
        } else if p.file_name().map(|f| f != CREATED).unwrap_or(false) {
            let rel = p.strip_prefix(root)?;
            ops.push(Op::Write(map_dir.join(rel), std::fs::read(&p)?));
        }
    }
    Ok(())
}

/// The window of a map's earlier versions: each save's backup, newest first, and going
/// back to the map as it was before it.
pub fn versions_window(ctx: &egui::Context, open: &mut bool, map_dir: &Path, actions: &mut Vec<crate::ui::Action>) {
    use crate::i18n::tr;
    if !*open {
        return;
    }
    let list = backups(map_dir);
    egui::Window::new(icons::labelled(Icon::Backup, tr("Earlier versions", "Прошлые версии"))).open(open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new(tr("Before every save, the files it replaces are kept (the last 10 saves). Going back puts the map as it was before that save; later changes go.", "Перед каждым сохранением файлы, которые оно заменяет, сохраняются (последние 10 сохранений). Возврат вернёт карту к виду до того сохранения; более поздние изменения пропадут.")).small().color(pal().text_dim));
        ui.add_space(6.0);
        if list.is_empty() {
            ui.label(tr("No earlier versions yet: they appear once the map is saved again.", "Прошлых версий пока нет: они появятся при следующем сохранении."));
        }
        for b in &list {
            let name = b.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(format!("{} {}", tr("before the save of", "до сохранения"), pretty(&name)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(icons::labelled(Icon::Backup, tr("Go back to this", "Вернуться к этой"))).clicked() {
                        actions.push(crate::ui::Action::RestoreBackup(b.clone()));
                    }
                });
            });
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(backups_of(map_dir).display().to_string()).small().color(pal().text_dim));
    });
}

/// A backup's folder name (20261010-174512) as a date and time (UTC).
fn pretty(stamp: &str) -> String {
    let s: Vec<char> = stamp.chars().collect();
    if s.len() < 15 {
        return stamp.to_string();
    }
    let g = |a: usize, b: usize| s[a..b].iter().collect::<String>();
    format!("{}-{}-{} {}:{}:{} UTC", g(0, 4), g(4, 6), g(6, 8), g(9, 11), g(11, 13), g(13, 15))
}

#[cfg(test)]
mod tests {
    /// A file found whatever its case, one not there taken as written, inside the folder.
    #[test]
    fn own_paths_stay_in_their_folder() {
        let d = std::env::temp_dir().join(format!("openmapper-ownpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Chrono").join("0100_A")).unwrap();
        // (the file system's own case rules may keep the spelling asked for)
        let same = |a: PathBuf, b: PathBuf| assert!(a.display().to_string().eq_ignore_ascii_case(&b.display().to_string()), "{} {}", a.display(), b.display());
        same(own_path(&d, "chrono\\0100_a"), d.join("Chrono").join("0100_A"));
        same(own_path(&d, "chrono/0100_a/tile_1_1.map"), d.join("Chrono").join("0100_A").join("tile_1_1.map"));
        assert_eq!(own_path(&d, "Holidays.txt"), d.join("Holidays.txt"));
        let _ = std::fs::remove_dir_all(&d);
    }

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("openmapper-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("maps").join("M")).unwrap();
        d
    }

    #[test]
    fn a_save_is_kept_and_can_be_taken_back() {
        let root = scratch("backup");
        let map = root.join("maps").join("M");
        std::fs::write(map.join("a.map"), "old a").unwrap();
        std::fs::write(map.join("gone.map"), "old gone").unwrap();
        let ops = vec![Op::Write(map.join("a.map"), b"new a".to_vec()), Op::Write(map.join("b.map"), b"new b".to_vec()), Op::Remove(map.join("gone.map"))];
        let b = backup(&map, &ops, &[]).unwrap().unwrap();
        commit(&ops).unwrap();
        assert_eq!(std::fs::read_to_string(map.join("a.map")).unwrap(), "new a");
        assert!(!map.join("gone.map").exists());
        // (no temporary files left)
        assert!(std::fs::read_dir(&map).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains("openmapper-new")));
        assert_eq!(backups(&map), vec![b.clone()]);
        assert!(b.starts_with(root.join("openMapper").join("backups")));
        restore(&map, &b).unwrap();
        assert_eq!(std::fs::read_to_string(map.join("a.map")).unwrap(), "old a");
        assert_eq!(std::fs::read_to_string(map.join("gone.map")).unwrap(), "old gone");
        assert!(!map.join("b.map").exists());
        assert!(backups(&map).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let root = scratch("fail");
        let map = root.join("maps").join("M");
        std::fs::write(map.join("a.map"), "old").unwrap();
        // (a folder where a file should go: it cannot be written)
        std::fs::create_dir_all(map.join("z.map")).unwrap();
        let ops = vec![Op::Write(map.join("a.map"), b"new".to_vec()), Op::Write(map.join("z.map"), b"z".to_vec())];
        assert!(commit(&ops).is_err());
        assert_eq!(std::fs::read_to_string(map.join("a.map")).unwrap(), "old");
        assert!(!map.join(".a.map.openmapper-new").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_newest_backups_stay() {
        let root = scratch("prune");
        let map = root.join("maps").join("M");
        std::fs::write(map.join("a.map"), "a").unwrap();
        for _ in 0..KEEP + 3 {
            backup(&map, &[Op::Write(map.join("a.map"), Vec::new())], &[]).unwrap();
        }
        assert_eq!(backups(&map).len(), KEEP);
        let _ = std::fs::remove_dir_all(&root);
    }
}
