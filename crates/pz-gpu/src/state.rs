//! The GL state `IndieGL`/`GLState` drive through the draw list, and the
//! wgpu pipelines that realise each combination of it.

use pz_render::{Blend, DrawCmd};

use crate::gl;

pub const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

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
    /// A `glClear` quad (no alpha test or discard).
    pub clear: bool,
}

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
            DrawCmd::Sprite(_) | DrawCmd::Quad { .. } | DrawCmd::Clear(_) => {}
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
            clear: false,
        }
    }

    /// `glClear(bit)`: colour clears write through the colour mask without
    /// blending; stencil clears replace through the stencil write mask.
    pub fn clear_key(&self, bit: u32) -> PipeKey {
        if bit == gl::STENCIL_BUFFER_BIT {
            PipeKey { blend: None, color_mask: 0, stencil: Some([gl::ALWAYS, 0xFF, self.stencil_write_mask & 0xFF, gl::REPLACE, gl::REPLACE, gl::REPLACE]), clear: true }
        } else {
            PipeKey { blend: None, color_mask: Self::mask_bits(self.color_mask), stencil: None, clear: true }
        }
    }
}

pub fn create_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, format: wgpu::TextureFormat, key: PipeKey) -> wgpu::RenderPipeline {
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
        Some([func, read, write, sfail, dpfail, dppass]) => {
            let face = wgpu::StencilFaceState {
                compare: gl::compare(func),
                fail_op: gl::stencil_op(sfail),
                // There is no depth buffer, so the depth test always passes.
                depth_fail_op: gl::stencil_op(dpfail),
                pass_op: gl::stencil_op(dppass),
            };
            wgpu::StencilState { front: face, back: face, read_mask: read, write_mask: write }
        }
        None => wgpu::StencilState { front: wgpu::StencilFaceState::IGNORE, back: wgpu::StencilFaceState::IGNORE, read_mask: 0, write_mask: 0 },
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 36,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4],
            })],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: STENCIL_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil,
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if key.clear { "fs_clear" } else { "fs_main" }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
