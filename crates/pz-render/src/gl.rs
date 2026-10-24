//! OpenGL enum values carried in draw lists.

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

pub const FUNC_ADD: u32 = 32774;
pub const MIN: u32 = 32775;
pub const MAX: u32 = 32776;
pub const FUNC_SUBTRACT: u32 = 32778;
pub const FUNC_REVERSE_SUBTRACT: u32 = 32779;

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
