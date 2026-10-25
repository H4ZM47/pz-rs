//! GPU backend: executes [`pz_render::DrawList`]s the way the original's
//! render thread executes `SpriteRenderer`'s ring buffer, on wgpu (Metal,
//! Vulkan, DX12, GL).
//!
//! Each frame is a list of segments, one per `Core.DoStartFrameStuff`:
//! viewport `0,0,w,h`, projection `ortho2D(0, w*zoom, h*zoom, 0)`. Quads are
//! drawn with a port of the game's `default` shader; GL state commands in
//! the list (blend function, colour mask, stencil, alpha test, clears,
//! scissor) update a GL-like state machine that persists across frames, as
//! `GLState` does. Quads are split into triangles as `SpriteRenderer`'s
//! ring buffer splits them (on the diagonal with matching corner colours),
//! so colour interpolation matches.
//!
//! Textures are uploaded on first use from [`TextureManager`] with the
//! original filter, wrap and mip settings.
//!
//! The world is drawn as the build-42 renderer draws it: tiles are recorded
//! into per-chunk colour + 16-bit depth textures with the depth-texture
//! shaders (`DrawCmd::ChunkBegin`..`ChunkEnd`), then composited into the
//! main target with `chunkShader`, the depth buffer resolving overlap.
//! Texture targets are drawn with clip-space y flipped, so texture
//! coordinates address them the way GL's bottom-up framebuffers are
//! addressed.

mod atlas;
mod combiner;
mod gl;
mod model;
mod outline;
mod vehicle;
mod state;
mod thumbnail;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use pz_render::{DrawCmd, DrawList, TextureManager, WorldShader};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub use state::GlState;
pub use thumbnail::THUMBNAIL_SIZE;
use state::{PipeKey, Program, TargetKind};

/// One `Core.DoStartFrameStuff` call and what was drawn after it.
#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// Viewport and projection size in pixels (the screen size).
    pub width: u32,
    pub height: u32,
    /// Projection zoom (`ortho2D(0, w * zoom, h * zoom, 0)`); 1 for UI.
    pub zoom: f32,
    /// `glClear` of the colour buffer before this segment, if any.
    pub clear: Option<[f64; 4]>,
}

impl FrameParams {
    pub fn ui(width: u32, height: u32) -> Self {
        FrameParams { width, height, zoom: 1.0, clear: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GpuError {
    NoAdapter(String),
    Device(String),
    Surface(String),
    /// The device ran out of memory (a texture allocation, or reported by
    /// the driver later): what GL reports as `GL_OUT_OF_MEMORY`, after
    /// which the original's `TextureFBO` finds its framebuffer incomplete
    /// and throws.
    OutOfMemory(String),
    /// A texture the device cannot create (larger than its limits).
    Allocation(String),
    /// The device was lost; nothing more can be drawn or read with it.
    DeviceLost(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter(e) => write!(f, "no graphics adapter: {e}"),
            GpuError::Device(e) => write!(f, "device: {e}"),
            GpuError::Surface(e) => write!(f, "surface: {e}"),
            GpuError::OutOfMemory(e) => write!(f, "out of GPU memory: {e}"),
            GpuError::Allocation(e) => write!(f, "texture allocation failed: {e}"),
            GpuError::DeviceLost(e) => write!(f, "GPU device lost: {e}"),
        }
    }
}

/// What the device reported outside the calls that check for errors: its
/// loss (`set_device_lost_callback`) and out-of-memory or internal errors
/// (`on_uncaptured_error`). The first one sticks.
#[derive(Clone, Default)]
struct Faults {
    first: Arc<Mutex<Option<GpuError>>>,
    /// Set while the `Gpu` is dropped, when losing the device is expected.
    closing: Arc<std::sync::atomic::AtomicBool>,
}

impl Faults {
    fn watch(device: &wgpu::Device) -> Self {
        let faults = Faults::default();
        let lost = faults.clone();
        device.set_device_lost_callback(move |reason, msg| {
            if !lost.closing.load(std::sync::atomic::Ordering::Relaxed) {
                lost.record(GpuError::DeviceLost(format!("{reason:?} {msg}").trim_end().to_string()));
            }
        });
        let errors = faults.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| match e {
            wgpu::Error::OutOfMemory { source } => errors.record(GpuError::OutOfMemory(source.to_string())),
            wgpu::Error::Internal { description, .. } => errors.record(GpuError::Device(description)),
            // A validation error is a bug in the draw code: fail loudly, as
            // wgpu's default handler does.
            e => panic!("wgpu: {e}"),
        }));
        faults
    }

    fn record(&self, e: GpuError) {
        let mut f = self.first.lock().unwrap_or_else(|p| p.into_inner());
        if f.is_none() {
            eprintln!("pz-gpu: {e}");
            *f = Some(e);
        }
    }

    fn get(&self) -> Option<GpuError> {
        self.first.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl std::error::Error for GpuError {}

impl Drop for Gpu {
    fn drop(&mut self) {
        self.faults.closing.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Vertex as `SpriteRenderer` packs it (position, texture 0 coordinates,
/// ABGR colour, texture 1 and 2 coordinates), plus the per-quad depth
/// values the world shaders take as uniforms in the original.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
    pub uv1: [f32; 2],
    pub uv2: [f32; 2],
    pub depth: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    mvp: [[f32; 4]; 4],
    alpha_func: u32,
    alpha_ref: f32,
    clip_z: f32,
    /// A per-program uniform (CircleStencil's `discardValue`).
    extra: f32,
    /// The screen pass's uniforms (see screen.wgsl).
    screen: [[f32; 4]; 2],
    /// The water and puddles uniforms (see surface.wgsl).
    surface: [[f32; 4]; 5],
}

/// `sky.wgsl`'s uniform block.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyUniforms {
    a: [f32; 4],
    sun_light: [f32; 4],
    sun_colour: [f32; 4],
    sky_h: [f32; 4],
    sky_l: [f32; 4],
    b: [f32; 4],
    wind: [f32; 4],
}

/// Uniform blocks are bound at 256-byte aligned offsets.
const GLOBALS_STRIDE: u64 = 256;

const IDENTITY: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

enum Target {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Headless { texture: wgpu::Texture },
}

/// The second texture a batch binds (group 2).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tex2 {
    None,
    /// A depth map, for the depth-texture shaders.
    Texture(u32),
    /// A chunk texture: colour in group 1 and depth in group 2.
    Chunk(u64, bool),
    /// The offscreen buffer's colour, in group 1.
    Offscreen,
    /// The batch texture sampled GL_NEAREST.
    Nearest,
    /// A surface draw's first reflection texture, white when missing.
    Surface(Option<u32>),
}

/// Where a pass draws.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PassTarget {
    Main,
    Chunk(u64),
    Offscreen,
    /// A sky render (`RenderSkyBox`) into texture `id` with uniform block
    /// `index` of the frame's sky uniforms.
    Sky(u32, usize),
    /// One of `WeatherFxMask`'s render textures.
    WeatherFx(usize),
    /// An atlas texture (stored with the chunk textures; scissor honoured).
    Atlas(u64),
    /// One of `ModelOutlines`' textures ([`outline::MASK`],
    /// [`outline::THICK_H`], [`outline::THICK_V`]); only outline draws go
    /// there.
    Outline(u8),
}

impl PassTarget {
    fn kind(self) -> TargetKind {
        match self {
            PassTarget::Main => TargetKind::Main,
            PassTarget::Chunk(_) | PassTarget::Atlas(_) => TargetKind::Chunk,
            PassTarget::Offscreen | PassTarget::Sky(..) | PassTarget::WeatherFx(_) | PassTarget::Outline(_) => TargetKind::Offscreen,
        }
    }
}

/// `MultiTextureFBO2`'s offscreen world buffer.
struct OffscreenTarget {
    width: u32,
    height: u32,
    color: wgpu::TextureView,
    depth_stencil: wgpu::TextureView,
    /// Sampled with `Core.screenFilter` (GL_NEAREST).
    bind: wgpu::BindGroup,
}

struct PassDesc {
    target: PassTarget,
    /// Chunk passes: generate the mip chain afterwards (`endRenderThread`).
    mipmaps: bool,
    /// Chunk passes: clear colour to 0 and depth to 1 first.
    clear: bool,
    batches: std::ops::Range<usize>,
}

/// A run of indices drawn with one state.
struct Batch {
    key: PipeKey,
    texture: Option<u32>,
    tex2: Tex2,
    /// The seam mask (group 3).
    tex3: Option<u32>,
    /// A model draw: its pipeline, mesh and uniform block.
    model: Option<(model::ModelKey, u64, u32)>,
    /// A vehicle draw: its pipeline, mesh, uniform block and textures.
    vehicle: Option<(vehicle::VehicleKey, u64, u32, [u32; 10])>,
    /// An outline draw (`ModelOutlines`).
    outline: Option<outline::Op>,
    stencil_ref: u32,
    scissor: Option<[u32; 4]>,
    globals: u32,
    viewport: (u32, u32),
    /// `glViewport`'s origin (GL window coordinates).
    origin: (u32, u32),
    indices: std::ops::Range<u32>,
}

/// A chunk's colour and depth textures (`FBORenderChunk.tex`/`depth`).
struct ChunkTarget {
    width: u32,
    height: u32,
    /// The colour texture with its mip chain, and views of each level.
    color_texture: wgpu::Texture,
    levels: Vec<wgpu::TextureView>,
    /// Level 0, rendered into.
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// Colour bind groups for compositing, with nearest and linear
    /// magnification.
    composite: [wgpu::BindGroup; 2],
    depth_bind: wgpu::BindGroup,
}

/// The texture id the vehicle bind groups use for an unset texture unit.
const NO_TEXTURE: u32 = u32::MAX;

/// Sampler choice: min nearest, mag nearest, mip-mapped, clamp to edge.
type Filter = (bool, bool, bool, bool);

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: Target,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    depth_stencil: wgpu::TextureView,
    programs: state::Programs,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    chunk_depth_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<PipeKey, wgpu::RenderPipeline>,
    chunks: HashMap<u64, ChunkTarget>,
    models: model::ModelPipelines,
    vehicles: vehicle::VehiclePipelines,
    /// `ModelOutlines`' textures and shaders.
    outlines: outline::OutlinePipelines,
    /// Each uploaded texture's sampler state.
    filters: HashMap<u32, Filter>,
    /// glGenerateMipmap's downsampling pipeline and its source layout.
    mip_pipeline: wgpu::RenderPipeline,
    mip_layout: wgpu::BindGroupLayout,
    mip_sampler: wgpu::Sampler,
    /// The sky shaders' pipeline, its uniform layout and the two sky
    /// render textures' views by id.
    sky_pipeline: wgpu::RenderPipeline,
    sky_layout: wgpu::BindGroupLayout,
    sky_targets: HashMap<u32, wgpu::TextureView>,
    /// Texture views by id, and bind groups sampling them GL_NEAREST.
    views: HashMap<u32, wgpu::TextureView>,
    nearest: HashMap<u32, wgpu::BindGroup>,
    offscreen: Option<OffscreenTarget>,
    /// `WeatherFxMask.fboMask` and `fboParticles`.
    weather: [Option<OffscreenTarget>; 2],
    textures: HashMap<u32, wgpu::BindGroup>,
    samplers: HashMap<Filter, wgpu::Sampler>,
    /// Bound for untextured quads; samples as 1.0 like `useTexture = 0`.
    white: Option<wgpu::BindGroup>,
    /// GL state carried across frames, like `GLState`.
    pub state: GlState,
    /// `glPushAttrib`'s saved states.
    state_stack: Vec<GlState>,
    /// `TextureCombiner`.
    combiner: combiner::CombinerState,
    faults: Faults,
}

