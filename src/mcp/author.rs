//! Authoring by hand, exactly as asked (for an agent building a reference area): road types
//! with their own cross-section and markings, junction plates from named road ends, spline
//! chains through points (tangent-continuous, joined to given ends), complex junctions drawn
//! from pieces and objects.

use glam::DVec2;
use serde_json::{json, Value};

use super::{number, Args, Server};
use crate::doc::{Sel, TileKey};
use crate::roads::Seg;

type Done = Result<Value, String>;

impl Server {
    /// A spline end from `{id, end: "start" | "end"}`, `[id, "end"]` or `"id:end"`.
    fn end_arg(&self, v: &Value) -> Result<(TileKey, i64, bool), String> {
        let (id, which) = match v {
            Value::Object(o) => (o.get("id").and_then(|x| x.as_i64()), o.get("end").and_then(|x| x.as_str()).unwrap_or("end").to_string()),
            Value::Array(a) => (a.first().and_then(|x| x.as_i64()), a.get(1).and_then(|x| x.as_str()).unwrap_or("end").to_string()),
            Value::String(s) => {
                let (i, e) = s.split_once(':').unwrap_or((s, "end"));
                (i.trim().parse().ok(), e.trim().to_string())
            }
            _ => (None, String::new()),
        };
        let id = id.ok_or_else(|| format!("a spline end is {{id, end: start|end}}, not {v}"))?;
        let at_end = match which.as_str() {
            "end" | "1" => true,
            "start" | "0" => false,
            e => return Err(format!("end `{e}`: start or end")),
        };
        match self.sel_of(id) {
            Some(Sel::Spline(k, id)) => Ok((k, id, at_end)),
            _ => Err(format!("{id} is not a spline of the map")),
        }
    }

    /// A texture file of the installation or the content folder (or a full path).
    fn texture_file(&self, v: &str) -> Result<std::path::PathBuf, String> {
        let rel = v.replace('\\', "/");
        let p = std::path::PathBuf::from(&rel);
        if p.is_absolute() && p.is_file() {
            return Ok(p);
        }
        for root in [&self.roots.content, &self.roots.omsi] {
            let q = root.join(&rel);
            if q.is_file() {
                return Ok(q);
            }
        }
        Err(format!("texture {v}: not found in the content folder or the installation"))
    }

    /// `road_type`: a road type of one's own (road maker): lanes, widths, footways, kerb,
    /// median, textures and painted lines. Written to `Splines\openMapper\<name>.sli`.
    pub fn road_type_make(&mut self, a: Args) -> Done {
        let mut spec_v = a.get("spec").cloned().ok_or("`spec` is needed: {name, lanes_fwd, lanes_back, lane_width, median, median_raised, footway, footway_right, kerb, pocket, roadway_tex, footway_tex, median_tex, marked, markings: [{x, style, color, width, dash, gap}]}")?;
        // (textures: names found in the content folder or the installation)
        for k in ["roadway_tex", "footway_tex", "median_tex"] {
            if let Some(t) = spec_v.get(k).and_then(|t| t.as_str()).filter(|t| !t.is_empty()).map(str::to_string) {
                spec_v[k] = json!(self.texture_file(&t)?);
            }
        }
        let mut spec: crate::road_maker::RoadSpec = serde_json::from_value(spec_v).map_err(|e| format!("spec: {e}"))?;
        anyhow_none(spec.name.trim().is_empty(), "spec.name is needed")?;
        if spec.roadway_tex.as_os_str().is_empty() {
            spec.roadway_tex = self.ed.asphalt.clone().ok_or("spec.roadway_tex is needed (no default asphalt found)")?;
        }
        if spec.footway_tex.as_os_str().is_empty() {
            spec.footway_tex = spec.roadway_tex.clone();
        }
        if a.bool("standard_markings").unwrap_or(false) || (spec.marked && spec.markings.is_empty()) {
            spec.markings = spec.standard_markings();
            spec.marked = true;
        }
        let rel = spec.write(&self.roots.content).map_err(|e| e.to_string())?;
        self.assets.forget_spline(&rel, &self.roots.content.join("Splines").join("openMapper"));
        let root = self.roots.content.clone();
        self.lib.add_file(&root, &root.join(rel.replace('\\', "/")), crate::library::Kind::Spline);
        self.wait_spline(&rel)?;
        let half = self.assets.spline(&rel).map(|m| m.half_width as f64).unwrap_or(0.0);
        Ok(json!({
            "file": rel, "roadway_width": number(spec.roadway_width() as f64), "total_width": number(spec.total_width() as f64), "half_width": number(half),
            "markings": spec.markings.iter().map(|m| json!({"x": m.x, "style": format!("{:?}", m.style), "color": m.color})).collect::<Vec<_>>(),
        }))
    }

