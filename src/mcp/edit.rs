//! What the server changes on the map: the editor's tools driven as a person drives them, and
//! the jobs of many clicks done in one call.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use serde_json::{json, Value};

use super::{number, p2, Args, Reply, Server};
use crate::doc::{tile_of, tile_origin, Document, Sel, TileKey};
use crate::editor::{LogicMode, TerrainMode, Tool, WaterShape};
use crate::library::{Category, Kind};

type Done = Result<Value, String>;

fn tool_of(s: &str) -> Result<Tool, String> {
    Ok(match s.to_lowercase().as_str() {
        "select" => Tool::Select,
        "roads" | "road" => Tool::Roads,
        "objects" | "object" => Tool::Objects,
        "logic" | "game_logic" => Tool::Logic,
        "terrain" => Tool::Terrain,
        "paint" => Tool::Paint,
        "tiles" => Tool::Tiles,
        _ => return Err(format!("no tool {s}: select, roads, objects, logic, terrain, paint, tiles")),
    })
}

fn terrain_mode_of(s: &str) -> Result<TerrainMode, String> {
    Ok(match s.to_lowercase().as_str() {
        "raise" => TerrainMode::Raise,
        "lower" => TerrainMode::Lower,
        "smooth" => TerrainMode::Smooth,
        "flatten" => TerrainMode::Flatten,
        "roughen" => TerrainMode::Roughen,
        "ramp" => TerrainMode::Ramp,
        "water" | "lake" | "river" => TerrainMode::Water,
        "along_spline" | "alongspline" => TerrainMode::AlongSpline,
        _ => return Err(format!("no terrain mode {s}: raise, lower, smooth, flatten, roughen, ramp, water, along_spline")),
    })
}

fn logic_of(s: &str) -> Result<LogicMode, String> {
    Ok(match s.to_lowercase().as_str() {
        "start" | "start_points" => LogicMode::Start,
        "stops" | "bus_stops" => LogicMode::Stops,
        "parking" | "car_parks" => LogicMode::Parking,
        "paths" | "ai_paths" => LogicMode::Paths,
        "other" => LogicMode::Other,
        _ => return Err(format!("no logic mode {s}: start, stops, parking, paths, other")),
    })
}

fn stop_kind_of(s: &str) -> crate::stops::StopKind {
    match s.to_lowercase().as_str() {
        "request" | "on_request" | "onrequest" => crate::stops::StopKind::OnRequest,
        "terminus" => crate::stops::StopKind::Terminus,
        _ => crate::stops::StopKind::Regular,
    }
}

/// "06:30" (or minutes, or hours as a number) as minutes after midnight.
fn minutes_of(v: Option<&Value>, default: f32) -> Result<f32, String> {
    match v {
        None => Ok(default),
        Some(Value::Number(n)) => {
            let n = n.as_f64().unwrap_or(0.0);
            Ok(if n <= 24.0 { n * 60.0 } else { n } as f32)
        }
        Some(Value::String(s)) => {
            let mut it = s.trim().split(':');
            let h: f32 = it.next().and_then(|x| x.trim().parse().ok()).ok_or(format!("a time is \"HH:MM\", not {s}"))?;
            let m: f32 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
            Ok(h * 60.0 + m)
        }
        Some(v) => Err(format!("a time is \"HH:MM\", not {v}")),
    }
}

impl Server {
    // ---------------------------------------------------------------------------- files

    /// A file of the catalogue as the map names it (`spline`: which kind is wanted), else what
    /// is like it.
    pub fn resolve(&mut self, name: &str, spline: Option<bool>) -> Result<String, String> {
        self.wait_lib();
        let rel = name.trim().replace('/', "\\");
        if let Some(a) = self.lib.find(&rel) {
            if spline.map(|s| s == (a.kind == Kind::Spline)).unwrap_or(true) {
                return Ok(a.rel.clone());
            }
            return Err(format!("{rel} is {}, not {}", if a.kind == Kind::Spline { "a spline" } else { "an object" }, if spline == Some(true) { "a spline" } else { "an object" }));
        }
        // (a file that is there but the catalogue leaves out)
        let lower = rel.to_lowercase();
        let ext_ok = match spline {
            Some(true) => lower.ends_with(".sli"),
            Some(false) => lower.ends_with(".sco"),
            None => lower.ends_with(".sli") || lower.ends_with(".sco"),
        };
        if ext_ok && (omsi_cfg::vfs::exists(&omsi_cfg::resolve_path(&self.roots.content, &rel)) || omsi_cfg::vfs::exists(&omsi_cfg::resolve_path(&self.roots.omsi, &rel))) {
            return Ok(rel);
        }
        let stem = rel.rsplit('\\').next().unwrap_or(&rel).split('.').next().unwrap_or("").to_lowercase();
        let kind = match spline {
            Some(true) => vec![Kind::Spline],
            Some(false) => vec![Kind::Object],
            None => vec![Kind::Spline, Kind::Object],
        };
        let like: Vec<String> = kind.iter().flat_map(|k| self.lib.filter(*k, None, &stem)).take(8).map(|a| a.rel.clone()).collect();
        Err(if like.is_empty() { format!("no file {rel} in the catalogue: search it with `catalogue`") } else { format!("no file {rel} in the catalogue; like it: {}", like.join(", ")) })
    }

    pub(super) fn wait_spline(&mut self, rel: &str) -> Result<(), String> {
        self.assets.spline_wait(rel, 15_000).map(|_| ()).ok_or_else(|| format!("{rel} could not be read"))
    }

    fn wait_object(&mut self, rel: &str) -> Result<(), String> {
        self.assets.object_wait(rel, 15_000).map(|_| ()).ok_or_else(|| format!("{rel} could not be read"))
    }

    /// The first catalogue file of a kind.
    fn first_asset(&mut self, f: impl Fn(&crate::library::Asset) -> bool) -> Option<String> {
        self.wait_lib();
        self.lib.assets.iter().find(|a| f(a)).map(|a| a.rel.clone())
    }

    /// Where an object or spline is, by IDCode.
    pub(super) fn sel_of(&self, id: i64) -> Option<Sel> {
        let d = self.ed.doc.as_ref()?;
        for (k, t) in &d.tiles {
            if t.tile.objects.iter().any(|o| o.id == id) {
                return Some(Sel::Object(*k, id));
            }
            if t.tile.splines.iter().any(|s| s.id == id && !s.deleted) {
                return Some(Sel::Spline(*k, id));
            }
            if t.tile.spline_attachments.iter().any(|a| a.id == id) {
                return Some(Sel::Attached(*k, id));
            }
        }
        None
    }

    fn sels(&self, ids: &[i64]) -> Result<Vec<Sel>, String> {
        ids.iter().map(|id| self.sel_of(*id).ok_or_else(|| format!("no object or spline with IDCode {id}"))).collect()
    }

    fn need_doc(&mut self) -> Result<&mut Document, String> {
        self.ed.doc.as_mut().ok_or_else(|| "no map is open".to_string())
    }

    // ---------------------------------------------------------------------------- the editor's tools

    fn setting(&mut self, key: &str, v: &Value) -> Result<(), String> {
        let num = || v.as_f64().ok_or_else(|| format!("{key} is a number"));
        let flag = || v.as_bool().ok_or_else(|| format!("{key} is true or false"));
        let text = || v.as_str().map(str::to_string).ok_or_else(|| format!("{key} is a text"));
        let e = &mut self.ed;
        match key {
            "tool" => {
                let t = tool_of(&text()?)?;
                e.set_tool(t);
            }
            "terrain_mode" => e.terrain_mode = terrain_mode_of(&text()?)?,
            "logic_mode" | "logic" => {
                let m = logic_of(&text()?)?;
                e.set_logic(m);
            }
            "water_shape" => {
                e.water_shape = match text()?.as_str() {
                    "lake" => WaterShape::Lake,
                    "river" => WaterShape::River,
                    "brush" => WaterShape::Brush,
                    s => return Err(format!("no water shape {s}: lake, river, brush")),
                }
            }
            "road_type" => {
                let rel = self.resolve(&text()?, Some(true))?;
                self.wait_spline(&rel)?;
                self.ed.road_type = Some(rel);
            }
            "object_type" => {
                let rel = self.resolve(&text()?, Some(false))?;
                self.wait_object(&rel)?;
                self.ed.object_type = Some(rel);
            }
            "path_type" => {
                let rel = self.resolve(&text()?, Some(true))?;
                self.wait_spline(&rel)?;
                self.ed.set_path_type(rel);
            }
            "plant_mix" => {
                let list: Vec<String> = v.as_array().ok_or("plant_mix is a list of tree files")?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
                let mut mix = Vec::new();
                for f in list {
                    mix.push(self.resolve(&f, Some(false))?);
                }
                self.ed.plant_mix = mix;
            }
            "brush_radius" => e.brush_radius = num()?.clamp(0.5, 2000.0) as f32,
            "brush_strength" => e.brush_strength = num()?.max(0.0) as f32,
            "flatten_height" => e.flatten_fixed = v.as_f64().map(|h| h as f32),
            "paint_layer" => e.paint_layer = num()?.max(0.0) as usize,
            "paint_erase" => e.paint_erase = flag()?,
            "water_depth" => e.water_depth = num()?.max(0.1) as f32,
            "river_width" => e.river_width = num()?.max(1.0) as f32,
            "place_heading" => e.place_heading = num()?,
            "align_to_road" => e.align_to_road = flag()?,
            "road_setback" => e.road_setback = num()?,
            "auto_junctions" => e.auto_junctions = flag()?,
            "auto_terrain" => e.auto_terrain = flag()?,
            "auto_adapt" => e.auto_adapt = flag()?,
            "auto_switches" => e.auto_switches = flag()?,
            "follow_ground" => e.follow_ground = flag()?,
            "keep_ground_under_roads" => e.keep_ground_under_roads = flag()?,
            "snap_roads" => e.snap_roads = flag()?,
            "keep_drawing" => e.keep_drawing = flag()?,
            "inherit_type" => e.inherit_type = flag()?,
            "roundabout" => e.roundabout = flag()?,
            "bus_pocket" => e.bus_pocket = flag()?,
            "pocket_len" => e.pocket_len = num()?.max(1.0),
            "pocket_width" => e.pocket_width = num()?.max(0.5) as f32,
            "plant_brush" => e.plant_brush = flag()?,
            "plant_erase" => e.plant_erase = flag()?,
            "plant_area" => e.plant_area = flag()?,
            "plant_spacing" => e.plant_spacing = num()?.max(0.5),
            "stop_both_sides" => e.stop_both_sides = flag()?,
            "stop_kind" => e.stop_kind = stop_kind_of(&text()?),
            "parking_row" => e.parking_row = flag()?,
            "parking_step" => e.parking_step = num()?.max(1.0),
            "snap_grid" => e.snap_grid = flag()?,
            "grid_step" => e.grid_step = num()?.max(0.01),
            "snap_angle" => e.snap_angle = flag()?,
            "angle_step" => e.angle_step = num()?.max(0.1),
            "keep_height" => e.keep_height = flag()?,
            "straight_lock" => e.straight_lock = flag()?,
            "show_lanes" => e.show_lanes = flag()?,
            _ => return Err(format!("no setting {key} (see the tool's description)")),
        }
        Ok(())
    }

