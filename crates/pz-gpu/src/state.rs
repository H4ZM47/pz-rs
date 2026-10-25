//! The GL state `IndieGL`/`GLState` drive through the draw list, and the
//! wgpu pipelines that realise each combination of it.

use pz_render::{Blend, DrawCmd};

use crate::gl;

/// The main target's depth/stencil buffer (`GL_DEPTH24_STENCIL8`).
pub const DEPTH_STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
/// Chunk textures' depth (`GL_DEPTH_COMPONENT16`).
pub const CHUNK_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// Chunk textures' colour.
pub const CHUNK_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The shader program a quad is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Program {
    /// `default` for UI and screen-space sprites.
    Default,
    /// `glClear` as a quad.
    Clear,
    /// `default` with world depth (`chunkDepth + zDepth`).
    WorldDefault,
    /// `tileWithDepth`.
    TileDepth,
    /// `opaqueWithDepth`.
    OpaqueDepth,
    /// `chunkShader`.
    Composite,
    /// `screen` (`WeatherShader`).
    Screen,
    /// `vboRenderer_PositionColorUV` for projected quads.
    Projected,
    /// `vboRenderer_PositionColorUVDepth` for projected quads with
    /// per-corner depth.
    ProjectedDepth,
    /// `seamFix2` (`TileSeamShader`).
    Seam,
    /// `CircleStencil` (cut-away walls).
    Cutaway,
    /// `DeadBodyAtlas`: an atlas texture with its own depth.
    DepthAtlas,
}

impl Program {
    pub fn uses_world_shader(self) -> bool {
        matches!(self, Program::WorldDefault | Program::TileDepth | Program::OpaqueDepth | Program::Composite | Program::Projected | Program::ProjectedDepth | Program::Seam | Program::Cutaway | Program::DepthAtlas)
    }
}

/// Fixed-function state that affects how quads are drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct GlState {
    pub blend: bool,
    /// `glBlendFuncSeparate(srcRGB, dstRGB, srcAlpha, dstAlpha)`.
    pub blend_func: [u32; 4],
    pub color_mask: [bool; 4],
    pub stencil_test: bool,
    /// `glStencilFunc(func, ref, mask)`.
    pub stencil_func: u32,
    pub stencil_ref: u32,
    pub stencil_value_mask: u32,
    /// `glStencilOp(sfail, dpfail, dppass)`.
    pub stencil_op: [u32; 3],
    /// `glStencilMask`.
    pub stencil_write_mask: u32,
    pub alpha_test: bool,
    pub alpha_func: u32,
    pub alpha_ref: f32,
    pub scissor_test: bool,
    /// Scissor box `[x, y, w, h]`, top-left origin in framebuffer pixels.
    pub scissor: [u32; 4],
    pub clear_color: [f64; 4],
    pub clear_stencil: u32,
    pub depth_test: bool,
    pub depth_func: u32,
    pub depth_mask: bool,
}

impl Default for GlState {
    /// GL defaults, with `TransparentStyle`'s blend function.
    fn default() -> Self {
        GlState {
            blend: true,
            blend_func: [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA],
            color_mask: [true; 4],
            stencil_test: false,
            stencil_func: gl::ALWAYS,
            stencil_ref: 0,
            stencil_value_mask: 0xFF,
            stencil_op: [gl::KEEP; 3],
            stencil_write_mask: 0xFF,
            alpha_test: false,
            // `DoStartFrameStuff` sets glAlphaFunc(GL_GREATER, 0).
            alpha_func: gl::GREATER,
            alpha_ref: 0.0,
            scissor_test: false,
            scissor: [0; 4],
            clear_color: [0.0; 4],
            clear_stencil: 0,
            depth_test: false,
            depth_func: gl::LESS,
            depth_mask: true,
        }
    }
}

/// Everything baked into a wgpu render pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PipeKey {
    pub blend: Option<[u32; 4]>,
    pub color_mask: u8,
    /// compare, read mask, write mask, sfail, dpfail, dppass.
    pub stencil: Option<[u32; 6]>,
    pub program: Program,
    /// Depth compare and write when the depth test is on.
    pub depth: Option<(u32, bool)>,
    /// The kind of target drawn into.
    pub target: TargetKind,
}

/// Render target kinds, which fix colour and depth formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetKind {
    Main,
    Chunk,
    Offscreen,
}