impl Gpu {
    /// Renders into `window`'s surface.
    pub fn new<W>(window: Arc<W>, width: u32, height: u32) -> Result<Self, GpuError>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static,
    {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone())));
        let surface = instance.create_surface(window).map_err(|e| GpuError::Surface(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let caps = surface.get_capabilities(&adapter);
        // The original renders to a plain (non-sRGB) RGBA8 back buffer.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        let (device, queue) = request_device(&adapter)?;
        surface.configure(&device, &config);
        Ok(Self::build(device, queue, Target::Surface { surface, config }, format, width.max(1), height.max(1)))
    }

    /// Renders into an offscreen RGBA8 texture, read with
    /// [`Gpu::read_pixels`].
    pub fn new_headless(width: u32, height: u32) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let (device, queue) = request_device(&adapter)?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (width, height) = (width.max(1), height.max(1));
        let texture = headless_texture(&device, format, width, height);
        Ok(Self::build(device, queue, Target::Headless { texture }, format, width, height))
    }

    fn build(device: wgpu::Device, queue: wgpu::Queue, target: Target, format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let faults = Faults::watch(&device);
        let module = |label: &str, src: String| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let sprite = module("default", include_str!("shader.wgsl").to_string());
        let world = include_str!("world.wgsl");
        let world_tile = module("tileWithDepth", format!("{world}\n{}", include_str!("world_tile.wgsl")));
        let world_composite = module("chunkShader", format!("{world}\n{}\n{}", include_str!("world_composite.wgsl"), include_str!("world_atlas.wgsl")));
        let screen = module("screen", include_str!("screen.wgsl").to_string());
        let surface = module("surface", include_str!("surface.wgsl").to_string());
        let particles = module("particles", include_str!("particles.wgsl").to_string());
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Globals>() as u64),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("diffuse"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let chunk_depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chunk depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let layout = |groups: &[Option<&wgpu::BindGroupLayout>]| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: groups, immediate_size: 0 });
        let programs = state::Programs {
            sprite_layout: layout(&[Some(&globals_layout), Some(&texture_layout)]),
            tile_layout: layout(&[Some(&globals_layout), Some(&texture_layout), Some(&texture_layout)]),
            composite_layout: layout(&[Some(&globals_layout), Some(&texture_layout), Some(&chunk_depth_layout)]),
            seam_layout: layout(&[Some(&globals_layout), Some(&texture_layout), Some(&texture_layout), Some(&texture_layout)]),
            sprite,
            world_tile,
            world_composite,
            surface,
            particles,
            screen,
        };
        let depth_stencil = depth_stencil_view(&device, width, height);
        let models = model::ModelPipelines::new(&device, &texture_layout);
        let vehicles = vehicle::VehiclePipelines::new(&device);
        let outlines = outline::OutlinePipelines::new(&device, &models.uniform_layout, &texture_layout, &globals_layout);
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mip source"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let mip_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("mipmap"), source: wgpu::ShaderSource::Wgsl(include_str!("mipmap.wgsl").into()) });
        let mip_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mipmap"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&mip_layout)], immediate_size: 0 })),
            vertex: wgpu::VertexState { module: &mip_module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mip_module,
                entry_point: Some("fs_down"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: state::CHUNK_COLOR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let sky_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("skybox"), source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()) });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skybox"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&sky_layout)], immediate_size: 0 })),
            vertex: wgpu::VertexState { module: &sky_module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: state::OFFSCREEN_COLOR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let combiner = combiner::CombinerState::new(&device, &texture_layout);
        let mut gpu = Gpu {
            device,
            queue,
            target,
            format,
            width,
            height,
            depth_stencil,
            programs,
            globals_layout,
            texture_layout,
            chunk_depth_layout,
            pipelines: HashMap::new(),
            chunks: HashMap::new(),
            models,
            vehicles,
            outlines,
            filters: HashMap::new(),
            mip_pipeline,
            mip_layout,
            mip_sampler,
            sky_pipeline,
            sky_layout,
            sky_targets: HashMap::new(),
            views: HashMap::new(),
            nearest: HashMap::new(),
            offscreen: None,
            weather: [None, None],
            textures: HashMap::new(),
            samplers: HashMap::new(),
            white: None,
            state: GlState::default(),
            state_stack: Vec::new(),
            combiner,
            faults,
        };
        gpu.white = Some(gpu.create_texture_bind_group(1, 1, &[vec![0xFF; 4]], (true, true, false, false)));
        gpu
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Back-buffer size changed (physical pixels).
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        self.width = width;
        self.height = height;
        match &mut self.target {
            Target::Surface { surface, config } => {
                config.width = width;
                config.height = height;
                surface.configure(&self.device, config);
            }
            Target::Headless { texture } => *texture = headless_texture(&self.device, self.format, width, height),
        }
        self.depth_stencil = depth_stencil_view(&self.device, width, height);
    }

    /// `Core.OptionVSync`.
    pub fn set_vsync(&mut self, on: bool) {
        if let Target::Surface { surface, config } = &mut self.target {
            config.present_mode = if on { wgpu::PresentMode::Fifo } else { wgpu::PresentMode::AutoNoVsync };
            surface.configure(&self.device, config);
        }
    }

    /// The first device loss or out-of-memory error, or a texture target
    /// that could not be allocated (its draws are skipped). After a loss
    /// frames are no longer drawn and [`Gpu::try_read_pixels`] fails.
    pub fn error(&self) -> Option<GpuError> {
        self.faults.get()
    }

    fn lost(&self) -> bool {
        matches!(self.faults.get(), Some(GpuError::DeviceLost(_)))
    }

    /// Runs `create` (which allocates GPU memory) inside out-of-memory and
    /// validation error scopes; a failure is recorded (see [`Gpu::error`])
    /// and `None` returned, so the caller skips that target instead of
    /// drawing into a texture that does not exist.
    fn allocate<T>(&self, what: &str, create: impl FnOnce(&wgpu::Device) -> T) -> Option<T> {
        if self.lost() {
            return None;
        }
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let invalid = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let t = create(&self.device);
        let invalid = pollster::block_on(invalid.pop());
        let oom = pollster::block_on(oom.pop());
        if let Some(e) = oom {
            self.faults.record(GpuError::OutOfMemory(format!("{what}: {e}")));
            return None;
        }
        if let Some(e) = invalid {
            self.faults.record(GpuError::Allocation(format!("{what}: {e}")));
            return None;
        }
        Some(t)
    }

    /// Draws one frame and presents it. Nothing is drawn once the device
    /// is lost; check [`Gpu::error`].
    pub fn render_frame(&mut self, segments: &[(FrameParams, &DrawList)], textures: &mut TextureManager) {
        if self.lost() {
            return;
        }
        let (frame, view) = match &self.target {
            Target::Surface { surface, .. } => {
                let st = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    _ => return,
                };
                let view = st.texture.create_view(&wgpu::TextureViewDescriptor::default());
                (Some(st), view)
            }
            Target::Headless { texture } => (None, texture.create_view(&wgpu::TextureViewDescriptor::default())),
        };
        self.draw(segments, textures, &view);
        if let Some(st) = frame {
            self.queue.present(st);
        }
    }

    /// The headless target's pixels, RGBA, top row first. Panics with the
    /// device's error if it was lost or the read failed; see
    /// [`Gpu::try_read_pixels`].
    pub fn read_pixels(&self) -> Vec<u8> {
        self.try_read_pixels().unwrap_or_else(|e| panic!("pz-gpu read_pixels: {e}"))
    }

    /// The headless target's pixels (empty for a window surface), or why
    /// they could not be read.
    pub fn try_read_pixels(&self) -> Result<Vec<u8>, GpuError> {
        if let Some(e @ GpuError::DeviceLost(_)) = self.faults.get() {
            return Err(e);
        }
        let Target::Headless { texture } = &self.target else { return Ok(Vec::new()) };
        let (w, h) = (self.width, self.height);
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let polled = self.device.poll(wgpu::PollType::wait_indefinitely());
        let lost = |why: String| self.faults.get().unwrap_or(GpuError::DeviceLost(why));
        match rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(lost(format!("readback map: {e}"))),
            Err(_) => return Err(lost(format!("readback never mapped ({polled:?})"))),
        }
        let data = slice.get_mapped_range().map_err(|e| lost(format!("readback range: {e:?}")))?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        Ok(out)
    }

    // ---- frame building ---------------------------------------------------

    fn draw(&mut self, segments: &[(FrameParams, &DrawList)], textures: &mut TextureManager, view: &wgpu::TextureView) {
        let mut f = FrameBuilder { viewport: (self.width, self.height), ..Default::default() };
        let mut clear_first: Option<[f64; 4]> = None;
        for (i, (params, list)) in segments.iter().enumerate() {
            f.viewport = (params.width.min(self.width), params.height.min(self.height));
            f.origin = (0, 0);
            if let Some(c) = params.clear {
                if i == 0 {
                    clear_first = Some(c);
                } else {
                    let g = f.globals(&self.state, IDENTITY, 0.5);
                    f.clear(&self.state, g, gl::COLOR_BUFFER_BIT, c);
                }
            }
            // `DoStartFrameStuff`: blending on, new projection and viewport.
            self.state.blend = true;
            let (w, h) = (params.width as f32 * params.zoom, params.height as f32 * params.zoom);
            f.mvp = gl::ortho2d(0.0, w, h, 0.0);
            for cmd in &list.cmds {
                self.record(&mut f, cmd);
            }
        }
        for i in 0..f.batches.len() {
            if let Some(id) = f.batches[i].texture {
                self.ensure_texture(id, textures);
            }
            if let Tex2::Texture(id) | Tex2::Surface(Some(id)) = f.batches[i].tex2 {
                self.ensure_texture(id, textures);
            }
            if let Some(id) = f.batches[i].tex3 {
                self.ensure_texture(id, textures);
            }
            if let (Tex2::Nearest, Some(id)) = (f.batches[i].tex2, f.batches[i].texture) {
                self.ensure_nearest(id);
            }
            if let Some(op) = f.batches[i].outline {
                match op {
                    outline::Op::Solid { kind, cull, texture, .. } => {
                        if let Some(id) = texture {
                            self.ensure_texture(id, textures);
                        }
                        self.outlines.ensure_solid(&self.device, kind, cull);
                    }
                    outline::Op::Sprite { texture: outline::SpriteSource::Texture(id), .. } => self.ensure_texture(id, textures),
                    outline::Op::Sprite { .. } => {}
                    outline::Op::Thicken { .. } => self.outlines.ensure_thicken(&self.device),
                    outline::Op::Blit { target } => self.outlines.ensure_blit(&self.device, target, self.format),
                }
                continue;
            }
            if let Some((key, _, _, ids)) = f.batches[i].vehicle {
                for id in ids.into_iter().filter(|&id| id != NO_TEXTURE) {
                    self.ensure_texture(id, textures);
                }
                self.vehicles.ensure(&self.device, self.format, key);
                self.ensure_vehicle_textures(ids);
                continue;
            }
            match f.batches[i].model {
                Some((key, _, _)) => self.models.ensure(&self.device, self.format, key),
                None => self.ensure_pipeline(f.batches[i].key),
            }
        }
        let FrameBuilder { mut vertices, mut indices, mut globals, batches, passes, freed, model_uniforms, sky, vehicle_uniforms, outline_uniforms, .. } = f;
        let mut obytes = vec![0u8; outline_uniforms.len().max(1) * outline::PASS_STRIDE as usize];
        for (i, u) in outline_uniforms.iter().enumerate() {
            let at = i * outline::PASS_STRIDE as usize;
            obytes[at..at + std::mem::size_of::<outline::PassUniforms>()].copy_from_slice(bytemuck::bytes_of(u));
        }
        let obuf = self.buffer(&obytes, wgpu::BufferUsages::UNIFORM);
        let obind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("outline passes"),
            layout: &self.outlines.pass_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &obuf, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<outline::PassUniforms>() as u64) }),
            }],
        });
        let mut vbytes = vec![0u8; vehicle_uniforms.len().max(1) * vehicle::VEHICLE_STRIDE as usize];
        for (i, u) in vehicle_uniforms.iter().enumerate() {
            let at = i * vehicle::VEHICLE_STRIDE as usize;
            vbytes[at..at + std::mem::size_of::<vehicle::VehicleUniforms>()].copy_from_slice(bytemuck::bytes_of(u));
        }
        let vehbuf = self.buffer(&vbytes, wgpu::BufferUsages::UNIFORM);
        let vehbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vehicles"),
            layout: &self.vehicles.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &vehbuf, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<vehicle::VehicleUniforms>() as u64) }),
            }],
        });
        let mut mbytes = vec![0u8; model_uniforms.len().max(1) * model::MODEL_STRIDE as usize];
        for (i, u) in model_uniforms.iter().enumerate() {
            let at = i * model::MODEL_STRIDE as usize;
            mbytes[at..at + std::mem::size_of::<model::ModelUniforms>()].copy_from_slice(bytemuck::bytes_of(u));
        }
        let mbuf = self.buffer(&mbytes, wgpu::BufferUsages::UNIFORM);
        let mbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("models"),
            layout: &self.models.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &mbuf, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<model::ModelUniforms>() as u64) }),
            }],
        });
        if vertices.is_empty() {
            vertices.push(Vertex::zeroed());
        }
        if indices.is_empty() {
            indices.push(0);
        }
        if globals.is_empty() {
            globals.push(Globals::zeroed());
        }
        let vbuf = self.buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let ibuf = self.buffer(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
        let mut gbytes = vec![0u8; globals.len() * GLOBALS_STRIDE as usize];
        for (i, g) in globals.iter().enumerate() {
            let at = i * GLOBALS_STRIDE as usize;
            gbytes[at..at + std::mem::size_of::<Globals>()].copy_from_slice(bytemuck::bytes_of(g));
        }
        let gbuf = self.buffer(&gbytes, wgpu::BufferUsages::UNIFORM);
        let gbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &gbuf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Globals>() as u64),
                }),
            }],
        });

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        // The main target is cleared (if asked) by its first pass, even
        // when that pass is empty.
        let mut passes = passes;
        if !passes.iter().any(|p| p.target == PassTarget::Main) {
            passes.push(PassDesc { target: PassTarget::Main, clear: false, mipmaps: false, batches: 0..0 });
        }
        let mut main_cleared = false;
        for pass_desc in &passes {
            if let PassTarget::Sky(id, u) = pass_desc.target {
                self.sky_pass(&mut enc, id, &sky[u]);
                continue;
            }
            if let PassTarget::Outline(i) = pass_desc.target {
                let Some(t) = self.outlines.targets.as_ref() else { continue };
                let load = if pass_desc.clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("outline"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.views[i as usize], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let white = self.white.as_ref().expect("white texture");
                for b in &batches[pass_desc.batches.clone()] {
                    let Some(op) = b.outline else { continue };
                    set_viewport(&mut pass, b, t.width, t.height, true);
                    match op {
                        outline::Op::Solid { kind, cull, mesh, uniform, texture } => {
                            let (Some(p), Some(m)) = (self.outlines.solid.get(&(kind, cull)), self.models.meshes.get(&mesh)) else { continue };
                            let tex = texture.and_then(|id| self.textures.get(&id)).unwrap_or(white);
                            pass.set_pipeline(p);
                            pass.set_bind_group(0, &mbind, &[uniform * model::MODEL_STRIDE as u32]);
                            pass.set_bind_group(1, tex, &[]);
                            pass.set_vertex_buffer(0, m.vertices.slice(..));
                            pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..m.count, 0, 0..1);
                        }
                        outline::Op::Thicken { vertical, uniform } => {
                            let Some(p) = self.outlines.thicken.as_ref() else { continue };
                            pass.set_pipeline(&p[usize::from(vertical)]);
                            pass.set_bind_group(0, &obind, &[uniform * outline::PASS_STRIDE as u32]);
                            pass.set_bind_group(1, &t.binds[if vertical { outline::THICK_H } else { outline::MASK } as usize], &[]);
                            pass.set_bind_group(2, &t.binds[outline::MASK as usize], &[]);
                            pass.draw(0..3, 0..1);
                        }
                        outline::Op::Sprite { texture, globals } => {
                            let tex = match texture {
                                outline::SpriteSource::Texture(id) => self.textures.get(&id),
                                outline::SpriteSource::Atlas(id) => self.chunks.get(&id).map(|c| &c.composite[1]),
                            };
                            let Some(tex) = tex else { continue };
                            pass.set_pipeline(&self.outlines.sprite);
                            pass.set_bind_group(0, &gbind, &[globals * GLOBALS_STRIDE as u32]);
                            pass.set_bind_group(1, tex, &[]);
                            pass.set_vertex_buffer(0, vbuf.slice(..));
                            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(b.indices.clone(), 0, 0..1);
                        }
                        outline::Op::Blit { .. } => {}
                    }
                }
                continue;
            }
            let (color_view, depth_view, load, depth_ops, stencil_ops, tw, th) = match pass_desc.target {
                PassTarget::Main => {
                    let load = match clear_first.filter(|_| !main_cleared) {
                        Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: c[3] }),
                        None => wgpu::LoadOp::Load,
                    };
                    main_cleared = true;
                    let keep = wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store };
                    (view, &self.depth_stencil, load, Some(keep), Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), self.width, self.height)
                }
                PassTarget::Chunk(id) | PassTarget::Atlas(id) => {
                    let Some(c) = self.chunks.get(&id) else { continue };
                    let (load, dload) = if pass_desc.clear {
                        (wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), wgpu::LoadOp::Clear(1.0))
                    } else {
                        (wgpu::LoadOp::Load, wgpu::LoadOp::Load)
                    };
                    (&c.color, &c.depth, load, Some(wgpu::Operations { load: dload, store: wgpu::StoreOp::Store }), None, c.width, c.height)
                }
                PassTarget::Sky(..) => continue,
                PassTarget::WeatherFx(i) => {
                    let Some(o) = self.weather[i].as_ref() else { continue };
                    let (load, dload) = if pass_desc.clear {
                        (wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), wgpu::LoadOp::Clear(1.0))
                    } else {
                        (wgpu::LoadOp::Load, wgpu::LoadOp::Load)
                    };
                    let keep_s = wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store };
                    (&o.color, &o.depth_stencil, load, Some(wgpu::Operations { load: dload, store: wgpu::StoreOp::Store }), Some(keep_s), o.width, o.height)
                }
                PassTarget::Outline(_) => continue,
                PassTarget::Offscreen => {
                    let Some(o) = self.offscreen.as_ref() else { continue };
                    let keep = wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store };
                    let keep_s = wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store };
                    (&o.color, &o.depth_stencil, wgpu::LoadOp::Load, Some(keep), Some(keep_s), o.width, o.height)
                }
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops, stencil_ops }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            let white = self.white.as_ref().expect("white texture");
            for b in &batches[pass_desc.batches.clone()] {
                let [sx, sy, sw, sh] = match b.scissor {
                    Some(s) if !matches!(pass_desc.target, PassTarget::Chunk(_)) => clip_scissor(s, tw, th),
                    _ => [0, 0, tw, th],
                };
                if sw == 0 || sh == 0 {
                    continue;
                }
                if let Some(op) = b.outline {
                    let outline::Op::Blit { target } = op else { continue };
                    let fmt = match target {
                        TargetKind::Main => self.format,
                        TargetKind::Offscreen => state::OFFSCREEN_COLOR_FORMAT,
                        TargetKind::Chunk => state::CHUNK_COLOR_FORMAT,
                    };
                    let (Some(p), Some(t)) = (self.outlines.blit.get(&(target, fmt)), self.outlines.targets.as_ref()) else { continue };
                    set_viewport(&mut pass, b, tw, th, pass_desc.target != PassTarget::Main);
                    pass.set_scissor_rect(sx, sy, sw, sh);
                    pass.set_pipeline(p);
                    pass.set_bind_group(0, &obind, &[0]);
                    pass.set_bind_group(1, &t.binds[outline::THICK_V as usize], &[]);
                    pass.set_bind_group(2, &t.binds[outline::MASK as usize], &[]);
                    pass.draw(0..3, 0..1);
                    continue;
                }
                if let Some((key, mesh, u, ids)) = b.vehicle {
                    let (Some(p), Some(m), Some(tex)) = (self.vehicles.pipelines.get(&key), self.vehicles.meshes.get(&mesh), self.vehicles.bind_groups.get(&ids)) else { continue };
                    set_viewport(&mut pass, b, tw, th, pass_desc.target != PassTarget::Main);
                    pass.set_scissor_rect(sx, sy, sw, sh);
                    pass.set_pipeline(p);
                    pass.set_bind_group(0, &vehbind, &[u * vehicle::VEHICLE_STRIDE as u32]);
                    pass.set_bind_group(1, tex, &[]);
                    pass.set_vertex_buffer(0, m.vertices.slice(..));
                    pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                    pass.set_vertex_buffer(0, vbuf.slice(..));
                    pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    continue;
                }
                if let Some((key, mesh, u)) = b.model {
                    let (Some(p), Some(m)) = (self.models.pipelines.get(&key), self.models.meshes.get(&mesh)) else { continue };
                    let tex = b.texture.and_then(|id| self.textures.get(&id)).unwrap_or(white);
                    set_viewport(&mut pass, b, tw, th, pass_desc.target != PassTarget::Main);
                    pass.set_scissor_rect(sx, sy, sw, sh);
                    pass.set_pipeline(p);
                    pass.set_bind_group(0, &mbind, &[u * model::MODEL_STRIDE as u32]);
                    pass.set_bind_group(1, tex, &[]);
                    pass.set_vertex_buffer(0, m.vertices.slice(..));
                    pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                    pass.set_vertex_buffer(0, vbuf.slice(..));
                    pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    continue;
                }
                let tex = match b.tex2 {
                    Tex2::Chunk(id, linear) => match self.chunks.get(&id) {
                        Some(c) => {
                            pass.set_bind_group(2, &c.depth_bind, &[]);
                            &c.composite[usize::from(linear)]
                        }
                        None => continue,
                    },
                    Tex2::Texture(id) => {
                        let Some(t2) = self.textures.get(&id) else { continue };
                        pass.set_bind_group(2, t2, &[]);
                        b.texture.and_then(|id| self.textures.get(&id)).unwrap_or(white)
                    }
                    Tex2::Nearest => match b.texture.and_then(|id| self.nearest.get(&id)) {
                        Some(bg) => bg,
                        None => continue,
                    },
                    Tex2::Offscreen => match self.offscreen.as_ref() {
                        Some(o) => &o.bind,
                        None => continue,
                    },
                    Tex2::Surface(id) => {
                        pass.set_bind_group(2, id.and_then(|id| self.textures.get(&id)).unwrap_or(white), &[]);
                        pass.set_bind_group(3, b.tex3.and_then(|id| self.textures.get(&id)).unwrap_or(white), &[]);
                        b.texture.and_then(|id| self.textures.get(&id)).unwrap_or(white)
                    }
                    Tex2::None => b.texture.and_then(|id| self.textures.get(&id)).unwrap_or(white),
                };
                if let (Some(id), false) = (b.tex3, matches!(b.tex2, Tex2::Surface(_))) {
                    let Some(t3) = self.textures.get(&id) else { continue };
                    pass.set_bind_group(3, t3, &[]);
                }
                set_viewport(&mut pass, b, tw, th, pass_desc.target != PassTarget::Main);
                pass.set_pipeline(&self.pipelines[&b.key]);
                pass.set_bind_group(0, &gbind, &[b.globals * GLOBALS_STRIDE as u32]);
                pass.set_bind_group(1, tex, &[]);
                pass.set_stencil_reference(b.stencil_ref);
                pass.set_scissor_rect(sx, sy, sw, sh);
                pass.draw_indexed(b.indices.clone(), 0, 0..1);
            }
            drop(pass);
            if let (true, PassTarget::Chunk(id)) = (pass_desc.mipmaps, pass_desc.target) {
                self.generate_mips(&mut enc, id);
            }
        }
        self.queue.submit([enc.finish()]);
        for id in freed {
            self.chunks.remove(&id);
        }
    }

    fn record(&mut self, f: &mut FrameBuilder, cmd: &DrawCmd) {
        match cmd {
            DrawCmd::Sprite(s) => {
                let g = f.globals(&self.state, f.mvp_for_target(), 0.5);
                let key = self.key(f, Program::Default);
                f.push(key, s.texture.as_ref().and_then(|t| t.id), Tex2::None, &self.state, g, sprite_vertices(s));
            }
            DrawCmd::WorldSprite(s, shader) => {
                let mut g = f.globals(&self.state, f.mvp_for_target(), 0.5);
                let mut v = sprite_vertices(s);
                let (program, tex2) = match shader {
                    WorldShader::Default { z } => {
                        // GL clip z (-1..1) to window depth.
                        let wz = (z + 1.0) * 0.5;
                        v.iter_mut().for_each(|v| v.depth = [wz, 0.0]);
                        (Program::WorldDefault, Tex2::None)
                    }
                    WorldShader::TileDepth { depth, uv1, front, far, opaque } => {
                        for (vx, uv) in v.iter_mut().zip(uv1) {
                            vx.uv1 = *uv;
                            vx.depth = [*front, *far];
                        }
                        let Some(id) = depth.id else { return };
                        (if *opaque { Program::OpaqueDepth } else { Program::TileDepth }, Tex2::Texture(id))
                    }
                    WorldShader::Cutaway { depth, uv1, stencil, uv2, front, far, discard_value } => {
                        for ((vx, a), b) in v.iter_mut().zip(uv1).zip(uv2) {
                            vx.uv1 = *a;
                            vx.uv2 = *b;
                            vx.depth = [*front, *far];
                        }
                        let (Some(id), Some(sid)) = (depth.id, stencil.id) else { return };
                        f.tex3 = Some(sid);
                        let mut gl = f.globals[g as usize];
                        gl.extra = *discard_value;
                        g = f.push_globals(gl);
                        (Program::Cutaway, Tex2::Texture(id))
                    }
                    WorldShader::CutawayNoDepth { stencil, uv2 } => {
                        for (vx, b) in v.iter_mut().zip(uv2) {
                            vx.uv2 = *b;
                        }
                        let Some(sid) = stencil.id else { return };
                        f.tex3 = Some(sid);
                        (Program::CutawayNoDepth, Tex2::Texture(sid))
                    }
                    WorldShader::Seam { depth, uv1, mask, uv2, front, far } => {
                        for ((vx, a), b) in v.iter_mut().zip(uv1).zip(uv2) {
                            vx.uv1 = *a;
                            vx.uv2 = *b;
                            vx.depth = [*front, *far];
                        }
                        let (Some(id), Some(mid)) = (depth.id, mask.id) else { return };
                        f.tex3 = Some(mid);
                        (Program::Seam, Tex2::Texture(id))
                    }
                };
                let key = self.key(f, program);
                f.push(key, s.texture.as_ref().and_then(|t| t.id), tex2, &self.state, g, v);
                f.tex3 = None;
            }
            DrawCmd::Quad { texture, x, y, w, h, color } => {
                let mut dl = DrawList::default();
                dl.render(texture.as_ref(), *x, *y, *w, *h, *color);
                for c in &dl.cmds {
                    self.record(f, c);
                }
            }
            DrawCmd::Clear(mask) => {
                let g = f.globals(&self.state, IDENTITY, 1.0);
                for bit in [gl::STENCIL_BUFFER_BIT, gl::DEPTH_BUFFER_BIT, gl::COLOR_BUFFER_BIT] {
                    if mask & bit != 0 {
                        f.clear(&self.state, g, bit, self.state.clear_color);
                    }
                }
            }
            DrawCmd::Ortho { left, right, bottom, top, width, height } => {
                f.mvp = gl::ortho2d(*left, *right, *bottom, *top);
                f.viewport = (*width, *height);
            }
            DrawCmd::Viewport { x, y, width, height } => {
                f.origin = (*x, *y);
                f.viewport = (*width, *height);
            }
            DrawCmd::ChunkBegin { id, width, height, clear } => {
                self.ensure_chunk(*id, *width, *height);
                // TextureFBO.startDrawing remembers the bound buffer.
                if !matches!(f.target, PassTarget::Chunk(_)) {
                    f.chunk_return = f.target;
                    f.chunk_origin = f.origin;
                }
                // StartFrameFlipY: glViewport(0, 0, w, h).
                f.origin = (0, 0);
                f.target = PassTarget::Chunk(*id);
                f.passes.push(PassDesc { target: f.target, clear: *clear, mipmaps: false, batches: f.batches.len()..f.batches.len() });
            }
            DrawCmd::ChunkEnd => {
                // FBORenderChunk.endRenderThread: glGenerateMipmap.
                let t = f.target;
                if let Some(p) = f.passes.iter_mut().rev().find(|p| p.target == t) {
                    p.mipmaps = true;
                }
                f.target = f.chunk_return;
                f.origin = f.chunk_origin;
            }
            DrawCmd::ChunkComposite { id, x, y, w, h, color, chunk_depth, mag_linear } => {
                if !self.chunks.contains_key(id) {
                    return;
                }
                let g = f.globals(&self.state, f.mvp_for_target(), 0.5);
                let s = pz_render::Sprite {
                    texture: None,
                    x: [*x, x + w, x + w, *x],
                    y: [*y, *y, y + h, y + h],
                    u: [0.0, 1.0, 1.0, 0.0],
                    v: [0.0, 0.0, 1.0, 1.0],
                    color: [*color; 4],
                    flipped: false,
                };
                let mut v = sprite_vertices(&s);
                for (vx, (u, vv)) in v.iter_mut().zip(s.u.iter().zip(s.v)) {
                    vx.uv = [*u, vv];
                    vx.depth = [*chunk_depth, 0.0];
                }
                let key = self.key(f, Program::Composite);
                f.push(key, None, Tex2::Chunk(*id, *mag_linear), &self.state, g, v);
            }
            DrawCmd::ChunkFree(id) => f.freed.push(*id),
            DrawCmd::AtlasBegin { .. } | DrawCmd::AtlasEnd | DrawCmd::AtlasQuad { .. } => self.record_atlas(f, cmd),
            DrawCmd::OffscreenBegin { width, height } => {
                self.ensure_offscreen(*width, *height);
                f.target = PassTarget::Offscreen;
            }
            DrawCmd::OffscreenEnd => f.target = PassTarget::Main,
            DrawCmd::WeatherFxBegin { index, width, height } => {
                let i = (*index).min(1);
                self.ensure_weather(i, *width, *height);
                // TextureFBO.startDrawing remembers the bound buffer.
                if !matches!(f.target, PassTarget::WeatherFx(_)) {
                    f.fx_return = f.target;
                }
                f.target = PassTarget::WeatherFx(i);
                f.passes.push(PassDesc { target: f.target, clear: true, mipmaps: false, batches: f.batches.len()..f.batches.len() });
            }
            DrawCmd::WeatherFxEnd => f.target = f.fx_return,
            DrawCmd::FogCircle { sprite, width, height } => {
                // The shader's own u_mvp, flipped like any texture target's.
                let mut m = gl::ortho2d(0.0, *width, *height, 0.0);
                if f.target != PassTarget::Main {
                    for col in &mut m {
                        col[1] = -col[1];
                    }
                }
                let g = f.globals(&self.state, m, 0.5);
                let key = self.key(f, Program::FogCircle);
                f.push(key, sprite.texture.as_ref().and_then(|t| t.id), Tex2::None, &self.state, g, sprite_vertices(sprite));
            }
            DrawCmd::Model(d) => {
                if d.alpha < 0.01 {
                    return;
                }
                if !self.models.meshes.contains_key(&d.mesh.id) {
                    let m = model::upload(&self.device, &self.queue, &d.mesh);
                    self.models.meshes.insert(d.mesh.id, m);
                }
                let flip = f.target != PassTarget::Main;
                f.model_uniforms.push(model::ModelUniforms::new(d, flip));
                let u = (f.model_uniforms.len() - 1) as u32;
                let key = model::ModelKey { kind: d.mesh.kind, pass: d.pass, target: f.target.kind() };
                f.push_model(key, d.mesh.id, u, d.texture.as_ref().and_then(|t| t.id), &self.state);
            }
            DrawCmd::Surface(d) => {
                // The original pushes and pops the GL attributes around the
                // draw.
                let saved = self.state.clone();
                self.state.depth_test = true;
                self.state.depth_func = d.depth_func;
                self.state.depth_mask = false;
                self.state.blend = true;
                self.state.blend_func = d.blend_func;
                self.state.alpha_test = false;
                let u = &d.uniforms;
                let flip = if f.target == PassTarget::Main { f.viewport.1 as f32 } else { 0.0 };
                let mut g = Globals::zeroed();
                g.mvp = f.mvp_for_target();
                g.alpha_func = 8;
                g.clip_z = 0.5;
                g.surface = [[u.time, u.reflection_param, flip, 0.0], u.offset, u.viewport, u.params[0], u.params[1]];
                let gi = f.push_globals(g);
                let key = self.key(f, Program::Surface(d.program));
                let tex = d.ground.as_ref().and_then(|t| t.id);
                let refl = |i: usize| d.reflection[i].as_ref().and_then(|t| t.id);
                f.tex3 = refl(1);
                for q in &d.quads {
                    let v: [Vertex; 4] = std::array::from_fn(|i| {
                        let s = &q[i];
                        Vertex { pos: s.pos, uv: [s.attr[0], s.attr[1]], color: s.color, uv1: [s.attr[2], s.attr[3]], uv2: [0.0; 2], depth: [s.depth, 0.0] }
                    });
                    // IsoWater.renderSome splits on the 0-2 diagonal when
                    // corners 0 and 2 have the same depth; IsoPuddles always.
                    let split02 = !d.program.is_water() || q[0].attr[0] == q[2].attr[0];
                    f.push_split(key, tex, Tex2::Surface(refl(0)), &self.state, gi, v, split02);
                }
                f.tex3 = None;
                self.state = saved;
            }
            DrawCmd::Particles(d) => {
                // RenderParticles pushes and pops the GL attributes around
                // the two draws; this port leaves the depth test off.
                let saved = self.state.clone();
                let saved_viewport = f.viewport;
                self.state.depth_test = false;
                self.state.blend = true;
                self.state.blend_func = [d.blend_func[0], d.blend_func[1], d.blend_func[0], d.blend_func[1]];
                self.state.alpha_test = false;
                f.viewport = d.viewport;
                let mut mvp = d.mvp;
                if f.target != PassTarget::Main {
                    for col in &mut mvp {
                        col[1] = -col[1];
                    }
                }
                let mut g = Globals::zeroed();
                g.mvp = mvp;
                g.alpha_func = 8;
                g.clip_z = 0.5;
                let col = |c: usize| [d.param[c][0], d.param[c][1], d.param[c][2], 0.0];
                g.surface = [[d.time, 0.0, 0.0, 0.0], col(0), col(1), col(2), [0.0; 4]];
                let gi = f.push_globals(g);
                let key = self.key(f, Program::Particles(d.program));
                let tex = d.texture.as_ref().and_then(|t| t.id);
                // The strip (-1,-1), (1,-1), (-1,1), (1,1) as a quad split
                // on its 1-3 diagonal.
                let corners = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
                for p in &d.particles {
                    let v: [Vertex; 4] = std::array::from_fn(|i| Vertex { pos: corners[i], uv: [p[0], p[1]], color: 0xFFFF_FFFF, uv1: [p[2], p[3]], uv2: [0.0; 2], depth: [0.0; 2] });
                    f.push_split(key, tex, Tex2::None, &self.state, gi, v, false);
                }
                f.viewport = saved_viewport;
                self.state = saved;
            }
            DrawCmd::SkyBox(d) => {
                let id = pz_render::SKYBOX_TEXTURE_IDS[d.target.min(1)];
                self.ensure_sky_target(id);
                let u = &d.uniforms;
                let v3 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
                f.sky.push(SkyUniforms {
                    a: [u.time, u.cloud_count, u.cloud_size, u.fog],
                    sun_light: v3(u.sun_light),
                    sun_colour: v3(u.sun_colour),
                    sky_h: v3(u.sky_h_colour),
                    sky_l: v3(u.sky_l_colour),
                    b: [u.cloud_light, u.stars, if d.hires { 1.0 } else { 0.0 }, 0.0],
                    wind: v3(u.wind),
                });
                let at = f.batches.len();
                f.passes.push(PassDesc { target: PassTarget::Sky(id, f.sky.len() - 1), clear: true, mipmaps: false, batches: at..at });
            }
            DrawCmd::VehicleModel(d) => {
                if !self.vehicles.meshes.contains_key(&d.base.mesh.id) {
                    let m = vehicle::upload(&self.device, &self.queue, &d.base.mesh);
                    self.vehicles.meshes.insert(d.base.mesh.id, m);
                }
                let flip = f.target != PassTarget::Main;
                f.vehicle_uniforms.push(vehicle::VehicleUniforms::new(d, flip));
                let u = (f.vehicle_uniforms.len() - 1) as u32;
                let key = vehicle::VehicleKey {
                    kind: vehicle::layout_kind(d.base.mesh.kind),
                    shader: d.shader,
                    reflect: !d.noreflect,
                    invert_x: d.invert_x,
                    target: f.target.kind(),
                };
                let ids = std::array::from_fn(|i| d.textures[i].as_ref().and_then(|t| t.id).unwrap_or(NO_TEXTURE));
                f.push_vehicle(key, d.base.mesh.id, u, ids, &self.state);
            }
            DrawCmd::OutlineBegin { width, height, clear } => {
                let sampler = self.sampler((false, false, false, true));
                self.outlines.ensure_targets(&self.device, &self.texture_layout, &sampler, *width, *height);
                // TextureFBO.startDrawing remembers the bound buffer.
                if !matches!(f.target, PassTarget::Outline(_)) {
                    f.outline_return = f.target;
                }
                f.target = PassTarget::Outline(outline::MASK);
                let at = f.batches.len();
                f.passes.push(PassDesc { target: f.target, clear: *clear, mipmaps: false, batches: at..at });
            }
            DrawCmd::OutlineEnd => f.target = f.outline_return,
            DrawCmd::OutlineModel { draw, color } => {
                if f.target != PassTarget::Outline(outline::MASK) {
                    return;
                }
                if !self.models.meshes.contains_key(&draw.mesh.id) {
                    let m = model::upload(&self.device, &self.queue, &draw.mesh);
                    self.models.meshes.insert(draw.mesh.id, m);
                }
                let mut u = model::ModelUniforms::new(draw, true);
                u.tint = [color[0], color[1], color[2], 1.0];
                f.model_uniforms.push(u);
                let uniform = (f.model_uniforms.len() - 1) as u32;
                let cull = match draw.pass {
                    pz_render::model::ModelPass::Character { cull } | pz_render::model::ModelPass::AnimatedModel { cull } | pz_render::model::ModelPass::Item { cull } => cull,
                    // The mask pipeline culls GL_FRONT only.
                    pz_render::model::ModelPass::AnimatedModelCullBack => false,
                };
                let op = outline::Op::Solid { kind: draw.mesh.kind, cull, mesh: draw.mesh.id, uniform, texture: draw.texture.as_ref().and_then(|t| t.id) };
                f.push_outline(op, &self.state);
            }
            DrawCmd::OutlineSprite { texture, atlas, pos, uv, color } => {
                if f.target != PassTarget::Outline(outline::MASK) {
                    return;
                }
                let source = match (atlas, texture.as_ref().and_then(|t| t.id)) {
                    (Some(a), _) if self.chunks.contains_key(a) => outline::SpriteSource::Atlas(*a),
                    (None, Some(id)) => outline::SpriteSource::Texture(id),
                    _ => return,
                };
                let g = f.globals(&self.state, f.mvp_for_target(), 0.5);
                let c = pz_render::color_to_abgr(color[0], color[1], color[2], color[3]);
                let v: [Vertex; 4] = std::array::from_fn(|i| Vertex { pos: pos[i], uv: uv[i], color: c, uv1: [0.0; 2], uv2: [0.0; 2], depth: [0.0; 2] });
                f.push_outline_quad(outline::Op::Sprite { texture: source, globals: g }, &self.state, v);
            }
            DrawCmd::OutlineThicken { color, radius, resolution } => {
                if self.outlines.targets.is_none() {
                    return;
                }
                f.outline_uniforms.push(outline::PassUniforms { color: *color, params: [resolution[0], resolution[1], *radius, 0.0] });
                let uniform = (f.outline_uniforms.len() - 1) as u32;
                let back = f.target;
                for (i, vertical) in [(outline::THICK_H, false), (outline::THICK_V, true)] {
                    f.target = PassTarget::Outline(i);
                    let at = f.batches.len();
                    f.passes.push(PassDesc { target: f.target, clear: true, mipmaps: false, batches: at..at });
                    f.push_outline(outline::Op::Thicken { vertical, uniform }, &self.state);
                }
                f.target = back;
            }
            DrawCmd::OutlineBlit => {
                if self.outlines.targets.is_none() || matches!(f.target, PassTarget::Outline(_)) {
                    return;
                }
                if f.outline_uniforms.is_empty() {
                    f.outline_uniforms.push(outline::PassUniforms::zeroed());
                }
                f.push_outline(outline::Op::Blit { target: f.target.kind() }, &self.state);
            }
            DrawCmd::ProjectedQuad { texture, pos, uv, color, nearest } => {
                let Some(id) = texture.id else { return };
                // The corners are already in normalised device coordinates.
                let g = {
                    let m = f.mvp;
                    f.mvp = IDENTITY;
                    let mm = f.mvp_for_target();
                    f.mvp = m;
                    f.globals(&self.state, mm, 0.5)
                };
                let v: [Vertex; 4] = std::array::from_fn(|i| Vertex { pos: [pos[i][0], pos[i][1]], uv: uv[i], color: *color, uv1: [0.0; 2], uv2: [0.0; 2], depth: [pos[i][2], 0.0] });
                let key = self.key(f, Program::Projected);
                f.push(key, Some(id), if *nearest { Tex2::Nearest } else { Tex2::None }, &self.state, g, v);
            }
            DrawCmd::ProjectedDepthQuad { texture, pos, uv, color } => {
                let Some(id) = texture.id else { return };
                let g = {
                    let m = f.mvp;
                    f.mvp = IDENTITY;
                    let mm = f.mvp_for_target();
                    f.mvp = m;
                    f.globals(&self.state, mm, 0.5)
                };
                let v: [Vertex; 4] = std::array::from_fn(|i| Vertex { pos: [pos[i][0], pos[i][1]], uv: uv[i], color: *color, uv1: [0.0; 2], uv2: [0.0; 2], depth: [pos[i][2], 0.0] });
                let key = self.key(f, Program::ProjectedDepth);
                f.push(key, Some(id), Tex2::None, &self.state, g, v);
            }
            DrawCmd::ScreenPass { x, y, w, h, tex, params } => {
                let Some(o) = self.offscreen.as_ref() else { return };
                let (tw, th) = (o.width as f32, o.height as f32);
                // Texture.rendershader2: v runs bottom-up in the buffer.
                let (u0, u1) = (tex[0] / tw, (tex[0] + tex[2]) / tw);
                let (v_top, v_bottom) = ((tex[1] + tex[3]) / th, tex[1] / th);
                let mut g = Globals::zeroed();
                g.mvp = f.mvp_for_target();
                g.alpha_func = 8;
                g.clip_z = 0.5;
                g.screen = [[params.desaturation, params.timer, f.viewport.0 as f32, f.viewport.1 as f32], [tw, th, 0.0, 0.0]];
                let gi = f.push_globals(g);
                let corner = |px: f32, py: f32, u: f32, v: f32| Vertex { pos: [px, py], uv: [u, v], color: 0xFFFF_FFFF, uv1: [0.0; 2], uv2: [0.0; 2], depth: [0.0; 2] };
                let v = [corner(*x, *y, u0, v_top), corner(x + w, *y, u1, v_top), corner(x + w, y + h, u1, v_bottom), corner(*x, y + h, u0, v_bottom)];
                let key = PipeKey { blend: None, ..self.key(f, Program::Screen) };
                f.push(key, None, Tex2::Offscreen, &self.state, gi, v);
            }
            DrawCmd::ProjectedQuadF32 { texture, pos, uv, color, nearest } => {
                let Some(id) = texture.id else { return };
                let g = {
                    let m = f.mvp;
                    f.mvp = IDENTITY;
                    let mm = f.mvp_for_target();
                    f.mvp = m;
                    f.globals(&self.state, mm, 0.5)
                };
                // The float colour rides in uv1/uv2, which the
                // `ProjectedF32` vertex layout reads as one vec4.
                let v: [Vertex; 4] = std::array::from_fn(|i| Vertex { pos: [pos[i][0], pos[i][1]], uv: uv[i], color: 0, uv1: [color[0], color[1]], uv2: [color[2], color[3]], depth: [pos[i][2], 0.0] });
                let key = self.key(f, Program::ProjectedF32);
                f.push(key, Some(id), if *nearest { Tex2::Nearest } else { Tex2::None }, &self.state, g, v);
            }
            DrawCmd::PushAttrib => self.state_stack.push(self.state.clone()),
            DrawCmd::PopAttrib => {
                if let Some(s) = self.state_stack.pop() {
                    self.state = s;
                }
            }
            other => self.state.apply(other),
        }
    }

    fn key(&self, f: &FrameBuilder, program: Program) -> PipeKey {
        PipeKey { program, target: f.target.kind(), ..self.state.pipe_key() }
    }

    /// `glGenerateMipmap` for chunk texture `id`: each level the box filter
    /// of the one above.
    fn generate_mips(&self, enc: &mut wgpu::CommandEncoder, id: u64) {
        let Some(c) = self.chunks.get(&id) else { return };
        let _ = &c.color_texture;
        for l in 1..c.levels.len() {
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.mip_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&c.levels[l - 1]) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.mip_sampler) },
                ],
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mipmap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &c.levels[l],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.mip_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// `SkyBox`'s `TextureFBO`: a 512x512 colour texture, registered under
    /// its reserved id so surface draws sample it (`GL_LINEAR` both ways).
    fn ensure_sky_target(&mut self, id: u32) {
        if self.sky_targets.contains_key(&id) {
            return;
        }
        let n = pz_render::SKYBOX_TEXTURE_SIZE;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SkyBox.textureFbo"),
            size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: state::OFFSCREEN_COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sampler = self.sampler((false, false, false, true));
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        self.textures.insert(id, bg);
        self.views.insert(id, view.clone());
        self.sky_targets.insert(id, view);
    }

    /// `ModelManager.RenderSkyBox`: clear, then the sky over the whole
    /// texture.
    fn sky_pass(&self, enc: &mut wgpu::CommandEncoder, id: u32, u: &SkyUniforms) {
        let Some(view) = self.sky_targets.get(&id) else { return };
        let buf = self.buffer(bytemuck::bytes_of(u), wgpu::BufferUsages::UNIFORM);
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.sky_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("skybox"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.sky_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// `MultiTextureFBO2.create`: the offscreen buffer, with a
    /// `GL_DEPTH24_STENCIL8` renderbuffer.
    fn ensure_offscreen(&mut self, width: u32, height: u32) {
        if self.offscreen.as_ref().is_some_and(|o| o.width == width && o.height == height) {
            return;
        }
        let tex = |device: &wgpu::Device, format, usage, label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let Some((color, depth_stencil)) = self.allocate(&format!("offscreen buffer {width}x{height}"), |d| {
            (tex(d, state::OFFSCREEN_COLOR_FORMAT, usage, "offscreen"), tex(d, state::DEPTH_STENCIL_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT, "offscreen depth"))
        }) else {
            self.offscreen = None;
            return;
        };
        let color = color.create_view(&Default::default());
        let depth_stencil = depth_stencil.create_view(&Default::default());
        let sampler = self.sampler((true, true, false, true));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        self.offscreen = Some(OffscreenTarget { width, height, color, depth_stencil, bind });
    }

    /// `WeatherFxMask.checkFbos`: weather texture `i` (`new Texture(w, h,
    /// 16)` in a `TextureFBO`), sampled linearly by its reserved id.
    fn ensure_weather(&mut self, i: usize, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if self.weather[i].as_ref().is_some_and(|o| o.width == width && o.height == height) {
            return;
        }
        let tex = |format, usage, label| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = tex(state::OFFSCREEN_COLOR_FORMAT, usage, "weather fx").create_view(&Default::default());
        let depth_stencil = tex(state::DEPTH_STENCIL_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT, "weather fx depth").create_view(&Default::default());
        let sampler = self.sampler((false, false, false, true));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let id = pz_render::WEATHER_FX_TEXTURE_IDS[i];
        self.textures.insert(id, bind.clone());
        self.views.insert(id, color.clone());
        self.nearest.remove(&id);
        self.weather[i] = Some(OffscreenTarget { width, height, color, depth_stencil, bind });
    }

    /// `FBORenderChunk.init`: colour and 16-bit depth textures of the
    /// given size.
    fn ensure_chunk(&mut self, id: u64, width: u32, height: u32) {
        if self.chunks.get(&id).is_some_and(|c| c.width == width && c.height == height) {
            return;
        }
        let mips = 32 - width.max(height).max(1).leading_zeros();
        let tex = |device: &wgpu::Device, format, label, mips| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        // A target that cannot be allocated is left out (an old one of
        // another size freed): its passes and composites are skipped.
        let Some((color_texture, depth_texture)) = self.allocate(&format!("chunk texture {id} {width}x{height}"), |d| {
            (tex(d, state::CHUNK_COLOR_FORMAT, "FBORenderChunk.tex", mips), tex(d, state::CHUNK_DEPTH_FORMAT, "FBORenderChunk.depth", 1))
        }) else {
            self.chunks.remove(&id);
            return;
        };
        let levels: Vec<wgpu::TextureView> = (0..mips)
            .map(|l| color_texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: l, mip_level_count: Some(1), ..Default::default() }))
            .collect();
        let color = levels[0].clone();
        let all = color_texture.create_view(&Default::default());
        let depth = depth_texture.create_view(&Default::default());
        // Minification GL_LINEAR_MIPMAP_LINEAR over the generated levels,
        // magnification GL_NEAREST or GL_LINEAR.
        let composite = [true, false].map(|mag_nearest| {
            let sampler = self.sampler((false, mag_nearest, true, true));
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&all) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            })
        });
        let depth_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.chunk_depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth) }],
        });
        self.chunks.insert(id, ChunkTarget { width, height, color_texture, levels, color, depth, composite, depth_bind });
    }

    fn buffer(&self, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        let size = (data.len() as u64).div_ceil(4) * 4;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut padded = data.to_vec();
        padded.resize(size as usize, 0);
        self.queue.write_buffer(&buf, 0, &padded);
        buf
    }

    fn ensure_pipeline(&mut self, key: PipeKey) {
        if !self.pipelines.contains_key(&key) {
            let p = state::create_pipeline(&self.device, &self.programs, self.format, key);
            self.pipelines.insert(key, p);
        }
    }

    /// `TextureID.generateHwId` + `assignFilteringFlags`: uploads the image
    /// (every mip level for `MIPMAPS` textures) and picks the sampler.
    fn ensure_texture(&mut self, id: u32, textures: &mut TextureManager) {
        if self.textures.contains_key(&id) {
            return;
        }
        let Some(d) = textures.data(id) else { return };
        if !d.ready {
            return;
        }
        let levels = d.upload_levels();
        if levels.is_empty() {
            return;
        }
        // 1x1 textures are always sampled GL_NEAREST.
        let one = d.width == 1 && d.height == 1;
        let filter = (d.min_nearest() || one, d.mag_nearest() || one, d.mipmaps() && !one, d.clamp_to_edge());
        let pixels: Vec<Vec<u8>> = levels.iter().map(|l| l.pixels.clone()).collect();
        let (bind_group, view) = self.create_texture(levels[0].width as u32, levels[0].height as u32, &pixels, filter);
        self.textures.insert(id, bind_group);
        self.filters.insert(id, filter);
        self.views.insert(id, view);
        // The original frees the CPU copy once uploaded.
        if let Some(d) = textures.data_mut(id) {
            d.image = None;
        }
    }

    /// The vehicle shaders' texture bind group for texture ids `ids`
    /// (missing textures sample transparent black, like an empty unit).
    fn ensure_vehicle_textures(&mut self, ids: [u32; 10]) {
        if self.vehicles.bind_groups.contains_key(&ids) {
            return;
        }
        if !self.views.contains_key(&NO_TEXTURE) {
            let (_, view) = self.create_texture(1, 1, &[vec![0; 4]], (true, true, false, false));
            self.views.insert(NO_TEXTURE, view);
        }
        let mut views = Vec::new();
        let mut samplers = Vec::new();
        for id in ids {
            let (view, filter) = match (self.views.get(&id), self.filters.get(&id)) {
                (Some(v), Some(f)) => (v.clone(), *f),
                _ => (self.views[&NO_TEXTURE].clone(), (true, true, false, false)),
            };
            views.push(view);
            samplers.push(self.sampler(filter));
        }
        let mut entries = Vec::new();
        for (i, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: i as u32, resource: wgpu::BindingResource::TextureView(v) });
        }
        for (i, s) in samplers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 10 + i as u32, resource: wgpu::BindingResource::Sampler(s) });
        }
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vehicle textures"), layout: &self.vehicles.texture_layout, entries: &entries });
        self.vehicles.bind_groups.insert(ids, bg);
    }

    fn sampler(&mut self, filter: Filter) -> wgpu::Sampler {
        let device = &self.device;
        self.samplers
            .entry(filter)
            .or_insert_with(|| {
                let (min_nearest, mag_nearest, mips, clamp) = filter;
                let wrap = if clamp { wgpu::AddressMode::ClampToEdge } else { wgpu::AddressMode::Repeat };
                let f = |nearest: bool| if nearest { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear };
                device.create_sampler(&wgpu::SamplerDescriptor {
                    address_mode_u: wrap,
                    address_mode_v: wrap,
                    mag_filter: f(mag_nearest),
                    // GL_LINEAR_MIPMAP_LINEAR for mip-mapped textures.
                    min_filter: if mips { wgpu::FilterMode::Linear } else { f(min_nearest) },
                    mipmap_filter: if mips { wgpu::MipmapFilterMode::Linear } else { wgpu::MipmapFilterMode::Nearest },
                    lod_max_clamp: if mips { 32.0 } else { 0.0 },
                    ..Default::default()
                })
            })
            .clone()
    }

    fn create_texture_bind_group(&mut self, width: u32, height: u32, levels: &[Vec<u8>], filter: Filter) -> wgpu::BindGroup {
        self.create_texture(width, height, levels, filter).0
    }

    /// The bind group sampling texture `id` with `GL_NEAREST` both ways.
    fn ensure_nearest(&mut self, id: u32) {
        if self.nearest.contains_key(&id) {
            return;
        }
        let Some(view) = self.views.get(&id).cloned() else { return };
        let sampler = self.sampler((true, true, false, false));
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        self.nearest.insert(id, bg);
    }

    fn create_texture(&mut self, width: u32, height: u32, levels: &[Vec<u8>], filter: Filter) -> (wgpu::BindGroup, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let (mut w, mut h) = (width, height);
        for (level, px) in levels.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.sampler(filter);
        let device = &self.device;
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        (bg, view)
    }
}

