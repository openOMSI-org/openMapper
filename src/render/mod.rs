//! Drawing the map: the ground of every tile (with its painted layers), the splines and the
//! scenery objects with their materials and PBR maps, the sky, and the editor's overlays.
//! Also renders the little pictures of the asset catalogue.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{DVec2, DVec3, Mat4, Vec3, Vec4};
use omsi_geometry::{MeshData, SplineCurve};
use omsi_map::{MapSpline, TERRAIN_SAMPLES};
use wgpu::util::DeviceExt;

use crate::assets::{Alpha, Assets, Material, ObjectModel, SplineModel};
use crate::camera::Camera;
use crate::doc::{tile_origin, tile_size, Document, Sel, TileKey};
use crate::place::Pose;
use crate::gpu::{Gpu, DEPTH};

mod build;
mod cull;
pub mod daylight;
pub mod imagery;
pub mod profile;
pub mod quality;
pub mod traffic_lights;

use cull::{Aabb, Frustum};

const COMMON: &str = include_str!("common.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    origin: [f32; 4],
    eye: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    fog: [f32; 4],
    brush: [f32; 4],
    brush_color: [f32; 4],
    grid: [f32; 4],
    light_vp: [[f32; 4]; 4],
    shadow: [f32; 4],
    light_b: [f32; 4],
    light_c: [f32; 4],
}


#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TerrainVertex {
    pos: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Instance {
    m: [[f32; 4]; 4],
    tint: [f32; 4],
    params: [f32; 4],
}

/// Tints of what is selected and what is under the mouse.
// (a selected thing glows orange: a negative alpha asks mesh.wgsl for the rim light)
const SELECTED: [f32; 4] = [0.98, 0.52, 0.16, -1.0];
const HOVERED: [f32; 4] = [1.0, 1.0, 1.0, 0.2];

/// Surface codes of the depth pull (see mesh.wgsl).
const SURF_SPLINE: f32 = 1.0;
/// The IDCode a tile's water is kept under among the splines' meshes (no spline has it).
const WATER: i64 = i64::MIN;
const SURF_OBJECT: f32 = 1.25;

impl Instance {
    pub fn new(m: Mat4, tint: [f32; 4]) -> Instance {
        Instance { m: m.to_cols_array_2d(), tint, params: [0.0; 4] }
    }

    pub fn surface(mut self, code: f32) -> Instance {
        self.params[0] = code;
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayVertex {
    pos: [f32; 3],
    color: [f32; 4],
}

/// A point of the editor's overlay (world metres).
#[derive(Clone, Copy)]
pub struct OverlayPoint {
    pub pos: DVec3,
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MatUniform {
    color: [f32; 4],
    mode: [f32; 4],
    pbr: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TileUniform {
    origin: [f32; 4],
    flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GroundUniform {
    layers: [[f32; 4]; 9],
    count: [f32; 4],
}

/// The brush circle drawn on the ground.
#[derive(Clone, Copy)]
pub struct Brush {
    pub centre: DVec2,
    pub radius: f32,
    pub color: [f32; 4],
}

/// An object shown that is not (yet) on the map: what the tool is about to place.
pub struct Ghost {
    pub rel: String,
    pub world: DVec3,
    pub rot: [f64; 3],
    pub tint: [f32; 4],
}

/// What a frame shows besides the map.
#[derive(Default)]
pub struct FrameExtras {
    pub brush: Option<Brush>,
    pub grid: bool,
    pub overlay: Vec<OverlayPoint>,
    pub ghosts: Vec<Ghost>,
    /// Splines being drawn: (type, curve in world coordinates).
    pub ghost_splines: Vec<(String, MapSpline, [f32; 4])>,
    pub selected: Option<Sel>,
    /// Selected together with `selected`.
    pub selected_more: std::collections::HashSet<Sel>,
    pub hover: Option<Sel>,
    pub highlight_tile: Option<TileKey>,
    /// Draw what the game does not show (start points, car parks, AI paths, helpers).
    pub show_technical: bool,
    /// Layers left out of the view: trees, other objects, splines (View menu).
    pub hide: [bool; 3],
    /// Objects drawn at another size along their own axes (a scale being dragged); zero
    /// hides one (an object whose mesh is being edited is drawn by the edit view).
    pub resized: Vec<(Sel, DVec3)>,
    /// Triangles drawn solid among the map (hidden by what is in front, hiding what is
    /// behind): a mesh being edited.
    pub solid: Vec<OverlayPoint>,
    /// Like `overlay`, but hidden by what is in front: its edges and vertices.
    pub overlay_depth: Vec<OverlayPoint>,
}

struct GpuMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    ranges: Vec<(u32, u32, u32)>,
    /// Drawn as the game draws it: the back of a face shows nothing (an object's mesh, not a
    /// road or an overlay).
    one_sided: bool,
}

struct GpuTile {
    vb: wgpu::Buffer,
    uniform: wgpu::Buffer,
    /// Its own paint masks, made when it has paint (else the shared blank one: a tile of
    /// plain ground costs the GPU its terrain alone, so a map of thousands of tiles fits).
    masks: Option<[wgpu::Texture; 2]>,
    /// Where the ground is cut away under roads (alpha 0), `CUT` texels a side; its own only
    /// while something is cut (else the shared uncut one).
    cut: Option<wgpu::Texture>,
    cut_key: u64,
    bind: wgpu::BindGroup,
    terrain_rev: u64,
    mask_rev: u64,
    /// The ground's walls down from the rims of its holes to the roads' edges (vertices,
    /// indices, index count; openOMSI's `terrain_hole_walls`).
    walls: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    /// What its uniform holds now.
    uniform_now: [f32; 8],
    /// Its paint masks' and ground cut's views (the bind group is made again with them
    /// when its imagery changes) and the imagery's revision bound.
    views: [wgpu::TextureView; 3],
    imagery_rev: u64,
    /// Its box relative to the render anchor.
    bbox: Aabb,
}

/// Texels a side of a tile's ground cut (openOMSI's surface raster).
const CUT: usize = 512;
/// How near the ground a surface must lie for the ground under it to be cut (openOMSI's
/// default `OMSI_SURFACE_FLUSH`).
const FLUSH: f32 = 0.12;

struct GpuTexture {
    albedo: wgpu::TextureView,
    normal: Option<wgpu::TextureView>,
    orm: Option<wgpu::TextureView>,
    flags: [f32; 4],
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MatKey {
    texture: Option<PathBuf>,
    glow: Option<(PathBuf, bool)>,
    alpha: u8,
    color: [u8; 4],
}

struct GpuModel {
    parts: Vec<(GpuMesh, Vec<Material>)>,
    /// Not seen in the game: casts no shadow.
    technical: bool,
}

struct Batch {
    model: String,
    tree: bool,
    first: u32,
    count: u32,
    /// The batch's instances by place on the map (a tile's worth each), so what lies out of
    /// view or too far off for its size can be left out.
    runs: Vec<Run>,
}

struct Run {
    first: u32,
    count: u32,
    bounds: Aabb,
    /// The largest of its instances (m).
    radius: f32,
}

/// An instance waiting to be put in a batch: its bounding radius and what selects it.
#[derive(Clone)]
struct Pending {
    inst: Instance,
    radius: f32,
    sel: Option<Sel>,
}

/// A signal head of the map, kept between the frames that make the scene over: the program it
/// takes its state from (`[varparent]`) and the model its lenses are cut from, so that what is
/// lit is worked out each frame without walking the tiles.
struct Head {
    /// The base model's key (its labels resolved), for its lenses' own models.
    key: String,
    model: Arc<ObjectModel>,
    program: Option<Arc<ObjectModel>>,
    /// The light of the program it shows: its first label (`[stringvarnamelist]`), which
    /// openMapper writes as the light's place; none names the first.
    light: usize,
    /// Where it stands, relative to the render anchor.
    mat: Mat4,
    radius: f32,
    sel: Sel,
}

type Groups = HashMap<String, (bool, Vec<Pending>)>;

impl Batch {
    /// One batch that is always drawn whole (the ghosts).
    fn whole(model: String, tree: bool, first: u32, count: u32) -> Batch {
        Batch { model, tree, first, count, runs: vec![Run { first, count, bounds: Aabb::ALL, radius: 1e6 }] }
    }

    /// The instance ranges worth drawing (neighbouring runs drawn as one).
    fn visible(&self, f: &Frustum, eye: Vec3, far: f32, radii: f32, out: &mut Vec<std::ops::Range<u32>>) {
        out.clear();
        for r in &self.runs {
            if !f.sees(&r.bounds) || !cull::near_enough(&r.bounds, r.radius, eye, radii) || r.bounds.distance(eye) > far {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.end == r.first => last.end = r.first + r.count,
                _ => out.push(r.first..r.first + r.count),
            }
        }
    }
}

struct SplineEntry {
    hash: u64,
    mesh: Option<GpuMesh>,
    /// The mesh on the CPU (relative to its tile's corner) and its bounds there, for the
    /// ground cut; whether it cuts the ground, and whether it carries traffic.
    cpu: Option<Arc<MeshData>>,
    bounds: [f32; 4],
    /// Its box relative to the render anchor.
    bbox: Aabb,
    cuts: bool,
    drivable: bool,
    /// Not seen in the game (an AI path, `[onlyeditor]`): casts no shadow.
    technical: bool,
    /// Left out of the view this frame (what the game does not show, hidden).
    hidden: bool,
    /// Its `[spline_terrain_align]` outlines (world x, y).
    outlines: Vec<Vec<DVec2>>,
    /// The same with the heights of the road's edge (world), for the ground's walls.
    rims: Vec<Vec<DVec3>>,
    materials: Vec<Material>,
    instance: u32,
    /// Drawn as part of its tile's chunk (then it has no mesh of its own on the GPU).
    in_chunk: bool,
}

/// A tile's splines in one mesh, a draw per material: thousands of splines become a few
/// hundred draws. The selected and hovered spline (tinted) stays out and is drawn on its
/// own. Editor-only splines (no shadow) go in a second chunk, not this one.
struct Chunk {
    /// The splines in it (IDCode, revision hash).
    members: HashMap<i64, u64>,
    mesh: Option<GpuMesh>,
    materials: Vec<Material>,
    bbox: Aabb,
    instance: u32,
}

pub struct Thumb {
    pub id: egui::TextureId,
    _tex: wgpu::Texture,
}

pub const THUMB: u32 = 112;

pub struct Renderer {
    /// The hour the view is lit at (View menu; None: openOMSI's plain day).
    pub daytime: Option<daylight::Daytime>,
    /// The splines are left out of the view (FrameExtras::hide).
    hide_splines: bool,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    thumb_globals: wgpu::Buffer,
    thumb_globals_bind: wgpu::BindGroup,
    /// The sun's shadow map, and the globals the pass that draws it uses (its camera is
    /// the sun; no shadow map bound while it is drawn into).
    shadow_view: wgpu::TextureView,
    shadow_globals: wgpu::Buffer,
    shadow_globals_bind: wgpu::BindGroup,
    shadow_pipeline: wgpu::RenderPipeline,
    tile_layout: wgpu::BindGroupLayout,
    ground_layout: wgpu::BindGroupLayout,
    mat_layout: wgpu::BindGroupLayout,
    terrain_pipeline: wgpu::RenderPipeline,
    mesh_opaque: wgpu::RenderPipeline,
    mesh_blend: wgpu::RenderPipeline,
    /// The same with the back faces culled, as the game draws an object's mesh (text on a sign
    /// reads the one way round, a window modelled once per side shows one glass).
    mesh_opaque_cull: wgpu::RenderPipeline,
    mesh_blend_cull: wgpu::RenderPipeline,
    spline_opaque: wgpu::RenderPipeline,
    ghost_pipeline: wgpu::RenderPipeline,
    spline_blend: wgpu::RenderPipeline,
    /// The same with the back faces culled, as the game draws a spline (see
    /// `omsi_geometry::build_spline_mesh`: every profile face is oriented, and a fence or
    /// guard rail is two faces a couple of centimetres apart, one per side, which drawn from
    /// both sides fight in the depth buffer and flicker as the camera moves).
    spline_opaque_cull: wgpu::RenderPipeline,
    spline_blend_cull: wgpu::RenderPipeline,
    thumb_opaque: wgpu::RenderPipeline,
    thumb_blend: wgpu::RenderPipeline,
    thumb_sky: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    overlay_buf: Option<(wgpu::Buffer, u32)>,
    solid_pipeline: wgpu::RenderPipeline,
    solid_buf: Option<(wgpu::Buffer, u32)>,
    overlay_depth_pipeline: wgpu::RenderPipeline,
    overlay_depth_buf: Option<(wgpu::Buffer, u32)>,
    terrain_index: wgpu::Buffer,
    terrain_index_count: u32,
    tiles: HashMap<TileKey, GpuTile>,
    /// The tiles drawn when only a window of the map is loaded (None: all of them), and a
    /// number that changes with the set.
    live: Option<std::collections::HashSet<TileKey>>,
    live_rev: u64,
    ground_bind: Option<wgpu::BindGroup>,
    ground_key: String,
    linear: wgpu::Sampler,
    mask_sampler: wgpu::Sampler,
    /// Pictures over single tiles' ground (`src/geo`).
    pub imagery: imagery::Imagery,
    white: wgpu::TextureView,
    flat_normal: wgpu::TextureView,
    default_orm: wgpu::TextureView,
    zero_mask: wgpu::TextureView,
    /// Nothing cut from the ground (every texel opaque), for the tiles with no cut of their own.
    uncut: wgpu::TextureView,
    textures: HashMap<PathBuf, GpuTexture>,
    materials: HashMap<MatKey, wgpu::BindGroup>,
    /// How each picture's transparency really is (opaque, cut out, fading), read from the
    /// picture itself. A `[matl_alpha] 2` slot showing one is drawn the way the picture says
    /// rather than blended, so that its solid part writes depth and what is drawn after it
    /// cannot come over it; Omsi.exe writes depth there too (its ZWRITEENABLE is cleared by
    /// `[matl_noZwrite]` alone). The rule travels with the content, not with a model's name.
    picture_alpha: HashMap<PathBuf, Alpha>,
    models: HashMap<String, Option<GpuModel>>,
    tree_mesh: GpuMesh,
    marker: GpuMesh,
    instances: Option<wgpu::Buffer>,
    /// The map's object instances (their own buffer: sent when the map changes, a few of
    /// them when the selection's tint does).
    cached_scene: Vec<Instance>,
    scene_buf: Option<wgpu::Buffer>,
    scene_by_sel: HashMap<Sel, Vec<u32>>,
    /// Each tile's object instances (by what they were made from).
    tile_groups: HashMap<TileKey, (u64, Groups)>,
    /// Counts up when models have come in since the tiles last looked.
    asset_epoch: u64,
    assets_seen: (usize, usize),
    assets_seen_at: std::time::Instant,
    tinted: Vec<(Sel, [f32; 4])>,
    /// Instances drawn resized, and their matrices as built.
    resized: Vec<(Sel, DVec3)>,
    resized_orig: HashMap<u32, [[f32; 4]; 4]>,
    /// The splines' and ghosts' instances as last sent.
    dyn_last: Vec<Instance>,
    blank: Instance,
    /// Materials may be missing (new things to draw, textures still coming).
    warm_pending: bool,
    warm_seen_pending: usize,
    /// The point everything is drawn relative to (a tile corner of the map).
    anchor: DVec3,
    anchor_for: u64,
    /// The anchor is set (0, 0 is one too: a new map lies round the origin).
    anchored: bool,
    /// Each tile with its chrono patches laid over, by content revision.
    display: HashMap<TileKey, (u64, Arc<omsi_map::Tile>)>,
    /// Folders ground textures are looked for in besides the map's own.
    pub roots: Vec<PathBuf>,
    batches: Vec<Batch>,
    ghost_batches: Vec<Batch>,
    /// The lenses lit at the signal heads, drawn from the buffer that changes every frame.
    signal_batches: Vec<Batch>,
    /// The map's signal heads, gathered when the scene is made over.
    signal_heads: Vec<Head>,
    scene_key: u64,
    splines: HashMap<(TileKey, i64), SplineEntry>,
    /// Each tile's splines' hashes, and the tile revision they were worked out for (hashing a
    /// spline lowercases its file's name, so a map of thousands was an allocation each, each
    /// frame).
    spline_hashes: HashMap<TileKey, (u64, std::sync::Arc<Vec<u64>>)>,
    /// The scene's materials, once each, and the scene they were collected for.
    warm_mats: Vec<Material>,
    warm_for: u64,
    /// How far the walk over them has gone.
    warm_done: usize,
    /// Whether the last walk met one that could not be put up yet (its picture still loading).
    warm_missing: bool,
    /// When the list was last collected.
    warm_collect_at: Option<std::time::Instant>,
    chunks: HashMap<TileKey, Chunk>,
    /// `[onlyeditor]` splines of a tile in one mesh. Drawn with the roads when "what the game
    /// does not show" is on, and never into the shadow map.
    editor_chunks: HashMap<TileKey, Chunk>,
    /// Builds the splines' meshes off the main thread.
    builder: build::Builder,
    ghost_spline_meshes: Vec<(u64, Option<GpuMesh>, Vec<Material>, u32)>,
    /// Thumbnails of the catalogue by asset path (lower case); None: tried, nothing to show.
    pub thumbs: HashMap<String, Option<Thumb>>,
    thumb_depth: wgpu::TextureView,
    pub time: f32,
    /// Timings for `OPENMAPPER_PROFILE`.
    pub profile: profile::Profile,
    pub quality: quality::Quality,
    gpu_target: Option<wgpu::Texture>,
}

fn shader(device: &wgpu::Device, name: &str, src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(name), source: wgpu::ShaderSource::Wgsl(format!("{COMMON}\n{src}").into()) })
}

fn texture_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }
}

fn solid_texture(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4], srgb: bool) -> wgpu::TextureView {
    let t = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("solid"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &rgba,
    );
    t.create_view(&Default::default())
}

