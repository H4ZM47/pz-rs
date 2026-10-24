//! Chunk textures: `FBORenderLevels`' level pairing and texture sizes,
//! `FBORenderChunkManager`'s recording offsets and
//! `FBORenderChunk.renderInWorldMainThread`'s composite.

use pz_render::iso::{self, calculate_min_level, x_to_screen, y_to_screen, PIXELS_PER_LEVEL, TILE_SCALE};
use pz_render::DrawCmd;

/// `FBORenderChunk.FLOOR_HEIGHT`.
pub const FLOOR_HEIGHT: i32 = 32 * TILE_SCALE;
/// `FBORenderChunk.JUMBO_L_HEIGHT`.
pub const JUMBO_L_HEIGHT: i32 = FLOOR_HEIGHT * 8;
/// `FBORenderChunk.FLOOR_WIDTH`.
pub const FLOOR_WIDTH: i32 = 64 * TILE_SCALE;
/// `FBORenderChunk.JUMBO_L_WIDTH`, `JUMBO_XL_WIDTH`, `JUMBO_XXL_WIDTH`.
pub const JUMBO_L_WIDTH: i32 = FLOOR_WIDTH * 3;
pub const JUMBO_XL_WIDTH: i32 = FLOOR_WIDTH * 5;
pub const JUMBO_XXL_WIDTH: i32 = FLOOR_WIDTH * 7;

/// `FBORenderChunk` dirty flags.
pub mod dirty {
    pub const BLOOD: u64 = 1;
    pub const CORPSE: u64 = 2;
    pub const ITEM_ADD: u64 = 4;
    pub const ITEM_REMOVE: u64 = 8;
    pub const ITEM_MODIFY: u64 = 16;
    pub const LIGHTING: u64 = 32;
    pub const OBJECT_ADD: u64 = 64;
    pub const OBJECT_REMOVE: u64 = 128;
    pub const OBJECT_MODIFY: u64 = 256;
    pub const CREATE: u64 = 512;
    pub const REDRAW: u64 = 1024;
    pub const CUTAWAYS: u64 = 2048;
    pub const TREES: u64 = 4096;
    pub const OBSCURING: u64 = 8192;
    pub const REDO_CUTAWAYS: u64 = 16384;
}

/// `PZMath.smallestEncompassingPowerOfTwo`.
pub fn pow2(v: i32) -> i32 {
    if v <= 1 {
        1
    } else {
        1 << (32 - ((v - 1) as u32).leading_zeros())
    }
}

/// `FBORenderLevels.extraHeightForJumboTrees`.
pub fn extra_height_for_jumbo_trees(min_level: i32, max_level: i32) -> i32 {
    if min_level != 0 || max_level > 1 {
        0
    } else if max_level == 1 {
        JUMBO_L_HEIGHT - PIXELS_PER_LEVEL * 2 - FLOOR_HEIGHT / 2
    } else {
        JUMBO_L_HEIGHT - PIXELS_PER_LEVEL - FLOOR_HEIGHT / 2
    }
}

fn min4(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a.min(b).min(c).min(d)
}

fn max4(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a.max(b).max(c).max(d)
}

/// The screen-space extent of a chunk's footprint at levels 0 and
/// `num_levels`, by `f(x, y, z)`.
fn extent(num_levels: i32, f: impl Fn(f32, f32, f32) -> f32) -> f32 {
    let corners = |z: f32| [f(0.0, 0.0, z), f(8.0, 0.0, z), f(8.0, 8.0, z), f(0.0, 8.0, z)];
    let [a, b, c, d] = corners(0.0);
    let (mut lo, mut hi) = (min4(a, b, c, d), max4(a, b, c, d));
    let [a, b, c, d] = corners(num_levels as f32);
    lo = lo.min(min4(a, b, c, d));
    hi = hi.max(max4(a, b, c, d));
    hi - lo
}

/// `FBORenderLevels.calculateTextureWidthForLevels` (texture scale 1).
pub fn texture_width(min_level: i32, max_level: i32) -> i32 {
    let w = extent(max_level - min_level + 1, |x, y, _| x_to_screen(x, y)).ceil() as i32;
    pow2(w)
}

/// `FBORenderLevels.calculateTextureHeightForLevels` (texture scale 1).
pub fn texture_height(min_level: i32, max_level: i32) -> i32 {
    let h = extent(max_level - min_level + 1, |x, y, z| y_to_screen(x, y, z, 0)).ceil() as i32;
    pow2(h + extra_height_for_jumbo_trees(min_level, max_level))
}

/// The level pair one chunk texture holds for `level`, clamped to the
/// chunk's levels (`FBORenderLevels.createFBOForLevel`).
pub fn level_pair(level: i32, chunk_min: i32, chunk_max: i32) -> (i32, i32) {
    let mut min = calculate_min_level(level);
    let max = (min + 1).min(chunk_max);
    if level < 0 {
        min = min.max(chunk_min);
    }
    (min, max)
}

/// One chunk texture: chunk `(wx, wy)`, levels `min_level..=top_level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkTexture {
    pub wx: i32,
    pub wy: i32,
    pub min_level: i32,
    pub top_level: i32,
}