/// `glViewport` for a batch on a `tw` x `th` target. GL counts the origin
/// from the bottom row; texture targets are drawn with y flipped (their
/// first row is GL's bottom row), the main target is not.
fn set_viewport(pass: &mut wgpu::RenderPass<'_>, b: &Batch, tw: u32, th: u32, texture_target: bool) {
    let (x, y) = (b.origin.0.min(tw), b.origin.1.min(th));
    let (w, h) = (b.viewport.0.min(tw - x), b.viewport.1.min(th - y));
    let top = if texture_target { y } else { th - y - h };
    pass.set_viewport(x as f32, top as f32, w.max(1) as f32, h.max(1) as f32, 0.0, 1.0);
}

/// What `AtlasBegin` saves: target, projection, viewport, GL state.
type AtlasSave = (PassTarget, [[f32; 4]; 4], (u32, u32), (u32, u32), GlState);

/// Vertices, indices, uniform blocks, draw runs and passes of one frame.
struct FrameBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    globals: Vec<Globals>,
    batches: Vec<Batch>,
    passes: Vec<PassDesc>,
    /// Chunk textures released once the frame is drawn.
    freed: Vec<u64>,
    target: PassTarget,
    /// The target a chunk texture's `ChunkEnd` returns to.
    chunk_return: PassTarget,
    /// The target bound before `WeatherFxBegin`.
    fx_return: PassTarget,
    /// The seam mask of the quad being pushed.
    tex3: Option<u32>,
    model_uniforms: Vec<model::ModelUniforms>,
    /// The frame's sky renders' uniforms.
    sky: Vec<SkyUniforms>,
    vehicle_uniforms: Vec<vehicle::VehicleUniforms>,
    mvp: [[f32; 4]; 4],
    viewport: (u32, u32),
    /// `glViewport`'s origin (GL window coordinates, bottom-left).
    origin: (u32, u32),
    /// The origin `ChunkEnd` brings back.
    chunk_origin: (u32, u32),
    /// What `AtlasBegin` saved: target, projection, viewport, GL state.
    atlas_stack: Vec<AtlasSave>,
    /// The target `OutlineEnd` returns to.
    outline_return: PassTarget,
    outline_uniforms: Vec<outline::PassUniforms>,
}