/// Upload a picture with its mip chain.
fn upload_image(device: &wgpu::Device, queue: &wgpu::Queue, img: &omsi_texture::Image, srgb: bool) -> wgpu::TextureView {
    let mut levels = vec![img.clone()];
    while levels.last().map(|l| l.width > 1 || l.height > 1).unwrap_or(false) && levels.len() < 12 {
        let next = crate::assets::half(levels.last().unwrap());
        levels.push(next);
    }
    let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("texture"),
        size: wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, l) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &l.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * l.width), rows_per_image: Some(l.height) },
            wgpu::Extent3d { width: l.width, height: l.height, depth_or_array_layers: 1 },
        );
    }
    tex.create_view(&Default::default())
}

/// A tile's bind group: its uniform, paint masks, ground cut and imagery.
fn tile_bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, uniform: &wgpu::Buffer, views: &[wgpu::TextureView; 3], mask_sampler: &wgpu::Sampler, imagery: &imagery::Imagery, k: TileKey) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[1]) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(mask_sampler) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&views[2]) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(imagery.view(k).0) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&imagery.sampler) },
        ],
    })
}

fn upload_mesh(device: &wgpu::Device, m: &MeshData) -> GpuMesh {
    let verts: Vec<MeshVertex> = (0..m.positions.len())
        .map(|i| MeshVertex {
            pos: m.positions[i].to_array(),
            normal: m.normals.get(i).copied().unwrap_or(Vec3::Z).to_array(),
            uv: m.uvs.get(i).map(|u| u.to_array()).unwrap_or([0.0, 0.0]),
        })
        .collect();
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh ib"), contents: bytemuck::cast_slice(&m.indices), usage: wgpu::BufferUsages::INDEX });
    GpuMesh { vb, ib, ranges: m.ranges.clone(), one_sided: m.one_sided }
}

fn mesh_layouts() -> [wgpu::VertexBufferLayout<'static>; 2] {
    const V: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
    const I: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
    [
        wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<MeshVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &V },
        wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &I },
    ]
}