impl ChunkTexture {
    pub fn new(wx: i32, wy: i32, level: i32, chunk_min: i32, chunk_max: i32) -> Self {
        let (min_level, top_level) = level_pair(level, chunk_min, chunk_max);
        ChunkTexture { wx, wy, min_level, top_level }
    }

    pub fn width(&self) -> i32 {
        texture_width(self.min_level, self.top_level)
    }

    pub fn height(&self) -> i32 {
        texture_height(self.min_level, self.top_level)
    }

    /// A stable id for the texture (chunk and level pair).
    pub fn id(&self) -> u64 {
        let pair = (calculate_min_level(self.min_level) / 2 + 16) as u64 & 0x3F;
        ((self.wx as u32 as u64) << 32 | (self.wy as u16 as u64) << 16) ^ pair << 6 | pair
    }

    /// `FBORenderChunkManager.xoff/yoff` while recording into this texture.
    pub fn record_offset(&self) -> (f32, f32) {
        let xoff = self.width() as f32 / 2.0;
        let mut yoff = ((self.top_level - self.min_level + 1) * PIXELS_PER_LEVEL) as f32;
        yoff += (self.min_level * PIXELS_PER_LEVEL) as f32;
        yoff += extra_height_for_jumbo_trees(self.min_level, self.top_level) as f32;
        (xoff, yoff)
    }

    /// The commands that start recording into this texture: end of the
    /// current frame, `StartFrameFlipY` and `FBORenderChunkStart`.
    pub fn begin(&self, clear: bool) -> Vec<DrawCmd> {
        let (w, h) = (self.width(), self.height());
        vec![
            DrawCmd::ChunkBegin { id: self.id(), width: w as u32, height: h as u32, clear },
            DrawCmd::Ortho { left: 0.0, right: w as f32, bottom: 0.0, top: h as f32, width: w as u32, height: h as u32 },
            DrawCmd::GlEnable(pz_render::gl::BLEND),
            DrawCmd::DepthFunc(pz_render::gl::ALWAYS),
            DrawCmd::AlphaFunc(pz_render::gl::GREATER, 0.0),
            DrawCmd::BlendFuncSeparate(pz_render::gl::ONE, pz_render::gl::ONE_MINUS_SRC_ALPHA, pz_render::gl::ONE_MINUS_DST_ALPHA, pz_render::gl::ONE),
        ]
    }

    /// `FBORenderChunk.renderInWorldMainThread`: the composite into the
    /// offscreen buffer under a screen-pixel projection.
    pub fn composite(&self, cam: &crate::Camera) -> Vec<DrawCmd> {
        let r = iso::chunk_depth_data(cam.chunk_x(), cam.chunk_y(), self.wx, self.wy, self.min_level);
        let mut depth = ((r.index_x + r.index_y) as f32 - cam.fix_jiggly_square_x - cam.fix_jiggly_square_y) / 8.0 / 40.0;
        depth *= 0.461_873_35;
        depth -= calculate_min_level(self.min_level) as f32 * iso::LEVEL_DEPTH;
        let (bx, by) = ((self.wx * 8) as f32, (self.wy * 8) as f32);
        let mut x = x_to_screen(bx, by);
        let mut y = y_to_screen(bx, by, self.min_level as f32, 0);
        let (mut w, mut h) = (self.width() as f32, self.height() as f32);
        y -= (PIXELS_PER_LEVEL * (self.top_level - self.min_level + 1)) as f32;
        y -= extra_height_for_jumbo_trees(self.min_level, self.top_level) as f32;
        x -= cam.off_x as f32;
        y -= cam.off_y as f32;
        x /= cam.zoom;
        y /= cam.zoom;
        w /= cam.zoom;
        h /= cam.zoom;
        x -= w / 2.0;
        x += cam.fix_jiggly_x;
        y += cam.fix_jiggly_y;
        vec![
            DrawCmd::BlendFuncSeparate(pz_render::gl::ONE, pz_render::gl::ONE_MINUS_SRC_ALPHA, pz_render::gl::ONE_MINUS_DST_ALPHA, pz_render::gl::ONE),
            DrawCmd::DepthFunc(pz_render::gl::LEQUAL),
            DrawCmd::DepthMask(true),
            DrawCmd::GlEnable(pz_render::gl::DEPTH_TEST),
            DrawCmd::ChunkComposite { id: self.id(), x, y, w, h, color: 0xFFFF_FFFF, chunk_depth: depth, mag_linear: cam.zoom == 0.75 },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_sizes() {
        assert_eq!(extra_height_for_jumbo_trees(0, 0), 288);
        assert_eq!(extra_height_for_jumbo_trees(0, 1), 96);
        assert_eq!(texture_width(0, 1), 1024);
        // 512 floor span + 2 levels of 192 + 96.
        assert_eq!(texture_height(0, 1), 1024);
        assert_eq!(texture_height(2, 3), 1024);
        assert_eq!(texture_height(0, 0), 1024);
        assert_eq!(level_pair(3, 0, 3), (2, 3));
        assert_eq!(level_pair(2, 0, 2), (2, 2));
        assert_eq!(level_pair(-1, -1, 0), (-1, -1));
        assert_eq!(pow2(1024), 1024);
        assert_eq!(pow2(1025), 2048);
    }
}
