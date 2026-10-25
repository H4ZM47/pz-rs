//! `zombie.iso.areas.isoregion`: the world regions behind player-built
//! rooms (`IsoRegions`, `IsoRegionWorker`, the region jobs and their
//! `isoregiondata` save files).
//!
//! # What builds regions
//!
//! Each square gets a flag byte from its properties
//! ([`calculate_square_flags`]: floor, north/west walls and whether they
//! are windows or door walls, stairs). Per chunk level, squares that
//! connect without crossing a wall form an `IsoChunkRegion`; chunk regions
//! that connect across chunk edges form an `IsoWorldRegion`, which is a
//! "player room" (`isFogMask`) when no edge of it opens to an unloaded or
//! regionless chunk (`isEnclosed`) and every square has a roof above it
//! (`isFullyRoofed`). See [`data`] and [`regions`].
//!
//! The original only reads chunks into the region data in three places,
//! and this port has the same three entry points:
//!
//! * [`IsoRegions::init`] (`IsoWorld.init`) loads the region data the save
//!   holds (`isoregiondata/datachunk_<x>_<y>.bin`, `RegionHeader.bin`);
//! * [`IsoRegions::square_changed`] (`IsoGridSquare.AddTileObject`,
//!   `RemoveTileObject`, `AddSpecialObject`, `addFloor`, ...): a changed
//!   square of a chunk seen before updates that square; otherwise the
//!   loaded chunks in a 7x7 chunk area around it are read
//!   (`readSurroundingChunks`). Both save the touched chunks;
//! * the debug option `debugLoadAllChunks`, which reads the chunks around
//!   the player whenever the player's chunk changes.
//!
//! Chunk loading itself does not call `IsoRegions` (chunks only reach it
//! through those calls), so [`crate::save::chunk_file::finish_chunk_load`]
//! and the streamer do not either. Squares put into a chunk drop their
//! cached region (`IsoGridSquare.ResetIsoWorldRegion`, see
//! [`crate::chunk::IsoGridSquare::reset_iso_world_region`]).
//!
//! # The worker thread
//!
//! The original queues jobs from the main thread to `IsoRegionWorker`,
//! which batches them until an `ApplyChanges` job arrives, applies the
//! batch to its own `DataRoot` and asks the main thread to swap that root
//! with the one the game reads (`IsoRegions.update`), applies the batch a
//! second time to the root it got back, and then writes the save files.
//! Both roots see every batch once, in the same order, so their regions
//! stay equal; what differs is the state `WorldRegionToMetaGrid` keeps on
//! them (each region's `BuildingDef`, each root's `changedCells`), so both
//! roots are kept here too. [`IsoRegions::update_with_buildings`] runs one
//! worker loop iteration synchronously (taking every queued job, as the
//! worker does when the main thread queued them between two of its polls);
//! at the swap it does the main thread's side (the squares drop their
//! cached regions, then [`metagrid::WorldRegionToMetaGrid`] turns the
//! changed cells' player rooms into meta-grid buildings) before the
//! worker's second cycle, the order the original gets when the main thread
//! answers within the worker's 5 ms poll. [`IsoRegions::update`] is the
//! same without a meta grid.
//!
//! Single player only: the network jobs and packets
//! (`JobServerSendFullData`, `readServerUpdatePacket`, ...) are not ported.

pub mod data;
pub mod metagrid;
pub mod regions;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::chunk::{IsoCell, IsoChunk, IsoGridSquare};
use crate::sprite::IsoSpriteManager;

pub use data::{ChunkIdx, DataChunk, DataRoot, SelectInfo};
pub use regions::{
    ChunkRegionRef, IsoChunkRegion, IsoRegionManager, IsoWorldRegion, WorldRegionRef,
};

/// `IsoRegions.SINGLE_CHUNK_PACKET_SIZE`.
pub const SINGLE_CHUNK_PACKET_SIZE: usize = 2076;
/// `IsoRegions.CHUNKS_DATA_PACKET_SIZE` (a chunk-update job's buffer).
pub const CHUNKS_DATA_PACKET_SIZE: usize = 65536;
/// `IsoRegions.CHUNK_LOAD_DIMENSIONS`.
pub const CHUNK_LOAD_DIMENSIONS: i32 = 7;
/// The version written into region files (`IsoWorld.getWorldVersion()`
/// of the build this follows).
pub const REGION_WORLD_VERSION: i32 = 249;
/// `IsoRegions.FILE_DIR`.
pub const FILE_DIR: &str = "isoregiondata";
/// The header file in [`FILE_DIR`].
pub const HEADER_FILE: &str = "RegionHeader.bin";

