//! `shot`: the map as the editor draws it (its meshes, textures, ground, shadows), drawn with
//! no window into a picture: for an agent to see what an edit really looks like, with the
//! screen locked or no screen at all. The view is a plan from above or a perspective, and
//! labels can be drawn over it (spline IDs, free ends, the map check's issues).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use glam::{DVec2, DVec3};
use serde_json::json;

use super::{number, Args, Out, Reply, Server};
use crate::camera::Camera;
use crate::gpu::Gpu;
use crate::render::Renderer;

/// The device and renderer of the pictures, made on the first `shot` and kept.
pub(crate) struct Shooter {
    gpu: Gpu,
    renderer: Renderer,
    /// The map the renderer holds (another one opened: made again).
    cfg: Option<PathBuf>,
    /// The content's generation its models were read in (files written since: read again).
    generation: u64,
}

impl Server {
    pub fn shot(&mut self, a: Args) -> Reply {
        self.wait_lib();
        let cfg = self.cfg.clone();
        let (w, h) = (a.u32_or("width", 1280).clamp(64, 4096), a.u32_or("height", 800).clamp(64, 4096));
        let d = self.doc()?;
        // the view
        let mut cam = Camera::default();
        let centre = match a.get("center").or(a.get("at")) {
            Some(c) => self.point(c)?,
            None => return Err("center [x, y] is needed".into()),
        };
        let ground = d.ground(centre);
        let tz = d.traffic.timezone.clone();
        let plan = a.bool("plan").unwrap_or(a.str("mode") == Some("plan"));
        let radius = a.f64("radius").unwrap_or(120.0).clamp(5.0, 3000.0);
        if plan {
            // (a narrow view from high up: near to a true plan, north up)
            cam.fov_y = 20f32.to_radians();
            cam.yaw = 0.0;
            cam.pitch = 89.5;
            let half = radius * (h as f64 / w.min(h) as f64);
            cam.jump_to(DVec3::new(centre.x, centre.y, ground), half / (cam.fov_y as f64 / 2.0).tan());
        } else {
            cam.fov_y = a.f64("fov").unwrap_or(50.0).clamp(10.0, 100.0).to_radians() as f32;
            cam.yaw = a.f64("yaw").unwrap_or(0.0);
            cam.pitch = a.f64("pitch").unwrap_or(35.0).clamp(-30.0, 89.5);
            cam.jump_to(DVec3::new(centre.x, centre.y, ground + a.f64("lift").unwrap_or(0.0)), a.f64("distance").unwrap_or(radius));
        }
        cam.snap();
        let technical = a.bool("technical").unwrap_or(false);

        // the device and renderer (kept between calls)
        if self.shooter.as_ref().map(|s| s.cfg != cfg).unwrap_or(true) {
            let quality = crate::render::quality::Quality::High;
            let gpu = Gpu::headless(w, h, quality.samples()).map_err(|e| format!("no graphics device: {e}"))?;
            let mut renderer = Renderer::new(&gpu, quality);
            renderer.roots = vec![self.roots.content.clone(), self.roots.omsi.clone()];
            self.shooter = Some(Box::new(Shooter { gpu, renderer, cfg: cfg.clone(), generation: omsi_cfg::content_generation() }));
        }
        // (taken out while drawing, so the map and the models can be lent beside it)
        let mut held = self.shooter.take().unwrap();
        let sh = &mut *held;
        if sh.generation != omsi_cfg::content_generation() {
            sh.generation = omsi_cfg::content_generation();
            sh.renderer.reset_models();
        }
        if (sh.gpu.config.width, sh.gpu.config.height) != (w, h) {
            sh.gpu.resize(w, h);
        }
        if let Some(hour) = a.f64("hour") {
            sh.renderer.daytime = Some(crate::render::daylight::at_map(hour as f32, &tz));
        }

        // drawn again and again until everything is up (or the time is out)
        let d = self.ed.doc.as_ref().ok_or("no map is open")?;
        let wait = Duration::from_secs_f64(a.f64("wait").unwrap_or(60.0).clamp(1.0, 600.0));
        let start = Instant::now();
        let mut settled_frames = 0;
        let mut frames = 0;
        let target = sh.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: sh.gpu.scene_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut extras = self.ed.extras(&cam, &mut self.assets);
        extras.selected = None;
        extras.selected_more.clear();
        extras.hover = None;
        extras.grid = false;
        extras.brush = None;
        extras.ghosts.clear();
        extras.ghost_splines.clear();
        extras.overlay.clear();
        extras.overlay_depth.clear();
        extras.highlight_tile = None;
        extras.show_technical = technical;
        loop {
            self.assets.poll();
            sh.renderer.sync(&sh.gpu, d, &mut self.assets, &extras, &cam);
            let mut enc = sh.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shot") });
            sh.renderer.draw(&sh.gpu, &mut enc, &view, &cam, &extras, [0.0, 0.0, w as f32, h as f32]);
            sh.gpu.queue.submit([enc.finish()]);
            let _ = sh.gpu.device.poll(wgpu::PollType::wait_indefinitely());
            frames += 1;
            if sh.renderer.settled(&self.assets) {
                settled_frames += 1;
                // (a few more frames: what was put up in the last one is drawn too)
                if settled_frames >= 3 {
                    break;
                }
            } else {
                settled_frames = 0;
                std::thread::sleep(Duration::from_millis(5));
            }
            if start.elapsed() > wait {
                break;
            }
        }
        let complete = settled_frames >= 3;
        let progress = sh.renderer.progress();
        let pending = self.assets.pending;
        let loading: Vec<String> = self.assets.textures_loading().into_iter().take(5).collect();
        let img = crate::app::read_frame(&sh.gpu, &target).map_err(|e| e.to_string());
        self.shooter = Some(held);
        let mut img = img?;

        // labels over the picture
        let layers = a.strings("overlay");
        let mut drawn = 0usize;
        if !layers.is_empty() {
            drawn = super::overlay::draw(self, &a, &cam, &mut img, &layers);
        }
        // the dated imagery under the same view: side by side, blended, or alone
        let compare = a.str("compare").map(str::to_string).or(a.get("imagery").map(|_| "side".to_string()));
        if let Some(mode) = compare {
            let mut sat = self.imagery_view(&a, &cam, w, h, ground)?;
            if !layers.is_empty() && mode == "side" {
                drawn += super::overlay::draw(self, &a, &cam, &mut sat, &layers);
            }
            img = match mode.as_str() {
                "blend" => {
                    let k = a.f64("opacity").unwrap_or(0.5).clamp(0.0, 1.0) as f32;
                    let mut o = img.clone();
                    for (p, q) in o.pixels_mut().zip(sat.pixels()) {
                        for c in 0..3 {
                            p.0[c] = (p.0[c] as f32 * (1.0 - k) + q.0[c] as f32 * k).round() as u8;
                        }
                    }
                    o
                }
                "imagery" => sat,
                _ => {
                    let mut o = image::RgbaImage::from_pixel(w * 2 + 6, h, image::Rgba([255, 255, 255, 255]));
                    image::imageops::replace(&mut o, &img, 0, 0);
                    image::imageops::replace(&mut o, &sat, (w + 6) as i64, 0);
                    o
                }
            };
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).to_rgb8().write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        if let Some(file) = a.str("file") {
            std::fs::write(file, &png).map_err(|e| format!("{file}: {e}"))?;
        }
        let eye = cam.eye();
        let meta = json!({
            "pixels": [w, h], "imagery": self.imagery_src.as_ref().map(|s| s.0.clone()), "frames": frames, "complete": complete, "seconds": number(start.elapsed().as_secs_f64()),
            "eye": [number(eye.x), number(eye.y), number(eye.z)], "looking_at": [number(centre.x), number(centre.y), number(ground)],
            "plan": plan, "labels_drawn": drawn, "progress": if complete { serde_json::Value::Null } else { json!({"renderer": progress, "assets_pending": pending, "pictures_loading": loading}) },
            "note": if complete { "everything was loaded" } else { "the wait ran out before every model and texture was up: call again" },
        });
        Ok(Out::Image { png, meta })
    }
}

