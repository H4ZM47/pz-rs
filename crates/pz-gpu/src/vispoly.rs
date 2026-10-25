//! `VisibilityPolygon2.Drawer`'s blur target (`visBlur`, `GL_R8`, and
//! `visBlurDepth`, `GL_DEPTH_COMPONENT16`, both as large as the next powers
//! of two of the blur size) and the pipelines of its shaders
//! (vispoly.wgsl: `visPolygon`, `visibilityBlur`).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use crate::state::{self, TargetKind};

/// `visBlur`'s format: `GL_R8` (sampled as `(r, 0, 0, 1)`).
pub const BLUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// `visBlurDepth`'s format.
pub const BLUR_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// vispoly.wgsl's `Vis` block.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub mvp: [[f32; 4]; 4],
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub d: [f32; 4],
}

/// Uniform blocks are bound at 256-byte aligned offsets.
pub const STRIDE: u64 = 256;

/// One vision draw of a batch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// `renderPolygons`: vertices `start..end` of the frame's polygon
    /// vertices into the blur target.
    Polygon { start: u32, end: u32, uniform: u32 },
    /// `renderToScreen`: the blur pass into a target of the given kind
    /// over viewport `x, y, w, h`.
    Blur {
        uniform: u32,
        target: TargetKind,
        viewport: [f32; 4],
    },
}

/// The blur textures.
pub struct VisTargets {
    /// The blur size (`Core.width / downscale`, ...).
    pub size: [u32; 2],
    pub hw: [u32; 2],
    pub color: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    /// `visBlur` sampled GL_LINEAR with GL_REPEAT.
    pub color_bind: wgpu::BindGroup,
    pub depth_bind: wgpu::BindGroup,
}

pub struct VisPipelines {
    module: wgpu::ShaderModule,
    pub uniform_layout: wgpu::BindGroupLayout,
    polygon_layout: wgpu::PipelineLayout,
    blur_layout: wgpu::PipelineLayout,
    pub polygon: Option<wgpu::RenderPipeline>,
    pub blur: HashMap<(TargetKind, wgpu::TextureFormat), wgpu::RenderPipeline>,
    pub targets: Option<VisTargets>,
}

fn depth_state(format: wgpu::TextureFormat, write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: wgpu::StencilState {
            front: wgpu::StencilFaceState::IGNORE,
            back: wgpu::StencilFaceState::IGNORE,
            read_mask: 0,
            write_mask: 0,
        },
        bias: wgpu::DepthBiasState::default(),
    }
}

impl VisPipelines {
    pub fn new(
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        depth_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("visibility"),
            source: wgpu::ShaderSource::Wgsl(include_str!("vispoly.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("visibility"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                },
                count: None,
            }],
        });
        let polygon_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("visPolygon"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });
        let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("visibilityBlur"),
            bind_group_layouts: &[
                Some(&uniform_layout),
                Some(texture_layout),
                Some(depth_layout),
            ],
            immediate_size: 0,
        });
        VisPipelines {
            module,
            uniform_layout,
            polygon_layout,
            blur_layout,
            polygon: None,
            blur: HashMap::new(),
            targets: None,
        }
    }

    /// `visPolygon` into the blur target: blending (ONE, ZERO), depth
    /// test `LEQUAL` with writes, no culling.
    pub fn ensure_polygon(&mut self, device: &wgpu::Device) {
        if self.polygon.is_some() {
            return;
        }
        self.polygon = Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("visPolygon"),
            layout: Some(&self.polygon_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_polygon"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32],
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(depth_state(BLUR_DEPTH_FORMAT, true)),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("fs_polygon"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: BLUR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        }));
    }

    /// `visibilityBlur` into a target of the given kind: blending
    /// (SRC_ALPHA, ONE_MINUS_SRC_ALPHA), depth test `LEQUAL` without
    /// writes.
    pub fn ensure_blur(
        &mut self,
        device: &wgpu::Device,
        target: TargetKind,
        main_format: wgpu::TextureFormat,
    ) {
        let (format, depth) = match target {
            TargetKind::Main => (main_format, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Offscreen => (state::OFFSCREEN_COLOR_FORMAT, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Chunk => (state::CHUNK_COLOR_FORMAT, state::CHUNK_DEPTH_FORMAT),
        };
        if self.blur.contains_key(&(target, format)) {
            return;
        }
        let c = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("visibilityBlur"),
            layout: Some(&self.blur_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_blur"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(depth, false)),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("fs_blur"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState { color: c, alpha: c }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.blur.insert((target, format), p);
    }

    /// `initBlur`: (re)creates the blur textures for a blur size of
    /// `size`.
    pub fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        depth_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
    ) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let hw = [
            size[0].max(1).next_power_of_two(),
            size[1].max(1).next_power_of_two(),
        ];
        let make = |label: &str, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: hw[0],
                        height: hw[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = make("visBlur", BLUR_FORMAT);
        let depth = make("visBlurDepth", BLUR_DEPTH_FORMAT);
        let color_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("visBlur"),
            layout: texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let depth_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("visBlurDepth"),
            layout: depth_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&depth),
            }],
        });
        self.targets = Some(VisTargets {
            size,
            hw,
            color,
            depth,
            color_bind,
            depth_bind,
        });
    }
}

/// The blur pass's uniforms (`renderToScreen`).
pub fn blur_uniforms(
    d: &pz_render::VisibilityPolygonDraw,
    hw: [u32; 2],
    main_height: Option<u32>,
) -> Uniforms {
    let [left, top, w, h] = d.screen;
    Uniforms {
        mvp: [[0.0; 4]; 4],
        a: [w as f32, h as f32, left as f32, top as f32],
        b: [
            d.display_size[0],
            d.display_size[1],
            d.blur_size[0] as f32,
            d.blur_size[1] as f32,
        ],
        c: [hw[0] as f32, hw[1] as f32, d.zoom, d.opacity_scale as f32],
        d: [main_height.map_or(0.0, |h| h as f32), 0.0, 0.0, 0.0],
    }
}
