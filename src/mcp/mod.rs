//! A bridge from openMapper to other programs: `openmapper --mcp [path/to/global.cfg]` is a
//! Model Context Protocol server (https://modelcontextprotocol.io) on stdin / stdout, with no
//! window. It holds the editor itself (the same `Editor` the window drives, with its undo and
//! redo), the catalogue and the models, so an agent works on a map as a person does:
//!
//! * it sees the map: its facts, the check, what stands where, the ground, the catalogue, the
//!   timetable, and a picture of any part of it from above (`render`);
//! * it uses every tool of the editor by clicks, strokes and the menu's commands
//!   (`set_editor`, `click`, `stroke`, `command`, `select`);
//! * and does in one call what takes a person many: a road along points, a roundabout, a bus
//!   pocket, stops with their names, a whole bus line with its route and timetable, hills,
//!   lakes, rivers, paint, plants, objects in hundreds, the check's fixes, the map's script,
//!   any of these together in `batch` as one undo step.
//!
//! Read-only unless started with `--allow-edit`. The messages are JSON, one per line (the
//! protocol's stdio transport); the log goes to stderr and the log file, never to stdout.

mod edit;
mod read;
mod render;
mod schema;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DVec2, DVec3};
use serde_json::{json, Value};

use crate::assets::Assets;
use crate::doc::{tile_of, tile_origin, Document};
use crate::editor::Editor;
use crate::geo::Georef;
use crate::library::Library;

/// The newest protocol version this answers with when the client names none we know.
const PROTOCOL: &str = "2025-06-18";
/// The versions it can speak (the messages used are the same in all of them).
const KNOWN: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

/// What the server says about itself to an agent at the start.
const INSTRUCTIONS: &str = "openMapper: an editor of openOMSI / OMSI 2 maps, driven as a person drives it.\n\
Positions are world metres (x east, y north): give a point as [x, y], {x, y}, {lat, lon} or {tile: [tx, ty], local: [x, y]}. Headings are degrees clockwise from north.\n\
Start with `map_info`, look with `render` (a picture from above) and `query`; find files to build with in `catalogue` (roads, objects, trees, stops...).\n\
Build with the one-call tools (`draw_road`, `roundabout`, `place_objects`, `bus_stop`, `make_line`, `terrain_brush`, `paint`, `plant`, `water`...) or drive the editor's tools like a person (`set_editor`, `click`, `stroke`, `command`, `select`).\n\
Every edit answers with what it added and removed and the editor's own messages; `undo` / `redo` step back and forth; `batch` runs many calls in one go as one undo step. Nothing is written until `save`. Check the map with `check` and put things right with `fix`.";

/// What a tool gives back: data, or a picture with data about it.
pub(crate) enum Out {
    Json(Value),
    Image { png: Vec<u8>, meta: Value },
}

impl From<Value> for Out {
    fn from(v: Value) -> Out {
        Out::Json(v)
    }
}

pub(crate) type Reply = Result<Out, String>;

const PROTECTED: &str = "publisher-protected maps cannot be exposed through MCP";

pub(crate) fn load_document(roots: &crate::install::Roots, cfg: &Path) -> anyhow::Result<Document> {
    let doc = Document::load(roots, cfg, |_| {})?;
    anyhow::ensure!(!doc._publisher_protected_source, PROTECTED);
    Ok(doc)
}

pub(crate) struct Server {
    pub roots: crate::install::Roots,
    pub cfg: Option<PathBuf>,
    pub ed: Editor,
    pub assets: Assets,
    pub lib: Library,
    /// The timetable's bookkeeping a save does (the trips' ways found again over changed roads).
    pub tt: crate::ui_timetable::TimetableUi,
    pub allow_edit: bool,
    /// The spline types of the map are loaded (the road tool joins to them by their widths).
    warmed: bool,
    /// Inside `batch`: one undo step for all of it.
    in_batch: bool,
}

