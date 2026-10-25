//! The chunk streamer: `IsoChunkMap` (the grid of chunks around each local
//! player), `WorldStreamer` (the load-request queue), `ChunkSaveWorker`
//! (saves of unloaded chunks) and the `IsoCell`/`IsoWorld` glue that drives
//! them, so a world can be walked.
//!
//! # Threads in the original and how they are serialized here
//!
//! The original runs this on three threads:
//!
//! * the **main thread** moves each player's chunk map
//!   (`IsoChunkMap.ProcessChunkPos`, once per frame per player, before
//!   `IsoWorld.update`): it shifts the grid by one chunk, unloads the chunks
//!   that fall off (`IsoChunk.removeFromWorld`, `ChunkSaveWorker.Add`) and
//!   queues the new edge (`LoadChunkForLater` → `WorldStreamer.addJob`, a
//!   concurrent `jobQueue`). Later in the frame `IsoCell.update` calls
//!   `IsoChunkMap.update` per player, which takes finished chunks from the
//!   concurrent `IsoChunk.loadGridSquare` queue (at most
//!   `1 + n * 3 / chunkGridWidth` of the `n` waiting), puts them in every
//!   chunk map that covers them (`setChunkDirect`) and runs
//!   `doLoadGridsquare`;
//! * the **"World Streamer" thread** loops `threadLoop`: it moves `jobQueue`
//!   into `jobList` (dropping duplicates), returns jobs nobody references
//!   any more to `IsoChunkMap.chunkStore`, sorts `jobList` by distance to
//!   each player's position ten tiles ahead of their movement
//!   (`ChunkComparator`), takes the closest, lets `ChunkSaveWorker.Update`
//!   write one queued save (that chunk's own first), loads the chunk
//!   (`IsoChunk.LoadChunk`: the save file if there is one, else
//!   `LoadBrandNew`) and `loadInWorldStreamerThread`, then appends it to
//!   `loadGridSquare`. One chunk per loop; it only sleeps when idle;
//! * the **"World Reuser" thread** recycles chunks nobody wanted
//!   (`WorldReuserThread.addReuseChunk` → `chunkStore`).
//!
//! What runs when therefore depends on timing: how many chunks the streamer
//! serves between two frames, and whether it picks up a frame's new jobs in
//! one batch or part of it. [`ChunkStreamer::update`] makes this
//! deterministic: one frame is
//!
//! 1. `ProcessChunkPos` for each player, in player order;
//! 2. streamer loop iterations ([`Serve`]): a fixed number, or until it is
//!    idle (no jobs and no queued saves, like the `isBusy` wait the original
//!    does while loading the world);
//! 3. `IsoChunkMap.update` (the hand-off) for each player.
//!
//! Inside those steps everything is in the original's order: requests in
//! `LoadChunkForLater` order, unloads in grid-walk order, the comparator's
//! stable sort (`Collections.sort` is a stable merge sort; ties keep
//! `jobList` order and the last one is served first) and the
//! `loadGridSquare` FIFO with its per-frame limit.
//!
//! Chunk contents: `loadInWorldStreamerThread` only reads and writes the
//! chunk itself, so it runs at hand-off here, as part of
//! [`finish_chunk_load`] (with `loadInMainThread` and the erosion hook
//! calls of `doLoadGridsquare`), for chunks from save files and brand-new
//! chunks alike. The original skips it only for chunks nobody references
//! any more, and those are never handed off.
//!
//! `chunkStore` is a pool of `IsoChunk` objects the original reuses to save
//! allocations. Rust chunks are plain values, so the places that return a
//! chunk to the store (cancelled jobs, discarded chunks, the three extra
//! chunks of `LoadBrandNew`, saved chunks) drop it, and report it as a
//! [`StreamEvent`] where it matters.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use pz_sqlite::Connection;

use crate::chunk::{load_brand_new, BrandNewHook, ChunkLoadEnv, IsoCell, IsoChunk};
use crate::fix2x::Fix2xTable;
use crate::java::{fastfloor, GameRand};
use crate::meta_grid::IsoMetaGrid;
use crate::objects::{ObjectLoadHooks, ObjectLoadSettings, ObjectTables};
use crate::save::chunk_file::{
    chunk_path, finish_chunk_load, load_chunk, save_chunk, ChunkLoadHook,
};
use crate::save::items::ItemRegistry;
use crate::save::vehicles_db;
use crate::save::{SaveCtx, SaveHooks, SaveSettings, SaveTables};
use crate::sprite::IsoSpriteManager;

/// `IsoChunkMap.chunkGridWidth` before `CalcChunkWidth` runs.
pub const START_CHUNK_GRID_WIDTH: i32 = 13;

/// `IsoChunkMap.CalcChunkWidth()` for a screen of `w` x `h` pixels (the
/// debug overrides to fixed widths are left out): 13 chunks at 1920x1080
/// scaled down for smaller screens, times 1.5, made odd, at most 19.
pub fn calc_chunk_width(w: i32, h: i32) -> i32 {
    let delx = w as f32 / 1920.0;
    let dely = h as f32 / 1080.0;
    let del = delx.max(dely).min(1.0);
    let mut width = ((13.0f32 * del) as f64 * 1.5) as i32;
    if width / 2 * 2 == width {
        width += 1;
    }
    width.min(19)
}

/// `IsoChunkMap.worldXa`/`worldYa`: the chunk map position of a player
/// that has not been placed yet.
const WORLD_XA: i32 = 11702;
const WORLD_YA: i32 = 6896;

/// The key of `IsoChunkMap.SharedChunks`: `(wx << 16) + wy`, with Java's
/// wrapping (so it only separates chunks with `0 <= wy < 65536`).
fn shared_key(wx: i32, wy: i32) -> i32 {
    (wx << 16).wrapping_add(wy)
}

/// A player as the chunk streamer sees it (`IsoPlayer.players[n]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerPos {
    pub x: f32,
    pub y: f32,
    /// `getLastX()`/`getLastY()`: last frame's position. The load order
    /// favours chunks ahead of the player's movement.
    pub last_x: f32,
    pub last_y: f32,
}

impl PlayerPos {
    /// A player standing at `(x, y)`.
    pub fn at(x: f32, y: f32) -> Self {
        PlayerPos {
            x,
            y,
            last_x: x,
            last_y: y,
        }
    }

    /// A player that moved from `(last_x, last_y)` to `(x, y)`.
    pub fn moved(last_x: f32, last_y: f32, x: f32, y: f32) -> Self {
        PlayerPos {
            x,
            y,
            last_x,
            last_y,
        }
    }
}

/// Where a served chunk came from.
#[derive(Clone, Debug, PartialEq)]
pub enum ChunkSource {
    /// `LoadFromDisk`: the save file `map/<wx>/<wy>.bin`.
    Disk,
    /// `LoadBrandNew`: no save file.
    BrandNew,
    /// The save file failed to load (`BackupBlam`, then `LoadBrandNew`).
    BrandNewAfterError(String),
}