/// `IsoRegions.hash(x, y)`.
pub fn hash(x: i32, y: i32) -> i32 {
    (y << 16) ^ x
}

/// `IsoRegions.GetOppositeDir(dir)`.
pub fn get_opposite_dir(dir: i8) -> i8 {
    match dir {
        0 => 2,
        1 => 3,
        2 => 0,
        3 => 1,
        4 => 5,
        5 => 4,
        _ => -1,
    }
}

/// `IsoRegions.getChunkFile` name: `datachunk_<x>_<y>.bin`.
pub fn chunk_file_name(chunk_x: i32, chunk_y: i32) -> String {
    format!("datachunk_{chunk_x}_{chunk_y}.bin")
}

/// `IsoRegions.calculateSquareFlags(gs)`.
pub fn calculate_square_flags(gs: Option<&IsoGridSquare>, m: &IsoSpriteManager) -> i8 {
    let mut flags = 0i8;
    let Some(gs) = gs else {
        return flags;
    };
    let f = &m.tables.flag;
    let ty = &m.tables.ty;
    let named = |n: &str| m.tables.flag_from_string(n).is_some_and(|fl| gs.has(fl));
    if gs.has(f.solidfloor) {
        flags |= data::BIT_HAS_FLOOR;
    }
    if gs.has(f.cutN) || gs.has_type(ty.doorFrN) {
        flags |= data::BIT_WALL_N;
        if gs.has(f.WindowN) || gs.has(f.windowN) || named("DoorWallN") {
            flags |= data::BIT_PATH_WALL_N;
        }
    }
    if !gs.has(f.WallSE) && (gs.has(f.cutW) || gs.has_type(ty.doorFrW)) {
        flags |= data::BIT_WALL_W;
        if gs.has(f.WindowW) || gs.has(f.windowW) || named("DoorWallW") {
            flags |= data::BIT_PATH_WALL_W;
        }
    }
    if gs.has_stairs_north(m) || gs.has_stairs_west(m) {
        flags |= data::BIT_STAIRCASE;
    }
    flags
}

fn put_i32(b: &mut Vec<u8>, v: i32) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn get_i32(b: &[u8], pos: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(pos..pos + 4)?.try_into().ok()?))
}

fn set_i32(b: &mut [u8], pos: usize, v: i32) {
    b[pos..pos + 4].copy_from_slice(&v.to_be_bytes());
}

/// `ChunkUpdate.writeIsoChunkIntoBuffer(c, bb)`.
pub fn write_iso_chunk_into_buffer(c: &IsoChunk, m: &IsoSpriteManager, bb: &mut Vec<u8>) {
    let start = bb.len();
    put_i32(bb, 0);
    put_i32(bb, c.max_level);
    put_i32(bb, (c.max_level + 1) * 64);
    for z in 0..=c.max_level {
        for p in 0..64 {
            let sq = c.get_grid_square(p % 8, p / 8, z);
            bb.push(calculate_square_flags(sq, m) as u8);
        }
    }
    let len = (bb.len() - start) as i32;
    set_i32(bb, start, len);
}

/// `JobChunkUpdate`: up to [`CHUNKS_DATA_PACKET_SIZE`] bytes of chunk
/// blocks, `[total length][chunk count]` then per chunk
/// `[block length][version][chunk x][chunk y][DataChunk data]`. A block is
/// exactly what a region chunk file holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobChunkUpdate {
    buffer: Vec<u8>,
    chunk_count: i32,
    buffer_max_bytes: i32,
}

