//! A simple bridge from openMapper to other programs: `openmapper --mcp path/to/global.cfg`
//! is a Model Context Protocol server (https://modelcontextprotocol.io) on stdin / stdout. It
//! reads the map once, with no window, and answers questions about it until it is closed:
//! the map's size and counts, a point in latitude / longitude, world metres or tile + local
//! metres, the map check as data, what stands near a point, the ground's height.
//!
//! Read-only unless started with `--allow-edit`, which adds placing and deleting an object and
//! saving. The messages are JSON, one per line (the protocol's stdio transport); the log goes
//! to stderr and the log file, never to stdout.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use glam::DVec2;
use serde_json::{json, Value};

use crate::doc::{tile_of, tile_origin, Document, Sel, TileKey};
use crate::geo::Georef;
use crate::library::Library;

/// The newest protocol version this answers with when the client names none we know.
const PROTOCOL: &str = "2025-06-18";
/// The versions it can speak (the messages used are the same in all of them).
const KNOWN: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

struct Server {
    roots: crate::install::Roots,
    cfg: PathBuf,
    doc: Document,
    lib: Library,
    allow_edit: bool,
}

type Reply = Result<Value, String>;

/// `openmapper --mcp`: serve until stdin closes. Returns the exit code.
pub fn cli(cfg: &Path, allow_edit: bool) -> i32 {
    let prefs = crate::install::Prefs::load();
    let Some(omsi) = crate::install::find_omsi_root(&prefs) else {
        eprintln!("no OMSI 2 installation found");
        return 2;
    };
    let roots = crate::install::Roots::new(omsi);
    let mut lib = Library::default();
    lib.start_scan(&roots);
    let doc = match Document::load(&roots, cfg, |_| {}) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", cfg.display());
            return 2;
        }
    };
    log::info!("mcp: {} read, {} tiles{}", cfg.display(), doc.tiles.len(), if allow_edit { ", edits allowed" } else { ", read only" });
    let mut server = Server { roots, cfg: cfg.to_path_buf(), doc, lib, allow_edit };
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

fn number(v: f64) -> Value {
    json!((v * 1000.0).round() / 1000.0)
}

