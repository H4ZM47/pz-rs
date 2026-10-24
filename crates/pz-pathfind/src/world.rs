//! `World`, `Cell`, `Chunk` and `ChunkLevel`: the square grid the
//! pathfinder works on, plus the JNI-facing entry points that feed it
//! (`initWorld`, `updateChunk`, `removeChunk`, `updateSquare`, `update`).
//!
//! In the native library `World` owns the cells and vehicles while
//! `PolygonalMap2` (a separate singleton) owns the visibility graphs and
//! the A* state. Both are singletons that reach each other through globals,
//! so this port keeps all of that state in one [`World`] value; the
//! `PolygonalMap2` methods live in [`crate::polygonal_map2`].

use std::collections::BTreeMap;

use crate::bytes::{ByteReader, PathfindError};
use crate::high_level::hl_chunk_level::HlChunkLevel;
use crate::high_level::HighLevelState;
use crate::line_clear_collide::LineClearCollide;
use crate::pool::{Id, Pool};
use crate::request::PmMover;
use crate::square::Square;
use crate::util::{coorddivision, Dir};
use crate::vgastar::VgAStar;
use crate::visibility::VisState;

pub type SquareId = Id<Square>;
pub type ChunkId = Id<Chunk>;

/// Squares per chunk side.
pub const CPW: i32 = 8;
/// Chunks per cell side.
pub const CHUNKS_PER_CELL: i32 = 32;
/// Squares per cell side.
pub const SQUARES_PER_CELL: i32 = 256;

/// `ChunkLevel`: the 8x8 squares of one chunk on one level, plus the
/// high-level (region / staircase) data derived from them.
pub struct ChunkLevel {
    /// The chunk this level belongs to.
    pub chunk: ChunkId,
    /// Level, with the +32 offset.
    pub z: i32,
    /// `ChunkSquares`, indexed `[x][y]`.
    pub(crate) squares: [[Option<SquareId>; 8]; 8],
    pub(crate) hl: HlChunkLevel,
}

impl ChunkLevel {
    fn new(chunk: ChunkId, z: i32) -> Self {
        ChunkLevel {
            chunk,
            z,
            squares: [[None; 8]; 8],
            hl: HlChunkLevel::new(),
        }
    }
}

/// `Chunk`.
pub struct Chunk {
    pub(crate) wx: i32,
    pub(crate) wy: i32,
    pub(crate) min_level: i32,
    pub(crate) max_level: i32,
    /// Levels `min_level..=max_level`.
    pub(crate) levels: Vec<Option<ChunkLevel>>,
    /// `IsoChunk.getLoadID()` of the chunk that filled this one, -1 when
    /// unset.
    pub(crate) load_id: i16,
    pub(crate) visibility_graphs: Vec<crate::visibility::VGraphId>,
}

impl Chunk {
    #[inline]
    pub fn min_x(&self) -> i32 {
        self.wx * CPW
    }
    #[inline]
    pub fn min_y(&self) -> i32 {
        self.wy * CPW
    }
    #[inline]
    pub fn max_x(&self) -> i32 {
        (self.wx + 1) * CPW - 1
    }
    #[inline]
    pub fn max_y(&self) -> i32 {
        (self.wy + 1) * CPW - 1
    }
    #[inline]
    pub fn is_valid_level(&self, level: i32) -> bool {
        level >= self.min_level && level <= self.max_level
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.min_x() && y >= self.min_y() && x <= self.max_x() && y <= self.max_y()
    }
    pub(crate) fn level(&self, level: i32) -> Option<&ChunkLevel> {
        if self.is_valid_level(level) {
            self.levels[(level - self.min_level) as usize].as_ref()
        } else {
            None
        }
    }
    pub(crate) fn level_mut(&mut self, level: i32) -> Option<&mut ChunkLevel> {
        if self.is_valid_level(level) {
            self.levels[(level - self.min_level) as usize].as_mut()
        } else {
            None
        }
    }
}

/// `Cell`: a 32x32 block of chunk slots.
pub struct Cell {
    pub(crate) cx: i32,
    pub(crate) cy: i32,
    pub(crate) chunks: Option<Box<[Option<ChunkId>]>>,
}