impl Default for FrameBuilder {
    fn default() -> Self {
        FrameBuilder {
            vertices: Vec::new(),
            indices: Vec::new(),
            globals: Vec::new(),
            batches: Vec::new(),
            passes: Vec::new(),
            freed: Vec::new(),
            target: PassTarget::Main,
            chunk_return: PassTarget::Main,
            fx_return: PassTarget::Main,
            tex3: None,
            model_uniforms: Vec::new(),
            sky: Vec::new(),
            vehicle_uniforms: Vec::new(),
            mvp: IDENTITY,
            viewport: (1, 1),
            origin: (0, 0),
            chunk_origin: (0, 0),
            atlas_stack: Vec::new(),
            outline_return: PassTarget::Main,
            outline_uniforms: Vec::new(),
        }
    }
}

impl FrameBuilder {
    /// The projection, with y flipped when drawing into a texture.
    fn mvp_for_target(&self) -> [[f32; 4]; 4] {
        let mut m = self.mvp;
        if self.target != PassTarget::Main {
            for col in &mut m {
                col[1] = -col[1];
            }
        }
        m
    }

    fn globals(&mut self, st: &GlState, mvp: [[f32; 4]; 4], clip_z: f32) -> u32 {
        let (alpha_func, alpha_ref) = if st.alpha_test { (st.alpha_func.wrapping_sub(gl::NEVER), st.alpha_ref) } else { (8, 0.0) };
        let g = Globals { mvp, alpha_func, alpha_ref, clip_z, extra: 0.0, screen: [[0.0; 4]; 2], surface: [[0.0; 4]; 5] };
        self.push_globals(g)
    }

