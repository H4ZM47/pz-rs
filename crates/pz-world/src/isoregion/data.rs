//! `zombie.iso.areas.isoregion.data`: `DataRoot`, `DataCell`, `DataChunk`
//! and `DataRoot.SelectInfo`.
//!
//! A `DataChunk` keeps one flag byte per square (`IsoRegions.BIT_*`) for
//! each active level and, after `recalculate`, the index of the chunk
//! region each square belongs to. The region build is the original's:
//! a flood fill per level inside the chunk (`floodFill`), linking with the
//! four neighbour chunks along their shared edges (`link`), world regions
//! grown over the connected chunk regions (`interConnect`) and the roof
//! count from the top level down (`recalcRoofs`).
//!
//! Chunks live in an arena of the root ([`ChunkIdx`]); `DataChunk` methods
//! that reach the root (dirty queues, region manager, neighbour chunks) are
//! methods of [`DataRoot`] taking the chunk index.

use std::collections::{HashMap, HashSet, VecDeque};

use super::regions::{ChunkRegionRef, IsoRegionManager, WorldRegionRef};
use super::{get_opposite_dir, hash};

/// A `DataChunk` object (its slot in [`DataRoot`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkIdx(pub u32);

/// `IsoRegions.BIT_WALL_N`.
pub const BIT_WALL_N: i8 = 1;
/// `IsoRegions.BIT_WALL_W`.
pub const BIT_WALL_W: i8 = 2;
/// `IsoRegions.BIT_PATH_WALL_N`.
pub const BIT_PATH_WALL_N: i8 = 4;
/// `IsoRegions.BIT_PATH_WALL_W`.
pub const BIT_PATH_WALL_W: i8 = 8;
/// `IsoRegions.BIT_HAS_FLOOR`.
pub const BIT_HAS_FLOOR: i8 = 16;
/// `IsoRegions.BIT_STAIRCASE`.
pub const BIT_STAIRCASE: i8 = 32;
/// `IsoRegions.BIT_HAS_ROOF`.
pub const BIT_HAS_ROOF: i8 = 64;

/// `PZMath.coordmodulo`.
fn coordmodulo(v: i32, m: i32) -> i32 {
    v.rem_euclid(m)
}

/// `PZMath.fastfloor((float) v / 32.0f)`.
fn floor_div32_f(v: i32) -> i32 {
    ((v as f32) / 32.0).floor() as i32
}

fn coord_1d(x: i32, y: i32, z: i32) -> usize {
    (z * 64 + y * 8 + x) as usize
}

/// `DataChunk`.
#[derive(Debug)]
pub struct DataChunk {
    hash_id: i32,
    chunk_x: i32,
    chunk_y: i32,
    highest_z: i32,
    last_update_stamp: i64,
    active_z_layers: [bool; 32],
    dirty_z_layers: [bool; 32],
    square_flags: Option<Vec<i8>>,
    region_ids: Option<Vec<i8>>,
    chunk_regions: Vec<Vec<ChunkRegionRef>>,
}

impl DataChunk {
    fn new(chunk_x: i32, chunk_y: i32, chunk_id: i32) -> Self {
        DataChunk {
            hash_id: if chunk_id < 0 {
                hash(chunk_x, chunk_y)
            } else {
                chunk_id
            },
            chunk_x,
            chunk_y,
            highest_z: 0,
            last_update_stamp: 0,
            active_z_layers: [false; 32],
            dirty_z_layers: [false; 32],
            square_flags: None,
            region_ids: None,
            chunk_regions: vec![Vec::new(); 32],
        }
    }
    /// `getHashId()`.
    pub fn hash_id(&self) -> i32 {
        self.hash_id
    }
    /// `getChunkX()`.
    pub fn chunk_x(&self) -> i32 {
        self.chunk_x
    }
    /// `getChunkY()`.
    pub fn chunk_y(&self) -> i32 {
        self.chunk_y
    }
    /// `getCellX()` (rounds down, unlike the `chunkx / 32` the root's
    /// lookups use).
    pub fn cell_x(&self) -> i32 {
        floor_div32_f(self.chunk_x)
    }
    /// `getCellY()`.
    pub fn cell_y(&self) -> i32 {
        floor_div32_f(self.chunk_y)
    }
    /// `highestZ`.
    pub fn highest_z(&self) -> i32 {
        self.highest_z
    }
    /// `getLastUpdateStamp()`.
    pub fn last_update_stamp(&self) -> i64 {
        self.last_update_stamp
    }
    /// `setLastUpdateStamp(stamp)`.
    pub fn set_last_update_stamp(&mut self, s: i64) {
        self.last_update_stamp = s;
    }
    /// `activeZLayers[z]`.
    pub fn is_active(&self, z: i32) -> bool {
        (0..32).contains(&z) && self.active_z_layers[z as usize]
    }
    /// `getChunkRegions(z)`.
    pub fn chunk_regions(&self, z: i32) -> &[ChunkRegionRef] {
        &self.chunk_regions[z as usize]
    }
    /// `isDirty(z)`.
    fn is_dirty(&self, z: i32) -> bool {
        self.active_z_layers[z as usize] && self.dirty_z_layers[z as usize]
    }
    fn valid_coords(&self, x: i32, y: i32, z: i32) -> bool {
        (0..8).contains(&x) && (0..8).contains(&y) && z >= 0 && z < self.highest_z + 1
    }