/// A chunk level, identified by chunk and level. Stands in for the native
/// `HLChunkLevel*` in lists such as the allowed chunk levels of a mover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LevelRef {
    pub chunk: ChunkId,
    pub z: i32,
}

/// Which mover a `PolygonalMap2::canMoveBetween` call uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoverSel {
    Request,
    FloodFill,
}

/// The whole native pathfinder state: `World` + `PolygonalMap2` +
/// `HLGlobals`.
pub struct World {
    pub(crate) is_server: bool,
    pub(crate) min_x: i32,
    pub(crate) min_y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) cells: Vec<Cell>,
    pub(crate) chunks: Pool<Chunk>,
    pub(crate) squares: Pool<Square>,

    /// `PolygonalMap2` rebuild flag, set by every world change and consumed
    /// by the next `findPath`.
    pub(crate) rebuild: bool,
    /// `HLAStar::ModificationCount`.
    pub(crate) modification_count: i32,

    /// Visibility graphs, vehicles and their geometry.
    pub(crate) vis: VisState,
    /// `PolygonalMap2::squareToNode`, keyed by square id.
    pub(crate) square_to_node: BTreeMap<u32, crate::visibility::NodeId>,

    /// Low-level search (`VGAStar`).
    pub(crate) vga: VgAStar,
    /// The mover of the request being processed.
    pub(crate) request_mover: PmMover,
    /// `FloodFill`'s private mover.
    pub(crate) flood_mover: PmMover,

    /// High-level search state (`HLGlobals`, `HLAStar`, `FloodFill`).
    pub(crate) hl: HighLevelState,

    pub(crate) lcc: LineClearCollide,
}

impl World {
    /// `PathfindNative.initWorld(minX, minY, width, height, isServer)`;
    /// the bounds are in cells (256x256 squares) as reported by
    /// `IsoMetaGrid`.
    pub fn new(min_x: i32, min_y: i32, width: i32, height: i32, is_server: bool) -> World {
        let mut cells = Vec::new();
        for y in 0..height.max(0) {
            for x in 0..width.max(0) {
                cells.push(Cell {
                    cx: min_x + x,
                    cy: min_y + y,
                    chunks: None,
                });
            }
        }
        let flood_mover = PmMover {
            kind: crate::request::MoverKind::Zombie,
            ..PmMover::default()
        };
        World {
            is_server,
            min_x,
            min_y,
            width,
            height,
            cells,
            chunks: Pool::default(),
            squares: Pool::default(),
            rebuild: false,
            modification_count: 0,
            vis: VisState::default(),
            square_to_node: BTreeMap::new(),
            vga: VgAStar::default(),
            request_mover: PmMover::default(),
            flood_mover,
            hl: HighLevelState::default(),
            lcc: LineClearCollide::default(),
        }
    }

    /// Whether the world was created for a server.
    pub fn is_server(&self) -> bool {
        self.is_server
    }

    #[inline]
    pub(crate) fn sq(&self, id: SquareId) -> &Square {
        &self.squares[id]
    }

    #[inline]
    pub(crate) fn chunk(&self, id: ChunkId) -> &Chunk {
        &self.chunks[id]
    }

    pub(crate) fn chunk_level(&self, r: LevelRef) -> &ChunkLevel {
        self.chunks[r.chunk]
            .level(r.z)
            .expect("chunk level must exist")
    }

    pub(crate) fn chunk_level_mut(&mut self, r: LevelRef) -> &mut ChunkLevel {
        self.chunks[r.chunk]
            .level_mut(r.z)
            .expect("chunk level must exist")
    }

    fn cell_index(&self, x: i32, y: i32) -> Option<usize> {
        let lx = x.wrapping_sub(self.min_x.wrapping_mul(SQUARES_PER_CELL));
        let ly = y.wrapping_sub(self.min_y.wrapping_mul(SQUARES_PER_CELL));
        if lx < 0 || ly < 0 {
            return None;
        }
        let (cx, cy) = (lx >> 8, ly >> 8);
        if cx >= self.width || cy >= self.height {
            return None;
        }
        Some((cx + self.width * cy) as usize)
    }

