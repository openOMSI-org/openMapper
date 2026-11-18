//! The PBR window: a texture's normal and occlusion / roughness / metalness maps made from
//! its colours, shown as they are tuned, then written beside it (`pbr_maker`).

use std::path::{Path, PathBuf};

use egui::{RichText, Vec2};

use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::pbr_maker::{self, PbrParams};
use crate::ui::Action;
use crate::theme::pal;

const PREVIEW: u32 = 192;

#[derive(Default)]
pub struct PbrWindow {
    pub open: bool,
    /// The textures to choose from (an object's), and the one being worked on.
    textures: Vec<PathBuf>,
    texture: Option<PathBuf>,
    params: PbrParams,
    /// Made here before (Configure), or maps of its own beside it (from its author).
    made: bool,
    foreign: bool,
    /// The texture at preview size, and the previews (colour, normal, orm).
    small: Option<(u32, u32, Vec<u8>)>,
    shown: Option<[egui::TextureHandle; 4]>,
    shown_for: Option<PbrParams>,
    error: Option<String>,
}

impl PbrWindow {
    /// Open on an object's textures (the first one chosen).
    pub fn open_for(&mut self, textures: Vec<PathBuf>, roots: &Roots) {
        self.open = true;
        self.textures = textures;
        let first = self.textures.first().cloned();
        self.choose(first, roots);
    }

