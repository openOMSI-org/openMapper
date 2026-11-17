//! The map's ground as a grey picture and back (Terrain tool): a 16-bit PNG over all the
//! map's tiles, one pixel per terrain sample, its lowest and highest height kept beside it
//! in a `.txt`, so it can be shaped in another program (World Machine, Gaea, an image
//! editor) and put back; any picture (8-bit too) is laid over the map stretched to its tiles.

use std::path::{Path, PathBuf};

use glam::DVec2;
use omsi_map::TERRAIN_SAMPLES;

use crate::doc::{tile_of, tile_origin, tile_size, Document, TileKey};
use crate::history::History;
use crate::i18n::tr;
use crate::theme::pal;

/// The tiles' rectangle: the lowest and the highest tile key.
pub fn extent(doc: &Document) -> Option<(TileKey, TileKey)> {
    let mut keys = doc.tiles.keys();
    let first = *keys.next()?;
    Some(keys.fold((first, first), |(lo, hi), k| ((lo.0.min(k.0), lo.1.min(k.1)), (hi.0.max(k.0), hi.1.max(k.1)))))
}

/// Pixels per tile edge (one per terrain cell).
const PER_TILE: usize = TERRAIN_SAMPLES - 1;

/// The ground's height at a world position, from whichever tile holds it (a point on a
/// tile's edge is in both); None where the map has no tile.
fn height(doc: &Document, p: DVec2) -> Option<f32> {
    let e = 1e-6;
    for q in [p, p - DVec2::new(e, 0.0), p - DVec2::new(0.0, e), p - DVec2::splat(e)] {
        let k = tile_of(q);
        if let Some(t) = doc.tiles.get(&k) {
            let l = (p - tile_origin(k)).clamp(DVec2::ZERO, DVec2::splat(tile_size()));
            return Some(t.height_at(l.x, l.y));
        }
    }
    None
}

/// What the `.txt` beside a picture says (or what an export wrote).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub lowest: f32,
    pub highest: f32,
}

pub fn sidecar(png: &Path) -> PathBuf {
    png.with_extension("txt")
}

/// The heights a sidecar names, if it is there and readable.
pub fn read_range(png: &Path) -> Option<Range> {
    let text = std::fs::read_to_string(sidecar(png)).ok()?;
    let value = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)?.trim().parse::<f32>().ok());
    Some(Range { lowest: value("lowest")?, highest: value("highest")? })
}

/// The map's ground written as a 16-bit grey PNG (north up) with its `.txt`.
pub fn export(doc: &Document, path: &Path) -> anyhow::Result<(u32, u32, Range)> {
    let Some((lo, hi)) = extent(doc) else { anyhow::bail!("the map has no tiles") };
    let w = (hi.0 - lo.0 + 1) as usize * PER_TILE + 1;
    let h = (hi.1 - lo.1 + 1) as usize * PER_TILE + 1;
    let cell = tile_size() / PER_TILE as f64;
    let west = tile_origin(lo).x;
    let north = tile_origin((lo.0, hi.1 + 1)).y;
    let mut vals: Vec<Option<f32>> = Vec::with_capacity(w * h);
    for py in 0..h {
        for px in 0..w {
            vals.push(height(doc, DVec2::new(west + px as f64 * cell, north - py as f64 * cell)));
        }
    }
    let (min, max) = vals.iter().flatten().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let range = Range { lowest: min, highest: if max - min < 0.01 { min + 1.0 } else { max } };
    let span = range.highest - range.lowest;
    let px: Vec<u16> = vals.iter().map(|v| (((v.unwrap_or(min) - range.lowest) / span).clamp(0.0, 1.0) * 65535.0).round() as u16).collect();
    let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(w as u32, h as u32, px).expect("size");
    img.save_with_format(path, image::ImageFormat::Png)?;
    let note = format!(
        "openMapper height map of {}\nlowest {}\nhighest {}\nmetres per pixel {}\ntiles {} {} to {} {}\n",
        doc.global.friendly_name, range.lowest, range.highest, cell, lo.0, lo.1, hi.0, hi.1
    );
    crate::doc::safe::write_atomic(&sidecar(path), note.as_bytes())?;
    Ok((w as u32, h as u32, range))
}