    /// `World::getChunkFromChunkPos`.
    pub(crate) fn get_chunk_from_chunk_pos(&self, wx: i32, wy: i32) -> Option<ChunkId> {
        let ci = self.cell_index(wx.wrapping_mul(CPW), wy.wrapping_mul(CPW))?;
        cell_chunk(&self.cells[ci], wx, wy)
    }

    /// `World::getChunkFromSquarePos`.
    pub(crate) fn get_chunk_from_square_pos(&self, x: i32, y: i32) -> Option<ChunkId> {
        let ci = self.cell_index(x, y)?;
        cell_chunk(
            &self.cells[ci],
            coorddivision(x, CPW),
            coorddivision(y, CPW),
        )
    }

    /// `PolygonalMap2::getSquare` / `Chunk::getSquare`.
    pub(crate) fn get_square(&self, x: i32, y: i32, z: i32) -> Option<SquareId> {
        let c = self.get_chunk_from_square_pos(x, y)?;
        self.chunk_get_square(c, x, y, z)
    }

    pub(crate) fn chunk_get_square(&self, c: ChunkId, x: i32, y: i32, z: i32) -> Option<SquareId> {
        let chunk = &self.chunks[c];
        let level = chunk.level(z)?;
        let lx = x.wrapping_sub(chunk.min_x());
        let ly = y.wrapping_sub(chunk.min_y());
        if !(0..8).contains(&lx) || !(0..8).contains(&ly) {
            return None;
        }
        level.squares[lx as usize][ly as usize]
    }

    /// The squares of one chunk level, `[x][y]`.
    pub(crate) fn squares_for_level(
        &self,
        c: ChunkId,
        z: i32,
    ) -> Option<&[[Option<SquareId>; 8]; 8]> {
        self.chunks[c].level(z).map(|l| &l.squares)
    }

    /// `World::allocChunkIfNeeded`.
    fn alloc_chunk_if_needed(&mut self, wx: i32, wy: i32) -> Option<ChunkId> {
        let ci = self.cell_index(wx.wrapping_mul(CPW), wy.wrapping_mul(CPW))?;
        let (cx, cy) = (self.cells[ci].cx, self.cells[ci].cy);
        let lx = wx - cx * CHUNKS_PER_CELL;
        let ly = wy - cy * CHUNKS_PER_CELL;
        if !(0..CHUNKS_PER_CELL).contains(&lx) || !(0..CHUNKS_PER_CELL).contains(&ly) {
            return None;
        }
        let slot = (lx | (ly * CHUNKS_PER_CELL)) as usize;
        if let Some(existing) = self.cells[ci].chunks.as_ref().and_then(|c| c[slot]) {
            return Some(existing);
        }
        let id = self.chunks.alloc(Chunk {
            wx: lx | (cx << 5),
            wy: ly | (cy << 5),
            min_level: 32,
            max_level: 32,
            levels: Vec::new(),
            load_id: -1,
            visibility_graphs: Vec::new(),
        });
        self.chunks[id].levels = vec![Some(ChunkLevel::new(id, 32))];
        let cell = &mut self.cells[ci];
        let chunks = cell
            .chunks
            .get_or_insert_with(|| vec![None; (CHUNKS_PER_CELL * CHUNKS_PER_CELL) as usize].into());
        chunks[slot] = Some(id);
        Some(id)
    }

    /// Release everything a chunk level owns (`ChunkLevel::clear`).
    fn clear_chunk_level(&mut self, level: ChunkLevel) {
        for col in level.squares.iter() {
            for s in col.iter().flatten() {
                self.squares.release(*s);
            }
        }
        let mut hl = level.hl;
        self.hl_remove_from_world(&mut hl);
    }