    /// `getSquare(x, y, z)`: the square's flags, or -1 (no data or an
    /// inactive level). Out-of-range levels, which throw upstream, give -1.
    pub fn get_square(&self, x: i32, y: i32, z: i32) -> i8 {
        self.get_square_checked(x, y, z, false)
    }

    /// `getSquare(x, y, z, ignoreCoordCheck)`.
    pub fn get_square_checked(&self, x: i32, y: i32, z: i32, ignore_coord_check: bool) -> i8 {
        let Some(f) = &self.square_flags else {
            return -1;
        };
        if !(ignore_coord_check || self.valid_coords(x, y, z)) || !(0..32).contains(&z) {
            return -1;
        }
        if self.active_z_layers[z as usize] {
            f.get(coord_1d(x, y, z)).copied().unwrap_or(-1)
        } else {
            -1
        }
    }

    /// `squareGetFlags(x, y, z)`: the raw byte (0 past the arrays).
    pub fn square_get_flags(&self, x: i32, y: i32, z: i32) -> i8 {
        self.raw_flags(coord_1d(x, y, z))
    }

    fn raw_flags(&self, c: usize) -> i8 {
        self.square_flags
            .as_ref()
            .and_then(|f| f.get(c).copied())
            .unwrap_or(0)
    }

    fn square_has_flags(&self, c: usize, flags: i8) -> bool {
        self.raw_flags(c) & flags == flags
    }

    /// `squareCanConnect(coord1D, z, dir)`.
    fn square_can_connect(&self, c: usize, z: i32, dir: i8) -> bool {
        if z >= 0 && z < self.highest_z + 1 {
            return match dir {
                0 => !self.square_has_flags(c, BIT_WALL_N),
                1 => !self.square_has_flags(c, BIT_WALL_W),
                2 | 3 => true,
                4 => !self.square_has_flags(c, BIT_HAS_ROOF),
                5 => !self.square_has_flags(c, BIT_HAS_FLOOR),
                _ => false,
            };
        }
        false
    }

    /// `getIsoChunkRegion(coord1D, z)`.
    fn region_at(&self, c: usize, z: i32) -> Option<ChunkRegionRef> {
        if z < 0 || z > self.highest_z || z >= 32 {
            return None;
        }
        let id = *self.region_ids.as_ref()?.get(c)?;
        if id >= 0 && (id as usize) < self.chunk_regions[z as usize].len() {
            return Some(self.chunk_regions[z as usize][id as usize]);
        }
        None
    }

    /// `getIsoChunkRegion(x, y, z)`.
    pub fn get_iso_chunk_region(&self, x: i32, y: i32, z: i32) -> Option<ChunkRegionRef> {
        self.region_at(coord_1d(x, y, z), z)
    }

    /// `regionIds[coord]` (-1 before the level is recalculated).
    pub fn region_id(&self, x: i32, y: i32, z: i32) -> i8 {
        self.region_ids
            .as_ref()
            .and_then(|r| r.get(coord_1d(x, y, z)).copied())
            .unwrap_or(-1)
    }

    /// `setRegion(x, y, z, regionIndex)`.
    pub fn set_region(&mut self, x: i32, y: i32, z: i32, region_index: i8) {
        if let Some(r) = self.region_ids.as_mut() {
            if let Some(v) = r.get_mut(coord_1d(x, y, z)) {
                *v = region_index;
            }
        }
    }

    /// `ensureSquareArray(zlayer)`.
    fn ensure_square_array(&mut self, z: i32) {
        let size = ((z + 1) * 64) as usize;
        if self.square_flags.as_ref().is_none_or(|f| f.len() < size) {
            let mut flags = vec![0i8; size];
            let mut regions = vec![0i8; size];
            if let (Some(of), Some(or)) = (&self.square_flags, &self.region_ids) {
                flags[..of.len()].copy_from_slice(of);
                regions[..or.len()].copy_from_slice(or);
            }
            self.square_flags = Some(flags);
            self.region_ids = Some(regions);
        }
    }

    /// `ensureSquares(zlayer)`.
    fn ensure_squares(&mut self, z: i32) {
        if !(0..32).contains(&z) {
            return;
        }
        if !self.active_z_layers[z as usize] {
            self.ensure_square_array(z);
            self.active_z_layers[z as usize] = true;
            if z > self.highest_z {
                self.highest_z = z;
            }
            let f = self.square_flags.as_mut().unwrap();
            for c in f[(z * 64) as usize..((z + 1) * 64) as usize].iter_mut() {
                *c = if z == 0 { BIT_HAS_FLOOR } else { 0 };
            }
        }
    }

