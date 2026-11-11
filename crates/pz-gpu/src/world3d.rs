//! Perspective 3D scene quads (`DrawCmd::Scene3DBegin` / `Quad3D`).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use crate::state::{self, TargetKind};

/// One corner of a 3D quad.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct World3dVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: u32,
}

/// Uniform blocks are bound at 256-byte aligned offsets.
pub const STRIDE: u64 = 256;

/// The two triangles of a quad as a fan 0-1-2, 0-2-3.
pub fn fan(v: [World3dVertex; 4]) -> [World3dVertex; 6] {
    [v[0], v[1], v[2], v[0], v[2], v[3]]
}

pub struct World3dPipelines {
    module: wgpu::ShaderModule,
    pub uniform_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    /// Nearest-magnification sampler, bound at group 2 for every quad.
    pub sampler_bind: wgpu::BindGroup,
    pub pipelines: HashMap<TargetKind, wgpu::RenderPipeline>,
}

impl World3dPipelines {
    pub fn new(device: &wgpu::Device, texture_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("world3d"), source: wgpu::ShaderSource::Wgsl(include_str!("world3d.wgsl").into()) });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world3d"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(64) },
                count: None,
            }],
        });
        let sampler_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world3d sampler"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("world3d nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let sampler_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world3d sampler"),
            layout: &sampler_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world3d"),
            bind_group_layouts: &[Some(&uniform_layout), Some(texture_layout), Some(&sampler_layout)],
            immediate_size: 0,
        });
        World3dPipelines { module, uniform_layout, layout, sampler_bind, pipelines: HashMap::new() }
    }

    pub fn ensure(&mut self, device: &wgpu::Device, main_format: wgpu::TextureFormat, target: TargetKind) {
        if self.pipelines.contains_key(&target) {
            return;
        }
        let (format, depth_format) = match target {
            TargetKind::Main => (main_format, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Offscreen => (state::OFFSCREEN_COLOR_FORMAT, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Chunk => (state::CHUNK_COLOR_FORMAT, state::CHUNK_DEPTH_FORMAT),
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world3d"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<World3dVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4],
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState { front: wgpu::StencilFaceState::IGNORE, back: wgpu::StencilFaceState::IGNORE, read_mask: 0, write_mask: 0 },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(target, p);
    }
}