fn hash_of<T: std::hash::Hash>(v: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// The order a selection is taken in for tinting: a thing's kind, then its tile, then its
/// IDCode (a key of numbers, not of names made for each).
fn sel_order(s: Sel) -> (u8, i32, i32, i64) {
    match s {
        Sel::Object(k, id) => (0, k.0, k.1, id),
        Sel::Spline(k, id) => (1, k.0, k.1, id),
        Sel::Attached(k, id) => (2, k.0, k.1, id),
    }
}

fn spline_hash(s: &MapSpline, origin: DVec2) -> u64 {
    let nums = [s.pos[0], s.pos[1], s.pos[2], s.heading, s.length, s.radius, s.grad_start, s.grad_end, s.delta_h.unwrap_or(-1e9), s.cant_start, s.cant_end, s.skew_start, s.skew_end, s.tex_offset, origin.x, origin.y];
    let bits: Vec<u64> = nums.iter().map(|v| v.to_bits()).collect();
    hash_of(&(s.file.to_lowercase(), bits, s.mirror, s.terrain_align.map(f64::to_bits), s.terrain_align_flag))
}

/// The matrix that puts a map object where it stands.
pub fn object_matrix(world: DVec3, rot: [f64; 3]) -> Mat4 {
    Mat4::from_translation(world.as_vec3()) * omsi_geometry::object_rotation(omsi_geometry::map_rotation(rot))
}

/// Which of a set of four pipelines a mesh is drawn with: back faces are culled when the mesh
/// is one-sided, and blended ones go in the pass that does not write depth.
///
/// A spline's faces are oriented for one side (`omsi_geometry::build_spline_mesh` turns its
/// indices to the content meshes' clockwise front and sets `one_sided`), and a fence or guard
/// rail is two faces a couple of centimetres apart, one per side: drawn from both sides they
/// fight in the depth buffer and flicker as the camera moves. Object meshes have been drawn
/// from one side since they were first read; splines were not.
fn pipeline_index(one_sided: bool, blend: bool) -> usize {
    match (one_sided, blend) {
        (false, false) => 0,
        (false, true) => 1,
        (true, false) => 2,
        (true, true) => 3,
    }
}

impl Renderer {
    /// The pipeline an object's mesh is drawn with, set only when it changes.
    fn set_object_pipeline<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, one_sided: bool, blend: bool, set: &mut Option<bool>) {
        if *set == Some(one_sided) {
            return;
        }
        *set = Some(one_sided);
        pass.set_pipeline([&self.mesh_opaque, &self.mesh_blend, &self.mesh_opaque_cull, &self.mesh_blend_cull][pipeline_index(one_sided, blend)]);
    }

    /// The pipeline a spline or junction plate is drawn with, set only when it changes. The
    /// same rule as an object's, for the reason in `pipeline_index`.
    fn set_spline_pipeline<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, one_sided: bool, blend: bool, set: &mut Option<bool>) {
        if *set == Some(one_sided) {
            return;
        }
        *set = Some(one_sided);
        pass.set_pipeline([&self.spline_opaque, &self.spline_blend, &self.spline_opaque_cull, &self.spline_blend_cull][pipeline_index(one_sided, blend)]);
    }

    pub fn new(gpu: &Gpu, quality: quality::Quality) -> Renderer {
        let device = &gpu.device;
        let samples = gpu.samples;
        let queue = &gpu.queue;
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                uniform_entry(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
            ],
        });
        let depth_texture = |label: &str, size: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let shadow_view = depth_texture("shadow map", quality.shadow_size().max(1));
        // (bound where no shadow is wanted: thumbnails, and the shadow pass itself)
        let no_shadow = depth_texture("no shadow", 1);
        let compare = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let mk_globals = |shadow: &wgpu::TextureView| {
            let b = device.create_buffer(&wgpu::BufferDescriptor { label: Some("globals"), size: std::mem::size_of::<Globals>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globals"),
                layout: &globals_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&compare) },
                ],
            });
            (b, bg)
        };
        let (globals, globals_bind) = mk_globals(&shadow_view);
        let (thumb_globals, thumb_globals_bind) = mk_globals(&no_shadow);
        let (shadow_globals, shadow_globals_bind) = mk_globals(&no_shadow);
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile"),
            entries: &[uniform_entry(0), texture_entry(1, wgpu::TextureViewDimension::D2), texture_entry(2, wgpu::TextureViewDimension::D2), sampler_entry(3), texture_entry(4, wgpu::TextureViewDimension::D2), texture_entry(5, wgpu::TextureViewDimension::D2), sampler_entry(6)],
        });
        let ground_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ground"),
            entries: &[uniform_entry(0), texture_entry(1, wgpu::TextureViewDimension::D2Array), texture_entry(2, wgpu::TextureViewDimension::D2Array), sampler_entry(3)],
        });
        let mat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[uniform_entry(0), texture_entry(1, wgpu::TextureViewDimension::D2), texture_entry(2, wgpu::TextureViewDimension::D2), texture_entry(3, wgpu::TextureViewDimension::D2), sampler_entry(4), texture_entry(5, wgpu::TextureViewDimension::D2)],
        });
        let format = gpu.scene_format;
        let terrain_sh = shader(device, "terrain", include_str!("terrain.wgsl"));
        let mesh_sh = shader(device, "mesh", include_str!("mesh.wgsl"));
        let sky_sh = shader(device, "sky", include_str!("sky.wgsl"));
        let overlay_sh = shader(device, "overlay", include_str!("overlay.wgsl"));

        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("terrain"), bind_group_layouts: &[Some(&globals_layout), Some(&tile_layout), Some(&ground_layout)], immediate_size: 0 });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mesh"), bind_group_layouts: &[Some(&globals_layout), Some(&mat_layout)], immediate_size: 0 });
        let global_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("globals only"), bind_group_layouts: &[Some(&globals_layout)], immediate_size: 0 });

        let depth = |write: bool, bias: i32| {
            Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(write),
                // (reversed depth: nearer is greater, and a bias towards the eye is positive)
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: bias, slope_scale: if bias != 0 { 1.5 } else { 0.0 }, clamp: 0.0 },
            })
        };
        let blend_alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &terrain_sh,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<TerrainVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3] }],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: depth(true, 0),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState { module: &terrain_sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(format.into())] }),
            multiview_mask: None,
            cache: None,
        });
        let mesh_pipeline_with = |label: &str, fmt: wgpu::TextureFormat, samples: u32, blend: Option<wgpu::BlendState>, write: bool, bias: i32, cull: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState { module: &mesh_sh, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &mesh_layouts() },
                // (as openOMSI's `one_sided_primitive`: the visible side of an object's face
                // arrives clockwise)
                primitive: wgpu::PrimitiveState { cull_mode: cull.then_some(wgpu::Face::Back), front_face: if cull { wgpu::FrontFace::Cw } else { wgpu::FrontFace::Ccw }, ..Default::default() },
                depth_stencil: depth(write, bias),
                multisample: wgpu::MultisampleState { count: samples, alpha_to_coverage_enabled: false, ..Default::default() },
                fragment: Some(wgpu::FragmentState { module: &mesh_sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: fmt, blend, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh_pipeline = |label: &str, fmt: wgpu::TextureFormat, samples: u32, blend: Option<wgpu::BlendState>, write: bool, bias: i32| mesh_pipeline_with(label, fmt, samples, blend, write, bias, false);
        // the sun's view: depth only, sloped faces pushed back against acne
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState { module: &mesh_sh, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &mesh_layouts() },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &mesh_sh, entry_point: Some("fs_shadow"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        let thumb_fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mesh_opaque = mesh_pipeline("mesh", format, samples, None, true, 0);
        let mesh_blend = mesh_pipeline("mesh blend", format, samples, blend_alpha, false, 0);
        let mesh_opaque_cull = mesh_pipeline_with("mesh culled", format, samples, None, true, 0, true);
        let mesh_blend_cull = mesh_pipeline_with("mesh blend culled", format, samples, blend_alpha, false, 0, true);
        // the road being drawn: over everything (the ground is shaped to it only once built)
        let ghost_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ghost"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState { module: &mesh_sh, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &mesh_layouts() },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState { module: &mesh_sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: blend_alpha, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None,
            cache: None,
        });
        let spline_opaque = mesh_pipeline("spline", format, samples, None, true, 0);
        let spline_blend = mesh_pipeline("spline blend", format, samples, blend_alpha, false, 0);
        let spline_opaque_cull = mesh_pipeline_with("spline culled", format, samples, None, true, 0, true);
        let spline_blend_cull = mesh_pipeline_with("spline blend culled", format, samples, blend_alpha, false, 0, true);
        let thumb_opaque = mesh_pipeline("thumb", thumb_fmt, 1, None, true, 0);
        let thumb_blend = mesh_pipeline("thumb blend", thumb_fmt, 1, blend_alpha, false, 0);
        let sky = |fmt: wgpu::TextureFormat, samples: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky"),
                layout: Some(&global_only),
                vertex: wgpu::VertexState { module: &sky_sh, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::GreaterEqual), stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState { module: &sky_sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(fmt.into())] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky_pipeline = sky(format, samples);
        let thumb_sky = sky(thumb_fmt, 1);
        // (reversed depth: what is nearer has the greater depth)
        let overlay_with = |label: &str, write: bool, compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&global_only),
                vertex: wgpu::VertexState {
                    module: &overlay_sh,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<OverlayVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4] }],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState { module: &overlay_sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: blend_alpha, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid_pipeline = overlay_with("solid", true, wgpu::CompareFunction::GreaterEqual);
        let overlay_depth_pipeline = overlay_with("overlay with depth", false, wgpu::CompareFunction::GreaterEqual);
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&global_only),
            vertex: wgpu::VertexState {
                module: &overlay_sh,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<OverlayVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4] }],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState { module: &overlay_sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: blend_alpha, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None,
            cache: None,
        });

        // one index list serves every tile (61 × 61 samples, split as OMSI splits its cells)
        let n = TERRAIN_SAMPLES as u32;
        let mut idx: Vec<u32> = Vec::new();
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = j * n + i;
                let b = a + 1;
                let c = a + n;
                let d = c + 1;
                idx.extend_from_slice(&[a, d, c, a, b, d]);
            }
        }
        let terrain_index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("terrain ib"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mask"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = solid_texture(device, queue, [255, 255, 255, 255], true);
        let flat_normal = solid_texture(device, queue, [128, 128, 255, 255], false);
        let default_orm = solid_texture(device, queue, [255, 200, 0, 255], false);
        let zero_mask = solid_texture(device, queue, [0, 0, 0, 0], false);
        let uncut = solid_texture(device, queue, [255; 4], false);
        let tree_mesh = upload_mesh(device, &crate::assets::tree_mesh());
        let marker = upload_mesh(device, &marker_mesh());
        let thumb_depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("thumb depth"),
                size: wgpu::Extent3d { width: THUMB, height: THUMB, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Renderer {
            hide_splines: false,
            globals,
            globals_bind,
            thumb_globals,
            thumb_globals_bind,
            shadow_view,
            shadow_globals,
            shadow_globals_bind,
            shadow_pipeline,
            tile_layout,
            ground_layout,
            mat_layout,
            terrain_pipeline,
            mesh_opaque,
            mesh_blend,
            mesh_opaque_cull,
            mesh_blend_cull,
            spline_opaque,
            ghost_pipeline,
            spline_blend,
            spline_opaque_cull,
            spline_blend_cull,
            thumb_opaque,
            thumb_blend,
            thumb_sky,
            sky_pipeline,
            overlay_pipeline,
            overlay_buf: None,
            solid_pipeline,
            solid_buf: None,
            overlay_depth_pipeline,
            overlay_depth_buf: None,
            terrain_index,
            terrain_index_count: idx.len() as u32,
            tiles: HashMap::new(),
            live: None,
            live_rev: 0,
            ground_bind: None,
            ground_key: String::new(),
            linear,
            mask_sampler,
            imagery: imagery::Imagery::new(gpu),
            white,
            flat_normal,
            default_orm,
            zero_mask,
            uncut,
            textures: HashMap::new(),
            materials: HashMap::new(),
            picture_alpha: HashMap::new(),
            models: HashMap::new(),
            tree_mesh,
            marker,
            instances: None,
            cached_scene: Vec::new(),
            scene_buf: None,
            scene_by_sel: HashMap::new(),
            tile_groups: HashMap::new(),
            asset_epoch: 0,
            assets_seen: (0, 0),
            assets_seen_at: std::time::Instant::now(),
            tinted: Vec::new(),
            resized: Vec::new(),
            resized_orig: HashMap::new(),
            dyn_last: Vec::new(),
            blank: Instance::new(Mat4::IDENTITY, [0.0; 4]),
            warm_pending: true,
            warm_seen_pending: 0,
            anchor: DVec3::ZERO,
            anchor_for: 0,
            anchored: false,
            display: HashMap::new(),
            roots: Vec::new(),
            batches: Vec::new(),
            ghost_batches: Vec::new(),
            signal_batches: Vec::new(),
            signal_heads: Vec::new(),
            scene_key: 0,
            daytime: None,
            splines: HashMap::new(),
            spline_hashes: HashMap::new(),
            warm_mats: Vec::new(),
            warm_for: 0,
            warm_done: 0,
            warm_missing: false,
            warm_collect_at: None,
            chunks: HashMap::new(),
            editor_chunks: HashMap::new(),
            builder: build::Builder::new(),
            ghost_spline_meshes: Vec::new(),
            thumbs: HashMap::new(),
            thumb_depth,
            time: 0.0,
            profile: Default::default(),
            quality,
            gpu_target: None,
        }
    }

    /// The graphics memory ran out: everything held on the GPU that is made again from the
    /// document and the content when it is needed is let go (the ground, the roads, the models
    /// and their textures), and the next frames build what is in view.
    fn free_memory(&mut self) {
        log::warn!("graphics memory ran out: letting go of the map's graphics ({} tiles, {} models, {} textures)", self.tiles.len(), self.models.len(), self.textures.len());
        self.tiles.clear();
        self.splines.clear();
        self.chunks.clear();
        self.editor_chunks.clear();
        self.builder.reset();
        self.scene_key = 0;
        self.tile_groups.clear();
        self.textures.clear();
        self.picture_alpha.clear();
        self.materials.clear();
        self.models.clear();
        self.anchored = false;
    }

    /// Forget the map (another one is opened).
    pub fn reset_map(&mut self) {
        self.profile.opened = Some(std::time::Instant::now());
        self.tiles.clear();
        self.splines.clear();
        self.chunks.clear();
        self.editor_chunks.clear();
        self.builder.reset();
        self.display.clear();
        self.anchor = DVec3::ZERO;
        self.anchored = false;
        self.ground_key.clear();
        self.ground_bind = None;
        self.scene_key = 0;
        self.tile_groups.clear();
        self.signal_heads.clear();
        self.signal_batches.clear();
    }

    /// Forget loaded models (files changed on disk).
    /// Forget one object's model and the pictures in its folder (its files were written
    /// anew): they are made again when it is next drawn.
    pub fn forget_model(&mut self, rel: &str, dir: &std::path::Path) {
        let k = rel.to_lowercase();
        let labelled = format!("{k}\u{1}");
        let lens = format!("{k}\u{3}");
        self.models.retain(|key, _| *key != k && !key.starts_with(&labelled) && !key.starts_with(&lens));
        self.textures.retain(|p, _| !p.starts_with(dir));
        self.materials.retain(|m, _| !m.texture.as_ref().map(|t| t.starts_with(dir)).unwrap_or(false) && !m.glow.as_ref().map(|g| g.0.starts_with(dir)).unwrap_or(false));
        self.thumbs.remove(&k);
        self.asset_epoch += 1;
        self.tile_groups.clear();
    }

    pub fn reset_models(&mut self) {
        self.models.clear();
        self.materials.clear();
        // (a texture's PBR maps may have been made or taken away)
        self.textures.clear();
        self.picture_alpha.clear();
        self.splines.clear();
        self.chunks.clear();
        self.editor_chunks.clear();
        self.builder.reset();
        self.scene_key = 0;
        self.tile_groups.clear();
    }

    fn globals_for(&self, cam: &Camera, aspect: f32, extras: &FrameExtras, anchor: DVec3) -> Globals {
        let vp = cam.proj(aspect) * cam.view();
        let o = cam.origin() - anchor;
        let eye = cam.eye() - cam.origin();
        let day = daylight::light(self.daytime);
        let sun = day.sun;
        let (n, f) = cam.near_far();
        let _ = n;
        let light = light_view(cam, sun, self.quality.shadow_size().max(1), self.quality.shadow_reach().max(40.0));
        let brush = extras.brush.map(|b| [(b.centre.x - anchor.x) as f32, (b.centre.y - anchor.y) as f32, b.radius, 1.0]).unwrap_or([0.0; 4]);
        let ht = extras.highlight_tile.map(|t| [t.0 as f32, t.1 as f32]).unwrap_or([1e9, 1e9]);
        Globals {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            origin: [o.x as f32, o.y as f32, o.z as f32, 0.0],
            eye: [eye.x as f32, eye.y as f32, eye.z as f32, self.time],
            sun_dir: [sun.x, sun.y, sun.z, 0.8],
            sun_color: [day.sun_color[0], day.sun_color[1], day.sun_color[2], 1.0],
            sky_top: [day.sky_top[0], day.sky_top[1], day.sky_top[2], 1.0],
            sky_horizon: [day.sky_horizon[0], day.sky_horizon[1], day.sky_horizon[2], 1.0],
            // (thick enough by the view's reach that its end is not a hard edge)
            fog: [day.fog[0], day.fog[1], day.fog[2], (1.0 / (f * 0.55)).max(1.6 / self.quality.view_distance())],
            brush,
            brush_color: extras.brush.map(|b| b.color).unwrap_or([1.0; 4]),
            grid: [tile_size() as f32, if extras.grid { 1.0 } else { 0.0 }, ht[0], ht[1]],
            light_vp: light.0.to_cols_array_2d(),
            // (z: how much night it is: the lights of objects on)
            shadow: [if self.quality.shadow_size() > 0 && day.shadows { 1.0 } else { 0.0 }, light.1, day.night, 0.0],
            light_b: [day.light_b[0], day.light_b[1], day.light_b[2], 0.0],
            light_c: [day.light_c[0], day.light_c[1], day.light_c[2], 0.0],
        }
    }

    /// A material's bind group (None while its texture loads).
    fn material(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, assets: &mut Assets, m: &Material) -> Option<&wgpu::BindGroup> {
        // (a picture that cannot be read: the part is drawn in its colour, not left out;
        // the glow one at night the same way, one that cannot be read: no glow)
        let readable = m.texture.as_ref().filter(|p| !assets.texture_failed(p)).cloned();
        let glow_path = m.glow.as_ref().filter(|g| !assets.texture_failed(&g.0)).map(|g| g.0.clone());
        // (the pictures are read before the mode is chosen: the picture itself decides it. A
        // cut-out one — every texel clear or solid, only the antialiased outline between — is
        // what the parked buses and vans of the maps are painted on)
        if let Some(p) = &readable {
            if !self.textures.contains_key(p) {
                let img = assets.texture(p)?;
                let mode = crate::models::alpha_of_pixels(img.image.rgba.chunks_exact(4).map(|q| q[3]));
                let alpha = match mode {
                    0 => Alpha::Opaque,
                    1 => Alpha::Test,
                    _ => Alpha::Blend,
                };
                self.picture_alpha.insert(p.clone(), alpha);
                let albedo = upload_image(device, queue, &img.image, true);
                let normal = img.normal.as_ref().map(|n| upload_image(device, queue, n, false));
                let orm = img.orm.as_ref().map(|n| upload_image(device, queue, n, false));
                self.textures.insert(p.clone(), GpuTexture { albedo, normal, orm, flags: img.pbr_flags });
            }
        }
        if let Some(p) = &glow_path {
            if !self.textures.contains_key(p) {
                let img = assets.texture(p)?;
                let albedo = upload_image(device, queue, &img.image, true);
                self.textures.insert(p.clone(), GpuTexture { albedo, normal: None, orm: None, flags: [0.0; 4] });
            }
        }
        // (a `[matl_alpha] 2` slot showing a picture that is opaque or cut out is drawn that
        // way, as Omsi.exe draws it: its solid part writes depth, so what is drawn after it
        // cannot come over the paint)
        let alpha = effective_alpha(m, &self.picture_alpha);
        let key = mat_key_with(m, alpha);
        if !self.materials.contains_key(&key) {
            let mut flags = [0.0f32; 4];
            let mut has_tex = 0.0;
            if readable.is_some() {
                has_tex = 1.0;
            }
            let t = readable.as_ref().and_then(|p| self.textures.get(p));
            if let Some(t) = t {
                flags = t.flags;
                if t.normal.is_none() {
                    flags[0] = 0.0;
                }
                if t.orm.is_none() {
                    flags[1] = 0.0;
                    flags[2] = 0.0;
                    flags[3] = 0.0;
                }
            }
            // (a blended colour keeps its own opacity: the water without a picture)
            let color = if has_tex > 0.0 { [1.0, 1.0, 1.0, m.color[3].max(0.0)] } else { [m.color[0], m.color[1], m.color[2], if alpha == Alpha::Blend { m.color[3] } else { 1.0 }] };
            let glow_mode = match glow_path.as_ref().filter(|p| self.textures.contains_key(*p)) {
                Some(_) => match m.glow.as_ref().map(|g| g.1) {
                    Some(true) => 1.0,
                    _ => 2.0,
                },
                None => 0.0,
            };
            let u = MatUniform { color, mode: [key.alpha as f32, has_tex, 1.0, glow_mode], pbr: flags };
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("material"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
            let albedo = t.map(|t| &t.albedo).unwrap_or(&self.white);
            let normal = t.and_then(|t| t.normal.as_ref()).unwrap_or(&self.flat_normal);
            let orm = t.and_then(|t| t.orm.as_ref()).unwrap_or(&self.default_orm);
            let glow = glow_path.as_ref().and_then(|p| self.textures.get(p)).map(|t| &t.albedo).unwrap_or(&self.white);
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material"),
                layout: &self.mat_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(albedo) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(normal) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(orm) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.linear) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(glow) },
                ],
            });
            self.materials.insert(key.clone(), bg);
        }
        self.materials.get(&key)
    }

    fn model(&mut self, device: &wgpu::Device, key: &str, m: &ObjectModel) {
        if self.models.contains_key(key) {
            return;
        }
        // (a signal head's lenses wait for the crossing's program: dark until then)
        let parts = m.parts.iter().filter(|p| p.lamp().is_none()).map(|p| (upload_mesh(device, &p.mesh), p.materials.clone())).collect::<Vec<_>>();
        self.models.insert(key.to_string(), Some(GpuModel { parts, technical: m.technical || m.editor_parts }));
    }

    /// The model as an object with these labels shows it: one with text textures gets a
    /// copy of its own per text, the text drawn into those slots, and one whose label picks a
    /// picture (`[matl_freetex]`) gets the picture it names. Returns the model's key.
    fn model_with_labels(&mut self, device: &wgpu::Device, key: &str, m: &ObjectModel, labels: &[String], assets: &Assets) -> String {
        if m.texts.is_empty() || m.parts.iter().all(|p| p.texts.is_empty()) {
            // (no text: a picture picked by a label is still a model of its own per picture)
            if m.parts.iter().all(|p| p.freetex.is_empty()) {
                self.model(device, key, m);
                return key.to_string();
            }
        }
        let shown: Vec<&str> = m.texts.iter().map(|tt| crate::texts::label_of(tt).and_then(|i| labels.get(i)).map(String::as_str).unwrap_or("")).collect();
        // (the pictures the map's labels pick, resolved once per label value)
        let picked: Vec<(&str, Option<PathBuf>)> = m
            .parts
            .iter()
            .flat_map(|p| p.freetex.iter())
            .filter_map(|&(_, i)| labels.get(i).map(String::as_str).map(str::trim).filter(|s| !s.is_empty()).map(|name| (name, assets.freetex_texture(key, name))))
            .collect();
        let key = if picked.is_empty() {
            format!("{key}\u{1}{}", shown.join("\u{1}"))
        } else {
            format!("{key}\u{1}{}\u{2}{}", shown.join("\u{1}"), picked.iter().map(|(n, _)| *n).collect::<Vec<_>>().join("\u{1}"))
        };
        if !self.models.contains_key(&key) {
            // (a signal head's lenses wait for the crossing's program: dark until then)
            let parts = m
                .parts
                .iter()
                .filter(|p| p.lamp().is_none())
                .map(|p| {
                    let mut mats = p.materials.clone();
                    for &(slot, t) in &p.texts {
                        if let Some(mat) = mats.get_mut(slot) {
                            mat.texture = Some(crate::texts::path(&m.texts[t], shown[t]));
                            // (the text's ground is clear: only the letters show)
                            if mat.alpha == Alpha::Opaque {
                                mat.alpha = Alpha::Blend;
                            }
                            mat.color[3] = 1.0;
                        }
                    }
                    // (a slot whose picture the placement's label names: the picture replaces
                    // the slot's own; an empty label or one naming nothing leaves it, as in
                    // Omsi.exe)
                    for &(slot, i) in &p.freetex {
                        let Some(name) = labels.get(i).map(|s| s.trim()).filter(|s| !s.is_empty()) else { continue };
                        let Some(path) = picked.iter().find(|(n, _)| *n == name).and_then(|(_, p)| p.clone()) else {
                            log::debug!("freetex: \"{name}\" is nowhere near {key}");
                            continue;
                        };
                        if let Some(mat) = mats.get_mut(slot) {
                            mat.texture = Some(path);
                        }
                    }
                    (upload_mesh(device, &p.mesh), mats)
                })
                .collect();
            self.models.insert(key.clone(), Some(GpuModel { parts, technical: m.technical || m.editor_parts }));
        }
        key
    }

    /// One lens of a signal head as a model of its own, keyed by its place among the head's
    /// parts: the frame that lights it draws the instance, the head's body being drawn with the
    /// map's objects (its lens meshes have no text of their own, so the head's labels need not
    /// be laid over them).
    fn lens_model(&mut self, device: &wgpu::Device, key: &str, m: &ObjectModel, part: usize) -> String {
        let key = format!("{key}\u{3}{part}");
        if !self.models.contains_key(&key) {
            let parts = m.parts.get(part).map(|p| vec![(upload_mesh(device, &p.mesh), p.materials.clone())]).unwrap_or_default();
            self.models.insert(key.clone(), Some(GpuModel { parts, technical: m.technical || m.editor_parts }));
            // (a lamp's own picture and glow are warmed before it is drawn)
            self.warm_pending = true;
        }
        key
    }

    /// The map's signal heads, their programs and the models their lenses are cut from,
    /// gathered when the scene is made over (the lenses themselves are lit every frame, as the
    /// clock runs the programs).
    fn gather_heads(&mut self, device: &wgpu::Device, doc: &Document, assets: &mut Assets, disp: &[(TileKey, Arc<omsi_map::Tile>)], extras: &FrameExtras, anchor: DVec3) -> Vec<Head> {
        let mut heads = Vec::new();
        for (k, t) in disp {
            let mut parents: HashMap<i64, (Pose, Arc<ObjectModel>)> = HashMap::new();
            for o in &t.objects {
                let Some(model) = assets.object(&o.file) else { continue };
                let pose = Pose::of_object(doc.object_world(*k, o, model.abs_height), o.rot);
                if let Some(h) = self.head_of(device, doc, assets, *k, o, pose.matrix_from(anchor), model.clone(), extras) {
                    heads.push(h);
                }
                parents.insert(o.id, (pose, model));
            }
            // (a head hanging on a pole of its tile, as its body is drawn)
            for o in &t.attach_objects {
                let Some(model) = assets.object(&o.file) else { continue };
                let Some((parent, pmodel)) = o.parent_id.and_then(|p| parents.get(&p)).cloned() else { continue };
                let attach = pmodel.attachments.get(o.attach_index).copied().unwrap_or(Mat4::IDENTITY);
                let pose = parent.attached(&attach, omsi_geometry::map_rotation(o.rot));
                if let Some(h) = self.head_of(device, doc, assets, *k, o, pose.matrix_from(anchor), model, extras) {
                    heads.push(h);
                }
            }
        }
        heads
    }

    /// A signal head, where it stands and the model its lenses are cut from: None for an
    /// object that is no head, or one the view leaves out.
    #[allow(clippy::too_many_arguments)]
    fn head_of(&mut self, device: &wgpu::Device, doc: &Document, assets: &mut Assets, k: TileKey, o: &omsi_map::MapObject, mat: Mat4, model: Arc<ObjectModel>, extras: &FrameExtras) -> Option<Head> {
        if !model.light_head || model.technical && !extras.show_technical || hidden_layer(&model, extras) {
            return None;
        }
        let key = self.model_with_labels(device, &o.file.to_lowercase(), &model, &o.extra, assets);
        let scale = mat.x_axis.truncate().length().max(mat.y_axis.truncate().length()).max(mat.z_axis.truncate().length());
        let program = o.var_parent.and_then(|p| crate::selection::object_by_id(doc, p)).and_then(|(_, p)| assets.object(&p.file));
        let light = o.extra.first().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
        let radius = (model.centre().length() + model.radius()) * scale;
        Some(Head { key, model, program, light, mat, radius, sel: Sel::Object(k, o.id) })
    }

    /// Draw only these tiles (None: all). The others lose their graphics at once; they are
    /// built again when they come back.
    pub fn set_live(&mut self, live: Option<std::collections::HashSet<TileKey>>) {
        if self.live != live {
            self.live = live;
            self.live_rev += 1;
        }
    }

    /// Bring the GPU copy of the map up to date with the document.
    pub fn sync(&mut self, gpu: &Gpu, doc: &Document, assets: &mut Assets, extras: &FrameExtras, cam: &Camera) {
        let device = &gpu.device;
        let queue = &gpu.queue;
        if crate::gpu::take_out_of_memory() {
            self.free_memory();
        }
        // the map's anchor: a tile corner in its middle (everything on the GPU is relative to it)
        if self.anchor_for != doc.global_rev || !self.anchored {
            let keys: Vec<TileKey> = doc.tiles.keys().copied().collect();
            if let Some(mid) = keys.get(keys.len() / 2) {
                let o = tile_origin(*mid);
                if !self.anchored || (DVec3::new(o.x, o.y, 0.0) - self.anchor).length() > 20_000.0 {
                    self.anchored = true;
                    self.anchor = DVec3::new(o.x, o.y, 0.0);
                    self.tiles.clear();
                    self.splines.clear();
                    self.chunks.clear();
                    self.editor_chunks.clear();
                    self.builder.reset();
                    self.scene_key = 0;
                    self.tile_groups.clear();
                }
            }
            self.anchor_for = doc.global_rev;
        }
        let anchor = self.anchor;
        let t_sync = std::time::Instant::now();
        self.sync_ground(gpu, doc, assets);
        let mut t = std::time::Instant::now();
        // tiles
        let live = self.live.clone();
        let is_live = |k: &TileKey| live.as_ref().map(|l| l.contains(k)).unwrap_or(true);
        self.tiles.retain(|k, _| doc.tiles.contains_key(k) && is_live(k));
        let keys: Vec<TileKey> = doc.tiles.keys().copied().filter(|k| is_live(k)).collect();
        for k in keys {
            let t = &doc.tiles[&k];
            let need_new = !self.tiles.contains_key(&k);
            if need_new {
                let vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("tile vb"), size: (TERRAIN_SAMPLES * TERRAIN_SAMPLES * std::mem::size_of::<TerrainVertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("tile u"), size: std::mem::size_of::<TileUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let views = [self.zero_mask.clone(), self.zero_mask.clone(), self.uncut.clone()];
                let bind = tile_bind(device, &self.tile_layout, &uniform, &views, &self.mask_sampler, &self.imagery, k);
                let imagery_rev = self.imagery.view(k).1;
                self.tiles.insert(k, GpuTile { vb, uniform, masks: None, cut: None, cut_key: 0, bind, terrain_rev: 0, mask_rev: 0, walls: None, uniform_now: [f32::NAN; 8], views, imagery_rev, bbox: Aabb::ALL });
            }
            let gt = self.tiles.get_mut(&k).unwrap();
            if gt.imagery_rev != self.imagery.view(k).1 {
                gt.imagery_rev = self.imagery.view(k).1;
                gt.bind = tile_bind(device, &self.tile_layout, &gt.uniform, &gt.views, &self.mask_sampler, &self.imagery, k);
            }
            let o = tile_origin(k) - anchor.truncate();
            let selected = extras.highlight_tile == Some(k);
            let u = TileUniform { origin: [o.x as f32, o.y as f32, 0.0, 0.0], flags: [if selected { 1.0 } else { 0.0 }, self.imagery.opacity(k), 0.0, 0.0] };
            let now: [f32; 8] = bytemuck::cast(u);
            if bytemuck::cast::<_, [u32; 8]>(now) != bytemuck::cast::<_, [u32; 8]>(gt.uniform_now) {
                gt.uniform_now = now;
                queue.write_buffer(&gt.uniform, 0, bytemuck::bytes_of(&u));
            }
            if gt.terrain_rev != t.terrain_rev {
                gt.terrain_rev = t.terrain_rev;
                let verts = terrain_vertices(doc, k);
                let (zl, zh) = verts.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v.pos[2]), h.max(v.pos[2])));
                let ts = tile_size() as f32;
                gt.bbox = Aabb { lo: Vec3::new(o.x as f32, o.y as f32, zl - 10.0), hi: Vec3::new(o.x as f32 + ts, o.y as f32 + ts, zh + 10.0) };
                queue.write_buffer(&gt.vb, 0, bytemuck::cast_slice(&verts));
            }
            if gt.mask_rev != t.mask_rev {
                gt.mask_rev = t.mask_rev;
                let painted = (1..9).any(|layer| matches!(t.masks.get(layer), Some(Some(_))) && doc.global.ground_textures.get(layer).is_some());
                if !painted {
                    // (no paint: the shared blank mask)
                    if gt.masks.take().is_some() {
                        gt.views[0] = self.zero_mask.clone();
                        gt.views[1] = self.zero_mask.clone();
                        gt.bind = tile_bind(device, &self.tile_layout, &gt.uniform, &gt.views, &self.mask_sampler, &self.imagery, k);
                    }
                } else {
                    if gt.masks.is_none() {
                        let mk = || {
                            device.create_texture(&wgpu::TextureDescriptor {
                                label: Some("mask"),
                                size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                                view_formats: &[],
                            })
                        };
                        let masks = [mk(), mk()];
                        gt.views[0] = masks[0].create_view(&Default::default());
                        gt.views[1] = masks[1].create_view(&Default::default());
                        gt.masks = Some(masks);
                        gt.bind = tile_bind(device, &self.tile_layout, &gt.uniform, &gt.views, &self.mask_sampler, &self.imagery, k);
                    }
                    for (half, tex) in gt.masks.as_ref().unwrap().iter().enumerate() {
                        let mut data = vec![0u8; 256 * 256 * 4];
                        for ch in 0..4 {
                            let layer = 1 + half * 4 + ch;
                            let (Some(Some(m)), Some(g)) = (t.masks.get(layer), doc.global.ground_textures.get(layer)) else { continue };
                            let size = crate::doc::mask_size(g);
                            for y in 0..256 {
                                for x in 0..256 {
                                    data[(y * 256 + x) * 4 + ch] = m[(y * size / 256) * size + x * size / 256];
                                }
                            }
                        }
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                            &data,
                            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(256) },
                            wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
                        );
                    }
                }
            }
        }

        self.profile.add("sync.tiles", profile::ms(t));
        t = std::time::Instant::now();
        // every tile as the game shows it (chrono patches laid over)
        self.display.retain(|k, _| doc.tiles.contains_key(k) && is_live(k));
        for (k, t) in doc.tiles.iter().filter(|(k, _)| is_live(k)) {
            if self.display.get(k).map(|d| d.0 != t.content_rev).unwrap_or(true) {
                self.display.insert(*k, (t.content_rev, Arc::new(t.display(k.1).into_owned())));
            }
        }
        let disp: Vec<(TileKey, Arc<omsi_map::Tile>)> = self.display.iter().map(|(k, v)| (*k, v.1.clone())).collect();
        // objects: instance lists are made again when anything about them changed (not
        // when only the selection or the hover moves: those just tint a few instances)
        // (while models come in, the tiles look again a few times a second, not each frame)
        let assets_now = (assets.objects.len(), assets.pending);
        if assets_now != self.assets_seen && (assets.pending == 0 || self.assets_seen_at.elapsed().as_millis() > 400) {
            self.assets_seen = assets_now;
            self.assets_seen_at = std::time::Instant::now();
            self.asset_epoch += 1;
        }
        let mut key_parts: Vec<u64> = doc.tiles.iter().filter(|(k, _)| is_live(k)).flat_map(|(_, t)| [t.content_rev, t.terrain_rev]).collect();
        key_parts.push(self.live_rev);
        key_parts.push(extras.show_technical as u64);
        key_parts.extend(extras.hide.map(|h| h as u64));
        self.hide_splines = extras.hide[2];
        key_parts.push(self.asset_epoch);
        let key = hash_of(&key_parts);
        if key != self.scene_key {
            self.scene_key = key;
            // each tile's instances are kept, and made again only when it changed
            self.tile_groups.retain(|k, _| doc.tiles.contains_key(k) && is_live(k));
            for (k, t) in &disp {
                let Some(td) = doc.tiles.get(k) else { continue };
                let tkey = hash_of(&[td.content_rev, td.terrain_rev, extras.show_technical as u64, self.asset_epoch, extras.hide[0] as u64, extras.hide[1] as u64]);
                if self.tile_groups.get(k).map(|g| g.0 == tkey).unwrap_or(false) {
                    continue;
                }
                let mut groups: Groups = HashMap::new();
                // what [attachObj] records may hang on: (pose, model) by IDCode
                let mut parents: HashMap<i64, (Pose, Arc<ObjectModel>)> = HashMap::new();
                for o in &t.objects {
                    let rel_key = o.file.to_lowercase();
                    let Some(model) = assets.object(&o.file) else { continue };
                    if model.technical && !extras.show_technical || hidden_layer(&model, extras) {
                        continue;
                    }
                    let key = self.model_with_labels(device, &rel_key, &model, &o.extra, assets);
                    let world = doc.object_world(*k, o, model.abs_height);
                    let pose = Pose::of_object(world, o.rot);
                    // (one standing on the ground outside its own tile is never seen in OMSI or
                    // openOMSI: its height is looked for on that tile's ground only)
                    let edge = tile_size() + 1e-3;
                    let inside = (-1e-3..=edge).contains(&o.pos[0]) && (-1e-3..=edge).contains(&o.pos[1]);
                    if model.abs_height || inside {
                        push_object(&mut groups, &key, &model, o.extra.first(), pose.matrix_from(anchor), [0.0; 4], Some(Sel::Object(*k, o.id)), extras.show_technical);
                    }
                    parents.insert(o.id, (pose, model));
                }
                // the rows along the splines (lamps, posts, parking bays)
                let org = tile_origin(*k);
                for a in t.spline_attachments.iter().filter(|_| !extras.hide[1]) {
                    let Some(model) = assets.object(&a.file) else { continue };
                    let rel_key = self.model_with_labels(device, &a.file.to_lowercase(), &model, &a.strings, assets);
                    let objs = crate::place::row_objects(a, &t.splines, org);
                    let first = objs.iter().map(|o| o.index).min();
                    // (each record is a thing of its own: clicking one of its copies picks it,
                    // and they take its tint - selected or hovered - as an object does)
                    let own = Some(Sel::Attached(*k, a.id));
                    for ro in &objs {
                        if a.repeater.is_none() && Some(ro.index) == first {
                            parents.entry(a.id).or_insert((ro.pose, model.clone()));
                        }
                        push_object(&mut groups, &rel_key, &model, a.strings.first(), ro.pose.matrix_from(anchor), [0.0; 4], own, extras.show_technical);
                    }
                }
                // objects hanging on others (signs on posts, lamps on masts)
                for o in &t.attach_objects {
                    let Some(model) = assets.object(&o.file) else { continue };
                    if hidden_layer(&model, extras) {
                        continue;
                    }
                    let Some((parent, pmodel)) = o.parent_id.and_then(|p| parents.get(&p)).cloned() else { continue };
                    let rel_key = self.model_with_labels(device, &o.file.to_lowercase(), &model, &o.extra, assets);
                    let attach = pmodel.attachments.get(o.attach_index).copied().unwrap_or(Mat4::IDENTITY);
                    let pose = parent.attached(&attach, omsi_geometry::map_rotation(o.rot));
                    push_object(&mut groups, &rel_key, &model, o.extra.first(), pose.matrix_from(anchor), [0.0; 4], Some(Sel::Object(*k, o.id)), extras.show_technical);
                    parents.insert(o.id, (pose, model));
                }
                self.tile_groups.insert(*k, (tkey, groups));
            }
            let mut groups: Groups = HashMap::new();
            for (_, g) in self.tile_groups.values() {
                for (model, (tree, list)) in g {
                    let e = groups.entry(model.clone()).or_insert_with(|| (*tree, Vec::new()));
                    e.1.extend(list.iter().cloned());
                }
            }
            let (instances, batches, by_sel) = flatten(groups, 0);
            self.batches = batches;
            self.scene_by_sel = by_sel;
            self.cached_scene = instances;
            // (the signal heads, for the frames that light their lenses: they are found again
            // here, with everything else that is made over, not every frame)
            let heads = self.gather_heads(device, doc, assets, &disp, extras, anchor);
            self.signal_heads = heads;
            self.tinted.clear();
            self.resized.clear();
            self.resized_orig.clear();
            self.warm_pending = true;
            // (one instance at least: a buffer may not be empty)
            let bytes: &[u8] = if self.cached_scene.is_empty() { bytemuck::bytes_of(&self.blank) } else { bytemuck::cast_slice(&self.cached_scene) };
            let fits = self.scene_buf.as_ref().map(|b| b.size() >= bytes.len() as u64).unwrap_or(false);
            if !fits {
                self.scene_buf = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("scene instances"), size: (bytes.len() as u64 * 5 / 4).max(4096), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
            }
            queue.write_buffer(self.scene_buf.as_ref().unwrap(), 0, bytes);
        }
        // the selected and the hovered object: their instances tinted in place
        let mut want: Vec<(Sel, [f32; 4])> = Vec::new();
        if let Some(s) = extras.selected {
            want.push((s, SELECTED));
        }
        let mut more: Vec<Sel> = extras.selected_more.iter().copied().filter(|s| Some(*s) != extras.selected).collect();
        // (in a fixed order, that the same selection compares equal frame after frame: by
        // numbers, not by a name made for each, which a selection of thousands would pay for
        // every frame)
        more.sort_unstable_by_key(|s| sel_order(*s));
        want.extend(more.into_iter().map(|s| (s, SELECTED)));
        if let Some(s) = extras.hover.filter(|h| Some(*h) != extras.selected && !extras.selected_more.contains(h)) {
            want.push((s, HOVERED));
        }
        if want != self.tinted {
            let old = std::mem::take(&mut self.tinted);
            let size = std::mem::size_of::<Instance>() as u64;
            let set = |sel: Sel, tint: [f32; 4], scene: &mut Vec<Instance>| {
                for &i in self.scene_by_sel.get(&sel).into_iter().flatten() {
                    let inst = &mut scene[i as usize];
                    inst.tint = tint;
                    if let Some(b) = &self.scene_buf {
                        queue.write_buffer(b, i as u64 * size, bytemuck::bytes_of(inst));
                    }
                }
            };
            for (s, _) in &old {
                set(*s, [0.0; 4], &mut self.cached_scene);
            }
            for (s, t) in &want {
                set(*s, *t, &mut self.cached_scene);
            }
            self.tinted = want;
        }
        if extras.resized != self.resized {
            let size = std::mem::size_of::<Instance>() as u64;
            for (i, m) in std::mem::take(&mut self.resized_orig) {
                let inst = &mut self.cached_scene[i as usize];
                inst.m = m;
                if let Some(b) = &self.scene_buf {
                    queue.write_buffer(b, i as u64 * size, bytemuck::bytes_of(inst));
                }
            }
            for (sel, f) in &extras.resized {
                for &i in self.scene_by_sel.get(sel).into_iter().flatten() {
                    let inst = &mut self.cached_scene[i as usize];
                    self.resized_orig.entry(i).or_insert(inst.m);
                    for (c, k) in [f.x, f.y, f.z].into_iter().enumerate() {
                        for r in 0..4 {
                            inst.m[c][r] *= k as f32;
                        }
                    }
                    if let Some(b) = &self.scene_buf {
                        queue.write_buffer(b, i as u64 * size, bytemuck::bytes_of(inst));
                    }
                }
            }
            self.resized = extras.resized.clone();
        }
        let _ = cam;
        self.profile.add("sync.objects", profile::ms(t));
        t = std::time::Instant::now();

        // splines
        let mut alive: Vec<(TileKey, i64)> = Vec::new();
        let mut complete = assets.pending == 0;
        for (key, e) in self.builder.take() {
            self.splines.insert(key, e);
            self.warm_pending = true;
        }
        let start = std::time::Instant::now();
        let mut spline_instances: Vec<Instance> = Vec::new();
        // (each tile's splines' hashes, made again only when the tile changed)
        for (k, t) in &disp {
            let rev = self.display.get(k).map(|d| d.0).unwrap_or(0);
            if self.spline_hashes.get(k).map(|(r, _)| *r != rev).unwrap_or(true) {
                let org = tile_origin(*k);
                let v: Vec<u64> = t.splines.iter().map(|s| spline_hash(s, org)).collect();
                self.spline_hashes.insert(*k, (rev, std::sync::Arc::new(v)));
            }
        }
        self.spline_hashes.retain(|k, _| self.display.contains_key(k));
        for (k, t) in &disp {
            let org = tile_origin(*k);
            let corner = Vec3::new((org.x - anchor.x) as f32, (org.y - anchor.y) as f32, 0.0);
            let hashes = self.spline_hashes.get(k).map(|(_, v)| v.clone()).unwrap_or_default();
            for (i, s) in t.splines.iter().enumerate().filter(|(_, s)| !s.deleted && !s.file.trim().is_empty()) {
                alive.push((*k, s.id));
                let h = hashes.get(i).copied().unwrap_or_else(|| spline_hash(s, org));
                let sel = Some(Sel::Spline(*k, s.id));
                let tint = if extras.selected == sel || extras.selected_more.contains(&Sel::Spline(*k, s.id)) {
                    SELECTED
                } else if extras.hover == sel {
                    HOVERED
                } else {
                    [0.0; 4]
                };
                let entry_ok = self.splines.get(&(*k, s.id)).map(|e| e.hash == h).unwrap_or(false);
                if !entry_ok {
                    let Some(model) = assets.spline(&s.file) else { continue };
                    // (what is being edited is built at once, so it follows the mouse; the
                    // rest on the builder's threads)
                    let editing = extras.selected == sel || extras.selected_more.contains(&Sel::Spline(*k, s.id));
                    if editing && start.elapsed().as_millis() < 25 {
                        self.builder.forget(&(*k, s.id));
                        self.splines.insert((*k, s.id), build::make_entry(&model, s, org, corner, h));
                        self.warm_pending = true;
                    } else {
                        self.builder.request((*k, s.id), h, model, s, org, corner);
                        complete = false;
                    }
                }
                if let Some(e) = self.splines.get_mut(&(*k, s.id)) {
                    e.hidden = spline_hidden(e.technical, extras);
                    e.instance = spline_instances.len() as u32;
                    spline_instances.push(Instance::new(Mat4::from_translation(corner), tint).surface(SURF_SPLINE));
                }
            }
        }
        // each tile's water: one quad at its four corner heights, blended over the ground
        // (an ordinary instance, not a surface: openOMSI's reason, the banks must win)
        let water_tex = ["texture/water.tga", "Texture/water.tga"].iter().map(|r| doc.dir.join(r)).chain(doc.source.iter().map(|s| s.join("texture").join("water.tga"))).find(|p| p.is_file());
        for (k, td) in doc.tiles.iter().filter(|(k, _)| is_live(k)) {
            let Some(w) = td.water else { continue };
            let key = (*k, WATER);
            alive.push(key);
            let org = tile_origin(*k);
            let corner = Vec3::new((org.x - anchor.x) as f32, (org.y - anchor.y) as f32, 0.0);
            let h = hash_of(&(w.map(f32::to_bits), org.x.to_bits(), org.y.to_bits(), &water_tex));
            if self.splines.get(&key).map(|e| e.hash != h).unwrap_or(true) {
                let t = tile_size() as f32;
                let mut m = MeshData::default();
                for (i, (x, y)) in [(0.0, 0.0), (t, 0.0), (0.0, t), (t, t)].into_iter().enumerate() {
                    m.positions.push(Vec3::new(x, y, w[i]));
                    m.normals.push(Vec3::Z);
                    m.uvs.push(glam::Vec2::new(x / 40.0, y / 40.0));
                }
                m.indices.extend_from_slice(&[0, 1, 2, 2, 1, 3]);
                m.ranges.push((0, 6, 0));
                let mut bbox = Aabb::EMPTY;
                m.positions.iter().for_each(|p| bbox.add_sphere(*p + corner, 0.0));
                let material = Material { texture: water_tex.clone(), color: [0.22, 0.38, 0.45, 0.72], alpha: Alpha::Blend, glow: None };
                let mesh = Some(upload_mesh(device, &m));
                self.splines.insert(key, SplineEntry { hash: h, mesh, cpu: None, bounds: [0.0, 0.0, t, t], bbox, cuts: false, drivable: false, technical: true, hidden: false, outlines: Vec::new(), rims: Vec::new(), materials: vec![material], instance: 0, in_chunk: false });
                self.warm_pending = true;
            }
            if let Some(e) = self.splines.get_mut(&key) {
                e.instance = spline_instances.len() as u32;
                spline_instances.push(Instance::new(Mat4::from_translation(corner), [0.0; 4]));
            }
        }
        if self.splines.len() > alive.len() {
            let set: std::collections::HashSet<_> = alive.into_iter().collect();
            self.splines.retain(|k, _| set.contains(k));
        }
        complete &= self.builder.busy() == 0;
        self.profile.count("splines building", self.builder.busy() as u32);
        self.sync_chunks(device, &disp, extras, &mut spline_instances, anchor);

        self.profile.add("sync.splines", profile::ms(t));
        t = std::time::Instant::now();
        complete &= self.cut_ground(gpu, doc, assets, &disp);
        if complete {
            self.profile.ready();
        }
        self.profile.add("sync.cut", profile::ms(t));
        t = std::time::Instant::now();

        // ghosts: objects about to be placed and roads being drawn
        let mut ghost_groups: Groups = HashMap::new();
        for g in &extras.ghosts {
            let key = g.rel.to_lowercase();
            if let Some(model) = assets.object(&g.rel) {
                self.model(device, &key, &model);
                push_object(&mut ghost_groups, &key, &model, None, object_matrix(g.world - anchor, g.rot), g.tint, None, true);
            } else if assets.object_failed(&g.rel) {
                ghost_groups.entry("\u{0}marker".into()).or_default().1.push(Pending { inst: Instance::new(object_matrix(g.world - anchor, g.rot), g.tint), radius: 2.0, sel: None });
            }
        }
        if !self.ghost_spline_meshes.is_empty() || !extras.ghost_splines.is_empty() || !ghost_groups.is_empty() {
            self.warm_pending = true;
        }
        self.ghost_spline_meshes.clear();
        let mut ghost_spline_instances = Vec::new();
        for (rel, s, tint) in &extras.ghost_splines {
            let Some(model) = assets.spline(rel) else { continue };
            let mut s = s.clone();
            s.pos[0] -= anchor.x;
            s.pos[1] -= anchor.y;
            let mesh = build_spline(&model, &s, DVec2::ZERO);
            ghost_spline_instances.push(Instance::new(Mat4::IDENTITY, *tint).surface(SURF_SPLINE));
            self.ghost_spline_meshes.push((0, mesh.map(|m| upload_mesh(device, &m)), model.materials.clone(), 0));
        }

        // what changes from frame to frame (splines, ghosts) in a buffer of its own: the
        // map's objects stay where they are
        let mut all = spline_instances;
        let ghost_spline_base = all.len() as u32;
        for (i, g) in self.ghost_spline_meshes.iter_mut().enumerate() {
            g.3 = ghost_spline_base + i as u32;
        }
        all.extend(ghost_spline_instances);
        let (ghosts, ghost_batches, _) = flatten(ghost_groups, all.len() as u32);
        self.ghost_batches = ghost_batches.into_iter().map(|b| Batch::whole(b.model, b.tree, b.first, b.count)).collect();
        all.extend(ghosts);
        // the signal heads' lenses: one instance for each lamp the crossing's program has lit
        // (a program's phases are seconds of the game's own time, so they run on the editor's
        // clock whatever hour the view is lit at; the lenses are models of their own, the
        // heads' bodies drawn with the map's objects, and they are put here rather than among
        // the tiles because what is lit changes with the clock, not with the map)
        let mut lenses: Vec<(String, Arc<ObjectModel>, Mat4, Option<Sel>, f32, [f32; 4], usize)> = Vec::new();
        for h in &self.signal_heads {
            // (a head Omsi.exe cannot link to a crossing - no `[varparent]`, one that leads
            // nowhere, a light with no phases - shows phase 0: red, as the game leaves it)
            let state = match &h.program {
                Some(p) => match p.lights.get(h.light) {
                    Some(light) if !light.phases.is_empty() => traffic_lights::state_at(&light.phases, p.cycle, self.time),
                    _ => traffic_lights::UNLINKED_STATE,
                },
                None => traffic_lights::UNLINKED_STATE,
            };
            let lit = traffic_lights::Lit::of_state(state);
            if lit.is_dark() {
                continue;
            }
            let tint = if extras.selected == Some(h.sel) || extras.selected_more.contains(&h.sel) {
                SELECTED
            } else if extras.hover == Some(h.sel) {
                HOVERED
            } else {
                [0.0; 4]
            };
            // (a head being resized in the view takes its lenses with it)
            let mut mat = h.mat;
            if let Some((_, f)) = extras.resized.iter().find(|(s, _)| *s == h.sel) {
                for (c, k) in [f.x, f.y, f.z].into_iter().enumerate() {
                    *mat.col_mut(c) *= k as f32;
                }
            }
            for (i, p) in h.model.parts.iter().enumerate() {
                if p.lamp().map(|lamp| lit.shows(lamp)).unwrap_or(false) {
                    lenses.push((h.key.clone(), h.model.clone(), mat, Some(h.sel), h.radius, tint, i));
                }
            }
        }
        let mut lens_groups: Groups = HashMap::new();
        for (key, model, mat, sel, radius, tint, part) in lenses {
            let key = self.lens_model(device, &key, &model, part);
            lens_groups.entry(key).or_default().1.push(Pending { inst: Instance::new(mat, tint), radius, sel });
        }
        let (lens_instances, lens_batches, _) = flatten(lens_groups, all.len() as u32);
        self.signal_batches = lens_batches;
        all.extend(lens_instances);
        if all.is_empty() {
            all.push(self.blank);
        }
        // (unchanged since the last frame: nothing to send)
        if bytemuck::cast_slice::<Instance, u8>(&all) != bytemuck::cast_slice::<Instance, u8>(&self.dyn_last) {
            let bytes: &[u8] = bytemuck::cast_slice(&all);
            let fits = self.instances.as_ref().map(|b| b.size() >= bytes.len() as u64).unwrap_or(false);
            if !fits {
                self.instances = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("instances"), size: (bytes.len() as u64 * 3 / 2).max(4096), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
            }
            queue.write_buffer(self.instances.as_ref().unwrap(), 0, bytes);
            self.dyn_last = all;
        }

        // overlays
        let upload = |list: &[OverlayPoint], label: &str| -> Option<(wgpu::Buffer, u32)> {
            if list.is_empty() {
                return None;
            }
            let verts: Vec<OverlayVertex> = list.iter().map(|p| OverlayVertex { pos: (p.pos - anchor).as_vec3().to_array(), color: p.color }).collect();
            let b = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
            Some((b, list.len() as u32))
        };
        self.overlay_buf = upload(&extras.overlay, "overlay");
        self.solid_buf = upload(&extras.solid, "solid");
        self.overlay_depth_buf = upload(&extras.overlay_depth, "overlay with depth");
        self.profile.add("sync.upload", profile::ms(t));
        t = std::time::Instant::now();
        // materials of everything that will be drawn (when something new is there to draw)
        if self.warm_pending || assets.pending != self.warm_seen_pending {
            self.warm_seen_pending = assets.pending;
            self.warm_pending = self.warm_materials(device, queue, assets);
        }
        self.profile.add("sync.materials", profile::ms(t));
        self.profile.add("sync", profile::ms(t_sync));
    }

    /// Bring the tiles' spline chunks up to date: one whose splines changed is made again at
    /// once (an old one would show the old roads); new ones and ones with splines to add a
    /// few per frame, those splines drawn on their own until then.
    fn sync_chunks(&mut self, device: &wgpu::Device, disp: &[(TileKey, Arc<omsi_map::Tile>)], extras: &FrameExtras, instances: &mut Vec<Instance>, anchor: DVec3) {
        let start = std::time::Instant::now();
        let keep: std::collections::HashSet<TileKey> = disp.iter().map(|d| d.0).collect();
        self.chunks.retain(|k, _| keep.contains(k));
        self.editor_chunks.retain(|k, _| keep.contains(k));
        for (k, t) in disp {
            let org = tile_origin(*k);
            // what belongs in each batch (roads cast a shadow; editor-only splines do not)
            let mut members: HashMap<i64, u64> = HashMap::new();
            let mut editor_members: HashMap<i64, u64> = HashMap::new();
            for s in t.splines.iter().filter(|s| !s.deleted) {
                let Some(e) = self.splines.get(&(*k, s.id)) else { continue };
                let sel = Sel::Spline(*k, s.id);
                let picked = extras.selected == Some(sel) || extras.hover == Some(sel) || extras.selected_more.contains(&sel);
                match spline_batch(e.technical, picked, e.cpu.is_some()) {
                    Some(false) => {
                        members.insert(s.id, e.hash);
                    }
                    Some(true) => {
                        editor_members.insert(s.id, e.hash);
                    }
                    None => {}
                }
            }
            self.refresh_chunk(device, *k, members, false, start);
            self.refresh_chunk(device, *k, editor_members, true, start);
            for s in t.splines.iter().filter(|s| !s.deleted) {
                let road = self.chunks.get(k).and_then(|c| c.members.get(&s.id).copied());
                let editor = self.editor_chunks.get(k).and_then(|c| c.members.get(&s.id).copied());
                let Some(e) = self.splines.get_mut(&(*k, s.id)) else { continue };
                e.in_chunk = road == Some(e.hash) || editor == Some(e.hash);
                if e.in_chunk {
                    e.mesh = None;
                } else if e.mesh.is_none() {
                    e.mesh = e.cpu.as_deref().map(|m| upload_mesh(device, m));
                }
            }
            let at = Instance::new(Mat4::from_translation(Vec3::new((org.x - anchor.x) as f32, (org.y - anchor.y) as f32, 0.0)), [0.0; 4]).surface(SURF_SPLINE);
            if let Some(c) = self.chunks.get_mut(k) {
                c.instance = instances.len() as u32;
                instances.push(at);
            }
            if self.editor_chunks.get(k).map(|c| c.mesh.is_some()).unwrap_or(false) {
                let n = instances.len() as u32;
                self.editor_chunks.get_mut(k).unwrap().instance = n;
                instances.push(at);
            }
        }
    }

    /// Rebuild one tile's merged spline mesh when its members changed. `editor`: the
    /// `[onlyeditor]` batch.
    fn refresh_chunk(&mut self, device: &wgpu::Device, key: TileKey, members: HashMap<i64, u64>, editor: bool, start: std::time::Instant) {
        let (stale, grown) = {
            let chunks = if editor { &self.editor_chunks } else { &self.chunks };
            match chunks.get(&key) {
                Some(c) => (c.members.iter().any(|(id, h)| members.get(id) != Some(h)), members.len() > c.members.len()),
                None => (false, !members.is_empty()),
            }
        };
        // (stale: a spline in it changed or went; else only new ones came, which can wait)
        if !(stale || (grown && start.elapsed().as_millis() < 8)) {
            return;
        }
        let mut ids: Vec<i64> = members.keys().copied().collect();
        ids.sort_unstable();
        let (mesh, materials, bbox) = {
            let parts: Vec<(&MeshData, &[Material], Aabb)> = ids.iter().filter_map(|id| self.splines.get(&(key, *id))).filter_map(|e| e.cpu.as_deref().map(|m| (m, e.materials.as_slice(), e.bbox))).collect();
            merge_meshes(&parts, &self.picture_alpha)
        };
        let mesh = (!mesh.indices.is_empty()).then(|| upload_mesh(device, &mesh));
        let chunks = if editor { &mut self.editor_chunks } else { &mut self.chunks };
        chunks.insert(key, Chunk { members, mesh, materials, bbox, instance: 0 });
    }

    /// The ground cut of the tiles whose roads, surfaces or ground changed, made on the
    /// builder's threads. Whether every tile's cut is up to date.
    fn cut_ground(&mut self, gpu: &Gpu, doc: &Document, assets: &mut Assets, disp: &[(TileKey, Arc<omsi_map::Tile>)]) -> bool {
        let ts = tile_size();
        let mut complete = true;
        let start = std::time::Instant::now();
        let by_key: HashMap<TileKey, &Arc<omsi_map::Tile>> = disp.iter().map(|(k, t)| (*k, t)).collect();
        let keys: Vec<TileKey> = self.tiles.keys().copied().collect();
        for key in keys {
            // what the cut depends on: this tile's and its neighbours' contents, its ground, and
            // how many of their splines and objects are ready
            let mut parts: Vec<u64> = Vec::new();
            let mut ready = 0u64;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = (key.0 + dx, key.1 + dy);
                    if let Some(t) = doc.tiles.get(&n) {
                        parts.push(t.content_rev);
                    }
                    if let Some(t) = by_key.get(&n) {
                        ready += t.splines.iter().filter(|s| self.splines.get(&(n, s.id)).map(|e| e.cpu.is_some()).unwrap_or(false)).count() as u64;
                    }
                }
            }
            parts.push(doc.tiles.get(&key).map(|t| t.terrain_rev).unwrap_or(0));
            parts.push(ready);
            parts.push(assets.objects.len() as u64 - assets.pending.min(assets.objects.len()) as u64);
            let h = hash_of(&parts);
            if self.tiles.get(&key).map(|g| g.cut_key == h).unwrap_or(true) {
                continue;
            }
            complete = false;
            // (what is asked for is gathered here: a few milliseconds' worth a frame)
            if self.builder.cut_pending(key) || start.elapsed().as_millis() > 4 {
                continue;
            }
            // what the cut is made of (on the builder's threads)
            let (x0, y0) = (key.0 as f64 * ts, key.1 as f64 * ts);
            let (x1, y1) = (x0 + ts, y0 + ts);
            let outside = |b: [f64; 4]| b[2] < x0 || b[0] > x1 || b[3] < y0 || b[1] > y1;
            let mut input = build::CutInput { key, hash: h, splines: Vec::new(), objects: Vec::new(), terrain: doc.tiles.get(&key).map(|t| t.terrain.clone()) };
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = (key.0 + dx, key.1 + dy);
                    let Some(t) = by_key.get(&n) else { continue };
                    let org = tile_origin(n);
                    for s in t.splines.iter().filter(|s| !s.deleted) {
                        let Some(e) = self.splines.get(&(n, s.id)) else { continue };
                        let Some(m) = &e.cpu else { continue };
                        let b = [org.x + e.bounds[0] as f64, org.y + e.bounds[1] as f64, org.x + e.bounds[2] as f64, org.y + e.bounds[3] as f64];
                        if !e.cuts || outside(b) {
                            continue;
                        }
                        input.splines.push((m.clone(), org, e.drivable, e.outlines.clone(), e.rims.clone()));
                    }
                    // surface objects (junction plates, car parks) and [terrainhole] cutters
                    for o in &t.objects {
                        let Some(model) = assets.object(&o.file) else { continue };
                        if !model.surface && model.holes.is_empty() {
                            continue;
                        }
                        let pose = Pose::of_object(doc.object_world(n, o, model.abs_height), o.rot);
                        let r = model.radius() as f64 + 5.0;
                        if outside([pose.pos.x - r, pose.pos.y - r, pose.pos.x + r, pose.pos.y + r]) {
                            continue;
                        }
                        input.objects.push((model, pose));
                    }
                }
            }
            self.builder.request_cut(input);
        }
        // the cuts that are done
        for (key, h, cut) in self.builder.take_cuts() {
            let Some(g) = self.tiles.get_mut(&key) else { continue };
            g.walls = cut.walls.map(|m| {
                let verts: Vec<TerrainVertex> = m.positions.iter().zip(&m.normals).map(|(p, n)| TerrainVertex { pos: p.to_array(), normal: n.to_array() }).collect();
                let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("walls vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("walls ib"), contents: bytemuck::cast_slice(&m.indices), usage: wgpu::BufferUsages::INDEX });
                (vb, ib, m.indices.len() as u32)
            });
            g.cut_key = h;
            // (nothing cut: the shared uncut ground; a texture of its own only where roads
            // or plates take the ground away)
            if cut.rgba.iter().all(|&b| b == 255) {
                if g.cut.take().is_some() {
                    g.views[2] = self.uncut.clone();
                    g.bind = tile_bind(&gpu.device, &self.tile_layout, &g.uniform, &g.views, &self.mask_sampler, &self.imagery, key);
                }
                continue;
            }
            if g.cut.is_none() {
                let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("cut"),
                    size: wgpu::Extent3d { width: CUT as u32, height: CUT as u32, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                g.views[2] = tex.create_view(&Default::default());
                g.cut = Some(tex);
                g.bind = tile_bind(&gpu.device, &self.tile_layout, &g.uniform, &g.views, &self.mask_sampler, &self.imagery, key);
            }
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: g.cut.as_ref().unwrap(), mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &cut.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * CUT as u32), rows_per_image: Some(CUT as u32) },
                wgpu::Extent3d { width: CUT as u32, height: CUT as u32, depth_or_array_layers: 1 },
            );
        }
        complete
    }

    /// Whether some are still missing.
    ///
    /// What the scene draws is collected once and kept, and the walk over it carries on where
    /// the last frame's twenty milliseconds ran out: collected anew and started again from the
    /// first material in every frame, a scene whose materials take longer than that to look at
    /// never got through them at all - it spent the twenty milliseconds and began again, every
    /// frame, for ever.
    fn warm_materials(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, assets: &mut Assets) -> bool {
        // (a map still streaming in gives a new scene key nearly every frame: collected again
        // that often, the list itself cost more than uploading what was in it)
        let stale = self.warm_for != self.scene_key;
        let waited = self.warm_collect_at.map(|t| t.elapsed().as_millis() > 500).unwrap_or(true);
        // (collected again only once the walk through the list has finished: starting it over
        // half way threw away the work, and a map that is still streaming gives a new scene key
        // nearly every frame, so it was started over in every frame and never got through)
        let finished = self.warm_done >= self.warm_mats.len();
        if self.warm_mats.is_empty() || (stale && finished && waited) {
            self.warm_mats = self.scene_materials();
            self.warm_for = self.scene_key;
            self.warm_collect_at = Some(std::time::Instant::now());
            self.warm_done = 0;
            self.warm_missing = false;
        } else if finished && self.warm_missing && !stale {
            // (called again when pictures arrived: a walk that missed some goes again for them)
            self.warm_done = 0;
            self.warm_missing = false;
        }
        let mats = std::mem::take(&mut self.warm_mats);
        let start = std::time::Instant::now();
        let mut i = self.warm_done;
        while i < mats.len() {
            // (uploading textures is spread over frames)
            if start.elapsed().as_millis() > 20 {
                break;
            }
            self.warm_missing |= self.material(device, queue, assets, &mats[i]).is_none();
            i += 1;
        }
        let more = i < mats.len();
        self.warm_mats = mats;
        self.warm_done = i;
        // (a scene changed since the list was made is asked about again until it is collected)
        more || self.warm_for != self.scene_key
    }

    /// Every material the scene draws, once each.
    fn scene_materials(&self) -> Vec<Material> {
        let mut mats: Vec<Material> = Vec::new();
        mats.push(marker_material());
        for b in self.batches.iter().chain(self.ghost_batches.iter()).chain(self.signal_batches.iter()) {
            if let Some(p) = b.model.strip_prefix("tree:") {
                mats.push(Material { texture: Some(PathBuf::from(p)), color: [1.0; 4], alpha: Alpha::Test, glow: None });
            }
            if let Some(Some(m)) = self.models.get(&b.model) {
                for (_, ms) in &m.parts {
                    mats.extend(ms.iter().cloned());
                }
            }
        }
        for e in self.splines.values() {
            mats.extend(e.materials.iter().cloned());
        }
        for c in self.chunks.values().chain(self.editor_chunks.values()) {
            mats.extend(c.materials.iter().cloned());
        }
        for g in &self.ghost_spline_meshes {
            mats.extend(g.2.iter().cloned());
        }
        // (told apart by what they name rather than by a key made for each: a map of thousands
        // of splines went through a pair of strings apiece, every time this ran)
        let mut out: Vec<Material> = Vec::with_capacity(mats.len());
        let mut seen = std::collections::HashSet::with_capacity(mats.len());
        for m in &mats {
            let key = (m.texture.as_deref(), m.glow.as_ref().map(|g| g.0.as_path()), m.alpha as u8, m.color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
            if seen.insert(key) {
                out.push(m.clone());
            }
        }
        out
    }

    fn sync_ground(&mut self, gpu: &Gpu, doc: &Document, assets: &mut Assets) {
        let mut roots = vec![doc.dir.clone(), doc.source.clone().unwrap_or_default()];
        roots.extend(self.roots.iter().cloned());
        // (a season's picture first: `Texture\Winter\gras.bmp` for `Texture\gras.bmp`)
        let seasons = omsi_texture::season_folders();
        let find = |rel: &str| -> Option<PathBuf> {
            let (dir, file) = rel.rsplit_once(['\\', '/']).unwrap_or(("", rel));
            let seasonal = seasons.iter().map(|s| if dir.is_empty() { format!("{s}\\{file}") } else { format!("{dir}\\{s}\\{file}") });
            for rel in seasonal.chain(std::iter::once(rel.to_string())) {
                for r in &roots {
                    if r.as_os_str().is_empty() {
                        continue;
                    }
                    let p = omsi_cfg::resolve_path(r, &rel);
                    if omsi_cfg::vfs::is_file(&p) {
                        return Some(p);
                    }
                }
            }
            None
        };
        let layers: Vec<_> = doc.global.ground_textures.iter().take(9).collect();
        let mut key = seasons.join("|");
        let mut imgs = Vec::new();
        for g in &layers {
            let a = find(&g.texture).and_then(|p| assets.texture(&p));
            let d = find(&g.detail_texture).and_then(|p| assets.texture(&p));
            key.push_str(&format!("{}|{}|{}|{}|{:?};", g.texture, g.detail_texture, a.is_some(), d.is_some(), g.params));
            imgs.push((a, d, g.repeats(), g.detail_repeats()));
        }
        if key == self.ground_key && self.ground_bind.is_some() {
            return;
        }
        self.ground_key = key;
        let device = &gpu.device;
        let queue = &gpu.queue;
        let n = imgs.len().max(1) as u32;
        let mk_array = |size: u32, pick: &dyn Fn(usize) -> Option<omsi_texture::Image>, fallback: [u8; 4]| {
            let mips = (size as f32).log2() as u32 + 1;
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ground"),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: n },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for layer in 0..n {
                let img = pick(layer as usize).map(|i| resize(&i, size)).unwrap_or_else(|| omsi_texture::Image { width: size, height: size, rgba: fallback.iter().copied().cycle().take((size * size * 4) as usize).collect(), has_alpha: false });
                let mut level = img;
                for mip in 0..mips {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
                        &level.rgba,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * level.width), rows_per_image: Some(level.height) },
                        wgpu::Extent3d { width: level.width, height: level.height, depth_or_array_layers: 1 },
                    );
                    level = crate::assets::half(&level);
                }
            }
            tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
        };
        let tex = mk_array(512, &|i| imgs.get(i).and_then(|x| x.0.as_ref()).map(|t| t.image.clone()), [96, 120, 70, 255]);
        let det = mk_array(256, &|i| imgs.get(i).and_then(|x| x.1.as_ref()).map(|t| t.image.clone()), [255, 255, 255, 255]);
        let mut u = GroundUniform { layers: [[1.0, 1.0, 0.0, 0.0]; 9], count: [imgs.len() as f32, 0.0, 0.0, 0.0] };
        for (i, x) in imgs.iter().enumerate() {
            u.layers[i] = [x.2, x.3, 0.0, 0.0];
        }
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ground"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
        self.ground_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ground"),
            layout: &self.ground_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&tex) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&det) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.linear) },
            ],
        }));
        let _ = &self.zero_mask;
    }

    /// The sun's shadow map: the roads and objects round the camera seen from the sun.
    fn draw_shadows(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, gl: &Globals, eye: Vec3) -> u32 {
        let mut sg = *gl;
        sg.view_proj = gl.light_vp;
        // (the eye far off towards the sun: the roads' pull towards it is then a slight bias)
        let sun = Vec3::new(gl.sun_dir[0], gl.sun_dir[1], gl.sun_dir[2]);
        sg.eye = [sun.x * 1e5, sun.y * 1e5, sun.z * 1e5, gl.eye[3]];
        gpu.queue.write_buffer(&self.shadow_globals, 0, bytemuck::bytes_of(&sg));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.shadow_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let (Some(dynamic), Some(scene)) = (&self.instances, &self.scene_buf) else { return 0 };
        let f = Frustum::new(Mat4::from_cols_array_2d(&gl.light_vp) * Mat4::from_translation(-Vec4::from(gl.origin).truncate()));
        let mut calls = 0;
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(0, &self.shadow_globals_bind, &[]);
        pass.set_vertex_buffer(1, dynamic.slice(..));
        // (editor-only splines are not here: an invisible street casts no shadow)
        for c in self.chunks.values().filter(|c| !self.hide_splines && f.sees(&c.bbox)) {
            if let Some(mesh) = &c.mesh {
                calls += draw_mesh(&mut pass, &self.materials, mesh, &c.materials, Some(false), &[c.instance..c.instance + 1], &self.picture_alpha);
            }
        }
        for e in self.splines.values().filter(|e| !e.technical && !e.hidden && !e.in_chunk && f.sees(&e.bbox)) {
            if let Some(mesh) = &e.mesh {
                calls += draw_mesh(&mut pass, &self.materials, mesh, &e.materials, Some(false), &[e.instance..e.instance + 1], &self.picture_alpha);
            }
        }
        pass.set_vertex_buffer(1, scene.slice(..));
        let mut ranges = Vec::new();
        for b in &self.batches {
            if b.model == "\u{0}marker" {
                continue;
            }
            // (what casts a shadow is cut off sooner: a small one's shadow is less than it)
            b.visible(&f, eye, f32::INFINITY, self.quality.detail() * 0.5, &mut ranges);
            if ranges.is_empty() {
                continue;
            }
            if b.tree {
                let tex = b.model.strip_prefix("tree:").map(PathBuf::from);
                let m = Material { texture: tex, color: [1.0; 4], alpha: Alpha::Test, glow: None };
                calls += draw_mesh(&mut pass, &self.materials, &self.tree_mesh, std::slice::from_ref(&m), Some(false), &ranges, &self.picture_alpha);
                continue;
            }
            let Some(Some(model)) = self.models.get(&b.model).filter(|m| m.as_ref().map(|m| !m.technical).unwrap_or(false)) else { continue };
            for (mesh, mats) in &model.parts {
                calls += draw_mesh(&mut pass, &self.materials, mesh, mats, Some(false), &ranges, &self.picture_alpha);
            }
        }
        calls
    }

    /// Draw the map into the frame.
    /// `vp`: the 3D view's rectangle in pixels (x, y, width, height).
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, cam: &Camera, extras: &FrameExtras, vp: [f32; 4]) {
        let aspect = vp[2] / vp[3].max(1.0);
        let gl = self.globals_for(cam, aspect, extras, self.anchor);
        gpu.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&gl));
        let t = std::time::Instant::now();
        let origin = Vec4::from(gl.origin).truncate();
        let eye = origin + Vec4::from(gl.eye).truncate();
        let shadow_calls = if self.quality.shadow_size() > 0 { self.draw_shadows(gpu, encoder, &gl, eye) } else { 0 };
        let far = self.quality.view_distance();
        let in_reach = |b: &Aabb| b.distance(eye) <= far;
        self.profile.count("shadow draws", shadow_calls);
        let f = Frustum::new(Mat4::from_cols_array_2d(&gl.view_proj) * Mat4::from_translation(-origin));
        let mut calls = 0u32;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                // (without edge smoothing straight into the frame)
                view: if gpu.samples > 1 { &gpu.msaa } else { target },
                depth_slice: None,
                resolve_target: (gpu.samples > 1).then_some(target),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.6, g: 0.7, b: 0.8, a: 1.0 }), store: if gpu.samples > 1 { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &gpu.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(vp[0], vp[1], vp[2], vp[3], 0.0, 1.0);
        pass.set_bind_group(0, &self.globals_bind, &[]);
        // ground
        if let Some(ground) = &self.ground_bind {
            pass.set_pipeline(&self.terrain_pipeline);
            pass.set_bind_group(2, ground, &[]);
            pass.set_index_buffer(self.terrain_index.slice(..), wgpu::IndexFormat::Uint32);
            for t in self.tiles.values().filter(|t| f.sees(&t.bbox) && in_reach(&t.bbox)) {
                pass.set_bind_group(1, &t.bind, &[]);
                pass.set_vertex_buffer(0, t.vb.slice(..));
                pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
                calls += 1;
            }
            for t in self.tiles.values().filter(|t| f.sees(&t.bbox) && in_reach(&t.bbox)) {
                if let Some((vb, ib, n)) = &t.walls {
                    pass.set_bind_group(1, &t.bind, &[]);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                    calls += 1;
                }
            }
        }
        let (Some(dynamic), Some(scene)) = (&self.instances, &self.scene_buf) else { return };
        // what is in view, the same for both layers
        let mut splines: Vec<(&GpuMesh, &Vec<Material>, u32)> = self.chunks.values().filter(|c| !self.hide_splines && f.sees(&c.bbox) && in_reach(&c.bbox)).filter_map(|c| c.mesh.as_ref().map(|m| (m, &c.materials, c.instance))).collect();
        if extras.show_technical && !self.hide_splines {
            splines.extend(self.editor_chunks.values().filter(|c| f.sees(&c.bbox) && in_reach(&c.bbox)).filter_map(|c| c.mesh.as_ref().map(|m| (m, &c.materials, c.instance))));
        }
        splines.extend(self.splines.values().filter(|e| !e.hidden && !e.in_chunk && f.sees(&e.bbox) && in_reach(&e.bbox)).filter_map(|e| e.mesh.as_ref().map(|m| (m, &e.materials, e.instance))));
        let mut visible: Vec<(&Batch, Vec<std::ops::Range<u32>>, bool)> = Vec::new();
        for b in &self.batches {
            let mut ranges = Vec::new();
            b.visible(&f, eye, far, self.quality.detail(), &mut ranges);
            if !ranges.is_empty() {
                visible.push((b, ranges, false));
            }
        }
        for b in &self.ghost_batches {
            visible.push((b, vec![b.first..b.first + b.count], true));
        }
        // (the signal heads' lenses: drawn from the buffer that changes every frame)
        for b in &self.signal_batches {
            let mut ranges = Vec::new();
            b.visible(&f, eye, far, self.quality.detail(), &mut ranges);
            if !ranges.is_empty() {
                visible.push((b, ranges, true));
            }
        }
        let mut instances_drawn = 0;
        let mut spline_calls = 0;
        for (_, r, _) in &visible {
            instances_drawn += r.iter().map(|r| r.len() as u32).sum::<u32>();
        }
        for blend in [false, true] {
            // splines
            pass.set_vertex_buffer(1, dynamic.slice(..));
            let mut spline_set: Option<bool> = None;
            for &(mesh, mats, i) in &splines {
                self.set_spline_pipeline(&mut pass, mesh.one_sided, blend, &mut spline_set);
                let n = draw_mesh(&mut pass, &self.materials, mesh, mats, Some(blend), &[i..i + 1], &self.picture_alpha);
                calls += n;
                spline_calls += n;
            }
            // objects
            let mut culled: Option<bool> = None;
            let mut bound_dynamic = true;
            for (b, ranges, ghost) in &visible {
                if *ghost != bound_dynamic {
                    pass.set_vertex_buffer(1, if *ghost { dynamic.slice(..) } else { scene.slice(..) });
                    bound_dynamic = *ghost;
                }
                if b.model == "\u{0}marker" {
                    if !blend {
                        self.set_object_pipeline(&mut pass, false, blend, &mut culled);
                        let m = marker_material();
                        calls += draw_mesh(&mut pass, &self.materials, &self.marker, std::slice::from_ref(&m), None, ranges, &self.picture_alpha);
                    }
                    continue;
                }
                if b.tree {
                    // (the tree's texture is the batch: "tree:<path>")
                    if !blend {
                        self.set_object_pipeline(&mut pass, false, blend, &mut culled);
                        let tex = b.model.strip_prefix("tree:").map(PathBuf::from);
                        let m = Material { texture: tex, color: [1.0; 4], alpha: Alpha::Test, glow: None };
                        calls += draw_mesh(&mut pass, &self.materials, &self.tree_mesh, std::slice::from_ref(&m), None, ranges, &self.picture_alpha);
                    }
                    continue;
                }
                let Some(Some(model)) = self.models.get(&b.model) else { continue };
                for (mesh, mats) in &model.parts {
                    self.set_object_pipeline(&mut pass, mesh.one_sided, blend, &mut culled);
                    calls += draw_mesh(&mut pass, &self.materials, mesh, mats, Some(blend), ranges, &self.picture_alpha);
                }
            }
            if !blend {
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
            }
        }
        // the road being drawn, see-through over the ground
        if !self.ghost_spline_meshes.is_empty() {
            pass.set_pipeline(&self.ghost_pipeline);
            pass.set_vertex_buffer(1, dynamic.slice(..));
            for g in &self.ghost_spline_meshes {
                let Some(mesh) = &g.1 else { continue };
                calls += draw_mesh(&mut pass, &self.materials, mesh, &g.2, None, &[g.3..g.3 + 1], &self.picture_alpha);
            }
        }
        if let Some((b, n)) = &self.solid_buf {
            pass.set_pipeline(&self.solid_pipeline);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((b, n)) = &self.overlay_depth_buf {
            pass.set_pipeline(&self.overlay_depth_pipeline);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((b, n)) = &self.overlay_buf {
            pass.set_pipeline(&self.overlay_pipeline);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        drop(pass);
        self.profile.count("draws", calls);
        self.profile.count("instances drawn", instances_drawn);
        self.profile.count("spline draws", spline_calls);
        self.profile.add("draw", profile::ms(t));
    }

    /// The GPU's time for the view, drawn once more into a picture of its own and waited
    /// for (`OPENMAPPER_PROFILE=gpu`).
    pub fn time_gpu(&mut self, gpu: &Gpu, cam: &Camera, extras: &FrameExtras, vp: [f32; 4]) {
        let (w, h) = (gpu.config.width, gpu.config.height);
        if self.gpu_target.as_ref().map(|t| (t.width(), t.height()) != (w, h)).unwrap_or(true) {
            self.gpu_target = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gpu timing"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: gpu.scene_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }));
        }
        let view = self.gpu_target.as_ref().unwrap().create_view(&Default::default());
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let t = std::time::Instant::now();
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.draw(gpu, &mut enc, &view, cam, extras, vp);
        gpu.queue.submit([enc.finish()]);
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        self.profile.add("gpu view", profile::ms(t));
    }

    /// Render up to `budget` pictures for the catalogue (objects whose model and textures
    /// are there).
    pub fn render_thumbs(&mut self, gpu: &Gpu, egui_r: &mut egui_wgpu::Renderer, assets: &mut Assets, wanted: &[(String, bool)], budget: usize) {
        let mut done = 0;
        for (rel, is_spline) in wanted {
            if done >= budget {
                break;
            }
            let key = rel.to_lowercase();
            if self.thumbs.contains_key(&key) {
                continue;
            }
            let device = &gpu.device;
            let queue = &gpu.queue;
            // what to draw: (meshes with materials, bounds)
            let (parts, lo, hi): (Vec<(GpuMesh, Vec<Material>)>, Vec3, Vec3) = if *is_spline {
                let Some(m) = assets.spline(rel) else {
                    if matches!(assets.splines.get(&key), Some(crate::assets::Slot::Failed)) {
                        self.thumbs.insert(key, None);
                    }
                    continue;
                };
                let s = MapSpline { file: rel.clone(), length: 14.0, heading: 0.0, ..Default::default() };
                let Some(mesh) = build_spline(&m, &s, DVec2::ZERO) else {
                    self.thumbs.insert(key, None);
                    continue;
                };
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for p in &mesh.positions {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                (vec![(upload_mesh(device, &mesh), m.materials.clone())], lo, hi)
            } else {
                let Some(m) = assets.object(rel) else {
                    if assets.object_failed(rel) {
                        self.thumbs.insert(key, None);
                    }
                    continue;
                };
                if m.parts.is_empty() {
                    self.thumbs.insert(key, None);
                    continue;
                }
                (m.parts.iter().map(|p| (upload_mesh(device, &p.mesh), p.materials.clone())).collect(), m.lo, m.hi)
            };
            // every texture must be there before the picture is taken
            let mut ready = true;
            for (_, mats) in &parts {
                for mat in mats {
                    if self.material(device, queue, assets, mat).is_none() {
                        ready = false;
                    }
                }
            }
            if !ready {
                continue;
            }
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("thumb"),
                size: wgpu::Extent3d { width: THUMB, height: THUMB, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            // a camera that frames the bounds from the front right, a little from above
            let centre = (lo + hi) * 0.5;
            let radius = ((hi - lo).length() * 0.5).max(0.5);
            let mut cam = Camera::default();
            let target = centre.as_dvec3();
            // (three quarters from the front, from high enough that a flat roof still shows)
            cam.yaw = if *is_spline { 35.0 } else { 215.0 };
            cam.pitch = if *is_spline { 38.0 } else { 24.0 };
            cam.fov_y = 35f32.to_radians();
            let dist = radius as f64 / (cam.fov_y as f64 * 0.5).sin() * 1.02;
            cam.jump_to(target, dist);
            cam.update(10.0);
            // (closer until its corners, as seen, fill the picture: a long low building is
            // small inside the sphere round it)
            let corners: Vec<Vec3> = (0..8).map(|i| Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z })).collect();
            for _ in 0..4 {
                let vp = cam.proj(1.0) * cam.view();
                let o = cam.origin().as_vec3();
                let (mut mn, mut mx) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
                for c in &corners {
                    let p = vp * (*c - o).extend(1.0);
                    if p.w <= 1e-4 {
                        continue;
                    }
                    let q = glam::Vec2::new(p.x / p.w, p.y / p.w);
                    mn = mn.min(q);
                    mx = mx.max(q);
                }
                if mn.x > mx.x {
                    break;
                }
                // centre what is seen, then fill 90 % of the picture
                let mid = (mn + mx) * 0.5;
                let span = (mx - mn).max_element().max(1e-3);
                let (_, r, u) = { let f = (target - cam.eye()).normalize(); let r = f.cross(DVec3::Z).normalize_or(DVec3::X); (f, r, r.cross(f)) };
                let half_h = cam.distance * (cam.fov_y as f64 * 0.5).tan();
                let shifted = cam.target + r * (mid.x as f64 * half_h) + u * (mid.y as f64 * half_h);
                let d = cam.distance * (span as f64 / 1.8);
                cam.jump_to(shifted, d.max(0.3));
                cam.update(10.0);
            }
            let mut gl = self.globals_for(&cam, 1.0, &FrameExtras::default(), DVec3::ZERO);
            gl.fog[3] = 0.0;
            gl.shadow[0] = 0.0;
            // (a studio light: the key light from over the camera's left shoulder, the shade
            // filled well, so the side shown is lit whatever the map's sun does)
            let toward_eye = (cam.eye() - target).as_vec3().normalize_or(Vec3::Z);
            let left = Vec3::Z.cross(toward_eye).normalize_or(Vec3::X);
            let light = (toward_eye + Vec3::Z * 0.8 + left * 0.5).normalize();
            gl.sun_dir = [light.x, light.y, light.z, 1.0];
            gl.sun_color = [1.5, 1.47, 1.4, 1.0];
            gl.light_b = [0.45, 0.47, 0.52, 0.0];
            gl.light_c = [0.95, 0.95, 0.98, 0.0];
            queue.write_buffer(&self.thumb_globals, 0, bytemuck::bytes_of(&gl));
            let inst = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("thumb inst"), contents: bytemuck::bytes_of(&Instance::new(Mat4::IDENTITY, [0.0; 4])), usage: wgpu::BufferUsages::VERTEX });
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("thumb"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.thumb_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.thumb_globals_bind, &[]);
                pass.set_vertex_buffer(1, inst.slice(..));
                for blend in [false, true] {
                    pass.set_pipeline(if blend { &self.thumb_blend } else { &self.thumb_opaque });
                    for (mesh, mats) in &parts {
                        pass.set_vertex_buffer(0, mesh.vb.slice(..));
                        pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
                        for &(first, count, slot) in &mesh.ranges {
                            let Some(m) = mats.get(slot as usize) else { continue };
                            if (m.alpha == Alpha::Blend) != blend {
                                continue;
                            }
                            let Some(bg) = self.materials.get(&mat_key(m)) else { continue };
                            pass.set_bind_group(1, bg, &[]);
                            pass.draw_indexed(first..first + count, 0, 0..1);
                        }
                    }
                }
                let _ = &self.thumb_sky;
            }
            queue.submit([enc.finish()]);
            // OPENMAPPER_THUMBS=<folder>: the pictures saved too (checks without a person at the screen)
            if let Some(dir) = std::env::var_os("OPENMAPPER_THUMBS") {
                let name: String = key.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                if let Ok(img) = read_texture(gpu, &tex) {
                    let _ = img.save(std::path::Path::new(&dir).join(format!("{name}.png")));
                }
            }
            let id = egui_r.register_native_texture(device, &view, wgpu::FilterMode::Linear);
            self.thumbs.insert(key, Some(Thumb { id, _tex: tex }));
            done += 1;
        }
    }
}