    /// `save(bb)`: `[length][highestZ][byte count][flags...]`. A chunk
    /// whose arrays are shorter than `highestZ` says (which throws while
    /// writing upstream) stops after the bytes it has, with the length left
    /// at 0, as the original's caught exception leaves it.
    pub fn save(&self, bb: &mut Vec<u8>) {
        let start = bb.len();
        bb.extend_from_slice(&0i32.to_be_bytes());
        bb.extend_from_slice(&self.highest_z.to_be_bytes());
        let max_bytes = (self.highest_z + 1) * 64;
        bb.extend_from_slice(&max_bytes.to_be_bytes());
        for i in 0..max_bytes.max(0) as usize {
            match self.square_flags.as_ref().and_then(|f| f.get(i)) {
                Some(&b) => bb.push(b as u8),
                None => return,
            }
        }
        let len = (bb.len() - start) as i32;
        bb[start..start + 4].copy_from_slice(&len.to_be_bytes());
    }

    /// `load(bb, worldVersion, readLength)`. Reads past the end of `bb` or
    /// past the flag array stop the load where the original's exception
    /// would.
    pub fn load(&mut self, bb: &[u8], pos: &mut usize, read_length: bool) {
        let get_int = |pos: &mut usize| -> Option<i32> {
            let b = bb.get(*pos..*pos + 4)?;
            *pos += 4;
            Some(i32::from_be_bytes(b.try_into().unwrap()))
        };
        if read_length && get_int(pos).is_none() {
            return;
        }
        let Some(hz) = get_int(pos) else {
            return;
        };
        self.highest_z = hz;
        let mut z = hz;
        while z >= 0 {
            self.ensure_squares(z);
            z -= 1;
        }
        let Some(max_bytes) = get_int(pos) else {
            return;
        };
        for i in 0..max_bytes.max(0) as usize {
            let Some(&b) = bb.get(*pos) else {
                return;
            };
            let Some(slot) = self.square_flags.as_mut().and_then(|f| f.get_mut(i)) else {
                return;
            };
            *slot = b as i8;
            *pos += 1;
        }
    }
}

/// `DataCell`: the data chunks of one 32x32-chunk cell, by chunk hash.
#[derive(Debug, Default)]
pub struct DataCell {
    data_chunks: HashMap<i32, ChunkIdx>,
    /// Insertion order of `data_chunks` (see [`DataRoot::get_all_chunks`]).
    order: Vec<i32>,
}

impl DataCell {
    fn get_chunk(&self, id: i32) -> Option<ChunkIdx> {
        self.data_chunks.get(&id).copied()
    }
    fn put(&mut self, id: i32, c: ChunkIdx) {
        if self.data_chunks.insert(id, c).is_none() {
            self.order.push(id);
        }
    }
}

/// `DataRoot.SelectInfo` after `reset`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectInfo {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub chunk_square_x: i32,
    pub chunk_square_y: i32,
    pub chunkx: i32,
    pub chunky: i32,
    pub cellx: i32,
    pub celly: i32,
    pub chunk_id: i32,
    pub cell_id: i32,
    pub has_cell: bool,
    pub chunk: Option<ChunkIdx>,
    pub square: i8,
}

/// `DataRoot`.
#[derive(Debug, Default)]
pub struct DataRoot {
    cell_map: HashMap<i32, DataCell>,
    cell_order: Vec<i32>,
    chunks: Vec<DataChunk>,
    /// `regionManager` (also holding `dirtyIsoWorldRegions`).
    pub region_manager: IsoRegionManager,
    dirty_chunks: Vec<ChunkIdx>,
    /// `changedCells`: cells whose world regions changed, for
    /// `WorldRegionToMetaGrid.clientProcessBuildings` (not ported).
    pub changed_cells: Vec<(i32, i32)>,
    /// `recalcs`, `floodFills`, `merges` of the last `processDirtyChunks`.
    pub recalcs: i32,
    pub flood_fills: i32,
    pub merges: i32,
}

impl DataRoot {
    pub fn new() -> Self {
        Self::default()
    }

    /// The data chunk object `c`.
    pub fn chunk(&self, c: ChunkIdx) -> &DataChunk {
        &self.chunks[c.0 as usize]
    }
    fn ch(&mut self, c: ChunkIdx) -> &mut DataChunk {
        &mut self.chunks[c.0 as usize]
    }

    /// `getAllChunks(list)`. The original walks its `HashMap`s, so its
    /// order is the hash table's; this gives cells and chunks in insertion
    /// order.
    pub fn get_all_chunks(&self) -> Vec<ChunkIdx> {
        let mut out = Vec::new();
        for cid in &self.cell_order {
            let cell = &self.cell_map[cid];
            for id in &cell.order {
                out.push(cell.data_chunks[id]);
            }
        }
        out
    }

    fn add_cell(&mut self, cell_id: i32) {
        if self.cell_map.insert(cell_id, DataCell::default()).is_none() {
            self.cell_order.push(cell_id);
        }
    }

    fn add_chunk(&mut self, cell_id: i32, chunkx: i32, chunky: i32, chunk_id: i32) -> ChunkIdx {
        self.chunks.push(DataChunk::new(chunkx, chunky, chunk_id));
        let c = ChunkIdx(self.chunks.len() as u32 - 1);
        self.cell_map.get_mut(&cell_id).unwrap().put(chunk_id, c);
        c
    }

