//! Export a map for sharing: one `.zip` laid out like OMSI 2 (`maps\<folder>\…` and every
//! object, road, texture, vehicle and person file the map uses that is not part of the
//! installation: openMapper's junctions, imported models with their textures and PBR maps,
//! mods' assets), with a `readme.txt` saying what it is and what it needs. Files of the
//! installation are never packed: the readme lists them as requirements. Dropped onto
//! openMapper (or unpacked into an OMSI 2 folder) it is the map again.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::doc::Document;
use crate::install::Roots;

/// What the zip holds and what it needs.
#[derive(Debug, Default)]
pub struct Plan {
    /// (file on disk, its path in the zip with `/`).
    pub files: BTreeMap<String, PathBuf>,
    /// The installation's folders the map uses (`Sceneryobjects\Generic`), with how many
    /// of their files.
    pub stock: BTreeMap<String, usize>,
    /// The content folder's folders packed (`Sceneryobjects\openMapper\Town`).
    pub packed: BTreeSet<String>,
    /// Files the map names that are nowhere.
    pub missing: BTreeSet<String>,
}

/// Where a referenced file is.
enum Found {
    /// In the content folder (packed): its path relative to it.
    Content(PathBuf, String),
    Stock(String),
    Missing,
}

/// A path under `root`, matched without regard to case, part by part (not through openOMSI's
/// overlay, which lays the content folder over the installation).
fn find_in(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    // (each part as the disk spells it: a disk that ignores case would give back ours)
    for part in rel.split(['\\', '/']).filter(|x| !x.is_empty() && *x != ".") {
        let found = std::fs::read_dir(&p).ok()?.flatten().map(|e| e.path()).find(|q| q.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(part)).unwrap_or(false));
        p = found?;
    }
    Some(p)
}

fn rel_of(p: &Path, root: &Path) -> Option<String> {
    p.strip_prefix(root).ok().map(|r| r.to_string_lossy().replace('/', "\\"))
}

/// The installation folder a stock file belongs to, for the requirements (two levels at
/// most: `Vehicles\VW_Golf_2`, `Texture`).
fn stock_folder(rel: &str) -> String {
    let parts: Vec<&str> = rel.split('\\').collect();
    parts[..(parts.len().saturating_sub(1)).clamp(1, 2)].join("\\")
}

struct Collector<'a> {
    roots: &'a Roots,
    plan: Plan,
    seen: BTreeSet<String>,
}

impl Collector<'_> {
    fn locate(&self, rel: &str) -> Found {
        if let Some(p) = find_in(&self.roots.content, rel).filter(|p| p.is_file()) {
            let r = rel_of(&p, &self.roots.content).unwrap_or_else(|| rel.to_string());
            return Found::Content(p, r);
        }
        if let Some(p) = find_in(&self.roots.omsi, rel).filter(|p| p.is_file()) {
            return Found::Stock(rel_of(&p, &self.roots.omsi).unwrap_or_else(|| rel.to_string()));
        }
        Found::Missing
    }

    /// Pack one file of the content folder (`rel`: its path in it).
    fn pack(&mut self, p: &Path, rel: &str) {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        // (half-written files of a save)
        if name.ends_with(".openmapper-new") {
            return;
        }
        self.plan.files.entry(rel.replace('\\', "/")).or_insert_with(|| p.to_path_buf());
    }

    /// Pack a content folder with everything in it.
    fn pack_dir(&mut self, dir: &Path) {
        let Some(rel) = rel_of(dir, &self.roots.content) else { return };
        if !self.plan.packed.insert(rel) {
            return;
        }
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Some(r) = rel_of(&p, &self.roots.content) {
                    self.pack(&p, &r);
                }
            }
        }
    }

    /// A file the map uses (relative to the game's folder); `folder`: its whole folder comes
    /// along (an object's or a vehicle's model, textures, scripts and sounds lie beside it).
    fn file(&mut self, rel: &str, folder: bool) {
        let rel = rel.trim();
        if rel.is_empty() || !self.seen.insert(rel.to_lowercase()) {
            return;
        }
        match self.locate(rel) {
            Found::Content(p, r) => {
                let parent = p.parent().map(Path::to_path_buf);
                // (never a category's whole folder: `Sceneryobjects`, `Texture`…)
                let deep = parent.as_ref().and_then(|d| rel_of(d, &self.roots.content)).map(|r| r.contains('\\')).unwrap_or(false);
                if folder && deep {
                    self.pack_dir(parent.as_ref().unwrap());
                } else {
                    self.pack(&p, &r);
                }
                self.dependencies(&p, rel);
            }
            Found::Stock(r) => *self.plan.stock.entry(stock_folder(&r)).or_default() += 1,
            Found::Missing => {
                self.plan.missing.insert(rel.to_string());
            }
        }
    }

    /// What a content object or road needs outside its own folder: its model, meshes and
    /// textures (with their PBR maps and settings beside them).
    fn dependencies(&mut self, path: &Path, rel: &str) {
        let lower = rel.to_lowercase();
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        let mut textures: Vec<(String, Vec<PathBuf>)> = Vec::new();
        if lower.ends_with(".sli") {
            if let Ok(def) = omsi_scenery::Spline::load(path) {
                let dirs = crate::assets::texture_dirs(&self.roots.omsi, &self.roots.content, &dir);
                textures.extend(def.textures.iter().map(|t| (t.file.clone(), dirs.clone())));
            }
        } else if lower.ends_with(".sco") {
            let Ok(sco) = omsi_scenery::SceneryObject::load(path) else { return };
            let (model, model_dir) = match &sco.model_file {
                Some(m) => {
                    let mp = omsi_cfg::resolve_path(&dir, m);
                    self.absolute(&mp);
                    match omsi_model::Model::load(&mp) {
                        Ok(model) => (model, mp.parent().map(Path::to_path_buf).unwrap_or_default()),
                        Err(_) => return,
                    }
                }
                None => (sco.model.clone(), dir.clone()),
            };
            // (the fonts its texts are drawn in, when a mod brought them)
            for tt in &model.text_textures {
                for f in crate::texts::font_files(&tt.font, &self.roots.omsi, &self.roots.content) {
                    self.absolute(&f);
                }
            }
            let mut dirs = crate::assets::texture_dirs(&self.roots.omsi, &self.roots.content, &model_dir);
            dirs.insert(0, omsi_cfg::resolve_path(&dir, "texture"));
            for md in &model.meshes {
                for p in [omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&model_dir, "model"), &md.file), omsi_cfg::resolve_path(&model_dir, &md.file)] {
                    self.absolute(&p);
                }
                textures.extend(md.materials.iter().map(|m| (m.texture.clone(), dirs.clone())));
            }
        }
        for (name, dirs) in textures {
            if name.trim().is_empty() {
                continue;
            }
            let refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
            if let Some(t) = omsi_texture::find_texture(&name, &refs) {
                self.texture(&t);
            }
        }
    }

    /// A file by its place on disk: packed when it is in the content folder.
    fn absolute(&mut self, p: &Path) {
        if let (true, Some(r)) = (p.is_file(), rel_of(p, &self.roots.content)) {
            self.pack(p, &r);
        }
    }

    /// A texture of the content folder and what goes with it: `name_nn.dds`, `name.bmp.cfg`…
    fn texture(&mut self, t: &Path) {
        let Some(dir) = t.parent() else { return };
        if rel_of(t, &self.roots.content).is_none() {
            return;
        }
        let stem = t.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let name = t.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let f = e.file_name().to_string_lossy().to_lowercase();
            if f == name || f.starts_with(&format!("{stem}_")) || f.starts_with(&format!("{name}.")) || f.starts_with(&format!("{stem}.")) {
                self.absolute(&e.path());
            }
        }
    }
}