/// What the streamer did, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    /// `WorldStreamer.addJob` for a new chunk object (`LoadChunk` or
    /// `LoadChunkForLater` found no shared chunk).
    Requested { player: usize, wx: i32, wy: i32 },
    /// The streamer loaded a chunk (`DoChunk`).
    Served {
        wx: i32,
        wy: i32,
        source: ChunkSource,
    },
    /// `VehiclesDB2.loadChunk` for a served chunk, right after
    /// [`StreamEvent::Served`] (only when [`StreamCtx::vehicles_db`] is
    /// set): the vehicles added to [`IsoChunk::vehicles`], the rows that
    /// failed to load and were deleted, and the error that ended the
    /// statement early.
    VehiclesLoaded {
        wx: i32,
        wy: i32,
        added: usize,
        removed: Vec<(i32, String)>,
        error: Option<String>,
    },
    /// A queued job nobody references any more went back to `chunkStore`.
    Cancelled { wx: i32, wy: i32 },
    /// A finished chunk went into the chunk maps and the cell
    /// (`setChunkDirect` + `doLoadGridsquare`).
    HandedOff { wx: i32, wy: i32 },
    /// A finished chunk no chunk map wanted went to the reuse thread.
    Discarded { wx: i32, wy: i32 },
    /// `removeFromWorld` + `ChunkSaveWorker.Add`.
    Unloaded { wx: i32, wy: i32 },
    /// `VehiclesDB2.unloadChunk` while a chunk was saved, before its file
    /// is written (only when [`StreamCtx::vehicles_db`] is set): the ids of
    /// the rows written and the vehicles that were not (list index and
    /// error).
    VehiclesUnloaded {
        wx: i32,
        wy: i32,
        saved: Vec<i32>,
        errors: Vec<(usize, String)>,
    },
    /// `ChunkSaveWorker.WriteQueuedSave` (or `IsoChunkMap.Save`) wrote the
    /// chunk file (`written` is false when saving is off, `Core.isNoSave`).
    Saved { wx: i32, wy: i32, written: bool },
    /// The save queue became empty after a write
    /// (`ChunkSaveWorker.HotsaveAncilliarySystems`): the main thread now
    /// saves the meta grid, animals, game time and world map
    /// (`saveToBufferMap`) and `PlayerDB.savePlayers()` runs.
    HotSave,
    /// Quitting (`IngameState.exit`): the chunk left the world without
    /// being saved again (`removeFromWorld` only).
    Removed { wx: i32, wy: i32 },
    /// `IsoChunk.removeFromWorld` (an unload or quitting) took vehicles that
    /// keep updating in the meta (`shouldUpdateInMeta`) out of the chunk's
    /// list, queued their rows (`VehiclesDB2.updateVehicle`) and handed
    /// them to [`StreamCtx::vehicle_meta`]: the row ids (`-1` when the row
    /// could not be built), and the vehicles that failed (list index and
    /// exception). Only reported when something happened.
    VehiclesToMeta {
        wx: i32,
        wy: i32,
        ids: Vec<i32>,
        errors: Vec<(usize, String)>,
    },
    /// The chunk's meta cell had not generated its animal paths yet
    /// (`IsoMetaCell.checkAnimalZonesGenerated` set `animalZonesGenerated`):
    /// the engine runs `ZoneGenerator.genAnimalsPath(wx, wy)` (world
    /// generation, not ported), which queues the cell for saving
    /// (`IsoMetaGrid::add_cell_to_save`) unless the sandbox's animal path
    /// chance is "none".
    GenAnimalsPath { wx: i32, wy: i32 },
    /// `VehiclesDB2.updateWorldStreamer()` wrote the queued vehicle rows
    /// (only reported when the queue was not empty).
    VehiclesUpdated {
        saved: Vec<i32>,
        errors: Vec<(usize, String)>,
    },
    /// Saving failed; the chunk is dropped like the original logs and drops
    /// it.
    SaveFailed { wx: i32, wy: i32, error: String },
}

/// How many streamer loop iterations one [`ChunkStreamer::update`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Serve {
    /// At most this many `threadLoop` iterations (each serves at most one
    /// chunk and writes at most one queued save).
    Jobs(usize),
    /// Until there is nothing to load and nothing to save.
    UntilIdle,
}

/// Runtime inputs: the save directory, the systems chunk loading reads and
/// the hooks.
pub struct StreamCtx<'a> {
    /// The save directory (`map/<wx>/<wy>.bin` are read and written under
    /// it; failed loads are copied to `blam/`).
    pub save_dir: &'a Path,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a mut IsoSpriteManager,
    pub fix2x: &'a Fix2xTable,
    pub rand: &'a mut dyn GameRand,
    pub object_tables: &'a ObjectTables,
    pub object_settings: ObjectLoadSettings,
    pub object_hooks: &'a mut dyn ObjectLoadHooks,
    pub save_tables: &'a SaveTables,
    pub save_settings: &'a SaveSettings,
    pub items: &'a dyn ItemRegistry,
    pub save_hooks: &'a dyn SaveHooks,
    /// World generation and basements for brand-new chunks.
    pub brand_new: Option<&'a mut dyn BrandNewHook>,
    /// Erosion calls once a chunk is in the cell, and unload notices.
    pub chunk_hook: Option<&'a mut dyn ChunkLoadHook>,
    /// `Core.getInstance().isNoSave()`: unloaded chunks are not written.
    pub no_save: bool,
    /// The vehicle store's connection to `vehicles.db`
    /// (`VehiclesDB2.WorldStreamerThread.store`, opened by
    /// `SQLStore.create`). Every served chunk's vehicles are loaded from it
    /// ([`vehicles_db::load_chunk`]); `None` (no store, `Core.isNoSave`)
    /// loads none.
    pub vehicles_db: Option<&'a mut Connection>,
    /// How a saved chunk's vehicles become rows (`VehicleBuffer.set`).
    /// `None`: [`vehicles_db::LoadedVehicleSaver`] with
    /// [`vehicles_db::NoVehicleScripts`] (vehicles never added to the
    /// world, which the original cannot write either).
    pub vehicle_saver: Option<&'a dyn vehicles_db::ChunkVehicleSaver>,
    /// `VirtualVehicleManager.addToMeta` for the vehicles
    /// `IsoChunk.removeFromWorld` takes out of the world while they keep
    /// updating (the vehicle runtime's virtual vehicles). `None`: they are
    /// dropped after their rows are queued.
    pub vehicle_meta: Option<&'a mut dyn vehicles_db::VehicleMetaHook>,
    /// `VehiclesDB2`'s main-thread id sets: `removeFromWorld` unloads the
    /// vehicles' ids and allocates one for a vehicle sent to the meta
    /// without one. `None`: not tracked.
    pub vehicle_ids: Option<&'a mut vehicles_db::VehicleIds>,
}

