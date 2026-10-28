//! The vehicle model pass (`Model.DrawVehicle` with the vehicle shaders):
//! one pipeline per shader variant, mesh layout and target, a uniform
//! block per draw and a bind group of the ten textures the shaders sample.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use pz_render::model::{MeshKind, VehicleDraw, VehicleShader, MAX_BONES};

use crate::state::{self, TargetKind};

/// The `Vehicle` uniform block of vehicle.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VehicleUniforms {
    pub mvp: [[f32; 4]; 4],
    pub transform: [[f32; 4]; 4],
    pub palette: [[[f32; 4]; 4]; MAX_BONES],
    pub light_dir: [[f32; 4]; 5],
    pub light_col: [[f32; 4]; 5],
    pub ambient: [f32; 4],
    pub tint: [f32; 4],
    pub params: [f32; 4],
    pub paint: [f32; 4],
    pub reflection: [f32; 4],
    pub en: [[f32; 4]; 48],
}

/// Uniform blocks are bound at 256-byte aligned offsets.
pub const VEHICLE_STRIDE: u64 = (std::mem::size_of::<VehicleUniforms>() as u64).div_ceil(256) * 256;

impl VehicleUniforms {
    pub fn new(d: &VehicleDraw, flip_y: bool) -> Self {
        let b = &d.base;
        let mut u = VehicleUniforms::zeroed();
        u.mvp = b.mvp;
        u.transform = b.transform;
        for (dst, src) in u.palette.iter_mut().zip(b.palette.iter()) {
            *dst = *src;
        }
        for i in 0..5 {
            let l = b.lights[i];
            u.light_dir[i] = [l.direction[0], l.direction[1], l.direction[2], 0.0];
            u.light_col[i] = [l.colour[0], l.colour[1], l.colour[2], 0.0];
        }
        u.ambient = [b.ambient[0], b.ambient[1], b.ambient[2], 0.0];
        u.tint = [b.tint[0], b.tint[1], b.tint[2], 0.0];
        let vu = &d.uniforms;
        u.params = [
            b.target_depth,
            vu.texture_rust_a,
            if flip_y { -1.0 } else { 1.0 },
            0.0,
        ];
        u.paint = vu.paint_color;
        u.reflection = [vu.reflection[0], vu.reflection[1], vu.reflection[2], 0.0];
        let mats = [
            &vu.texture_uninstall,
            &vu.texture_lights_enables,
            &vu.texture_damage1_enables,
            &vu.texture_damage2_enables,
            &vu.matrix_blood1_enables,
            &vu.matrix_blood2_enables,
        ];
        for (m, pair) in mats.iter().enumerate() {
            for (k, arr) in pair.iter().enumerate() {
                for r in 0..4 {
                    u.en[(m * 2 + k) * 4 + r] =
                        [arr[r * 4], arr[r * 4 + 1], arr[r * 4 + 2], arr[r * 4 + 3]];
                }
            }
        }
        u
    }
}

/// Everything baked into a vehicle pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleKey {
    pub kind: MeshKind,
    pub shader: VehicleShader,
    pub reflect: bool,
    /// `invertX`: back faces culled instead of front faces.
    pub invert_x: bool,
    pub target: TargetKind,
}

/// Vehicle shader modules, layouts and pipelines.
pub struct VehiclePipelines {
    pub modules: HashMap<(VehicleShader, bool), wgpu::ShaderModule>,
    pub uniform_layout: wgpu::BindGroupLayout,
    pub texture_layout: wgpu::BindGroupLayout,
    pub layout: wgpu::PipelineLayout,
    pub pipelines: HashMap<VehicleKey, wgpu::RenderPipeline>,
    /// Texture bind groups by the texture ids they sample (`u32::MAX` =
    /// none).
    pub bind_groups: HashMap<[u32; 10], wgpu::BindGroup>,
    /// Meshes in the vehicle layouts, by mesh id.
    pub meshes: HashMap<u64, crate::model::GpuMesh>,
}

fn shader_index(s: VehicleShader) -> u32 {
    match s {
        VehicleShader::Vehicle => 0,
        VehicleShader::MultiUv => 1,
        VehicleShader::NoRandomMultiUv => 2,
        VehicleShader::Wheel => 3,
    }
}