impl JobChunkUpdate {
    pub fn new() -> Self {
        Self::default()
    }
    /// `getChunkCount()`.
    pub fn chunk_count(&self) -> i32 {
        self.chunk_count
    }
    /// The buffer up to its position.
    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }
    /// `canAddChunk()`.
    pub fn can_add_chunk(&self) -> bool {
        self.buffer.len() + SINGLE_CHUNK_PACKET_SIZE < CHUNKS_DATA_PACKET_SIZE
    }
    fn start_buffer_block(&mut self) -> usize {
        if self.chunk_count == 0 {
            self.buffer.clear();
            put_i32(&mut self.buffer, 0);
            put_i32(&mut self.buffer, 0);
        }
        let start = self.buffer.len();
        put_i32(&mut self.buffer, 0);
        start
    }
    fn end_buffer_block(&mut self, start: usize) {
        self.buffer_max_bytes = self.buffer.len() as i32;
        let len = self.buffer_max_bytes - start as i32;
        set_i32(&mut self.buffer, start, len);
        self.chunk_count += 1;
        let (max, count) = (self.buffer_max_bytes, self.chunk_count);
        set_i32(&mut self.buffer, 0, max);
        set_i32(&mut self.buffer, 4, count);
    }
    /// `addChunkFromDataChunk(chunk)`.
    pub fn add_chunk_from_data_chunk(&mut self, chunk: &DataChunk) -> bool {
        if !self.can_add_chunk() {
            return false;
        }
        let start = self.start_buffer_block();
        put_i32(&mut self.buffer, REGION_WORLD_VERSION);
        put_i32(&mut self.buffer, chunk.chunk_x());
        put_i32(&mut self.buffer, chunk.chunk_y());
        chunk.save(&mut self.buffer);
        self.end_buffer_block(start);
        true
    }
    /// `addChunkFromIsoChunk(isoChunk)`.
    pub fn add_chunk_from_iso_chunk(&mut self, c: &IsoChunk, m: &IsoSpriteManager) -> bool {
        if !self.can_add_chunk() {
            return false;
        }
        let start = self.start_buffer_block();
        put_i32(&mut self.buffer, REGION_WORLD_VERSION);
        put_i32(&mut self.buffer, c.wx);
        put_i32(&mut self.buffer, c.wy);
        write_iso_chunk_into_buffer(c, m, &mut self.buffer);
        self.end_buffer_block(start);
        true
    }
    /// `addChunkFromFile(bb)`: `bb` is the start of a region chunk file
    /// (at most [`SINGLE_CHUNK_PACKET_SIZE`] bytes). `None` where the
    /// original throws (a file too short for its header or length).
    pub fn add_chunk_from_file(&mut self, bb: &[u8]) -> Option<bool> {
        if self.buffer.len() + bb.len() >= CHUNKS_DATA_PACKET_SIZE {
            return Some(false);
        }
        let version = get_i32(bb, 4)?;
        let wx = get_i32(bb, 8)?;
        let wy = get_i32(bb, 12)?;
        let length = get_i32(bb, 16)?;
        let data = bb.get(16..16usize.checked_add(usize::try_from(length).ok()?)?)?;
        let start = self.start_buffer_block();
        put_i32(&mut self.buffer, version);
        put_i32(&mut self.buffer, wx);
        put_i32(&mut self.buffer, wy);
        self.buffer.extend_from_slice(data);
        self.end_buffer_block(start);
        Some(true)
    }

    /// `readChunksPacket(root, knownChunks)` (single player).
    pub fn read_chunks_packet(&self, root: &mut DataRoot, known_chunks: &mut Vec<i32>) {
        let b = &self.buffer;
        let Some(count) = get_i32(b, 4) else {
            return;
        };
        let mut pos = 8;
        for _ in 0..count {
            let (Some(_), Some(_), Some(cx), Some(cy)) = (
                get_i32(b, pos),
                get_i32(b, pos + 4),
                get_i32(b, pos + 8),
                get_i32(b, pos + 12),
            ) else {
                return;
            };
            pos += 16;
            let s = root.select(cx * 8, cy * 8, 0, true, false);
            let c = s.chunk.unwrap();
            let h = hash(cx, cy);
            if !known_chunks.contains(&h) {
                known_chunks.push(h);
            }
            root.load_chunk(c, b, &mut pos);
            root.set_dirty_all_active(c);
        }
    }

    /// The `(chunk x, chunk y, file bytes)` of each block, as
    /// `saveChunksToDisk` writes them.
    pub fn chunk_files(&self) -> Vec<(i32, i32, &[u8])> {
        let mut out = Vec::new();
        if self.chunk_count <= 0 {
            return out;
        }
        let b = &self.buffer;
        let count = get_i32(b, 4).unwrap_or(0);
        let mut pos = 8usize;
        for _ in 0..count {
            let (Some(len), Some(cx), Some(cy)) =
                (get_i32(b, pos), get_i32(b, pos + 8), get_i32(b, pos + 12))
            else {
                break;
            };
            let end = (pos + len.max(0) as usize).min(b.len());
            out.push((cx, cy, &b[pos..end]));
            pos += len.max(0) as usize;
        }
        out
    }
}

/// A queued `RegionJob` (single-player kinds).
#[derive(Clone, Debug, PartialEq)]
pub enum RegionJob {
    /// `JobSquareUpdate`.
    SquareUpdate { x: i32, y: i32, z: i32, flags: i8 },
    /// `JobChunkUpdate`.
    ChunkUpdate(JobChunkUpdate),
    /// `JobApplyChanges`.
    ApplyChanges { save_to_disk: bool },
    /// `JobDebugResetAllData`.
    DebugResetAllData,
}