/// Reborrow an optional hook for a shorter time.
fn chunk_hook<'r, 'a: 'r>(
    h: &'r mut Option<&'a mut (dyn ChunkLoadHook + 'a)>,
) -> Option<&'r mut (dyn ChunkLoadHook + 'r)> {
    match h {
        Some(h) => Some(&mut **h),
        None => None,
    }
}

fn brand_new_hook<'r, 'a: 'r>(
    h: &'r mut Option<&'a mut (dyn BrandNewHook + 'a)>,
) -> Option<&'r mut (dyn BrandNewHook + 'r)> {
    match h {
        Some(h) => Some(&mut **h),
        None => None,
    }
}

impl<'a> StreamCtx<'a> {
    fn save_ctx(&self) -> SaveCtx<'_> {
        SaveCtx {
            sprites: self.sprites,
            tables: self.save_tables,
            settings: self.save_settings,
            items: self.items,
            hooks: self.save_hooks,
            meta: Some(self.meta),
        }
    }
}

/// Identity of an `IsoChunk` object between its request and its unload.
type ChunkId = u64;

/// The streaming state of one `IsoChunk` object.
struct Record {
    wx: i32,
    wy: i32,
    /// `refs`: the chunk maps (players) that want it, in `add` order.
    refs: Vec<usize>,
    /// `loaded`: set when a chunk map took it (`setChunkDirect`).
    loaded: bool,
}

/// `IsoChunkMap`: one player's grid of `chunkGridWidth`² chunks centred
/// on `(world_x, world_y)`, double-buffered like the original (shifts write
/// the other buffer, then swap).
pub struct IsoChunkMap {
    pub player_id: usize,
    /// `ignore`: the map takes no chunks.
    pub ignore: bool,
    pub world_x: i32,
    pub world_y: i32,
    swap_a: Vec<Option<ChunkId>>,
    swap_b: Vec<Option<ChunkId>>,
    read_buffer_a: bool,
    /// `maxHeight`/`minHeight` (`calculateZExtentsForChunkMap`).
    pub max_height: i32,
    pub min_height: i32,
}

impl IsoChunkMap {
    fn new(player_id: usize, width: i32) -> Self {
        let n = (width * width) as usize;
        IsoChunkMap {
            player_id,
            ignore: false,
            world_x: WORLD_XA / 8,
            world_y: WORLD_YA / 8,
            swap_a: vec![None; n],
            swap_b: vec![None; n],
            read_buffer_a: true,
            max_height: 0,
            min_height: 0,
        }
    }

    fn read(&self) -> &Vec<Option<ChunkId>> {
        if self.read_buffer_a {
            &self.swap_a
        } else {
            &self.swap_b
        }
    }

    fn read_mut(&mut self) -> &mut Vec<Option<ChunkId>> {
        if self.read_buffer_a {
            &mut self.swap_a
        } else {
            &mut self.swap_b
        }
    }

    fn write_mut(&mut self) -> &mut Vec<Option<ChunkId>> {
        if self.read_buffer_a {
            &mut self.swap_b
        } else {
            &mut self.swap_a
        }
    }
}

/// A chunk waiting in `ChunkSaveWorker.toSaveQueue`.
struct QueuedSave {
    wx: i32,
    wy: i32,
    chunk: IsoChunk,
}

/// The chunk maps of all local players, the `WorldStreamer` queues and the
/// `ChunkSaveWorker` queue.
///
/// Per frame the engine calls [`ChunkStreamer::update`] with each player's
/// position, then reads the loaded chunks from the [`IsoCell`] and the
/// [`StreamEvent`]s from [`ChunkStreamer::take_events`].
pub struct ChunkStreamer {
    /// `IsoChunkMap.chunkGridWidth` (odd).
    pub width: i32,
    maps: Vec<IsoChunkMap>,
    /// `IsoChunkMap.SharedChunks`.
    shared: HashMap<i32, ChunkId>,
    records: HashMap<ChunkId, Record>,
    next_id: ChunkId,
    /// `WorldStreamer.jobQueue` (filled by the main thread).
    job_queue: VecDeque<ChunkId>,
    /// `WorldStreamer.jobList` (the streamer thread's sorted stack).
    job_list: Vec<ChunkId>,
    /// `IsoChunk.loadGridSquare`: served chunks waiting for the main thread.
    load_grid_square: VecDeque<(ChunkId, IsoChunk)>,
    /// `ChunkSaveWorker.toSaveQueue`.
    save_queue: VecDeque<QueuedSave>,
    /// `VehiclesDB2.worldStreamer.queue`: the rows `removeFromWorld` queued
    /// (`QueueUpdateVehicle`).
    vehicle_queue: VecDeque<crate::save::vehicle::VehicleRow>,
    events: Vec<StreamEvent>,
}

impl ChunkStreamer {
    /// Chunk maps for `num_players` local players (`IsoPlayer.numPlayers`)
    /// with a grid `width` chunks wide (see [`calc_chunk_width`]).
    pub fn new(num_players: usize, width: i32) -> Self {
        assert!(width > 0 && width % 2 == 1, "chunkGridWidth must be odd");
        ChunkStreamer {
            width,
            maps: (0..num_players)
                .map(|n| IsoChunkMap::new(n, width))
                .collect(),
            shared: HashMap::new(),
            records: HashMap::new(),
            next_id: 0,
            job_queue: VecDeque::new(),
            job_list: Vec::new(),
            load_grid_square: VecDeque::new(),
            save_queue: VecDeque::new(),
            vehicle_queue: VecDeque::new(),
            events: Vec::new(),
        }
    }

    /// The number of chunk maps (`IsoPlayer.numPlayers` at creation).
    pub fn num_players(&self) -> usize {
        self.maps.len()
    }

    pub fn chunk_map(&self, player: usize) -> &IsoChunkMap {
        &self.maps[player]
    }

    pub fn chunk_map_mut(&mut self, player: usize) -> &mut IsoChunkMap {
        &mut self.maps[player]
    }

    /// The events since the last call.
    pub fn take_events(&mut self) -> Vec<StreamEvent> {
        std::mem::take(&mut self.events)
    }

    /// `getChunk(x, y)` of a player's grid: the coordinates of the chunk in
    /// grid slot `(x, y)`, if one is there.
    pub fn grid_chunk(&self, player: usize, x: i32, y: i32) -> Option<(i32, i32)> {
        let id = self.get_chunk(player, x, y)?;
        let r = &self.records[&id];
        Some((r.wx, r.wy))
    }

    /// `WorldStreamer.isBusy()` (single player): jobs are waiting.
    pub fn is_busy(&self) -> bool {
        !self.job_queue.is_empty() || !self.job_list.is_empty()
    }

    /// Chunks waiting in `ChunkSaveWorker.toSaveQueue`.
    pub fn pending_saves(&self) -> usize {
        self.save_queue.len()
    }

    /// Chunks served and waiting for the main thread (`loadGridSquare`).
    pub fn pending_hand_offs(&self) -> usize {
        self.load_grid_square.len()
    }