    /// `plate`: a junction plate (or a joint of two) from road ends named one by one.
    pub fn plate_make(&mut self, a: Args) -> Done {
        let ends: Vec<(TileKey, i64, bool)> = a.arr("ends").ok_or("`ends` is needed: [{id, end}, ...]")?.iter().map(|v| self.end_arg(v)).collect::<Result<_, _>>()?;
        let ask = crate::editor::PlateAsk {
            lights: a.bool("lights"),
            crossings: a.get("crossings").map(|_| a.ids("crossings").into_iter().map(|i| i.max(0) as usize).collect()),
            back: a.f64("cut_back"),
            backs: a.get("cut_back").and_then(|v| v.as_array()).map(|v| v.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()),
        };
        let made = self.ed.plate_from_ends_with(&mut self.assets, &ends, &ask)?;
        Ok(json!({ "made": made }))
    }

    /// `lay_spline`: a chain of one spline type through points, tangent-continuous everywhere,
    /// optionally starting from and finishing at road ends (heading and height taken from them).
    pub fn lay_spline(&mut self, a: Args) -> Done {
        let file = self.resolve(a.str("type").or(a.str("file")).ok_or("`type` is needed (a spline file)")?, Some(true))?;
        self.wait_spline(&file)?;
        let from = a.get("from").map(|v| self.end_arg(v)).transpose()?;
        let to = a.get("to").map(|v| self.end_arg(v)).transpose()?;
        let mut pts: Vec<DVec2> = Vec::new();
        let doc = self.ed.doc.as_ref().ok_or("no map")?;
        let (f, t) = (from.and_then(|e| crate::ed_author::end_of(doc, e)), to.and_then(|e| crate::ed_author::end_of(doc, e)));
        if let Some((p, _)) = f {
            pts.push(p.truncate());
        }
        for v in a.arr("points").into_iter().flatten() {
            pts.push(self.point(v)?);
        }
        if let Some((p, _)) = t {
            pts.push(p.truncate());
        }
        let segs: Vec<Seg> = if let Some(raw) = a.arr("segs") {
            raw.iter()
                .map(|s| {
                    let g = |k: &str| s.get(k).and_then(|x| x.as_f64());
                    Ok(Seg { start: DVec2::new(g("x").ok_or("segs[].x")?, g("y").ok_or("segs[].y")?), heading: g("heading").ok_or("segs[].heading")?, length: g("length").ok_or("segs[].length")?, radius: g("radius").unwrap_or(0.0) })
                })
                .collect::<Result<_, String>>()?
        } else {
            // (the heading into `to`: against the way out of that road)
            let h1 = t.map(|(_, h)| crate::roads::norm_deg(h + 180.0)).or(a.f64("end_heading"));
            crate::ed_author::chain_through(&pts, f.map(|(_, h)| h).or(a.f64("start_heading")), h1)?
        };
        let made = self.ed.lay_chain(&file, &segs, from, to, a.f64("lift").unwrap_or(0.0))?;
        Ok(json!({
            "ids": made.iter().map(|m| m.1).collect::<Vec<_>>(),
            "pieces": segs.iter().map(|s| json!({"x": number(s.start.x), "y": number(s.start.y), "heading": number(s.heading), "radius": number(s.radius), "length": number(s.length)})).collect::<Vec<_>>(),
            "length": number(segs.iter().map(|s| s.length).sum()),
        }))
    }