    /// `Chunk::setMinMaxLevel`.
    fn chunk_set_min_max_level(&mut self, c: ChunkId, min_level: i32, max_level: i32) {
        let (old_min, old_max) = {
            let ch = &self.chunks[c];
            (ch.min_level, ch.max_level)
        };
        if min_level == old_min && max_level == old_max {
            return;
        }
        let mut old_levels = std::mem::take(&mut self.chunks[c].levels);
        for z in old_min..=old_max {
            if z < min_level || z > max_level {
                if let Some(level) = old_levels[(z - old_min) as usize].take() {
                    self.clear_chunk_level(level);
                }
            }
        }
        let mut new_levels = Vec::new();
        for z in min_level..=max_level {
            if z >= old_min && z <= old_max {
                new_levels.push(old_levels[(z - old_min) as usize].take());
            } else {
                new_levels.push(Some(ChunkLevel::new(c, z)));
            }
        }
        let ch = &mut self.chunks[c];
        ch.min_level = min_level;
        ch.max_level = max_level;
        ch.levels = new_levels;
    }

    /// `ChunkLevel::setBits`: (re)initialise the square at chunk-local
    /// `x, y`, or remove it when `bits` is zero. Resets any sloped surface.
    fn chunk_level_set_bits(&mut self, c: ChunkId, z: i32, x: i32, y: i32, bits: u64, cost: i16) {
        if !(0..8).contains(&x) || !(0..8).contains(&y) {
            return;
        }
        let (min_x, min_y) = {
            let ch = &self.chunks[c];
            (ch.min_x(), ch.min_y())
        };
        let existing = match self.chunks[c].level(z) {
            Some(l) => l.squares[x as usize][y as usize],
            None => return,
        };
        if bits == 0 {
            if let Some(s) = existing {
                self.squares.release(s);
                self.chunks[c].level_mut(z).unwrap().squares[x as usize][y as usize] = None;
            }
            return;
        }
        let mut sq = Square::new(min_x + x, min_y + y, z);
        sq.bits = bits;
        sq.cost = cost;
        match existing {
            Some(s) => self.squares[s] = sq,
            None => {
                let s = self.squares.alloc(sq);
                self.chunks[c].level_mut(z).unwrap().squares[x as usize][y as usize] = Some(s);
            }
        }
    }

    /// `PathfindNative.updateChunk(loadId, wx, wy, bb)`: replace the
    /// contents of a chunk with a buffer built by `ChunkUpdateTask.init`.
    ///
    /// Layout (big-endian): `i32 minLevel+32`, `i32 maxLevel+32`, then for
    /// every level, row (y) and column (x) an `i64` flag word and an `i16`
    /// cost, then an `i16` sloped-surface count followed by
    /// `u8 x, u8 y, u8 z+32, u8 direction, f32 min, f32 max` records.
    ///
    /// As in the native code, an update whose `load_id` does not match the
    /// one the chunk was first filled with is ignored.
    pub fn update_chunk(
        &mut self,
        load_id: i32,
        wx: i32,
        wy: i32,
        bytes: &[u8],
    ) -> Result<(), PathfindError> {
        let c = self
            .alloc_chunk_if_needed(wx, wy)
            .ok_or(PathfindError::InvalidChunkCoordinates)?;
        {
            let ch = &mut self.chunks[c];
            if ch.load_id == -1 {
                ch.load_id = load_id as i16;
            } else if load_id != ch.load_id as i32 {
                return Ok(());
            }
        }
        let mut bb = ByteReader::big_endian(bytes);
        let min_level = bb.get_i32()?;
        let max_level = bb.get_i32()?;
        self.chunk_set_min_max_level(c, min_level, max_level);
        let mut z = min_level;
        while z <= max_level {
            for y in 0..8 {
                for x in 0..8 {
                    let bits = bb.get_square_bits()?;
                    let cost = bb.get_i16()?;
                    self.chunk_level_set_bits(c, z, x, y, bits, cost);
                }
            }
            z += 1;
        }
        let count = bb.get_i16()? as i32;
        for _ in 0..count.max(0) {
            let x = bb.get_u8()? as i8 as i32;
            let y = bb.get_u8()? as i8 as i32;
            let level = bb.get_u8()? as i8 as i32;
            if !(0..8).contains(&x) || !(0..8).contains(&y) {
                return Err(PathfindError::InvalidChunkSquares);
            }
            let dir = bb.get_u8()? as i8 as i32;
            let min = bb.get_f32()?;
            let max = bb.get_f32()?;
            let s = self.chunks[c]
                .level(level)
                .and_then(|l| l.squares[x as usize][y as usize]);
            if let Some(s) = s {
                let sq = &mut self.squares[s];
                sq.slope_dir = Dir::from_index(dir);
                sq.slope_min = min;
                sq.slope_max = max;
            }
        }
        self.rebuild = true;
        Ok(())
    }

