//! `zombie.core.Color`'s packed-ABGR helpers, in the original's `f32`
//! arithmetic.

/// `1/255` as the original writes it.
const BYTE_TO_FLOAT: f32 = 0.003_921_569;

pub fn red(c: u32) -> f32 {
    BYTE_TO_FLOAT * (c & 0xFF) as f32
}

pub fn green(c: u32) -> f32 {
    BYTE_TO_FLOAT * (c >> 8 & 0xFF) as f32
}

pub fn blue(c: u32) -> f32 {
    BYTE_TO_FLOAT * (c >> 16 & 0xFF) as f32
}

pub fn alpha(c: u32) -> f32 {
    BYTE_TO_FLOAT * (c >> 24 & 0xFF) as f32
}

/// `PZMath.clamp(float, float, float)`.
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `Color.colorToABGR`.
pub fn to_abgr(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let byte = |v: f32| (clamp(v, 0.0, 1.0) * 255.0) as i32 as u32 & 0xFF;
    byte(a) << 24 | byte(b) << 16 | byte(g) << 8 | byte(r)
}

/// `Color.blendBGR`: RGB towards `target` by its alpha, keeping `value`'s
/// alpha.
pub fn blend_bgr(value: u32, target: u32) -> u32 {
    let ta = alpha(target);
    to_abgr(
        red(value) * (1.0 - ta) + red(target) * ta,
        green(value) * (1.0 - ta) + green(target) * ta,
        blue(value) * (1.0 - ta) + blue(target) * ta,
        alpha(value),
    )
}

/// `Color.tintABGR`.
pub fn tint_abgr(target: u32, tint: u32) -> u32 {
    let a = alpha(tint);
    to_abgr(
        red(tint) * a + red(target) * (1.0 - a),
        green(tint) * a + green(target) * (1.0 - a),
        blue(tint) * a + blue(target) * (1.0 - a),
        alpha(target),
    )
}

/// `Color.lerpABGR`.
pub fn lerp_abgr(a: u32, b: u32, t: f32) -> u32 {
    to_abgr(
        red(a) * (1.0 - t) + red(b) * t,
        green(a) * (1.0 - t) + green(b) * t,
        blue(a) * (1.0 - t) + blue(b) * t,
        alpha(a) * (1.0 - t) + alpha(b) * t,
    )
}

/// `Color.setAlphaChannelToABGR`.
pub fn set_alpha(c: u32, a: f32) -> u32 {
    ((clamp(a, 0.0, 1.0) * 255.0) as i32 as u32 & 0xFF) << 24 | c & 0xFF_FFFF
}

/// `ColorInfo`: a float colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorInfo {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ColorInfo {
    pub const WHITE: ColorInfo = ColorInfo { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ColorInfo { r, g, b, a }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_and_lerp() {
        let opaque_red = 0xFF00_00FF;
        assert_eq!(blend_bgr(0xFFFF_FFFF, opaque_red), opaque_red);
        assert_eq!(blend_bgr(0x80FF_FFFF, opaque_red), 0x8000_00FF);
        // Half way between black and white truncates to 127.
        assert_eq!(lerp_abgr(0xFF00_0000, 0xFFFF_FFFF, 0.5), 0xFF7F_7F7F);
        assert_eq!(set_alpha(0x1234_5678, 1.0), 0xFF34_5678);
    }
}