    /// `getDataChunk(chunkx, chunky)`.
    pub fn get_data_chunk(&self, chunkx: i32, chunky: i32) -> Option<ChunkIdx> {
        let cell = self.cell_map.get(&hash(chunkx / 32, chunky / 32))?;
        cell.get_chunk(hash(chunkx, chunky))
    }

    /// `SelectInfo.reset(x, y, z, createChunk, createSquare)`.
    pub fn select(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        create_chunk: bool,
        create_square: bool,
    ) -> SelectInfo {
        let mut s = SelectInfo {
            x,
            y,
            z,
            chunk_square_x: coordmodulo(x, 8),
            chunk_square_y: coordmodulo(y, 8),
            chunkx: x / 8,
            chunky: y / 8,
            cellx: x / 256,
            celly: y / 256,
            ..Default::default()
        };
        s.chunk_id = hash(s.chunkx, s.chunky);
        s.cell_id = hash(s.cellx, s.celly);
        s.square = -1;
        // `ensureSquare(createSquare)`.
        self.select_ensure_chunk(&mut s, create_square);
        if let Some(c) = s.chunk {
            if s.square == -1 {
                s.square =
                    self.chunk(c)
                        .get_square_checked(s.chunk_square_x, s.chunk_square_y, z, true);
            }
            if s.square == -1 && create_square {
                s.square =
                    self.set_or_add_square(c, s.chunk_square_x, s.chunk_square_y, z, 0, true);
            }
        }
        if s.chunk.is_none() && create_chunk {
            self.select_ensure_chunk(&mut s, create_chunk);
        }
        s
    }

    /// `ensureCell` + `ensureChunk`.
    fn select_ensure_chunk(&mut self, s: &mut SelectInfo, create: bool) {
        if !s.has_cell {
            s.has_cell = self.cell_map.contains_key(&s.cell_id);
        }
        if !s.has_cell && create {
            self.add_cell(s.cell_id);
            s.has_cell = true;
        }
        if !s.has_cell {
            return;
        }
        if s.chunk.is_none() {
            s.chunk = self.cell_map[&s.cell_id].get_chunk(s.chunk_id);
        }
        if s.chunk.is_none() && create {
            s.chunk = Some(self.add_chunk(s.cell_id, s.chunkx, s.chunky, s.chunk_id));
        }
    }

    /// `getIsoWorldRegion(x, y, z)` (the lookup never creates anything:
    /// `reset(x, y, z, false)`).
    pub fn get_iso_world_region(&self, x: i32, y: i32, z: i32) -> Option<WorldRegionRef> {
        let cr = self.get_iso_chunk_region(x, y, z)?;
        self.region_manager.chunk_region(cr).iso_world_region()
    }

    /// `getIsoChunkRegion(x, y, z)`.
    pub fn get_iso_chunk_region(&self, x: i32, y: i32, z: i32) -> Option<ChunkRegionRef> {
        let c = self.get_data_chunk_for_square(x, y)?;
        self.chunk(c)
            .get_iso_chunk_region(coordmodulo(x, 8), coordmodulo(y, 8), z)
    }

    fn get_data_chunk_for_square(&self, x: i32, y: i32) -> Option<ChunkIdx> {
        let cell = self.cell_map.get(&hash(x / 256, y / 256))?;
        cell.get_chunk(hash(x / 8, y / 8))
    }

    /// `getSquareFlags(x, y, z)`.
    pub fn get_square_flags(&self, x: i32, y: i32, z: i32) -> i8 {
        match self.get_data_chunk_for_square(x, y) {
            Some(c) => {
                self.chunk(c)
                    .get_square_checked(coordmodulo(x, 8), coordmodulo(y, 8), z, true)
            }
            None => -1,
        }
    }

    /// `getIsoWorldRegionsInCell(cellX, cellY, list)` (chunks in insertion
    /// order; see [`DataRoot::get_all_chunks`]).
    pub fn get_iso_world_regions_in_cell(&self, cell_x: i32, cell_y: i32) -> Vec<WorldRegionRef> {
        let mut out = Vec::new();
        let Some(cell) = self.cell_map.get(&hash(cell_x, cell_y)) else {
            return out;
        };
        for id in &cell.order {
            let c = self.chunk(cell.data_chunks[id]);
            for z in 0..32 {
                for &cr in c.chunk_regions(z) {
                    if let Some(w) = self.region_manager.chunk_region(cr).iso_world_region() {
                        if !out.contains(&w) {
                            out.push(w);
                        }
                    }
                }
            }
        }
        out
    }