/// `openmapper --mcp`: serve until stdin closes. Returns the exit code.
pub fn cli(cfg: Option<&Path>, allow_edit: bool) -> i32 {
    let prefs = crate::install::Prefs::load();
    let Some(omsi) = crate::install::find_omsi_root(&prefs) else {
        eprintln!("no OMSI 2 installation found");
        return 2;
    };
    let roots = crate::install::Roots::new(omsi);
    let mut server = Server::new(roots, allow_edit);
    if let Some(cfg) = cfg {
        if let Err(e) = server.open(cfg) {
            eprintln!("{e}");
            return 2;
        }
    }
    log::info!("mcp: {}{}", cfg.map(|c| c.display().to_string()).unwrap_or_else(|| "no map yet".into()), if allow_edit { ", edits allowed" } else { ", read only" });
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = server.handle(&line) {
            if writeln!(out, "{answer}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
    0
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A number rounded to a millimetre.
pub(crate) fn number(v: f64) -> Value {
    json!((v * 1000.0).round() / 1000.0)
}

/// A world point, short: [x, y] to a centimetre.
pub(crate) fn p2(p: DVec2) -> Value {
    json!([(p.x * 100.0).round() / 100.0, (p.y * 100.0).round() / 100.0])
}

/// The arguments of a call, read leniently.
#[derive(Clone, Copy)]
pub(crate) struct Args<'a>(pub &'a Value);

impl<'a> Args<'a> {
    pub fn get(&self, k: &str) -> Option<&'a Value> {
        self.0.get(k).filter(|v| !v.is_null())
    }
    pub fn f64(&self, k: &str) -> Option<f64> {
        self.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
    }
    pub fn i64(&self, k: &str) -> Option<i64> {
        self.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
    }
    pub fn usize(&self, k: &str) -> Option<usize> {
        self.i64(k).map(|v| v.max(0) as usize)
    }
    pub fn bool(&self, k: &str) -> Option<bool> {
        self.get(k).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| matches!(s, "true" | "yes" | "1"))))
    }
    pub fn str(&self, k: &str) -> Option<&'a str> {
        self.get(k).and_then(|v| v.as_str())
    }
    pub fn arr(&self, k: &str) -> Option<&'a Vec<Value>> {
        self.get(k).and_then(|v| v.as_array())
    }
    pub fn strings(&self, k: &str) -> Vec<String> {
        match self.get(k) {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        }
    }
    pub fn ids(&self, k: &str) -> Vec<i64> {
        match self.get(k) {
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_i64()).collect(),
            Some(v) => v.as_i64().into_iter().collect(),
            None => Vec::new(),
        }
    }
}

/// What the map holds, by IDCode, to tell what an edit added and took away.
pub(crate) struct Before {
    objects: HashSet<i64>,
    splines: HashSet<i64>,
    tiles: usize,
}

impl Server {
    pub fn new(roots: crate::install::Roots, allow_edit: bool) -> Server {
        let mut lib = Library::default();
        lib.start_scan(&roots);
        let assets = Assets::new(roots.omsi.clone(), roots.content.clone());
        let mut ed = Editor::default();
        ed.content_dir = Some(roots.content.clone());
        ed.road_styles = crate::road_maker::Styles::load(&roots.content);
        ed.asphalt = crate::junction::asphalt_texture(&roots.omsi);
        Server { roots, cfg: None, ed, assets, lib, tt: crate::ui_timetable::TimetableUi::new(), allow_edit, warmed: false, in_batch: false }
    }

    /// Open a map (its global.cfg), dropping the one open.
    pub fn open(&mut self, cfg: &Path) -> Result<(), String> {
        let doc = load_document(&self.roots, cfg).map_err(|e| format!("{}: {e}", cfg.display()))?;
        self.put(doc);
        self.cfg = Some(cfg.to_path_buf());
        Ok(())
    }

    /// A map in the editor, as the window opens one.
    pub fn put(&mut self, doc: Document) {
        self.ed.open(doc);
        self.ed.set_tool(crate::editor::Tool::Select);
        self.ed.toasts.clear();
        self.tt = crate::ui_timetable::TimetableUi::new();
        self.warmed = false;
    }