    /// The chunk maps' `getWorldXMinTiles()`..`getWorldXMaxTiles()` (and Y):
    /// the tile rectangle a player's grid covers.
    pub fn tile_bounds(&self, player: usize) -> (i32, i32, i32, i32) {
        let m = &self.maps[player];
        let half = self.width / 2;
        let (x0, y0) = ((m.world_x - half) * 8, (m.world_y - half) * 8);
        (x0, y0, x0 + self.width * 8, y0 + self.width * 8)
    }

    // ---- grid access (`IsoChunkMap.getChunk`/`setChunk`) ----

    fn get_chunk(&self, m: usize, x: i32, y: i32) -> Option<ChunkId> {
        let w = self.width;
        if x < 0 || x >= w || y < 0 || y >= w {
            return None;
        }
        self.maps[m].read()[(w * y + x) as usize]
    }

    fn set_chunk(&mut self, m: usize, x: i32, y: i32, c: Option<ChunkId>) {
        let w = self.width;
        self.maps[m].write_mut()[(w * y + x) as usize] = c;
    }

    /// `SwapChunkBuffers()`.
    fn swap_chunk_buffers(&mut self, m: usize) {
        let map = &mut self.maps[m];
        map.read_mut().iter_mut().for_each(|c| *c = None);
        map.read_buffer_a = !map.read_buffer_a;
    }

    /// `refs.remove(this)` + `SharedChunks.remove` when no refs are left.
    /// Returns whether the chunk had this ref.
    fn release_ref(&mut self, id: ChunkId, m: usize) -> bool {
        let r = self.records.get_mut(&id).expect("chunk record");
        let Some(i) = r.refs.iter().position(|&p| p == m) else {
            return false;
        };
        r.refs.remove(i);
        if r.refs.is_empty() {
            let key = shared_key(r.wx, r.wy);
            if self.shared.get(&key) == Some(&id) {
                self.shared.remove(&key);
            }
        }
        true
    }

    fn new_chunk(&mut self, wx: i32, wy: i32) -> ChunkId {
        // `chunkStore.poll()` or `new IsoChunk`, then `assignLoadID()`.
        let id = self.next_id;
        self.next_id += 1;
        self.records.insert(
            id,
            Record {
                wx,
                wy,
                refs: Vec::new(),
                loaded: false,
            },
        );
        id
    }

    /// `WorldStreamer.addJob(chunk, wx, wy, ...)` (single player).
    fn add_job(&mut self, id: ChunkId, m: usize) {
        let r = &self.records[&id];
        self.events.push(StreamEvent::Requested {
            player: m,
            wx: r.wx,
            wy: r.wy,
        });
        debug_assert!(!self.job_queue.contains(&id) && !self.job_list.contains(&id));
        self.job_queue.push_back(id);
    }

    /// `IsoChunkMap.setInitialPos(wx, wy)`.
    pub fn set_initial_pos(&mut self, player: usize, wx: i32, wy: i32) {
        self.maps[player].world_x = wx;
        self.maps[player].world_y = wy;
    }

    /// `IsoChunkMap.LoadChunk(wx, wy, x, y)`: the initial load. A chunk
    /// another map already has is shared without a loaded check.
    pub fn load_chunk(&mut self, m: usize, wx: i32, wy: i32, x: i32, y: i32) {
        let key = shared_key(wx, wy);
        if let Some(&id) = self.shared.get(&key) {
            self.set_chunk(m, x, y, Some(id));
            self.records.get_mut(&id).unwrap().refs.push(m);
        } else {
            let id = self.new_chunk(wx, wy);
            self.shared.insert(key, id);
            self.records.get_mut(&id).unwrap().refs.push(m);
            self.add_job(id, m);
        }
    }

    /// `IsoChunkMap.LoadChunkForLater(wx, wy, x, y)`.
    fn load_chunk_for_later(
        &mut self,
        m: usize,
        wx: i32,
        wy: i32,
        x: i32,
        y: i32,
        meta: &IsoMetaGrid,
    ) {
        if !meta.is_valid_chunk(wx, wy) {
            return;
        }
        let key = shared_key(wx, wy);
        if let Some(&id) = self.shared.get(&key) {
            let r = self.records.get_mut(&id).unwrap();
            if !r.refs.contains(&m) {
                // `checkLightingLater_OnePlayer_AllLevels` is not ported.
                r.refs.push(m);
            }
            if !r.loaded {
                return;
            }
            self.set_chunk(m, x, y, Some(id));
        } else {
            let id = self.new_chunk(wx, wy);
            self.shared.insert(key, id);
            self.records.get_mut(&id).unwrap().refs.push(m);
            self.add_job(id, m);
        }
    }

    /// `IsoChunk.removeFromWorld()` + `ChunkSaveWorker.Add(chunk)`.
    fn remove_from_world(&mut self, id: ChunkId, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        let r = self.records.remove(&id).expect("chunk record");
        // A map that took the chunk without holding a ref (possible only
        // with overlapping split-screen maps) keeps a dangling slot in the
        // original; here the slot is cleared.
        for map in &mut self.maps {
            for slot in map.swap_a.iter_mut().chain(map.swap_b.iter_mut()) {
                if *slot == Some(id) {
                    *slot = None;
                }
            }
        }
        self.events
            .push(StreamEvent::Unloaded { wx: r.wx, wy: r.wy });
        if let Some(mut chunk) = cell.unload_chunk(r.wx, r.wy, chunk_hook(&mut ctx.chunk_hook)) {
            self.vehicles_to_meta(&mut chunk, ctx);
            self.save_queue.push_back(QueuedSave {
                wx: r.wx,
                wy: r.wy,
                chunk,
            });
        }
    }

    /// The edge column/row handling `Up`/`Down`/`Left`/`Right` share: a
    /// chunk still loading loses this map's ref; a loaded one is unloaded
    /// once no map references it.
    fn drop_edge(&mut self, m: usize, x: i32, y: i32, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        let half = self.width / 2;
        match self.get_chunk(m, x, y) {
            None => {
                let wx = self.maps[m].world_x - half + x;
                let wy = self.maps[m].world_y - half + y;
                if let Some(&id) = self.shared.get(&shared_key(wx, wy)) {
                    self.release_ref(id, m);
                }
            }
            Some(id) => {
                self.release_ref(id, m);
                if self.records[&id].refs.is_empty() {
                    self.remove_from_world(id, cell, ctx);
                }
            }
        }
    }

