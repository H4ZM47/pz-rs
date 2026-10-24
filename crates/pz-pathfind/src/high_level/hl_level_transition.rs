//! `HLLevelTransition` and its two kinds, `HLStaircase` and
//! `HLSlopedSurface`: a way from one level to the one above.

use super::TransId;
use crate::util::Dir;
use crate::world::{ChunkId, SquareId, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Staircase,
    SlopedSurface,
}

/// A staircase (direction N or W) or a sloped surface reaching the level
/// above (direction N, S, W or E). `x, y, z` is the bottom square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HlLevelTransition {
    pub kind: TransitionKind,
    pub dir: Dir,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl HlLevelTransition {
    pub fn bottom_floor_x(&self) -> i32 {
        self.x
    }
    pub fn bottom_floor_y(&self) -> i32 {
        self.y
    }
    pub fn bottom_floor_z(&self) -> i32 {
        self.z
    }
    pub fn top_floor_x(&self) -> i32 {
        self.x + self.dir.dx()
    }
    pub fn top_floor_y(&self) -> i32 {
        self.y + self.dir.dy()
    }
    pub fn top_floor_z(&self) -> i32 {
        self.z + 1
    }
    /// `getSearchNodeX(bBottom)`.
    pub fn search_node_x(&self, bottom: bool) -> f32 {
        (if bottom {
            self.bottom_floor_x()
        } else {
            self.top_floor_x()
        }) as f32
            + 0.5
    }
    /// `getSearchNodeY(bBottom)`.
    pub fn search_node_y(&self, bottom: bool) -> f32 {
        (if bottom {
            self.bottom_floor_y()
        } else {
            self.top_floor_y()
        }) as f32
            + 0.5
    }
    pub fn is_bottom_floor_at(&self, x: i32, y: i32) -> bool {
        x == self.bottom_floor_x() && y == self.bottom_floor_y()
    }
}

impl World {
    #[inline]
    pub(crate) fn trans(&self, t: TransId) -> &HlLevelTransition {
        &self.hl.transitions[t]
    }

    /// `HLLevelTransition::getBottomFloorSquare`.
    pub(crate) fn trans_bottom_floor_square(&self, t: TransId) -> Option<SquareId> {
        let tr = self.trans(t);
        self.get_square(
            tr.bottom_floor_x(),
            tr.bottom_floor_y(),
            tr.bottom_floor_z(),
        )
    }

    /// `HLLevelTransition::getTopFloorSquare`.
    pub(crate) fn trans_top_floor_square(&self, t: TransId) -> Option<SquareId> {
        let tr = self.trans(t);
        self.get_square(tr.top_floor_x(), tr.top_floor_y(), tr.top_floor_z())
    }

    /// `HLLevelTransition::getBottomFloorChunk`.
    pub(crate) fn trans_bottom_floor_chunk(&self, t: TransId) -> Option<ChunkId> {
        let tr = self.trans(t);
        self.get_chunk_from_square_pos(tr.bottom_floor_x(), tr.bottom_floor_y())
    }

    /// `HLLevelTransition::getTopFloorChunk`.
    pub(crate) fn trans_top_floor_chunk(&self, t: TransId) -> Option<ChunkId> {
        let tr = self.trans(t);
        self.get_chunk_from_square_pos(tr.top_floor_x(), tr.top_floor_y())
    }
}
