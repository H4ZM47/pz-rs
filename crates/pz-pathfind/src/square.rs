//! `Square`: one walkable cell of the pathfinding grid, holding the
//! 64-bit flag word produced by the game's `SquareUpdateTask.getBits` and a
//! traversal cost.
//!
//! The predicate methods mirror the native `Square::` methods; several of
//! them look at the neighbouring squares through the world, so they take
//! `&World`.

use crate::util::{lerp, Dir};
use crate::world::{SquareId, World};

/// Square flag bits (see `SquareUpdateTask` in the game).
pub mod bits {
    pub const SOLID: u64 = 1;
    pub const COLLIDE_W: u64 = 2;
    pub const COLLIDE_N: u64 = 4;
    pub const STAIR_TW: u64 = 8;
    pub const STAIR_MW: u64 = 0x10;
    pub const STAIR_BW: u64 = 0x20;
    pub const STAIR_TN: u64 = 0x40;
    pub const STAIR_MN: u64 = 0x80;
    pub const STAIR_BN: u64 = 0x100;
    pub const SOLID_FLOOR: u64 = 0x200;
    pub const SOLID_TRANS: u64 = 0x400;
    pub const WINDOW_W: u64 = 0x800;
    pub const WINDOW_N: u64 = 0x1000;
    pub const CAN_PATH_W: u64 = 0x2000;
    pub const CAN_PATH_N: u64 = 0x4000;
    pub const THUMP_W: u64 = 0x8000;
    pub const THUMP_N: u64 = 0x10000;
    pub const THUMPABLE: u64 = 0x20000;
    pub const DOOR_E: u64 = 0x40000;
    pub const DOOR_S: u64 = 0x80000;
    pub const WINDOW_W_UNBLOCKED: u64 = 0x100000;
    pub const WINDOW_N_UNBLOCKED: u64 = 0x200000;
    pub const DOOR_W_UNBLOCKED: u64 = 0x400000;
    pub const DOOR_N_UNBLOCKED: u64 = 0x800000;
    pub const HOPPABLE_N: u64 = 0x1000000;
    pub const HOPPABLE_W: u64 = 0x2000000;
    pub const TARGET_PATH_W: u64 = 0x4000000;
    pub const TARGET_PATH_N: u64 = 0x8000000;
    pub const BENDABLE_W: u64 = 0x10000000;
    pub const BENDABLE_N: u64 = 0x20000000;
    pub const TALL_HOPPABLE_N: u64 = 0x40000000;
    pub const TALL_HOPPABLE_W: u64 = 0x80000000;
    pub const FARMING_PLANT: u64 = 0x100000000;
    pub const ALL_STAIRS: u64 = 0x1f8;
    pub const ALL_SOLID: u64 = SOLID | SOLID_TRANS;
}

use bits::*;

/// `Square`.
#[derive(Clone, Debug)]
pub struct Square {
    pub x: i32,
    pub y: i32,
    /// Level with the +32 offset used everywhere inside the pathfinder.
    pub z: i32,
    pub bits: u64,
    pub cost: i16,
    pub slope_dir: Option<Dir>,
    pub slope_min: f32,
    pub slope_max: f32,
}

impl Square {
    pub(crate) fn new(x: i32, y: i32, z: i32) -> Self {
        Square {
            x,
            y,
            z,
            bits: 0,
            cost: 0,
            slope_dir: None,
            slope_min: 0.0,
            slope_max: 0.0,
        }
    }

    #[inline]
    pub fn has(&self, mask: u64) -> bool {
        self.bits & mask != 0
    }

    /// `TreatAsSolidFloor`.
    #[inline]
    pub fn treat_as_solid_floor(&self) -> bool {
        self.bits & (SOLID_FLOOR | ALL_STAIRS) != 0
    }

