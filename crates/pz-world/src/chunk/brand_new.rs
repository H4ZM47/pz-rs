//! `IsoChunk.LoadBrandNew`: a chunk with no save file, loaded from the map
//! files together with the other chunks of its 2x2 world-generation block.
//!
//! The parts other systems own run through [`BrandNewHook`], the plug-in
//! point for B42 world generation (`WorldGenChunk.generateChunks`) and
//! procedural basements (`Basements.onNewChunkLoaded`).

use std::collections::HashMap;

use super::{load_cell_binary_chunk, ChunkLoadEnv, IsoChunk, LotCache};
use crate::meta_grid::IsoMetaGrid;

/// `WorldGenUtils.getCornerOfGeneration(b)`: `Math.floorDiv(b, 2) * 2`, the
/// lower corner of the 2x2 block of chunks generated together.
pub fn get_corner_of_generation(b: i32) -> i32 {
    b.div_euclid(2) * 2
}

/// `zombie.iso.worldgen.ChunksCache`: the chunks of one `LoadBrandNew`,
/// keyed by chunk coordinates (`ChunkCoord`), with the bounds `put` keeps.
#[derive(Default)]
pub struct ChunksCache {
    chunks: HashMap<(i32, i32), IsoChunk>,
    /// Keys in `put` order (`LoadBrandNew`: x outer, y inner).
    order: Vec<(i32, i32)>,
    min_chunk_x: Option<i32>,
    max_chunk_x: i32,
    min_chunk_y: i32,
    max_chunk_y: i32,
}

impl ChunksCache {
    /// `put(new ChunkCoord(wx, wy), chunk)`.
    pub fn put(&mut self, wx: i32, wy: i32, chunk: IsoChunk) {
        match self.min_chunk_x {
            None => {
                self.min_chunk_x = Some(wx);
                (self.max_chunk_x, self.min_chunk_y, self.max_chunk_y) = (wx, wy, wy);
            }
            Some(min) => {
                self.min_chunk_x = Some(min.min(wx));
                self.max_chunk_x = self.max_chunk_x.max(wx);
                self.min_chunk_y = self.min_chunk_y.min(wy);
                self.max_chunk_y = self.max_chunk_y.max(wy);
            }
        }
        if self.chunks.insert((wx, wy), chunk).is_none() {
            self.order.push((wx, wy));
        }
    }

    /// `get(new ChunkCoord(wx, wy))`.
    pub fn get(&self, wx: i32, wy: i32) -> Option<&IsoChunk> {
        self.chunks.get(&(wx, wy))
    }

    pub fn get_mut(&mut self, wx: i32, wy: i32) -> Option<&mut IsoChunk> {
        self.chunks.get_mut(&(wx, wy))
    }

    /// The chunk coordinates in `put` order.
    pub fn coords(&self) -> &[(i32, i32)] {
        &self.order
    }

    /// `getMinChunkX()` (`Integer.MAX_VALUE` while empty).
    pub fn min_chunk_x(&self) -> i32 {
        self.min_chunk_x.unwrap_or(i32::MAX)
    }

    /// `getMaxChunkX()` (`Integer.MIN_VALUE` while empty).
    pub fn max_chunk_x(&self) -> i32 {
        self.min_chunk_x.map_or(i32::MIN, |_| self.max_chunk_x)
    }

    /// `getMinChunkY()` (`Integer.MAX_VALUE` while empty).
    pub fn min_chunk_y(&self) -> i32 {
        self.min_chunk_x.map_or(i32::MAX, |_| self.min_chunk_y)
    }

    /// `getMaxChunkY()` (`Integer.MIN_VALUE` while empty).
    pub fn max_chunk_y(&self) -> i32 {
        self.min_chunk_x.map_or(i32::MIN, |_| self.max_chunk_y)
    }

    /// `getMinTileX()`.
    pub fn min_tile_x(&self) -> i32 {
        self.min_chunk_x().wrapping_mul(8)
    }

    /// `getMaxTileX()`.
    pub fn max_tile_x(&self) -> i32 {
        self.max_chunk_x().wrapping_add(1).wrapping_mul(8)
    }

    /// `getMinTileY()`.
    pub fn min_tile_y(&self) -> i32 {
        self.min_chunk_y().wrapping_mul(8)
    }

    /// `getMaxTileY()`.
    pub fn max_tile_y(&self) -> i32 {
        self.max_chunk_y().wrapping_add(1).wrapping_mul(8)
    }

    /// The chunk and in-chunk coordinates `getGridSquare(localX, localY, z)`
    /// reads (Java's truncating `/`).
    fn locate(&self, local_x: i32, local_y: i32) -> ((i32, i32), i32, i32) {
        let (cx, cy) = (local_x / 8, local_y / 8);
        (
            (
                self.min_chunk_x().wrapping_add(cx),
                self.min_chunk_y().wrapping_add(cy),
            ),
            local_x - 8 * cx,
            local_y - 8 * cy,
        )
    }

    /// `getGridSquare(localX, localY, localZ)`: tile coordinates relative to
    /// the cache's minimum tile.
    pub fn get_grid_square(&self, local_x: i32, local_y: i32, z: i32) -> Option<crate::SquareRef> {
        let (k, x, y) = self.locate(local_x, local_y);
        self.chunks.get(&k)?.get_grid_square(x, y, z)
    }

    pub fn get_grid_square_mut(
        &mut self,
        local_x: i32,
        local_y: i32,
        z: i32,
    ) -> Option<crate::SquareRef> {
        let (k, x, y) = self.locate(local_x, local_y);
        self.chunks.get_mut(&k)?.get_grid_square_mut(x, y, z)
    }
}