impl Server {
    /// One message in, the answer out (none for a notification).
    fn handle(&mut self, line: &str) -> Option<Value> {
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
                    "instructions": "Questions about one openOMSI map. Positions are world metres (x east, y north) unless a tool says otherwise; `convert` turns a point between latitude / longitude, world metres and tile + local metres."
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": self.tools() }),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                match self.call(name, &args) {
                    Ok(v) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "isError": false }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                }
            }
            _ => return Some(error(&id, -32601, &format!("method not found: {method}"))),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn tools(&self) -> Vec<Value> {
        let point = json!({
            "type": "object",
            "description": "A point: {x, y} world metres, or {lat, lon} degrees, or {tile: [tx, ty], local: [x, y]}.",
            "properties": {
                "x": { "type": "number" }, "y": { "type": "number" },
                "lat": { "type": "number" }, "lon": { "type": "number" },
                "tile": { "type": "array", "items": { "type": "integer" } },
                "local": { "type": "array", "items": { "type": "number" } }
            }
        });
        let mut tools = vec![
            json!({ "name": "map_info", "description": "The map: its name, tiles and bounds, counts of objects, splines, bus stops and start points, and whether it is placed on the Earth.", "inputSchema": { "type": "object", "properties": {} } }),
            json!({ "name": "convert", "description": "A point in every form: latitude / longitude, world metres, and tile + local metres.", "inputSchema": point }),
            json!({
                "name": "check",
                "description": "The map check as data, nothing cut short: severity, kind, text, and where (world, tile + local, latitude / longitude) with the object or spline id.",
                "inputSchema": { "type": "object", "properties": {
                    "severity": { "type": "string", "enum": ["error", "warning", "info"], "description": "Only this severity." },
                    "limit": { "type": "integer", "description": "At most this many (all by default)." }
                } }
            }),
            json!({
                "name": "query",
                "description": "Objects, splines, bus stops and start points near a point, nearest first.",
                "inputSchema": { "type": "object", "properties": {
                    "at": point,
                    "radius": { "type": "number", "description": "Metres, 50 by default." },
                    "kinds": { "type": "array", "items": { "type": "string", "enum": ["objects", "splines", "stops", "entries"] } },
                    "limit": { "type": "integer", "description": "At most this many of each, 100 by default." }
                }, "required": ["at"] }
            }),
            json!({
                "name": "terrain",
                "description": "The ground's height (metres) at points, or at evenly spaced samples along a line.",
                "inputSchema": { "type": "object", "properties": {
                    "points": { "type": "array", "items": point },
                    "from": point, "to": point,
                    "samples": { "type": "integer", "description": "Along a line from `from` to `to`: how many, 21 by default." }
                } }
            }),
            json!({ "name": "reload", "description": "Read the map from disk again (after another program rewrote it). Edits not saved are dropped.", "inputSchema": { "type": "object", "properties": {} } }),
        ];
        if self.allow_edit {
            tools.push(json!({
                "name": "add_object",
                "description": "Put an object on the map (not saved until `save`).",
                "inputSchema": { "type": "object", "properties": {
                    "file": { "type": "string", "description": "The object's .sco, as the map names it: Sceneryobjects\\Folder\\name.sco" },
                    "at": point,
                    "heading": { "type": "number", "description": "Degrees clockwise from north, 0 by default." },
                    "height": { "type": "number", "description": "Metres above the ground, 0 by default." }
                }, "required": ["file", "at"] }
            }));
            tools.push(json!({
                "name": "delete_object",
                "description": "Take an object off the map by its id (not saved until `save`). A start point is not deleted this way.",
                "inputSchema": { "type": "object", "properties": { "id": { "type": "integer" } }, "required": ["id"] }
            }));
            tools.push(json!({ "name": "save", "description": "Save the map into its folder (the replaced files are kept in a backup, as for every save).", "inputSchema": { "type": "object", "properties": {} } }));
        }
        tools
    }

    fn call(&mut self, name: &str, args: &Value) -> Reply {
        match name {
            "map_info" => Ok(self.map_info()),
            "convert" => {
                let p = self.point(args)?;
                Ok(self.describe(p))
            }
            "check" => self.check(args),
            "query" => self.query(args),
            "terrain" => self.terrain(args),
            "reload" => self.reload(),
            "add_object" if self.allow_edit => self.add_object(args),
            "delete_object" if self.allow_edit => self.delete_object(args),
            "save" if self.allow_edit => self.save(),
            "add_object" | "delete_object" | "save" => Err("the server is read-only: start it with --allow-edit".into()),
            _ => Err(format!("no such tool: {name}")),
        }
    }

    // ---------------------------------------------------------------------------- points

    fn georef(&self) -> Option<Georef> {
        crate::geo::GeoFile::load(&self.doc.dir).georef(self.doc.global.world_coordinates, crate::doc::tile_size())
    }

    /// A point of the arguments in world metres.
    fn point(&self, v: &Value) -> Result<DVec2, String> {
        let num = |k: &str| v.get(k).and_then(|x| x.as_f64());
        if let (Some(x), Some(y)) = (num("x"), num("y")) {
            return Ok(DVec2::new(x, y));
        }
        if let (Some(la), Some(lo)) = (num("lat"), num("lon")) {
            let g = self.georef().ok_or("the map is not placed on the Earth: no latitude / longitude")?;
            return Ok(g.world(la, lo));
        }
        if let (Some(t), Some(l)) = (v.get("tile").and_then(|t| t.as_array()), v.get("local").and_then(|t| t.as_array())) {
            let n = |a: &[Value], i: usize| a.get(i).and_then(|x| x.as_f64());
            if let (Some(tx), Some(ty), Some(lx), Some(ly)) = (n(t, 0), n(t, 1), n(l, 0), n(l, 1)) {
                return Ok(tile_origin((tx as i32, ty as i32)) + DVec2::new(lx, ly));
            }
        }
        Err("a point needs {x, y}, {lat, lon} or {tile: [tx, ty], local: [x, y]}".into())
    }

    /// A world point in every form.
    fn describe(&self, p: DVec2) -> Value {
        let k = tile_of(p);
        let local = p - tile_origin(k);
        let mut v = json!({
            "global": { "x": number(p.x), "y": number(p.y) },
            "tile": [k.0, k.1],
            "local": { "x": number(local.x), "y": number(local.y) },
            "ground": number(self.doc.ground(p)),
        });
        if let Some(g) = self.georef() {
            let (la, lo) = g.lat_lon(p);
            v["lat"] = json!((la * 1e8).round() / 1e8);
            v["lon"] = json!((lo * 1e8).round() / 1e8);
        }
        v
    }

    // ---------------------------------------------------------------------------- reading

    fn map_info(&self) -> Value {
        let d = &self.doc;
        let keys: Vec<TileKey> = d.tiles.keys().copied().collect();
        let (mut objects, mut splines) = (0, 0);
        for t in d.tiles.values() {
            objects += t.tile.objects.len();
            splines += t.tile.splines.iter().filter(|s| !s.deleted).count();
        }
        let bounds = (!keys.is_empty()).then(|| {
            let (x0, x1) = (keys.iter().map(|k| k.0).min().unwrap_or(0), keys.iter().map(|k| k.0).max().unwrap_or(0));
            let (y0, y1) = (keys.iter().map(|k| k.1).min().unwrap_or(0), keys.iter().map(|k| k.1).max().unwrap_or(0));
            let ts = crate::doc::tile_size();
            json!({ "tiles": { "x": [x0, x1], "y": [y0, y1] }, "metres": { "min": [number(x0 as f64 * ts), number(y0 as f64 * ts)], "max": [number((x1 + 1) as f64 * ts), number((y1 + 1) as f64 * ts)] } })
        });
        json!({
            "name": d.global.name,
            "friendly_name": d.global.friendly_name,
            "folder": d.dir,
            "tile_size": crate::doc::tile_size(),
            "world_coordinates": d.global.world_coordinates,
            "tile_count": keys.len(),
            "tiles": keys,
            "bounds": bounds,
            "objects": objects,
            "splines": splines,
            "bus_stops": crate::stops::all(d).len(),
            "start_points": d.global.entry_points.len(),
            "lines": d.timetable.lines.len(),
            "placed_on_earth": self.georef().is_some(),
            "unsaved_changes": d.is_dirty(),
            "can_edit": self.allow_edit,
        })
    }

    fn check(&mut self, args: &Value) -> Reply {
        // (the catalogue is what the check tells the kinds of things by)
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.lib.poll();
        let want = args.get("severity").and_then(|s| s.as_str()).map(|s| s.to_lowercase());
        let limit = args.get("limit").and_then(|l| l.as_u64()).map(|l| l as usize).unwrap_or(usize::MAX);
        let issues = crate::check::issues(&self.doc, &self.lib);
        let mut out = Vec::new();
        let mut counts = [0usize; 3];
        for i in &issues {
            let sev = format!("{:?}", i.severity).to_lowercase();
            counts[match sev.as_str() { "error" => 0, "warning" => 1, _ => 2 }] += 1;
            if want.as_deref().map(|w| w != sev).unwrap_or(false) || out.len() >= limit {
                continue;
            }
            let mut v = json!({ "severity": sev, "kind": format!("{:?}", i.kind), "label": i.kind.label(), "title": i.title, "detail": i.detail, "fixable": i.fix.is_some() });
            if let Some(p) = i.at {
                v["at"] = self.describe(p);
            }
            match i.sel {
                Some(Sel::Object(k, id)) => v["object"] = json!({ "id": id, "tile": [k.0, k.1] }),
                Some(Sel::Spline(k, id)) => v["spline"] = json!({ "id": id, "tile": [k.0, k.1] }),
                Some(Sel::Attached(k, id)) => v["object"] = json!({ "id": id, "tile": [k.0, k.1], "on_spline": true }),
                None => {}
            }
            out.push(v);
        }
        Ok(json!({ "errors": counts[0], "warnings": counts[1], "infos": counts[2], "total": issues.len(), "issues": out }))
    }

    fn query(&mut self, args: &Value) -> Reply {
        while self.lib.scanning {
            self.lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let c = self.point(args.get("at").ok_or("`at` is needed")?)?;
        let radius = args.get("radius").and_then(|r| r.as_f64()).unwrap_or(50.0).max(0.0);
        let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(100) as usize;
        let kinds: Vec<String> = args.get("kinds").and_then(|k| k.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let wants = |k: &str| kinds.is_empty() || kinds.iter().any(|x| x == k);
        let d = &self.doc;
        let mut out = serde_json::Map::new();
        out.insert("at".into(), self.describe(c));
        out.insert("radius".into(), json!(radius));
        if wants("objects") {
            let mut found: Vec<(f64, Value)> = Vec::new();
            for (k, t) in &d.tiles {
                let org = tile_origin(*k);
                for o in &t.tile.objects {
                    let p = org + DVec2::new(o.pos[0], o.pos[1]);
                    let dist = (p - c).length();
                    if dist <= radius {
                        found.push((dist, json!({ "id": o.id, "file": o.file, "distance": number(dist), "heading": number(o.rot[0]), "height": number(o.pos[2]), "at": self.describe(p) })));
                    }
                }
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.insert("objects".into(), Value::Array(found.into_iter().take(limit).map(|f| f.1).collect()));
        }
        if wants("splines") {
            let mut found: Vec<(f64, Value)> = Vec::new();
            for (k, t) in &d.tiles {
                let org = tile_origin(*k);
                for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                    // (the start, the end and a few places between: how near the line comes)
                    let curve = omsi_geometry::SplineCurve::from_map(s, org);
                    let len = s.length.max(0.0);
                    let steps = ((len / 10.0).ceil() as usize).clamp(2, 200);
                    let mut near = f64::MAX;
                    for i in 0..=steps {
                        let p = curve.point_at(len * i as f64 / steps as f64).truncate();
                        near = near.min((p - c).length());
                    }
                    if near <= radius {
                        let (a, b) = (curve.point_at(0.0).truncate(), curve.end_point().truncate());
                        found.push((near, json!({ "id": s.id, "file": s.file, "distance": number(near), "length": number(len), "start": self.describe(a), "end": self.describe(b), "prev_id": s.prev_id, "next_id": s.next_id })));
                    }
                }
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.insert("splines".into(), Value::Array(found.into_iter().take(limit).map(|f| f.1).collect()));
        }
        if wants("stops") {
            let mut found: Vec<(f64, Value)> = crate::stops::all(d).into_iter().filter_map(|f| {
                let dist = (f.at - c).length();
                (dist <= radius).then(|| (dist, json!({ "id": f.id, "name": f.name, "distance": number(dist), "at": self.describe(f.at) })))
            }).collect();
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.insert("stops".into(), Value::Array(found.into_iter().take(limit).map(|f| f.1).collect()));
        }
        if wants("entries") {
            let mut found: Vec<(f64, Value)> = Vec::new();
            for (i, e) in d.global.entry_points.iter().enumerate() {
                if let Some((k, o)) = crate::selection::object_by_id(d, e.object_id) {
                    let p = tile_origin(k) + DVec2::new(o.pos[0], o.pos[1]);
                    let dist = (p - c).length();
                    if dist <= radius {
                        found.push((dist, json!({ "index": i, "name": e.name, "object": e.object_id, "distance": number(dist), "at": self.describe(p) })));
                    }
                }
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.insert("entries".into(), Value::Array(found.into_iter().take(limit).map(|f| f.1).collect()));
        }
        Ok(Value::Object(out))
    }

    fn terrain(&self, args: &Value) -> Reply {
        let mut points: Vec<DVec2> = Vec::new();
        if let Some(list) = args.get("points").and_then(|p| p.as_array()) {
            for p in list {
                points.push(self.point(p)?);
            }
        }
        if let (Some(a), Some(b)) = (args.get("from"), args.get("to")) {
            let (a, b) = (self.point(a)?, self.point(b)?);
            let n = args.get("samples").and_then(|s| s.as_u64()).unwrap_or(21).clamp(2, 5000) as usize;
            points.extend((0..n).map(|i| a.lerp(b, i as f64 / (n - 1) as f64)));
        }
        if points.is_empty() {
            return Err("give `points`, or `from` and `to`".into());
        }
        if points.len() > 5000 {
            return Err("at most 5000 points".into());
        }
        let d = &self.doc;
        let heights: Vec<Value> = points.iter().map(|p| {
            let on_map = d.tiles.contains_key(&tile_of(*p));
            json!({ "x": number(p.x), "y": number(p.y), "height": if on_map { number(d.ground(*p)) } else { Value::Null } })
        }).collect();
        Ok(json!({ "heights": heights }))
    }

    fn reload(&mut self) -> Reply {
        let doc = Document::load(&self.roots, &self.cfg, |_| {}).map_err(|e| format!("{}: {e}", self.cfg.display()))?;
        self.doc = doc;
        Ok(json!({ "reloaded": true, "tiles": self.doc.tiles.len() }))
    }

    // ---------------------------------------------------------------------------- editing

    fn add_object(&mut self, args: &Value) -> Reply {
        let file = args.get("file").and_then(|f| f.as_str()).ok_or("`file` is needed")?.replace('/', "\\");
        if !file.to_lowercase().ends_with(".sco") {
            return Err("`file` is an object's .sco, as Sceneryobjects\\Folder\\name.sco".into());
        }
        let p = self.point(args.get("at").ok_or("`at` is needed")?)?;
        let heading = args.get("heading").and_then(|h| h.as_f64()).unwrap_or(0.0);
        let height = args.get("height").and_then(|h| h.as_f64()).unwrap_or(0.0);
        let k = tile_of(p);
        if !self.doc.tiles.contains_key(&k) {
            return Err(format!("the map has no tile {}, {} there", k.0, k.1));
        }
        let id = self.doc.new_id();
        let local = p - tile_origin(k);
        let t = self.doc.tiles.get_mut(&k).ok_or("no such tile")?;
        t.tile.objects.push(omsi_map::MapObject { file, id, pos: [local.x, local.y, height], rot: [heading, 0.0, 0.0], ..Default::default() });
        t.touch_content();
        Ok(json!({ "id": id, "tile": [k.0, k.1], "at": self.describe(p) }))
    }

    fn delete_object(&mut self, args: &Value) -> Reply {
        let id = args.get("id").and_then(|i| i.as_i64()).ok_or("`id` is needed")?;
        if self.doc.global.entry_points.iter().any(|e| e.object_id == id) {
            return Err("that object is a start point: it is not deleted this way".into());
        }
        for t in self.doc.tiles.values_mut() {
            if let Some(i) = t.tile.objects.iter().position(|o| o.id == id) {
                t.tile.objects.remove(i);
                t.touch_content();
                return Ok(json!({ "deleted": id }));
            }
        }
        Err(format!("no object with id {id}"))
    }

    fn save(&mut self) -> Reply {
        crate::doc::save::save(&mut self.doc).map_err(|e| e.to_string())?;
        Ok(json!({ "saved": true, "folder": self.doc.dir }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server for a map that is not read from anywhere: enough to speak the protocol.
    fn talk(server: &mut Server, line: &str) -> Value {
        server.handle(line).expect("an answer")
    }

    #[test]
    fn speaks_the_protocol_without_a_map_on_disk() {
        let _map = crate::doc::one_map();
        let roots = crate::install::Roots { omsi: PathBuf::new(), content: PathBuf::new() };
        let mut doc = Document {
            global: Default::default(),
            _publisher_mounts: std::sync::Arc::new(Default::default()),
            _publisher_protected_source: false,
            tiles: Default::default(),
            order: Vec::new(),
            dir: PathBuf::new(),
            source: None,
            global_dirty: false,
            fresh: false,
            global_rev: 0,
            timetable: Default::default(),
            traffic: Default::default(),
            descriptions: Default::default(),
            groups: Vec::new(),
        };
        doc.add_tile((0, 0));
        let mut s = Server { roots, cfg: PathBuf::new(), doc, lib: Library::default(), allow_edit: false };
        let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#);
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert!(s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"map_info") && !names.contains(&"save"));
        // a point converts both ways
        let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"convert","arguments":{"tile":[1,2],"local":[10,20]}}}"#);
        assert_eq!(r["result"]["isError"], false);
        let text: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        let ts = crate::doc::tile_size();
        assert_eq!(text["global"]["x"], json!(number(ts + 10.0)));
        assert_eq!(text["tile"], json!([1, 2]));
        // editing is refused when read-only
        let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"save","arguments":{}}}"#);
        assert_eq!(r["result"]["isError"], true);
        let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":5,"method":"nothing"}"#);
        assert_eq!(r["error"]["code"], -32601);
    }
}
