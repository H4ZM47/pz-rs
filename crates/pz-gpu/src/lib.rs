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

mod gl;
mod state;

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use pz_render::{DrawCmd, DrawList, Texture, TextureManager};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub use state::GlState;
use state::PipeKey;

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

#[derive(Debug)]
pub enum GpuError {
    NoAdapter(String),
    Device(String),
    Surface(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter(e) => write!(f, "no graphics adapter: {e}"),
            GpuError::Device(e) => write!(f, "device: {e}"),
            GpuError::Surface(e) => write!(f, "surface: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// Vertex as `SpriteRenderer` packs it (36 bytes): position, texture 0
/// coordinates, ABGR colour, texture 1 and 2 coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
    pub uv1: [f32; 2],
    pub uv2: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    mvp: [[f32; 4]; 4],
    alpha_func: u32,
    alpha_ref: f32,
    _pad: [u32; 2],
}

/// Uniform blocks are bound at 256-byte aligned offsets.
const GLOBALS_STRIDE: u64 = 256;

const IDENTITY: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

enum Target {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Headless { texture: wgpu::Texture },
}

/// A run of indices drawn with one state.
struct Batch {
    key: PipeKey,
    texture: Option<u32>,
    stencil_ref: u32,
    scissor: Option<[u32; 4]>,
    globals: u32,
    indices: std::ops::Range<u32>,
}

/// Sampler choice: min nearest, mag nearest, mip-mapped, clamp to edge.
type Filter = (bool, bool, bool, bool);

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: Target,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    stencil: wgpu::TextureView,
    shader: wgpu::ShaderModule,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<PipeKey, wgpu::RenderPipeline>,
    textures: HashMap<u32, wgpu::BindGroup>,
    samplers: HashMap<Filter, wgpu::Sampler>,
    /// Bound for untextured quads; samples as 1.0 like `useTexture = 0`.
    white: Option<wgpu::BindGroup>,
    /// GL state carried across frames, like `GLState`.
    pub state: GlState,
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("default"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let stencil = stencil_view(&device, width, height);
        let mut gpu = Gpu {
            device,
            queue,
            target,
            format,
            width,
            height,
            stencil,
            shader,
            globals_layout,
            texture_layout,
            pipeline_layout,
            pipelines: HashMap::new(),
            textures: HashMap::new(),
            samplers: HashMap::new(),
            white: None,
            state: GlState::default(),
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
        self.stencil = stencil_view(&self.device, width, height);
    }

    /// `Core.OptionVSync`.
    pub fn set_vsync(&mut self, on: bool) {
        if let Target::Surface { surface, config } = &mut self.target {
            config.present_mode = if on { wgpu::PresentMode::Fifo } else { wgpu::PresentMode::AutoNoVsync };
            surface.configure(&self.device, config);
        }
    }

    /// Draws one frame and presents it.
    pub fn render_frame(&mut self, segments: &[(FrameParams, &DrawList)], textures: &mut TextureManager) {
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

    /// The headless target's pixels, RGBA, top row first.
    pub fn read_pixels(&self) -> Vec<u8> {
        let Target::Headless { texture } = &self.target else { return Vec::new() };
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
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let Ok(data) = slice.get_mapped_range() else { return Vec::new() };
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        out
    }

    // ---- frame building ---------------------------------------------------

    fn draw(&mut self, segments: &[(FrameParams, &DrawList)], textures: &mut TextureManager, view: &wgpu::TextureView) {
        let mut f = FrameBuilder::default();
        let mut viewports = Vec::new();
        let mut clear_first: Option<[f64; 4]> = None;
        for (i, (params, list)) in segments.iter().enumerate() {
            let first = f.batches.len();
            if let Some(c) = params.clear {
                if i == 0 {
                    clear_first = Some(c);
                } else {
                    let g = f.globals(&self.state, IDENTITY);
                    f.clear(&self.state, g, gl::COLOR_BUFFER_BIT, c);
                }
            }
            // `DoStartFrameStuff`: blending on, new projection and viewport.
            self.state.blend = true;
            let (w, h) = (params.width as f32 * params.zoom, params.height as f32 * params.zoom);
            let mvp = gl::ortho2d(0.0, w, h, 0.0);
            for cmd in &list.cmds {
                self.record(&mut f, cmd, mvp);
            }
            viewports.push((first..f.batches.len(), params.width, params.height));
        }
        for i in 0..f.batches.len() {
            if let Some(id) = f.batches[i].texture {
                self.ensure_texture(id, textures);
            }
            self.ensure_pipeline(f.batches[i].key);
        }
        let FrameBuilder { mut vertices, mut indices, mut globals, batches } = f;
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
        {
            let load = match clear_first {
                Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: c[3] }),
                None => wgpu::LoadOp::Load,
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.stencil,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            let white = self.white.as_ref().expect("white texture");
            for (range, vw, vh) in viewports {
                pass.set_viewport(0.0, 0.0, vw.min(self.width) as f32, vh.min(self.height) as f32, 0.0, 1.0);
                for b in &batches[range] {
                    let [sx, sy, sw, sh] = match b.scissor {
                        Some(s) => clip_scissor(s, self.width, self.height),
                        None => [0, 0, self.width, self.height],
                    };
                    if sw == 0 || sh == 0 {
                        continue;
                    }
                    pass.set_pipeline(&self.pipelines[&b.key]);
                    pass.set_bind_group(0, &gbind, &[b.globals * GLOBALS_STRIDE as u32]);
                    let tex = b.texture.and_then(|id| self.textures.get(&id)).unwrap_or(white);
                    pass.set_bind_group(1, tex, &[]);
                    pass.set_stencil_reference(b.stencil_ref);
                    pass.set_scissor_rect(sx, sy, sw, sh);
                    pass.draw_indexed(b.indices.clone(), 0, 0..1);
                }
            }
        }
        self.queue.submit([enc.finish()]);
    }

    fn record(&mut self, f: &mut FrameBuilder, cmd: &DrawCmd, mvp: [[f32; 4]; 4]) {
        match cmd {
            DrawCmd::Sprite(s) => {
                let g = f.globals(&self.state, mvp);
                f.quad(&self.state, g, s.texture.as_ref(), sprite_vertices(s));
            }
            DrawCmd::Quad { texture, x, y, w, h, color } => {
                let mut dl = DrawList::default();
                dl.render(texture.as_ref(), *x, *y, *w, *h, *color);
                for c in &dl.cmds {
                    self.record(f, c, mvp);
                }
            }
            DrawCmd::Clear(mask) => {
                let g = f.globals(&self.state, IDENTITY);
                for bit in [gl::STENCIL_BUFFER_BIT, gl::COLOR_BUFFER_BIT] {
                    if mask & bit != 0 {
                        f.clear(&self.state, g, bit, self.state.clear_color);
                    }
                }
            }
            other => self.state.apply(other),
        }
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
            let p = state::create_pipeline(&self.device, &self.pipeline_layout, &self.shader, self.format, key);
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
        let bind_group = self.create_texture_bind_group(levels[0].width as u32, levels[0].height as u32, &pixels, filter);
        self.textures.insert(id, bind_group);
        // The original frees the CPU copy once uploaded.
        if let Some(d) = textures.data_mut(id) {
            d.image = None;
        }
    }

    fn create_texture_bind_group(&mut self, width: u32, height: u32, levels: &[Vec<u8>], filter: Filter) -> wgpu::BindGroup {
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
        let device = &self.device;
        let sampler = self.samplers.entry(filter).or_insert_with(|| {
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
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }
}

/// Vertices, indices, uniform blocks and draw runs of one frame.
#[derive(Default)]
struct FrameBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    globals: Vec<Globals>,
    batches: Vec<Batch>,
}

impl FrameBuilder {
    fn globals(&mut self, st: &GlState, mvp: [[f32; 4]; 4]) -> u32 {
        let (alpha_func, alpha_ref) = if st.alpha_test { (st.alpha_func.wrapping_sub(gl::NEVER), st.alpha_ref) } else { (8, 0.0) };
        let g = Globals { mvp, alpha_func, alpha_ref, _pad: [0; 2] };
        if let Some(i) = self.globals.iter().position(|x| bytemuck::bytes_of(x) == bytemuck::bytes_of(&g)) {
            return i as u32;
        }
        self.globals.push(g);
        (self.globals.len() - 1) as u32
    }

    fn push(&mut self, key: PipeKey, texture: Option<u32>, stencil_ref: u32, scissor: Option<[u32; 4]>, globals: u32, verts: [Vertex; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&verts);
        let start = self.indices.len() as u32;
        self.indices.extend_from_slice(&quad_indices(base, verts[0].color, verts[2].color));
        let end = self.indices.len() as u32;
        match self.batches.last_mut() {
            Some(b) if b.key == key && b.texture == texture && b.stencil_ref == stencil_ref && b.scissor == scissor && b.globals == globals && b.indices.end == start => {
                b.indices.end = end
            }
            _ => self.batches.push(Batch { key, texture, stencil_ref, scissor, globals, indices: start..end }),
        }
    }

    fn quad(&mut self, st: &GlState, g: u32, tex: Option<&Texture>, verts: [Vertex; 4]) {
        self.push(st.pipe_key(), tex.and_then(|t| t.id), st.stencil_ref, st.scissor_box(), g, verts);
    }

    /// `glClear` through the render queue: a viewport quad that honours the
    /// scissor box and the colour/stencil write masks.
    fn clear(&mut self, st: &GlState, g: u32, bit: u32, color: [f64; 4]) {
        let c = pz_render::color_to_abgr(color[0] as f32, color[1] as f32, color[2] as f32, color[3] as f32);
        let verts = [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(x, y)| Vertex { pos: [x, y], uv: [0.0; 2], color: c, uv1: [0.0; 2], uv2: [0.0; 2] });
        self.push(st.clear_key(bit), None, st.clear_stencil, st.scissor_box(), g, verts);
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

fn stencil_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("stencil"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: state::STENCIL_FORMAT,
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
        Vertex { pos: [s.x[i], s.y[i]], uv, color: s.color[i], uv1: [0.0; 2], uv2: [0.0; 2] }
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