    fn neighbour<'a>(&self, w: &'a World, dx: i32, dy: i32) -> Option<&'a Square> {
        w.get_square(self.x + dx, self.y + dy, self.z)
            .map(|id| w.sq(id))
    }

    fn adjacent_to_window_or_hoppable(&self, w: &World) -> bool {
        // isAdjacentToWindow() || isAdjacentToHoppable(), with the own
        // window bits already known to be clear.
        if let Some(s) = self.neighbour(w, 0, 1) {
            if s.has(WINDOW_N) {
                return true;
            }
        }
        if let Some(e) = self.neighbour(w, 1, 0) {
            if e.has(WINDOW_W) {
                return true;
            }
        }
        if self.has(HOPPABLE_N | HOPPABLE_W) {
            return true;
        }
        if let Some(s) = self.neighbour(w, 0, 1) {
            if s.has(HOPPABLE_N) {
                return true;
            }
        }
        if let Some(e) = self.neighbour(w, 1, 0) {
            if e.has(HOPPABLE_W) {
                return true;
            }
        }
        false
    }

    /// `isReallySolid`.
    pub fn is_really_solid(&self, w: &World) -> bool {
        if self.has(SOLID) {
            return true;
        }
        if self.bits & (SOLID_TRANS | WINDOW_W | WINDOW_N) == SOLID_TRANS {
            return !self.adjacent_to_window_or_hoppable(w);
        }
        false
    }

    /// `isAdjacentToWindow`.
    pub fn is_adjacent_to_window(&self, w: &World) -> bool {
        if self.has(WINDOW_W | WINDOW_N) {
            return true;
        }
        if let Some(s) = self.neighbour(w, 0, 1) {
            if s.has(WINDOW_N) {
                return true;
            }
        }
        if let Some(e) = self.neighbour(w, 1, 0) {
            if e.has(WINDOW_W) {
                return true;
            }
        }
        false
    }

    /// `isAdjacentToHoppable`.
    pub fn is_adjacent_to_hoppable(&self, w: &World) -> bool {
        if self.has(HOPPABLE_N | HOPPABLE_W) {
            return true;
        }
        if let Some(s) = self.neighbour(w, 0, 1) {
            if s.has(HOPPABLE_N) {
                return true;
            }
        }
        if let Some(e) = self.neighbour(w, 1, 0) {
            if e.has(HOPPABLE_W) {
                return true;
            }
        }
        false
    }

    /// `isNonThumpableSolid`.
    pub fn is_non_thumpable_solid(&self, w: &World) -> bool {
        self.is_really_solid(w) && !self.has(THUMPABLE)
    }

    /// `hasSquareThumpable` (native only): a thumpable object filling the
    /// square that is not a window or hoppable.
    pub fn has_square_thumpable(&self, w: &World) -> bool {
        if self.bits & (THUMPABLE | WINDOW_W | WINDOW_N) != THUMPABLE {
            return false;
        }
        !self.adjacent_to_window_or_hoppable(w)
    }

    /// `isCanPathW`.
    pub fn is_can_path_w(&self, w: &World) -> bool {
        if self.has(CAN_PATH_W) {
            return true;
        }
        match self.neighbour(w, -1, 0) {
            Some(sq) => sq.has(THUMPABLE) || sq.has(DOOR_E),
            None => false,
        }
    }

    /// `isCanPathN`.
    pub fn is_can_path_n(&self, w: &World) -> bool {
        if self.has(CAN_PATH_N) {
            return true;
        }
        match self.neighbour(w, 0, -1) {
            Some(sq) => sq.has(THUMPABLE) || sq.has(DOOR_S),
            None => false,
        }
    }

    /// `isCollideW`.
    pub fn is_collide_w(&self, w: &World) -> bool {
        if self.has(COLLIDE_W) {
            return true;
        }
        match self.neighbour(w, -1, 0) {
            None => false,
            Some(sq) => {
                if sq.bits & (DOOR_E | STAIR_TN | STAIR_MN | STAIR_BN) != 0 {
                    true
                } else {
                    sq.is_really_solid(w)
                }
            }
        }
    }

    /// `isCollideN`.
    pub fn is_collide_n(&self, w: &World) -> bool {
        if self.has(COLLIDE_N) {
            return true;
        }
        match self.neighbour(w, 0, -1) {
            None => false,
            Some(sq) => {
                if sq.bits & (DOOR_S | STAIR_TW | STAIR_MW | STAIR_BW) != 0 {
                    true
                } else {
                    sq.is_really_solid(w)
                }
            }
        }
    }

    /// `isThumpW`.
    pub fn is_thump_w(&self, w: &World) -> bool {
        if self.has(THUMP_W) {
            return true;
        }
        self.neighbour(w, -1, 0)
            .map(|sq| sq.has(THUMPABLE))
            .unwrap_or(false)
    }

    /// `isThumpN`.
    pub fn is_thump_n(&self, w: &World) -> bool {
        if self.has(THUMP_N) {
            return true;
        }
        self.neighbour(w, 0, -1)
            .map(|sq| sq.has(THUMPABLE))
            .unwrap_or(false)
    }

    /// `isUnblockedWindowN`.
    pub fn is_unblocked_window_n(&self, w: &World) -> bool {
        if !self.has(WINDOW_N_UNBLOCKED) || self.is_really_solid(w) {
            return false;
        }
        match self.neighbour(w, 0, -1) {
            Some(n) => !n.is_really_solid(w),
            None => false,
        }
    }

    /// `isUnblockedWindowW`.
    pub fn is_unblocked_window_w(&self, w: &World) -> bool {
        if !self.has(WINDOW_W_UNBLOCKED) || self.is_really_solid(w) {
            return false;
        }
        match self.neighbour(w, -1, 0) {
            Some(n) => !n.is_really_solid(w),
            None => false,
        }
    }

    /// `isUnblockedDoorN`.
    pub fn is_unblocked_door_n(&self, w: &World) -> bool {
        if self.bits & (DOOR_N_UNBLOCKED | ALL_SOLID) != DOOR_N_UNBLOCKED {
            return false;
        }
        match self.neighbour(w, 0, -1) {
            Some(n) => n.bits & ALL_SOLID == 0,
            None => false,
        }
    }

    /// `isUnblockedDoorW`.
    pub fn is_unblocked_door_w(&self, w: &World) -> bool {
        if self.bits & (DOOR_W_UNBLOCKED | ALL_SOLID) != DOOR_W_UNBLOCKED {
            return false;
        }
        match self.neighbour(w, -1, 0) {
            Some(n) => n.bits & ALL_SOLID == 0,
            None => false,
        }
    }

    /// `hasSlopedSurface`.
    #[inline]
    pub fn has_sloped_surface(&self) -> bool {
        self.slope_dir.is_some()
    }

    /// `hasIdenticalSlopedSurface`.
    pub fn has_identical_sloped_surface(&self, other: &Square) -> bool {
        self.slope_dir == other.slope_dir
            && self.slope_min == other.slope_min
            && self.slope_max == other.slope_max
    }

    /// `getSlopedSurfaceHeight(IsoDirections edge)`.
    pub fn sloped_surface_height_edge(&self, edge: Dir) -> f32 {
        match self.slope_dir {
            None => 0.0,
            Some(d) if d == edge => self.slope_max,
            Some(d) if d.rot180() == edge => self.slope_min,
            Some(_) => -1.0,
        }
    }

    /// `getSlopedSurfaceHeight(float dx, float dy)` (uses the native
    /// operand order of `lerp`).
    pub fn sloped_surface_height_at(&self, dx: f32, dy: f32) -> f32 {
        let dir = match self.slope_dir {
            None => return 0.0,
            Some(d) => d,
        };
        let dx = crate::util::clamp(dx, 0.0, 1.0);
        let dy = crate::util::clamp(dy, 0.0, 1.0);
        let (lo, hi) = (self.slope_min, self.slope_max);
        match dir {
            Dir::N => lerp(lo, hi, 1.0 - dy),
            Dir::S => lerp(lo, hi, dy),
            Dir::W => lerp(lo, hi, 1.0 - dx),
            Dir::E => lerp(lo, hi, dx),
            _ => 0.0,
        }
    }

    /// `hasSlopedSurfaceToLevelAbove`.
    pub fn has_sloped_surface_to_level_above(&self, edge: Dir) -> bool {
        self.slope_dir.is_some() && self.sloped_surface_height_edge(edge) == 1.0
    }

    /// `hasTransitionToLevelAbove`.
    pub fn has_transition_to_level_above(&self, edge: Dir) -> bool {
        if (edge == Dir::N && self.has(STAIR_TN)) || (edge == Dir::W && self.has(STAIR_TW)) {
            return true;
        }
        self.has_sloped_surface_to_level_above(edge)
    }

    /// `isSlopedSurfaceEdgeBlocked`.
    pub fn is_sloped_surface_edge_blocked(&self, w: &World, edge: Dir) -> bool {
        if self.slope_dir.is_none() {
            return false;
        }
        match self.neighbour(w, edge.dx(), edge.dy()) {
            None => true,
            Some(sq2) => {
                self.sloped_surface_height_edge(edge)
                    != sq2.sloped_surface_height_edge(edge.rot180())
            }
        }
    }

    /// `isInside(x1, y1, x2, y2)`.
    pub fn is_inside(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
        self.x >= x1 && self.x < x2 && self.y >= y1 && self.y < y2
    }
}

impl World {
    /// `Square::getAdjacentSquare`.
    pub(crate) fn adjacent_square(&self, s: SquareId, dir: Dir) -> Option<SquareId> {
        let sq = self.sq(s);
        self.get_square(sq.x + dir.dx(), sq.y + dir.dy(), sq.z)
    }
}
