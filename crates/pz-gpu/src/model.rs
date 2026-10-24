//! The 3D model pass: `basicEffect` pipelines, uploaded meshes and the
//! per-draw uniform block.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use pz_render::model::{MeshData, MeshKind, ModelDraw, ModelPass, MAX_BONES};

use crate::state::{self, TargetKind};

/// The `Model` uniform block of model.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ModelUniforms {
    pub mvp: [[f32; 4]; 4],
    pub transform: [[f32; 4]; 4],
    pub palette: [[[f32; 4]; 4]; MAX_BONES],
    pub light_dir: [[f32; 4]; 5],
    pub light_col: [[f32; 4]; 5],
    pub ambient: [f32; 4],
    pub tint: [f32; 4],
    pub params: [f32; 4],
    pub extra: [f32; 4],
}

/// Uniform blocks are bound at 256-byte aligned offsets.
pub const MODEL_STRIDE: u64 = (std::mem::size_of::<ModelUniforms>() as u64).div_ceil(256) * 256;

impl ModelUniforms {
    pub fn new(d: &ModelDraw, flip_y: bool) -> Self {
        let mut u = ModelUniforms::zeroed();
        u.mvp = d.mvp;
        u.transform = d.transform;
        for (dst, src) in u.palette.iter_mut().zip(d.palette.iter()) {
            *dst = *src;
        }
        for i in 0..5 {
            let l = d.lights[i];
            u.light_dir[i] = [l.direction[0], l.direction[1], l.direction[2], 0.0];
            u.light_col[i] = [l.colour[0], l.colour[1], l.colour[2], 0.0];
        }
        u.ambient = [d.ambient[0], d.ambient[1], d.ambient[2], 0.0];
        u.tint = [d.tint[0], d.tint[1], d.tint[2], 0.0];
        u.params = [d.target_depth, d.final_scale, d.hue, d.alpha];
        u.extra = [d.uv_scale[0], d.uv_scale[1], if flip_y { -1.0 } else { 1.0 }, 0.0];
        u
    }
}

/// Everything baked into a model pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelKey {
    pub kind: MeshKind,
    pub pass: ModelPass,
    pub target: TargetKind,
}

/// An uploaded mesh.
pub struct GpuMesh {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub count: u32,
}

pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, mesh: &MeshData) -> GpuMesh {
    let buf = |data: &[u8], usage| {
        let size = (data.len() as u64).max(4).div_ceil(4) * 4;
        let b = device.create_buffer(&wgpu::BufferDescriptor { label: Some("mesh"), size, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut padded = data.to_vec();
        padded.resize(size as usize, 0);
        queue.write_buffer(&b, 0, &padded);
        b
    };
    GpuMesh {
        vertices: buf(bytemuck::cast_slice(&mesh.vertices), wgpu::BufferUsages::VERTEX),
        indices: buf(bytemuck::cast_slice(&mesh.indices), wgpu::BufferUsages::INDEX),
        count: mesh.indices.len() as u32,
    }
}

/// Model pipelines by key.
pub struct ModelPipelines {
    pub module: wgpu::ShaderModule,
    pub uniform_layout: wgpu::BindGroupLayout,
    pub layout: wgpu::PipelineLayout,
    pub pipelines: HashMap<ModelKey, wgpu::RenderPipeline>,
    pub meshes: HashMap<u64, GpuMesh>,
}

impl ModelPipelines {
    pub fn new(device: &wgpu::Device, texture_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("basicEffect"), source: wgpu::ShaderSource::Wgsl(include_str!("model.wgsl").into()) });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ModelUniforms>() as u64),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("model"), bind_group_layouts: &[Some(&uniform_layout), Some(texture_layout)], immediate_size: 0 });
        ModelPipelines { module, uniform_layout, layout, pipelines: HashMap::new(), meshes: HashMap::new() }
    }

    pub fn ensure(&mut self, device: &wgpu::Device, main_format: wgpu::TextureFormat, key: ModelKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let (blend, depth_compare, cull) = match key.pass {
            ModelPass::Character { cull } => (
                wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add } },
                wgpu::CompareFunction::Less,
                cull,
            ),
            ModelPass::Item { cull } => (
                wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::OneMinusDstAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } },
                wgpu::CompareFunction::LessEqual,
                cull,
            ),
        };
        // glCullFace(GL_FRONT) with counter-clockwise front faces; drawing
        // into a texture flips y, which flips the winding.
        let flipped = key.target != TargetKind::Main;
        let cull_mode = cull.then_some(if flipped { wgpu::Face::Back } else { wgpu::Face::Front });
        let (vs, attrs, stride): (&str, &[wgpu::VertexAttribute], u64) = match key.kind {
            MeshKind::Static => ("vs_static", &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2], 32),
            MeshKind::Skinned => ("vs_skinned", &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2], 64),
        };
        let (format, depth_format) = match key.target {
            TargetKind::Main => (main_format, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Offscreen => (state::OFFSCREEN_COLOR_FORMAT, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Chunk => (state::CHUNK_COLOR_FORMAT, state::CHUNK_DEPTH_FORMAT),
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("basicEffect"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: attrs })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(depth_compare),
                stencil: wgpu::StencilState { front: wgpu::StencilFaceState::IGNORE, back: wgpu::StencilFaceState::IGNORE, read_mask: 0, write_mask: 0 },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("fs_model"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, p);
    }
}