/// `IsoRegions` with its `IsoRegionWorker`.
#[derive(Debug)]
pub struct IsoRegions {
    /// `IsoRegions.dataRoot` and the worker's `rootBuffer`: `roots[main]`
    /// is the one the game reads. Both get every batch, one after the other,
    /// and a buffer swap flips `main`.
    roots: [DataRoot; 2],
    main: usize,
    /// `DataRoot.wrtmg` (shared by both roots).
    wrtmg: metagrid::WorldRegionToMetaGrid,
    /// `cacheDir`: the save's `isoregiondata` directory; `None` keeps
    /// everything in memory.
    dir: Option<PathBuf>,
    /// `Core.getInstance().isNoSave()`.
    pub no_save: bool,
    /// `IsoChunkMap.chunkGridWidth`.
    pub chunk_grid_width: i32,
    /// `debugLoadAllChunks` (debug builds only upstream).
    pub debug_load_all_chunks: bool,
    last_chunk_x: i32,
    last_chunk_y: i32,
    previous_flags: i8,
    square_change_per_tick: i32,
    discovered_chunks: Vec<i32>,
    thread_discovered_chunks: Vec<i32>,
    last_thread_discovered_chunks_size: usize,
    job_queue: VecDeque<RegionJob>,
    job_batched_processing: Vec<RegionJob>,
}

impl IsoRegions {
    /// A region system with no save directory (nothing is read or
    /// written).
    pub fn new(chunk_grid_width: i32) -> Self {
        IsoRegions {
            roots: [DataRoot::new(), DataRoot::new()],
            main: 0,
            wrtmg: Default::default(),
            dir: None,
            no_save: false,
            chunk_grid_width,
            debug_load_all_chunks: false,
            last_chunk_x: -1,
            last_chunk_y: -1,
            previous_flags: 0,
            square_change_per_tick: 0,
            discovered_chunks: Vec::new(),
            thread_discovered_chunks: Vec::new(),
            last_thread_discovered_chunks_size: 0,
            job_queue: VecDeque::new(),
            job_batched_processing: Vec::new(),
        }
    }

    /// `IsoRegions.init()` for the save at `save_dir`: creates
    /// `isoregiondata` if missing and queues the region chunks saved there
    /// (`IsoRegionWorker.load` → `loadSaveMap`). They are processed by the
    /// next [`IsoRegions::update`].
    pub fn init(save_dir: Option<&Path>, chunk_grid_width: i32) -> Self {
        let mut r = Self::new(chunk_grid_width);
        if let Some(sd) = save_dir {
            let dir = sd.join(FILE_DIR);
            if !dir.exists() {
                let _ = std::fs::create_dir(&dir);
            }
            r.dir = Some(dir);
        }
        r.load_save_map();
        r
    }

    /// `IsoRegions.getDataRoot()`: the root the game reads.
    pub fn data_root(&self) -> &DataRoot {
        &self.roots[self.main]
    }

    /// `IsoRegions.getDataRoot()`, mutable.
    pub fn data_root_mut(&mut self) -> &mut DataRoot {
        &mut self.roots[self.main]
    }

    /// The worker's root (`IsoRegionWorker.getRootBuffer()`).
    pub fn root_buffer(&self) -> &DataRoot {
        &self.roots[1 - self.main]
    }