/// The offscreen world buffer's colour format.
pub const OFFSCREEN_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl GlState {
    /// Applies a state command (`IndieGL` call) from the draw list.
    pub fn apply(&mut self, cmd: &DrawCmd) {
        match *cmd {
            DrawCmd::SetBlend(Blend::Alpha) => self.blend_func = [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA],
            DrawCmd::SetBlend(Blend::Additive) => self.blend_func = [gl::SRC_ALPHA, gl::ONE, gl::SRC_ALPHA, gl::ONE],
            DrawCmd::BlendFunc(s, d) => self.blend_func = [s, d, s, d],
            DrawCmd::BlendFuncSeparate(a, b, c, d) => self.blend_func = [a, b, c, d],
            DrawCmd::GlEnable(cap) | DrawCmd::GlDisable(cap) => {
                let on = matches!(cmd, DrawCmd::GlEnable(_));
                match cap {
                    gl::BLEND => self.blend = on,
                    gl::STENCIL_TEST => self.stencil_test = on,
                    gl::ALPHA_TEST => self.alpha_test = on,
                    gl::SCISSOR_TEST => self.scissor_test = on,
                    gl::DEPTH_TEST => self.depth_test = on,
                    _ => {}
                }
            }
            DrawCmd::StencilMask(m) => self.stencil_write_mask = m,
            DrawCmd::StencilFunc(f, r, m) => {
                self.stencil_func = f;
                self.stencil_ref = r.clamp(0, 255) as u32;
                self.stencil_value_mask = m;
            }
            DrawCmd::StencilOp(a, b, c) => self.stencil_op = [a, b, c],
            DrawCmd::ColorMask(r, g, b, a) => self.color_mask = [r, g, b, a],
            DrawCmd::AlphaFunc(f, r) => {
                self.alpha_func = f;
                self.alpha_ref = r.clamp(0.0, 1.0);
            }
            DrawCmd::Clip(Some([x, y, w, h])) => {
                self.scissor_test = true;
                self.scissor = [x.max(0) as u32, y.max(0) as u32, (w + x.min(0)).max(0) as u32, (h + y.min(0)).max(0) as u32];
            }
            DrawCmd::Clip(None) => self.scissor_test = false,
            DrawCmd::DepthFunc(f) => self.depth_func = f,
            DrawCmd::DepthMask(m) => self.depth_mask = m,
            _ => {}
        }
    }

    pub fn scissor_box(&self) -> Option<[u32; 4]> {
        self.scissor_test.then_some(self.scissor)
    }

    fn mask_bits(m: [bool; 4]) -> u8 {
        m.iter().enumerate().fold(0, |acc, (i, &b)| acc | (u8::from(b) << i))
    }

    pub fn pipe_key(&self) -> PipeKey {
        PipeKey {
            blend: self.blend.then_some(self.blend_func),
            color_mask: Self::mask_bits(self.color_mask),
            stencil: self.stencil_test.then_some([
                self.stencil_func,
                self.stencil_value_mask & 0xFF,
                self.stencil_write_mask & 0xFF,
                self.stencil_op[0],
                self.stencil_op[1],
                self.stencil_op[2],
            ]),
            program: Program::Default,
            depth: self.depth_test.then_some((self.depth_func, self.depth_mask)),
            target: TargetKind::Main,
        }
    }

    /// `glClear(bit)`: colour clears write through the colour mask without
    /// blending; stencil clears replace through the stencil write mask.
    pub fn clear_key(&self, bit: u32) -> PipeKey {
        let base = PipeKey { blend: None, color_mask: 0, stencil: None, program: Program::Clear, depth: None, target: TargetKind::Main };
        match bit {
            gl::STENCIL_BUFFER_BIT => PipeKey { stencil: Some([gl::ALWAYS, 0xFF, self.stencil_write_mask & 0xFF, gl::REPLACE, gl::REPLACE, gl::REPLACE]), ..base },
            gl::DEPTH_BUFFER_BIT => PipeKey { depth: Some((gl::ALWAYS, self.depth_mask)), ..base },
            _ => PipeKey { color_mask: Self::mask_bits(self.color_mask), ..base },
        }
    }
}

