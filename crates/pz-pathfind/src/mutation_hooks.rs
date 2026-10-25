//! The pathfinder side of `pz_world::square_mutation`'s
//! `PathfindSquareHooks`: square edits and new chunk levels reaching
//! [`PathfindService`] the way `PathfindNative.squareChanged` and
//! `addChunkToWorld` do.
//!
//! The trait lives on a pz-world branch this crate is not built against
//! yet, so [`PathfindMutationHooks`] has the methods and
//! [`pathfind_square_hooks!`](crate::pathfind_square_hooks) defines a
//! wrapper implementing the trait in a crate that has both:
//!
//! ```ignore
//! pz_pathfind::pathfind_square_hooks!(pz_world);
//! let mut hooks = PathfindSquareHooksImpl(PathfindMutationHooks { .. });
//! ```

use pz_world::chunk::IsoCell;
use pz_world::{IsoMetaGrid, IsoSpriteManager};

use crate::service::PathfindService;
use crate::square_bits::{SquareBits, SquareBitsHooks};

/// What a square mutation tells the pathfinder.
pub struct PathfindMutationHooks<'a> {
    pub service: &'a mut PathfindService,
    pub sprites: &'a IsoSpriteManager,
    pub meta: &'a IsoMetaGrid,
    /// Barricades and other object state for `getBits` (usually
    /// `pz_world::ObjectHooks` over the same cell).
    pub bits_hooks: &'a dyn SquareBitsHooks,
}

impl PathfindMutationHooks<'_> {
    /// `PathfindNative.squareChanged(sq)`: update tasks for each existing
    /// neighbour (`IsoDirections` order) and the square, built now from
    /// the cell. Squares of chunks the pathfinder was not given are
    /// skipped (`loadedBits & 2`).
    pub fn square_changed(&mut self, cell: &IsoCell, sq: (i32, i32, i32)) {
        let Some(square) = cell.get_grid_square(sq.0, sq.1, sq.2) else {
            return;
        };
        let Ok(bits) = SquareBits::new(self.sprites, self.meta, cell, self.bits_hooks) else {
            return;
        };
        self.service.square_changed(square, &bits);
    }

    /// `PathfindNative.addChunkToWorld(chunk)` after `IsoChunk.setSquare`
    /// added a level: the chunk's `ChunkUpdateTask` built now, under the
    /// load id the chunk was added with.
    pub fn add_chunk_to_world(&mut self, cell: &IsoCell, wx: i32, wy: i32) {
        let Some(chunk) = cell.chunks.get(&(wx, wy)) else {
            return;
        };
        let Ok(bits) = SquareBits::new(self.sprites, self.meta, cell, self.bits_hooks) else {
            return;
        };
        let load_id = self.service.chunk_load_id(wx, wy).unwrap_or(0);
        self.service.add_chunk_to_world(chunk, load_id, &bits);
    }
}

/// Defines `PathfindSquareHooksImpl<'a>`, a local wrapper around
/// [`PathfindMutationHooks`], and implements
/// `$world::square_mutation::PathfindSquareHooks` for it (`$world` is the
/// `pz_world` crate path; the wrapper is needed because neither the trait
/// nor `PathfindMutationHooks` is local to the invoking crate). The
/// MapCollisionData notification is the population manager's and is left
/// at the trait's default.
#[macro_export]
macro_rules! pathfind_square_hooks {
    ($($world:tt)+) => {
        /// `PathfindMutationHooks` as pz-world's `PathfindSquareHooks`.
        pub struct PathfindSquareHooksImpl<'a>(pub $crate::mutation_hooks::PathfindMutationHooks<'a>);

        impl $($world)+::square_mutation::PathfindSquareHooks for PathfindSquareHooksImpl<'_> {
            fn square_changed(&mut self, cell: &$($world)+::chunk::IsoCell, sq: (i32, i32, i32)) {
                self.0.square_changed(cell, sq)
            }

            fn add_chunk_to_world(&mut self, cell: &$($world)+::chunk::IsoCell, wx: i32, wy: i32) {
                self.0.add_chunk_to_world(cell, wx, wy)
            }
        }
    };
}