    pub fn set_editor(&mut self, a: Args) -> Done {
        let s = a.get("settings").and_then(|s| s.as_object()).cloned().or_else(|| a.0.as_object().cloned()).ok_or("`settings` is needed")?;
        // (the tool first: choosing it calls off what the last one did)
        if let Some(t) = s.get("tool") {
            self.setting("tool", t)?;
        }
        for (k, v) in s.iter().filter(|(k, _)| *k != "tool" && *k != "settings") {
            self.setting(k, v)?;
        }
        Ok(json!({ "tool": format!("{:?}", self.ed.tool).to_lowercase() }))
    }

    pub fn click(&mut self, a: Args) -> Done {
        let pts = self.points(a, "points")?;
        let shift = a.bool("shift").unwrap_or(false);
        self.ed.shift_held = shift;
        for p in &pts {
            self.click_at(*p, shift);
        }
        self.ed.shift_held = false;
        match a.str("finish").unwrap_or("none") {
            "confirm" => self.ed.confirm_road(&mut self.assets, &self.lib),
            "cancel" => {
                self.ed.right_click();
            }
            _ => {}
        }
        Ok(json!({ "clicks": pts.len(), "still_drawing": self.ed.road_start_active() }))
    }

    pub fn stroke(&mut self, a: Args) -> Done {
        let path = self.points(a, "path")?;
        let seconds = a.f64("seconds").unwrap_or(1.0).clamp(0.05, 600.0);
        self.hold(&path, seconds);
        Ok(json!({ "seconds": seconds }))
    }

    pub fn undo_redo(&mut self, a: Args, undo: bool) -> Done {
        let n = a.usize("steps").unwrap_or(1).clamp(1, 500);
        let mut done = 0;
        for _ in 0..n {
            let can = if undo { self.ed.history.can_undo() } else { self.ed.history.can_redo() };
            if !can {
                break;
            }
            if undo {
                self.ed.undo();
            } else {
                self.ed.redo();
            }
            done += 1;
        }
        let (u, r) = self.ed.history.steps();
        Ok(json!({ if undo { "undone" } else { "redone" }: done, "next_undo": u.last(), "next_redo": r.first() }))
    }