/// Draw a mesh's parts whose material is ready (`blend`: only the blended ones, or only the
/// others; None: all), once for each range of instances. Returns the draw calls made.
fn draw_mesh(pass: &mut wgpu::RenderPass, materials: &HashMap<MatKey, wgpu::BindGroup>, mesh: &GpuMesh, mats: &[Material], blend: Option<bool>, ranges: &[std::ops::Range<u32>], pictures: &HashMap<PathBuf, Alpha>) -> u32 {
    let mut calls = 0;
    let mut bound = false;
    for &(first, count, slot) in &mesh.ranges {
        let Some(m) = mats.get(slot as usize) else { continue };
        // (a picture that is not blended is drawn in the pass that writes depth)
        let alpha = effective_alpha(m, pictures);
        if blend.map(|b| (alpha == Alpha::Blend) != b).unwrap_or(false) {
            continue;
        }
        let Some(bg) = materials.get(&mat_key_with(m, alpha)) else { continue };
        if !bound {
            pass.set_vertex_buffer(0, mesh.vb.slice(..));
            pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
            bound = true;
        }
        pass.set_bind_group(1, bg, &[]);
        for r in ranges {
            pass.draw_indexed(first..first + count, 0, r.clone());
            calls += 1;
        }
    }
    calls
}

