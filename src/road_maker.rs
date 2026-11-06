//! Road types made in the editor and how junctions treat each road type.
//!
//! A road type is written as an ordinary OMSI 2 spline (`.sli`) into
//! `Splines\openMapper\<name>.sli` of the content folder, laid out as the stock roads are:
//! a roadway of lanes (one texture across it), a kerb and a footway each side, height
//! profiles for the ground and the wheels, and a `[path]` for every lane and footway. The
//! textures it names are copied beside it (with their `.cfg` / `.surf` sidecars), so it
//! works on any machine the map goes to.
//!
//! Junction styles (`openMapper\road_styles.json` of the content folder) say, per road
//! type, how large its junctions' corners are and whether they get lights and zebra
//! crossings by themselves.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How a painted line along the road looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkStyle {
    Solid,
    Dashed,
    /// Two solid lines side by side (no overtaking).
    Double,
    /// A solid line with a dashed one beside it (overtaking from the dashed side).
    SolidDashed,
}

/// One painted line along the roadway: where, how it looks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marking {
    /// Across the roadway from its middle (m, right of the road's way positive).
    pub x: f32,
    pub style: MarkStyle,
    pub color: [u8; 3],
    /// The line's width (m); a double line's lines are this wide each.
    pub width: f32,
    /// A dashed line's dash and the gap after it (m).
    pub dash: f32,
    pub gap: f32,
}

pub const WHITE: [u8; 3] = [235, 235, 230];
pub const YELLOW: [u8; 3] = [232, 190, 60];

impl Default for Marking {
    fn default() -> Self {
        Marking { x: 0.0, style: MarkStyle::Solid, color: WHITE, width: 0.12, dash: 3.0, gap: 6.0 }
    }
}

impl Marking {
    /// The lines it is made of: across position, and whether dashed.
    pub fn lines_at(&self) -> Vec<(f32, bool)> {
        let off = (0.1 + self.width) / 2.0;
        match self.style {
            MarkStyle::Solid => vec![(self.x, false)],
            MarkStyle::Dashed => vec![(self.x, true)],
            MarkStyle::Double => vec![(self.x - off, false), (self.x + off, false)],
            MarkStyle::SolidDashed => vec![(self.x - off, false), (self.x + off, true)],
        }
    }
}

/// What is drawn on the roadway by hand: a transparent picture laid over the roadway texture
/// (`HAND_W` across, `HAND_H` along `mark_period` metres), drawn in metres and wrapping where
/// the texture repeats so a stroke leaving at one end comes in at the other.
#[derive(Debug, Clone, PartialEq)]
pub struct Hand {
    pub img: image::RgbaImage,
}

pub const HAND_W: u32 = 1024;
pub const HAND_H: u32 = 512;

impl Default for Hand {
    fn default() -> Self {
        Hand { img: image::RgbaImage::new(HAND_W, HAND_H) }
    }
}

impl Hand {
    pub fn is_empty(&self) -> bool {
        self.img.pixels().all(|p| p.0[3] == 0)
    }

    /// Lay `color` over the pixels `cover(x_m, y_m)` says (0..1) within a box, wrapped along.
    fn paint(&mut self, cw: f32, len: f32, lo: (f32, f32), hi: (f32, f32), color: [u8; 3], erase: bool, cover: &dyn Fn(f32, f32) -> f32) {
        let (kx, ky) = (HAND_W as f32 / cw, HAND_H as f32 / len);
        let x0 = ((lo.0 * kx).floor() as i64).max(0);
        let x1 = ((hi.0 * kx).ceil() as i64).min(HAND_W as i64 - 1);
        let y0 = (lo.1 * ky).floor() as i64;
        let y1 = (hi.1 * ky).ceil() as i64;
        for py in y0..=y1 {
            let row = py.rem_euclid(HAND_H as i64) as u32;
            for px in x0..=x1 {
                let (mx, my) = ((px as f32 + 0.5) / kx, (py as f32 + 0.5) / ky);
                let c = cover(mx, my).clamp(0.0, 1.0);
                if c <= 0.0 {
                    continue;
                }
                let p = self.img.get_pixel_mut(px as u32, row);
                let a = p.0[3] as f32 / 255.0;
                if erase {
                    p.0[3] = (a * (1.0 - c) * 255.0).round() as u8;
                } else {
                    let out = c + a * (1.0 - c);
                    for k in 0..3 {
                        p.0[k] = ((color[k] as f32 * c + p.0[k] as f32 * a * (1.0 - c)) / out.max(1e-6)).round() as u8;
                    }
                    p.0[3] = (out * 255.0).round() as u8;
                }
            }
        }
    }

