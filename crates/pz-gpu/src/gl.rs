//! The OpenGL enum values the draw list carries, and their wgpu forms.

#![allow(dead_code)]

pub use pz_render::gl::*;

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