/// Meshes (in the same frame) as one, its triangles grouped by material.
///
/// The grouping uses the mode each material is really drawn with (see `effective_alpha`), not
/// the one its slot says: `draw_mesh` looks a merged part up by that key, so grouping by the
/// slot's own would leave a part whose picture changed the mode out of the bind groups and it
/// would not be drawn at all.
fn merge_meshes(parts: &[(&MeshData, &[Material], Aabb)], pictures: &HashMap<PathBuf, Alpha>) -> (MeshData, Vec<Material>, Aabb) {
    // (one-sided only when every part is: they are drawn with one pipeline, and drawing a
    // two-sided part from one side would leave it half missing. A tile's chunk merges the
    // splines drawn near it, and every spline the geometry crate builds is one-sided - which
    // this lost, begun from `MeshData::default()`, so the merged splines were the ones still
    // drawn from both sides)
    let mut out = MeshData { one_sided: !parts.is_empty() && parts.iter().all(|(m, _, _)| m.one_sided), ..Default::default() };
    let mut materials: Vec<Material> = Vec::new();
    let mut keys: Vec<MatKey> = Vec::new();
    let mut by_mat: Vec<Vec<u32>> = Vec::new();
    let mut bbox = Aabb::EMPTY;
    for (m, mats, b) in parts {
        bbox.lo = bbox.lo.min(b.lo);
        bbox.hi = bbox.hi.max(b.hi);
        let base = out.positions.len() as u32;
        out.positions.extend_from_slice(&m.positions);
        out.normals.extend((0..m.positions.len()).map(|i| m.normals.get(i).copied().unwrap_or(Vec3::Z)));
        out.uvs.extend((0..m.positions.len()).map(|i| m.uvs.get(i).copied().unwrap_or(glam::Vec2::ZERO)));
        for &(first, count, slot) in &m.ranges {
            let Some(mat) = mats.get(slot as usize) else { continue };
            let key = mat_key_with(mat, effective_alpha(mat, pictures));
            let i = match keys.iter().position(|k| *k == key) {
                Some(i) => i,
                None => {
                    keys.push(key);
                    materials.push(mat.clone());
                    by_mat.push(Vec::new());
                    keys.len() - 1
                }
            };
            let Some(tris) = m.indices.get(first as usize..(first + count) as usize) else { continue };
            by_mat[i].extend(tris.iter().map(|v| v + base));
        }
    }
    for (slot, tris) in by_mat.into_iter().enumerate() {
        out.ranges.push((out.indices.len() as u32, tris.len() as u32, slot as u32));
        out.indices.extend(tris);
    }
    (out, materials, bbox)
}