impl VehiclePipelines {
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vehicle"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<VehicleUniforms>() as u64
                    ),
                },
                count: None,
            }],
        });
        let mut entries = Vec::new();
        for i in 0..10 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        for i in 10..20 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vehicle textures"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vehicle"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        VehiclePipelines {
            modules: HashMap::new(),
            uniform_layout,
            texture_layout,
            layout,
            pipelines: HashMap::new(),
            bind_groups: HashMap::new(),
            meshes: HashMap::new(),
        }
    }

    pub fn ensure(
        &mut self,
        device: &wgpu::Device,
        main_format: wgpu::TextureFormat,
        key: VehicleKey,
    ) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let module = self
            .modules
            .entry((key.shader, key.reflect))
            .or_insert_with(|| {
                let src = format!(
                    "const SHADER: u32 = {}u;\nconst REFLECT: bool = {};\n{}",
                    shader_index(key.shader),
                    key.reflect,
                    include_str!("vehicle.wgsl")
                );
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("vehicle"),
                    source: wgpu::ShaderSource::Wgsl(src.into()),
                })
            });
        let alpha = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let blend = wgpu::BlendState {
            color: alpha,
            alpha,
        };
        // glCullFace(invertX ? GL_BACK : GL_FRONT) with counter-clockwise
        // front faces; drawing into a texture flips y, which flips the
        // winding.
        let flipped = key.target != TargetKind::Main;
        let cull_front = !key.invert_x;
        let cull_mode = Some(if cull_front != flipped {
            wgpu::Face::Front
        } else {
            wgpu::Face::Back
        });
        let (vs, attrs, stride): (&str, &[wgpu::VertexAttribute], u64) = match key.kind {
            MeshKind::StaticUv2 | MeshKind::Static => (
                "vs_static",
                &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x2],
                40,
            ),
            MeshKind::SkinnedUv2 | MeshKind::Skinned => (
                "vs_skinned",
                &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2, 5 => Float32x2],
                72,
            ),
        };
        let (format, depth_format) = match key.target {
            TargetKind::Main => (main_format, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Offscreen => (state::OFFSCREEN_COLOR_FORMAT, state::DEPTH_STENCIL_FORMAT),
            TargetKind::Chunk => (state::CHUNK_COLOR_FORMAT, state::CHUNK_DEPTH_FORMAT),
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vehicle"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState::IGNORE,
                    back: wgpu::StencilFaceState::IGNORE,
                    read_mask: 0,
                    write_mask: 0,
                },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_vehicle"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, p);
    }
}

/// The mesh layout a vehicle mesh is uploaded with: the 1-UV layouts
/// are widened with a zero second UV set.
pub fn vehicle_vertices(kind: MeshKind, vertices: &[f32]) -> Vec<f32> {
    match kind {
        MeshKind::StaticUv2 | MeshKind::SkinnedUv2 => vertices.to_vec(),
        MeshKind::Static | MeshKind::Skinned => {
            let s = kind.stride();
            let mut v = Vec::with_capacity(vertices.len() / s * (s + 2));
            for c in vertices.chunks_exact(s) {
                v.extend_from_slice(c);
                v.extend_from_slice(&[0.0, 0.0]);
            }
            v
        }
    }
}

/// Uploads a mesh in its vehicle layout.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mesh: &pz_render::model::MeshData,
) -> crate::model::GpuMesh {
    let vertices = vehicle_vertices(mesh.kind, &mesh.vertices);
    let kind = match mesh.kind {
        MeshKind::Static | MeshKind::StaticUv2 => MeshKind::StaticUv2,
        MeshKind::Skinned | MeshKind::SkinnedUv2 => MeshKind::SkinnedUv2,
    };
    let m = pz_render::model::MeshData {
        id: mesh.id,
        kind,
        vertices,
        indices: mesh.indices.clone(),
    };
    crate::model::upload(device, queue, &m)
}

/// The pipeline layout of a mesh kind (the 1-UV kinds are widened).
pub fn layout_kind(kind: MeshKind) -> MeshKind {
    match kind {
        MeshKind::Static | MeshKind::StaticUv2 => MeshKind::StaticUv2,
        MeshKind::Skinned | MeshKind::SkinnedUv2 => MeshKind::SkinnedUv2,
    }
}