    /// `IsoChunkMap.Up/Down/Left/Right()`: shift the grid one chunk, the
    /// world position moving by `(dx, dy)`.
    fn shift(&mut self, m: usize, dx: i32, dy: i32, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        let w = self.width;
        // The slot that falls off is on the side opposite the movement.
        if dx != 0 {
            for y in 0..w {
                let xs: Vec<i32> = if dx < 0 {
                    (1..w).rev().collect()
                } else {
                    (0..w - 1).collect()
                };
                let edge = if dx < 0 { w - 1 } else { 0 };
                for x in xs {
                    if x == edge {
                        self.drop_edge(m, x, y, cell, ctx);
                    }
                    let c = self.get_chunk(m, x + dx, y);
                    self.set_chunk(m, x, y, c);
                }
                self.set_chunk(m, if dx < 0 { 0 } else { w - 1 }, y, None);
            }
            self.maps[m].world_x += dx;
        } else {
            for x in 0..w {
                let ys: Vec<i32> = if dy < 0 {
                    (1..w).rev().collect()
                } else {
                    (0..w - 1).collect()
                };
                let edge = if dy < 0 { w - 1 } else { 0 };
                for y in ys {
                    if y == edge {
                        self.drop_edge(m, x, y, cell, ctx);
                    }
                    let c = self.get_chunk(m, x, y + dy);
                    self.set_chunk(m, x, y, c);
                }
                self.set_chunk(m, x, if dy < 0 { 0 } else { w - 1 }, None);
            }
            self.maps[m].world_y += dy;
        }
    }

    /// `LoadLeft/LoadRight/LoadUp/LoadDown`: shift, queue the new edge,
    /// swap buffers. (`WorldSimulation.scrollGround*` and
    /// `LightingThread.scroll*` are not ported; `UpdateCellCache` is
    /// empty.)
    fn load_side(
        &mut self,
        m: usize,
        dx: i32,
        dy: i32,
        cell: &mut IsoCell,
        ctx: &mut StreamCtx<'_>,
    ) {
        self.shift(m, dx, dy, cell, ctx);
        let half = self.width / 2;
        let (wx, wy) = (self.maps[m].world_x, self.maps[m].world_y);
        for i in -half..=half {
            if dx != 0 {
                let col = if dx < 0 { 0 } else { self.width - 1 };
                self.load_chunk_for_later(m, wx + dx * half, wy + i, col, i + half, ctx.meta);
            } else {
                let row = if dy < 0 { 0 } else { self.width - 1 };
                self.load_chunk_for_later(m, wx + i, wy + dy * half, i + half, row, ctx.meta);
            }
        }
        self.swap_chunk_buffers(m);
    }

    /// `releaseChunkBeingLoaded(wx, wy)`.
    fn release_chunk_being_loaded(&mut self, m: usize, wx: i32, wy: i32) {
        if let Some(&id) = self.shared.get(&shared_key(wx, wy)) {
            self.release_ref(id, m);
        }
    }

    /// `IsoChunkMap.Unload()`: release every chunk of the map (used when
    /// the player moves a whole grid away).
    pub fn unload(&mut self, m: usize, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        let w = self.width;
        let half = w / 2;
        for y in 0..w {
            for x in 0..w {
                match self.get_chunk(m, x, y) {
                    None => {
                        let (wx, wy) = (
                            self.maps[m].world_x - half + x,
                            self.maps[m].world_y - half + y,
                        );
                        self.release_chunk_being_loaded(m, wx, wy);
                        continue;
                    }
                    Some(id) => {
                        if self.release_ref(id, m) && self.records[&id].refs.is_empty() {
                            self.remove_from_world(id, cell, ctx);
                        }
                    }
                }
                let i = (y * w + x) as usize;
                self.maps[m].swap_a[i] = None;
                self.maps[m].swap_b[i] = None;
            }
        }
    }

    /// `IsoChunkMap.ProcessChunkPos(player)`: follow the player by at most
    /// one chunk per axis step, or reload the whole grid after a jump of a
    /// grid width or more. (The vehicle look-ahead is not ported.)
    pub fn process_chunk_pos(
        &mut self,
        m: usize,
        p: PlayerPos,
        cell: &mut IsoCell,
        ctx: &mut StreamCtx<'_>,
    ) {
        let x = fastfloor(p.x / 8.0);
        let y = fastfloor(p.y / 8.0);
        let (wx, wy) = (self.maps[m].world_x, self.maps[m].world_y);
        if x == wx && y == wy {
            return;
        }
        let w = self.width;
        if (x - wx).abs() >= w || (y - wy).abs() >= w {
            self.unload(m, cell, ctx);
            self.maps[m].world_x = x;
            self.maps[m].world_y = y;
            let half = w / 2;
            let (minwx, minwy) = (x - half, y - half);
            for xx in minwx..=x + half {
                for yy in minwy..=y + half {
                    self.load_chunk_for_later(m, xx, yy, xx - minwx, yy - minwy, ctx.meta);
                }
            }
            self.swap_chunk_buffers(m);
        } else if x != wx {
            self.load_side(m, if x < wx { -1 } else { 1 }, 0, cell, ctx);
        } else {
            self.load_side(m, 0, if y < wy { -1 } else { 1 }, cell, ctx);
        }
        self.calculate_z_extents(m, cell);
    }

    /// `calculateZExtentsForChunkMap()`.
    fn calculate_z_extents(&mut self, m: usize, cell: &IsoCell) {
        let (mut max, mut min) = (0, 0);
        for id in self.maps[m].read().iter().flatten() {
            let r = &self.records[id];
            if let Some(c) = cell.chunks.get(&(r.wx, r.wy)) {
                max = max.max(c.max_level);
                min = min.min(c.min_level);
            }
        }
        self.maps[m].max_height = max;
        self.maps[m].min_height = min;
    }

    // ---- WorldStreamer thread ----

    /// `ChunkComparator.init()`: per player, the position 10 tiles ahead
    /// of the last movement, or the position itself; `(-1, -1)` when there
    /// is no player.
    fn comparator_targets(players: &[Option<PlayerPos>]) -> [(f32, f32); 4] {
        let mut out = [(-1.0f32, -1.0f32); 4];
        for (o, p) in out.iter_mut().zip(players) {
            let Some(p) = p else { continue };
            if p.last_x != p.x || p.last_y != p.y {
                let (mut vx, mut vy) = (p.x - p.last_x, p.y - p.last_y);
                normalize(&mut vx, &mut vy);
                normalize(&mut vx, &mut vy);
                *o = (vx * 10.0 + p.x, vy * 10.0 + p.y);
            } else {
                *o = (p.x, p.y);
            }
        }
        out
    }