    fn push_globals(&mut self, g: Globals) -> u32 {
        if let Some(i) = self.globals.iter().position(|x| bytemuck::bytes_of(x) == bytemuck::bytes_of(&g)) {
            return i as u32;
        }
        self.globals.push(g);
        (self.globals.len() - 1) as u32
    }

    /// A vehicle draw as its own batch.
    fn push_vehicle(&mut self, key: vehicle::VehicleKey, mesh: u64, uniform: u32, textures: [u32; 10], st: &GlState) {
        if self.passes.last().is_none_or(|p| p.target != self.target) {
            let at = self.batches.len();
            self.passes.push(PassDesc { target: self.target, clear: false, mipmaps: false, batches: at..at });
        }
        let at = self.indices.len() as u32;
        self.batches.push(Batch {
            key: st.pipe_key(),
            texture: None,
            tex2: Tex2::None,
            tex3: None,
            model: None,
            vehicle: Some((key, mesh, uniform, textures)),
            outline: None,
            stencil_ref: st.stencil_ref,
            scissor: st.scissor_box(),
            globals: 0,
            viewport: self.viewport,
            origin: self.origin,
            indices: at..at,
        });
        self.passes.last_mut().expect("pass").batches.end = self.batches.len();
    }

    /// An outline draw as its own batch.
    /// An outline op drawing a quad of the frame's vertices.
    fn push_outline_quad(&mut self, op: outline::Op, st: &GlState, verts: [Vertex; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&verts);
        let start = self.indices.len() as u32;
        self.indices.extend_from_slice(&quad_indices(base, verts[0].color, verts[2].color));
        let end = self.indices.len() as u32;
        self.push_outline(op, st);
        self.batches.last_mut().expect("batch").indices = start..end;
    }