fn mat_key(m: &Material) -> MatKey {
    mat_key_with(m, m.alpha)
}

/// The key of a material drawn with this alpha mode (the mode is part of it).
fn mat_key_with(m: &Material, alpha: Alpha) -> MatKey {
    MatKey {
        texture: m.texture.clone(),
        glow: m.glow.clone(),
        alpha: match alpha {
            Alpha::Opaque => 0,
            Alpha::Test => 1,
            Alpha::Blend => 2,
        },
        color: m.color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8),
    }
}

/// How a material is really drawn: a `[matl_alpha] 2` slot that has nothing to fade by is drawn
/// as an opaque one, so that it writes depth.
///
/// Omsi.exe's blend writes depth there — its ZWRITEENABLE is cleared by `[matl_noZwrite]` alone
/// — which is why a sign panel or a road marking stays whole there, while a slot drawn as a
/// no-write blend here was eaten in bands by the surfaces drawn after it (the road, the terrain,
/// the markings), coming back as the camera moved. That is what a panel shows as the wide
/// pale band in a gantry's face.
///
/// A slot has nothing to fade by when its picture has no transparency at all — no alpha channel,
/// or 255 throughout, as many sign and marking textures are — and then blending it draws exactly
/// what drawing it opaque draws, only without the depth. A picture with a real fade stays blended
/// (glass, a soft shadow, a feathered border). So does a slot with a tint of its own
/// (`[matl] … alpha`), which is how a glass panel gets its look from a picture that is opaque.
///
/// A cut-out picture (every texel clear or solid, the antialiased outline between) is drawn cut
/// out instead: the same texels are discarded as blending discarded, and the solid part writes
/// depth.
fn effective_alpha(m: &Material, pictures: &HashMap<PathBuf, Alpha>) -> Alpha {
    if m.alpha != Alpha::Blend {
        // (a slot saying opaque or cut out is drawn as it says, as it always was)
        return m.alpha;
    }
    // (the blend is the only thing that fades a slot by its colour's alpha: leave it be)
    if m.color[3] < 0.98 {
        return Alpha::Blend;
    }
    let Some(p) = m.texture.as_ref() else {
        // (a colour with no picture: the water, which fades by its own alpha)
        return Alpha::Blend;
    };
    match pictures.get(p) {
        // (a picture that is missing from the content: bare, as Omsi.exe leaves it)
        None => Alpha::Opaque,
        Some(Alpha::Opaque) => Alpha::Opaque,
        Some(Alpha::Test) => Alpha::Test,
        Some(Alpha::Blend) => Alpha::Blend,
    }
}