/// The parts of `IsoChunk.LoadBrandNew` that belong to other systems. This
/// is the plug-in point for B42 world generation: a world-generation port
/// implements [`BrandNewHook::generate_chunks`], and a basements port
/// [`BrandNewHook::on_new_chunk_loaded`].
///
/// [`load_brand_new`] calls them in the original's order, once per brand-new
/// chunk, after all four chunks of the 2x2 block have been loaded from the
/// map files (`CellLoader.LoadCellBinaryChunk`, including tile object
/// creation and its `Rand` draws) and before the chunk is returned:
///
/// 1. `generate_chunks` if [`BrandNewHook::do_worldgen`];
/// 2. `on_new_chunk_loaded` for the requested chunk;
/// 3. [`BrandNewHook::add_zombie_on_cell_load`] decides
///    [`IsoChunk::add_zombies`].
///
/// The squares are not recalculated yet (no `RecalcProperties` or
/// matrices); that is `loadInWorldStreamerThread`
/// ([`IsoCell::recalc_chunk_squares`](super::IsoCell::recalc_chunk_squares))
/// once the chunk is in the cell.
pub trait BrandNewHook {
    /// `IsoWorld.instance.getWgChunk().generateChunks(chunks)`: world
    /// generation over the 2x2 block. The original walks the block from
    /// `chunks.min_chunk_x()/min_chunk_y()` (x outer, y inner) and, per
    /// chunk, runs `genRandomChunk` when `hasEmptySquaresOnLevelZero()` and
    /// otherwise `genMapChunk` and two `cleanChunk` calls. It may change
    /// every chunk in `chunks` (objects, squares, levels, generation
    /// status); all four chunks are generated although only the requested
    /// one is kept.
    fn generate_chunks(
        &mut self,
        chunks: &mut ChunksCache,
        meta: &IsoMetaGrid,
        env: &mut ChunkLoadEnv<'_>,
    );

    /// `Basements.getInstance().onNewChunkLoaded(chunk)`: spawns the
    /// basements placed over `chunk` (`NewMapBinaryFile.SpawnBasementInChunk`).
    /// Only the requested chunk gets this call.
    fn on_new_chunk_loaded(
        &mut self,
        chunk: &mut IsoChunk,
        meta: &IsoMetaGrid,
        env: &mut ChunkLoadEnv<'_>,
    );

    /// `IsoChunk.doWorldgen` (true unless turned off for debugging).
    fn do_worldgen(&self) -> bool {
        true
    }

    /// `!GameClient.client && Core.addZombieOnCellLoad`; the latter is
    /// false in the `Tutorial` and `LastStand` game modes.
    fn add_zombie_on_cell_load(&self) -> bool {
        true
    }
}

/// `IsoChunk.LoadBrandNew(wx, wy)` with a fresh lot cache; see
/// [`load_brand_new_cached`].
pub fn load_brand_new(
    meta: &IsoMetaGrid,
    wx: i32,
    wy: i32,
    env: &mut ChunkLoadEnv<'_>,
    gen: Option<&mut dyn BrandNewHook>,
) -> (IsoChunk, Vec<IsoChunk>) {
    load_brand_new_cached(meta, wx, wy, env, gen, &mut LotCache::default())
}

/// `IsoChunk.LoadBrandNew(wx, wy)`:
///
/// 1. the corner of the 2x2 generation block
///    ([`get_corner_of_generation`] of `wx` and `wy`);
/// 2. `LoadCellBinaryChunk` for each chunk of the block, x outer and y
///    inner, into a [`ChunksCache`] (the requested chunk and three chunks
///    from `chunkStore`);
/// 3. the [`BrandNewHook`] calls (world generation, basements) and
///    `addZombies`;
/// 4. the other three chunks go back to `IsoChunkMap.chunkStore`
///    (`resetForStore`). Here they are returned to the caller instead, in
///    the same order, as world generation left them.
///
/// Without a hook nothing is generated (`doWorldgen` off, no basements)
/// and `addZombies` is set as in single player.
pub fn load_brand_new_cached(
    meta: &IsoMetaGrid,
    wx: i32,
    wy: i32,
    env: &mut ChunkLoadEnv<'_>,
    mut gen: Option<&mut dyn BrandNewHook>,
    cache: &mut LotCache,
) -> (IsoChunk, Vec<IsoChunk>) {
    let wx_t = get_corner_of_generation(wx);
    let wy_t = get_corner_of_generation(wy);
    let mut chunks = ChunksCache::default();
    for i in wx_t..wx_t + 2 {
        for j in wy_t..wy_t + 2 {
            let mut c = IsoChunk::new(i, j);
            load_cell_binary_chunk(meta, i, j, &mut c, env, cache);
            chunks.put(i, j, c);
        }
    }
    if let Some(g) = gen.as_deref_mut() {
        if g.do_worldgen() {
            g.generate_chunks(&mut chunks, meta, env);
        }
    }
    let mut this = chunks.chunks.remove(&(wx, wy)).expect("requested chunk");
    if let Some(g) = gen.as_deref_mut() {
        g.on_new_chunk_loaded(&mut this, meta, env);
    }
    if gen.as_deref().is_none_or(|g| g.add_zombie_on_cell_load()) {
        this.add_zombies = true;
    }
    let others = chunks
        .order
        .iter()
        .filter(|&&k| k != (wx, wy))
        .map(|k| chunks.chunks.remove(k).expect("cached chunk"))
        .collect();
    (this, others)
}

#[cfg(test)]
mod tests;