    pub fn doc(&self) -> Result<&Document, String> {
        self.ed.doc.as_ref().ok_or_else(|| "no map is open: use `open_map` or `new_map`".to_string())
    }

    /// One message in, the answer out (none for a notification).
    pub fn handle(&mut self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return Some(error(&Value::Null, -32700, &format!("parse error: {e}"))),
        };
        // (a message with no id is a notification: nothing is sent back)
        let id = msg.get("id").cloned()?;
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
                let version = if KNOWN.contains(&asked) { asked } else { PROTOCOL };
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "openMapper", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": self.tools() }),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                match self.call(name, &args) {
                    Ok(Out::Json(v)) => json!({ "content": [{ "type": "text", "text": v.to_string() }], "isError": false }),
                    Ok(Out::Image { png, meta }) => json!({ "content": [
                        { "type": "image", "data": base64(&png), "mimeType": "image/png" },
                        { "type": "text", "text": meta.to_string() }
                    ], "isError": false }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                }
            }
            _ => return Some(error(&id, -32601, &format!("method not found: {method}"))),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    /// A tool by its name. (A tool that changes the map is refused when the server is read-only.)
    pub fn call(&mut self, name: &str, args: &Value) -> Reply {
        let a = Args(args);
        if schema::EDITS.contains(&name) && !self.allow_edit {
            return Err("the server is read-only: start it with --allow-edit".into());
        }
        // (the files a map is opened from or made in: no map need be open)
        match name {
            "open_map" => return self.open_map(a),
            "new_map" => return self.new_map(a),
            "catalogue" => return self.catalogue(a),
            "list_maps" => return self.list_maps(),
            _ => {}
        }
        if self.doc()?._publisher_protected_source {
            return Err(PROTECTED.into());
        }
        match name {
            // seeing
            "map_info" => self.map_info().map(Out::from),
            "convert" => {
                let p = self.point(args)?;
                Ok(self.describe(p).into())
            }
            "check" => self.check(a),
            "query" => self.query(a),
            "inspect" => self.inspect(a),
            "terrain" => self.terrain(a),
            "render" => self.render(a),
            "timetable" => self.timetable(a),
            "editor_state" => self.editor_state(),
            "reload" => self.reload(),
            // the editor's own tools, as a person uses them
            "set_editor" => self.edit(|s| s.set_editor(a)),
            "click" => self.edit(|s| s.click(a)),
            "stroke" => self.edit(|s| s.stroke(a)),
            "command" => self.edit(|s| s.command(a)),
            "select" => self.select(a),
            "undo" => self.edit(|s| s.undo_redo(a, true)),
            "redo" => self.edit(|s| s.undo_redo(a, false)),
            // in one call
            "draw_road" => self.edit(|s| s.draw_road(a)),
            "roundabout" => self.edit(|s| s.roundabout(a)),
            "bus_pocket" => self.edit(|s| s.bus_pocket(a)),
            "place_objects" => self.edit(|s| s.place_objects(a)),
            "bus_stop" => self.edit(|s| s.bus_stop(a)),
            "start_point" => self.edit(|s| s.start_point(a)),
            "ai_path" => self.edit(|s| s.ai_path(a)),
            "terrain_brush" => self.edit(|s| s.terrain_brush(a)),
            "ramp" => self.edit(|s| s.ramp(a)),
            "water" => self.edit(|s| s.water(a)),
            "paint" => self.edit(|s| s.paint(a)),
            "plant" => self.edit(|s| s.plant(a)),
            "edit_object" => self.edit(|s| s.edit_object(a)),
            "edit_spline" => self.edit(|s| s.edit_spline(a)),
            "delete" => self.edit(|s| s.delete(a)),
            "tiles" => self.edit(|s| s.tiles(a)),
            "ground_textures" => self.edit(|s| s.ground_textures(a)),
            "make_line" => self.edit(|s| s.make_line(a)),
            "fix" => self.edit(|s| s.fix(a)),
            "sample" => self.edit(|s| s.sample(a)),
            "script" => self.script(a),
            "batch" => self.batch(a),
            "save" => self.save(),
            "export" => self.export(a),
            _ => Err(format!("no such tool: {name}")),
        }
    }

    /// Many calls in one: in order, each one's answer kept; one undo step for all of them.
    fn batch(&mut self, a: Args) -> Reply {
        let calls = a.arr("calls").ok_or("`calls` is needed: [{tool, arguments}, ...]")?;
        if self.in_batch {
            return Err("a batch inside a batch".into());
        }
        let stop = a.bool("stop_on_error").unwrap_or(true);
        let label = a.str("label").unwrap_or("Agent's batch").to_string();
        self.in_batch = true;
        let mark = self.ed.history.mark();
        let mut results = Vec::new();
        let mut failed = 0;
        for (i, c) in calls.iter().enumerate() {
            let name = c.get("tool").or_else(|| c.get("name")).and_then(|n| n.as_str()).unwrap_or("");
            let args = c.get("arguments").or_else(|| c.get("args")).cloned().unwrap_or(json!({}));
            let r = if name == "batch" { Err("a batch inside a batch".into()) } else { self.call(name, &args) };
            match r {
                Ok(Out::Json(v)) => results.push(json!({ "i": i, "tool": name, "ok": true, "result": v })),
                Ok(Out::Image { meta, .. }) => results.push(json!({ "i": i, "tool": name, "ok": true, "result": meta, "note": "pictures are not sent from a batch: call render alone" })),
                Err(e) => {
                    failed += 1;
                    results.push(json!({ "i": i, "tool": name, "ok": false, "error": e }));
                    if stop {
                        break;
                    }
                }
            }
        }
        self.in_batch = false;
        self.ed.history.commit();
        self.ed.history.merge_since(mark, &label);
        Ok(json!({ "done": results.len() - failed, "failed": failed, "of": calls.len(), "results": results }).into())
    }

    // ---------------------------------------------------------------------------- points

    pub fn georef(&self) -> Option<Georef> {
        let d = self.ed.doc.as_ref()?;
        crate::geo::GeoFile::load(&d.dir).georef(d.global.world_coordinates, crate::doc::tile_size())
    }

    /// A point of the arguments in world metres: [x, y], {x, y}, {lat, lon} or
    /// {tile: [tx, ty], local: [x, y]}.
    pub fn point(&self, v: &Value) -> Result<DVec2, String> {
        if let Some(a) = v.as_array() {
            if let (Some(x), Some(y)) = (a.first().and_then(|x| x.as_f64()), a.get(1).and_then(|y| y.as_f64())) {
                return Ok(DVec2::new(x, y));
            }
        }
        let num = |k: &str| v.get(k).and_then(|x| x.as_f64());
        if let (Some(x), Some(y)) = (num("x"), num("y")) {
            return Ok(DVec2::new(x, y));
        }
        if let (Some(la), Some(lo)) = (num("lat"), num("lon").or_else(|| num("lng"))) {
            let g = self.georef().ok_or("the map is not placed on the Earth: no latitude / longitude")?;
            return Ok(g.world(la, lo));
        }
        if let (Some(t), Some(l)) = (v.get("tile").and_then(|t| t.as_array()), v.get("local").and_then(|t| t.as_array())) {
            let n = |a: &[Value], i: usize| a.get(i).and_then(|x| x.as_f64());
            if let (Some(tx), Some(ty), Some(lx), Some(ly)) = (n(t, 0), n(t, 1), n(l, 0), n(l, 1)) {
                return Ok(tile_origin((tx as i32, ty as i32)) + DVec2::new(lx, ly));
            }
        }
        Err(format!("a point is [x, y], {{x, y}}, {{lat, lon}} or {{tile: [tx, ty], local: [x, y]}}, not {v}"))
    }

    /// A list of points (`key`).
    pub fn points(&self, a: Args, key: &str) -> Result<Vec<DVec2>, String> {
        let list = a.arr(key).ok_or_else(|| format!("`{key}` is needed: a list of points"))?;
        list.iter().map(|p| self.point(p)).collect()
    }

    /// A world point in every form.
    pub fn describe(&self, p: DVec2) -> Value {
        let k = tile_of(p);
        let local = p - tile_origin(k);
        let on_map = self.ed.doc.as_ref().map(|d| d.tiles.contains_key(&k)).unwrap_or(false);
        let mut v = json!({
            "global": { "x": number(p.x), "y": number(p.y) },
            "tile": [k.0, k.1],
            "local": { "x": number(local.x), "y": number(local.y) },
            "on_map": on_map,
        });
        if let Some(d) = self.ed.doc.as_ref().filter(|_| on_map) {
            v["ground"] = number(d.ground(p));
        }
        if let Some(g) = self.georef() {
            let (la, lo) = g.lat_lon(p);
            v["lat"] = json!((la * 1e8).round() / 1e8);
            v["lon"] = json!((lo * 1e8).round() / 1e8);
        }
        v
    }

    // ---------------------------------------------------------------------------- the editor

    /// The catalogue read to the end (what is found by name and kind comes from it).
    pub fn wait_lib(&mut self) {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        self.lib.poll();
    }

    /// The models asked for are read and the editor has done what waited for them.
    pub fn settle(&mut self) {
        let start = Instant::now();
        loop {
            self.assets.poll();
            self.ed.tick(&mut self.assets, 0.0, false);
            if self.assets.pending == 0 || start.elapsed() > Duration::from_secs(20) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.ed.tick(&mut self.assets, 0.0, false);
    }

    /// Every spline type of the map loaded (roads join to roads by their widths and lanes;
    /// once per map).
    pub fn warm(&mut self) {
        if self.warmed {
            return;
        }
        self.warmed = true;
        let files: HashSet<String> = self.ed.doc.as_ref().map(|d| d.tiles.values().flat_map(|t| t.tile.splines.iter().filter(|s| !s.deleted).map(|s| s.file.to_lowercase())).collect()).unwrap_or_default();
        for f in &files {
            let _ = self.assets.spline(f);
        }
        let start = Instant::now();
        while self.assets.pending > 0 && start.elapsed() < Duration::from_secs(60) {
            self.assets.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The ground's height at a point (0 off the map).
    pub fn ground(&self, p: DVec2) -> f64 {
        self.ed.doc.as_ref().map(|d| d.ground(p)).unwrap_or(0.0)
    }

    /// The cursor put on a point, as a mouse over the view does.
    pub fn hover(&mut self, p: DVec2) {
        let z = self.ground(p);
        self.ed.cursor_in_view = true;
        self.ed.hover_point(&mut self.assets, &self.lib, DVec3::new(p.x, p.y, z));
    }

    /// One click of the left button at a point.
    pub fn click_at(&mut self, p: DVec2, shift: bool) {
        self.hover(p);
        self.ed.left_press(&mut self.assets, &self.lib, shift);
        self.ed.tick(&mut self.assets, 0.0, false);
        self.ed.left_release(&mut self.assets, &self.lib, false);
    }

    /// The left button held down along a path for a time (a brush stroke, a drag).
    pub fn hold(&mut self, path: &[DVec2], seconds: f64) {
        let Some(first) = path.first().copied() else { return };
        self.hover(first);
        self.ed.left_press(&mut self.assets, &self.lib, false);
        let steps = ((seconds / 0.05).round() as usize).clamp(1, 20_000);
        // (evenly along the path's length)
        let lens: Vec<f64> = path.windows(2).map(|w| (w[1] - w[0]).length()).collect();
        let total: f64 = lens.iter().sum();
        for i in 0..steps {
            if path.len() > 1 && total > 0.0 {
                let mut s = total * (i + 1) as f64 / steps as f64;
                let mut p = *path.last().unwrap();
                for (j, l) in lens.iter().enumerate() {
                    if s <= *l {
                        p = path[j].lerp(path[j + 1], if *l > 0.0 { s / l } else { 0.0 });
                        break;
                    }
                    s -= l;
                }
                self.hover(p);
            }
            self.ed.tick(&mut self.assets, 0.05, true);
        }
        self.ed.tick(&mut self.assets, 0.0, false);
        self.ed.left_release(&mut self.assets, &self.lib, path.len() > 1);
    }

    /// What the map holds now, to tell what an edit changed.
    pub fn before(&self) -> Before {
        let mut b = Before { objects: HashSet::new(), splines: HashSet::new(), tiles: 0 };
        if let Some(d) = &self.ed.doc {
            b.tiles = d.tiles.len();
            for t in d.tiles.values() {
                b.objects.extend(t.tile.objects.iter().map(|o| o.id));
                b.splines.extend(t.tile.splines.iter().filter(|s| !s.deleted).map(|s| s.id));
            }
        }
        b
    }

    /// What changed since `before`, and what the editor said meanwhile.
    pub fn changes(&mut self, before: &Before) -> Value {
        const SHOWN: usize = 200;
        let mut added_o = Vec::new();
        let mut added_s = Vec::new();
        let mut now_o = HashSet::new();
        let mut now_s = HashSet::new();
        let mut tiles = 0;
        if let Some(d) = &self.ed.doc {
            tiles = d.tiles.len();
            for (k, t) in &d.tiles {
                let org = tile_origin(*k);
                for o in &t.tile.objects {
                    now_o.insert(o.id);
                    if !before.objects.contains(&o.id) {
                        added_o.push(json!({ "id": o.id, "file": o.file, "at": p2(org + DVec2::new(o.pos[0], o.pos[1])), "heading": number(o.rot[0]) }));
                    }
                }
                for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                    now_s.insert(s.id);
                    if !before.splines.contains(&s.id) {
                        let c = omsi_geometry::SplineCurve::from_map(s, org);
                        added_s.push(json!({ "id": s.id, "file": s.file, "from": p2(c.point_at(0.0).truncate()), "to": p2(c.end_point().truncate()), "length": number(s.length) }));
                    }
                }
            }
        }
        let removed_o: Vec<i64> = before.objects.difference(&now_o).copied().collect();
        let removed_s: Vec<i64> = before.splines.difference(&now_s).copied().collect();
        let mut v = serde_json::Map::new();
        let mut put = |name: &str, list: Vec<Value>| {
            if !list.is_empty() {
                let n = list.len();
                v.insert(name.into(), Value::Array(list.into_iter().take(SHOWN).collect()));
                if n > SHOWN {
                    v.insert(format!("{name}_count"), json!(n));
                }
            }
        };
        put("added_objects", added_o);
        put("added_splines", added_s);
        put("removed_objects", removed_o.into_iter().map(Value::from).collect());
        put("removed_splines", removed_s.into_iter().map(Value::from).collect());
        if tiles != before.tiles {
            v.insert("tiles".into(), json!(tiles));
        }
        let (msgs, errs): (Vec<_>, Vec<_>) = self.ed.toasts.drain(..).partition(|t| !t.error);
        if !msgs.is_empty() {
            v.insert("messages".into(), json!(msgs.into_iter().map(|t| t.text).collect::<Vec<_>>()));
        }
        if !errs.is_empty() {
            v.insert("errors".into(), json!(errs.into_iter().map(|t| t.text).collect::<Vec<_>>()));
        }
        Value::Object(v)
    }

    /// An edit: what it returns, with what it changed on the map and what the editor said.
    pub fn edit(&mut self, f: impl FnOnce(&mut Self) -> Result<Value, String>) -> Reply {
        self.doc()?;
        self.wait_lib();
        self.warm();
        self.ed.toasts.clear();
        let before = self.before();
        let r = f(self);
        self.settle();
        let mut changes = self.changes(&before);
        let v = r?;
        if let (Value::Object(c), Value::Object(own)) = (&mut changes, v) {
            for (k, x) in own {
                c.insert(k, x);
            }
        }
        if !self.in_batch {
            self.ed.history.commit();
        }
        changes["unsaved"] = json!(self.ed.doc.as_ref().map(|d| d.is_dirty()).unwrap_or(false));
        Ok(changes.into())
    }
}

/// Standard base64 (the picture of `render`).
pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