    fn push_outline(&mut self, op: outline::Op, st: &GlState) {
        if self.passes.last().is_none_or(|p| p.target != self.target) {
            let at = self.batches.len();
            self.passes.push(PassDesc { target: self.target, clear: false, mipmaps: false, batches: at..at });
        }
        let at = self.indices.len() as u32;
        self.batches.push(Batch {
            key: st.pipe_key(),
            texture: None,
            tex2: Tex2::None,
            tex3: None,
            model: None,
            vehicle: None,
            outline: Some(op),
            stencil_ref: st.stencil_ref,
            scissor: st.scissor_box(),
            globals: 0,
            viewport: self.viewport,
            origin: self.origin,
            indices: at..at,
        });
        self.passes.last_mut().expect("pass").batches.end = self.batches.len();
    }

    /// A model draw as its own batch.
    fn push_model(&mut self, key: model::ModelKey, mesh: u64, uniform: u32, texture: Option<u32>, st: &GlState) {
        if self.passes.last().is_none_or(|p| p.target != self.target) {
            let at = self.batches.len();
            self.passes.push(PassDesc { target: self.target, clear: false, mipmaps: false, batches: at..at });
        }
        let at = self.indices.len() as u32;
        self.batches.push(Batch {
            key: st.pipe_key(),
            texture,
            tex2: Tex2::None,
            tex3: None,
            model: Some((key, mesh, uniform)),
            vehicle: None,
            outline: None,
            stencil_ref: st.stencil_ref,
            scissor: st.scissor_box(),
            globals: 0,
            viewport: self.viewport,
            origin: self.origin,
            indices: at..at,
        });
        self.passes.last_mut().expect("pass").batches.end = self.batches.len();
    }

