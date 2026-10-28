//! `ModelOutlines`' three screen-sized textures (`fboA`, the solid mask;
//! `fboB` and `fboC`, the horizontally and vertically thickened outline)
//! and the pipelines of the outline shaders (outline_solid.wgsl,
//! outline_pass.wgsl).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use pz_render::model::MeshKind;

use crate::state::{self, TargetKind};

/// The outline textures' format (`TextureFBO` colour: GL_RGBA8).
pub const OUTLINE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The thickening and blit passes' uniforms (outline_pass.wgsl `Pass`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PassUniforms {
    pub color: [f32; 4],
    pub params: [f32; 4],
}

/// Uniform blocks are bound at 256-byte aligned offsets.
pub const PASS_STRIDE: u64 = 256;

/// One outline draw of a batch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// A mesh into the mask: its layout, whether faces are culled, the
    /// mesh, its `ModelUniforms` block and texture.
    Solid {
        kind: MeshKind,
        cull: bool,
        mesh: u64,
        uniform: u32,
        texture: Option<u32>,
    },
    /// `aim_outline_h` (false) or `aim_outline_v` (true).
    Thicken { vertical: bool, uniform: u32 },
    /// `aim_outline_blit` into a target of the given kind.
    Blit { target: TargetKind },
    /// `vboRenderer_SpriteOutline` into the mask: a quad of the frame's
    /// vertices with its globals block, sampling a texture or an atlas.
    Sprite { texture: SpriteSource, globals: u32 },
}

/// What a sprite outline samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpriteSource {
    Texture(u32),
    Atlas(u64),
}

/// `fboA`, `fboB`, `fboC`: textures as large as the next powers of two of
/// the screen size (`ImageData`'s hardware size).
pub struct OutlineTargets {
    pub width: u32,
    pub height: u32,
    pub views: [wgpu::TextureView; 3],
    /// Each texture sampled GL_LINEAR with clamp to edge (the FBO's
    /// texture parameters).
    pub binds: [wgpu::BindGroup; 3],
}

/// Which outline texture a pass draws into.
pub const MASK: u8 = 0;
pub const THICK_H: u8 = 1;
pub const THICK_V: u8 = 2;

pub struct OutlinePipelines {
    solid_module: wgpu::ShaderModule,
    pass_module: wgpu::ShaderModule,
    pub pass_uniform_layout: wgpu::BindGroupLayout,
    solid_layout: wgpu::PipelineLayout,
    pass_layout: wgpu::PipelineLayout,
    pub solid: HashMap<(MeshKind, bool), wgpu::RenderPipeline>,
    pub thicken: Option<[wgpu::RenderPipeline; 2]>,
    pub blit: HashMap<(TargetKind, wgpu::TextureFormat), wgpu::RenderPipeline>,
    pub targets: Option<OutlineTargets>,
    /// `vboRenderer_SpriteOutline` into the mask.
    pub sprite: wgpu::RenderPipeline,
}

fn alpha_blend() -> wgpu::BlendState {
    let c = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState { color: c, alpha: c }
}

