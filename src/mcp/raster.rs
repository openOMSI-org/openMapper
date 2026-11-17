//! `raster`: what the map has where, as a grid of classes (a grey PNG, one byte a cell), for
//! comparing the map with imagery cell by cell: carriageways, footways, paths, tracks, plates and
//! surfaces, buildings, other objects, trees.

use glam::{DVec2, Vec3};
use serde_json::json;

use super::{number, Args, Out, Reply, Server};
use crate::doc::tile_origin;
use crate::library::Category;

pub const NONE: u8 = 0;
pub const CARRIAGEWAY: u8 = 1;
pub const FOOTWAY: u8 = 2;
pub const PATH: u8 = 3;
pub const TRACK: u8 = 4;
pub const SURFACE: u8 = 5;
pub const BUILDING: u8 = 6;
pub const OBJECT: u8 = 7;
pub const TREE: u8 = 8;

impl Server {
    pub fn raster(&mut self, a: Args) -> Reply {
        self.wait_lib();
        let lo = self.point(a.get("min").ok_or("`min` is needed")?)?;
        let hi = self.point(a.get("max").ok_or("`max` is needed")?)?;
        let cell = a.f64("cell").unwrap_or(1.0).clamp(0.1, 10.0);
        let (w, h) = (((hi.x - lo.x) / cell).ceil().max(1.0) as usize, ((hi.y - lo.y) / cell).ceil().max(1.0) as usize);
        if w * h > 40_000_000 {
            return Err("too many cells: a larger `cell` or a smaller box".into());
        }
        let doc = self.ed.doc.as_ref().ok_or("no map")?;
        self.assets.load_map_models(doc, 60_000);
        let mut g = vec![NONE; w * h];
        // (higher classes win where things overlap, but a carriageway is never painted over by
        // its own footway)
        let rank = |c: u8| match c {
            NONE => 0,
            TREE => 1,
            OBJECT => 2,
            PATH => 3,
            FOOTWAY => 4,
            SURFACE => 5,
            TRACK => 6,
            CARRIAGEWAY => 7,
            BUILDING => 8,
            _ => 0,
        };
        let put = |g: &mut Vec<u8>, p: DVec2, c: u8| {
            let (x, y) = (((p.x - lo.x) / cell).floor(), ((p.y - lo.y) / cell).floor());
            if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
                return;
            }
            // (row 0 is the north edge, as in a picture)
            let i = (h - 1 - y as usize) * w + x as usize;
            if rank(c) > rank(g[i]) {
                g[i] = c;
            }
        };
        let inside = |p: DVec2, m: f64| p.x >= lo.x - m && p.y >= lo.y - m && p.x <= hi.x + m && p.y <= hi.y + m;
        for (k, t) in &doc.tiles {
            let org = tile_origin(*k);
            for sp in t.tile.splines.iter().filter(|s| !s.deleted) {
                if !inside(org + DVec2::new(sp.pos[0], sp.pos[1]), sp.length + 30.0) {
                    continue;
                }
                let cat = self.lib.find(&sp.file).map(|x| x.category);
                let Some(model) = self.assets.spline(&sp.file) else { continue };
                let cat = cat.unwrap_or(if model.is_track() { Category::Tracks } else { Category::Roads });
                let m = if sp.mirror { -1.0 } else { 1.0 };
                // the cross-section: the carriageway where the street lanes run, the whole width
                let lanes: Vec<(f64, f64)> = model.def.paths.iter().filter(|q| q.kind == 0).map(|q| (q.start[0] as f64 * m, q.width as f64)).collect();
                let carriage = (!lanes.is_empty()).then(|| (lanes.iter().map(|(x, w)| x - w / 2.0).fold(f64::MAX, f64::min), lanes.iter().map(|(x, w)| x + w / 2.0).fold(f64::MIN, f64::max)));
                let span = model.def.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.x as f64 * m).fold((f64::MAX, f64::MIN), |a, x| (a.0.min(x), a.1.max(x)));
                if span.0 > span.1 {
                    continue;
                }
                let c = omsi_geometry::SplineCurve::from_map(sp, org);
                let n = ((sp.length / (cell * 0.5)).ceil() as usize).max(1);
                for i in 0..=n {
                    let s = sp.length * i as f64 / n as f64;
                    let p = c.point_at(s).truncate();
                    let hd = c.heading_at(s).to_radians();
                    let right = DVec2::new(hd.cos(), -hd.sin());
                    let mut x = span.0;
                    while x <= span.1 {
                        let class = match cat {
                            Category::Roads => match carriage {
                                Some((a, b)) if x >= a && x <= b => CARRIAGEWAY,
                                Some(_) => FOOTWAY,
                                None => if model.half_width <= 2.5 { PATH } else { CARRIAGEWAY },
                            },
                            Category::Paths => PATH,
                            Category::Tracks => TRACK,
                            _ => NONE,
                        };
                        if class != NONE {
                            put(&mut g, p + right * x, class);
                        }
                        x += cell * 0.5;
                    }
                }
            }
            for o in &t.tile.objects {
                let p = org + DVec2::new(o.pos[0], o.pos[1]);
                if !inside(p, 80.0) {
                    continue;
                }
                let Some(model) = self.assets.object(&o.file) else { continue };
                if model.technical || model.editor_parts {
                    continue;
                }
                let (sx, sy) = ((model.hi.x - model.lo.x) as f64, (model.hi.y - model.lo.y) as f64);
                let tall = (model.hi.z - model.lo.z) as f64;
                // (trees drawn as crossed cards are objects too: their folders say what they are)
                let named_tree = crate::street_check::is_tree_file(&o.file);
                let class = if model.tree.is_some() || named_tree {
                    TREE
                } else if model.surface || !model.paths.is_empty() && tall < 1.0 {
                    SURFACE
                } else if tall >= 2.5 && sx.min(sy) >= 3.0 && sx * sy >= 15.0 {
                    BUILDING
                } else {
                    OBJECT
                };
                if class == TREE {
                    put(&mut g, p, TREE);
                    continue;
                }
                let pose = crate::place::Pose::of_object(glam::DVec3::new(p.x, p.y, 0.0), o.rot);
                let step = (cell * 0.5) as f32;
                let mut y = model.lo.y;
                while y <= model.hi.y {
                    let mut x = model.lo.x;
                    while x <= model.hi.x {
                        let q = pose.rot.transform_vector3(Vec3::new(x, y, 0.0)).as_dvec3();
                        put(&mut g, p + q.truncate(), class);
                        x += step;
                    }
                    y += step;
                }
            }
        }
        let img = image::GrayImage::from_raw(w as u32, h as u32, g).ok_or("raster")?;
        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        if let Some(f) = a.str("file") {
            std::fs::write(f, &png).map_err(|e| format!("{f}: {e}"))?;
        }
        Ok(Out::Image { png, meta: json!({ "min": [number(lo.x), number(lo.y)], "cell": cell, "cells": [w, h], "rows": "row 0 is the north edge", "classes": { "0": "none", "1": "carriageway", "2": "footway", "3": "path", "4": "track", "5": "plate or surface", "6": "building", "7": "other object", "8": "tree" } }) })
    }
}
