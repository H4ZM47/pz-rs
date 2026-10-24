//! The OpenGL enum values the draw list carries, and their wgpu forms.

#![allow(dead_code)]

pub const NEVER: u32 = 512;
pub const LESS: u32 = 513;
pub const EQUAL: u32 = 514;
pub const LEQUAL: u32 = 515;
pub const GREATER: u32 = 516;
pub const NOTEQUAL: u32 = 517;
pub const GEQUAL: u32 = 518;
pub const ALWAYS: u32 = 519;

pub const ZERO: u32 = 0;
pub const ONE: u32 = 1;
pub const SRC_COLOR: u32 = 768;
pub const ONE_MINUS_SRC_COLOR: u32 = 769;
pub const SRC_ALPHA: u32 = 770;
pub const ONE_MINUS_SRC_ALPHA: u32 = 771;
pub const DST_ALPHA: u32 = 772;
pub const ONE_MINUS_DST_ALPHA: u32 = 773;
pub const DST_COLOR: u32 = 774;
pub const ONE_MINUS_DST_COLOR: u32 = 775;
pub const SRC_ALPHA_SATURATE: u32 = 776;
pub const CONSTANT_COLOR: u32 = 32769;
pub const ONE_MINUS_CONSTANT_COLOR: u32 = 32770;

pub const KEEP: u32 = 7680;
pub const REPLACE: u32 = 7681;
pub const INCR: u32 = 7682;
pub const DECR: u32 = 7683;
pub const INVERT: u32 = 5386;
pub const INCR_WRAP: u32 = 34055;
pub const DECR_WRAP: u32 = 34056;

pub const DEPTH_TEST: u32 = 2929;
pub const STENCIL_TEST: u32 = 2960;
pub const ALPHA_TEST: u32 = 3008;
pub const BLEND: u32 = 3042;
pub const SCISSOR_TEST: u32 = 3089;

pub const DEPTH_BUFFER_BIT: u32 = 0x100;
pub const STENCIL_BUFFER_BIT: u32 = 0x400;
pub const COLOR_BUFFER_BIT: u32 = 0x4000;

/// JOML `Matrix4f.setOrtho2D(left, right, bottom, top)`, column-major.
pub fn ortho2d(left: f32, right: f32, bottom: f32, top: f32) -> [[f32; 4]; 4] {
    [
        [2.0 / (right - left), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (top - bottom), 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
        [-(right + left) / (right - left), -(top + bottom) / (top - bottom), 0.0, 1.0],
    ]
}

pub fn compare(func: u32) -> wgpu::CompareFunction {
    match func {
        NEVER => wgpu::CompareFunction::Never,
        LESS => wgpu::CompareFunction::Less,
        EQUAL => wgpu::CompareFunction::Equal,
        LEQUAL => wgpu::CompareFunction::LessEqual,
        GREATER => wgpu::CompareFunction::Greater,
        NOTEQUAL => wgpu::CompareFunction::NotEqual,
        GEQUAL => wgpu::CompareFunction::GreaterEqual,
        _ => wgpu::CompareFunction::Always,
    }
}

pub fn stencil_op(op: u32) -> wgpu::StencilOperation {
    match op {
        ZERO => wgpu::StencilOperation::Zero,
        REPLACE => wgpu::StencilOperation::Replace,
        INCR => wgpu::StencilOperation::IncrementClamp,
        DECR => wgpu::StencilOperation::DecrementClamp,
        INVERT => wgpu::StencilOperation::Invert,
        INCR_WRAP => wgpu::StencilOperation::IncrementWrap,
        DECR_WRAP => wgpu::StencilOperation::DecrementWrap,
        _ => wgpu::StencilOperation::Keep,
    }
}

pub fn blend_factor(f: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as B;
    match f {
        ZERO => B::Zero,
        SRC_COLOR => B::Src,
        ONE_MINUS_SRC_COLOR => B::OneMinusSrc,
        SRC_ALPHA => B::SrcAlpha,
        ONE_MINUS_SRC_ALPHA => B::OneMinusSrcAlpha,
        DST_ALPHA => B::DstAlpha,
        ONE_MINUS_DST_ALPHA => B::OneMinusDstAlpha,
        DST_COLOR => B::Dst,
        ONE_MINUS_DST_COLOR => B::OneMinusDst,
        SRC_ALPHA_SATURATE => B::SrcAlphaSaturated,
        CONSTANT_COLOR => B::Constant,
        ONE_MINUS_CONSTANT_COLOR => B::OneMinusConstant,
        _ => B::One,
    }
}