    /// A line from `a` to `b` (m across from the left edge, m along), `r` m each side of it,
    /// with round ends and soft edges.
    pub fn stroke(&mut self, cw: f32, len: f32, a: (f32, f32), b: (f32, f32), r: f32, color: [u8; 3], erase: bool) {
        let soft = (cw / HAND_W as f32).max(len / HAND_H as f32);
        let (ab, l2) = ((b.0 - a.0, b.1 - a.1), ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).max(1e-9));
        let dist = move |x: f32, y: f32| {
            let t = (((x - a.0) * ab.0 + (y - a.1) * ab.1) / l2).clamp(0.0, 1.0);
            ((x - (a.0 + ab.0 * t)).powi(2) + (y - (a.1 + ab.1 * t)).powi(2)).sqrt()
        };
        let cover = move |x: f32, y: f32| (r - dist(x, y)) / soft + 0.5;
        let (lo, hi) = ((a.0.min(b.0) - r - soft, a.1.min(b.1) - r - soft), (a.0.max(b.0) + r + soft, a.1.max(b.1) + r + soft));
        // (the stroke and its copies a texture length either way: what runs off one end comes in at the other)
        self.paint(cw, len, lo, hi, color, erase, &|x, y| cover(x, y).max(cover(x, y + len)).max(cover(x, y - len)));
    }

    /// The polygon `pts` filled.
    pub fn fill(&mut self, cw: f32, len: f32, pts: &[(f32, f32)], color: [u8; 3], erase: bool) {
        if pts.len() < 3 {
            return;
        }
        let inside = |x: f32, y: f32| {
            let mut on = false;
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
                    on = !on;
                }
            }
            on
        };
        let (kx, ky) = (HAND_W as f32 / cw, HAND_H as f32 / len);
        let (hx, hy) = (0.25 / kx, 0.25 / ky);
        let cover = |x: f32, y: f32| {
            let mut n = 0;
            for (dx, dy) in [(-hx, -hy), (hx, -hy), (-hx, hy), (hx, hy)] {
                if inside(x + dx, y + dy) || inside(x + dx, y + dy + len) || inside(x + dx, y + dy - len) {
                    n += 1;
                }
            }
            n as f32 / 4.0
        };
        let lo = pts.iter().fold((f32::MAX, f32::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
        let hi = pts.iter().fold((f32::MIN, f32::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
        self.paint(cw, len, (lo.0 - 0.05, lo.1 - 0.05), (hi.0 + 0.05, hi.1 + 0.05), color, erase, &cover);
    }

    /// The drawing laid over `base` (same size).
    pub fn over(&self, base: &mut image::RgbaImage) {
        for (b, h) in base.pixels_mut().zip(self.img.pixels()) {
            let a = h.0[3] as f32 / 255.0 * 0.97;
            if a > 0.0 {
                for k in 0..3 {
                    b.0[k] = (b.0[k] as f32 + (h.0[k] as f32 - b.0[k] as f32) * a) as u8;
                }
            }
        }
    }
}

/// An arrow of the road marking's kind, pointing along (+y) from `at`: a shaft and a head, 3 m
/// long; `back` turns it round.
pub fn arrow_polygon(at: (f32, f32), back: bool) -> Vec<(f32, f32)> {
    let s = if back { -1.0 } else { 1.0 };
    [(-0.15, -1.5), (0.15, -1.5), (0.15, 0.3), (0.55, 0.3), (0.0, 1.5), (-0.55, 0.3), (-0.15, 0.3)].iter().map(|&(x, y)| (at.0 + x, at.1 + s * y)).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoadSpec {
    pub name: String,
    /// Lanes the way the road is drawn, and against it (0: one-way).
    pub lanes_fwd: u8,
    pub lanes_back: u8,
    pub lane_width: f32,
    /// A strip between the two ways (m, 0: none).
    pub median: f32,
    /// The strip stands as high as the footways, with a kerb along it (else it lies level with
    /// the roadway).
    pub median_raised: bool,
    /// Footway each side (m, 0: none) and the kerb's height. The footway on the right is the
    /// same unless `footway_right` says otherwise.
    pub footway: f32,
    pub footway_right: Option<f32>,
    pub kerb: f32,
    /// A bus pocket: a lane (m, 0: none) beside the roadway on the right, between it and the
    /// footway, for buses to stop in. A road type made with one is what the Roads tool's bus
    /// pocket tool lays where a bay is wanted.
    pub pocket: f32,
    /// Texture files (found in the installation or the content folder).
    pub roadway_tex: PathBuf,
    pub footway_tex: PathBuf,
    /// The middle strip's own texture (grass, paving); the roadway's runs on under it if none.
    pub median_tex: PathBuf,
    /// The lines are painted by the maker (`markings`) onto the roadway texture, which should
    /// be plain asphalt, instead of being the ones the texture has.
    pub marked: bool,
    pub markings: Vec<Marking>,
    /// What is drawn over them by hand (kept beside the road as `<name>.marks.png`).
    #[serde(skip)]
    pub hand: Option<std::sync::Arc<Hand>>,
}

impl Default for RoadSpec {
    fn default() -> Self {
        RoadSpec { name: String::new(), lanes_fwd: 1, lanes_back: 1, lane_width: 3.5, median: 0.0, median_raised: false, footway: 3.0, footway_right: None, kerb: 0.15, pocket: 0.0, roadway_tex: PathBuf::new(), footway_tex: PathBuf::new(), median_tex: PathBuf::new(), marked: false, markings: Vec::new(), hand: None }
    }
}

fn num(v: f32) -> String {
    format!("{v:.3}")
}

/// A ready-made start for the road maker: (name, what it is made of).
pub fn presets() -> Vec<(&'static str, RoadSpec)> {
    use crate::i18n::tr;
    let base = RoadSpec::default();
    vec![
        (tr("Quiet street", "Тихая улица"), RoadSpec { lane_width: 3.0, footway: 2.0, ..base.clone() }),
        (tr("Street with a footway on one side", "Улица с тротуаром с одной стороны"), RoadSpec { lane_width: 3.0, footway: 0.0, footway_right: Some(2.0), ..base.clone() }),
        (tr("Main road", "Главная дорога"), RoadSpec { lanes_fwd: 2, lanes_back: 2, lane_width: 3.25, footway: 3.0, ..base.clone() }),
        (tr("Avenue with a green strip", "Проспект с газоном"), RoadSpec { lanes_fwd: 2, lanes_back: 2, lane_width: 3.25, median: 4.0, median_raised: true, footway: 3.5, ..base.clone() }),
        (tr("One-way street", "Улица с односторонним движением"), RoadSpec { lanes_fwd: 2, lanes_back: 0, lane_width: 3.0, footway: 2.5, ..base.clone() }),
        (tr("Street with a bus pocket", "Улица с автобусным карманом"), RoadSpec { lane_width: 3.0, footway: 2.5, pocket: 3.0, ..base.clone() }),
        (tr("Country road", "Просёлочная дорога"), RoadSpec { lane_width: 3.0, footway: 0.0, kerb: 0.0, ..base.clone() }),
        (tr("Service road", "Служебный проезд"), RoadSpec { lanes_fwd: 1, lanes_back: 0, lane_width: 3.5, footway: 0.0, kerb: 0.0, ..base }),
    ]
}

impl RoadSpec {
    pub fn roadway_width(&self) -> f32 {
        (self.lanes_fwd + self.lanes_back) as f32 * self.lane_width + self.median_width()
    }

    /// The middle strip's width (none on a one-way road).
    pub fn median_width(&self) -> f32 {
        if self.lanes_fwd > 0 && self.lanes_back > 0 { self.median } else { 0.0 }
    }

    pub fn footway_left(&self) -> f32 {
        self.footway
    }

    pub fn footway_right(&self) -> f32 {
        self.footway_right.unwrap_or(self.footway)
    }

    pub fn total_width(&self) -> f32 {
        self.roadway_width() + self.pocket + self.footway_left() + self.footway_right()
    }

    /// Position of the line between the opposite lanes.
    pub fn centerline(&self) -> f32 {
        (self.lanes_back as f32 - self.lanes_fwd as f32) * self.lane_width / 2.0
    }

    /// Where a place across the roadway (m from its middle) lies in the roadway texture (0..1).
    pub fn roadway_u(&self, x: f32) -> f32 {
        let rw = self.roadway_width();
        let (xl, mid) = (-rw / 2.0, self.median_width());
        if mid > 0.0 && !self.median_tex.as_os_str().is_empty() {
            let lanes_w = (rw - mid).max(0.1);
            let u_mid = (-mid / 2.0 - xl) / lanes_w;
            if x <= -mid / 2.0 {
                (x - xl) / lanes_w
            } else if x >= mid / 2.0 {
                u_mid + (x - mid / 2.0) / lanes_w
            } else {
                u_mid
            }
        } else {
            (x - xl) / rw.max(0.1)
        }
    }

    /// How wide the roadway texture lies across the road (m): the roadway, less a middle strip
    /// of its own, which has its own texture.
    pub fn canvas_width(&self) -> f32 {
        let own = self.median_width() > 0.0 && !self.median_tex.as_os_str().is_empty();
        (self.roadway_width() - if own { self.median_width() } else { 0.0 }).max(0.5)
    }

    /// How far along the road the painted roadway texture runs before it repeats (m): about
    /// ten, a whole number of the first dashed line's periods so its dashes go on unbroken.
    pub fn mark_period(&self) -> f32 {
        let first = self.markings.iter().filter(|m| matches!(m.style, MarkStyle::Dashed | MarkStyle::SolidDashed)).map(|m| (m.dash + m.gap).max(0.5)).next().unwrap_or(10.0);
        (first * (10.0 / first).round().max(1.0)).clamp(2.0, 40.0)
    }

    /// A dashed line's period as painted: the texture's length holds a whole number of them.
    pub fn dash_period(&self, m: &Marking) -> f32 {
        let l = self.mark_period();
        l / (l / (m.dash + m.gap).max(0.5)).round().max(1.0)
    }

    /// The v scale of the roadway's profiles: 0.1 (ten metres to the texture), or the painted
    /// texture's own length.
    fn roadway_v(&self) -> f32 {
        if self.marked { 1.0 / self.mark_period() } else { 0.1 }
    }

    fn roadway_tex_name(&self) -> String {
        if self.marked {
            format!("{}_lines.bmp", self.file_stem())
        } else {
            self.roadway_tex.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
        }
    }

    /// The markings a road like this usually has: edge lines, the middle (double yellow, or
    /// white edge lines of the strip), dashed lines between lanes of one way.
    pub fn standard_markings(&self) -> Vec<Marking> {
        let (rw, mid, lw) = (self.roadway_width(), self.median_width(), self.lane_width);
        let two_way = self.lanes_fwd > 0 && self.lanes_back > 0;
        let line = |x: f32, style: MarkStyle, color: [u8; 3]| Marking { x, style, color, ..Default::default() };
        let mut v = Vec::new();
        let edge = if self.pocket > 0.0 { MarkStyle::Dashed } else { MarkStyle::Solid };
        let offset = self.centerline();
        v.push(line(-rw / 2.0 + 0.2, MarkStyle::Solid, WHITE));
        v.push(line(rw / 2.0 - 0.2, edge, WHITE));
        if two_way {
            let gap = mid / 2.0;
            if mid <= 0.0 {
                v.push(line(offset, MarkStyle::Double, YELLOW));
            } else {
                v.push(line(offset - gap - 0.2, MarkStyle::Solid, WHITE));
                v.push(line(offset + gap + 0.2, MarkStyle::Solid, WHITE));
            }
            for i in 1..self.lanes_back {
                v.push(line(offset - gap - i as f32 * lw, MarkStyle::Dashed, WHITE));
            }
            for i in 1..self.lanes_fwd {
                v.push(line(offset + gap + i as f32 * lw, MarkStyle::Dashed, WHITE));
            }
        } else {
            for i in 1..(self.lanes_fwd + self.lanes_back) {
                v.push(line(-rw / 2.0 + i as f32 * lw, MarkStyle::Dashed, WHITE));
            }
        }
        v
    }

    /// The roadway texture with the markings painted on: the texture's picture stretched
    /// across the roadway as it is laid (u across, v along `mark_period` metres), lines drawn
    /// with soft edges.
    pub fn painted_roadway(&self) -> Option<image::RgbaImage> {
        let img = omsi_texture::decode_file(&self.roadway_tex).ok()?;
        let src = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.rgba.clone())?;
        let (w, h) = (1024usize, 512usize);
        let mut buf = image::imageops::resize(&src, w as u32, h as u32, image::imageops::FilterType::Triangle);
        let len = self.mark_period();
        for m in &self.markings {
            let period = self.dash_period(m);
            for (x, dashed) in m.lines_at() {
                let (a, b) = (self.roadway_u(x - m.width / 2.0) * w as f32, self.roadway_u(x + m.width / 2.0) * w as f32);
                let (lo, hi) = (a.min(b), a.max(b));
                for px in (lo.floor().max(0.0) as usize)..(hi.ceil().max(0.0) as usize).min(w) {
                    let cover = (hi.min(px as f32 + 1.0) - lo.max(px as f32)).clamp(0.0, 1.0);
                    if cover <= 0.0 {
                        continue;
                    }
                    for py in 0..h {
                        if dashed && ((py as f32 + 0.5) / h as f32 * len) % period >= m.dash.min(period) {
                            continue;
                        }
                        let p = buf.get_pixel_mut(px as u32, py as u32);
                        for c in 0..3 {
                            p.0[c] = (p.0[c] as f32 + (m.color[c] as f32 - p.0[c] as f32) * cover * 0.96) as u8;
                        }
                    }
                }
            }
        }
        if let Some(h) = &self.hand {
            h.over(&mut buf);
        }
        Some(buf)
    }

    /// The file name it is saved as.
    pub fn file_stem(&self) -> String {
        let s: String = self.name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        if s.is_empty() { "road".into() } else { s }
    }

    /// The `.sli` text (OMSI's own keywords; `\n`, turned into CR LF when written).
    pub fn sli(&self) -> String {
        let rw = self.roadway_width();
        let (xl, xr) = (-rw / 2.0, rw / 2.0);
        let (fl, fr) = (self.footway_left(), self.footway_right());
        let pw = self.pocket.max(0.0);
        // (the right footway and its kerb start past the pocket)
        let xr2 = xr + pw;
        let road_h = 0.10f32;
        let side_h = road_h + self.kerb;
        let mid = self.median_width();
        let own_median = mid > 0.0 && !self.median_tex.as_os_str().as_encoded_bytes().is_empty();
        let raised = mid > 0.0 && self.median_raised;
        let rv = self.roadway_v();
        let mut s = format!("File created with openMapper\n\n[friendlyname]\n{}\n\n", self.name.trim());
        // the ground and the wheels: footways high, the roadway low
        let height = |s: &mut String, a: f32, b: f32, h: f32| {
            let _ = write!(s, "[heightprofile]\n{}\n{}\n{}\n{}\n\n", num(a), num(b), num(h), num(h));
        };
        if fl > 0.0 {
            height(&mut s, xl - fl, xl, side_h);
        }
        if raised {
            height(&mut s, xl, -mid / 2.0, road_h);
            height(&mut s, -mid / 2.0, mid / 2.0, side_h);
            height(&mut s, mid / 2.0, xr, road_h);
        } else {
            height(&mut s, xl, xr, road_h);
        }
        if pw > 0.0 {
            height(&mut s, xr, xr2, road_h);
        }
        if fr > 0.0 {
            height(&mut s, xr2, xr2 + fr, side_h);
        }
        let name = |p: &Path| p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let _ = write!(s, "[texture]\n{}\n\n", self.roadway_tex_name());
        let side_tex = (fl > 0.0 || fr > 0.0 || raised) && !self.footway_tex.as_os_str().is_empty();
        let mut next = 1u32;
        let mut side_ix = 0;
        if side_tex {
            let _ = write!(s, "[texture]\n{}\n\n", name(&self.footway_tex));
            side_ix = next;
            next += 1;
        }
        let mut median_ix = 0;
        if own_median {
            let _ = write!(s, "[texture]\n{}\n\n", name(&self.median_tex));
            median_ix = next;
        }
        let profile = |s: &mut String, tex: u32, a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
            let _ = write!(s, "[profile]\n{tex}\n\n");
            for p in [a, b] {
                let _ = write!(s, "[profilepnt]\n{}\n{}\n{}\n{}\n\n", num(p.0), num(p.1), num(p.2), num(p.3));
            }
        };
        // the roadway: one texture across it, u 0 at its left edge and 1 at its right (as the
        // stock lane textures are painted); a strip of its own texture in the middle takes its
        // own place, the roadway's u running across the lanes either side of it
        if own_median {
            let lanes_w = rw - mid;
            let u_mid = ((-mid / 2.0 - xl) / lanes_w).clamp(0.0, 1.0);
            let y = |on: bool| if on { side_h } else { road_h };
            profile(&mut s, 0, (xl, road_h, 0.0, rv), (-mid / 2.0, road_h, u_mid, rv));
            profile(&mut s, 0, (mid / 2.0, road_h, u_mid, rv), (xr, road_h, 1.0, rv));
            if raised && side_tex {
                profile(&mut s, side_ix, (-mid / 2.0, road_h, 0.995, 0.2), (-mid / 2.0, y(true), 0.953, 0.2));
                profile(&mut s, side_ix, (mid / 2.0, y(true), 0.953, 0.2), (mid / 2.0, road_h, 0.995, 0.2));
            }
            profile(&mut s, median_ix, (-mid / 2.0, y(raised), 0.0, mid), (mid / 2.0, y(raised), 1.0, mid));
        } else {
            profile(&mut s, 0, (xl, road_h, 0.0, rv), (xr, road_h, 1.0, rv));
            if raised && side_tex {
                profile(&mut s, side_ix, (-mid / 2.0, road_h, 0.995, 0.2), (-mid / 2.0, side_h, 0.953, 0.2));
                profile(&mut s, side_ix, (-mid / 2.0, side_h, 0.953, 0.2), (mid / 2.0, side_h, 0.187, 0.2));
                profile(&mut s, side_ix, (mid / 2.0, side_h, 0.953, 0.2), (mid / 2.0, road_h, 0.995, 0.2));
            }
        }
        if side_tex {
            // kerb face and footway top, on the stock footway texture's kerb and pavement parts
            let t = side_ix;
            if fl > 0.0 {
                profile(&mut s, t, (xl - fl, side_h, 0.187, 0.2), (xl, side_h, 0.953, 0.2));
                profile(&mut s, t, (xl, side_h, 0.953, 0.2), (xl, road_h, 0.995, 0.2));
            }
            if fr > 0.0 {
                profile(&mut s, t, (xr2, road_h, 0.995, 0.2), (xr2, side_h, 0.953, 0.2));
                profile(&mut s, t, (xr2, side_h, 0.953, 0.2), (xr2 + fr, side_h, 0.187, 0.2));
            }
        }
        if pw > 0.0 {
            // the pocket: the roadway's own asphalt at the roadway's scale (u 1 is the roadway's
            // width), centred on the middle of its first lane; squeezed into a sliver of the
            // texture it would be a smear of one column across the whole bay
            let lanes = (self.lanes_fwd + self.lanes_back).max(1) as f32;
            let span = pw / rw.max(0.1);
            let u0 = (0.5 / lanes - span / 2.0).clamp(0.0, (1.0 - span).max(0.0));
            profile(&mut s, 0, (xr, road_h, u0, rv), (xr2, road_h, u0 + span, rv));
        }
        // lanes: against the road's way left of its middle, with it right of it (as OMSI 2's
        // roads are laid out where traffic keeps right; a left-hand map mirrors the spline)
        let mid_gap = mid / 2.0;
        let offset = self.centerline();
        let one_way = self.lanes_back == 0 || self.lanes_fwd == 0;
        if fl > 0.0 {
            let _ = write!(s, "[path]\n1\n{}\n{}\n{}\n2\n\n", num(xl - fl / 2.0), num(side_h), num(fl * 0.4));
        }
        for i in 0..self.lanes_back {
            let x = if one_way { xl + (i as f32 + 0.5) * self.lane_width } else { offset - mid_gap - (self.lanes_back - i) as f32 * self.lane_width + self.lane_width / 2.0 };
            let _ = write!(s, "[path]\n0\n{}\n{}\n{}\n1\n\n", num(x), num(road_h), num(self.lane_width * 0.9));
        }
        for i in 0..self.lanes_fwd {
            let x = if one_way { xl + (i as f32 + 0.5) * self.lane_width } else { offset + mid_gap + (i as f32 + 0.5) * self.lane_width };
            let _ = write!(s, "[path]\n0\n{}\n{}\n{}\n0\n\n", num(x), num(road_h), num(self.lane_width * 0.9));
        }
        if pw > 0.0 {
            // (traffic with the road's way: right of its middle)
            let _ = write!(s, "[path]\n0\n{}\n{}\n{}\n0\n\n", num(xr + pw / 2.0), num(road_h), num(pw * 0.9));
        }
        if fr > 0.0 {
            let _ = write!(s, "[path]\n1\n{}\n{}\n{}\n2\n\n", num(xr2 + fr / 2.0), num(side_h), num(fr * 0.4));
        }
        s
    }

    /// Write it into the content folder with its textures; returns its relative path
    /// (`Splines\openMapper\<name>.sli`).
    pub fn write(&self, content: &Path) -> anyhow::Result<String> {
        anyhow::ensure!(self.lanes_fwd + self.lanes_back > 0, "a road needs a lane");
        anyhow::ensure!(self.roadway_tex.is_file(), "the roadway texture is missing");
        let dir = content.join("Splines").join("openMapper");
        let tex_dir = dir.join("texture");
        std::fs::create_dir_all(&tex_dir)?;
        for t in [&self.roadway_tex, &self.footway_tex, &self.median_tex].into_iter().filter(|t| t.is_file()) {
            let name = t.file_name().unwrap();
            if t.parent() != Some(tex_dir.as_path()) {
                std::fs::copy(t, tex_dir.join(name))?;
                // (its settings and surface sound go with it)
                for ext in ["cfg", "surf"] {
                    let side = PathBuf::from(format!("{}.{ext}", t.display()));
                    if side.is_file() {
                        std::fs::copy(&side, tex_dir.join(format!("{}.{ext}", name.to_string_lossy())))?;
                    }
                }
            }
        }
        let stem = self.file_stem();
        if self.marked {
            let img = self.painted_roadway().ok_or_else(|| anyhow::anyhow!("the roadway texture could not be read"))?;
            image::DynamicImage::ImageRgba8(img).to_rgb8().save(tex_dir.join(self.roadway_tex_name()))?;
        }
        let path = dir.join(format!("{stem}.sli"));
        crate::doc::safe::write_atomic(&path, &omsi_cfg::encode_text(&self.sli().replace('\n', "\r\n")))?;
        // (the spec beside it, so the road can be opened in the maker again)
        crate::doc::safe::write_atomic(&dir.join(format!("{stem}.road.json")), serde_json::to_string_pretty(self)?.as_bytes())?;
        let marks = dir.join(format!("{stem}.marks.png"));
        match self.hand.as_ref().filter(|h| self.marked && !h.is_empty()) {
            Some(h) => h.img.save(&marks)?,
            None => {
                let _ = std::fs::remove_file(&marks);
            }
        }
        Ok(format!("Splines\\openMapper\\{stem}.sli"))
    }

    /// The spec a road type was made with here, if it was.
    pub fn of(sli: &Path) -> Option<RoadSpec> {
        let json = sli.with_extension("road.json");
        let mut spec: RoadSpec = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
        if let Ok(img) = image::open(sli.with_extension("marks.png")) {
            let img = img.to_rgba8();
            if (img.width(), img.height()) == (HAND_W, HAND_H) {
                spec.hand = Some(std::sync::Arc::new(Hand { img }));
            }
        }
        Some(spec)
    }
}

/// How junctions treat a road type.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JunctionStyle {
    /// Corners rounded larger (above 1) or tighter (below 1) than the roads' widths ask.
    pub corner: f64,
    /// New junctions with this road get traffic lights / zebra crossings on every arm.
    pub lights: bool,
    pub crossings: bool,
    /// Where it meets another road a junction is made at all (else the roads only cross).
    pub junctions: bool,
}

