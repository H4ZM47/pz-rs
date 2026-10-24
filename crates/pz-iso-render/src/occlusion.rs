//! `FBORenderOcclusion`: squares hidden behind the floors of the levels
//! above them.
//!
//! Every rendered square that blocks vision downwards (or has nothing
//! below it) is an occluder; it covers the squares three steps further
//! along the screen diagonal per level below it. The grid stores, per
//! "level-0" square, the highest occluder projected onto it
//! (`FBORenderCutaways.ChunkLevelData.calculateOccludingSquares`), and a
//! square is occluded when all ten grid cells under its footprint are
//! covered by a higher level (`isOccluded`).

use pz_render::iso::{fastfloor, x_to_iso, y_to_iso};

use crate::Camera;

/// The occluded-squares grid of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct OccludedGrid {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    grid: Vec<i32>,
}

/// `isOccluded`'s footprint: the square and the nine around it towards the
/// viewer.
const FOOTPRINT: [(i32, i32); 10] = [(0, 0), (1, 1), (2, 2), (3, 3), (1, 0), (0, 1), (2, 1), (1, 2), (3, 2), (2, 3)];

impl OccludedGrid {
    /// An empty grid over `[x1, x2] x [y1, y2]` (filled with -32).
    pub fn new(bounds: [i32; 4]) -> Self {
        let [x1, y1, x2, y2] = bounds;
        let w = (x2 - x1 + 1).max(0) as usize;
        let h = (y2 - y1 + 1).max(0) as usize;
        OccludedGrid { x1, y1, x2, y2, grid: vec![-32; w * h] }
    }

    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let zx = x - z * 3 - self.x1;
        let zy = y - z * 3 - self.y1;
        if zx < 0 || zy < 0 || zx > self.x2 - self.x1 || zy > self.y2 - self.y1 {
            return None;
        }
        Some((zx + zy * (self.x2 - self.x1 + 1)) as usize)
    }

    /// Records the square at `(x, y, z)` as an occluder; false when it
    /// projects outside the grid.
    pub fn add_occluder(&mut self, x: i32, y: i32, z: i32) -> bool {
        match self.index(x, y, z) {
            Some(i) => {
                self.grid[i] = self.grid[i].max(z);
                true
            }
            None => false,
        }
    }

    /// `isOccludedAux` against level `level`.
    fn covered(&self, x: i32, y: i32, z: i32, level: i32) -> bool {
        self.index(x, y, z).is_some_and(|i| self.grid[i] > level)
    }

    /// `FBORenderOcclusion.isOccluded`.
    pub fn is_occluded(&self, x: i32, y: i32, z: i32) -> bool {
        FOOTPRINT.iter().all(|&(dx, dy)| self.covered(x - dx, y - dy, z, z))
    }
}

/// `FBORenderCell.renderInternal`'s `occludedGridX1/Y1/X2/Y2`: the squares
/// under the view, from the top-left corner at level 0 to the
/// bottom-right at level 6. `offscreen_w/h` are
/// `IsoCamera.getOffscreenWidth/Height` (the screen size times the zoom).
pub fn grid_bounds(cam: &Camera, offscreen_w: i32, offscreen_h: i32) -> [i32; 4] {
    let (ox, oy) = (cam.off_x as f32, cam.off_y as f32);
    let (w, h) = (offscreen_w as f32, offscreen_h as f32);
    let top_left_x = x_to_iso(ox, oy, 0.0);
    let top_right_y = y_to_iso(w + ox, oy, 0.0);
    let bottom_right_x = x_to_iso(w + ox, h + oy, 6.0);
    let bottom_left_y = y_to_iso(ox, h + oy, 6.0);
    // Java's (int) casts truncate towards zero.
    [top_left_x as i32, top_right_y as i32, bottom_right_x as i32, bottom_left_y as i32]
}

/// The chunk `(wx, wy)` a square lies in.
pub fn chunk_of(x: i32, y: i32) -> (i32, i32) {
    (fastfloor(x as f32 / 8.0), fastfloor(y as f32 / 8.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_floor_above_hides_the_squares_it_covers() {
        let mut g = OccludedGrid::new([-10, -10, 30, 30]);
        // A 6x6 floor at level 1 over (3..9, 3..9).
        for y in 3..9 {
            for x in 3..9 {
                assert!(g.add_occluder(x, y, 1));
            }
        }
        // Level 0 squares under it are shifted by three per level: the
        // level-1 square (x, y) projects onto (x - 3, y - 3) at level 0.
        assert!(g.is_occluded(5, 5, 0));
        assert!(g.is_occluded(3, 3, 0));
        // Near the floor's top-left edge part of the footprint is open.
        assert!(!g.is_occluded(1, 1, 0));
        // Squares at the occluder's own level are never hidden by it.
        assert!(!g.is_occluded(5, 5, 1));
        // Outside the grid nothing is occluded.
        assert!(!g.is_occluded(100, 100, 0));
    }
}