impl OutlinePipelines {
    pub fn new(
        device: &wgpu::Device,
        model_uniform_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
        globals_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let module = |label: &str, src: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            })
        };
        let solid_module = module("aim_outline_solid", include_str!("outline_solid.wgsl"));
        let pass_module = module("aim_outline", include_str!("outline_pass.wgsl"));
        let pass_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("outline pass"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<PassUniforms>() as u64,
                        ),
                    },
                    count: None,
                }],
            });
        let solid_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline solid"),
            bind_group_layouts: &[Some(model_uniform_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let pass_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline pass"),
            bind_group_layouts: &[
                Some(&pass_uniform_layout),
                Some(texture_layout),
                Some(texture_layout),
            ],
            immediate_size: 0,
        });
        let sprite_module = module("vboRenderer_SpriteOutline", include_str!("outline_sprite.wgsl"));
        let sprite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline sprite"),
            bind_group_layouts: &[Some(globals_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let sprite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vboRenderer_SpriteOutline"),
            layout: Some(&sprite_layout),
            vertex: wgpu::VertexState {
                module: &sprite_module,
                entry_point: Some("vs_sprite_outline"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sprite_module,
                entry_point: Some("fs_sprite_outline"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTLINE_FORMAT,
                    blend: Some(alpha_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        OutlinePipelines {
            sprite,
            solid_module,
            pass_module,
            pass_uniform_layout,
            solid_layout,
            pass_layout,
            solid: HashMap::new(),
            thicken: None,
            blit: HashMap::new(),
            targets: None,
        }
    }

    /// The mask pipeline for a mesh layout: blending (SRC_ALPHA,
    /// ONE_MINUS_SRC_ALPHA), no depth attachment (the draws never write
    /// depth), front faces culled as `Model.DrawChar` culls them (drawn
    /// into a texture, so with y flipped).
    pub fn ensure_solid(&mut self, device: &wgpu::Device, kind: MeshKind, cull: bool) {
        if self.solid.contains_key(&(kind, cull)) {
            return;
        }
        let (vs, attrs, stride): (&str, &[wgpu::VertexAttribute], u64) = match kind {
            MeshKind::Static => (
                "vs_solid_static",
                &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                32,
            ),
            MeshKind::Skinned => (
                "vs_solid_skinned",
                &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2],
                64,
            ),
            MeshKind::StaticUv2 => (
                "vs_solid_static",
                &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                40,
            ),
            MeshKind::SkinnedUv2 => (
                "vs_solid_skinned",
                &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2],
                72,
            ),
        };
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("aim_outline_solid"),
            layout: Some(&self.solid_layout),
            vertex: wgpu::VertexState {
                module: &self.solid_module,
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
                cull_mode: cull.then_some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.solid_module,
                entry_point: Some("fs_solid"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTLINE_FORMAT,
                    blend: Some(alpha_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.solid.insert((kind, cull), p);
    }

    fn pass_pipeline(
        &self,
        device: &wgpu::Device,
        fs: &str,
        format: wgpu::TextureFormat,
        depth: Option<wgpu::TextureFormat>,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(&self.pass_layout),
            vertex: wgpu::VertexState {
                module: &self.pass_module,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            // The blit draws into the world buffer with the depth test off.
            depth_stencil: depth.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
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
                module: &self.pass_module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(alpha_blend()),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn ensure_thicken(&mut self, device: &wgpu::Device) {
        if self.thicken.is_none() {
            let h = self.pass_pipeline(device, "fs_thicken_h", OUTLINE_FORMAT, None);
            let v = self.pass_pipeline(device, "fs_thicken_v", OUTLINE_FORMAT, None);
            self.thicken = Some([h, v]);
        }
    }

    /// The blit pipeline for a target kind (`main_format` for the main
    /// target).
    pub fn ensure_blit(
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
        if !self.blit.contains_key(&(target, format)) {
            let p = self.pass_pipeline(device, "fs_blit", format, Some(depth));
            self.blit.insert((target, format), p);
        }
    }

    /// `checkFBOs`: (re)creates the three textures for a screen of
    /// `width` x `height`.
    pub fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
    ) {
        let (w, h) = (
            width.max(1).next_power_of_two(),
            height.max(1).next_power_of_two(),
        );
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.width == w && t.height == h)
        {
            return;
        }
        let make = |label: &str| {
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: OUTLINE_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            (view, bind)
        };
        let (a, ab) = make("ModelOutlines.fboA");
        let (b, bb) = make("ModelOutlines.fboB");
        let (c, cb) = make("ModelOutlines.fboC");
        self.targets = Some(OutlineTargets {
            width: w,
            height: h,
            views: [a, b, c],
            binds: [ab, bb, cb],
        });
    }
}

#[cfg(test)]
#[path = "outline_tests.rs"]
mod tests;
