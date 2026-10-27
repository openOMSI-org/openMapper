//! The window's GPU: wgpu (openOMSI's fork) on Metal, Vulkan, Direct3D 12 or GL.

use std::sync::Arc;

use winit::window::Window;

pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The graphics memory ran out (wgpu reported it instead of a creation succeeding).
static OUT_OF_MEMORY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the graphics memory ran out since the last time asked. The renderer lets go of what
/// it can rebuild (the models, the far ground) and goes on, in place of the editor closing.
pub fn take_out_of_memory() -> bool {
    OUT_OF_MEMORY.swap(false, std::sync::atomic::Ordering::Relaxed)
}

pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub adapter_name: String,
    /// The format the 3D view is drawn in (the sRGB view of the surface; the interface
    /// draws into the plain one, as egui wants).
    pub scene_format: wgpu::TextureFormat,
    pub depth: wgpu::TextureView,
    pub msaa: wgpu::TextureView,
    /// Samples a pixel of the 3D view (1: no edge smoothing, drawn straight into the frame).
    pub samples: u32,
    /// The most samples a pixel this adapter draws for both the colour and the depth target.
    pub max_samples: u32,
}

impl Gpu {
    pub fn new(window: Arc<Window>) -> anyhow::Result<Gpu> {
        // (the first way that opens a device wins: every backend, then GL alone, then the
        // software adapter; an old driver that fails one of them still starts the editor)
        let attempts = [(wgpu::Backends::PRIMARY | wgpu::Backends::GL, false), (wgpu::Backends::GL, false), (wgpu::Backends::PRIMARY | wgpu::Backends::GL, true)];
        let mut found = None;
        let mut last = anyhow::anyhow!("no graphics adapter");
        for (backends, fallback) in attempts {
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.backends = backends;
            let instance = wgpu::Instance::new(descriptor);
            let surface = match instance.create_surface(window.clone()) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("graphics {backends:?} (software {fallback}): no surface: {e}");
                    last = e.into();
                    continue;
                }
            };
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: fallback,
                ..Default::default()
            })) {
                Ok(adapter) => {
                    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                        label: Some("openMapper"),
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                        // (big maps: blocks are not set aside beyond what is used)
                        memory_hints: wgpu::MemoryHints::MemoryUsage,
                        ..Default::default()
                    })) {
                        Ok((device, queue)) => {
                            found = Some((surface, adapter, device, queue));
                            break;
                        }
                        Err(e) => {
                            log::warn!("graphics {backends:?} (software {fallback}): no device: {e}");
                            last = e.into();
                        }
                    }
                }
                Err(e) => {
                    log::warn!("graphics {backends:?} (software {fallback}): no adapter: {e}");
                    last = e.into();
                }
            }
        }
        let Some((surface, adapter, device, queue)) = found else { return Err(last) };
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?})", info.name, info.backend);
        // (running out of graphics memory is not a reason to close the editor: the renderer
        // frees what it can make again; any other error is fatal, as wgpu has it by default)
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| match e {
            wgpu::Error::OutOfMemory { source } => {
                log::error!("graphics memory ran out: {source}");
                OUT_OF_MEMORY.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            other => panic!("wgpu error: {other}"),
        }));
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // (8 bits a channel first: a surface may also list formats such as Rgba16Unorm, which
        // need device features this editor does not ask for; the native Wayland path leads with one)
        let usable = |f: &wgpu::TextureFormat| device.features().contains(f.required_features());
        let format = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
            .into_iter()
            .find(|f| caps.formats.contains(f))
            .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb() && usable(f)))
            .or_else(|| caps.formats.iter().copied().find(usable))
            .unwrap_or(caps.formats[0])
            .remove_srgb_suffix();
        let scene_format = format.add_srgb_suffix();
        // (COPY_SRC: the screenshot mode reads the picture back)
        let copy = if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | copy,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            // (profiling: as fast as it can, not held to the screen)
            present_mode: if crate::render::profile::enabled() { wgpu::PresentMode::AutoNoVsync } else { wgpu::PresentMode::AutoVsync },
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if scene_format != format { vec![scene_format] } else { vec![] },
        };
        surface.configure(&device, &config);
        let supported = |f: wgpu::TextureFormat| adapter.get_texture_format_features(f).flags.supported_sample_counts();
        let (colour, depth_counts) = (supported(scene_format), supported(DEPTH));
        let max_samples = [8u32, 4, 2].into_iter().find(|n| colour.contains(n) && depth_counts.contains(n)).unwrap_or(1);
        let samples = max_samples.min(4);
        let (depth, msaa) = targets(&device, &config, samples);
        Ok(Gpu { surface, device, queue, config, scene_format, adapter_name: format!("{} ({:?})", info.name, info.backend), depth, msaa, samples, max_samples })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        let (d, m) = targets(&self.device, &self.config, self.samples);
        self.depth = d;
        self.msaa = m;
    }

    /// Change the samples a pixel (the renderer is made again after it).
    pub fn set_samples(&mut self, n: u32) {
        let n = if n > self.max_samples { self.max_samples } else { n };
        self.samples = n;
        let (d, m) = targets(&self.device, &self.config, n);
        self.depth = d;
        self.msaa = m;
    }
}

fn targets(device: &wgpu::Device, c: &wgpu::SurfaceConfiguration, samples: u32) -> (wgpu::TextureView, wgpu::TextureView) {
    let size = wgpu::Extent3d { width: c.width, height: c.height, depth_or_array_layers: 1 };
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size,
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("msaa"),
        size,
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: c.format.add_srgb_suffix(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    (depth.create_view(&Default::default()), msaa.create_view(&Default::default()))
}