    fn push(&mut self, key: PipeKey, texture: Option<u32>, tex2: Tex2, st: &GlState, globals: u32, verts: [Vertex; 4]) {
        let split02 = verts[0].color == verts[2].color;
        self.push_split(key, texture, tex2, st, globals, verts, split02);
    }

    /// [`FrameBuilder::push`] with the quad's diagonal given: 0-2 when
    /// `split02`, else 1-3.
    #[allow(clippy::too_many_arguments)]
    fn push_split(&mut self, key: PipeKey, texture: Option<u32>, tex2: Tex2, st: &GlState, globals: u32, verts: [Vertex; 4], split02: bool) {
        if self.passes.last().is_none_or(|p| p.target != self.target) {
            let at = self.batches.len();
            self.passes.push(PassDesc { target: self.target, clear: false, mipmaps: false, batches: at..at });
        }
        let (stencil_ref, scissor) = (st.stencil_ref, st.scissor_box());
        let viewport = self.viewport;
        let origin = self.origin;
        let tex3 = self.tex3;
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&verts);
        let start = self.indices.len() as u32;
        self.indices.extend_from_slice(&if split02 { quad_indices(base, 0, 0) } else { quad_indices(base, 0, 1) });
        let end = self.indices.len() as u32;
        let pass = self.passes.last_mut().expect("pass");
        match self.batches.last_mut() {
            Some(b)
                if pass.batches.end > pass.batches.start
                    && b.key == key
                    && b.texture == texture
                    && b.tex2 == tex2
                    && b.tex3 == tex3
                    && b.model.is_none()
                    && b.outline.is_none()
                    && b.stencil_ref == stencil_ref
                    && b.scissor == scissor
                    && b.globals == globals
                    && b.viewport == viewport
                    && b.origin == origin
                    && b.indices.end == start =>
            {
                b.indices.end = end
            }
            _ => {
                self.batches.push(Batch { model: None, vehicle: None, outline: None, key, texture, tex2, tex3, stencil_ref, scissor, globals, viewport, origin, indices: start..end });
                pass.batches.end = self.batches.len();
            }
        }
    }

    /// `glClear` through the render queue: a viewport quad that honours the
    /// scissor box and the colour/stencil/depth write masks. Depth clears
    /// write the quad's depth, 1.
    fn clear(&mut self, st: &GlState, g: u32, bit: u32, color: [f64; 4]) {
        let c = pz_render::color_to_abgr(color[0] as f32, color[1] as f32, color[2] as f32, color[3] as f32);
        let verts = [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(x, y)| Vertex { pos: [x, y], uv: [0.0; 2], color: c, uv1: [0.0; 2], uv2: [0.0; 2], depth: [0.0; 2] });
        let key = PipeKey { target: self.target.kind(), ..st.clear_key(bit) };
        let stencil = if bit == gl::STENCIL_BUFFER_BIT { st.clear_stencil } else { st.stencil_ref };
        let mut st2 = st.clone();
        st2.stencil_ref = stencil;
        self.push(key, None, Tex2::None, &st2, g, verts);
    }
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GpuError> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("pz-gpu"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|e| GpuError::Device(e.to_string()))
}