    pub fn command(&mut self, a: Args) -> Done {
        let name = a.str("name").ok_or("`name` is needed")?.to_lowercase();
        // (commands that need the selected objects' models: the models read first, as the
        // window has them by the time a person clicks)
        if matches!(name.as_str(), "level_under_objects" | "lay_on_ground") {
            let files: std::collections::BTreeSet<String> = self.ed.selection().iter().filter_map(|s| match *s {
                Sel::Object(k, id) => self.ed.doc.as_ref().and_then(|d| d.object(k, id)).map(|o| o.file.clone()),
                _ => None,
            }).collect();
            for f in files {
                let _ = self.wait_object(&f);
            }
        }
        let empty = json!({});
        let c = Args(a.get("args").unwrap_or(&empty));
        let lib = &self.lib;
        let e = &mut self.ed;
        let mut said = Value::Null;
        match name.as_str() {
            "undo" => e.undo(),
            "redo" => e.redo(),
            "cancel" => {
                e.right_click();
                e.cancel();
            }
            "confirm" => e.confirm_road(&mut self.assets, lib),
            "delete" => e.delete_selected(),
            "copy" => said = json!(e.copy(lib)),
            "cut" => e.cut(lib),
            "paste" => {
                if let Some(at) = c.get("at") {
                    let p = self.point(at)?;
                    self.hover(p);
                }
                self.ed.paste(&self.lib);
            }
            "duplicate" => e.duplicate(lib),
            "select_all" => e.select_all(),
            "select_none" => e.select(None),
            "rotate" => {
                let t = e.tool;
                e.set_tool(Tool::Select);
                e.rotate(c.f64("degrees").or(c.f64("deg")).ok_or("`degrees` is needed")?);
                if t != Tool::Select {
                    e.tool = t;
                }
            }
            "move" => e.move_selection_by(DVec2::new(c.f64("dx").unwrap_or(0.0), c.f64("dy").unwrap_or(0.0))),
            "raise" => e.raise_selection(c.f64("dz").ok_or("`dz` is needed")?),
            "reverse" => e.reverse_selected(),
            "lay_on_ground" => e.lay_selected_on_ground(),
            "even_chain" => e.even_selected_chain(),
            "complete_to" => e.complete_to(),
            "continue_spline" => said = json!(e.continue_spline()),
            "parallel_copy" => e.parallel_copy(c.f64("distance").ok_or("`distance` is needed (m, + right)")?),
            "level_under_objects" => said = json!(e.level_under_objects(&mut self.assets)),
            "align_to_roads" => said = json!(e.align_to_roads()),
            "turn_objects" => e.turn_objects(c.bool("random").unwrap_or(true)),
            "lean_objects" => e.lean_objects(c.bool("lean").unwrap_or(true)),
            "replace_objects" | "retype_splines" | "wire_along" => {
                let spline = name != "replace_objects";
                let f = self.resolve(c.str("file").ok_or("`file` is needed")?, Some(spline))?;
                match name.as_str() {
                    "replace_objects" => self.ed.replace_selected_objects(&f),
                    "retype_splines" => self.ed.retype_splines(&f),
                    _ => said = json!(self.ed.wire_along(&f)),
                }
            }
            "replace_everywhere" => {
                let splines = c.bool("splines").unwrap_or(false);
                let from = c.str("from").ok_or("`from` is needed")?.replace('/', "\\");
                let to = self.resolve(c.str("to").ok_or("`to` is needed")?, Some(splines))?;
                said = json!(self.ed.replace_everywhere(&from, &to, splines));
            }
            "cut_spline" => {
                let id = c.i64("id").ok_or("`id` is needed")?;
                let Some(Sel::Spline(k, id)) = self.sel_of(id) else { return Err(format!("no spline {id}")) };
                self.ed.cut_spline(k, id, c.f64("at").ok_or("`at` is needed (m along it)")?);
            }
            "join_with_next" => {
                let id = c.i64("id").ok_or("`id` is needed")?;
                let Some(Sel::Spline(k, id)) = self.sel_of(id) else { return Err(format!("no spline {id}")) };
                self.ed.join_with_next(k, id);
            }
            "link_ends" => {
                // two free spline ends lying on one another (any angle) linked, as the map check's "Join"
                let (a, b) = (c.i64("a").ok_or("`a` is needed")?, c.i64("b").ok_or("`b` is needed")?);
                let (Some(Sel::Spline(ka, a)), Some(Sel::Spline(kb, b))) = (self.sel_of(a), self.sel_of(b)) else { return Err("`a` and `b` must be splines".into()) };
                let doc = self.ed.doc.as_ref().ok_or("no map")?;
                let ends = |k: TileKey, id: i64| -> Option<[DVec3; 2]> {
                    let s = doc.spline(k, id)?;
                    let cv = omsi_geometry::SplineCurve::from_map(s, tile_origin(k));
                    Some([cv.point_at(0.0), cv.point_at(cv.length)])
                };
                let (ea, eb) = (ends(ka, a).ok_or("no spline a")?, ends(kb, b).ok_or("no spline b")?);
                let mut best: Option<(f64, usize, usize)> = None;
                for (i, p) in ea.iter().enumerate() {
                    for (j, q) in eb.iter().enumerate() {
                        let d = (*p - *q).truncate().length();
                        if best.map_or(true, |x| d < x.0) {
                            best = Some((d, i, j));
                        }
                    }
                }
                let (d, i, j) = best.unwrap();
                if d > 0.5 {
                    return Err(format!("the nearest ends are {d:.2} m apart (at most 0.5 m)"));
                }
                said = json!(crate::check::apply(&mut self.ed, &crate::check::Fix::Link((ka, a, i == 1), (kb, b, j == 1))));
            }
            "paint_along" => {
                e.paint_along = crate::roadpaint::Along { layer: c.usize("layer").ok_or("`layer` is needed")?, width: c.f64("width").unwrap_or(4.0) as _, under: c.bool("under").unwrap_or(false), ..Default::default() };
                said = json!(e.paint_along_selected(lib));
            }
            "array" => {
                let v2 = |k: &str| c.get(k).and_then(|v| v.as_array()).map(|a| DVec2::new(a.first().and_then(|x| x.as_f64()).unwrap_or(0.0), a.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0)));
                e.array(lib, c.usize("count").unwrap_or(2).clamp(1, 1000), v2("step").unwrap_or(DVec2::new(10.0, 0.0)), c.f64("turn").unwrap_or(0.0), c.usize("rows").unwrap_or(1).clamp(1, 1000), v2("row_step").unwrap_or(DVec2::new(0.0, 10.0)));
            }
            "group" => e.group_selection(),
            "ungroup" => e.ungroup_selection(),
            "remake_junctions" => said = json!(e.remake_all_junctions()),
            "flatten_terrain" => said = json!(e.flatten_terrain()),
            "grow_map" => said = json!(e.grow_map()),
            "align_ground" => {
                let which: Vec<(TileKey, i64)> = e.selection().into_iter().filter_map(|s| if let Sel::Spline(k, id) = s { Some((k, id)) } else { None }).collect();
                said = json!(e.align_ground_to(&which));
            }
            "place_rows" => e.place_rows(),
            "detach_lanes" => e.detach_lanes(&mut self.assets, &self.roots),
            _ => return Err(format!("no command {name} (see the tool's description)")),
        }
        Ok(json!({ "command": name, "result": said, "selection": self.ed.selection().len() }))
    }

    /// What lies within `radius` of `c` (objects by their place, splines by their line).
    fn near(&self, c: DVec2, radius: f64, objects: bool, splines: bool, file: Option<&str>) -> Vec<Sel> {
        let Some(d) = self.ed.doc.as_ref() else { return Vec::new() };
        let ok = |f: &str| file.map(|w| f.to_lowercase().contains(&w.to_lowercase().replace('/', "\\"))).unwrap_or(true);
        let mut out = Vec::new();
        for (k, t) in &d.tiles {
            let org = tile_origin(*k);
            if objects {
                out.extend(t.tile.objects.iter().filter(|o| ok(&o.file) && (org + DVec2::new(o.pos[0], o.pos[1]) - c).length() <= radius).map(|o| Sel::Object(*k, o.id)));
            }
            if splines {
                for s in t.tile.splines.iter().filter(|s| !s.deleted && ok(&s.file)) {
                    if (org + DVec2::new(s.pos[0], s.pos[1]) - c).length() > radius + s.length {
                        continue;
                    }
                    let curve = omsi_geometry::SplineCurve::from_map(s, org);
                    let n = ((s.length / 2.0).ceil() as usize).clamp(2, 500);
                    if (0..=n).any(|i| (curve.point_at(s.length * i as f64 / n as f64).truncate() - c).length() <= radius) {
                        out.push(Sel::Spline(*k, s.id));
                    }
                }
            }
        }
        out
    }

    /// Every object and spline of the map (of the kinds asked) whose file contains `file`.
    fn matching(&self, objects: bool, splines: bool, file: Option<&str>) -> Vec<Sel> {
        let Some(d) = self.ed.doc.as_ref() else { return Vec::new() };
        let ok = |f: &str| file.map(|w| f.to_lowercase().contains(&w.to_lowercase().replace('/', "\\"))).unwrap_or(true);
        let mut out = Vec::new();
        for (k, t) in &d.tiles {
            if objects {
                out.extend(t.tile.objects.iter().filter(|o| ok(&o.file)).map(|o| Sel::Object(*k, o.id)));
            }
            if splines {
                out.extend(t.tile.splines.iter().filter(|s| !s.deleted && ok(&s.file)).map(|s| Sel::Spline(*k, s.id)));
            }
        }
        out
    }

    pub fn select(&mut self, a: Args) -> Reply {
        self.doc()?;
        let add = a.bool("add").unwrap_or(false);
        let mut v: Vec<Sel> = if add { self.ed.selection() } else { Vec::new() };
        if a.bool("none").unwrap_or(false) {
            self.ed.select(None);
            return Ok(json!({ "selected": 0 }).into());
        }
        if a.bool("all").unwrap_or(false) {
            let kinds = a.strings("kinds");
            let file = a.str("file");
            // (everything, unless `kinds` or `file` narrow it: then all that matches them)
            if kinds.is_empty() && file.is_none() && !add {
                self.ed.select_all();
                return Ok(json!({ "selected": self.ed.selection().len() }).into());
            }
            let (o, s) = if kinds.is_empty() { (true, true) } else { (kinds.iter().any(|k| k == "objects"), kinds.iter().any(|k| k == "splines")) };
            v.extend(self.matching(o, s, file));
            v.dedup();
            self.ed.set_selection(v);
            return Ok(json!({ "selected": self.ed.selection().len() }).into());
        }
        if let Some(id) = a.i64("alike") {
            let s = self.sel_of(id).ok_or(format!("no object or spline {id}"))?;
            let n = self.ed.select_alike(s);
            return Ok(json!({ "selected": n }).into());
        }
        if let Some(id) = a.i64("chain") {
            let Some(Sel::Spline(k, id)) = self.sel_of(id) else { return Err(format!("no spline {id}")) };
            self.ed.select_chain(k, id);
            return Ok(json!({ "selected": self.ed.selection().len() }).into());
        }
        v.extend(self.sels(&a.ids("ids"))?);
        if let Some(at) = a.get("at") {
            let c = self.point(at)?;
            let kinds = a.strings("kinds");
            let (o, s) = if kinds.is_empty() { (true, true) } else { (kinds.iter().any(|k| k == "objects"), kinds.iter().any(|k| k == "splines")) };
            v.extend(self.near(c, a.f64("radius").unwrap_or(10.0), o, s, a.str("file")));
        }
        v.dedup();
        self.ed.set_selection(v);
        let sel: Vec<i64> = self.ed.selection().into_iter().map(|s| match s {
            Sel::Object(_, id) | Sel::Spline(_, id) | Sel::Attached(_, id) => id,
        }).collect();
        Ok(json!({ "selected": sel.len(), "ids": sel.iter().take(500).collect::<Vec<_>>() }).into())
    }

    // ---------------------------------------------------------------------------- roads

    /// The road tool's switches for one call; gives back what they were.
    fn road_options(&mut self, a: Args) -> Result<Vec<(String, Value)>, String> {
        let mut old = Vec::new();
        if let Some(o) = a.get("options").and_then(|o| o.as_object()) {
            for (k, v) in o {
                let was = match k.as_str() {
                    "auto_junctions" => self.ed.auto_junctions,
                    "auto_terrain" => self.ed.auto_terrain,
                    "follow_ground" => self.ed.follow_ground,
                    "keep_ground_under_roads" => self.ed.keep_ground_under_roads,
                    "auto_adapt" => self.ed.auto_adapt,
                    "auto_switches" => self.ed.auto_switches,
                    "snap_roads" => self.ed.snap_roads,
                    "inherit_type" => self.ed.inherit_type,
                    _ => {
                        self.restore(old);
                        return Err(format!("no road option {k}"));
                    }
                };
                old.push((k.clone(), json!(was)));
                // (a bad value puts back what this call set before it: else it stays for the calls after)
                if let Err(e) = self.setting(k, v) {
                    self.restore(old);
                    return Err(e);
                }
            }
        }
        Ok(old)
    }

    fn restore(&mut self, old: Vec<(String, Value)>) {
        for (k, v) in old {
            let _ = self.setting(&k, &v);
        }
    }

    fn road_type(&mut self, a: Args) -> Result<String, String> {
        let rel = match a.str("type").or(a.str("file")) {
            Some(t) => self.resolve(t, Some(true))?,
            None => self.ed.road_type.clone().ok_or("`type` is needed: a road type from the catalogue")?,
        };
        self.wait_spline(&rel)?;
        Ok(rel)
    }

    /// `auto_junctions`: Auto-Junction over the roads already on the map (see
    /// `Editor::auto_junctions_on_map`), in `center` + `radius` or everywhere.
    pub fn auto_junctions(&mut self, a: Args) -> Done {
        let area = match (a.get("center"), a.f64("radius")) {
            (Some(c), Some(r)) => Some((self.point(c)?, r)),
            _ => None,
        };
        let (roads, made) = self.ed.auto_junctions_on_map(&mut self.assets, area);
        Ok(json!({ "roads": roads, "junctions": made }))
    }

    pub fn build_streets(&mut self, a: Args) -> Done {
        let path = a.str("file").ok_or("`file` is needed (an Overpass JSON answer)")?;
        let body = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let g = self.georef().ok_or("the map is not placed on the Earth: no latitude / longitude")?;
        let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let mut ways = Vec::new();
        for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
            if e.get("type").and_then(|t| t.as_str()) != Some("way") {
                continue;
            }
            let tags: std::collections::HashMap<String, String> = e.get("tags").and_then(|t| t.as_object()).map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
            if !tags.contains_key("highway") {
                continue;
            }
            let nodes: Vec<i64> = e.get("nodes").and_then(|n| n.as_array()).into_iter().flatten().filter_map(|n| n.as_i64()).collect();
            let pts: Vec<DVec2> = e.get("geometry").and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some(g.world(p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect();
            if nodes.len() == pts.len() && pts.len() >= 2 {
                ways.push(crate::street_model::RawWay { id: e.get("id").and_then(|i| i.as_i64()).unwrap_or(0), tags, nodes, pts });
            }
        }
        let mut net = crate::street_model::network(&ways);
        // the 2018 (or any dated) imagery: widths and offsets of the carriageways measured on it
        let mut measured = 0usize;
        if let Some(im) = a.get("imagery") {
            let url = im.get("url").and_then(|u| u.as_str()).ok_or("imagery.url is needed")?.to_string();
            // (as `shot` takes it: a zoom past the tiles' 2^z would overflow)
            let z = im.get("z").and_then(|z| z.as_u64()).unwrap_or(19).clamp(1, 22) as u8;
            let src = crate::geo::sources::Source { name: format!("street-model {}", url.len()), url, attribution: String::new(), max_zoom: z, kind: Default::default(), needs_key: false, key_link: String::new(), bounds: None, resolution: 0.0, tile_px: 256, ortho: false, builtin: false };
            let prefs = crate::geo::sources::GeoPrefs::default();
            let doc = self.ed.doc.as_ref().ok_or("no map")?;
            let ts = crate::doc::tile_size();
            let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
            for k in doc.tiles.keys() {
                for c in [tile_origin(*k), tile_origin(*k) + DVec2::splat(ts)] {
                    let m = g.merc(c);
                    lo = lo.min(m);
                    hi = hi.max(m);
                }
            }
            let fetch = |z: u8, x: u32, y: u32| crate::geo::fetch::tile(&src, &prefs, z, x, y);
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let mosaic = crate::geo::imagery::Mosaic::fetch(z, lo, hi, false, &fetch, &cancel).map_err(|e| format!("imagery: {e}"))?;
            let inside = |p: DVec2| doc.tiles.contains_key(&crate::doc::tile_of(p));
            let colour = |p: DVec2| -> Option<[u8; 3]> { inside(p).then(|| mosaic.colour(g.merc(p))) };
            measured = crate::street_model::apply_imagery(&mut net, &colour).iter().filter(|m| m.is_some()).count();
        }
        crate::street_model::smooth_joints(&mut net);
        net.settle_paths();
        if a.bool("model_only").unwrap_or(false) {
            return Ok(json!({ "model": crate::street_model::net_json(&net), "imagery_measured": measured }));
        }
        let t = a.get("types").ok_or("`types` is needed")?;
        let mut choices = Vec::new();
        for c in t.get("choices").and_then(|c| c.as_array()).into_iter().flatten() {
            let f = c.get("file").and_then(|f| f.as_str()).ok_or("choices[].file is needed")?;
            choices.push(crate::street_model::TypeChoice {
                file: self.resolve(f, Some(true))?,
                width: c.get("width").and_then(|w| w.as_f64()).ok_or("choices[].width is needed")?,
                oneway: c.get("oneway").and_then(|w| w.as_bool()).unwrap_or(false),
                sidewalks: c.get("sidewalks").and_then(|w| w.as_bool()).unwrap_or(true),
                classes: c.get("classes").and_then(|w| w.as_array()).map(|v| v.iter().filter_map(|x| x.as_u64().map(|x| x as u8)).collect()).unwrap_or(vec![0, 1, 2]),
            });
        }
        // hardstanding: the ground texture laid under it, the car parks of the OSM answer and
        // the paved areas given
        let apron = match t.get("apron").and_then(|v| v.as_str()) {
            Some(tex) => Some(self.ground_texture(tex)?),
            None => None,
        };
        let mut paved: Vec<Vec<DVec2>> = Vec::new();
        for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
            let tags = e.get("tags");
            let tag = |k: &str| tags.and_then(|t| t.get(k)).and_then(|x| x.as_str()).unwrap_or("");
            let parking = tag("amenity") == "parking" && !matches!(tag("parking"), "underground" | "multi-storey" | "rooftop");
            if !(parking || tag("area:highway") != "" || tag("landuse") == "garages") {
                continue;
            }
            let ring: Vec<DVec2> = e.get("geometry").and_then(|g| g.as_array()).into_iter().flatten().filter_map(|p| Some(g.world(p.get("lat")?.as_f64()?, p.get("lon")?.as_f64()?))).collect();
            if ring.len() >= 4 {
                paved.push(ring);
            }
        }
        for area in a.arr("paved").into_iter().flatten() {
            let ring: Vec<DVec2> = area.as_array().ok_or("paved: a list of outlines")?.iter().map(|q| self.point(q)).collect::<Result<_, _>>()?;
            if ring.len() >= 3 {
                paved.push(ring);
            }
        }
        let mut pick = |k: &str, i: usize| -> Result<String, String> {
            let f = t.get(k).and_then(|x| if x.is_array() { x.get(i) } else { Some(x) }).and_then(|x| x.as_str()).ok_or(format!("types.{k} is needed"))?;
            self.resolve(f, Some(true))
        };
        let types = crate::editor::StreetTypes {
            road: [(pick("main", 0)?, pick("main", 1)?), (pick("street", 0)?, pick("street", 1)?), (pick("service", 0)?, pick("service", 1)?)],
            path: pick("path", 0)?,
            choices,
            make: match (t.get("make_types").and_then(|v| v.get("roadway")).and_then(|v| v.as_str()), t.get("make_types").and_then(|v| v.get("paving")).and_then(|v| v.as_str())) {
                (Some(r), Some(p)) => Some((std::path::PathBuf::from(r), std::path::PathBuf::from(p))),
                _ => None,
            },
            apron,
            paved,
            // (`heights`: a JSON file of the designed heights, [[[x, y, z], ...], ...], one line
            // of points per way)
            heights: match a.str("heights") {
                Some(f) => {
                    let body = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
                    // ([[x, y, z], ...] per way, or {"<osm way id>": [[x, y, z], ...], ...})
                    let v: Value = serde_json::from_str(&body).map_err(|e| format!("heights: {e}"))?;
                    let pts = |l: &Value| -> Vec<[f64; 3]> { l.as_array().into_iter().flatten().filter_map(|q| Some([q.get(0)?.as_f64()?, q.get(1)?.as_f64()?, q.get(2)?.as_f64()?])).collect() };
                    let lines: Vec<(i64, Vec<[f64; 3]>)> = match &v {
                        Value::Object(m) => m.iter().map(|(k, l)| (k.parse().unwrap_or(0), pts(l))).collect(),
                        Value::Array(a) => a.iter().map(|l| (0, pts(l))).collect(),
                        _ => return Err("heights: a list of lines or an object of ways".into()),
                    };
                    Some(std::sync::Arc::new(crate::street_model::Heights::of_ways(&lines)))
                }
                None => None,
            },
        };
        // (a dry run: the model only, for looking at it)
        if a.bool("dry_run").unwrap_or(false) {
            let mut deg = vec![0usize; net.path_nodes.len()];
            for (_, ab) in &net.path_edges {
                deg[ab[0]] += 1;
                deg[ab[1]] += 1;
            }
            return Ok(json!({
                "streets": net.streets.iter().map(|s| json!({"class": s.class, "name": s.name, "width": number(s.width), "median": number(s.median), "oneway": s.oneway, "sidewalk": s.sidewalk, "ends": s.ends, "axis": s.axis.iter().map(|p| p2(*p)).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "nodes": net.nodes.iter().map(|p| p2(*p)).collect::<Vec<_>>(),
                "path_nodes": net.path_nodes.iter().zip(&deg).map(|(p, d)| json!({"at": p2(*p), "degree": d})).collect::<Vec<_>>(),
                "path_edges": net.path_edges.iter().map(|(pts, ab)| json!({"ends": ab, "points": pts.iter().map(|p| p2(*p)).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            }));
        }
        let rep = self.ed.build_streets(&mut self.assets, &net, &types);
        let crossings: Vec<Value> = net.crossings.iter().map(|(si, p)| {
            let ax = &net.streets[*si].axis;
            let k = ax.windows(2).enumerate().min_by(|x, y| {
                let d = |w: &[DVec2]| { let m = (w[0] + w[1]) * 0.5; (m - *p).length() };
                d(x.1).total_cmp(&d(y.1))
            }).map(|x| x.0).unwrap_or(0);
            json!({ "at": p2(*p), "street_heading": number(crate::roads::heading_of(ax[k + 1] - ax[k])), "class": net.streets[*si].class })
        }).collect();
        Ok(json!({ "report": rep, "model": { "measured": measured, "widths": net.streets.iter().map(|s| number(s.width)).collect::<Vec<_>>(), "streets": net.streets.len(), "paths": net.paths.len(), "sidewalks": net.sidewalks.len(), "divided": net.streets.iter().filter(|s| s.median > 0.0).count() }, "crossings": crossings, "unsaved": true }))
    }

    pub fn draw_road(&mut self, a: Args) -> Done {
        let pts = self.points(a, "points")?;
        if pts.len() < 2 {
            return Err("a road needs two points or more".into());
        }
        let rel = self.road_type(a)?;
        let old = self.road_options(a)?;
        self.ed.set_tool(Tool::Roads);
        self.ed.roundabout = false;
        self.ed.bus_pocket = false;
        self.ed.road_type = Some(rel.clone());
        // `shape: "osm"`: laid as the OpenStreetMap import lays a way: straights, its corners
        // rounded into arcs (`corner` radius in metres, else the class's: 10 m service, 20 m
        // street, 60 m main road), the ground following it with Auto-Terrain on
        if a.str("shape") == Some("osm") {
            let class = match a.str("class").unwrap_or("service") {
                "main" => crate::osm::Class::Main,
                "street" => crate::osm::Class::Street,
                "rail" => crate::osm::Class::Rail,
                "tram" => crate::osm::Class::Tram,
                _ => crate::osm::Class::Service,
            };
            let road = crate::osm::Road { class, oneway: false, points: pts.clone() };
            let keep = std::mem::replace(&mut self.ed.osm_keep_overlaps, a.bool("keep_overlaps").unwrap_or(true));
            let (n, j) = self.ed.build_osm(&mut self.assets, &[road], &[(class, rel.clone())], None);
            self.ed.osm_keep_overlaps = keep;
            self.restore(old);
            return Ok(json!({ "type": rel, "built": n, "junctions": j }));
        }
        let keep = std::mem::replace(&mut self.ed.keep_drawing, true);
        if a.bool("separate").unwrap_or(false) {
            for w in pts.windows(2) {
                self.click_at(w[0], false);
                self.click_at(w[1], false);
                self.ed.right_click();
            }
        } else {
            for p in &pts {
                self.click_at(*p, false);
            }
            if a.bool("closed").unwrap_or(false) {
                self.click_at(pts[0], false);
            }
            self.ed.right_click();
        }
        self.ed.keep_drawing = keep;
        self.restore(old);
        Ok(json!({ "type": rel }))
    }

    pub fn roundabout(&mut self, a: Args) -> Done {
        let c = self.point(a.get("center").or(a.get("at")).ok_or("`center` is needed")?)?;
        let r = a.f64("radius").ok_or("`radius` is needed")?.max(crate::roundabout::MIN_RADIUS);
        let rel = self.road_type(a)?;
        self.ed.set_tool(Tool::Roads);
        self.ed.road_type = Some(rel.clone());
        let z = self.ground(c);
        self.ed.build_roundabout(&mut self.assets, DVec3::new(c.x, c.y, z), r);
        Ok(json!({ "type": rel, "radius": number(r) }))
    }

    pub fn bus_pocket(&mut self, a: Args) -> Done {
        let p = self.point(a.get("at").ok_or("`at` is needed")?)?;
        self.ed.set_tool(Tool::Roads);
        if let Some(t) = a.str("type") {
            let rel = self.resolve(t, Some(true))?;
            self.wait_spline(&rel)?;
            self.ed.pocket_type = Some(rel);
        }
        if let Some(l) = a.f64("length") {
            self.ed.pocket_len = l.max(1.0);
        }
        if let Some(w) = a.f64("width") {
            self.ed.pocket_width = w.max(0.5) as f32;
        }
        self.ed.bus_pocket = true;
        self.hover(p);
        self.ed.pocket_click(&mut self.assets);
        self.ed.bus_pocket = false;
        Ok(json!({}))
    }

    pub fn ai_path(&mut self, a: Args) -> Done {
        let pts = self.points(a, "points")?;
        if pts.len() < 2 {
            return Err("a path needs two points or more".into());
        }
        let rel = match a.str("type") {
            Some(t) => self.resolve(t, Some(true))?,
            None => self.ed.path_type.clone().or_else(|| self.lib.find("Splines\\Marcel\\invis_street.sli").map(|a| a.rel.clone())).or_else(|| self.lib.assets.iter().find(|a| a.category == Category::Technical && a.kind == Kind::Spline && a.lanes.0 > 0).map(|a| a.rel.clone())).ok_or("`type` is needed: an AI path spline (catalogue category technical)")?,
        };
        self.wait_spline(&rel)?;
        self.ed.set_tool(Tool::Logic);
        self.ed.set_logic(LogicMode::Paths);
        self.ed.set_path_type(rel.clone());
        for p in &pts {
            self.click_at(*p, false);
        }
        self.ed.right_click();
        Ok(json!({ "type": rel }))
    }

    // ---------------------------------------------------------------------------- objects

    /// The objects put on tile `k` (and its neighbours) with an IDCode from `first` on.
    fn new_objects_near(&self, k: TileKey, first: i64) -> Vec<(TileKey, i64)> {
        let Some(d) = self.ed.doc.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let kk = (k.0 + dx, k.1 + dy);
                if let Some(t) = d.tiles.get(&kk) {
                    out.extend(t.tile.objects.iter().filter(|o| o.id >= first).map(|o| (kk, o.id)));
                }
            }
        }
        out
    }

    /// Change objects directly, as one undo step.
    fn change_objects(&mut self, which: &[(TileKey, i64)], label: &str, mut f: impl FnMut(&mut omsi_map::MapObject)) {
        let Some(doc) = self.ed.doc.as_mut() else { return };
        self.ed.history.begin(label);
        for (k, id) in which {
            self.ed.history.touch_tile(doc, *k);
            if let Some(t) = doc.tiles.get_mut(k) {
                if let Some(o) = t.tile.objects.iter_mut().find(|o| o.id == *id) {
                    f(o);
                }
                t.touch_content();
            }
        }
        self.ed.history.commit();
    }

    pub fn place_objects(&mut self, a: Args) -> Done {
        let items = a.arr("items").ok_or("`items` is needed")?.clone();
        if items.len() > 5000 {
            return Err("at most 5000 at once".into());
        }
        let default = a.str("file").map(str::to_string);
        let mut files: HashMap<String, String> = HashMap::new();
        let mark = self.ed.history.mark();
        self.ed.set_tool(Tool::Objects);
        self.ed.plant_brush = false;
        self.ed.poly_draw = false;
        let (snap, keep_align) = (self.ed.snap_grid, self.ed.align_to_road);
        self.ed.snap_grid = false;
        let mut placed = 0;
        let mut problems = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let it = Args(it);
            let r: Result<(), String> = (|| {
                let file = it.str("file").map(str::to_string).or(default.clone()).ok_or("no `file`")?;
                // `scale`: [x, y, z] along the object's own axes (x across, y up, z along), made
                // as a scaled copy of its type as the Scale handles do
                // (the copies are made in the editor's frame, as the Scale handles give them:
                // x across, y along, z up; the call names them as OMSI does: x, y up, z along)
                let scale = it.get("scale").and_then(|v| v.as_array()).filter(|a| a.len() == 3).map(|a| DVec3::new(a[0].as_f64().unwrap_or(1.0), a[2].as_f64().unwrap_or(1.0), a[1].as_f64().unwrap_or(1.0)));
                let key = match scale {
                    Some(s) => format!("{file}|{:.3}_{:.3}_{:.3}", s.x, s.y, s.z),
                    None => file.clone(),
                };
                let rel = match files.get(&key) {
                    Some(r) => r.clone(),
                    None => {
                        let mut r = self.resolve(&file, Some(false))?;
                        if let Some(s) = scale {
                            r = crate::scaled::variant(&self.roots, &r, s).map_err(|e| e.to_string())?;
                        }
                        self.wait_object(&r)?;
                        files.insert(key, r.clone());
                        r
                    }
                };
                let p = self.point(it.get("at").ok_or("no `at`")?)?;
                let k = tile_of(p);
                if !self.doc()?.tiles.contains_key(&k) {
                    return Err(format!("the map has no tile {}, {} there", k.0, k.1));
                }
                let align = it.bool("align_to_road").unwrap_or(false);
                self.ed.object_type = Some(rel);
                self.ed.align_to_road = align;
                self.ed.place_heading = it.f64("heading").unwrap_or(0.0);
                let first = self.doc()?.global.next_id_code;
                self.click_at(p, false);
                let made = self.new_objects_near(k, first);
                if made.is_empty() {
                    return Err("not placed".into());
                }
                let (height, heading, labels) = (it.f64("height"), it.f64("heading").filter(|_| !align), it.get("labels").and_then(|l| l.as_array()).map(|l| l.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<String>>()));
                if height.is_some() || heading.is_some() || labels.is_some() {
                    self.change_objects(&made, "Place objects", |o| {
                        if let Some(h) = height {
                            o.pos[2] = h;
                        }
                        if let Some(h) = heading {
                            o.rot[0] = h;
                        }
                        if let Some(l) = &labels {
                            o.extra = l.clone();
                            o.flag = o.flag.max(l.len() as i32);
                        }
                    });
                }
                placed += made.len();
                Ok(())
            })();
            if let Err(e) = r {
                problems.push(json!({ "item": i, "error": e }));
            }
        }
        self.ed.snap_grid = snap;
        self.ed.align_to_road = keep_align;
        self.ed.history.merge_since(mark, "Place objects");
        let mut v = json!({ "placed": placed });
        if !problems.is_empty() {
            v["problems"] = json!(problems);
        }
        Ok(v)
    }

    pub fn bus_stop(&mut self, a: Args) -> Done {
        let p = self.point(a.get("at").ok_or("`at` is needed")?)?;
        let name = a.str("name").ok_or("`name` is needed")?.to_string();
        let rel = match a.str("file") {
            Some(f) => self.resolve(f, Some(false))?,
            None => self.lib.find("Sceneryobjects\\Generic\\bus_stop.sco").map(|x| x.rel.clone()).or_else(|| self.first_asset(|x| x.is_bus_stop)).ok_or("no bus stop in the catalogue")?,
        };
        self.wait_object(&rel)?;
        let kind = stop_kind_of(a.str("kind").unwrap_or("regular"));
        let mark = self.ed.history.mark();
        self.ed.set_tool(Tool::Logic);
        self.ed.set_logic(LogicMode::Stops);
        self.ed.stop_both_sides = a.bool("both_sides").unwrap_or(true);
        self.ed.stop_kind = kind;
        let keep = std::mem::replace(&mut self.ed.align_to_road, true);
        self.ed.object_type = Some(rel);
        let first = self.doc()?.global.next_id_code;
        self.click_at(p, false);
        self.ed.align_to_road = keep;
        let made = self.new_objects_near(tile_of(p), first);
        if made.is_empty() {
            return Err("no stop was placed (is there a road with lanes within 12 m?)".into());
        }
        self.change_objects(&made, "Bus stop", |o| {
            let mut s = crate::stops::StopSettings::read(&o.extra);
            s.name = name.clone();
            s.set_kind(kind);
            s.write(&mut o.extra);
            o.flag = o.flag.max(7);
        });
        self.ed.history.merge_since(mark, "Bus stop");
        Ok(json!({ "stops": made.iter().map(|m| m.1).collect::<Vec<_>>(), "name": name }))
    }

    pub fn start_point(&mut self, a: Args) -> Done {
        let p = self.point(a.get("at").ok_or("`at` is needed")?)?;
        let rel = self.lib.find("Sceneryobjects\\Generic\\entrypoint_bus.sco").map(|x| x.rel.clone()).or_else(|| self.first_asset(|x| x.is_entry_point && x.rel.to_lowercase().contains("bus"))).or_else(|| self.first_asset(|x| x.is_entry_point)).ok_or("no start point object in the catalogue")?;
        self.wait_object(&rel)?;
        let mark = self.ed.history.mark();
        self.ed.set_tool(Tool::Logic);
        self.ed.set_logic(LogicMode::Start);
        let heading = a.f64("heading");
        let keep = std::mem::replace(&mut self.ed.align_to_road, heading.is_none());
        self.ed.place_heading = heading.unwrap_or(0.0);
        self.ed.object_type = Some(rel);
        let first = self.doc()?.global.next_id_code;
        self.click_at(p, false);
        self.ed.align_to_road = keep;
        let made = self.new_objects_near(tile_of(p), first);
        let (k, id) = *made.first().ok_or("the start point was not placed")?;
        if let Some(h) = heading {
            self.change_objects(&[(k, id)], "Start point", |o| o.rot[0] = h);
        }
        let doc = self.need_doc()?;
        if let Some(name) = a.str("name") {
            if let Some(e) = doc.global.entry_points.iter_mut().find(|e| e.object_id == id) {
                e.name = name.to_string();
            }
        }
        crate::editor::sync_entry_points(doc);
        self.ed.history.merge_since(mark, "Start point");
        Ok(json!({ "object": id }))
    }

    pub fn edit_object(&mut self, a: Args) -> Done {
        let id = a.i64("id").ok_or("`id` is needed")?;
        let Some(Sel::Object(k, id)) = self.sel_of(id) else { return Err(format!("no object {id}")) };
        let mark = self.ed.history.mark();
        if let Some(f) = a.str("file") {
            let rel = self.resolve(f, Some(false))?;
            self.ed.set_object_file(k, id, &rel);
        }
        if let Some(at) = a.get("at") {
            let p = self.point(at)?;
            self.ed.put_object(k, id, p);
            self.ed.history.commit();
        }
        // (it may stand on another tile now)
        let Some(Sel::Object(k, id)) = self.sel_of(id) else { return Err("the object is gone".into()) };
        if let Some(h) = a.f64("heading") {
            let now = self.ed.doc.as_ref().and_then(|d| d.object(k, id)).map(|o| o.rot[0]).unwrap_or(0.0);
            self.ed.set_selection(vec![Sel::Object(k, id)]);
            let t = self.ed.tool;
            self.ed.set_tool(Tool::Select);
            self.ed.rotate(crate::roads::norm_deg(h - now));
            self.ed.set_tool(t);
            self.ed.history.commit();
        }
        let labels = a.get("labels").and_then(|l| l.as_array()).map(|l| l.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<String>>());
        let (height, stop_name, stop_kind) = (a.f64("height"), a.str("stop_name").map(str::to_string), a.str("stop_kind").map(stop_kind_of));
        if height.is_some() || labels.is_some() || stop_name.is_some() || stop_kind.is_some() {
            self.change_objects(&[(k, id)], "Edit object", |o| {
                if let Some(h) = height {
                    o.pos[2] = h;
                }
                if let Some(l) = &labels {
                    o.extra = l.clone();
                    o.flag = o.flag.max(l.len() as i32);
                }
                if stop_name.is_some() || stop_kind.is_some() {
                    let mut s = crate::stops::StopSettings::read(&o.extra);
                    if let Some(n) = &stop_name {
                        s.name = n.clone();
                    }
                    if let Some(kd) = stop_kind {
                        s.set_kind(kd);
                    }
                    s.write(&mut o.extra);
                }
            });
        }
        if let Some(doc) = self.ed.doc.as_mut() {
            crate::editor::sync_entry_points(doc);
        }
        self.ed.history.merge_since(mark, "Edit object");
        let o = self.ed.doc.as_ref().and_then(|d| d.object(k, id)).cloned().ok_or("the object is gone")?;
        Ok(json!({ "id": id, "file": o.file, "at": p2(tile_origin(k) + DVec2::new(o.pos[0], o.pos[1])), "heading": number(o.rot[0]), "height": number(o.pos[2]), "labels": o.extra }))
    }

    pub fn edit_spline(&mut self, a: Args) -> Done {
        let id = a.i64("id").ok_or("`id` is needed")?;
        let Some(Sel::Spline(k, id)) = self.sel_of(id) else { return Err(format!("no spline {id}")) };
        let mark = self.ed.history.mark();
        if let Some(f) = a.str("file").or(a.str("type")) {
            let rel = self.resolve(f, Some(true))?;
            self.wait_spline(&rel)?;
            self.ed.set_selection(vec![Sel::Spline(k, id)]);
            self.ed.retype_splines(&rel);
        }
        for (end, h, g) in [(false, "start_height", "start_grade"), (true, "end_height", "end_grade")] {
            let (z, gr) = (a.f64(h), a.f64(g));
            if z.is_some() || gr.is_some() {
                self.ed.set_spline_end(k, id, end, z, gr);
                self.ed.history.commit();
            }
        }
        for (end, c) in [(false, "start_cant"), (true, "end_cant")] {
            if let Some(v) = a.f64(c) {
                self.ed.set_spline_cant(k, id, end, v);
                self.ed.history.commit();
            }
        }
        if let Some(at) = a.f64("cut_at") {
            self.ed.cut_spline(k, id, at);
        }
        self.ed.history.merge_since(mark, "Edit spline");
        Ok(json!({ "id": id }))
    }

    pub fn delete(&mut self, a: Args) -> Done {
        let ids = a.ids("ids");
        if ids.is_empty() {
            return Err("`ids` is needed".into());
        }
        let v = self.sels(&ids)?;
        self.ed.set_tool(Tool::Select);
        self.ed.set_selection(v);
        self.ed.delete_selected();
        Ok(json!({}))
    }

    // ---------------------------------------------------------------------------- the ground

    pub fn terrain_brush(&mut self, a: Args) -> Done {
        let mode = terrain_mode_of(a.str("mode").ok_or("`mode` is needed")?)?;
        if matches!(mode, TerrainMode::Ramp | TerrainMode::Water | TerrainMode::AlongSpline) {
            return Err("for a ramp use `ramp`, for water `water`; this is raise, lower, smooth, flatten or roughen".into());
        }
        let path = self.points(a, "path").or_else(|_| self.point(a.get("at").ok_or("`path` is needed")?).map(|p| vec![p]))?;
        self.ed.set_tool(Tool::Terrain);
        self.ed.terrain_mode = mode;
        let keep = (self.ed.brush_radius, self.ed.brush_strength, self.ed.flatten_fixed);
        if let Some(r) = a.f64("radius") {
            self.ed.brush_radius = r.clamp(0.5, 2000.0) as f32;
        }
        if let Some(s) = a.f64("strength") {
            self.ed.brush_strength = s.max(0.0) as f32;
        }
        self.ed.flatten_fixed = a.f64("height").map(|h| h as f32);
        let seconds = a.f64("seconds").unwrap_or(1.0).clamp(0.05, 600.0);
        let before = path.iter().map(|p| self.ground(*p)).collect::<Vec<_>>();
        self.hold(&path, seconds);
        let after = path.iter().map(|p| self.ground(*p)).collect::<Vec<_>>();
        (self.ed.brush_radius, self.ed.brush_strength, self.ed.flatten_fixed) = keep;
        Ok(json!({ "ground_before": before.iter().map(|h| number(*h)).collect::<Vec<_>>(), "ground_after": after.iter().map(|h| number(*h)).collect::<Vec<_>>() }))
    }

    pub fn ramp(&mut self, a: Args) -> Done {
        let (f, t) = (self.point(a.get("from").ok_or("`from` is needed")?)?, self.point(a.get("to").ok_or("`to` is needed")?)?);
        let keep = self.ed.brush_radius;
        if let Some(w) = a.f64("width") {
            self.ed.brush_radius = (w * 0.5).clamp(0.5, 1000.0) as f32;
        }
        self.ed.set_tool(Tool::Terrain);
        self.ed.terrain_mode = TerrainMode::Ramp;
        let (zf, zt) = (a.f64("from_height").unwrap_or(self.ground(f)), a.f64("to_height").unwrap_or(self.ground(t)));
        let grade = self.ed.ramp(DVec3::new(f.x, f.y, zf), DVec3::new(t.x, t.y, zt));
        self.ed.brush_radius = keep;
        Ok(json!({ "grade_percent": number(grade) }))
    }

    pub fn water(&mut self, a: Args) -> Done {
        let pts = self.points(a, "points")?;
        let lake = match a.str("shape").unwrap_or("lake") {
            "lake" => true,
            "river" => false,
            s => return Err(format!("no shape {s}: lake or river")),
        };
        if pts.len() < if lake { 3 } else { 2 } {
            return Err("a lake needs three points of its shore, a river two of its line".into());
        }
        self.ed.set_tool(Tool::Terrain);
        self.ed.terrain_mode = TerrainMode::Water;
        self.ed.water_shape = if lake { WaterShape::Lake } else { WaterShape::River };
        if let Some(d) = a.f64("depth") {
            self.ed.water_depth = d.max(0.1) as f32;
        }
        if let Some(w) = a.f64("width") {
            self.ed.river_width = w.max(1.0) as f32;
        }
        self.ed.cancel();
        for p in &pts {
            self.click_at(*p, false);
        }
        let made = self.ed.make_water();
        Ok(json!({ "made": made }))
    }

    pub fn paint(&mut self, a: Args) -> Done {
        let path = self.points(a, "path").or_else(|_| self.point(a.get("at").ok_or("`path` is needed")?).map(|p| vec![p]))?;
        let layer = match (a.usize("layer"), a.str("texture")) {
            (Some(l), _) => l,
            (None, Some(t)) => self.ground_texture(t)?,
            (None, None) => self.ed.paint_layer,
        };
        let n = self.doc()?.global.ground_textures.len();
        if layer >= n {
            return Err(format!("the map has {n} ground textures (layers 0 to {}); add one with `ground_textures`", n.saturating_sub(1)));
        }
        self.ed.set_tool(Tool::Paint);
        self.ed.paint_layer = layer;
        self.ed.paint_erase = a.bool("erase").unwrap_or(false);
        let keep = (self.ed.brush_radius, self.ed.brush_strength);
        if let Some(r) = a.f64("radius") {
            self.ed.brush_radius = r.clamp(0.5, 2000.0) as f32;
        }
        if let Some(s) = a.f64("strength") {
            self.ed.brush_strength = s.max(0.0) as f32;
        }
        self.hold(&path, a.f64("seconds").unwrap_or(1.0).clamp(0.05, 600.0));
        (self.ed.brush_radius, self.ed.brush_strength) = keep;
        self.ed.paint_erase = false;
        Ok(json!({ "layer": layer }))
    }

    /// The layer of a ground texture, added to the map if it has it not.
    pub(super) fn ground_texture(&mut self, tex: &str) -> Result<usize, String> {
        let rel = tex.trim().replace('/', "\\");
        let (content, omsi) = (self.roots.content.clone(), self.roots.omsi.clone());
        let doc = self.ed.doc.as_mut().ok_or("no map is open")?;
        if let Some(i) = doc.global.ground_textures.iter().position(|g| g.texture.eq_ignore_ascii_case(&rel)) {
            return Ok(i);
        }
        let exists = [&content, &omsi].iter().any(|r| omsi_cfg::vfs::exists(&omsi_cfg::resolve_path(r, &rel))) || omsi_cfg::vfs::exists(&omsi_cfg::resolve_path(&doc.dir, &rel));
        if !exists {
            return Err(format!("no texture {rel} (a path like Texture\\\\...)"));
        }
        let hist = &mut self.ed.history;
        hist.begin("Add ground texture");
        hist.touch_global(doc);
        let detail = if rel.to_lowercase().contains("gras") { "Texture\\gras_det.bmp" } else { "Texture\\noise_low.bmp" };
        doc.global.ground_textures.push(omsi_map::GroundTex { texture: rel, detail_texture: detail.into(), params: [8.0, 16.0, 4.0] });
        doc.sync_mask_slots();
        doc.global_dirty = true;
        doc.global_rev = crate::doc::next_rev();
        hist.commit();
        Ok(doc.global.ground_textures.len() - 1)
    }

    pub fn ground_textures(&mut self, a: Args) -> Done {
        let added = match a.str("add") {
            Some(t) => Some(self.ground_texture(t)?),
            None => None,
        };
        let d = self.doc()?;
        Ok(json!({ "added": added, "layers": d.global.ground_textures.iter().enumerate().map(|(i, g)| json!({ "layer": i, "texture": g.texture, "detail": g.detail_texture })).collect::<Vec<_>>() }))
    }

    pub fn plant(&mut self, a: Args) -> Done {
        let mix = a.strings("mix");
        let mix = if mix.is_empty() {
            // (a few different trees: the same tree is often in several mods' folders)
            let mut names = std::collections::HashSet::new();
            let trees: Vec<String> = self.lib.filter(Kind::Object, Some(Category::Trees), "").filter(|a| names.insert(a.name.to_lowercase())).take(4).map(|a| a.rel.clone()).collect();
            if self.ed.plant_mix.is_empty() { trees } else { self.ed.plant_mix.clone() }
        } else {
            let mut v = Vec::new();
            for m in mix {
                v.push(self.resolve(&m, Some(false))?);
            }
            v
        };
        if mix.is_empty() {
            return Err("no trees to plant: give `mix`".into());
        }
        for m in &mix {
            self.wait_object(m)?;
        }
        self.ed.set_tool(Tool::Objects);
        self.ed.plant_brush = true;
        self.ed.plant_mix = mix.clone();
        self.ed.plant_erase = a.bool("erase").unwrap_or(false);
        if let Some(s) = a.f64("spacing") {
            self.ed.plant_spacing = s.max(0.5);
        }
        let r = if a.get("area").is_some() {
            let pts = self.points(a, "area")?;
            if pts.len() < 3 {
                return Err("an area needs three points or more".into());
            }
            self.ed.plant_area = true;
            self.ed.cancel();
            for p in &pts {
                self.click_at(*p, false);
            }
            let n = self.ed.finish_plant_outline(&mut self.assets);
            json!({ "planted": n })
        } else {
            let path = self.points(a, "path")?;
            self.ed.plant_area = false;
            let keep = self.ed.brush_radius;
            if let Some(r) = a.f64("radius") {
                self.ed.brush_radius = r.clamp(0.5, 2000.0) as f32;
            }
            self.hold(&path, a.f64("seconds").unwrap_or(1.0).clamp(0.05, 600.0));
            self.ed.brush_radius = keep;
            json!({})
        };
        self.ed.plant_brush = false;
        self.ed.plant_area = false;
        self.ed.plant_erase = false;
        Ok(r)
    }

    pub fn tiles(&mut self, a: Args) -> Done {
        let key = |v: &Value| -> Result<TileKey, String> {
            let t = v.as_array().ok_or("a tile is [tx, ty]")?;
            Ok((t.first().and_then(|x| x.as_i64()).ok_or("a tile is [tx, ty]")? as i32, t.get(1).and_then(|x| x.as_i64()).ok_or("a tile is [tx, ty]")? as i32))
        };
        let ts = crate::doc::tile_size();
        let mid = |k: TileKey| tile_origin(k) + DVec2::splat(ts * 0.5);
        let mark = self.ed.history.mark();
        let mut problems = Vec::new();
        if a.bool("grow").unwrap_or(false) {
            self.ed.grow_map();
        }
        if let Some(list) = a.arr("add") {
            // (each next to the map: those next to the ones added first come after them)
            let mut todo: Vec<TileKey> = list.iter().map(key).collect::<Result<_, _>>()?;
            let mut progress = true;
            while progress && !todo.is_empty() {
                progress = false;
                let mut rest = Vec::new();
                for k in todo {
                    let d = self.doc()?;
                    let next = d.tiles.is_empty() || [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| d.tiles.contains_key(&(k.0 + dx, k.1 + dy)));
                    if d.tiles.contains_key(&k) {
                        continue;
                    }
                    if !next {
                        rest.push(k);
                        continue;
                    }
                    self.ed.set_tool(Tool::Tiles);
                    self.click_at(mid(k), false);
                    progress = true;
                }
                todo = rest;
            }
            for k in todo {
                problems.push(format!("tile {}, {} is not next to the map", k.0, k.1));
            }
        }
        if let Some(list) = a.arr("remove") {
            for v in list {
                let k = key(v)?;
                self.ed.set_tool(Tool::Tiles);
                self.hover(mid(k));
                self.ed.right_click();
            }
        }
        if let Some(list) = a.arr("water") {
            for w in list {
                let k = key(w.get("tile").ok_or("water: {tile, level}")?)?;
                self.ed.set_water(k, w.get("level").and_then(|l| l.as_f64()).map(|l| l as f32));
                self.ed.history.commit();
            }
        }
        self.ed.history.merge_since(mark, "Tiles");
        let d = self.doc()?;
        let mut v = json!({ "tile_count": d.tiles.len() });
        if !problems.is_empty() {
            v["problems"] = json!(problems);
        }
        Ok(v)
    }

    // ---------------------------------------------------------------------------- lines

    pub fn make_line(&mut self, a: Args) -> Done {
        use crate::timetable::network::{Files, Network};
        use crate::timetable::route;
        let name = a.str("name").ok_or("`name` is needed")?.trim().to_string();
        let there = a.ids("stops");
        if there.len() < 2 {
            return Err("`stops` needs two stops or more (their IDCodes, in order)".into());
        }
        let speed = a.f64("speed").unwrap_or(22.0).clamp(5.0, 120.0);
        let first = minutes_of(a.get("first"), 6.0 * 60.0)?;
        let last = minutes_of(a.get("last"), 22.0 * 60.0)?.max(first);
        let every = a.f64("every").map(|e| e.max(1.0) as f32);
        let roots = vec![self.roots.content.clone(), self.roots.omsi.clone()];
        let omsi = self.roots.omsi.clone();
        let doc = self.ed.doc.as_mut().ok_or("no map is open")?;
        let all = crate::stops::all(doc);
        let name_of = |id: i64| all.iter().find(|s| s.id == id).map(|s| s.name.clone());
        for id in &there {
            if name_of(*id).is_none() {
                return Err(format!("{id} is not a bus stop (see `timetable` or `query` kinds stops)"));
            }
        }
        // the way back: as given, or each stop's pair across the road, last first
        let back: Vec<i64> = {
            let b = a.ids("back");
            if !b.is_empty() {
                b
            } else if a.bool("one_way").unwrap_or(false) {
                Vec::new()
            } else {
                there.iter().rev().map(|id| crate::stops::pair_of(&all, *id).first().map(|f| f.id).unwrap_or(*id)).collect()
            }
        };
        let before = crate::history::Snapshot { global: Some((doc.global.clone(), doc.order.clone())), timetable: Some(doc.timetable.clone()), traffic: Some(doc.traffic.clone()), ..Default::default() };
        // every stop of the map in the timetable's list
        let ids: Vec<(TileKey, i64)> = all.iter().filter_map(|f| crate::selection::object_by_id(doc, f.id).map(|(k, _)| (k, f.id))).collect();
        let entries = crate::timetable::stop_entries(doc, &ids);
        doc.timetable.sync_stops(entries);
        let mut files = Files::new(roots);
        let net = Network::build(doc, &mut files);
        let left = doc.global.left_hand_traffic;
        let line = doc.timetable.add_line(&name);
        let mut trips: Vec<(String, f32)> = Vec::new();
        let mut problems = Vec::new();
        for stops in [&there, &back] {
            if stops.len() < 2 {
                continue;
            }
            let (a0, b0) = (name_of(stops[0]).unwrap_or_default(), name_of(*stops.last().unwrap()).unwrap_or_default());
            let trip = doc.timetable.add_trip(&line, &format!("{line}_{a0}-{b0}"), stops.clone());
            if let Some(t) = doc.timetable.trip_mut(&trip) {
                t.terminus = b0.clone();
            }
            let pos: Vec<Option<DVec3>> = stops.iter().map(|id| crate::timetable::stop_world(doc, &mut files, *id)).collect();
            let r = route::find(&net, &pos, left, None);
            if !r.ok() {
                problems.push(format!("trip {trip}: no way found over the lanes: {:?}", r.problems));
                continue;
            }
            let m = ((r.length() / 1000.0 / speed * 60.0) + 0.5 * stops.len() as f64).ceil().max(2.0) as f32;
            let tt = &mut doc.timetable;
            tt.set_route(&trip, r.track(&net, &doc.order), r.links(&net, &doc.order, stops));
            if let Some(p) = tt.trip_mut(&trip).and_then(|x| x.profiles.first_mut()) {
                p.factor = m;
            }
            tt.reroute.remove(&trip.to_lowercase());
            trips.push((trip, m));
        }
        if trips.is_empty() || !problems.is_empty() {
            // (nothing half made stays)
            doc.timetable = before.timetable.unwrap_or_default();
            doc.traffic = before.traffic.unwrap_or_default();
            if let Some((g, o)) = before.global {
                doc.global = g;
                doc.order = o;
            }
            return Err(format!("the line was not made: {}", problems.join("; ")));
        }
        // the tours: each bus there and back from its first departure to the last
        let turn = 3.0f32;
        let cycle: f32 = trips.iter().map(|t| t.1 + turn).sum();
        let buses = match every {
            Some(e) => (cycle / e).ceil().max(1.0) as usize,
            None => 1,
        };
        // (each bus goes round once every `buses * every` minutes, waiting at the end for its time)
        let round = every.map(|e| e * buses as f32).unwrap_or(cycle).max(cycle);
        let mut tours = Vec::new();
        for b in 0..buses {
            let mut start = first + every.unwrap_or(0.0) * b as f32;
            let mut list = Vec::new();
            'rounds: while start <= last {
                let mut dep = start;
                for (t, m) in &trips {
                    if dep > last {
                        break 'rounds;
                    }
                    list.push(omsi_timetable::TourTrip { trip: t.clone(), profile: 0, departure: dep });
                    dep += m + turn;
                }
                start += round;
            }
            if !list.is_empty() {
                tours.push(omsi_timetable::Tour { number: (b + 1).to_string(), ai_group: String::new(), extra: crate::timetable::ALL_DAYS.to_string(), trips: list });
            }
        }
        let departures: usize = tours.iter().map(|t| t.trips.len()).sum();
        if let Some(l) = doc.timetable.lines.iter_mut().find(|l| l.name == line) {
            l.tours.extend(tours);
        }
        doc.timetable.touch_line(&line);
        crate::ui_timetable::add_depot(doc);
        crate::traffic::fill_named_groups(&mut doc.traffic, &omsi);
        self.ed.history.push_snapshot(&format!("Line {line}"), before);
        Ok(json!({ "line": line, "trips": trips.iter().map(|t| json!({ "name": t.0, "minutes": t.1 })).collect::<Vec<_>>(), "buses": buses, "departures": departures }))
    }

    // ---------------------------------------------------------------------------- the map

    pub fn fix(&mut self, a: Args) -> Done {
        let kinds: Vec<String> = a.strings("kinds").into_iter().map(|k| k.to_lowercase()).collect();
        let limit = a.usize("limit").unwrap_or(usize::MAX);
        let mark = self.ed.history.mark();
        let issues = crate::check::issues(self.doc()?, &self.lib);
        let (mut fixed, mut failed) = (0, 0);
        let mut seen = std::collections::HashSet::new();
        for i in issues {
            if fixed >= limit {
                break;
            }
            let Some(f) = i.fix else { continue };
            if !kinds.is_empty() && !kinds.contains(&format!("{:?}", i.kind).to_lowercase()) {
                continue;
            }
            // (a fix the check gives for many issues, once)
            if !seen.insert(format!("{f:?}")) {
                continue;
            }
            if crate::check::apply(&mut self.ed, &f) {
                fixed += 1;
            } else {
                failed += 1;
            }
            self.ed.history.commit();
        }
        self.settle();
        self.ed.history.merge_since(mark, "Fixes");
        let left = crate::check::issues(self.doc()?, &self.lib);
        Ok(json!({ "fixed": fixed, "failed": failed, "issues_left": left.len(), "fixable_left": left.iter().filter(|i| i.fix.is_some()).count() }))
    }

    pub fn sample(&mut self, a: Args) -> Done {
        let centre = match a.get("center") {
            Some(c) => self.point(c)?,
            None => {
                let d = self.doc()?;
                let n = d.tiles.len().max(1) as f64;
                d.tiles.keys().fold(DVec2::ZERO, |s, k| s + tile_origin(*k) + DVec2::splat(crate::doc::tile_size() * 0.5)) / n
            }
        };
        let mark = self.ed.history.mark();
        let before = self.doc().map(|d| crate::history::Snapshot { global: Some((d.global.clone(), d.order.clone())), timetable: Some(d.timetable.clone()), traffic: Some(d.traffic.clone()), ..Default::default() })?;
        crate::sample::build(&mut self.ed, &mut self.assets, &self.lib, &self.roots, centre)?;
        self.ed.history.push_snapshot("Sample", before);
        self.ed.history.merge_since(mark, "Sample");
        Ok(json!({ "center": p2(centre) }))
    }

    pub fn new_map(&mut self, a: super::Args) -> Reply {
        if !self.allow_edit {
            return Err("the server is read-only: start it with --allow-edit".into());
        }
        let folder = a.str("folder").ok_or("`folder` is needed")?.trim().to_string();
        if folder.is_empty() || folder.contains(['\\', '/', ':']) {
            return Err("`folder` is a plain folder name".into());
        }
        let friendly = a.str("name").unwrap_or(&folder).to_string();
        let size = a.i64("size").unwrap_or(2).clamp(1, 10) as i32;
        let mut name = folder.clone();
        let mut i = 2;
        while self.roots.content.join("maps").join(&name).exists() || self.roots.omsi.join("maps").join(&name).exists() {
            name = format!("{folder} {i}");
            i += 1;
        }
        let mut doc = Document::new_map(&self.roots, &name, &friendly, size);
        doc.global.left_hand_traffic = a.bool("left_hand").unwrap_or(false);
        if let Some(d) = a.str("description") {
            doc.global.description = d.to_string();
        }
        if let Some(h) = a.f64("height") {
            for t in doc.tiles.values_mut() {
                for x in t.terrain.heights.iter_mut() {
                    *x = h as f32;
                }
                t.touch_terrain();
            }
        }
        self.put(doc);
        self.cfg = None;
        crate::doc::save::save(self.ed.doc.as_mut().ok_or("no map")?).map_err(|e| e.to_string())?;
        omsi_cfg::content_changed();
        let mut v = self.map_info()?;
        v["note"] = json!("Made and saved; it is open now. Start with a road (draw_road) or the sample (sample).");
        Ok(v.into())
    }

    /// Bookkeeping the files do not contain, as the window does before a save.
    fn prepare_save(&mut self) -> Result<(), String> {
        self.ed.history.commit();
        let roots = self.roots.clone();
        let doc = self.ed.doc.as_mut().ok_or("no map is open")?;
        crate::editor::sync_entry_points(doc);
        self.tt.before_save(doc, Some(&roots));
        crate::timetable::follow_stop_tiles(doc);
        let order = doc.order.clone();
        doc.timetable.follow_order(&order);
        doc.sync_mask_slots();
        if !doc.traffic.signal_routes.is_empty() {
            let mut routes = std::mem::take(&mut doc.traffic.signal_routes);
            let changed = crate::signals::reindex(&mut routes, doc, &roots);
            doc.traffic.signal_routes = routes;
            if changed {
                doc.traffic.touch(crate::traffic::File::SignalRoutes);
            }
        }
        Ok(())
    }

    pub fn save(&mut self) -> Reply {
        self.prepare_save()?;
        let doc = self.ed.doc.as_mut().ok_or("no map is open")?;
        crate::doc::save(doc).map_err(|e| e.to_string())?;
        omsi_cfg::content_changed();
        let dir = doc.dir.clone();
        let gone = self.ed.clean_junction_files();
        let has_start = !self.doc()?.global.entry_points.is_empty();
        let mut v = json!({ "saved": true, "folder": dir, "cfg": dir.join("global.cfg"), "unused_junction_files_removed": gone });
        if !has_start {
            v["tip"] = json!("The map has no start point: add one (start_point) so the bus has somewhere to appear.");
        }
        Ok(v.into())
    }

    pub fn export(&mut self, a: super::Args) -> Reply {
        let out = std::path::PathBuf::from(a.str("path").ok_or("`path` is needed: where the .zip goes")?);
        self.save()?;
        let (plan, bytes) = crate::export::export(self.doc()?, &self.roots, &out).map_err(|e| e.to_string())?;
        Ok(json!({ "zip": out, "files": plan.files.len(), "megabytes": number(bytes as f64 / 1e6), "missing": plan.missing }).into())
    }
}