/// A picture read as heights from 0 to 1 (row 0 north).
pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
    /// Fewer than 16 bits a channel: it steps, and smoothing helps.
    pub coarse: bool,
}

impl Picture {
    pub fn load(path: &Path) -> anyhow::Result<Picture> {
        let img = image::open(path)?;
        let coarse = !matches!(img.color(), image::ColorType::L16 | image::ColorType::La16 | image::ColorType::Rgb16 | image::ColorType::Rgba16 | image::ColorType::Rgb32F | image::ColorType::Rgba32F);
        let g = img.to_luma16();
        let (w, h) = (g.width() as usize, g.height() as usize);
        if w < 2 || h < 2 {
            anyhow::bail!("{}", tr("the picture is too small", "картинка слишком мала"));
        }
        Ok(Picture { w, h, v: g.into_raw().into_iter().map(|x| x as f32 / 65535.0).collect(), coarse })
    }

    /// `passes` of a 3 × 3 average (the steps of an 8-bit picture gone).
    pub fn smooth(&mut self, passes: usize) {
        let (w, h) = (self.w, self.h);
        for _ in 0..passes {
            let src = self.v.clone();
            for y in 0..h {
                for x in 0..w {
                    let (mut s, mut n) = (0.0, 0.0);
                    for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                        for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                            s += src[yy * w + xx];
                            n += 1.0;
                        }
                    }
                    self.v[y * w + x] = s / n;
                }
            }
        }
    }

    /// Bilinear at (u, v) in 0..1 across, 0..1 down.
    fn at(&self, u: f64, v: f64) -> f32 {
        let fx = (u.clamp(0.0, 1.0) * (self.w - 1) as f64) as f32;
        let fy = (v.clamp(0.0, 1.0) * (self.h - 1) as f64) as f32;
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let p = |x: usize, y: usize| self.v[y * self.w + x];
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bottom = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        top * (1.0 - ay) + bottom * ay
    }
}

/// How a picture goes onto the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Apply {
    pub range: Range,
    /// Added to the ground as it is (hills on a slope) instead of taking its place.
    pub add: bool,
}

/// The picture laid over all the map's tiles (its edges on the map's outer edges), one undo
/// step. Gives back how many tiles changed.
pub fn import(doc: &mut Document, hist: &mut History, pic: &Picture, how: Apply) -> usize {
    let Some((lo, hi)) = extent(doc) else { return 0 };
    let west = tile_origin(lo).x;
    let south = tile_origin(lo).y;
    let width = (hi.0 - lo.0 + 1) as f64 * tile_size();
    let height = (hi.1 - lo.1 + 1) as f64 * tile_size();
    let span = how.range.highest - how.range.lowest;
    hist.begin(tr("Import height map", "Импорт карты высот"));
    let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
    for k in &keys {
        hist.touch_tile(doc, *k);
        let t = doc.tiles.get_mut(k).unwrap();
        let s = t.terrain.samples();
        let cells = (s - 1).max(1) as f64;
        let org = tile_origin(*k);
        for j in 0..s {
            for i in 0..s {
                let p = org + DVec2::new(i as f64, j as f64) * (tile_size() / cells);
                let v = pic.at((p.x - west) / width, 1.0 - (p.y - south) / height);
                let z = how.range.lowest + v * span;
                let h = &mut t.terrain.heights[j * s + i];
                *h = if how.add { *h + z } else { z };
            }
        }
        t.touch_terrain();
    }
    hist.commit();
    keys.len()
}