/// Shader modules and pipeline layouts the pipelines are built from.
pub struct Programs {
    pub sprite: wgpu::ShaderModule,
    /// `world.wgsl` with `world_tile.wgsl`.
    pub world_tile: wgpu::ShaderModule,
    /// `world.wgsl` with `world_composite.wgsl`.
    pub world_composite: wgpu::ShaderModule,
    pub screen: wgpu::ShaderModule,
    pub sprite_layout: wgpu::PipelineLayout,
    pub tile_layout: wgpu::PipelineLayout,
    pub composite_layout: wgpu::PipelineLayout,
    pub seam_layout: wgpu::PipelineLayout,
}

pub fn create_pipeline(device: &wgpu::Device, programs: &Programs, format: wgpu::TextureFormat, key: PipeKey) -> wgpu::RenderPipeline {
    let blend = key.blend.map(|[sc, dc, sa, da]| wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: gl::blend_factor(sc), dst_factor: gl::blend_factor(dc), operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent { src_factor: gl::blend_factor(sa), dst_factor: gl::blend_factor(da), operation: wgpu::BlendOperation::Add },
    });
    let mut write_mask = wgpu::ColorWrites::empty();
    for (i, w) in [wgpu::ColorWrites::RED, wgpu::ColorWrites::GREEN, wgpu::ColorWrites::BLUE, wgpu::ColorWrites::ALPHA].into_iter().enumerate() {
        if key.color_mask & (1 << i) != 0 {
            write_mask |= w;
        }
    }
    let stencil = match key.stencil {
        // Chunk textures have no stencil buffer.
        Some([func, read, write, sfail, dpfail, dppass]) if key.target != TargetKind::Chunk => {
            let face = wgpu::StencilFaceState {
                compare: gl::compare(func),
                fail_op: gl::stencil_op(sfail),
                depth_fail_op: gl::stencil_op(dpfail),
                pass_op: gl::stencil_op(dppass),
            };
            wgpu::StencilState { front: face, back: face, read_mask: read, write_mask: write }
        }
        _ => wgpu::StencilState { front: wgpu::StencilFaceState::IGNORE, back: wgpu::StencilFaceState::IGNORE, read_mask: 0, write_mask: 0 },
    };
    // GL writes no depth while the depth test is off.
    let (depth_compare, depth_write) = match key.depth {
        Some((func, write)) => (gl::compare(func), write),
        None => (wgpu::CompareFunction::Always, false),
    };
    let world = key.program.uses_world_shader();
    let (module, layout) = match key.program {
        Program::TileDepth | Program::OpaqueDepth => (&programs.world_tile, &programs.tile_layout),
        Program::Composite | Program::DepthAtlas => (&programs.world_composite, &programs.composite_layout),
        Program::Seam | Program::Cutaway => (&programs.world_tile, &programs.seam_layout),
        Program::WorldDefault | Program::Projected | Program::ProjectedDepth => (&programs.world_tile, &programs.sprite_layout),
        Program::Default | Program::Clear => (&programs.sprite, &programs.sprite_layout),
        Program::Screen => (&programs.screen, &programs.sprite_layout),
    };
    let (vs, fs) = match key.program {
        Program::Default => ("vs_main", "fs_main"),
        Program::Clear => ("vs_main", "fs_clear"),
        Program::WorldDefault => ("vs_world", "fs_default"),
        Program::TileDepth => ("vs_world", "fs_tile"),
        Program::OpaqueDepth => ("vs_world", "fs_opaque"),
        Program::Composite => ("vs_world", "fs_composite"),
        Program::Screen => ("vs_screen", "fs_screen"),
        Program::Projected => ("vs_world", "fs_projected"),
        Program::ProjectedDepth => ("vs_world", "fs_projected_depth"),
        Program::Seam => ("vs_world", "fs_seam"),
        Program::Cutaway => ("vs_world", "fs_cutaway"),
        Program::DepthAtlas => ("vs_atlas", "fs_atlas"),
    };
    let attributes: &[wgpu::VertexAttribute] = if world {
        &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4, 3 => Float32x2, 4 => Float32x2, 5 => Float32x2]
    } else {
        &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4]
    };
    let (format, depth_format) = match key.target {
        TargetKind::Chunk => (CHUNK_COLOR_FORMAT, CHUNK_DEPTH_FORMAT),
        TargetKind::Offscreen => (OFFSCREEN_COLOR_FORMAT, DEPTH_STENCIL_FORMAT),
        TargetKind::Main => (format, DEPTH_STENCIL_FORMAT),
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<crate::Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes })],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(depth_compare),
            stencil,
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