    /// `complex_junction`: a junction drawn from pieces (any spline types: asphalt, markings,
    /// kerbs, footways) along points or arcs, and objects, in world coordinates; the plate made
    /// where its road pieces cross. Put on the map as ordinary splines and objects.
    pub fn complex_junction(&mut self, a: Args) -> Done {
        let raw = a.arr("pieces").ok_or("`pieces` is needed: [{type, points | segs, lift}]")?.clone();
        let mut pieces = Vec::new();
        let mut all: Vec<DVec2> = Vec::new();
        for p in &raw {
            let file = self.resolve(p.get("type").or(p.get("file")).and_then(|f| f.as_str()).ok_or("pieces[].type is needed")?, Some(true))?;
            self.wait_spline(&file)?;
            let segs: Vec<Seg> = if let Some(s) = p.get("segs").and_then(|s| s.as_array()) {
                s.iter()
                    .map(|s| {
                        let g = |k: &str| s.get(k).and_then(|x| x.as_f64());
                        Ok(Seg { start: DVec2::new(g("x").ok_or("segs[].x")?, g("y").ok_or("segs[].y")?), heading: g("heading").ok_or("segs[].heading")?, length: g("length").ok_or("segs[].length")?, radius: g("radius").unwrap_or(0.0) })
                    })
                    .collect::<Result<_, String>>()?
            } else {
                let pts: Vec<DVec2> = p.get("points").and_then(|v| v.as_array()).ok_or("pieces[].points or segs is needed")?.iter().map(|v| self.point(v)).collect::<Result<_, _>>()?;
                crate::ed_author::chain_through(&pts, p.get("start_heading").and_then(|x| x.as_f64()), p.get("end_heading").and_then(|x| x.as_f64()))?
            };
            all.extend(segs.iter().flat_map(|s| [s.start, s.end()]));
            pieces.push((file, segs, p.get("lift").and_then(|x| x.as_f64()).unwrap_or(0.0)));
        }
        if all.is_empty() {
            return Err("no pieces".into());
        }
        let origin = match a.get("center") {
            Some(c) => self.point(c)?,
            None => all.iter().sum::<DVec2>() / all.len() as f64,
        };
        let mut layout = crate::cjunction::Layout { name: a.str("name").unwrap_or("mcp junction").to_string(), ..Default::default() };
        layout.build.plate = a.bool("plate").unwrap_or(true);
        if let Some(r) = a.f64("radius") {
            layout.build.radius = r;
        }
        for (file, segs, lift) in pieces {
            layout.pieces.push(crate::cjunction::Piece { file, segs: segs.iter().map(|s| crate::cjunction::LSeg { x: s.start.x - origin.x, y: s.start.y - origin.y, heading: s.heading, radius: s.radius, length: s.length }).collect(), lift, hidden: false, auto: false });
        }
        for o in a.arr("objects").into_iter().flatten() {
            let file = self.resolve(o.get("type").or(o.get("file")).and_then(|f| f.as_str()).ok_or("objects[].type is needed")?, Some(false))?;
            let at = self.point(o.get("at").ok_or("objects[].at is needed")?)?;
            layout.objects.push(crate::cjunction::LObject { file, x: at.x - origin.x, y: at.y - origin.y, z: o.get("z").and_then(|x| x.as_f64()).unwrap_or(0.0), heading: o.get("heading").and_then(|x| x.as_f64()).unwrap_or(0.0) });
        }
        self.ed.place_layout(&mut self.assets, &layout, origin, 0.0);
        let errs: Vec<String> = self.ed.toasts.iter().map(|t| t.text.clone()).collect();
        let sel = self.ed.selection();
        if sel.is_empty() {
            return Err(format!("nothing was put: {}", errs.join("; ")));
        }
        Ok(json!({ "ids": sel.iter().filter_map(|s| match s { Sel::Spline(_, id) | Sel::Object(_, id) => Some(*id), _ => None }).collect::<Vec<_>>(), "origin": [number(origin.x), number(origin.y)] }))
    }
}

