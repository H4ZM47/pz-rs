//! `FBORenderLevels`' per-chunk bookkeeping: which level-pair textures are
//! on screen, which exist, and which need re-recording (dirty flags).

use std::collections::HashMap;

use pz_render::iso::{calculate_min_level, x_to_screen, y_to_screen, PIXELS_PER_LEVEL, TILE_SCALE};
use pz_render::DrawCmd;

use crate::chunk::{dirty, extra_height_for_jumbo_trees, ChunkTexture, FLOOR_HEIGHT};
use crate::Camera;

/// `FBORenderChunk.JUMBO_XXL_HEIGHT`.
const JUMBO_XXL_HEIGHT: i32 = FLOOR_HEIGHT * 16;

/// `FBORenderLevels.calculateRenderBounds(level)` then `calculateOnScreen`
/// for a view of `width` x `height` screen pixels.
pub fn level_on_screen(cam: &Camera, width: u32, height: u32, wx: i32, wy: i32, level: i32) -> bool {
    let min_level = calculate_min_level(level);
    let max_level = min_level + 1;
    let n = max_level - min_level + 1;
    let mut w = (64 * TILE_SCALE * 8) as f32;
    let mut h = (FLOOR_HEIGHT * 8 + PIXELS_PER_LEVEL * n) as f32;
    let (bx, by) = ((wx * 8) as f32, (wy * 8) as f32);
    let mut x = x_to_screen(bx, by);
    let mut y = y_to_screen(bx, by, min_level as f32, 0);
    y -= (PIXELS_PER_LEVEL * n) as f32;
    let dh = extra_height_for_jumbo_trees(min_level, max_level);
    h += dh as f32;
    y -= dh as f32;
    if min_level == 0 {
        let mut y1 = y_to_screen(bx, by, min_level as f32, 0);
        y1 += FLOOR_HEIGHT as f32;
        y1 -= JUMBO_XXL_HEIGHT as f32;
        let dh1 = y - y1;
        if dh1 > 0.0 {
            h += dh1;
            y -= dh1;
        }
    }
    x -= cam.off_x as f32;
    y -= cam.off_y as f32;
    let z = cam.zoom;
    x /= z;
    w /= z;
    x -= w / 2.0;
    y /= z;
    h /= z;
    x + w > 0.0 && x < width as f32 && y + h > 0.0 && y < height as f32
}

/// The state of one level pair's texture.
#[derive(Clone, Copy, Debug, Default)]
struct Pair {
    texture: Option<ChunkTexture>,
    dirty: u64,
}

/// Chunk texture bookkeeping across frames.
#[derive(Default)]
pub struct ChunkCache {
    pairs: HashMap<(i32, i32, i32), Pair>,
}

impl ChunkCache {
    /// `invalidateLevel(level, flags)`.
    pub fn invalidate(&mut self, wx: i32, wy: i32, level: i32, flags: u64) {
        self.pairs.entry((wx, wy, calculate_min_level(level))).or_default().dirty |= flags;
    }

    /// Every level pair of a chunk (`invalidateAll`).
    pub fn invalidate_chunk(&mut self, wx: i32, wy: i32, flags: u64) {
        for ((x, y, _), p) in self.pairs.iter_mut() {
            if (*x, *y) == (wx, wy) {
                p.dirty |= flags;
            }
        }
    }

    /// `FBORenderCell.invalidateAll`: every level pair of every chunk.
    pub fn invalidate_all(&mut self, flags: u64) {
        for p in self.pairs.values_mut() {
            p.dirty |= flags;
        }
    }

    /// Applies a frame's [`crate::weather::SurfaceChecks`] result: water
    /// quality changes redraw the on-screen chunks' level 0
    /// (`checkWaterQualityOption`, `REDRAW`), the low-quality puddle and
    /// snow checks every texture (`invalidateAll(CUTAWAYS)`). Re-initialising
    /// the puddles (`reinit_puddles`) is the caller's.
    pub fn apply_surface_checks(&mut self, inv: &crate::weather::SurfaceInvalidation) {
        if inv.level_zero {
            for ((_, _, l), p) in self.pairs.iter_mut() {
                if *l == 0 {
                    p.dirty |= dirty::REDRAW;
                }
            }
        }
        if inv.all {
            self.invalidate_all(dirty::CUTAWAYS);
        }
    }

    /// For one on-screen level pair: the texture to use and whether it must
    /// be recorded this frame (new, resized or dirty). Clears its dirty
    /// flags.
    pub fn begin_level(&mut self, wx: i32, wy: i32, level: i32, chunk_min: i32, chunk_max: i32) -> (ChunkTexture, bool) {
        let tex = ChunkTexture::new(wx, wy, level, chunk_min, chunk_max);
        let p = self.pairs.entry((wx, wy, calculate_min_level(level))).or_default();
        let record = p.texture != Some(tex) || p.dirty != 0;
        if p.texture.is_none() {
            p.dirty |= dirty::CREATE;
        }
        p.texture = Some(tex);
        p.dirty = 0;
        (tex, record)
    }

    /// `freeFBOsForLevel` for every pair not in `keep`: its texture is
    /// released.
    pub fn free_except(&mut self, keep: &[(i32, i32, i32)], out: &mut Vec<DrawCmd>) {
        self.pairs.retain(|k, p| {
            if keep.contains(k) {
                return true;
            }
            if let Some(t) = p.texture {
                out.push(DrawCmd::ChunkFree(t.id()));
            }
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_once_until_dirty() {
        let mut c = ChunkCache::default();
        assert!(c.begin_level(1, 2, 0, 0, 0).1);
        assert!(!c.begin_level(1, 2, 0, 0, 0).1);
        c.invalidate(1, 2, 0, dirty::LIGHTING);
        assert!(c.begin_level(1, 2, 0, 0, 0).1);
        // The chunk gained a level: its texture now holds both levels.
        assert!(c.begin_level(1, 2, 0, 0, 1).1);
        assert!(!c.begin_level(1, 2, 1, 0, 1).1);
        let mut out = Vec::new();
        c.free_except(&[], &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn surface_checks_redraw_the_textures() {
        let mut c = ChunkCache::default();
        c.begin_level(0, 0, 0, 0, 3);
        c.begin_level(0, 0, 2, 0, 3);
        c.apply_surface_checks(&crate::weather::SurfaceInvalidation { level_zero: true, ..Default::default() });
        assert!(c.begin_level(0, 0, 0, 0, 3).1);
        assert!(!c.begin_level(0, 0, 2, 0, 3).1);
        c.apply_surface_checks(&crate::weather::SurfaceInvalidation { all: true, ..Default::default() });
        assert!(c.begin_level(0, 0, 0, 0, 3).1);
        assert!(c.begin_level(0, 0, 2, 0, 3).1);
    }

    #[test]
    fn on_screen_bounds() {
        let cam = Camera::centred(4.0, 4.0, 0.0, 256, 128, 1.0);
        assert!(level_on_screen(&cam, 256, 128, 0, 0, 0));
        assert!(!level_on_screen(&cam, 256, 128, 10, 10, 0));
    }
}