impl Server {
    /// The imagery (a tile URL with {z}/{x}/{y}) seen from the shot's camera, laid flat at the
    /// height of the point looked at. The source is remembered for the next calls.
    fn imagery_view(&mut self, a: &Args, cam: &Camera, w: u32, h: u32, z0: f64) -> Result<image::RgbaImage, String> {
        if let Some(im) = a.get("imagery").filter(|v| v.is_object()) {
            let url = im.get("url").and_then(|u| u.as_str()).ok_or("imagery.url is needed")?.to_string();
            let z = im.get("z").and_then(|z| z.as_u64()).unwrap_or(19).clamp(1, 22) as u8;
            self.imagery_src = Some((url, z));
        }
        let (url, z) = self.imagery_src.clone().ok_or("no imagery yet: give imagery {url, z} once (a tile URL with {z}, {x}, {y})")?;
        let g = self.georef().ok_or("the map is not placed on the Earth: no latitude / longitude")?;
        // world points of a coarse grid of pixels (the ground plane at z0)
        let step = 8u32;
        let (gw, gh) = (w / step + 2, h / step + 2);
        let centre = cam.smooth_target_now().truncate();
        let reach = a.f64("radius").unwrap_or(150.0).max(50.0) * 6.0;
        let mut grid = Vec::with_capacity((gw * gh) as usize);
        let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for gy in 0..gh {
            for gx in 0..gw {
                let (o, d) = cam.ray((gx * step) as f32, (gy * step) as f32, w as f32, h as f32);
                let p = if d.z < -1e-4 { o + d * ((z0 - o.z) / d.z) } else { o + d * reach };
                let mut q = p.truncate();
                // (towards the horizon: no further than the reach)
                if (q - centre).length() > reach {
                    q = centre + (q - centre).normalize() * reach;
                }
                grid.push(Some(q).filter(|_| d.z < -1e-4));
                let m = g.merc(q);
                lo = lo.min(m);
                hi = hi.max(m);
            }
        }
        let src = crate::geo::sources::Source { name: format!("shot {}", url.len()), url, attribution: String::new(), max_zoom: z, kind: Default::default(), needs_key: false, key_link: String::new(), bounds: None, resolution: 0.0, tile_px: 256, ortho: false, builtin: false };
        let prefs = crate::geo::sources::GeoPrefs::default();
        let fetch = |z: u8, x: u32, y: u32| crate::geo::fetch::tile(&src, &prefs, z, x, y);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mosaic = crate::geo::imagery::Mosaic::fetch(z, lo, hi, false, &fetch, &cancel).map_err(|e| format!("imagery: {e}"))?;
        let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([150, 180, 210, 255]));
        for y in 0..h {
            for x in 0..w {
                let (gx, gy) = ((x / step) as usize, (y / step) as usize);
                let (u, v) = ((x % step) as f64 / step as f64, (y % step) as f64 / step as f64);
                let at = |i: usize, j: usize| grid[j * gw as usize + i];
                let (Some(a0), Some(a1), Some(b0), Some(b1)) = (at(gx, gy), at(gx + 1, gy), at(gx, gy + 1), at(gx + 1, gy + 1)) else { continue };
                let p = (a0 * (1.0 - u) + a1 * u) * (1.0 - v) + (b0 * (1.0 - u) + b1 * u) * v;
                let c = mosaic.colour(g.merc(p));
                out.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], 255]));
            }
        }
        Ok(out)
    }
}