    /// `PathfindNative.removeChunk(wx, wy)`.
    pub fn remove_chunk(&mut self, wx: i32, wy: i32) -> Result<(), PathfindError> {
        let ci = self
            .cell_index(wx.wrapping_mul(CPW), wy.wrapping_mul(CPW))
            .ok_or(PathfindError::InvalidChunkCoordinates)?;
        let (cx, cy) = (self.cells[ci].cx, self.cells[ci].cy);
        let lx = wx - cx * CHUNKS_PER_CELL;
        let ly = wy - cy * CHUNKS_PER_CELL;
        if !(0..CHUNKS_PER_CELL).contains(&lx) || !(0..CHUNKS_PER_CELL).contains(&ly) {
            return Ok(());
        }
        let slot = (lx | (ly * CHUNKS_PER_CELL)) as usize;
        let id = match self.cells[ci].chunks.as_mut().and_then(|c| c[slot].take()) {
            Some(id) => id,
            None => return Ok(()),
        };
        let levels = std::mem::take(&mut self.chunks[id].levels);
        for level in levels.into_iter().flatten() {
            self.clear_chunk_level(level);
        }
        self.chunks[id].visibility_graphs.clear();
        self.chunks.release(id);
        self.rebuild = true;
        Ok(())
    }

    /// `PathfindNative.updateSquare(loadId, x, y, z+32, bits, cost,
    /// slopeDirection, slopeMin, slopeMax)`; `slope_dir` is an
    /// `IsoDirections` ordinal, 8 for none.
    #[allow(clippy::too_many_arguments)]
    pub fn update_square(
        &mut self,
        load_id: i32,
        x: i32,
        y: i32,
        z: i32,
        bits: i64,
        cost: i16,
        slope_dir: i32,
        slope_min: f32,
        slope_max: f32,
    ) {
        let c = match self.get_chunk_from_square_pos(x, y) {
            Some(c) => c,
            None => return,
        };
        if load_id != self.chunks[c].load_id as i32 {
            return;
        }
        let (min_x, min_y) = (self.chunks[c].min_x(), self.chunks[c].min_y());
        self.chunk_level_set_bits(c, z, x - min_x, y - min_y, bits as u64, cost);
        if let Some(s) = self.chunk_get_square(c, x, y, z) {
            let sq = &mut self.squares[s];
            sq.slope_dir = Dir::from_index(slope_dir);
            sq.slope_min = slope_min;
            sq.slope_max = slope_max;
        }
        self.rebuild = true;
    }

    /// `PathfindNative.update()`. In the shipped library chunk and square
    /// updates are applied immediately by `updateChunk` / `updateSquare`,
    /// so this does nothing; it exists for API parity.
    pub fn update(&mut self) {}

    /// The mover the low-level search uses (`VGAStar::mover`): the mover
    /// of the request being processed.
    #[inline]
    pub(crate) fn mover(&self) -> &PmMover {
        &self.request_mover
    }

    /// Number of chunks currently loaded (for diagnostics and tests).
    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.live()
    }
}

fn cell_chunk(cell: &Cell, wx: i32, wy: i32) -> Option<ChunkId> {
    let chunks = cell.chunks.as_ref()?;
    let lx = wx.wrapping_sub(cell.cx * CHUNKS_PER_CELL);
    let ly = wy.wrapping_sub(cell.cy * CHUNKS_PER_CELL);
    if (lx as u32) < 32 && (ly as u32) < 32 {
        chunks[(lx | (ly * 32)) as usize]
    } else {
        None
    }
}