/// The Terrain tool's height map buttons and the import's settings.
#[derive(Default)]
pub struct HeightMapUi {
    /// A picture chosen, waiting for its heights.
    pending: Option<(PathBuf, Picture, Apply, usize)>,
}

impl HeightMapUi {
    pub fn panel(&mut self, ui: &mut egui::Ui, ed: &mut crate::editor::Editor) {
        use crate::icons::{self, Icon};
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tr("Height map", "Карта высот")).strong());
        ui.label(egui::RichText::new(tr("The ground as a grey picture, to shape in another program and bring back", "Рельеф как серая картинка: обработать в другой программе и вернуть")).small().color(pal().text_dim));
        ui.horizontal(|ui| {
            if ui.add_enabled(ed.doc.is_some(), egui::Button::new(icons::labelled(Icon::Save, tr("Export…", "Экспорт…")))).clicked() {
                self.export(ed);
            }
            if ui.add_enabled(ed.doc.is_some(), egui::Button::new(icons::labelled(Icon::Import, tr("Import…", "Импорт…")))).clicked() {
                self.choose(ed);
            }
        });
        let Some((path, pic, how, smooth)) = &mut self.pending else { return };
        let mut go = false;
        let mut cancel = false;
        egui::Frame::new().fill(pal().field).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).strong());
            let (w, h) = ed.doc.as_ref().and_then(extent).map(|(lo, hi)| ((hi.0 - lo.0 + 1) as f64 * tile_size(), (hi.1 - lo.1 + 1) as f64 * tile_size())).unwrap_or((0.0, 0.0));
            ui.label(egui::RichText::new(format!("{} × {} px {} {:.0} × {:.0} m ({:.1} m {})", pic.w, pic.h, tr("over", "на"), w, h, w / (pic.w - 1) as f64, tr("a pixel", "на пиксель"))).small().color(pal().text_dim));
            egui::Grid::new("heightmap import").num_columns(2).show(ui, |ui| {
                ui.label(if how.add { tr("Black adds", "Чёрный добавляет") } else { tr("Black is", "Чёрный это") });
                ui.add(egui::DragValue::new(&mut how.range.lowest).speed(0.5).suffix(" m"));
                ui.end_row();
                ui.label(if how.add { tr("White adds", "Белый добавляет") } else { tr("White is", "Белый это") });
                ui.add(egui::DragValue::new(&mut how.range.highest).speed(0.5).suffix(" m"));
                ui.end_row();
                ui.label(tr("Smoothing", "Сглаживание"));
                ui.add(egui::Slider::new(smooth, 0..=6));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut how.add, false, tr("Replace the ground", "Заменить рельеф"));
                ui.selectable_value(&mut how.add, true, tr("Add to it", "Добавить к нему"));
            });
            ui.label(egui::RichText::new(tr("Roads and objects keep their heights: best done before building", "Дороги и объекты остаются на своей высоте: лучше делать до строительства")).small().color(pal().text_dim));
            if pic.coarse {
                ui.label(egui::RichText::new(tr("An 8-bit picture has only 256 steps of height: smoothing hides them", "В 8-битной картинке лишь 256 ступеней высоты: сглаживание их скрывает")).small().color(pal().text_dim));
            }
            ui.horizontal(|ui| {
                go = ui.button(icons::labelled(Icon::Terrain, tr("Lay it on the map", "Наложить на карту"))).clicked();
                cancel = ui.button(tr("Cancel", "Отмена")).clicked();
            });
        });
        if cancel {
            self.pending = None;
        } else if go {
            self.apply(ed);
        }
    }

    /// The picture chosen laid on the map.
    pub fn apply(&mut self, ed: &mut crate::editor::Editor) {
        let Some((_, mut pic, how, smooth)) = self.pending.take() else { return };
        pic.smooth(smooth);
        if let Some(doc) = ed.doc.as_mut() {
            let n = import(doc, &mut ed.history, &pic, how);
            ed.toast(format!("{} {n} {}", tr("Height map laid over", "Карта высот наложена на"), tr("tiles", "тайлов")));
        }
    }

    fn export(&mut self, ed: &mut crate::editor::Editor) {
        let Some(doc) = ed.doc.as_ref() else { return };
        let name = format!("{}_heights.png", doc.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "map".into()));
        let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(name).set_title(tr("Export the height map", "Экспорт карты высот")).save_file() else { return };
        match export(doc, &path) {
            Ok((w, h, r)) => ed.toast(format!("{} ({w} × {h} px, {:.1}…{:.1} m)", tr("Height map written", "Карта высот записана"), r.lowest, r.highest)),
            Err(e) => ed.error(format!("{}: {e}", tr("The height map could not be written", "Не удалось записать карту высот"))),
        }
    }

    fn choose(&mut self, ed: &mut crate::editor::Editor) {
        let Some(path) = rfd::FileDialog::new().add_filter(tr("Pictures", "Картинки"), &["png", "tif", "tiff", "bmp", "jpg", "jpeg"]).set_title(tr("A height map for the whole map", "Карта высот для всей карты")).pick_file() else { return };
        self.open(ed, path);
    }

    /// A picture to import, its heights from its `.txt` when it has one.
    pub fn open(&mut self, ed: &mut crate::editor::Editor, path: PathBuf) {
        match Picture::load(&path) {
            Ok(pic) => {
                let range = read_range(&path).unwrap_or(Range { lowest: 0.0, highest: 100.0 });
                let smooth = if pic.coarse { 2 } else { 0 };
                self.pending = Some((path, pic, Apply { range, add: false }, smooth));
            }
            Err(e) => ed.error(format!("{}: {e}", tr("The picture could not be read", "Не удалось прочитать картинку"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_and_import_give_the_ground_back() {
        let _grid = crate::doc::one_map();
        let mut doc = crate::selection::tests::flat_map(2);
        // a slope and a bump across the tiles
        for (k, t) in doc.tiles.iter_mut() {
            let s = t.terrain.samples();
            let org = tile_origin(*k);
            for j in 0..s {
                for i in 0..s {
                    let p = org + DVec2::new(i as f64, j as f64) * (tile_size() / (s - 1) as f64);
                    t.terrain.heights[j * s + i] = (20.0 + p.x * 0.05 + (p.y * 0.02).sin() * 4.0) as f32;
                }
            }
        }
        let before: Vec<Vec<f32>> = doc.tiles.values().map(|t| t.terrain.heights.clone()).collect();
        let dir = std::env::temp_dir().join(format!("openmapper-heightmap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("h.png");
        let (w, h, range) = export(&doc, &png).unwrap();
        let (lo, hi) = extent(&doc).unwrap();
        assert_eq!(w as usize, (hi.0 - lo.0 + 1) as usize * PER_TILE + 1);
        assert_eq!(h as usize, (hi.1 - lo.1 + 1) as usize * PER_TILE + 1);
        assert_eq!(read_range(&png), Some(range));
        for t in doc.tiles.values_mut() {
            t.terrain.heights.iter_mut().for_each(|h| *h = 0.0);
        }
        let pic = Picture::load(&png).unwrap();
        assert!(!pic.coarse);
        let mut hist = History::default();
        import(&mut doc, &mut hist, &pic, Apply { range, add: false });
        for (a, b) in before.iter().zip(doc.tiles.values()) {
            for (x, y) in a.iter().zip(&b.terrain.heights) {
                assert!((x - y).abs() < 0.01, "{x} came back as {y}");
            }
        }
        // added on top: twice as high
        import(&mut doc, &mut hist, &pic, Apply { range, add: true });
        let (x, y) = (before[0][100], doc.tiles.values().next().unwrap().terrain.heights[100]);
        assert!((x * 2.0 - y).abs() < 0.02);
        let _ = std::fs::remove_dir_all(dir);
    }
}