    /// `IsoRegions.getDirectory()`.
    pub fn directory(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The warnings `IsoRegions.warn` logged, oldest first.
    pub fn warnings(&self) -> &[String] {
        &self.roots[self.main].region_manager.warnings
    }

    /// `regionManager`.
    pub fn manager(&self) -> &IsoRegionManager {
        &self.roots[self.main].region_manager
    }

    /// The jobs waiting for the next [`IsoRegions::update`].
    pub fn queued_jobs(&self) -> impl Iterator<Item = &RegionJob> {
        self.job_queue
            .iter()
            .chain(self.job_batched_processing.iter())
    }

    /// `discoveredChunks` (main-thread list, chunk hashes).
    pub fn discovered_chunks(&self) -> &[i32] {
        &self.discovered_chunks
    }

    /// `threadDiscoveredChunks` (the worker's list, written to the header).
    pub fn thread_discovered_chunks(&self) -> &[i32] {
        &self.thread_discovered_chunks
    }

    /// `IsoRegions.getSquareFlags(x, y, z)`.
    pub fn get_square_flags(&self, x: i32, y: i32, z: i32) -> i8 {
        self.roots[self.main].get_square_flags(x, y, z)
    }

    /// `IsoRegions.getIsoWorldRegion(x, y, z)`.
    pub fn get_iso_world_region(&self, x: i32, y: i32, z: i32) -> Option<WorldRegionRef> {
        self.roots[self.main].get_iso_world_region(x, y, z)
    }

    /// `IsoRegions.getChunkRegion(x, y, z)`.
    pub fn get_chunk_region(&self, x: i32, y: i32, z: i32) -> Option<ChunkRegionRef> {
        self.roots[self.main].get_iso_chunk_region(x, y, z)
    }

    /// `IsoRegions.getDataChunk(chunkx, chunky)`.
    pub fn get_data_chunk(&self, chunkx: i32, chunky: i32) -> Option<&DataChunk> {
        let root = &self.roots[self.main];
        root.get_data_chunk(chunkx, chunky).map(|c| root.chunk(c))
    }

    /// `IsoRegions.getIsoWorldRegionsInCell(cellX, cellY, list)`.
    pub fn get_iso_world_regions_in_cell(&self, cell_x: i32, cell_y: i32) -> Vec<WorldRegionRef> {
        self.roots[self.main].get_iso_world_regions_in_cell(cell_x, cell_y)
    }

    /// The world region object `r`.
    pub fn world_region(&self, r: WorldRegionRef) -> &IsoWorldRegion {
        self.roots[self.main].region_manager.world_region(r)
    }

    /// The chunk region object `r`.
    pub fn chunk_region(&self, r: ChunkRegionRef) -> &IsoChunkRegion {
        self.roots[self.main].region_manager.chunk_region(r)
    }

    /// `IWorldRegion.isFogMask()`.
    pub fn is_fog_mask(&self, r: WorldRegionRef) -> bool {
        self.roots[self.main]
            .region_manager
            .world_region_is_fog_mask(r)
    }

    /// `IWorldRegion.isPlayerRoom()`.
    pub fn is_player_room(&self, r: WorldRegionRef) -> bool {
        self.roots[self.main]
            .region_manager
            .world_region_is_player_room(r)
    }

    /// `IsoWorldRegion.isEnclosed()`.
    pub fn is_enclosed(&self, r: WorldRegionRef) -> bool {
        self.roots[self.main]
            .region_manager
            .world_region_is_enclosed(r)
    }

    /// `IsoChunkRegion.getIsEnclosed()`.
    pub fn chunk_region_is_enclosed(&self, r: ChunkRegionRef) -> bool {
        self.roots[self.main]
            .region_manager
            .chunk_region_is_enclosed(r)
    }

    /// `IWorldRegion.getNeighbors()`.
    pub fn neighbors(&self, r: WorldRegionRef) -> &[WorldRegionRef] {
        self.world_region(r).neighbors()
    }

    /// `IsoRegions.setPreviousFlags(gs)`: call before changing a square's
    /// objects.
    pub fn set_previous_flags(&mut self, gs: Option<&IsoGridSquare>, m: &IsoSpriteManager) {
        self.previous_flags = calculate_square_flags(gs, m);
    }

    /// `IsoRegions.squareChanged(gs, isRemoval)`: call after changing the
    /// square at `(x, y, z)` in `cell`.
    pub fn square_changed(
        &mut self,
        cell: &IsoCell,
        m: &IsoSpriteManager,
        x: i32,
        y: i32,
        z: i32,
        is_removal: bool,
    ) {
        let Some(gs) = cell.get_grid_square(x, y, z) else {
            return;
        };
        let flags = calculate_square_flags(Some(gs), m);
        if flags == self.previous_flags {
            return;
        }
        self.add_square_changed_job(cell, m, gs.x, gs.y, gs.z, is_removal, flags);
        self.square_change_per_tick += 1;
        self.previous_flags = 0;
    }

    /// `IsoRegionWorker.addSquareChangedJob(x, y, z, isRemoval, flags)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_square_changed_job(
        &mut self,
        cell: &IsoCell,
        m: &IsoSpriteManager,
        x: i32,
        y: i32,
        z: i32,
        is_removal: bool,
        flags: i8,
    ) {
        if z < 0 {
            return;
        }
        let (cx, cy) = (x / 8, y / 8);
        if self.discovered_chunks.contains(&hash(cx, cy)) {
            self.job_queue
                .push_back(RegionJob::SquareUpdate { x, y, z, flags });
            self.read_surrounding_chunks(cell, m, cx, cy, CHUNK_LOAD_DIMENSIONS, false, false);
            self.apply_chunk_changes(true);
        } else {
            if is_removal {
                return;
            }
            self.read_surrounding_chunks(cell, m, cx, cy, CHUNK_LOAD_DIMENSIONS, true, false);
        }
    }