    /// `resetAllData()` (cells and chunks in insertion order).
    pub fn reset_all_data(&mut self) {
        let mut world_regions = Vec::new();
        for cid in self.cell_order.clone() {
            let cell = &self.cell_map[&cid];
            let ids: Vec<ChunkIdx> = cell.order.iter().map(|i| cell.data_chunks[i]).collect();
            for c in ids {
                for z in 0..32 {
                    let regions = self.chunk(c).chunk_regions[z].clone();
                    for r in regions {
                        if let Some(w) = self.region_manager.chunk_region(r).iso_world_region() {
                            if !world_regions.contains(&w) {
                                world_regions.push(w);
                            }
                        }
                        self.region_manager.cr(r).set_iso_world_region(None);
                        self.region_manager.release_iso_chunk_region(r);
                    }
                }
            }
        }
        self.cell_map.clear();
        self.cell_order.clear();
        for w in world_regions {
            self.region_manager.release_iso_world_region(w);
        }
    }

    /// `EnqueueDirtyDataChunk(chunk)`.
    fn enqueue_dirty_data_chunk(&mut self, c: ChunkIdx) {
        if !self.dirty_chunks.contains(&c) {
            self.dirty_chunks.push(c);
        }
    }

    /// `EnqueueDirtyIsoWorldRegion(mr)`.
    fn enqueue_dirty_iso_world_region(&mut self, w: WorldRegionRef) {
        let d = &mut self.region_manager.dirty_iso_world_regions;
        if !d.contains(&w) {
            d.push(w);
        }
    }

    /// `DataChunk.setDirty(z)`.
    fn set_dirty(&mut self, c: ChunkIdx, z: i32) {
        if self.chunk(c).active_z_layers[z as usize] {
            self.ch(c).dirty_z_layers[z as usize] = true;
            self.enqueue_dirty_data_chunk(c);
        }
    }

    /// `DataChunk.setDirtyAllActive()`.
    pub(crate) fn set_dirty_all_active(&mut self, c: ChunkIdx) {
        let mut queued = false;
        for z in 0..32 {
            if !self.chunk(c).active_z_layers[z] {
                continue;
            }
            self.ch(c).dirty_z_layers[z] = true;
            if queued {
                continue;
            }
            self.enqueue_dirty_data_chunk(c);
            queued = true;
        }
    }

    /// `DataChunk.setOrAddSquare(x, y, z, flags, ignoreCoordCheck)`.
    pub(crate) fn set_or_add_square(
        &mut self,
        c: ChunkIdx,
        x: i32,
        y: i32,
        z: i32,
        flags: i8,
        ignore_coord_check: bool,
    ) -> i8 {
        if !(ignore_coord_check || self.chunk(c).valid_coords(x, y, z)) {
            return -1;
        }
        self.ch(c).ensure_squares(z);
        let id = coord_1d(x, y, z);
        let Some(cur) = self
            .chunk(c)
            .square_flags
            .as_ref()
            .and_then(|f| f.get(id).copied())
        else {
            // Out of the array: `ArrayIndexOutOfBoundsException` upstream.
            return -1;
        };
        if cur != flags {
            self.set_dirty(c, z);
        }
        self.ch(c).square_flags.as_mut().unwrap()[id] = flags;
        flags
    }

    /// `updateExistingSquare(x, y, z, flags)`.
    pub(crate) fn update_existing_square(&mut self, x: i32, y: i32, z: i32, flags: i8) {
        let s = self.select(x, y, z, false, false);
        if let Some(c) = s.chunk {
            let buffer_flags = s.square;
            if flags == buffer_flags {
                return;
            }
            self.set_or_add_square(c, s.chunk_square_x, s.chunk_square_y, s.z, flags, true);
        } else {
            self.region_manager.warnings.push(
                "DataRoot.updateExistingSquare -> trying to change a square on a unknown chunk"
                    .into(),
            );
        }
    }

    /// `DataChunk.load` of `c` from `bb` at `pos` (`readLength` set).
    pub(crate) fn load_chunk(&mut self, c: ChunkIdx, bb: &[u8], pos: &mut usize) {
        self.ch(c).load(bb, pos, true);
    }

    /// `processDirtyChunks()`.
    pub fn process_dirty_chunks(&mut self) {
        if self.dirty_chunks.is_empty() {
            return;
        }
        self.recalcs = 0;
        self.flood_fills = 0;
        self.merges = 0;
        let dirty = self.dirty_chunks.clone();
        for &c in &dirty {
            // `clearBuildingDefs`: regions never get a `BuildingDef` here
            // (`WorldRegionToMetaGrid` is not ported), so only the cell is
            // recorded.
            let cell = (self.chunk(c).cell_x(), self.chunk(c).cell_y());
            if !self.changed_cells.contains(&cell) {
                self.changed_cells.push(cell);
            }
        }
        for &c in &dirty {
            self.recalculate(c);
            self.recalcs += 1;
        }
        for &c in &dirty {
            let (cx, cy) = (self.chunk(c).chunk_x, self.chunk(c).chunk_y);
            let n = self.get_data_chunk(cx, cy - 1);
            let w = self.get_data_chunk(cx - 1, cy);
            let s = self.get_data_chunk(cx, cy + 1);
            let e = self.get_data_chunk(cx + 1, cy);
            self.link(c, n, w, s, e);
        }
        for &c in &dirty {
            self.inter_connect(c);
        }
        for &c in &dirty {
            self.recalc_roofs(c);
            self.ch(c).dirty_z_layers = [false; 32];
        }
        if !self.region_manager.dirty_iso_world_regions.is_empty() {
            let mut done: HashSet<WorldRegionRef> = HashSet::new();
            let dw = self.region_manager.dirty_iso_world_regions.clone();
            for &w in &dw {
                self.region_manager.unlink_world_neighbors(w);
            }
            for &w in &dw {
                self.region_manager.link_world_neighbors(w);
            }
            for &w in &dw {
                let wc = self.world_region_cell(w);
                if !done.contains(&w) {
                    if let Some(wc) = wc {
                        if !self.changed_cells.contains(&wc) {
                            self.changed_cells.push(wc);
                        }
                    }
                    done.insert(w);
                }
                let ns = self.region_manager.world_region(w).neighbors().to_vec();
                for n in ns {
                    if done.contains(&n) {
                        continue;
                    }
                    // Upstream tests the dirty region's cell and adds the
                    // neighbour's.
                    if wc.is_some_and(|wc| !self.changed_cells.contains(&wc)) {
                        if let Some(nc) = self.world_region_cell(n) {
                            self.changed_cells.push(nc);
                        }
                    }
                    done.insert(n);
                }
            }
            self.region_manager.dirty_iso_world_regions.clear();
        }
        self.dirty_chunks.clear();
    }