impl Server {
    /// `street_check`: what the street model forbids, found on the map (in a circle or all of
    /// it); with `fix` the fixable ones put right (one undo step).
    pub fn street_check(&mut self, a: Args) -> Done {
        let area = match a.get("center") {
            Some(c) => Some((self.point(c)?, a.f64("radius").unwrap_or(150.0))),
            None => None,
        };
        let kinds = a.strings("kinds");
        let doc = self.ed.doc.as_ref().ok_or("no map")?;
        let mut found = crate::street_check::check(doc, &self.lib, &mut self.assets, area);
        if !kinds.is_empty() {
            found.retain(|i| kinds.iter().any(|k| k == i.kind));
        }
        let mut fixed = 0;
        if a.bool("fix").unwrap_or(false) && found.iter().any(|i| i.fix.is_some()) {
            self.ed.history.begin("Street check: put right");
            let doc = self.ed.doc.as_mut().unwrap();
            let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
            for k in keys {
                self.ed.history.touch_tile(doc, k);
            }
            fixed = crate::street_check::apply(doc, &found);
            self.ed.history.commit();
        }
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for i in &found {
            *counts.entry(i.kind).or_default() += 1;
        }
        let limit = a.usize("limit").unwrap_or(200);
        Ok(json!({
            "counts": counts, "fixed": fixed,
            "issues": found.iter().take(limit).map(|i| json!({"kind": i.kind, "at": [number(i.at.x), number(i.at.y)], "ids": i.ids, "detail": i.detail, "fixable": i.fix.is_some()})).collect::<Vec<_>>(),
        }))
    }
}

impl Server {
    /// `paint_area`: a ground layer laid over areas (closed outlines).
    pub fn paint_area(&mut self, a: Args) -> Done {
        let layer = match (a.usize("layer"), a.str("texture")) {
            (Some(l), _) => l,
            (None, Some(t)) => self.ground_texture(t)?,
            (None, None) => return Err("`layer` or `texture` is needed".into()),
        };
        let mut polys: Vec<Vec<DVec2>> = Vec::new();
        for p in a.arr("areas").ok_or("`areas` is needed: [[[x, y], ...], ...]")? {
            let ring: Vec<DVec2> = p.as_array().ok_or("an area is a list of points")?.iter().map(|v| self.point(v)).collect::<Result<_, _>>()?;
            if ring.len() >= 3 {
                polys.push(ring);
            }
        }
        let n = self.ed.paint_areas(layer, &polys, a.f64("feather").unwrap_or(1.0).clamp(0.0, 20.0), a.bool("erase").unwrap_or(false))?;
        Ok(json!({ "layer": layer, "areas": polys.len(), "texels": n }))
    }
}

impl Server {
    /// `track_beds`: the ground made the tracks' bed (see `Editor::track_beds`).
    pub fn track_beds(&mut self, a: Args) -> Done {
        let layer = match a.str("texture") {
            Some(t) => Some(self.ground_texture(t)?),
            None => a.usize("layer"),
        };
        let r = self.ed.track_beds(&mut self.assets, a.f64("top").unwrap_or(0.64), a.f64("core").unwrap_or(3.2), a.f64("blend").unwrap_or(6.0), layer, a.f64("paint").unwrap_or(3.4));
        Ok(serde_json::to_value(r).map_err(|e| e.to_string())?)
    }
}

impl Server {
    /// `form_ground`: the ground formed once under everything on it (see `Editor::form_ground`).
    pub fn form_ground(&mut self, a: Args) -> Done {
        let layer = match a.str("texture") {
            Some(t) => Some(self.ground_texture(t)?),
            None => a.usize("layer"),
        };
        let set = crate::ed_ground::FormSettings {
            bed_top: a.f64("bed_top").unwrap_or(0.60),
            bed_half: a.f64("bed_half").unwrap_or(2.4),
            blend: a.f64("blend").unwrap_or(6.0).max(0.5),
            limit: a.f64("limit").unwrap_or(1.5),
            layer,
            paint_extra: a.f64("paint_extra").unwrap_or(0.4),
            dry_run: a.bool("dry_run").unwrap_or(false),
            region: match (a.get("min"), a.get("max")) {
                (Some(x), Some(y)) => Some((self.point(x)?, self.point(y)?)),
                _ => None,
            },
        };
        let r = self.ed.form_ground(&mut self.assets, &set);
        Ok(serde_json::to_value(r).map_err(|e| e.to_string())?)
    }

    pub fn ground_under(&mut self, _a: Args) -> Done {
        let n = self.ed.ground_under(&mut self.assets);
        Ok(json!({ "samples_lowered": n }))
    }
}

fn anyhow_none(bad: bool, what: &str) -> Result<(), String> {
    if bad {
        Err(what.to_string())
    } else {
        Ok(())
    }
}