    /// One `WorldStreamer.threadLoop()` iteration (single player). Returns
    /// whether it had a job or a save to do.
    fn thread_loop(&mut self, players: &[Option<PlayerPos>], ctx: &mut StreamCtx<'_>) -> bool {
        while let Some(id) = self.job_queue.pop_front() {
            if !self.job_list.contains(&id) {
                self.job_list.push(id);
            }
        }
        if self.job_list.is_empty() {
            let saved = self.save_worker_update(None, ctx);
            if !saved {
                self.update_vehicles_db(ctx);
            }
            return saved;
        }
        for i in (0..self.job_list.len()).rev() {
            let id = self.job_list[i];
            if self.records[&id].refs.is_empty() {
                self.job_list.remove(i);
                self.cancel(id);
            }
        }
        let next = if self.job_list.is_empty() {
            None
        } else {
            let targets = Self::comparator_targets(players);
            let records = &self.records;
            self.job_list.sort_by(|a, b| {
                let (ra, rb) = (&records[a], &records[b]);
                compare_chunks(&targets, (ra.wx, ra.wy), (rb.wx, rb.wy))
            });
            self.job_list.pop()
        };
        let about = next.map(|id| (self.records[&id].wx, self.records[&id].wy));
        let saved = self.save_worker_update(about, ctx);
        if next.is_none() && !saved {
            self.update_vehicles_db(ctx);
        }
        if let Some(id) = next {
            if self.records[&id].refs.is_empty() {
                self.cancel(id);
            } else {
                self.do_chunk(id, ctx);
            }
        }
        true
    }

    /// A job back to `chunkStore`.
    fn cancel(&mut self, id: ChunkId) {
        let r = self.records.remove(&id).expect("chunk record");
        self.events
            .push(StreamEvent::Cancelled { wx: r.wx, wy: r.wy });
    }

    /// `ChunkSaveWorker.Update(aboutToLoad)`: write one queued save, the
    /// chunk about to be loaded first. Returns whether there was one.
    fn save_worker_update(&mut self, about: Option<(i32, i32)>, ctx: &mut StreamCtx<'_>) -> bool {
        if self.save_queue.is_empty() {
            return false;
        }
        let i = about
            .and_then(|(wx, wy)| {
                self.save_queue
                    .iter()
                    .position(|q| q.wx == wx && q.wy == wy)
            })
            .unwrap_or(0);
        let q = self.save_queue.remove(i).unwrap();
        self.write_queued_save(q, ctx);
        // `HotsaveAncilliarySystems` (single player): the engine runs the
        // main-thread part when it sees the event.
        if self.save_queue.is_empty() {
            self.events.push(StreamEvent::HotSave);
        }
        true
    }

    /// `WriteQueuedSave` → `IsoChunk.Save(false)`.
    fn write_queued_save(&mut self, q: QueuedSave, ctx: &mut StreamCtx<'_>) {
        self.save_chunk_file(q.wx, q.wy, &q.chunk, ctx);
    }