    /// `IsoWorldRegion.getCellX/Y()` (`None` where the original throws for
    /// a region without chunk regions).
    pub fn world_region_cell(&self, w: WorldRegionRef) -> Option<(i32, i32)> {
        let first = *self
            .region_manager
            .world_region(w)
            .chunk_regions()
            .first()?;
        let c = self.chunk(self.region_manager.chunk_region(first).data_chunk()?);
        Some((c.cell_x(), c.cell_y()))
    }

    /// `DataChunk.recalculate()`.
    fn recalculate(&mut self, c: ChunkIdx) {
        for z in 0..=self.chunk(c).highest_z.min(31) {
            if !self.chunk(c).dirty_z_layers[z as usize]
                || !self.chunk(c).active_z_layers[z as usize]
            {
                continue;
            }
            self.recalculate_z(c, z);
        }
    }

    fn recalculate_z(&mut self, c: ChunkIdx, z: i32) {
        let mut i = self.chunk(c).chunk_regions[z as usize].len();
        while i > 0 {
            i -= 1;
            let r = self.chunk(c).chunk_regions[z as usize][i];
            if let Some(mr) = self.region_manager.unlink_from_iso_world_region(r) {
                if self.region_manager.world_region(mr).size() == 0 {
                    self.region_manager.release_iso_world_region(mr);
                }
            }
            self.region_manager.release_iso_chunk_region(r);
            self.ch(c).chunk_regions[z as usize].remove(i);
        }
        self.ch(c).chunk_regions[z as usize].clear();
        if let Some(r) = self.ch(c).region_ids.as_mut() {
            let lo = (z * 64) as usize;
            for v in r[lo..lo + 64].iter_mut() {
                *v = -1;
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                if self.chunk(c).region_id(x, y, z) != -1 {
                    continue;
                }
                self.flood_fill(c, x, y, z);
            }
        }
    }

    /// `DataChunk.floodFill(x, y, z)`.
    fn flood_fill(&mut self, c: ChunkIdx, x: i32, y: i32, z: i32) -> ChunkRegionRef {
        let region = self.region_manager.alloc_iso_chunk_region(c, z);
        let region_index = self.chunk(c).chunk_regions[z as usize].len() as i8;
        self.ch(c).chunk_regions[z as usize].push(region);
        let mut explored = [false; 64];
        let mut tmp_squares: VecDeque<(i32, i32, i32)> = VecDeque::new();
        let mut tmp_linked: HashSet<i32> = HashSet::new();
        tmp_squares.push_back((x, y, z));
        let zbase = (z * 64) as usize;
        while let Some(pos) = tmp_squares.pop_front() {
            let coord = coord_1d(pos.0, pos.1, pos.2);
            explored[coord - zbase] = true;
            if self.chunk(c).region_ids.as_ref().unwrap()[coord] != -1 {
                continue;
            }
            self.ch(c).region_ids.as_mut().unwrap()[coord] = region_index;
            self.region_manager.cr(region).add_square_count();
            for dir in 0..4i8 {
                if let Some(nb) = get_neighbor(pos, dir) {
                    let ncoord = coord_1d(nb.0, nb.1, nb.2);
                    if explored[ncoord - zbase] {
                        continue;
                    }
                    let ch = self.chunk(c);
                    if ch.square_can_connect(coord, pos.2, dir)
                        && ch.square_can_connect(ncoord, nb.2, get_opposite_dir(dir))
                    {
                        if ch.region_ids.as_ref().unwrap()[ncoord] == -1 {
                            tmp_squares.push_back(nb);
                            explored[ncoord - zbase] = true;
                            continue;
                        }
                    } else if let Some(nc) = ch.region_at(ncoord, nb.2) {
                        if nc != region {
                            let nid = self.region_manager.chunk_region(nc).id();
                            if !tmp_linked.contains(&nid) {
                                self.region_manager.cr(region).add_neighbor(nc);
                                self.region_manager.cr(nc).add_neighbor(region);
                                tmp_linked.insert(nid);
                            }
                            explored[ncoord - zbase] = true;
                            continue;
                        }
                    }
                    continue;
                }
                if !self.chunk(c).square_can_connect(coord, pos.2, dir) {
                    continue;
                }
                self.region_manager
                    .cr(region)
                    .add_chunk_border_squares_cnt();
            }
        }
        region
    }