impl Default for JunctionStyle {
    fn default() -> Self {
        JunctionStyle { corner: 1.0, lights: false, crossings: false, junctions: true }
    }
}

/// The styles of the road types, by relative path (lower case), and the junction piece each
/// one is joined with by preference (the object's relative path).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Styles {
    #[serde(default)]
    pub styles: HashMap<String, JunctionStyle>,
    #[serde(default)]
    pub pieces: HashMap<String, String>,
}

impl Styles {
    fn path(content: &Path) -> PathBuf {
        content.join("openMapper").join("road_styles.json")
    }

    pub fn load(content: &Path) -> Styles {
        let Ok(text) = std::fs::read_to_string(Self::path(content)) else { return Styles::default() };
        if let Ok(s) = serde_json::from_str::<Styles>(&text) {
            // (the first versions kept just the styles, as a plain map: a style of a road type
            // is an object with "corner" in it, which the named fields do not hold)
            if !s.styles.is_empty() || !s.pieces.is_empty() || text.contains("\"styles\"") {
                return s;
            }
        }
        serde_json::from_str::<HashMap<String, JunctionStyle>>(&text).map(|styles| Styles { styles, pieces: HashMap::new() }).unwrap_or_default()
    }

    pub fn save(&self, content: &Path) -> std::io::Result<()> {
        let p = Self::path(content);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        crate::doc::safe::write_atomic(&p, serde_json::to_string_pretty(self).unwrap_or_default().as_bytes())
    }