    /// `IsoChunk.Save(bPreventChunkReuse)` (not a hot save): the chunk's
    /// bytes, then `VehiclesDB2.unloadChunk` (inside `Save(ByteBuffer)`,
    /// after the erosion data), then `SafeWrite`. Nothing with saving off.
    fn save_chunk_file(&mut self, wx: i32, wy: i32, chunk: &IsoChunk, ctx: &mut StreamCtx<'_>) {
        if ctx.no_save {
            self.events.push(StreamEvent::Saved {
                wx,
                wy,
                written: false,
            });
            return;
        }
        let save_ctx = SaveCtx {
            sprites: ctx.sprites,
            tables: ctx.save_tables,
            settings: ctx.save_settings,
            items: ctx.items,
            hooks: ctx.save_hooks,
            meta: Some(ctx.meta),
        };
        let bytes = save_chunk(chunk, &save_ctx).map_err(|e| e.0);
        if bytes.is_ok() {
            if let Some(conn) = ctx.vehicles_db.as_deref_mut() {
                let default_saver = vehicles_db::LoadedVehicleSaver {
                    scripts: &vehicles_db::NoVehicleScripts,
                };
                let saver = ctx.vehicle_saver.unwrap_or(&default_saver);
                let r = vehicles_db::unload_chunk(Some(conn), chunk, &save_ctx, saver);
                self.events.push(StreamEvent::VehiclesUnloaded {
                    wx,
                    wy,
                    saved: r.saved,
                    errors: r.errors,
                });
            }
        }
        let result = bytes.and_then(|bytes| {
            let path = chunk_path(ctx.save_dir, wx, wy);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| e.to_string())
        });
        self.events.push(match result {
            Ok(()) => StreamEvent::Saved {
                wx,
                wy,
                written: true,
            },
            Err(error) => StreamEvent::SaveFailed { wx, wy, error },
        });
    }

    /// `WorldStreamer.DoChunk(chunk, null)` → `DoChunkAlways`:
    /// `IsoChunk.LoadChunk` (the save file if there is one, else, or when
    /// it fails, `LoadBrandNew`), then `VehiclesDB2.instance.loadChunk`
    /// for either kind of chunk, before `loadInWorldStreamerThread` creates
    /// the missing squares (which here runs at hand-off, so vehicles see
    /// the same squares). `ZoneGenerator.genForaging` is not ported.
    fn do_chunk(&mut self, id: ChunkId, ctx: &mut StreamCtx<'_>) {
        let (wx, wy) = (self.records[&id].wx, self.records[&id].wy);
        let path = chunk_path(ctx.save_dir, wx, wy);
        let mut source = ChunkSource::BrandNew;
        let mut chunk = None;
        if path.exists() {
            let loaded = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| load_chunk(&b, wx, wy, &ctx.save_ctx()).map_err(|e| e.0));
            match loaded {
                Ok(l) => {
                    source = ChunkSource::Disk;
                    chunk = Some(l.chunk);
                }
                Err(e) => {
                    backup_blam(ctx.save_dir, wx, wy, &e);
                    source = ChunkSource::BrandNewAfterError(e);
                }
            }
        }
        let mut chunk = match chunk {
            Some(c) => c,
            None => {
                let mut env = ChunkLoadEnv {
                    sprites: ctx.sprites,
                    fix2x: ctx.fix2x,
                    rand: ctx.rand,
                    tables: ctx.object_tables,
                    settings: ctx.object_settings,
                    hooks: ctx.object_hooks,
                };
                // The other three chunks of the 2x2 block go back to the
                // chunk store.
                load_brand_new(
                    ctx.meta,
                    wx,
                    wy,
                    &mut env,
                    brand_new_hook(&mut ctx.brand_new),
                )
                .0
            }
        };
        self.events.push(StreamEvent::Served { wx, wy, source });
        if let Some(conn) = ctx.vehicles_db.as_deref_mut() {
            let save_ctx = SaveCtx {
                sprites: ctx.sprites,
                tables: ctx.save_tables,
                settings: ctx.save_settings,
                items: ctx.items,
                hooks: ctx.save_hooks,
                meta: Some(ctx.meta),
            };
            let r = vehicles_db::load_chunk(Some(conn), &mut chunk, &save_ctx);
            self.events.push(StreamEvent::VehiclesLoaded {
                wx,
                wy,
                added: r.added.len(),
                removed: r.removed,
                error: r.error,
            });
        }
        self.load_grid_square.push_back((id, chunk));
    }

    // ---- main-thread hand-off ----

    /// `IsoChunkMap.setChunkDirect(chunk, false)`.
    fn set_chunk_direct(&mut self, m: usize, id: ChunkId) -> bool {
        let half = self.width / 2;
        let r = &self.records[&id];
        let x = r.wx - self.maps[m].world_x + half;
        let y = r.wy - self.maps[m].world_y + half;
        let w = self.width;
        if r.refs.is_empty() || x < 0 || y < 0 || x >= w || y >= w {
            self.release_ref(id, m);
            return false;
        }
        self.maps[m].read_mut()[(w * y + x) as usize] = Some(id);
        self.records.get_mut(&id).unwrap().loaded = true;
        true
    }

    /// Give a served chunk to every chunk map that covers it, then put it
    /// in the cell and finish loading it (`doLoadGridsquare`), or drop it.
    /// Returns whether it was taken.
    fn hand_off(
        &mut self,
        id: ChunkId,
        chunk: IsoChunk,
        cell: &mut IsoCell,
        ctx: &mut StreamCtx<'_>,
    ) -> bool {
        let mut loaded = false;
        for n in 0..self.maps.len() {
            if self.maps[n].ignore || !self.set_chunk_direct(n, id) {
                continue;
            }
            loaded = true;
        }
        let (wx, wy) = (self.records[&id].wx, self.records[&id].wy);
        if !loaded {
            self.records.remove(&id);
            self.events.push(StreamEvent::Discarded { wx, wy });
            return false;
        }
        debug_assert!(!cell.chunks.contains_key(&(wx, wy)));
        cell.chunks.insert((wx, wy), chunk);
        finish_chunk_load(
            cell,
            wx,
            wy,
            ctx.meta,
            ctx.sprites,
            chunk_hook(&mut ctx.chunk_hook),
        );
        self.events.push(StreamEvent::HandedOff { wx, wy });
        // `doLoadGridsquare`, after `loadInMainThread`:
        // `metaCell.checkAnimalZonesGenerated(wx, wy)`.
        let (cx, cy) = (fastfloor(wx as f32 / 32.0), fastfloor(wy as f32 / 32.0));
        if ctx.meta.check_animal_zones_generated(cx, cy) {
            self.events.push(StreamEvent::GenAnimalsPath { wx, wy });
        }
        true
    }

    /// `IsoChunkMap.update()` (`updateInternal`) for one player: hand off
    /// up to `1 + n * 3 / chunkGridWidth` of the `n` served chunks.
    /// (`IsoChunk.update`, hot saves and vehicles are not ported.)
    pub fn chunk_map_update(&mut self, m: usize, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        let mut changed = false;
        let mut count = self.load_grid_square.len() as i32;
        if count != 0 {
            count = 1 + count * 3 / self.width;
        }
        while count > 0 {
            if let Some((id, chunk)) = self.load_grid_square.pop_front() {
                if !self.hand_off(id, chunk, cell, ctx) {
                    count -= 1;
                    continue;
                }
                changed = true;
            }
            count -= 1;
        }
        if changed {
            self.calculate_z_extents(m, cell);
        }
    }

    /// `IsoChunkMap.processAllLoadGridSquare()`: hand off every served
    /// chunk.
    pub fn process_all_load_grid_square(&mut self, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        while let Some((id, chunk)) = self.load_grid_square.pop_front() {
            self.hand_off(id, chunk, cell, ctx);
        }
    }

    /// Run the streamer thread: see [`Serve`].
    pub fn serve(&mut self, players: &[Option<PlayerPos>], serve: Serve, ctx: &mut StreamCtx<'_>) {
        match serve {
            Serve::Jobs(n) => {
                for _ in 0..n {
                    if !self.thread_loop(players, ctx) {
                        break;
                    }
                }
            }
            Serve::UntilIdle => while self.thread_loop(players, ctx) {},
        }
    }

    /// Loading a game (`CellLoader.LoadCellBinaryChunk` and the rest of
    /// `IsoWorld.init`, then `IngameState.enter`): player 0's map is
    /// centred on chunk `(wx, wy)`, every valid chunk of the grid is
    /// requested (x outer, y inner), the streamer runs until idle and all
    /// chunks are handed off. No player exists yet while the streamer
    /// sorts, so the comparator ties everything and the last request is
    /// served first.
    pub fn load_world(&mut self, wx: i32, wy: i32, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        self.set_initial_pos(0, wx, wy);
        let half = self.width / 2;
        let (minwx, minwy) = (wx - half, wy - half);
        for x in minwx..wx + half + 1 {
            for y in minwy..wy + half + 1 {
                if ctx.meta.is_valid_chunk(x, y) {
                    self.load_chunk(0, x, y, x - minwx, y - minwy);
                }
            }
        }
        self.serve(&[], Serve::UntilIdle, ctx);
        self.process_all_load_grid_square(cell, ctx);
        self.calculate_z_extents(0, cell);
        self.chunk_map_update(0, cell, ctx);
    }

    /// One game frame: `ProcessChunkPos` for each player (`players[n]` is
    /// player `n`; `None` skips it), the streamer's iterations, then
    /// `IsoChunkMap.update` for each player.
    pub fn update(
        &mut self,
        players: &[Option<PlayerPos>],
        serve: Serve,
        cell: &mut IsoCell,
        ctx: &mut StreamCtx<'_>,
    ) {
        for n in 0..self.maps.len() {
            if let Some(&Some(p)) = players.get(n) {
                if !self.maps[n].ignore {
                    self.process_chunk_pos(n, p, cell, ctx);
                }
            }
        }
        self.serve(players, serve, ctx);
        for n in 0..self.maps.len() {
            if let Some(Some(_)) = players.get(n) {
                self.chunk_map_update(n, cell, ctx);
            }
        }
    }

    /// `WorldStreamer.stop()` → `ChunkSaveWorker.SaveNow()`: write every
    /// queued save (quitting).
    pub fn save_now(&mut self, ctx: &mut StreamCtx<'_>) {
        while let Some(q) = self.save_queue.pop_front() {
            self.write_queued_save(q, ctx);
        }
    }

    /// The start of `IsoCell.save`: wait while `ChunkSaveWorker.saving`,
    /// i.e. let the streamer thread write the queued saves one per loop
    /// (`ChunkSaveWorker.Update(null)`), ending with the hot save event.
    /// The chunk loads the streamer would serve in between are left
    /// queued.
    pub fn wait_for_saves(&mut self, ctx: &mut StreamCtx<'_>) {
        while self.save_worker_update(None, ctx) {}
    }

    /// `IsoChunkMap.Save()` for every player's map in player order
    /// (`IsoCell.save`): each chunk of the grid, x outer and y inner, is
    /// saved with `IsoChunk.Save(true)` (file and vehicles; it stays loaded
    /// and is not reused). A chunk in two maps is saved twice, like the
    /// original.
    pub fn save_chunk_maps(&mut self, cell: &IsoCell, ctx: &mut StreamCtx<'_>) {
        for m in 0..self.maps.len() {
            for x in 0..self.width {
                for y in 0..self.width {
                    let Some(id) = self.get_chunk(m, x, y) else {
                        continue;
                    };
                    let (wx, wy) = (self.records[&id].wx, self.records[&id].wy);
                    if let Some(chunk) = cell.chunks.get(&(wx, wy)) {
                        self.save_chunk_file(wx, wy, chunk, ctx);
                    }
                }
            }
        }
    }

    /// `IngameState.exit` after the game was saved: `WorldStreamer.stop()`
    /// (pending jobs are dropped, then `ChunkSaveWorker.SaveNow()`), then
    /// every chunk of every player's map (`n % width`, `n / width`) loses
    /// that map's ref and leaves the world once no map holds it
    /// (`removeFromWorld`, [`StreamEvent::Removed`]), without another save.
    /// The loaded chunks waiting for the hand-off stay with the streamer
    /// and are dropped with it.
    pub fn stop(&mut self, cell: &mut IsoCell, ctx: &mut StreamCtx<'_>) {
        self.job_list.clear();
        self.job_queue.clear();
        self.save_now(ctx);
        let w = self.width;
        for m in 0..self.maps.len() {
            for n in 0..w * w {
                let Some(id) = self.get_chunk(m, n % w, n / w) else {
                    continue;
                };
                let Some(r) = self.records.get(&id) else {
                    continue;
                };
                if !r.refs.contains(&m) {
                    continue;
                }
                self.release_ref(id, m);
                if !self.records[&id].refs.is_empty() {
                    continue;
                }
                let r = self.records.remove(&id).expect("chunk record");
                self.events
                    .push(StreamEvent::Removed { wx: r.wx, wy: r.wy });
                if let Some(mut chunk) =
                    cell.unload_chunk(r.wx, r.wy, chunk_hook(&mut ctx.chunk_hook))
                {
                    self.vehicles_to_meta(&mut chunk, ctx);
                }
            }
        }
    }

    /// The vehicle part of `IsoChunk.removeFromWorld`
    /// ([`vehicles_db::remove_vehicles_to_meta`]) with this context's saver
    /// and meta hook; the rows join the streamer's vehicle queue.
    fn vehicles_to_meta(&mut self, chunk: &mut IsoChunk, ctx: &mut StreamCtx<'_>) {
        if chunk.vehicles.is_empty() {
            return;
        }
        let default_saver = vehicles_db::LoadedVehicleSaver {
            scripts: &vehicles_db::NoVehicleScripts,
        };
        let saver = ctx.vehicle_saver.unwrap_or(&default_saver);
        let save_ctx = SaveCtx {
            sprites: ctx.sprites,
            tables: ctx.save_tables,
            settings: ctx.save_settings,
            items: ctx.items,
            hooks: ctx.save_hooks,
            meta: Some(ctx.meta),
        };
        let hook = ctx
            .vehicle_meta
            .as_mut()
            .map(|h| &mut **h as &mut dyn vehicles_db::VehicleMetaHook);
        let r = vehicles_db::remove_vehicles_to_meta(
            chunk,
            &save_ctx,
            saver,
            ctx.vehicle_ids.as_deref_mut(),
            hook,
        );
        if r.to_meta.is_empty() && r.errors.is_empty() {
            return;
        }
        self.vehicle_queue.extend(r.queued);
        self.events.push(StreamEvent::VehiclesToMeta {
            wx: chunk.wx,
            wy: chunk.wy,
            ids: r.to_meta,
            errors: r.errors,
        });
    }

    /// `VehiclesDB2.instance.updateWorldStreamer()`: the queued rows are
    /// written with the store's connection. The streamer thread does this
    /// at the end of a loop that served no chunk and wrote no save;
    /// `VehiclesDB2.Reset` (quitting, after [`ChunkStreamer::stop`]) does it
    /// once more ([`ChunkStreamer::take_vehicle_queue`]).
    pub fn update_vehicles_db(&mut self, ctx: &mut StreamCtx<'_>) {
        if self.vehicle_queue.is_empty() {
            return;
        }
        let r = vehicles_db::update_world_streamer(
            ctx.vehicles_db.as_deref_mut(),
            &mut self.vehicle_queue,
        );
        self.events.push(StreamEvent::VehiclesUpdated {
            saved: r.saved,
            errors: r.errors,
        });
    }

    /// `VehiclesDB2.updateVehicle` from the main thread (the vehicle
    /// runtime's virtual vehicles when they stop updating in the meta, or
    /// on a game save): the rows join the queue [`Self::update_vehicles_db`]
    /// writes.
    pub fn queue_vehicle_updates(
        &mut self,
        rows: impl IntoIterator<Item = crate::save::vehicle::VehicleRow>,
    ) {
        self.vehicle_queue.extend(rows);
    }

    /// The vehicle rows still queued for the streamer thread, for
    /// `VehiclesDB2.Reset` once the streamer is stopped.
    pub fn take_vehicle_queue(&mut self) -> VecDeque<crate::save::vehicle::VehicleRow> {
        std::mem::take(&mut self.vehicle_queue)
    }
}