    /// `DataChunk.link(n, w, s, e)`.
    fn link(
        &mut self,
        c: ChunkIdx,
        n: Option<ChunkIdx>,
        w: Option<ChunkIdx>,
        s: Option<ChunkIdx>,
        e: Option<ChunkIdx>,
    ) {
        for z in 0..=self.chunk(c).highest_z.min(31) {
            if !self.chunk(c).dirty_z_layers[z as usize]
                || !self.chunk(c).active_z_layers[z as usize]
            {
                continue;
            }
            self.link_regions_on_side(c, z, n, 0);
            self.link_regions_on_side(c, z, w, 1);
            self.link_regions_on_side(c, z, s, 2);
            self.link_regions_on_side(c, z, e, 3);
        }
    }

    /// `DataChunk.linkRegionsOnSide(z, opposite, dir)`.
    fn link_regions_on_side(&mut self, c: ChunkIdx, z: i32, opposite: Option<ChunkIdx>, dir: i8) {
        let (x_start, x_max, y_start, y_max) = if dir == 0 || dir == 2 {
            let ys = if dir == 0 { 0 } else { 7 };
            (0, 8, ys, ys + 1)
        } else {
            let xs = if dir == 1 { 0 } else { 7 };
            (xs, xs + 1, 0, 8)
        };
        let odir = get_opposite_dir(dir);
        if let Some(o) = opposite {
            if (0..32).contains(&z) && self.chunk(o).is_dirty(z) {
                // `resetEnclosedSide(z, opposite dir)`.
                let regions = self.chunk(o).chunk_regions[z as usize].clone();
                for r in regions {
                    if self.region_manager.chunk_region(r).z_layer() != z {
                        continue;
                    }
                    self.region_manager.cr(r).set_enclosed(odir, true);
                }
            }
        }
        let mut last_cur: Option<ChunkRegionRef> = None;
        let mut last_other_full: Option<ChunkRegionRef> = None;
        for y in y_start..y_max {
            for x in x_start..x_max {
                let (xo, yo) = if dir == 0 || dir == 2 {
                    (x, if dir == 0 { 7 } else { 0 })
                } else {
                    (if dir == 1 { 7 } else { 0 }, y)
                };
                let coord = coord_1d(x, y, z);
                let ocoord = coord_1d(xo, yo, z);
                let cur = self.chunk(c).region_at(coord, z);
                let other = opposite.and_then(|o| self.chunk(o).region_at(ocoord, z));
                let Some(cur) = cur else {
                    self.region_manager
                        .warnings
                        .push("ds.getRegion()==null, shouldnt happen at this point.".into());
                    continue;
                };
                if last_cur.is_some() && last_cur != Some(cur) {
                    last_other_full = None;
                }
                if last_cur == Some(cur) && other.is_some() && last_other_full == other {
                    continue;
                }
                match (opposite, other) {
                    (Some(o), Some(other)) => {
                        let m = &mut self.region_manager;
                        if self.chunks[c.0 as usize].square_can_connect(coord, z, dir)
                            && self.chunks[o.0 as usize].square_can_connect(ocoord, z, odir)
                        {
                            m.cr(cur).add_connected_neighbor(other);
                            m.cr(other).add_connected_neighbor(cur);
                            m.cr(cur).add_neighbor(other);
                            m.cr(other).add_neighbor(cur);
                            if !m.chunk_region_is_enclosed(other) {
                                m.cr(other).set_enclosed(odir, true);
                            }
                            last_other_full = Some(other);
                        } else {
                            m.cr(cur).add_neighbor(other);
                            m.cr(other).add_neighbor(cur);
                            if !m.chunk_region_is_enclosed(other) {
                                m.cr(other).set_enclosed(odir, true);
                            }
                            last_other_full = None;
                        }
                    }
                    _ => {
                        if self.chunk(c).square_can_connect(coord, z, dir) {
                            self.region_manager.cr(cur).set_enclosed(dir, false);
                        }
                    }
                }
                last_cur = Some(cur);
            }
        }
    }