    /// `IsoRegionWorker.EnqueueJob(job)`.
    pub fn enqueue_job(&mut self, job: RegionJob) {
        self.job_queue.push_back(job);
    }

    /// `IsoRegionWorker.ApplyChunkChanges(saveToDisk)`.
    pub fn apply_chunk_changes(&mut self, save_to_disk: bool) {
        self.job_queue
            .push_back(RegionJob::ApplyChanges { save_to_disk });
    }

    /// `IsoRegions.ResetAllDataDebug()`.
    pub fn reset_all_data_debug(&mut self) {
        self.job_queue.push_back(RegionJob::DebugResetAllData);
    }

    /// `IsoRegionWorker.readSurroundingChunks(chunkX, chunkY, dimension,
    /// applyChanges, forceRecalc)` over the chunks loaded in `cell`.
    #[allow(clippy::too_many_arguments)]
    pub fn read_surrounding_chunks(
        &mut self,
        cell: &IsoCell,
        m: &IsoSpriteManager,
        chunk_x: i32,
        chunk_y: i32,
        dimension: i32,
        apply_changes: bool,
        force_recalc: bool,
    ) {
        let mut sides = 1;
        if dimension > 0 && dimension <= self.chunk_grid_width {
            sides = dimension / 2;
            if sides + sides >= self.chunk_grid_width {
                sides -= 1;
            }
        }
        let mut j = JobChunkUpdate::new();
        let mut has_added = false;
        for xx in chunk_x - sides..=chunk_x + sides {
            for yy in chunk_y - sides..=chunk_y + sides {
                let Some(c) = cell.chunks.get(&(xx, yy)) else {
                    continue;
                };
                let h = hash(c.wx, c.wy);
                if !force_recalc && self.discovered_chunks.contains(&h) {
                    continue;
                }
                self.discovered_chunks.push(h);
                if !j.can_add_chunk() {
                    self.job_queue
                        .push_back(RegionJob::ChunkUpdate(std::mem::take(&mut j)));
                }
                j.add_chunk_from_iso_chunk(c, m);
                has_added = true;
            }
        }
        if j.chunk_count() > 0 {
            self.job_queue.push_back(RegionJob::ChunkUpdate(j));
        }
        if has_added && apply_changes {
            self.apply_chunk_changes(true);
        }
    }

    /// `IsoRegions.update()` for a game without a meta grid to build into:
    /// one `IsoRegionWorker` loop iteration over the queued jobs, where the
    /// main thread's side of a buffer swap drops the squares' cached regions
    /// (`clientResetCachedRegionReferences` over `cell`'s squares) and the
    /// changed cells (no player-built buildings are made; see
    /// [`IsoRegions::update_with_buildings`]); then the `debugLoadAllChunks`
    /// read around `player` (square position). Returns whether a swap
    /// happened.
    pub fn update(
        &mut self,
        cell: &IsoCell,
        m: &IsoSpriteManager,
        player: Option<(f32, f32)>,
    ) -> bool {
        let swapped = self.worker_tick_with(&mut |r: &mut IsoRegions| {
            client_reset_cached_region_references(cell);
            r.roots[r.main].changed_cells.clear();
        });
        self.square_change_per_tick = 0;
        self.debug_load_around(cell, m, player);
        swapped
    }

    /// `IsoRegions.update()`: one `IsoRegionWorker` loop iteration over the
    /// queued jobs. At the buffer swap the main thread's side runs before
    /// the worker's second cycle (as when the main thread answers the swap
    /// within the worker's 5 ms poll): the squares drop their cached
    /// regions and `DataRoot.clientProcessBuildings` turns the changed
    /// cells' player rooms into meta-grid buildings
    /// ([`metagrid::WorldRegionToMetaGrid`]). Then the
    /// `debugLoadAllChunks` read around `player`. Returns whether a swap
    /// happened.
    pub fn update_with_buildings(
        &mut self,
        ctx: &mut metagrid::BuildingCtx<'_>,
        player: Option<(f32, f32)>,
    ) -> bool {
        let swapped = self.worker_tick_with(&mut |r: &mut IsoRegions| {
            client_reset_cached_region_references(ctx.cell);
            r.client_process_buildings(ctx);
        });
        self.square_change_per_tick = 0;
        self.debug_load_around(ctx.cell, ctx.sprites, player);
        swapped
    }

