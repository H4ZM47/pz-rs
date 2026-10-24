//! `HLChunkRegion`: a connected set of walkable squares inside one chunk
//! level, with bit masks of its outline on each chunk edge row/column.

use super::RegionId;
use crate::world::{LevelRef, World};

/// `HLChunkRegion`.
#[derive(Clone, Debug)]
pub struct HlChunkRegion {
    pub level: LevelRef,
    /// `squaresMask`: bit `x` of `mask[y]`.
    pub mask: [u8; 8],
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub edge_n: [i32; 8],
    pub edge_s: [i32; 8],
    pub edge_w: [i32; 8],
    pub edge_e: [i32; 8],
}

impl HlChunkRegion {
    pub(crate) fn new(level: LevelRef) -> Self {
        HlChunkRegion {
            level,
            mask: [0; 8],
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            edge_n: [0; 8],
            edge_s: [0; 8],
            edge_w: [0; 8],
            edge_e: [0; 8],
        }
    }

    /// `containsSquareLocal`.
    #[inline]
    pub fn contains_square_local(&self, x: i32, y: i32) -> bool {
        (0..8).contains(&x) && (0..8).contains(&y) && self.mask[y as usize] & (1 << x) != 0
    }

    #[inline]
    pub(crate) fn set_local(&mut self, x: i32, y: i32) {
        if (0..8).contains(&x) && (0..8).contains(&y) {
            self.mask[y as usize] |= 1 << x;
        }
    }

    /// Centre of the region's bounding box (the region search node's
    /// position).
    pub fn center_x(&self) -> f32 {
        (self.min_x + self.max_x + 1) as f32 / 2.0
    }

    pub fn center_y(&self) -> f32 {
        (self.min_y + self.max_y + 1) as f32 / 2.0
    }

    /// `initEdges`.
    pub(crate) fn init_edges(&mut self) {
        for y in 0..8 {
            self.edge_n[y as usize] = 0;
            self.edge_s[y as usize] = 0;
            for x in 0..8 {
                if !self.contains_square_local(x, y) {
                    continue;
                }
                if !self.contains_square_local(x, y - 1) {
                    self.edge_n[y as usize] |= 1 << x;
                }
                if !self.contains_square_local(x, y + 1) {
                    self.edge_s[y as usize] |= 1 << x;
                }
            }
        }
        for x in 0..8 {
            self.edge_w[x as usize] = 0;
            self.edge_e[x as usize] = 0;
            for y in 0..8 {
                if !self.contains_square_local(x, y) {
                    continue;
                }
                if !self.contains_square_local(x - 1, y) {
                    self.edge_w[x as usize] |= 1 << y;
                }
                if !self.contains_square_local(x + 1, y) {
                    self.edge_e[x as usize] |= 1 << y;
                }
            }
        }
    }
}

impl World {
    #[inline]
    pub(crate) fn region(&self, r: RegionId) -> &HlChunkRegion {
        &self.hl.regions[r]
    }

    /// `HLChunkRegion::containsSquare` (world coordinates).
    pub(crate) fn region_contains_square(&self, r: RegionId, x: i32, y: i32) -> bool {
        let reg = &self.hl.regions[r];
        let c = self.chunk(reg.level.chunk);
        reg.contains_square_local(x.wrapping_sub(c.min_x()), y.wrapping_sub(c.min_y()))
    }

    /// `HLChunkRegion::isOnEdgeOfLoadedArea`.
    pub(crate) fn region_is_on_edge_of_loaded_area(&self, r: RegionId) -> bool {
        let reg = &self.hl.regions[r];
        let c = self.chunk(reg.level.chunk);
        let (wx, wy) = (c.wx, c.wy);
        let mut on_edge = false;
        if reg.edge_n[0] != 0 && self.get_chunk_from_chunk_pos(wx, wy - 1).is_none() {
            on_edge = true;
        }
        if reg.edge_s[7] != 0 && self.get_chunk_from_chunk_pos(wx, wy + 1).is_none() {
            on_edge = true;
        }
        if reg.edge_w[0] != 0 && self.get_chunk_from_chunk_pos(wx - 1, wy).is_none() {
            on_edge = true;
        }
        if reg.edge_e[7] != 0 && self.get_chunk_from_chunk_pos(wx + 1, wy).is_none() {
            on_edge = true;
        }
        on_edge
    }
}
