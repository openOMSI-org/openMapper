//! Writing the map as OMSI 2 does: UTF-16 text with CR LF, tile files of version 14, the
//! `.terrain` height fields and the painting masks. A map read from the installation is
//! first copied whole into the content folder, so the original is never written to.

use std::fmt::Write as _;
use std::path::Path;

use omsi_map::{MapObject, MapRule, MapSpline, SplineAttachment, Tile};

use super::{mask_size, Document, TileKey};

/// Text the way OMSI's editor writes it: UTF-16 little-endian with a byte order mark and
/// CR LF line ends.
pub(crate) fn utf16(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    for u in crlf.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// A number as OMSI reads it (a point, no exponent for ordinary values).
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

pub(crate) fn header() -> String {
    format!("File created with openMapper {}\n\n", env!("CARGO_PKG_VERSION"))
}

/// Save the map into its folder. Nothing is written unless all of it can be: the files
/// go under temporary names first, and what they replace is kept in a backup of this save
/// (see `safe`).
pub fn save(doc: &mut Document) -> anyhow::Result<()> {
    if doc._publisher_protected_source {
        anyhow::bail!("publisher-protected maps cannot be saved as plaintext");
    }
    std::fs::create_dir_all(&doc.dir)?;
    // a map of the installation (or the template) is copied over first: its timetables,
    // AI lists, light maps and chrono folders come along
    if let Some(src) = doc.source.clone() {
        if src != doc.dir {
            copy_missing(&src, &doc.dir)?;
            // the template's own tile is not one of ours
            if doc.fresh {
                for f in ["tile_0_0.map", "tile_0_0.map.terrain", "tile_0_0.map.LM.bmp"] {
                    if !doc.tiles.contains_key(&(0, 0)) {
                        let _ = std::fs::remove_file(doc.dir.join(f));
                    }
                }
            }
        }
        // (only once it is all there: a copy cut short is finished on the next save)
        doc.source = None;
    }
    doc.fresh = false;
    doc.sync_mask_slots();
    let (mut ops, written) = plan(doc, &doc.dir, false);
    // the timetable's and the traffic's files go with the map's, all or nothing
    let (dir, order) = (doc.dir.clone(), doc.order.clone());
    crate::timetable::follow_stop_tiles(doc);
    ops.extend(crate::timetable::plan(&mut doc.timetable, &dir, &order));
    ops.extend(crate::traffic::plan(&doc.traffic, &dir));
    ops.extend(doc.descriptions.plan(&dir));
    // (the depot files taken away, as they were: their copies for openOMSI go too)
    let gone: Vec<(String, Vec<u8>)> = ops
        .iter()
        .filter_map(|o| match o {
            super::safe::Op::Remove(p) if p.extension().map(|x| x.eq_ignore_ascii_case("hof")).unwrap_or(false) => Some((p.file_name()?.to_string_lossy().into_owned(), std::fs::read(p).ok()?)),
            _ => None,
        })
        .collect();
    let kept = super::safe::backup(&doc.dir, &ops, &[]).map_err(|e| anyhow::anyhow!("{}: {e}", crate::i18n::tr("could not keep a backup, nothing was saved", "не удалось сделать резервную копию, ничего не сохранено")))?;
    super::safe::commit(&ops)?;
    if let Some(b) = kept {
        log::info!("the files this save replaced are kept in {}", b.display());
    }
    for k in written {
        if let Some(t) = doc.tiles.get_mut(&k) {
            t.tile.version = 14;
            t.tile.has_terrain = true;
            t.dirty = false;
        }
    }
    doc.global_dirty = false;
    if doc.traffic.depots_changed() {
        crate::traffic::share_depots(&doc.traffic, &doc.dir, &gone);
    }
    crate::timetable::saved(&mut doc.timetable);
    crate::traffic::saved(&mut doc.traffic);
    doc.descriptions.saved();
    Ok(())
}

/// What saving the map into `dir` writes and removes, and the tiles that it writes. The
/// changed tiles only (`all`: every tile); an autosave keeps the files it would remove.
pub fn plan(doc: &Document, dir: &Path, all: bool) -> (Vec<super::safe::Op>, Vec<TileKey>) {
    plan_known(doc, dir, all, &|k| doc.tiles.contains_key(k))
}

/// `plan` for a document that holds only the tiles to write (an autosave's snapshot): the
/// tile list of global.cfg is told which tiles the map has by `has_tile`.
pub(crate) fn plan_known(doc: &Document, dir: &Path, all: bool, has_tile: &dyn Fn(&TileKey) -> bool) -> (Vec<super::safe::Op>, Vec<TileKey>) {
    use super::safe::Op;
    let mut ops = Vec::new();
    let mut written = Vec::new();
    let map_tex = dir.join("texture").join("map");
    for (k, t) in doc.tiles.iter() {
        if !t.dirty && !all {
            continue;
        }
        let file = file_of(doc, *k);
        let mut own = t.tile.clone();
        own.version = 14;
        own.has_terrain = true;
        own.has_water = t.water.is_some();
        super::unfit_from_world_grid(&mut own, k.1);
        ops.push(Op::Write(dir.join(&file), utf16(&tile_text(&own))));
        ops.push(Op::Write(dir.join(format!("{file}.terrain")), t.terrain.to_bytes()));
        match t.water {
            Some(w) => {
                let mut b = 1u32.to_le_bytes().to_vec();
                w.iter().for_each(|h| b.extend_from_slice(&h.to_le_bytes()));
                ops.push(Op::Write(dir.join(format!("{file}.water")), b));
            }
            None => ops.push(Op::Remove(dir.join(format!("{file}.water")))),
        }
        // a light map of the old ground would put the old shadows on the new one
        ops.push(Op::Remove(dir.join(format!("{file}.LM.bmp"))));
        ops.push(Op::Remove(dir.join(format!("{file}.prt"))));
        for (li, g) in doc.global.ground_textures.iter().enumerate().skip(1) {
            let p = map_tex.join(format!("{file}.{li}.dds"));
            match t.masks.get(li).and_then(|m| m.as_ref()) {
                Some(m) if m.iter().any(|&a| a > 0) => ops.push(Op::Write(p, super::dds::alpha8(mask_size(g), m))),
                _ => ops.push(Op::Remove(p)),
            }
        }
        written.push(*k);
    }
    // (last: the tile list that names the tiles)
    ops.push(Op::Write(dir.join("global.cfg"), utf16(&global_text_known(doc, has_tile))));
    // (only what is there to remove)
    ops.retain(|o| !matches!(o, Op::Remove(p) if !p.exists()));
    (ops, written)
}

pub fn tile_file(k: TileKey) -> String {
    format!("tile_{}_{}.map", k.0, k.1)
}

/// The file a tile is kept in: the one global.cfg named when the map was read (a tile not
/// edited is not written again, and chrono folders name it so), else `tile_x_y.map`.
fn file_of(doc: &Document, k: TileKey) -> String {
    doc.global.tiles.iter().find(|t| (t.x, t.y) == k).map(|t| t.file.clone()).unwrap_or_else(|| tile_file(k))
}

/// Copy a folder's files that are not at the destination yet.
pub(crate) fn copy_missing(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            // (`Backup` folders of OMSI's editor are not needed)
            if e.file_name().to_string_lossy().eq_ignore_ascii_case("backup") {
                continue;
            }
            copy_missing(&from, &to)?;
        } else if !to.exists() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub(crate) fn global_text(doc: &Document) -> String {
    global_text_known(doc, &|k| doc.tiles.contains_key(k))
}

pub(crate) fn global_text_known(doc: &Document, has_tile: &dyn Fn(&TileKey) -> bool) -> String {
    let g = &doc.global;
    let mut s = header();
    let kw = |s: &mut String, k: &str, lines: &[String]| {
        let _ = writeln!(s, "[{k}]");
        for l in lines {
            let _ = writeln!(s, "{l}");
        }
        s.push('\n');
    };
    kw(&mut s, "name", &[g.name.clone()]);
    kw(&mut s, "friendlyname", &[g.friendly_name.clone()]);
    let mut d: Vec<String> = g.description.lines().map(str::to_string).collect();
    d.push("[end]".into());
    kw(&mut s, "description", &d);
    kw(&mut s, "version", &["14".into()]);
    kw(&mut s, "NextIDCode", &[g.next_id_code.to_string()]);
    if g.world_coordinates {
        kw(&mut s, "worldcoordinates", &[]);
    }
    if g.dyn_helper_active {
        kw(&mut s, "dynhelperactive", &[]);
    }
    if g.real_rail {
        kw(&mut s, "realrail", &[]);
    }
    if g.left_hand_traffic {
        kw(&mut s, "LHT", &[]);
    }
    if g.background_image.len() == 5 {
        // (OMSI's editor reads six lines here, openOMSI's reader keeps five: without the sixth
        // the original editor takes the next keyword for it and cannot open the map)
        let mut lines = g.background_image.clone();
        lines.push("0".into());
        kw(&mut s, "backgroundimage", &lines);
    }
    if g.map_cam.len() == 8 {
        kw(&mut s, "mapcam", &g.map_cam.iter().map(|v| num(*v)).collect::<Vec<_>>());
    }
    if !g.money_system.is_empty() {
        kw(&mut s, "moneysystem", &[g.money_system.clone()]);
    }
    if !g.ticket_pack.is_empty() {
        kw(&mut s, "ticketpack", &[g.ticket_pack.clone()]);
    }
    kw(&mut s, "repair_time_min", &[num(g.repair_time_min as f64)]);
    if g.years != (0, 0) {
        kw(&mut s, "years", &[g.years.0.to_string(), g.years.1.to_string()]);
    }
    if g.real_year_offset != 0 {
        kw(&mut s, "realyearoffset", &[g.real_year_offset.to_string()]);
    }
    if !g.standard_depot.is_empty() {
        kw(&mut s, "standarddepot", &[g.standard_depot.clone()]);
    }
    for t in &g.ground_textures {
        kw(&mut s, "groundtex", &[t.texture.clone(), t.detail_texture.clone(), num(t.params[0] as f64), num(t.params[1] as f64), num(t.params[2] as f64)]);
    }
    for se in &g.seasons {
        kw(&mut s, "addseason", &[se.kind.to_string(), se.start_day.to_string(), se.end_day.to_string()]);
    }
    for (h, d) in &g.traffic_density_road {
        kw(&mut s, "trafficdensity_road", &[num(*h as f64), num(*d as f64)]);
    }
    for (h, d) in &g.traffic_density_passenger {
        kw(&mut s, "trafficdensity_passenger", &[num(*h as f64), num(*d as f64)]);
    }
    if !g.entry_points.is_empty() {
        let mut lines = vec![g.entry_points.len().to_string()];
        for e in &g.entry_points {
            lines.push(e.index.to_string());
            lines.push(e.object_id.to_string());
            lines.push(e.unknown.to_string());
            // x, height, y
            lines.push(num(e.pos[0]));
            lines.push(num(e.pos[2]));
            lines.push(num(e.pos[1]));
            for q in e.quat {
                lines.push(num(q));
            }
            lines.push(e.group.to_string());
            lines.push(e.name.clone());
        }
        kw(&mut s, "entrypoints", &lines);
    }
    if !g.spline_obj_types.is_empty() {
        kw(&mut s, "splineObjTypes", &g.spline_obj_types);
    }
    if !g.scen_obj_list.is_empty() {
        kw(&mut s, "scenObjList", &g.scen_obj_list);
    }
    // every entry of the list stays, also those of tiles that have no file (maps ship such,
    // Berlin-Spandau 40): start points, repeaters and timetables name tiles by their place
    // in it, and a [worldcoordinates] map's grid is measured over all its rows
    for k in &doc.order {
        if has_tile(k) {
            kw(&mut s, "map", &[k.0.to_string(), k.1.to_string(), file_of(doc, *k)]);
        } else if let Some(t) = g.tiles.iter().find(|t| (t.x, t.y) == *k) {
            kw(&mut s, "map", &[k.0.to_string(), k.1.to_string(), t.file.clone()]);
        }
    }
    s
}

pub(crate) fn tile_text(t: &Tile) -> String {
    let mut s = header();
    // ([terrain] and [water] carry a line of their own, empty, then the blank line that
    // ends the section, as OMSI's editor writes them and expects them)
    s.push_str("[version]\n14\n\n[terrain]\n\n\n");
    if t.has_water {
        s.push_str("[water]\n\n\n");
    }
    if t.variable_terrain_lightmap {
        s.push_str("[variable_terrainlightmap]\n\n");
    }
    if t.variable_terrain {
        s.push_str("[variable_terrain]\n\n");
    }
    for sp in t.splines.iter().filter(|s| !s.deleted) {
        spline(&mut s, sp);
    }
    // (every object of the tile is numbered by its place, as OMSI's editor numbers them)
    let mut n = 0;
    let mut numbered = |s: &mut String| {
        let _ = writeln!(s, "Object Nr. {n}");
        n += 1;
    };
    for o in &t.objects {
        numbered(&mut s);
        object(&mut s, o, false);
    }
    for a in &t.spline_attachments {
        numbered(&mut s);
        attachment(&mut s, a);
    }
    for o in &t.attach_objects {
        numbered(&mut s);
        object(&mut s, o, true);
    }
    s
}

fn labels(s: &mut String, count: i32, extra: &[String]) {
    let n = (count.max(0) as usize).max(extra.len());
    let _ = writeln!(s, "{n}");
    for i in 0..n {
        let _ = writeln!(s, "{}", extra.get(i).map(String::as_str).unwrap_or(""));
    }
}

pub(crate) fn rules(s: &mut String, rules: &[MapRule]) {
    for r in rules {
        let _ = write!(
            s,
            "[{}]\n{}\n{}\n{}\n{}\n\n",
            if r.kill { "kill_rule" } else { "rule" },
            r.path_index,
            r.kind,
            num(r.value),
            num(r.extra)
        );
    }
}

fn var_parent(s: &mut String, v: Option<i64>) {
    if let Some(id) = v {
        let _ = write!(s, "[varparent]\n{id}\n\n");
    }
}

pub(crate) fn object(s: &mut String, o: &MapObject, attach: bool) {
    if attach {
        let _ = write!(s, "[attachObj]\n0\n{}\n{}\n{}\n0\n{}\n", o.file, o.id, o.parent_id.unwrap_or(0), o.attach_index);
    } else {
        let _ = write!(s, "[object]\n0\n{}\n{}\n{}\n{}\n{}\n", o.file, o.id, num(o.pos[0]), num(o.pos[1]), num(o.pos[2]));
    }
    let _ = write!(s, "{}\n{}\n{}\n", num(o.rot[0]), num(o.rot[1]), num(o.rot[2]));
    labels(s, o.flag, &o.extra);
    s.push('\n');
    var_parent(s, o.var_parent);
    rules(s, &o.rules);
}

pub(crate) fn spline(s: &mut String, sp: &MapSpline) {
    let _ = write!(
        s,
        "[{}]\n0\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        if sp.is_h { "spline_h" } else { "spline" },
        sp.file,
        sp.id,
        sp.prev_id,
        sp.next_id,
        num(sp.pos[0]),
        num(sp.pos[2]),
        num(sp.pos[1]),
        num(sp.heading),
        num(sp.length),
        num(sp.radius),
        num(sp.grad_start),
        num(sp.grad_end),
    );
    if sp.is_h {
        let _ = writeln!(s, "{}", num(sp.delta_h.unwrap_or(0.0)));
    }
    let _ = write!(s, "{}\n{}\n{}\n{}\n{}\n", num(sp.cant_start), num(sp.cant_end), num(sp.skew_start), num(sp.skew_end), num(sp.tex_offset));
    // (a line for the mirror flag, empty when there is none, then the section's blank line)
    s.push_str(if sp.mirror { "mirror\n\n" } else { "\n\n" });
    if sp.terrain_align_flag {
        s.push_str("[spline_terrain_align]\n\n");
    }
    if let Some(v) = sp.terrain_align {
        let _ = write!(s, "[spline_terrain_align_2]\n{}\n\n", num(v));
    }
    rules(s, &sp.rules);
}

pub(crate) fn attachment(s: &mut String, a: &SplineAttachment) {
    match a.repeater {
        Some((tile, first)) => {
            let _ = write!(s, "[splineAttachement_repeater]\n0\n{tile}\n{first}\n");
        }
        None => s.push_str("[splineAttachement]\n0\n"),
    }
    let _ = write!(
        s,
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        a.file,
        a.id,
        a.spline_index,
        num(a.offset[0]),
        num(a.offset[1]),
        num(a.offset[2]),
        num(a.rot[0]),
        num(a.rot[1]),
        num(a.rot[2]),
        num(a.interval),
        num(a.range),
        if a.tilt { 1 } else { 0 },
    );
    labels(s, a.strings.len() as i32, &a.strings);
    s.push('\n');
    var_parent(s, a.var_parent);
    rules(s, &a.rules);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is written reads back through openOMSI's own reader unchanged.
    #[test]
    fn a_tile_reads_back() {
        let mut t = Tile { version: 14, has_terrain: true, ..Default::default() };
        t.splines.push(MapSpline {
            file: "Splines\\Marcel\\str_2spur_8m_borkumer1.sli".into(),
            id: 7,
            prev_id: 0,
            next_id: 8,
            pos: [10.5, 20.25, 3.0],
            heading: 45.0,
            length: 50.0,
            radius: -120.0,
            grad_start: 1.5,
            grad_end: 1.5,
            cant_start: 0.0,
            cant_end: 0.0,
            tex_offset: 0.0,
            map_chain_offset: Some(0.0),
            mirror: true,
            rules: vec![MapRule { path_index: 1, kind: "speedlimit".into(), value: 30.0, extra: 0.0, kill: false }],
            ..Default::default()
        });
        t.objects.push(MapObject {
            file: "Sceneryobjects\\Generic\\bus_stop.sco".into(),
            id: 9,
            pos: [100.0, 150.5, 0.0],
            rot: [90.0, 0.0, 0.0],
            flag: 2,
            extra: vec!["Hauptstr.".into(), "".into()],
            ..Default::default()
        });
        let text = tile_text(&t);
        let back = Tile::parse(&omsi_cfg::CfgFile::from_str("tile_0_0.map", &text));
        assert_eq!(back.splines.len(), 1);
        let s = &back.splines[0];
        assert_eq!((s.id, s.next_id, s.pos, s.heading, s.radius, s.mirror), (7, 8, [10.5, 20.25, 3.0], 45.0, -120.0, true));
        assert_eq!(s.rules.len(), 1);
        assert_eq!(back.objects.len(), 1);
        let o = &back.objects[0];
        assert_eq!((o.id, o.pos, o.rot[0]), (9, [100.0, 150.5, 0.0], 90.0));
        assert_eq!(o.extra, vec!["Hauptstr.".to_string()]);
    }

    /// `OMSI_ROUNDTRIP_IN=<folder of .map files> OMSI_ROUNDTRIP_OUT=<folder>`: every tile of the
    /// input is read and written again (to compare with what OMSI's own editor wrote).
    #[test]
    #[ignore]
    fn rewrite_tiles_for_comparison() {
        let (Some(i), Some(o)) = (std::env::var_os("OMSI_ROUNDTRIP_IN"), std::env::var_os("OMSI_ROUNDTRIP_OUT")) else { return };
        std::fs::create_dir_all(&o).unwrap();
        for e in std::fs::read_dir(i).unwrap().flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "map").unwrap_or(false) {
                let t = Tile::load(&p).unwrap();
                std::fs::write(Path::new(&o).join(p.file_name().unwrap()), utf16(&tile_text(&t))).unwrap();
            }
        }
    }

    /// `OMSI_ROUNDTRIP_CFG=<global.cfg> OMSI_ROUNDTRIP_OUT=<folder>`: a map's global.cfg read
    /// and written again.
    #[test]
    #[ignore]
    fn rewrite_global_for_comparison() {
        let (Some(c), Some(o), Some(root)) = (std::env::var_os("OMSI_ROUNDTRIP_CFG"), std::env::var_os("OMSI_ROUNDTRIP_OUT"), std::env::var_os("OMSI_ROOT")) else { return };
        let roots = crate::install::Roots::new(root.into());
        let doc = Document::load(&roots, Path::new(&c), |_| {}).unwrap();
        std::fs::create_dir_all(&o).unwrap();
        std::fs::write(Path::new(&o).join("global.cfg"), utf16(&global_text(&doc))).unwrap();
    }

    #[test]
    fn utf16_has_a_bom_and_crlf() {
        let b = utf16("a\nb");
        assert_eq!(&b[..2], &[0xFF, 0xFE]);
        assert_eq!(omsi_cfg::decode_text(&b), "a\r\nb");
    }
}
