//! Painting a ground layer by a rule over the whole map (Paint tool): where the ground is
//! as steep as asked (rock on slopes, grass on the flat) and as high (sand by the water,
//! snow on the tops), fading in over a margin so it has no hard edge. The masks it writes
//! are the same the brush paints, so the brush touches it up afterwards.

use glam::DVec2;

use crate::doc::{mask_size, tile_size, Document};
use crate::history::History;
use crate::i18n::tr;
use crate::theme::pal;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    /// Slope in degrees.
    pub slope: [f32; 2],
    /// Height in metres.
    pub height: [f32; 2],
    pub height_on: bool,
    /// How far past the limits it fades out (degrees, and metres for height).
    pub soft: f32,
    /// How strongly it covers (0..1).
    pub amount: f32,
}

impl Default for Rule {
    fn default() -> Self {
        Rule { slope: [25.0, 90.0], height: [0.0, 100.0], height_on: false, soft: 4.0, amount: 1.0 }
    }
}

/// 1 inside `lo..hi`, falling to 0 over `soft` outside it.
pub(crate) fn band(v: f32, [lo, hi]: [f32; 2], soft: f32) -> f32 {
    let out = (lo - v).max(v - hi).max(0.0);
    if soft <= 0.0 {
        return if out > 0.0 { 0.0 } else { 1.0 };
    }
    let t = (1.0 - out / soft).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Rule {
    /// How much of the layer goes where the ground is so high and so steep (degrees).
    pub fn weight(&self, height: f32, slope: f32) -> f32 {
        let mut w = band(slope, self.slope, self.soft);
        if self.height_on {
            w *= band(height, self.height, self.soft);
        }
        w * self.amount
    }
}

/// The slope (degrees) of a tile's ground at tile-local (x, y).
fn slope_at(t: &crate::doc::TileDoc, x: f64, y: f64) -> f32 {
    let d = 2.5;
    let ts = tile_size();
    let (x0, x1) = ((x - d).max(0.0), (x + d).min(ts));
    let (y0, y1) = ((y - d).max(0.0), (y + d).min(ts));
    let gx = (t.height_at(x1, y) - t.height_at(x0, y)) as f64 / (x1 - x0);
    let gy = (t.height_at(x, y1) - t.height_at(x, y0)) as f64 / (y1 - y0);
    gx.hypot(gy).atan().to_degrees() as f32
}

/// `layer` (above the base ground) painted by `rule` on every tile (what was painted kept where the rule gives
/// less; the layers above it give way as under the brush), one undo step. How many texels
/// it covered.
pub fn apply(doc: &mut Document, hist: &mut History, layer: usize, rule: &Rule) -> usize {
    let keys: Vec<_> = doc.tiles.keys().copied().collect();
    paint_where(doc, hist, layer, tr("Paint by slope and height", "Покраска по уклону и высоте"), &keys, |_, t, p| rule.weight(t.height_at(p.x, p.y), slope_at(t, p.x, p.y)))
}

/// `layer` painted on the tiles `keys` as much as `weight` (0..1) says at each texel (given
/// the tile and the tile-local point): what was painted kept where it says less, the layers
/// above giving way as under the brush. One undo step; how many texels it covered.
pub fn paint_where(doc: &mut Document, hist: &mut History, layer: usize, label: &'static str, keys: &[crate::doc::TileKey], weight: impl Fn(crate::doc::TileKey, &crate::doc::TileDoc, DVec2) -> f32) -> usize {
    let layers = doc.global.ground_textures.len();
    if layer == 0 || layer >= layers {
        return 0;
    }
    doc.sync_mask_slots();
    let sizes: Vec<usize> = doc.global.ground_textures.iter().map(mask_size).collect();
    let ts = tile_size();
    hist.begin(label);
    let mut covered = 0;
    for &k in keys {
        if !doc.tiles.contains_key(&k) {
            continue;
        }
        hist.touch_tile(doc, k);
        let t = doc.tiles.get_mut(&k).unwrap();
        // the weight on the layer's own mask, read for the layers above at their sizes
        let mut changed = false;
        let size = sizes[layer];
        let texel = ts / size as f64;
        let mut w = vec![0f32; size * size];
        for yi in 0..size {
            for xi in 0..size {
                let p = DVec2::new((xi as f64 + 0.5) * texel, (yi as f64 + 0.5) * texel);
                let v = weight(k, t, p);
                // (row 0 is the north edge)
                w[(size - 1 - yi) * size + xi] = v;
                if v > 0.004 {
                    covered += 1;
                }
            }
        }
        let at = |w: &[f32], n: usize, row: usize, col: usize| -> f32 {
            // the weight under texel (row, col) of an n × n mask
            let r = (row * size / n).min(size - 1);
            let c = (col * size / n).min(size - 1);
            w[r * size + c]
        };
        for li in layer..layers {
            let n = sizes[li];
            if li > layer && t.masks[li].is_none() {
                continue;
            }
            let mask = t.masks[li].get_or_insert_with(|| vec![0u8; n * n]);
            if mask.len() != n * n {
                *mask = vec![0u8; n * n];
            }
            for row in 0..n {
                for col in 0..n {
                    let k = at(&w, n, row, col);
                    if k <= 0.0 {
                        continue;
                    }
                    let a = &mut mask[row * n + col];
                    let v = *a as f32;
                    let nv = if li == layer { v.max(255.0 * k) } else { v * (1.0 - k) };
                    let nv = nv.round().clamp(0.0, 255.0) as u8;
                    if nv != *a {
                        *a = nv;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            t.touch_masks();
        }
    }
    hist.commit();
    covered
}

/// The Paint tool's rule, under its brush.
pub fn panel(ui: &mut egui::Ui, rule: &mut Rule, ed: &mut crate::editor::Editor) {
    use crate::icons::{self, Icon};
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tr("By slope and height", "По уклону и высоте")).strong());
    ui.label(egui::RichText::new(tr("The chosen layer painted over the whole map where the ground is so steep (and so high)", "Выбранный слой по всей карте там, где земля так крута (и так высока)")).small().color(pal().text_dim));
    egui::Grid::new("autopaint").num_columns(2).show(ui, |ui| {
        ui.label(tr("Slope", "Уклон"));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut rule.slope[0]).range(0.0..=90.0).speed(0.5).suffix("°"));
            ui.label("–");
            ui.add(egui::DragValue::new(&mut rule.slope[1]).range(0.0..=90.0).speed(0.5).suffix("°"));
        });
        ui.end_row();
        crate::ui::check(ui, &mut rule.height_on, tr("Height", "Высота"));
        ui.add_enabled_ui(rule.height_on, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut rule.height[0]).speed(0.5).suffix(" m"));
                ui.label("–");
                ui.add(egui::DragValue::new(&mut rule.height[1]).speed(0.5).suffix(" m"));
            });
        });
        ui.end_row();
        ui.label(tr("Fade", "Переход"));
        ui.add(egui::Slider::new(&mut rule.soft, 0.0..=20.0).fixed_decimals(1));
        ui.end_row();
        ui.label(tr("Cover", "Покрытие"));
        ui.add(egui::Slider::new(&mut rule.amount, 0.05..=1.0).fixed_decimals(2));
        ui.end_row();
    });
    if rule.slope[0] > rule.slope[1] {
        rule.slope.swap(0, 1);
    }
    if rule.height[0] > rule.height[1] {
        rule.height.swap(0, 1);
    }
    let layer = ed.paint_layer;
    let can = ed.doc.as_ref().map(|d| layer > 0 && layer < d.global.ground_textures.len()).unwrap_or(false);
    let r = ui.add_enabled(can, egui::Button::new(icons::labelled(Icon::Brush, tr("Paint the map", "Покрасить карту"))));
    if !can {
        r.on_disabled_hover_text(tr("Choose a layer above the base ground", "Выберите слой над основой"));
    } else if r.clicked() {
        if let Some(doc) = ed.doc.as_mut() {
            let n = apply(doc, &mut ed.history, layer, rule);
            let m2 = n as f64 * {
                let s = tile_size() / mask_size(&doc.global.ground_textures[layer]) as f64;
                s * s
            };
            if n == 0 {
                ed.toast(tr("No ground on the map is like that", "Такой земли на карте нет"));
            } else {
                ed.toast(format!("{} {:.2} km²", tr("Painted", "Покрашено"), m2 / 1e6));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slope_is_painted_and_the_flat_is_not() {
        let _grid = crate::doc::one_map();
        let mut doc = crate::selection::tests::flat_map(2);
        doc.global.ground_textures = vec![
            omsi_map::GroundTex { texture: "Texture\\gras.bmp".into(), detail_texture: String::new(), params: [0.0, 1.0, 60.0] },
            omsi_map::GroundTex { texture: "Texture\\rock.bmp".into(), detail_texture: String::new(), params: [8.0, 16.0, 4.0] },
        ];
        // tile 1, 0 rises 1 m in 1 m (45°) east of x = 450; the rest is flat
        let k = (1, 0);
        let t = doc.tiles.get_mut(&k).unwrap();
        let s = t.terrain.samples();
        for j in 0..s {
            for i in 0..s {
                let x = 300.0 + i as f64 * 300.0 / (s - 1) as f64;
                t.terrain.heights[j * s + i] = (x - 450.0).max(0.0) as f32;
            }
        }
        let mut hist = History::default();
        let rule = Rule { slope: [30.0, 90.0], soft: 2.0, ..Default::default() };
        assert!(apply(&mut doc, &mut hist, 1, &rule) > 0);
        let n = mask_size(&doc.global.ground_textures[1]);
        let m = doc.tiles[&k].masks[1].as_ref().unwrap();
        let col = |x: f64| ((x / 300.0) * n as f64) as usize;
        let mid = n / 2;
        assert_eq!(m[mid * n + col(250.0)], 255, "the slope is painted");
        assert_eq!(m[mid * n + col(50.0)], 0, "the flat is not");
        assert!(doc.tiles[&(0, 0)].masks.get(1).map(|m| m.as_ref().map(|m| m.iter().all(|v| *v == 0)).unwrap_or(true)).unwrap_or(true));
        assert_eq!(band(10.0, [20.0, 30.0], 0.0), 0.0);
        assert!(band(18.0, [20.0, 30.0], 4.0) > 0.0);
    }
}