    fn choose(&mut self, t: Option<PathBuf>, roots: &Roots) {
        self.small = None;
        self.shown = None;
        self.shown_for = None;
        self.error = None;
        self.texture = t.clone();
        let Some(t) = t else { return };
        let made = pbr_maker::made(&t, &roots.omsi, &roots.content);
        self.made = made.is_some();
        self.params = made.unwrap_or_default();
        self.foreign = !self.made && !omsi_texture::pbr::find(&t).is_empty();
        match omsi_texture::decode_file(&t) {
            Ok(img) => {
                let src = image::RgbaImage::from_raw(img.width, img.height, img.rgba).unwrap_or_default();
                let k = (PREVIEW as f32 / img.width.max(img.height).max(1) as f32).min(1.0);
                let (w, h) = (((img.width as f32 * k) as u32).max(1), ((img.height as f32 * k) as u32).max(1));
                let s = image::imageops::resize(&src, w, h, image::imageops::FilterType::Triangle);
                self.small = Some((w, h, s.into_raw()));
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn window(&mut self, ctx: &egui::Context, roots: Option<&Roots>, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let Some(roots) = roots else { return };
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Texture, tr("PBR material", "PBR-материал"))).open(&mut open).resizable(false).default_width(640.0).show(ctx, |ui| {
            // which texture
            ui.horizontal(|ui| {
                let name = |p: &Path| p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let current = self.texture.as_deref().map(name).unwrap_or_else(|| tr("none", "нет").to_string());
                let mut pick = None;
                egui::ComboBox::from_id_salt("pbr texture").width(300.0).selected_text(current).show_ui(ui, |ui| {
                    for t in &self.textures {
                        if ui.selectable_label(Some(t) == self.texture.as_ref(), name(t)).clicked() {
                            pick = Some(t.clone());
                        }
                    }
                });
                if ui.button(icons::labelled(Icon::Open, tr("Own picture…", "Своя картинка…"))).clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter(tr("Pictures", "Картинки"), &["png", "jpg", "jpeg", "bmp", "tga", "dds"]).pick_file() {
                        self.textures.push(f.clone());
                        pick = Some(f);
                    }
                }
                if let Some(t) = pick {
                    self.choose(Some(t), roots);
                }
            });
            let Some(tex) = self.texture.clone() else {
                ui.label(tr("Choose a texture of the object, or a picture of your own.", "Выберите текстуру объекта или свою картинку."));
                return;
            };
            if let Some(e) = &self.error {
                ui.colored_label(pal().error, e);
                return;
            }
            // the previews follow the sliders
            if self.shown_for != Some(self.params) {
                if let Some((w, h, px)) = &self.small {
                    let m = pbr_maker::make(*w, *h, px, &self.params);
                    let up = |name: &str, data: &[u8]| ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], data), egui::TextureOptions::LINEAR);
                    // (the occlusion and roughness, as they look: grey)
                    let rough: Vec<u8> = m.orm.chunks(4).flat_map(|c| [c[1], c[1], c[1], 255]).collect();
                    let ao: Vec<u8> = m.orm.chunks(4).flat_map(|c| [c[0], c[0], c[0], 255]).collect();
                    self.shown = Some([up("pbr colour", px), up("pbr normal", &m.normal), up("pbr rough", &rough), up("pbr ao", &ao)]);
                    self.shown_for = Some(self.params);
                }
            }
            if let Some(shown) = &self.shown {
                ui.horizontal(|ui| {
                    for (t, label) in shown.iter().zip([tr("Colour", "Цвет"), tr("Normals (_nn)", "Нормали (_nn)"), tr("Roughness", "Шероховатость"), tr("Shadow in hollows", "Тень во впадинах")]) {
                        ui.vertical(|ui| {
                            ui.add(egui::Image::new((t.id(), Vec2::splat(150.0))));
                            ui.label(RichText::new(label).small());
                        });
                    }
                });
            }
            ui.add_space(4.0);
            let p = &mut self.params;
            egui::Grid::new("pbr params").num_columns(2).spacing(Vec2::new(10.0, 4.0)).show(ui, |ui| {
                ui.label(tr("Bumps", "Рельеф"));
                ui.add(egui::Slider::new(&mut p.bumps, 0.0..=10.0));
                ui.end_row();
                ui.label(tr("Only big shapes", "Только крупные формы"));
                ui.add(egui::Slider::new(&mut p.blur, 0.0..=8.0).step_by(1.0).suffix(" px"));
                ui.end_row();
                ui.label("");
                crate::ui::check(ui, &mut p.invert, tr("Dark parts stand out (not sink in)", "Тёмное выступает (а не утоплено)"));
                ui.end_row();
                ui.label(tr("Roughness", "Шероховатость"));
                ui.add(egui::Slider::new(&mut p.rough, 0.0..=1.0));
                ui.end_row();
                ui.label(tr("Bright parts smoother", "Светлое глаже"));
                ui.add(egui::Slider::new(&mut p.rough_detail, 0.0..=1.0));
                ui.end_row();
                ui.label(tr("Metal", "Металл"));
                ui.add(egui::Slider::new(&mut p.metal, 0.0..=1.0));
                ui.end_row();
                ui.label(tr("Shadow in hollows", "Тень во впадинах"));
                ui.add(egui::Slider::new(&mut p.occlusion, 0.0..=1.0));
                ui.end_row();
            });
            if ui.small_button(icons::labelled(Icon::Restart, tr("Back to the defaults", "Вернуть по умолчанию"))).clicked() {
                self.params = PbrParams::default();
            }
            if self.foreign {
                ui.label(RichText::new(tr("This texture has PBR maps of its own already: new ones made here are taken instead of them.", "У этой текстуры уже есть свои PBR-карты: сделанные здесь будут использоваться вместо них.")).small().color(pal().warn));
            }
            let [nn, orm, _] = pbr_maker::files(&tex, &roots.omsi, &roots.content);
            ui.label(RichText::new(format!("{}\n{}\n{}", tr("Written as:", "Записывается как:"), nn.display(), orm.display())).small().color(pal().text_dim));
            ui.horizontal(|ui| {
                let label = if self.made { tr("Apply", "Применить") } else { tr("Create PBR", "Создать PBR") };
                if ui.button(icons::labelled(Icon::CheckCircle, label)).clicked() {
                    match pbr_maker::create(&tex, &roots.omsi, &roots.content, &self.params) {
                        Ok(()) => {
                            self.made = true;
                            self.foreign = false;
                            actions.push(Action::ReloadContent(tr("PBR maps made", "PBR-карты созданы").to_string()));
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                if self.made && ui.button(icons::labelled(Icon::Delete, tr("Delete the PBR material", "Удалить PBR-материал"))).clicked() {
                    match pbr_maker::remove(&tex, &roots.omsi, &roots.content) {
                        Ok(()) => {
                            self.made = false;
                            self.foreign = !omsi_texture::pbr::find(&tex).is_empty();
                            self.params = PbrParams::default();
                            actions.push(Action::ReloadContent(tr("PBR maps taken away", "PBR-карты удалены").to_string()));
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            });
        });
        self.open = open;
    }
}

/// Is a texture's PBR made here (the button then reads Configure)?
pub fn is_made(texture: &Path, roots: &Roots) -> bool {
    pbr_maker::made(texture, &roots.omsi, &roots.content).is_some()
}