    /// `DataChunk.interConnect()`.
    fn inter_connect(&mut self, c: ChunkIdx) {
        for z in 0..=self.chunk(c).highest_z.min(31) {
            if !self.chunk(c).dirty_z_layers[z as usize]
                || !self.chunk(c).active_z_layers[z as usize]
            {
                continue;
            }
            let mut i = 0;
            while i < self.chunk(c).chunk_regions[z as usize].len() {
                let region = self.chunk(c).chunk_regions[z as usize][i];
                i += 1;
                let m = &self.region_manager;
                if m.chunk_region(region).z_layer() != z
                    || m.chunk_region(region).iso_world_region().is_some()
                {
                    continue;
                }
                if m.chunk_region(region).connected_neighbors().is_empty() {
                    let ms = self.region_manager.alloc_iso_world_region();
                    self.enqueue_dirty_iso_world_region(ms);
                    self.region_manager.add_iso_chunk_region(ms, region);
                    continue;
                }
                if let Some(neighbor) = m.connected_neighbor_with_largest_iso_world_region(region) {
                    let largest = m.chunk_region(neighbor).iso_world_region().unwrap();
                    let old_list = self.region_manager.take_iso_chunk_regions(largest);
                    for &r in &old_list {
                        self.region_manager.cr(r).set_iso_world_region(None);
                    }
                    self.region_manager.release_iso_world_region(largest);
                    let target = self.region_manager.alloc_iso_world_region();
                    self.enqueue_dirty_iso_world_region(target);
                    self.flood_fill_expand_world_region(region, target);
                    for &r in &old_list {
                        if self
                            .region_manager
                            .chunk_region(r)
                            .iso_world_region()
                            .is_some()
                        {
                            continue;
                        }
                        let mr = self.region_manager.alloc_iso_world_region();
                        self.enqueue_dirty_iso_world_region(mr);
                        self.flood_fill_expand_world_region(r, mr);
                    }
                    self.flood_fills += 1;
                    continue;
                }
                let ms = self.region_manager.alloc_iso_world_region();
                self.enqueue_dirty_iso_world_region(ms);
                self.flood_fill_expand_world_region(region, ms);
                self.flood_fills += 1;
            }
        }
    }

    /// `DataChunk.floodFillExpandWorldRegion(start, worldRegion)`.
    fn flood_fill_expand_world_region(&mut self, start: ChunkRegionRef, w: WorldRegionRef) {
        let mut queue: VecDeque<ChunkRegionRef> = VecDeque::new();
        queue.push_back(start);
        while let Some(cur) = queue.pop_front() {
            self.region_manager.add_iso_chunk_region(w, cur);
            let mut i = 0;
            while i < self
                .region_manager
                .chunk_region(cur)
                .connected_neighbors()
                .len()
            {
                let n = self.region_manager.chunk_region(cur).connected_neighbors()[i];
                i += 1;
                if queue.contains(&n) {
                    continue;
                }
                match self.region_manager.chunk_region(n).iso_world_region() {
                    None => queue.push_back(n),
                    Some(nw) if nw == w => {}
                    Some(nw) => self.region_manager.merge(w, nw),
                }
            }
        }
    }

    /// `DataChunk.recalcRoofs()`.
    fn recalc_roofs(&mut self, c: ChunkIdx) {
        let hz = self.chunk(c).highest_z;
        if hz < 1 {
            return;
        }
        for zl in 0..32 {
            let regions = self.chunk(c).chunk_regions[zl].clone();
            for r in regions {
                self.region_manager.chunk_region_reset_roof_cnt(r);
            }
        }
        let z = hz;
        for y in 0..8 {
            for x in 0..8 {
                let flags = self.chunk(c).get_square(x, y, z);
                let mut hasroof = false;
                if flags > 0 {
                    hasroof = self
                        .chunk(c)
                        .square_has_flags(coord_1d(x, y, z), BIT_HAS_FLOOR);
                }
                let mut zz = z - 1;
                while zz >= 0 {
                    let ch = self.chunk(c);
                    let flags = ch.get_square(x, y, zz);
                    let region = ch.get_iso_chunk_region(x, y, zz);
                    let world =
                        region.and_then(|r| self.region_manager.chunk_region(r).iso_world_region());
                    let enclosed_region =
                        world.is_some_and(|w| self.region_manager.world_region_is_enclosed(w));
                    if flags > 0 || enclosed_region {
                        hasroof = hasroof
                            || self
                                .chunk(c)
                                .square_has_flags(coord_1d(x, y, zz), BIT_STAIRCASE);
                        if hasroof {
                            if let Some(r) = region {
                                self.region_manager.chunk_region_add_roof(r);
                                if let Some(w) =
                                    self.region_manager.chunk_region(r).iso_world_region()
                                {
                                    if !self.region_manager.world_region_is_enclosed(w) {
                                        hasroof = false;
                                    }
                                }
                            } else {
                                hasroof = false;
                            }
                        }
                        if !hasroof {
                            hasroof = self
                                .chunk(c)
                                .square_has_flags(coord_1d(x, y, zz), BIT_HAS_FLOOR);
                        }
                    } else {
                        hasroof = false;
                    }
                    zz -= 1;
                }
            }
        }
    }
}

/// `DataChunk.getNeighbor(pos, dir)` inside the chunk.
fn get_neighbor(pos: (i32, i32, i32), dir: i8) -> Option<(i32, i32, i32)> {
    let (mut x, mut y) = (pos.0, pos.1);
    match dir {
        1 => x -= 1,
        3 => x += 1,
        0 => y -= 1,
        2 => y += 1,
        _ => {}
    }
    if !(0..8).contains(&x) || !(0..8).contains(&y) {
        return None;
    }
    Some((x, y, pos.2))
}