fn marker_material() -> Material {
    Material { texture: None, color: [0.95, 0.45, 0.2, 1.0], alpha: Alpha::Opaque, glow: None }
}

/// A small upright diamond for objects that have nothing to draw.
fn marker_mesh() -> MeshData {
    let mut m = MeshData::default();
    let top = Vec3::new(0.0, 0.0, 2.0);
    let bottom = Vec3::new(0.0, 0.0, 0.0);
    let mid = [Vec3::new(0.6, 0.0, 1.0), Vec3::new(0.0, 0.6, 1.0), Vec3::new(-0.6, 0.0, 1.0), Vec3::new(0.0, -0.6, 1.0)];
    for i in 0..4 {
        let a = mid[i];
        let b = mid[(i + 1) % 4];
        for tip in [top, bottom] {
            let base = m.positions.len() as u32;
            let n = (a - tip).cross(b - tip).normalize();
            m.positions.extend([tip, a, b]);
            m.normals.extend([n; 3]);
            m.uvs.extend([glam::Vec2::ZERO; 3]);
            m.indices.extend([base, base + 1, base + 2]);
        }
    }
    m.ranges.push((0, m.indices.len() as u32, 0));
    m
}

/// The object is in a layer left out of the view (trees, or the other objects; what the
/// game does not show goes by its own switch).
pub fn hidden_layer(model: &ObjectModel, extras: &FrameExtras) -> bool {
    !model.technical && if model.tree.is_some() { extras.hide[0] } else { extras.hide[1] }
}

/// The spline is left out of the view: `[onlyeditor]` ones - invisible streets, AI paths,
/// flight paths - are what Omsi.exe's editor draws and the game does not, so they go by the
/// same switch as the rest of "what the game does not show". They are no different from a
/// road once that switch is on: `invis_street.sli` has a profile (a 3 m strip of
/// `helper_path_cars_1.bmp`), and exactly that strip is what the editor shows.
pub fn spline_hidden(technical: bool, extras: &FrameExtras) -> bool {
    technical && !extras.show_technical || extras.hide[2]
}