/// `Vector2.normalize()`.
fn normalize(x: &mut f32, y: &mut f32) {
    let len_sq = *x * *x + *y * *y;
    if (1.0 - len_sq).abs() < 1.0e-5 {
        return;
    }
    if len_sq == 0.0 {
        (*x, *y) = (0.0, 0.0);
        return;
    }
    let len = (len_sq as f64).sqrt() as f32;
    *x /= len;
    *y /= len;
}

/// `IsoUtils.DistanceToSquared(fromX, fromY, toX, toY)`.
fn distance_to_squared(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    let (dx, dy) = ((tx - fx) as f64, (ty - fy) as f64);
    (dx * dx + dy * dy) as f32
}

/// `ChunkComparator.compare(a, b)`: the farther chunk sorts first.
fn compare_chunks(targets: &[(f32, f32); 4], a: (i32, i32), b: (i32, i32)) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut sa, mut sb) = (f32::MAX, f32::MAX);
    for &(x, y) in targets {
        if x == -1.0 && y == -1.0 {
            continue;
        }
        sa = sa.min(distance_to_squared(
            x,
            y,
            (a.0 * 8) as f32 + 4.0,
            (a.1 * 8) as f32 + 4.0,
        ));
        sb = sb.min(distance_to_squared(
            x,
            y,
            (b.0 * 8) as f32 + 4.0,
            (b.1 * 8) as f32 + 4.0,
        ));
    }
    if sa < sb {
        Ordering::Greater
    } else if sa > sb {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// `IsoChunk.BackupBlam`: keep the error and a copy of the chunk file that
/// failed to load under `blam/<wx>/`.
fn backup_blam(save_dir: &Path, wx: i32, wy: i32, error: &str) {
    let dir = save_dir.join("blam").join(wx.to_string());
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join(format!("{wy}_error.txt")), error);
    let src = chunk_path(save_dir, wx, wy);
    if src.exists() {
        let _ = std::fs::copy(&src, dir.join(format!("{wy}.bin")));
    }
}

#[cfg(test)]
mod tests;