/// What exporting the (saved) map packs.
pub fn plan(doc: &Document, roots: &Roots) -> Plan {
    let mut c = Collector { roots, plan: Plan::default(), seen: BTreeSet::new() };
    // the map's own folder, all of it
    c.pack_dir(&doc.dir);
    crate::geo::export_filter(&doc.dir, &mut c.plan.files);
    // its scripts for openOMSI, if it has them: the project, and its Rust plugin built
    if let Some(folder) = doc.dir.file_name() {
        let p = crate::script::project::Project::of(&roots.content, &folder.to_string_lossy());
        if p.dir.is_dir() {
            c.pack_dir(&p.dir);
        }
        let rust = p.rust_dir().with_extension("oop");
        if let Some(r) = rel_of(&rust, &roots.content).filter(|_| rust.is_file()) {
            c.pack(&rust, &r);
        }
    }
    for t in doc.tiles.values() {
        for o in t.tile.objects.iter().chain(t.tile.attach_objects.iter()) {
            c.file(&o.file, true);
        }
        for s in t.tile.splines.iter().filter(|s| !s.deleted) {
            c.file(&s.file, true);
        }
        for a in &t.tile.spline_attachments {
            c.file(&a.file, true);
        }
    }
    for g in &doc.global.ground_textures {
        c.file(&g.texture, false);
        c.file(&g.detail_texture, false);
    }
    let tr = &doc.traffic;
    for g in &tr.groups {
        for v in &g.vehicles {
            c.file(&v.file, true);
        }
        for tg in &g.typgroups {
            c.file(&tg.file, true);
        }
    }
    for f in tr.parked.iter().chain(&tr.humans).chain(&tr.drivers) {
        c.file(f, true);
    }
    c.plan
}