    pub fn get(&self, rel: &str) -> JunctionStyle {
        self.styles.get(&rel.to_lowercase()).copied().unwrap_or_default()
    }

    pub fn set(&mut self, rel: &str, s: JunctionStyle) {
        if s == JunctionStyle::default() {
            self.styles.remove(&rel.to_lowercase());
        } else {
            self.styles.insert(rel.to_lowercase(), s);
        }
    }

    /// The junction piece a road type is joined with by preference.
    pub fn piece(&self, rel: &str) -> Option<&str> {
        self.pieces.get(&rel.to_lowercase()).map(String::as_str)
    }

    pub fn set_piece(&mut self, rel: &str, piece: Option<String>) {
        match piece {
            Some(p) => self.pieces.insert(rel.to_lowercase(), p),
            None => self.pieces.remove(&rel.to_lowercase()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_made_road_reads_back_as_omsi_reads_it() {
        let dir = std::env::temp_dir().join(format!("om-roadmaker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tex = dir.join("asph.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([80, 80, 80, 255])).save(&tex).unwrap();
        let side = dir.join("side.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([150, 150, 150, 255])).save(&side).unwrap();
        let spec = RoadSpec { name: "Main street 4".into(), lanes_fwd: 2, lanes_back: 2, lane_width: 3.25, median: 1.0, footway: 2.5, kerb: 0.15, roadway_tex: tex, footway_tex: side, ..Default::default() };
        let content = dir.join("content");
        let rel = spec.write(&content).unwrap();
        assert_eq!(rel, "Splines\\openMapper\\Main_street_4.sli");
        let path = content.join("Splines/openMapper/Main_street_4.sli");
        let s = omsi_scenery::Spline::load(&path).unwrap();
        // four car lanes (two each way), two footways
        let cars: Vec<_> = s.paths.iter().filter(|p| p.kind == 0).collect();
        assert_eq!(cars.len(), 4);
        assert_eq!(s.paths.iter().filter(|p| p.kind == 1).count(), 2);
        assert_eq!(cars.iter().filter(|p| p.start[0] > 0.0 && p.direction == 0).count(), 2, "with the road's way: right of its middle");
        // the roadway spans 4 lanes and the median; the kerbs stand at its edges
        let half = spec.roadway_width() / 2.0;
        assert!((half - 7.0).abs() < 1e-6);
        assert!(s.profiles.iter().any(|p| p.points.iter().any(|q| (q.x - half).abs() < 1e-3)));
        assert!(content.join("Splines/openMapper/texture/asph.png").is_file());
        assert_eq!(RoadSpec::of(&path).unwrap(), spec);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_road_with_a_green_strip_and_one_footway_reads_back() {
        let dir = std::env::temp_dir().join(format!("om-roadmaker2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |n: &str, c: u8| {
            let p = dir.join(n);
            image::RgbaImage::from_pixel(4, 4, image::Rgba([c, c, c, 255])).save(&p).unwrap();
            p
        };
        let presets = presets();
        let (_, mut spec) = presets.into_iter().find(|(_, s)| s.median_raised).unwrap();
        spec.name = "Avenue".into();
        spec.footway = 0.0;
        spec.footway_right = Some(2.0);
        spec.roadway_tex = mk("a.png", 80);
        spec.footway_tex = mk("f.png", 150);
        spec.median_tex = mk("g.png", 100);
        let content = dir.join("content");
        spec.write(&content).unwrap();
        let s = omsi_scenery::Spline::load(&content.join("Splines/openMapper/Avenue.sli")).unwrap();
        assert_eq!(s.paths.iter().filter(|p| p.kind == 0).count(), 4);
        // one footway only, on the right
        let walks: Vec<_> = s.paths.iter().filter(|p| p.kind == 1).collect();
        assert_eq!(walks.len(), 1);
        assert!(walks[0].start[0] > 0.0);
        assert!(content.join("Splines/openMapper/texture/g.png").is_file());
        assert_eq!(RoadSpec::of(&content.join("Splines/openMapper/Avenue.sli")).unwrap(), spec);
        // an old spec file (before these fields) still reads
        let old: RoadSpec = serde_json::from_str("{\"name\":\"x\",\"lanes_fwd\":1,\"lanes_back\":1}").unwrap();
        assert_eq!(old.footway_right(), old.footway);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_road_with_a_bus_pocket_has_a_lane_for_it_before_the_footway() {
        let dir = std::env::temp_dir().join(format!("om-roadmaker3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |n: &str, c: u8| {
            let p = dir.join(n);
            image::RgbaImage::from_pixel(4, 4, image::Rgba([c, c, c, 255])).save(&p).unwrap();
            p
        };
        let spec = RoadSpec { name: "Bay".into(), pocket: 3.0, footway: 2.0, roadway_tex: mk("a.png", 80), footway_tex: mk("f.png", 150), ..Default::default() };
        assert!((spec.total_width() - (7.0 + 3.0 + 4.0)).abs() < 1e-6);
        let content = dir.join("content");
        spec.write(&content).unwrap();
        let s = omsi_scenery::Spline::load(&content.join("Splines/openMapper/Bay.sli")).unwrap();
        // the two through lanes and the pocket's, with the road's way, right of the roadway's edge (3.5)
        let cars: Vec<_> = s.paths.iter().filter(|p| p.kind == 0).collect();
        assert_eq!(cars.len(), 3);
        assert!(cars.iter().any(|p| (p.start[0] - 5.0).abs() < 1e-3 && p.direction == 0), "the pocket's lane in the middle of 3.5..6.5");
        // the right footway starts past the pocket
        let walk = s.paths.iter().find(|p| p.kind == 1 && p.start[0] > 0.0).unwrap();
        assert!((walk.start[0] - 7.5).abs() < 1e-3, "{}", walk.start[0]);
        let right = s.profiles.iter().flat_map(|p| p.points.iter()).map(|q| q.x).fold(f32::MIN, f32::max);
        assert!((right - 8.5).abs() < 1e-3, "{right}");
        // the spec reads back, and an old file without a pocket has none
        assert_eq!(RoadSpec::of(&content.join("Splines/openMapper/Bay.sli")).unwrap(), spec);
        let old: RoadSpec = serde_json::from_str("{\"name\":\"x\"}").unwrap();
        assert_eq!(old.pocket, 0.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn painted_markings_are_in_the_roadway_texture_and_the_road_reads_back() {
        let dir = std::env::temp_dir().join(format!("om-roadmaker4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tex = dir.join("a.png");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([80, 80, 80, 255])).save(&tex).unwrap();
        let mut spec = RoadSpec { name: "Lined".into(), roadway_tex: tex, marked: true, ..Default::default() };
        spec.markings = spec.standard_markings();
        // two-way, no strip: a double yellow in the middle and an edge line each side
        assert!(spec.markings.iter().any(|m| m.style == MarkStyle::Double && m.x == 0.0));
        let img = spec.painted_roadway().unwrap();
        let at = (spec.roadway_u(0.11) * img.width() as f32) as u32;
        let yellow_at_middle = (0..img.height()).any(|y| img.get_pixel(at, y).0[0] > 150);
        // (the gap between the two lines is asphalt)
        assert_eq!(img.get_pixel(img.width() / 2, 3).0[0], 80);
        assert!(yellow_at_middle);
        // (and plain asphalt where no line is)
        assert_eq!(img.get_pixel(img.width() / 4, 3).0[0], 80);
        let content = dir.join("content");
        spec.write(&content).unwrap();
        assert!(content.join("Splines/openMapper/texture/Lined_lines.bmp").is_file());
        let s = omsi_scenery::Spline::load(&content.join("Splines/openMapper/Lined.sli")).unwrap();
        assert!(s.textures[0].file.eq_ignore_ascii_case("Lined_lines.bmp"), "{}", s.textures[0].file);
        assert_eq!(RoadSpec::of(&content.join("Splines/openMapper/Lined.sli")).unwrap(), spec);
        // a dashed line's dashes come a whole number to the texture
        let d = Marking { style: MarkStyle::Dashed, dash: 3.0, gap: 6.0, ..Default::default() };
        let spec2 = RoadSpec { markings: vec![d.clone()], ..spec.clone() };
        assert!((spec2.mark_period() / spec2.dash_period(&d) - (spec2.mark_period() / spec2.dash_period(&d)).round()).abs() < 1e-4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_drawing_by_hand_wraps_along_the_road_and_is_kept_with_the_road() {
        let dir = std::env::temp_dir().join(format!("om-roadmaker5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tex = dir.join("a.png");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([80, 80, 80, 255])).save(&tex).unwrap();
        let mut spec = RoadSpec { name: "Drawn".into(), roadway_tex: tex, marked: true, ..Default::default() };
        let (cw, len) = (spec.canvas_width(), spec.mark_period());
        let mut h = Hand::default();
        assert!(h.is_empty());
        // a stroke across the end of the texture comes in again at its start
        h.stroke(cw, len, (3.0, len - 0.1), (3.0, len + 1.0), 0.1, YELLOW, false);
        let px = (3.0 / cw * HAND_W as f32) as u32;
        assert!(h.img.get_pixel(px, HAND_H - 3).0[3] > 200);
        assert!(h.img.get_pixel(px, 20).0[3] > 200, "wrapped");
        assert_eq!(h.img.get_pixel(px, HAND_H / 2).0[3], 0);
        // an eraser takes it away
        h.stroke(cw, len, (3.0, 0.0), (3.0, 0.3), 0.3, WHITE, true);
        assert_eq!(h.img.get_pixel(px, 20).0[3], 0);
        // a filled arrow is there
        h.fill(cw, len, &arrow_polygon((5.0, 5.0), false), WHITE, false);
        assert!(h.img.get_pixel((5.0 / cw * HAND_W as f32) as u32, (5.0 / len * HAND_H as f32) as u32).0[3] > 200);
        spec.hand = Some(std::sync::Arc::new(h));
        // it lies over the texture, and is read back with the road
        let img = spec.painted_roadway().unwrap();
        assert!(img.get_pixel(px, 41).0[0] > 150);
        let content = dir.join("content");
        spec.write(&content).unwrap();
        let back = RoadSpec::of(&content.join("Splines/openMapper/Drawn.sli")).unwrap();
        assert_eq!(back.hand, spec.hand);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn styles_keep_only_what_differs() {
        let mut st = Styles::default();
        st.set("Splines\\A.sli", JunctionStyle { lights: true, ..Default::default() });
        st.set("Splines\\B.sli", JunctionStyle::default());
        assert!(st.get("splines\\a.sli").lights);
        assert_eq!(st.styles.len(), 1);
        st.set_piece("Splines\\A.sli", Some("Sceneryobjects\\x.sco".into()));
        assert_eq!(st.piece("splines\\a.sli"), Some("Sceneryobjects\\x.sco"));
        // the file kept by the first versions (the plain map of styles) still loads
        let dir = std::env::temp_dir().join(format!("om-styles-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("openMapper")).unwrap();
        std::fs::write(dir.join("openMapper/road_styles.json"), "{\"splines\\\\a.sli\": {\"corner\": 2.0, \"lights\": true, \"crossings\": false, \"junctions\": true}}").unwrap();
        assert!(Styles::load(&dir).get("splines\\a.sli").lights);
        st.save(&dir).unwrap();
        assert_eq!(Styles::load(&dir).piece("splines\\a.sli"), Some("Sceneryobjects\\x.sco"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