    /// `DataRoot.clientProcessBuildings()` on the root the game reads: the
    /// changed cells go to `WorldRegionToMetaGrid` and are cleared.
    pub fn client_process_buildings(&mut self, ctx: &mut metagrid::BuildingCtx<'_>) {
        let changed = std::mem::take(&mut self.roots[self.main].changed_cells);
        let mut w = std::mem::take(&mut self.wrtmg);
        w.client_process_buildings(&mut self.roots, self.main, &changed, ctx);
        self.wrtmg = w;
    }

    fn debug_load_around(
        &mut self,
        cell: &IsoCell,
        m: &IsoSpriteManager,
        player: Option<(f32, f32)>,
    ) {
        if self.debug_load_all_chunks {
            if let Some((px, py)) = player {
                let cx = px.floor() as i32 / 8;
                let cy = py.floor() as i32 / 8;
                if self.last_chunk_x != cx || self.last_chunk_y != cy {
                    self.last_chunk_x = cx;
                    self.last_chunk_y = cy;
                    let dim = self.chunk_grid_width - 2;
                    self.read_surrounding_chunks(cell, m, cx, cy, dim, true, false);
                }
            }
        }
    }

    /// `IsoRegionWorker.thread_main_loop()` once, synchronously, for callers
    /// without a main thread side: at a swap only the changed cells are
    /// dropped. Returns whether a swap happened.
    pub fn worker_tick(&mut self) -> bool {
        self.worker_tick_with(&mut |r: &mut IsoRegions| r.roots[r.main].changed_cells.clear())
    }

    /// `IsoRegionWorker.thread_main_loop()` once, synchronously; `on_swap`
    /// is the main thread's side of each buffer swap, called with the new
    /// root in place. Returns whether a swap happened.
    pub fn worker_tick_with(&mut self, on_swap: &mut dyn FnMut(&mut IsoRegions)) -> bool {
        let mut swapped = false;
        let mut apply_idx = self
            .job_batched_processing
            .iter()
            .position(|j| matches!(j, RegionJob::ApplyChanges { .. }));
        while let Some(job) = self.job_queue.pop_front() {
            match job {
                RegionJob::DebugResetAllData => {
                    let w = 1 - self.main;
                    self.roots[w].reset_all_data();
                    self.main = w;
                    swapped = true;
                    on_swap(self);
                    let w = 1 - self.main;
                    self.roots[w].reset_all_data();
                }
                RegionJob::ApplyChanges { mut save_to_disk } => {
                    if let Some(i) = apply_idx {
                        if let RegionJob::ApplyChanges { save_to_disk: s1 } =
                            self.job_batched_processing.remove(i)
                        {
                            save_to_disk = save_to_disk || s1;
                        }
                    }
                    apply_idx = Some(self.job_batched_processing.len());
                    self.job_batched_processing
                        .push(RegionJob::ApplyChanges { save_to_disk });
                }
                other => self.job_batched_processing.push(other),
            }
        }
        if apply_idx.is_some() {
            swapped |= self.run_batched_jobs(on_swap);
            self.job_batched_processing.clear();
        }
        swapped
    }

    /// `thread_run_batched_jobs()`: two cycles over the batch, each on the
    /// worker's root. The first cycle's `ApplyChanges` processes the dirty
    /// chunks and swaps the roots (`on_swap`); the second's processes them
    /// on the other root and writes the files.
    fn run_batched_jobs(&mut self, on_swap: &mut dyn FnMut(&mut IsoRegions)) -> bool {
        let mut swapped = false;
        for cycle in 0..2 {
            let mut index = 0;
            while index < self.job_batched_processing.len() {
                let w = 1 - self.main;
                match &self.job_batched_processing[index] {
                    RegionJob::SquareUpdate { x, y, z, flags } => {
                        let (x, y, z, flags) = (*x, *y, *z, *flags);
                        self.roots[w].update_existing_square(x, y, z, flags);
                    }
                    RegionJob::ChunkUpdate(j) => {
                        j.read_chunks_packet(
                            &mut self.roots[w],
                            &mut self.thread_discovered_chunks,
                        );
                    }
                    RegionJob::ApplyChanges { save_to_disk } => {
                        let save = *save_to_disk;
                        self.roots[w].process_dirty_chunks();
                        if cycle == 0 {
                            self.main = w;
                            swapped = true;
                            on_swap(self);
                        } else if save {
                            self.save_batch();
                        }
                    }
                    RegionJob::DebugResetAllData => {}
                }
                index += 1;
            }
        }
        swapped
    }