/// The readme of the zip.
pub fn readme(doc: &Document, plan: &Plan) -> String {
    let folder = doc.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let mut o = String::new();
    o.push_str(&format!("{}\n{}\n\n", doc.display_name(), "=".repeat(doc.display_name().chars().count().max(3))));
    let desc = doc.global.description.trim();
    if !desc.is_empty() {
        o.push_str(desc);
        o.push_str("\n\n");
    }
    let tt = &doc.timetable;
    o.push_str(&format!("Map folder: maps\\{folder}\n{} tiles, {} lines, {} trips.\n\n", doc.tiles.len(), tt.lines.len(), tt.trips.len()));
    o.push_str("Install\n-------\nDrop this .zip onto openMapper's window (or use its Import button), or unpack it into\nthe OMSI 2 folder (or openOMSI's content folder). Then choose the map in openOMSI.\n\n");
    o.push_str("Needs\n-----\nOMSI 2 with these folders of its own (not included):\n");
    for (f, n) in &plan.stock {
        o.push_str(&format!("  {f}  ({n} file{})\n", if *n == 1 { "" } else { "s" }));
    }
    if !plan.packed.is_empty() {
        o.push_str("\nIncluded besides the map (made for it, imported or from mods):\n");
        for f in plan.packed.iter().filter(|f| !f.to_lowercase().starts_with("maps\\")) {
            o.push_str(&format!("  {f}\n"));
        }
    }
    if plan.packed.iter().any(|f| f.to_lowercase().starts_with("plugins\\")) {
        o.push_str("\nThe map has a Lua script for openOMSI (plugins\\map_...). It is code, so openMapper's Import does not install it: read it, and copy that folder into openOMSI's plugins folder if you trust it.\n");
    }
    if !plan.missing.is_empty() {
        o.push_str("\nNamed by the map but not found when it was exported (they will be missing):\n");
        for f in &plan.missing {
            o.push_str(&format!("  {f}\n"));
        }
    }
    o.push_str(&format!("\nExported with openMapper {} ({} files).\n", env!("CARGO_PKG_VERSION"), plan.files.len()));
    o.replace('\n', "\r\n")
}

/// Write the zip: the planned files and the readme.
pub fn write_zip(plan: &Plan, readme: &str, out: &Path) -> anyhow::Result<u64> {
    let tmp = out.with_extension("zip.openmapper-new");
    let f = std::fs::File::create(&tmp)?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("readme.txt", opts)?;
    z.write_all(readme.as_bytes())?;
    for (rel, p) in &plan.files {
        z.start_file(rel.as_str(), opts)?;
        z.write_all(&std::fs::read(p)?)?;
    }
    z.finish()?;
    std::fs::rename(&tmp, out)?;
    Ok(std::fs::metadata(out).map(|m| m.len()).unwrap_or(0))
}

/// Put the planned files into an OMSI 2 folder (`maps\<folder>`, the objects, roads and
/// textures made for the map), the way unpacking the exported zip into it would. A file that is
/// already there and as new as ours is left alone. (copied, left alone)
pub fn install_into(plan: &Plan, omsi: &Path) -> anyhow::Result<(usize, usize)> {
    let (mut copied, mut same) = (0, 0);
    for (rel, src) in &plan.files {
        let dst = rel.split('/').fold(omsi.to_path_buf(), |p, part| p.join(part));
        let (sm, dm) = (std::fs::metadata(src)?, std::fs::metadata(&dst).ok());
        if let Some(dm) = dm {
            let newer = match (sm.modified(), dm.modified()) {
                (Ok(s), Ok(d)) => d >= s,
                _ => false,
            };
            if dm.len() == sm.len() && newer {
                same += 1;
                continue;
            }
        }
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?;
        }
        // (under a temporary name first: a program that is reading the file never sees half of it)
        let tmp = dst.with_extension("openmapper-new");
        std::fs::copy(src, &tmp).map_err(|e| anyhow::anyhow!("{}: {e}", dst.display()))?;
        std::fs::rename(&tmp, &dst).map_err(|e| anyhow::anyhow!("{}: {e}", dst.display()))?;
        copied += 1;
    }
    Ok((copied, same))
}

/// Export the saved map to `out`.
pub fn export(doc: &Document, roots: &Roots, out: &Path) -> anyhow::Result<(Plan, u64)> {
    let plan = plan(doc, roots);
    let bytes = write_zip(&plan, &readme(doc, &plan), out)?;
    log::info!("export: {} files ({:.1} MB) to {}; {} installation folders needed, {} missing", plan.files.len(), bytes as f64 / 1e6, out.display(), plan.stock.len(), plan.missing.len());
    Ok((plan, bytes))
}

/// `openmapper --export path/to/global.cfg out.zip`: export a map without a window.
pub fn cli_export(cfg: &Path, out: &Path) -> i32 {
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::load()) else {
        eprintln!("no OMSI 2 installation found");
        return 2;
    };
    let roots = Roots::new(omsi);
    let doc = match Document::load(&roots, cfg, |_| {}) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", cfg.display());
            return 2;
        }
    };
    if doc.source.is_some() {
        eprintln!("{}: save the map into the content folder first (open and save it in openMapper)", cfg.display());
        return 2;
    }
    match export(&doc, &roots, out) {
        Ok((plan, bytes)) => {
            println!("{}: {} files, {:.1} MB; needs {} installation folders; {} missing", out.display(), plan.files.len(), bytes as f64 / 1e6, plan.stock.len(), plan.missing.len());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// `openmapper --import file`: import a mod, a map's zip or a model into the content folder,
/// as dropping it on the window does.
pub fn cli_import(path: &Path) -> i32 {
    match crate::import::import(path, &crate::install::content_dir()) {
        Ok(r) => {
            println!("{}", r.message);
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests;
