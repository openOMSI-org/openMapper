//! What the server tells about the map without changing it.

use std::collections::HashMap;

use glam::DVec2;
use serde_json::{json, Value};

use super::{number, p2, Args, Out, Reply, Server};
use crate::doc::{tile_of, tile_origin, Sel, TileKey};
use crate::library::{Category, Kind};

pub(crate) fn category_name(c: Category) -> &'static str {
    match c {
        Category::Objects => "objects",
        Category::Trees => "trees",
        Category::Stops => "stops",
        Category::Junctions => "junctions",
        Category::Roads => "roads",
        Category::Tracks => "tracks",
        Category::Paths => "paths",
        Category::Decoration => "decoration",
        Category::Technical => "technical",
    }
}

fn category_of(s: &str) -> Option<Category> {
    Some(match s.to_lowercase().as_str() {
        "objects" | "object" | "buildings" => Category::Objects,
        "trees" | "tree" | "plants" => Category::Trees,
        "stops" | "stop" => Category::Stops,
        "junctions" | "junction" => Category::Junctions,
        "roads" | "road" => Category::Roads,
        "tracks" | "track" | "rails" => Category::Tracks,
        "paths" | "footpaths" | "footpath" => Category::Paths,
        "decoration" | "fences" | "walls" => Category::Decoration,
        "technical" | "ai" => Category::Technical,
        _ => return None,
    })
}

impl Server {
    /// A catalogue entry, short.
    pub(crate) fn asset_json(a: &crate::library::Asset) -> Value {
        let mut v = json!({ "file": a.rel, "name": a.name, "category": category_name(a.category), "folder": a.dir });
        if a.kind == Kind::Spline {
            v["width"] = number(a.half_width as f64 * 2.0);
            if a.lanes != (0, 0, 0) {
                v["lanes"] = json!([a.lanes.0, a.lanes.1, a.lanes.2]);
            }
        }
        let flags: Vec<&str> = [(a.is_bus_stop, "bus_stop"), (a.is_entry_point, "start_point"), (a.is_car_park, "car_park"), (a.lamp, "lamp"), (a.light_head, "signal_head"), (a.light_program, "traffic_lights"), (!a.helpers.is_empty(), "spline_helpers"), (a.from_content, "mod")].into_iter().filter(|f| f.0).map(|f| f.1).collect();
        if !flags.is_empty() {
            v["flags"] = json!(flags);
        }
        v
    }

    pub fn map_info(&self) -> Result<Value, String> {
        let d = self.doc()?;
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
        let (undo, redo) = self.ed.history.steps();
        Ok(json!({
            "name": d.global.name,
            "friendly_name": d.global.friendly_name,
            "description": d.global.description,
            "folder": d.dir,
            "tile_size": crate::doc::tile_size(),
            "world_coordinates": d.global.world_coordinates,
            "left_hand_traffic": d.global.left_hand_traffic,
            "tile_count": keys.len(),
            "tiles": if keys.len() <= 200 { json!(keys) } else { json!(format!("{} tiles (see bounds)", keys.len())) },
            "bounds": bounds,
            "objects": objects,
            "splines": splines,
            "bus_stops": crate::stops::all(d).len(),
            "start_points": d.global.entry_points.iter().map(|e| json!({ "name": e.name, "object": e.object_id })).collect::<Vec<_>>(),
            "lines": d.timetable.lines.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
            "ground_textures": d.global.ground_textures.iter().enumerate().map(|(i, g)| json!({ "layer": i, "texture": g.texture })).collect::<Vec<_>>(),
            "placed_on_earth": self.georef().is_some(),
            "unsaved_changes": d.is_dirty(),
            "undo": undo.last(),
            "redo": redo.first(),
            "can_edit": self.allow_edit,
            "catalogue_ready": !self.lib.scanning,
        }))
    }

    pub fn list_maps(&mut self) -> Reply {
        let maps = crate::install::list_maps(&self.roots);
        Ok(json!({ "maps": maps.iter().map(|m| json!({ "name": m.name, "cfg": m.cfg, "in_content": m.in_content })).collect::<Vec<_>>() }).into())
    }