    /// The `isSaveToDisk` part of the second cycle: each square-update or
    /// chunk-update job's chunks are written, last job first, then the
    /// header when new chunks were seen.
    fn save_batch(&mut self) {
        let mut i = self.job_batched_processing.len();
        while i > 0 {
            i -= 1;
            let job = match &self.job_batched_processing[i] {
                RegionJob::SquareUpdate { x, y, z, .. } => {
                    let (x, y, z) = (*x, *y, *z);
                    let root = &mut self.roots[1 - self.main];
                    let s = root.select(x, y, z, true, false);
                    let mut j = JobChunkUpdate::new();
                    j.add_chunk_from_data_chunk(root.chunk(s.chunk.unwrap()));
                    j
                }
                RegionJob::ChunkUpdate(_) => match self.job_batched_processing.remove(i) {
                    RegionJob::ChunkUpdate(j) => j,
                    _ => unreachable!(),
                },
                _ => continue,
            };
            self.save_chunks_to_disk(&job);
        }
        if !self.thread_discovered_chunks.is_empty()
            && self.thread_discovered_chunks.len() > self.last_thread_discovered_chunks_size
            && !self.no_save
        {
            if let Some(dir) = &self.dir {
                let mut b = Vec::with_capacity(8 + 4 * self.thread_discovered_chunks.len());
                put_i32(&mut b, REGION_WORLD_VERSION);
                put_i32(&mut b, self.thread_discovered_chunks.len() as i32);
                for &h in &self.thread_discovered_chunks {
                    put_i32(&mut b, h);
                }
                if std::fs::write(dir.join(HEADER_FILE), &b).is_ok() {
                    self.last_thread_discovered_chunks_size = self.thread_discovered_chunks.len();
                }
            }
        }
    }

    /// `JobChunkUpdate.saveChunksToDisk()`.
    fn save_chunks_to_disk(&self, j: &JobChunkUpdate) {
        if self.no_save {
            return;
        }
        let Some(dir) = &self.dir else {
            return;
        };
        for (cx, cy, bytes) in j.chunk_files() {
            let _ = std::fs::write(dir.join(chunk_file_name(cx, cy)), bytes);
        }
    }

    /// `IsoRegionWorker.loadSaveMap()`.
    fn load_save_map(&mut self) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        let mut header_known: Vec<i32> = Vec::new();
        let mut has_read_header = false;
        if let Ok(h) = std::fs::read(dir.join(HEADER_FILE)) {
            has_read_header = true;
            let size = get_i32(&h, 4).unwrap_or(0);
            for i in 0..size.max(0) as usize {
                match get_i32(&h, 8 + 4 * i) {
                    Some(id) => header_known.push(id),
                    // `EOFException`: the whole load is abandoned.
                    None => return,
                }
            }
        }
        let mut j = JobChunkUpdate::new();
        let mut loaded = false;
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !(name.starts_with("datachunk_") && name.ends_with(".bin")) {
                    continue;
                }
                let Ok(all) = std::fs::read(e.path()) else {
                    continue;
                };
                let bb = &all[..all.len().min(SINGLE_CHUNK_PACKET_SIZE)];
                let (Some(x), Some(y)) = (get_i32(bb, 8), get_i32(bb, 12)) else {
                    // `BufferUnderflowException` ends the load.
                    break;
                };
                let h = hash(x, y);
                if !self.discovered_chunks.contains(&h) {
                    self.discovered_chunks.push(h);
                }
                if let Some(p) = header_known.iter().position(|&k| k == h) {
                    header_known.remove(p);
                } else {
                    self.roots[self.main].region_manager.warnings.push(
                        "IsoRegion: A chunk save has been found that was not in header known chunks list."
                            .into(),
                    );
                }
                if !j.can_add_chunk() {
                    self.job_queue
                        .push_back(RegionJob::ChunkUpdate(std::mem::take(&mut j)));
                }
                if j.add_chunk_from_file(bb).is_none() {
                    break;
                }
                loaded = true;
            }
        }
        if j.chunk_count() > 0 {
            self.job_queue.push_back(RegionJob::ChunkUpdate(j));
        }
        if loaded {
            self.apply_chunk_changes(false);
        }
        if has_read_header && !header_known.is_empty() {
            self.roots[self.main].region_manager.warnings.push(format!(
                "IsoRegion: {} previously discovered chunks have not been loaded.",
                header_known.len()
            ));
        }
    }
}

/// `IsoRegions.clientResetCachedRegionReferences()`: every loaded square
/// forgets its cached world region.
pub fn client_reset_cached_region_references(cell: &IsoCell) {
    for c in cell.chunks.values() {
        for sq in c.squares() {
            sq.set_iso_world_region(None);
        }
    }
}