/// Which merged mesh a spline belongs in.
/// `None`: drawn on its own (nothing built yet, or picked, so it can be tinted).
/// `Some(false)`: the tile's roads, which cast a shadow.
/// `Some(true)`: the tile's editor-only splines. One mesh for the tile, drawn only while
/// "what the game does not show" is on, and never into the shadow map.
pub fn spline_batch(technical: bool, picked: bool, has_mesh: bool) -> Option<bool> {
    if !has_mesh || picked { None } else { Some(technical) }
}

/// (`editor`: the meshes seen in the editor only are drawn too)
fn push_object(groups: &mut Groups, key: &str, model: &Arc<ObjectModel>, label: Option<&String>, m: Mat4, tint: [f32; 4], sel: Option<Sel>, editor: bool) {
    if !m.is_finite() {
        return;
    }
    let scale = m.x_axis.truncate().length().max(m.y_axis.truncate().length()).max(m.z_axis.truncate().length());
    if let Some((dirs, tex, h, r)) = &model.tree {
        // a tree's texture may be named by its first label
        let name = label.filter(|l| !l.trim().is_empty()).unwrap_or(tex);
        let refs: Vec<&std::path::Path> = dirs.iter().map(PathBuf::as_path).collect();
        if let Some(p) = omsi_texture::find_texture(name, &refs) {
            let card = Mat4::from_scale(Vec3::new(h * r, h * r, *h));
            let radius = h * r.max(1.0) * scale;
            groups.entry(format!("tree:{}", p.display())).or_insert_with(|| (true, Vec::new())).1.push(Pending { inst: Instance::new(m * card, tint), radius, sel });
        }
    }
    if model.editor_parts && !editor && model.tree.is_some() {
        return;
    }
    if model.parts.is_empty() {
        if model.tree.is_none() {
            groups.entry("\u{0}marker".into()).or_default().1.push(Pending { inst: Instance::new(m, tint), radius: 2.0 * scale, sel });
        }
        return;
    }
    let inst = Instance::new(m, tint);
    let radius = (model.centre().length() + model.radius()) * scale;
    groups.entry(key.to_string()).or_default().1.push(Pending { inst: if model.surface { inst.surface(SURF_OBJECT) } else { inst }, radius, sel });
}

/// Batches (one per model, in a fixed order) from the instances gathered, numbered from
/// `base`; each batch's instances sorted by tile into runs. Also where each selectable
/// object's instances went.
fn flatten(groups: Groups, base: u32) -> (Vec<Instance>, Vec<Batch>, HashMap<Sel, Vec<u32>>) {
    let mut instances = Vec::new();
    let mut batches = Vec::new();
    let mut by_sel: HashMap<Sel, Vec<u32>> = HashMap::new();
    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let ts = tile_size() as f32;
    let cell = |p: &Pending| ((p.inst.m[3][0] / ts).floor() as i32, (p.inst.m[3][1] / ts).floor() as i32);
    for (model, (tree, mut list)) in sorted {
        list.sort_by_key(cell);
        let first = base + instances.len() as u32;
        let mut runs: Vec<Run> = Vec::new();
        let mut last = None;
        for p in list {
            let i = base + instances.len() as u32;
            let c = cell(&p);
            if last != Some(c) {
                runs.push(Run { first: i, count: 0, bounds: Aabb::EMPTY, radius: 0.0 });
                last = Some(c);
            }
            let r = runs.last_mut().unwrap();
            r.count += 1;
            r.radius = r.radius.max(p.radius);
            r.bounds.add_sphere(Vec3::new(p.inst.m[3][0], p.inst.m[3][1], p.inst.m[3][2]), p.radius);
            if let Some(s) = p.sel {
                by_sel.entry(s).or_default().push(i);
            }
            instances.push(p.inst);
        }
        let count = base + instances.len() as u32 - first;
        batches.push(Batch { model, tree, first, count, runs });
    }
    (instances, batches, by_sel)
}

/// A spline's mesh, positions relative to `origin` (a tile's corner).
pub fn build_spline(model: &SplineModel, s: &MapSpline, origin: DVec2) -> Option<MeshData> {
    // (a broken record - no length, one of thousands of kilometres, a number that is no
    // number - is left out rather than built)
    let sane = |v: f64| v.is_finite() && v.abs() < 1e7;
    if !(s.length > 0.0 && s.length < 20_000.0) || !s.pos.iter().chain([&s.heading, &s.radius, &s.grad_start, &s.grad_end]).all(|v| sane(*v)) {
        return None;
    }
    let curve = SplineCurve::from_map(s, origin);
    let m = omsi_geometry::build_spline_mesh(&model.def, &curve, s.mirror, DVec3::new(origin.x, origin.y, 0.0));
    (!m.is_empty()).then_some(m)
}

/// Shrink or stretch a picture to `size` × `size`.
fn resize(img: &omsi_texture::Image, size: u32) -> omsi_texture::Image {
    let Some(buf) = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone()) else {
        return img.clone();
    };
    let r = image::imageops::resize(&buf, size, size, image::imageops::FilterType::Triangle);
    omsi_texture::Image { width: size, height: size, rgba: r.into_raw(), has_alpha: img.has_alpha }
}

/// The vertices of a tile's ground (tile-local positions; normals across the tile edges).
fn terrain_vertices(doc: &Document, k: TileKey) -> Vec<TerrainVertex> {
    let t = &doc.tiles[&k];
    let n = TERRAIN_SAMPLES;
    let cell = tile_size() as f32 / (n - 1) as f32;
    let h = |i: i32, j: i32| -> f32 {
        if i >= 0 && j >= 0 && (i as usize) < n && (j as usize) < n {
            return t.terrain.height_at(i as usize, j as usize);
        }
        // across the edge: the neighbour's sample, else the nearest own one
        let (dx, ix) = if i < 0 { (-1, i + n as i32 - 1) } else if i as usize >= n { (1, i - n as i32 + 1) } else { (0, i) };
        let (dy, jy) = if j < 0 { (-1, j + n as i32 - 1) } else if j as usize >= n { (1, j - n as i32 + 1) } else { (0, j) };
        if let Some(nb) = doc.tiles.get(&(k.0 + dx, k.1 + dy)) {
            return nb.terrain.height_at(ix.clamp(0, n as i32 - 1) as usize, jy.clamp(0, n as i32 - 1) as usize);
        }
        t.terrain.height_at(i.clamp(0, n as i32 - 1) as usize, j.clamp(0, n as i32 - 1) as usize)
    };
    let mut out = Vec::with_capacity(n * n);
    for j in 0..n as i32 {
        for i in 0..n as i32 {
            let z = h(i, j);
            let dzdx = (h(i + 1, j) - h(i - 1, j)) / (2.0 * cell);
            let dzdy = (h(i, j + 1) - h(i, j - 1)) / (2.0 * cell);
            let nrm = Vec3::new(-dzdx, -dzdy, 1.0).normalize();
            out.push(TerrainVertex { pos: [i as f32 * cell, j as f32 * cell, z], normal: nrm.to_array() });
        }
    }
    out
}

#[allow(dead_code)]
fn v4(v: Vec3, w: f32) -> Vec4 {
    v.extend(w)
}

/// Towards the sun: high in the south-west of a summer afternoon.
/// An RGBA8 texture read back from the GPU.
fn read_texture(gpu: &Gpu, tex: &wgpu::Texture) -> anyhow::Result<image::RgbaImage> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range();
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        let r = &data[(y * row) as usize..][..(w * 4) as usize];
        for x in 0..w {
            let p = &r[(x * 4) as usize..][..4];
            img.put_pixel(x, y, image::Rgba([p[0], p[1], p[2], p[3]]));
        }
    }
    Ok(img)
}

/// The sun's view of the scene round where the camera looks (relative to the render origin),
/// and the size of a shadow map texel there (m). The window follows the camera in whole
/// texels, so the shadows' edges do not crawl as it moves.
fn light_view(cam: &Camera, sun: Vec3, size: u32, reach: f32) -> (Mat4, f32) {
    let centre = (cam.smooth_target_now() - cam.origin()).as_vec3();
    let half = (cam.smooth_distance_now() as f32 * 1.4).clamp(40.0, reach);
    let up = if sun.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    let view = Mat4::look_at_rh(sun, Vec3::ZERO, up);
    let c = view.transform_point3(centre);
    let texel = 2.0 * half / size as f32;
    let (x, y) = ((c.x / texel).round() * texel, (c.y / texel).round() * texel);
    // (deep enough for anything standing up to a few hundred metres above the ground)
    let depth = half * 2.0 + 600.0;
    let proj = Mat4::orthographic_rh(x - half, x + half, y - half, y + half, -c.z - depth, -c.z + depth);
    (proj * view, texel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_material(alpha: Alpha, texture: Option<&str>) -> Material {
        Material { texture: texture.map(PathBuf::from), color: [1.0; 4], alpha, glow: None }
    }

    #[test]
    fn a_tinted_slot_keeps_blending() {
        let mut pictures = HashMap::new();
        pictures.insert(PathBuf::from("texture/white.bmp"), Alpha::Opaque);
        // (a glass panel gets its look from a tint and a picture that is opaque: the blend has
        // to stay, or it would be drawn as a white board)
        let tinted = Material { color: [1.0, 1.0, 1.0, 0.4], ..a_material(Alpha::Blend, Some("texture/white.bmp")) };
        assert_eq!(effective_alpha(&tinted, &pictures), Alpha::Blend);
        // (and the water: a colour of its own with no picture at all)
        let water = Material { texture: None, color: [0.22, 0.38, 0.45, 0.72], alpha: Alpha::Blend, glow: None };
        assert_eq!(effective_alpha(&water, &pictures), Alpha::Blend);
    }

    #[test]
    fn a_blended_slot_with_nothing_to_fade_by_is_drawn_opaque() {
        let mut pictures = HashMap::new();
        pictures.insert(PathBuf::from("texture/arrow.png"), Alpha::Test);
        pictures.insert(PathBuf::from("texture/sign.bmp"), Alpha::Opaque);
        pictures.insert(PathBuf::from("texture/glass.tga"), Alpha::Blend);
        // (a sign panel or marking whose picture is 255 throughout: drawn opaque, so it writes
        // depth and the surfaces drawn after it cannot eat it in bands)
        assert_eq!(effective_alpha(&a_material(Alpha::Blend, Some("texture/sign.bmp")), &pictures), Alpha::Opaque);
        // (a picture missing from the content: bare, as Omsi.exe leaves it, and never faded)
        assert_eq!(effective_alpha(&a_material(Alpha::Blend, Some("texture/missing.bmp")), &pictures), Alpha::Opaque);
        // (a hard-edged picture: cut out, so its solid part writes depth and the rest is discarded)
        assert_eq!(effective_alpha(&a_material(Alpha::Blend, Some("texture/arrow.png")), &pictures), Alpha::Test);
        // (a picture with a real fade stays blended: glass, a soft shadow, a feathered border)
        assert_eq!(effective_alpha(&a_material(Alpha::Blend, Some("texture/glass.tga")), &pictures), Alpha::Blend);
        // (a slot that already says opaque or cut out is left as it says)
        assert_eq!(effective_alpha(&a_material(Alpha::Opaque, Some("texture/arrow.png")), &pictures), Alpha::Opaque);
        assert_eq!(effective_alpha(&a_material(Alpha::Test, Some("texture/glass.tga")), &pictures), Alpha::Test);
    }

    #[test]
    fn a_merged_chunk_groups_its_parts_by_the_mode_they_are_drawn_with() {
        // (draw_mesh looks a merged part up by the mode it is really drawn with. Grouped by
        // the slot's own mode instead, a part whose picture changes the mode would be left
        // out of the bind groups and not drawn at all.)
        let mut pictures = HashMap::new();
        pictures.insert(PathBuf::from("texture/sign.bmp"), Alpha::Opaque);
        let mat = Material { texture: Some(PathBuf::from("texture/sign.bmp")), color: [1.0; 4], alpha: Alpha::Blend, glow: None };
        let mesh = MeshData { ranges: vec![(0, 3, 0)], indices: vec![0, 1, 2], one_sided: true, ..Default::default() };
        let mats = [mat];
        let aabb = Aabb::EMPTY;
        let (merged, materials, _) = merge_meshes(&[(&mesh, &mats, aabb)], &pictures);
        assert_eq!(materials.len(), 1);
        // the slot the draw loop will look for is the effective one
        let want = mat_key_with(&materials[0], effective_alpha(&materials[0], &pictures));
        assert_eq!(want.alpha, 0, "a picture with no transparency is drawn opaque");
        assert_eq!(merged.ranges[0].2, 0, "the merged part is in the first slot");
    }

    #[test]
    fn the_key_of_a_material_follows_the_mode_it_is_drawn_with() {
        let m = a_material(Alpha::Blend, Some("texture/bus.bmp"));
        assert_eq!(mat_key_with(&m, Alpha::Blend).alpha, 2);
        assert_eq!(mat_key_with(&m, Alpha::Test).alpha, 1);
        assert_eq!(mat_key_with(&m, Alpha::Opaque).alpha, 0);
        // (the rest of the key is the same: the texture, the glow and the colour)
        let k = mat_key(&m);
        assert_eq!(k.texture, mat_key_with(&m, Alpha::Test).texture);
        assert_eq!(k.color, mat_key_with(&m, Alpha::Test).color);
    }

    #[test]
    fn a_merged_chunk_keeps_the_one_sidedness_of_its_parts() {
        // (a tile's chunk merges the splines drawn near it into one draw, and the merged mesh
        // is what the draw loop sees: it has to carry the flag its parts had. Begun from
        // `MeshData::default()` it was two-sided, so every merged spline - which is most of
        // them - was still drawn from both sides.)
        let one = MeshData { one_sided: true, ..Default::default() };
        let mut two = one.clone();
        two.one_sided = false;
        let aabb = Aabb::EMPTY;
        let mats: [Material; 0] = [];
        let (merged, _, _) = merge_meshes(&[(&one, &mats, aabb), (&one, &mats, aabb)], &HashMap::new());
        assert!(merged.one_sided, "merged one-sided parts came out two-sided");
        // (one two-sided part makes the whole draw two-sided: they share one pipeline)
        let (merged, _, _) = merge_meshes(&[(&one, &mats, aabb), (&two, &mats, aabb)], &HashMap::new());
        assert!(!merged.one_sided, "a two-sided part was merged into a one-sided draw");
        // (nothing to merge is nothing to cull)
        let (merged, _, _) = merge_meshes(&[], &HashMap::new());
        assert!(!merged.one_sided);
    }

    #[test]
    fn a_one_sided_mesh_is_drawn_with_the_pipeline_that_culls_the_back() {
        // (the four pipelines a setter picks from are, in order: two-sided, two-sided blended,
        // one-sided, one-sided blended, so a one-sided mesh takes the last two)
        for blend in [false, true] {
            let two = pipeline_index(false, blend);
            let one = pipeline_index(true, blend);
            assert!(one > two, "a one-sided mesh takes a later pipeline than a two-sided one");
            assert_eq!(one % 2 == 1, blend, "the blended pipeline is the odd one");
        }
        assert_eq!([pipeline_index(false, false), pipeline_index(false, true), pipeline_index(true, false), pipeline_index(true, true)], [0, 1, 2, 3]);
    }

    #[test]
    fn an_editor_only_spline_is_seen_with_the_switch_on() {
        // (`invis_street.sli` is `[onlyeditor]` with a 3 m profile and AI paths: `technical`.
        // Its strip is drawn by Omsi.exe's editor and never by the game, so it has to go by
        // the "what the game does not show" switch - and be seen at all with it on.)
        let mut extras = FrameExtras::default();
        assert!(spline_hidden(true, &extras), "an invisible street is out of the view by default");
        extras.show_technical = true;
        assert!(!spline_hidden(true, &extras), "and is drawn with the editor's switch on");
        // (the roads' own layer still takes it away, as it takes every spline)
        extras.hide[2] = true;
        assert!(spline_hidden(true, &extras));
        assert!(spline_hidden(false, &extras), "the roads' layer hides ordinary roads too");
        // (an ordinary road is never held back by the editor switch)
        assert!(!spline_hidden(false, &FrameExtras::default()));
    }

    #[test]
    fn an_invisible_street_shares_its_tile_and_a_road_casts_the_shadow() {
        // (a road goes in the tile's shadowed mesh)
        assert_eq!(spline_batch(false, false, true), Some(false));
        // (`invis_street.sli`: one mesh with the tile's other editor-only splines, and that
        // mesh is not the one the shadow pass draws — `Some(true)` is the editor batch)
        assert_eq!(spline_batch(true, false, true), Some(true));
        assert!(spline_batch(true, false, true).unwrap());
        // (picked: drawn on its own, so the tint can be its alone)
        assert_eq!(spline_batch(true, true, true), None);
        assert_eq!(spline_batch(false, true, true), None);
        // (no mesh yet: nothing to merge)
        assert_eq!(spline_batch(true, false, false), None);
    }
}