    pub fn open_map(&mut self, a: Args) -> Reply {
        let path = a.str("path").ok_or("`path` is needed: a map's global.cfg")?;
        let mut p = std::path::PathBuf::from(path);
        if p.is_dir() {
            p = p.join("global.cfg");
        }
        if !p.is_file() {
            // (a map's name, as list_maps gives it)
            if let Some(m) = crate::install::list_maps(&self.roots).into_iter().find(|m| m.name.eq_ignore_ascii_case(path)) {
                p = m.cfg;
            }
        }
        self.open(&p)?;
        Ok(self.map_info()?.into())
    }

    pub fn reload(&mut self) -> Reply {
        let cfg = match (&self.cfg, &self.ed.doc) {
            (Some(c), _) => c.clone(),
            (None, Some(d)) => d.dir.join("global.cfg"),
            _ => return Err("no map is open".into()),
        };
        self.open(&cfg)?;
        Ok(json!({ "reloaded": true, "tiles": self.doc()?.tiles.len() }).into())
    }

    pub fn catalogue(&mut self, a: Args) -> Reply {
        self.wait_lib();
        let limit = a.usize("limit").unwrap_or(50).clamp(1, 1000);
        let offset = a.usize("offset").unwrap_or(0);
        if a.bool("on_map").unwrap_or(false) {
            let d = self.doc()?;
            let mut counts: HashMap<String, (usize, bool)> = HashMap::new();
            for t in d.tiles.values() {
                for o in &t.tile.objects {
                    counts.entry(o.file.to_lowercase()).or_insert((0, false)).0 += 1;
                }
                for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                    counts.entry(s.file.to_lowercase()).or_insert((0, true)).0 += 1;
                }
            }
            let words: Vec<String> = a.str("search").unwrap_or("").to_lowercase().split_whitespace().map(str::to_string).collect();
            let mut list: Vec<(String, usize, bool)> = counts.into_iter().filter(|(f, _)| words.iter().all(|w| f.contains(w))).map(|(f, (n, s))| (f, n, s)).collect();
            list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let total = list.len();
            let out: Vec<Value> = list.into_iter().skip(offset).take(limit).map(|(f, n, s)| {
                let mut v = match self.lib.find(&f) {
                    Some(a) => Self::asset_json(a),
                    None => json!({ "file": f, "missing": true }),
                };
                v["count"] = json!(n);
                v["spline"] = json!(s);
                v
            }).collect();
            return Ok(json!({ "total": total, "files": out }).into());
        }
        let search = a.str("search").unwrap_or("");
        let cat = a.str("category").map(|c| category_of(c).ok_or(format!("no category {c}"))).transpose()?;
        let kinds: Vec<Kind> = match a.str("kind") {
            Some("object") | Some("objects") => vec![Kind::Object],
            Some("spline") | Some("splines") => vec![Kind::Spline],
            _ => vec![Kind::Spline, Kind::Object],
        };
        let found: Vec<&crate::library::Asset> = kinds.iter().flat_map(|k| self.lib.filter(*k, cat, search)).collect();
        let total = found.len();
        let out: Vec<Value> = found.into_iter().skip(offset).take(limit).map(Self::asset_json).collect();
        Ok(json!({ "total": total, "shown": out.len(), "offset": offset, "assets": out }).into())
    }

    pub fn check(&mut self, a: Args) -> Reply {
        self.wait_lib();
        let want = a.str("severity").map(|s| s.to_lowercase());
        let limit = a.usize("limit").unwrap_or(usize::MAX);
        let issues = crate::check::issues(self.doc()?, &self.lib);
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
                v["at"] = p2(p);
            }
            match i.sel {
                Some(Sel::Object(_, id)) => v["object"] = json!(id),
                Some(Sel::Spline(_, id)) => v["spline"] = json!(id),
                Some(Sel::Attached(_, id)) => v["attached"] = json!(id),
                None => {}
            }
            out.push(v);
        }
        Ok(json!({ "errors": counts[0], "warnings": counts[1], "infos": counts[2], "total": issues.len(), "issues": out }).into())
    }

    pub fn query(&mut self, a: Args) -> Reply {
        self.wait_lib();
        let (centre, radius, boxed) = match (a.get("at"), a.get("min"), a.get("max")) {
            (_, Some(lo), Some(hi)) => {
                let (lo, hi) = (self.point(lo)?, self.point(hi)?);
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                ((lo + hi) * 0.5, f64::MAX, Some((lo, hi)))
            }
            (Some(at), _, _) => (self.point(at)?, a.f64("radius").unwrap_or(50.0).max(0.0), None),
            _ => return Err("give `at` (and `radius`), or `min` and `max`".into()),
        };
        let inside = |p: DVec2, dist: f64| match boxed {
            Some((lo, hi)) => p.x >= lo.x && p.y >= lo.y && p.x <= hi.x && p.y <= hi.y,
            None => dist <= radius,
        };
        let limit = a.usize("limit").unwrap_or(100);
        let full = a.bool("full").unwrap_or(false);
        let filter = a.str("file").map(|f| f.to_lowercase().replace('/', "\\"));
        let file_ok = |f: &str| filter.as_ref().map(|w| f.to_lowercase().contains(w.as_str())).unwrap_or(true);
        let kinds = a.strings("kinds");
        let wants = |k: &str| kinds.is_empty() || kinds.iter().any(|x| x == k);
        let pos = |p: DVec2| if full { self.describe(p) } else { p2(p) };
        let d = self.doc()?;
        let mut out = serde_json::Map::new();
        out.insert("at".into(), p2(centre));
        let take = |mut found: Vec<(f64, Value)>| -> Value {
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            let n = found.len();
            let mut list: Vec<Value> = found.into_iter().take(limit).map(|f| f.1).collect();
            if n > limit {
                list.push(json!(format!("... and {} more", n - limit)));
            }
            Value::Array(list)
        };
        if wants("objects") {
            let mut found: Vec<(f64, Value)> = Vec::new();
            for (k, t) in &d.tiles {
                let org = tile_origin(*k);
                for o in &t.tile.objects {
                    let p = org + DVec2::new(o.pos[0], o.pos[1]);
                    let dist = (p - centre).length();
                    if inside(p, dist) && file_ok(&o.file) {
                        let mut v = json!({ "id": o.id, "file": o.file, "at": pos(p), "heading": number(o.rot[0]), "distance": number(dist) });
                        if o.pos[2].abs() > 1e-3 {
                            v["height"] = number(o.pos[2]);
                        }
                        if let Some(c) = self.lib.find(&o.file) {
                            v["category"] = json!(category_name(c.category));
                        }
                        if let Some(n) = o.extra.first().filter(|n| !n.is_empty()) {
                            v["label"] = json!(n);
                        }
                        found.push((dist, v));
                    }
                }
            }
            out.insert("objects".into(), take(found));
        }
        if wants("splines") {
            let mut found: Vec<(f64, Value)> = Vec::new();
            for (k, t) in &d.tiles {
                let org = tile_origin(*k);
                for s in t.tile.splines.iter().filter(|s| !s.deleted) {
                    if !file_ok(&s.file) {
                        continue;
                    }
                    let start = org + DVec2::new(s.pos[0], s.pos[1]);
                    // (too far for any of it to come near)
                    if boxed.is_none() && (start - centre).length() > radius + s.length.max(0.0) {
                        continue;
                    }
                    let curve = omsi_geometry::SplineCurve::from_map(s, org);
                    let len = s.length.max(0.0);
                    let steps = ((len / 5.0).ceil() as usize).clamp(2, 400);
                    let mut near = f64::MAX;
                    let mut hit = false;
                    for i in 0..=steps {
                        let p = curve.point_at(len * i as f64 / steps as f64).truncate();
                        let dist = (p - centre).length();
                        near = near.min(dist);
                        hit |= inside(p, dist);
                    }
                    if hit {
                        let mut v = json!({ "id": s.id, "file": s.file, "from": pos(curve.point_at(0.0).truncate()), "to": pos(curve.end_point().truncate()), "length": number(len), "distance": number(near) });
                        if s.radius != 0.0 {
                            v["radius"] = number(s.radius);
                        }
                        if s.prev_id != 0 || s.next_id != 0 {
                            v["links"] = json!([s.prev_id, s.next_id]);
                        }
                        if let Some(c) = self.lib.find(&s.file) {
                            v["category"] = json!(category_name(c.category));
                        }
                        found.push((near, v));
                    }
                }
            }
            out.insert("splines".into(), take(found));
        }
        if wants("stops") {
            let found: Vec<(f64, Value)> = crate::stops::all(d).into_iter().filter_map(|f| {
                let dist = (f.at - centre).length();
                inside(f.at, dist).then(|| (dist, json!({ "id": f.id, "name": f.name, "at": pos(f.at), "distance": number(dist) })))
            }).collect();
            out.insert("stops".into(), take(found));
        }
        if wants("entries") {
            let mut found: Vec<(f64, Value)> = Vec::new();
            for (i, e) in d.global.entry_points.iter().enumerate() {
                if let Some((k, o)) = crate::selection::object_by_id(d, e.object_id) {
                    let p = tile_origin(k) + DVec2::new(o.pos[0], o.pos[1]);
                    let dist = (p - centre).length();
                    if inside(p, dist) {
                        found.push((dist, json!({ "index": i, "name": e.name, "object": e.object_id, "at": pos(p), "heading": number(o.rot[0]), "distance": number(dist) })));
                    }
                }
            }
            out.insert("entries".into(), take(found));
        }
        Ok(Value::Object(out).into())
    }

    pub fn inspect(&mut self, a: Args) -> Reply {
        let ids = a.ids("ids");
        let ids = if ids.is_empty() { a.ids("id") } else { ids };
        if ids.is_empty() {
            return Err("`ids` is needed".into());
        }
        self.wait_lib();
        let d = self.doc()?;
        let mut out = Vec::new();
        for id in ids {
            let mut found = None;
            'tiles: for (k, t) in &d.tiles {
                let org = tile_origin(*k);
                if let Some(o) = t.tile.objects.iter().find(|o| o.id == id) {
                    let at = org + DVec2::new(o.pos[0], o.pos[1]);
                    let mut v = json!({
                        "id": id, "type": "object", "file": o.file, "tile": [k.0, k.1], "at": p2(at), "local": [number(o.pos[0]), number(o.pos[1])],
                        "height": number(o.pos[2]), "ground": number(d.ground(at)), "heading": number(o.rot[0]), "pitch": number(o.rot[1]), "bank": number(o.rot[2]),
                        "labels": o.extra,
                    });
                    if let Some(p) = o.parent_id {
                        v["attached_to"] = json!({ "object": p, "point": o.attach_index });
                    }
                    if let Some(p) = o.var_parent {
                        v["var_parent"] = json!(p);
                    }
                    if !o.rules.is_empty() {
                        v["rules"] = json!(o.rules.len());
                    }
                    if let Some(c) = self.lib.find(&o.file) {
                        v["catalogue"] = Self::asset_json(c);
                        if c.is_bus_stop {
                            let s = crate::stops::StopSettings::read(&o.extra);
                            v["stop"] = json!({ "name": s.name, "kind": format!("{:?}", s.kind).to_lowercase(), "enter_max": s.enter_max, "enter_min": s.enter_min, "exit": s.exit, "length": s.length });
                        }
                    } else {
                        v["missing"] = json!(true);
                    }
                    if let Some(e) = d.global.entry_points.iter().find(|e| e.object_id == id) {
                        v["start_point"] = json!(e.name);
                    }
                    found = Some(v);
                    break 'tiles;
                }
                if let Some(s) = t.tile.splines.iter().find(|s| s.id == id && !s.deleted) {
                    let c = omsi_geometry::SplineCurve::from_map(s, org);
                    let (a0, b0) = (c.point_at(0.0), c.end_point());
                    let mut v = json!({
                        "id": id, "type": "spline", "file": s.file, "tile": [k.0, k.1],
                        "start": { "at": p2(a0.truncate()), "z": number(a0.z), "heading": number(s.heading), "grade": number(s.grad_start), "cant": number(s.cant_start) },
                        "end": { "at": p2(b0.truncate()), "z": number(b0.z), "heading": number(c.heading_at(s.length)), "grade": number(s.grad_end), "cant": number(s.cant_end) },
                        "length": number(s.length), "radius": number(s.radius), "prev": s.prev_id, "next": s.next_id, "mirror": s.mirror,
                    });
                    if let Some(c) = self.lib.find(&s.file) {
                        v["catalogue"] = Self::asset_json(c);
                    } else {
                        v["missing"] = json!(true);
                    }
                    let att: Vec<Value> = t.tile.spline_attachments.iter().filter(|x| t.tile.splines.get(x.spline_index as usize).map(|sp| sp.id) == Some(id)).map(|x| json!({ "id": x.id, "file": x.file, "offset": x.offset, "interval": x.interval })).collect();
                    if !att.is_empty() {
                        v["attachments"] = json!(att);
                    }
                    found = Some(v);
                    break 'tiles;
                }
            }
            out.push(found.unwrap_or_else(|| json!({ "id": id, "error": "no object or spline with this IDCode" })));
        }
        Ok(json!({ "items": out }).into())
    }

    pub fn terrain(&mut self, a: Args) -> Reply {
        let mut points: Vec<DVec2> = Vec::new();
        if a.get("points").is_some() {
            points.extend(self.points(a, "points")?);
        }
        if let (Some(f), Some(t)) = (a.get("from"), a.get("to")) {
            let (f, t) = (self.point(f)?, self.point(t)?);
            let n = a.usize("samples").unwrap_or(21).clamp(2, 5000);
            points.extend((0..n).map(|i| f.lerp(t, i as f64 / (n - 1) as f64)));
        }
        let d = self.doc()?;
        if let (Some(lo), Some(hi)) = (a.get("min"), a.get("max")) {
            let (lo, hi) = (self.point(lo)?, self.point(hi)?);
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            let step = a.f64("step").unwrap_or(10.0).max(0.5);
            let (nx, ny) = (((hi.x - lo.x) / step).floor() as usize + 1, ((hi.y - lo.y) / step).floor() as usize + 1);
            if nx * ny > 40_000 {
                return Err(format!("{nx} x {ny} points: at most 40000, give a larger `step`"));
            }
            // rows from the south, each from the west
            let rows: Vec<Value> = (0..ny).map(|j| Value::Array((0..nx).map(|i| {
                let p = lo + DVec2::new(i as f64 * step, j as f64 * step);
                d.height_at(p).map(|h| json!((h * 100.0).round() / 100.0)).unwrap_or(Value::Null)
            }).collect())).collect();
            return Ok(json!({ "min": p2(lo), "step": step, "columns": nx, "rows_from_south": rows }).into());
        }
        if points.is_empty() {
            return Err("give `points`, or `from` and `to`, or `min`, `max` and `step`".into());
        }
        if points.len() > 5000 {
            return Err("at most 5000 points".into());
        }
        let heights: Vec<Value> = points.iter().map(|p| {
            let on_map = d.tiles.contains_key(&tile_of(*p));
            json!({ "at": p2(*p), "height": if on_map { number(d.ground(*p)) } else { Value::Null } })
        }).collect();
        Ok(json!({ "heights": heights }).into())
    }

    pub fn timetable(&mut self, a: Args) -> Reply {
        let d = self.doc()?;
        let tt = &d.timetable;
        let only = a.str("line").map(str::to_lowercase);
        let stop_name = |id: i64| tt.stop_name(id).map(str::to_string).unwrap_or_else(|| format!("#{id}"));
        let lines: Vec<Value> = tt.lines.iter().filter(|l| only.as_ref().map(|o| l.name.to_lowercase() == *o).unwrap_or(true)).map(|l| {
            let trips: Vec<Value> = tt.trips.iter().filter(|t| t.line.eq_ignore_ascii_case(&l.name)).map(|t| json!({
                "name": t.name, "terminus": t.terminus,
                "stops": t.stations.iter().map(|id| json!({ "id": id, "name": stop_name(*id) })).collect::<Vec<_>>(),
                "minutes": t.profiles.first().map(|p| p.factor),
            })).collect();
            let tours: Vec<Value> = l.tours.iter().map(|t| {
                let deps: Vec<String> = t.trips.iter().map(|x| format!("{:02}:{:02} {}", (x.departure / 60.0) as i32, (x.departure % 60.0).round() as i32, x.trip)).collect();
                json!({ "number": t.number, "departures": deps.len(), "first": deps.first(), "last": deps.last() })
            }).collect();
            json!({ "name": l.name, "trips": trips, "tours": tours })
        }).collect();
        let stops: Vec<Value> = crate::stops::all(d).into_iter().map(|s| json!({ "id": s.id, "name": s.name, "at": p2(s.at), "in_timetable": tt.stops.iter().any(|e| e.object_id == s.id) })).collect();
        Ok(json!({ "lines": lines, "stops": stops }).into())
    }

    pub fn editor_state(&mut self) -> Reply {
        let e = &self.ed;
        let (undo, redo) = e.history.steps();
        let sel: Vec<Value> = e.selection().into_iter().map(|s| match s {
            Sel::Object(_, id) => json!({ "object": id }),
            Sel::Spline(_, id) => json!({ "spline": id }),
            Sel::Attached(_, id) => json!({ "attached": id }),
        }).collect();
        Ok(json!({
            "tool": format!("{:?}", e.tool).to_lowercase(),
            "terrain_mode": format!("{:?}", e.terrain_mode).to_lowercase(),
            "logic_mode": format!("{:?}", e.logic).to_lowercase(),
            "water_shape": format!("{:?}", e.water_shape).to_lowercase(),
            "road_type": e.road_type, "object_type": e.object_type, "path_type": e.path_type,
            "brush_radius": e.brush_radius, "brush_strength": e.brush_strength, "flatten_height": e.flatten_fixed,
            "paint_layer": e.paint_layer, "paint_erase": e.paint_erase,
            "place_heading": e.place_heading, "align_to_road": e.align_to_road,
            "auto_junctions": e.auto_junctions, "auto_terrain": e.auto_terrain, "follow_ground": e.follow_ground, "snap_roads": e.snap_roads, "keep_drawing": e.keep_drawing,
            "roundabout": e.roundabout, "bus_pocket": e.bus_pocket,
            "plant_brush": e.plant_brush, "plant_area": e.plant_area, "plant_spacing": e.plant_spacing, "plant_mix": e.plant_mix,
            "drawing_road": e.road_start_active(),
            "selection": sel,
            "undo_steps": undo.iter().rev().take(10).collect::<Vec<_>>(),
            "redo_steps": redo.iter().take(10).collect::<Vec<_>>(),
        }).into())
    }

    /// The map's Lua script: read, checked, written.
    pub fn script(&mut self, a: Args) -> Reply {
        let d = self.doc()?;
        let folder = d.dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let friendly = d.global.friendly_name.clone();
        let content = self.roots.content.clone();
        if a.bool("delete").unwrap_or(false) {
            if !self.allow_edit {
                return Err("the server is read-only: start it with --allow-edit".into());
            }
            crate::scripting::delete_lua(&content, &folder).map_err(|e| e.to_string())?;
            return Ok(json!({ "deleted": true }).into());
        }
        match a.str("text") {
            None => {
                let templates: Vec<Value> = crate::scripting::lua_templates().into_iter().map(|t| json!({ "title": t.title, "text": t.text })).collect();
                Ok(json!({ "path": crate::scripting::lua_path(&content, &folder), "text": crate::scripting::read_lua(&content, &folder), "templates": templates,
                    "notes": "The header above the line is made by openMapper: `on(event, fn)` and `every(seconds, fn)` run on this map only; MAP is the map's folder." }).into())
            }
            Some(text) => {
                let problems: Vec<Value> = crate::scripting::check_lua(text).into_iter().map(|p| json!({ "line": p.line, "message": p.message })).collect();
                if !problems.is_empty() {
                    return Ok(json!({ "written": false, "problems": problems }).into());
                }
                if !self.allow_edit {
                    return Ok(json!({ "written": false, "ok": true, "note": "read-only server: only checked" }).into());
                }
                crate::scripting::write_lua(&content, &folder, &friendly, text).map_err(|e| e.to_string())?;
                Ok(json!({ "written": true, "path": crate::scripting::lua_path(&content, &folder) }).into())
            }
        }
    }
}

/// So the tests can see what a tool gave.
#[allow(dead_code)]
pub(crate) fn out_json(o: Out) -> Value {
    match o {
        Out::Json(v) => v,
        Out::Image { meta, .. } => meta,
    }
}