fn headless_texture(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn depth_stencil_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth stencil"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: state::DEPTH_STENCIL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// GL scissor boxes are clamped to the framebuffer.
fn clip_scissor(s: [u32; 4], w: u32, h: u32) -> [u32; 4] {
    let x0 = s[0].min(w);
    let y0 = s[1].min(h);
    let x1 = s[0].saturating_add(s[2]).min(w);
    let y1 = s[1].saturating_add(s[3]).min(h);
    [x0, y0, x1 - x0, y1 - y0]
}

/// The four corners of a sprite as `RingBuffer.add` writes them: texture
/// coordinates mirrored left-right for flipped sprites, zero without a
/// texture.
pub fn sprite_vertices(s: &pz_render::Sprite) -> [Vertex; 4] {
    let flip = [1usize, 0, 3, 2];
    std::array::from_fn(|i| {
        let ui = if s.flipped { flip[i] } else { i };
        let uv = if s.texture.is_some() { [s.u[ui], s.v[i]] } else { [0.0, 0.0] };
        Vertex { pos: [s.x[i], s.y[i]], uv, color: s.color[i], uv1: [0.0; 2], uv2: [0.0; 2], depth: [0.0; 2] }
    })
}

/// `RingBuffer.add`'s index order: split on the 0-2 diagonal when corners
/// 0 and 2 share a colour, else on 1-3.
pub fn quad_indices(base: u32, c0: u32, c2: u32) -> [u32; 6] {
    if c0 == c2 {
        [base, base + 1, base + 2, base, base + 2, base + 3]
    } else {
        [base + 1, base + 2, base + 3, base + 1, base + 3, base]
    }
}

#[cfg(test)]
mod tests;
