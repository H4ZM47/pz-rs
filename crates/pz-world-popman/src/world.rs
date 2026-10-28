//! The population manager over a loaded world, in the original's call
//! order:
//!
//! 1. `IsoWorld.init`, after the meta grid and before the cell load:
//!    `MapCollisionData.init(metaGrid)` then
//!    `ZombiePopulationManager.init(metaGrid)` ([`WorldPopulation::init`]).
//!    The library reads `zpop_ids.bin` and `zpop_virtual.bin` here and each
//!    cell's `zpop_X_Y.bin` the first time a player's loaded area touches
//!    it.
//! 2. Every chunk `doLoadGridsquare` adds (not on a soft reset):
//!    `MapCollisionData.addChunkToWorld(chunk)` then
//!    `ZombiePopulationManager.addChunkToWorld(chunk)`
//!    ([`WorldPopulation::add_chunk_to_world`]); every chunk
//!    `IsoChunk.removeFromWorld` takes out:
//!    `ZombiePopulationManager.removeChunkFromWorld(chunk)` then
//!    `requestSaveCell` of its population cell
//!    ([`WorldPopulation::remove_chunk_from_world`]).
//! 3. `IngameState.enter` after the world loaded:
//!    `MapCollisionData.startGame()` ([`WorldPopulation::start_game`]).
//! 4. Per frame: `GameWindow.logic`'s `MapCollisionData.updateGameState()`
//!    ([`WorldPopulation::update_game_state`]), then in `IngameState.update`
//!    `MapCollisionData.updateMain()` and
//!    `ZombiePopulationManager.updateMain()` ([`WorldPopulation::update`]),
//!    which creates the real zombies through the [`PopulationHost`].
//! 5. On quit: `MapCollisionData.stop()` then
//!    `ZombiePopulationManager.stop()` ([`WorldPopulation::stop`]).
//!
//! The original's worker thread (`MCDThread`, the library's
//! `updateThread`) runs here in line, after the main-thread update of the
//! same frame ([`PopulationWorker`]).

use std::path::Path;

use pz_popman::manager::{ChunkZombie, RealZombieInfo};
use pz_popman::map_collision::{meta_init, McdGameState, SquareFlags};
use pz_popman::native::Native;
use pz_popman::{PopulationHost, ZombiePopulationManager};
use pz_sandbox::ZombieConfig;
use pz_world::sprite::IsoSpriteManager;
use pz_world::{IsoCell, IsoGridSquare, IsoMetaGrid};

use crate::PlayerChunkMaps;

/// `MCDThread.runInner`'s frame rate: one pass per 100 ms (`Sync`, 10 fps).
pub const WORKER_PERIOD_MS: i64 = 100;

/// The directory fields of [`McdGameState`] for the save in `save_dir`
/// (`<Saves>/<game mode>/<world>`): `GameModeCacheDir` is
/// `<Saves>/<game mode>/`, `GameSaveWorld` the world folder's name,
/// `SaveDir` `<Saves>` and `CacheDir` its parent. The population files are
/// `<save_dir>/zpop/zpop_*.bin`, the collision cache
/// `<save_dir>/chunkdata/chunkdata_X_Y.bin`. The sandbox and mode fields
/// are the caller's.
pub fn save_game_state(save_dir: &Path) -> McdGameState {
    let mode_dir = save_dir.parent().unwrap_or(Path::new(""));
    let saves = mode_dir.parent().unwrap_or(Path::new(""));
    McdGameState {
        game_save_world: save_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        game_mode_cache_dir: format!("{}/", mode_dir.display()),
        save_dir: saves.display().to_string(),
        cache_dir: saves
            .parent()
            .unwrap_or(Path::new(""))
            .display()
            .to_string(),
        ..McdGameState::default()
    }
}

/// `MapCollisionData.isSolid/isBlockedN/isBlockedW/isWater/isRoom(sq)`.
pub fn square_collision_flags(
    sq: &IsoGridSquare,
    meta: &IsoMetaGrid,
    m: &IsoSpriteManager,
) -> SquareFlags {
    let f = &m.tables.flag;
    let ty = &m.tables.ty;
    let named = |name: &str| m.tables.flag_from_string(name).is_some_and(|fl| sq.has(fl));
    SquareFlags {
        solid: sq.is_solid(m) || sq.is_solid_trans(m),
        has_stairs: sq.has_stairs(m),
        water: sq.has(f.water),
        tree: sq.has_type(ty.tree),
        hoppable_n: sq.has(f.HoppableN),
        blocks_n: sq.has(f.collideN)
            || sq.has_type(ty.doorFrN)
            || named("DoorWallN")
            || sq.has_type(ty.windowFN)
            || sq.has(f.windowN)
            || sq.has(f.WindowN),
        hoppable_w: sq.has(f.HoppableW),
        blocks_w: sq.has(f.collideW)
            || sq.has_type(ty.doorFrW)
            || named("DoorWallW")
            || sq.has_type(ty.windowFW)
            || sq.has(f.windowW)
            || sq.has(f.WindowW),
        room: sq.room_def(meta).is_some(),
    }
}

/// `MapCollisionData.addChunkToWorld`'s square bits of chunk `(wx, wy)`:
/// level 0, `x + y * 8`, `BIT_SOLID` where there is no square.
pub fn chunk_collision_bits(
    cell: &IsoCell,
    meta: &IsoMetaGrid,
    m: &IsoSpriteManager,
    wx: i32,
    wy: i32,
) -> [u8; 64] {
    let mut bits = [1u8; 64];
    let Some(chunk) = cell.chunks.get(&(wx, wy)) else {
        return bits;
    };
    for y in 0..8 {
        for x in 0..8 {
            if let Some(sq_owner) = chunk.get_grid_square(x, y, 0) {
                let sq = sq_owner.borrow();
                bits[(x + y * 8) as usize] = square_collision_flags(&sq, meta, m).bits();
            }
        }
    }
    bits
}

/// What one frame of the population reads from the game.
pub struct PopulationFrame<'a, H> {
    /// `GameTime.getMultiplier()`.
    pub multiplier: f32,
    /// `GameTime.getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// The local players' chunk maps ([`crate::player_chunk_maps`]).
    pub chunk_maps: &'a PlayerChunkMaps,
    /// The cell's zombie list (`IsoCell.getZombieList()`), as
    /// `updateRealZombies` packs it.
    pub real_zombies: &'a [RealZombieInfo<H>],
}

/// `MapCollisionData.MCDThread` run in line: a pass (`runInner`) every
/// [`WORKER_PERIOD_MS`] while it has work; once the library and the
/// collision grid say it should wait it sleeps until the main thread
/// notifies it (data for the thread, or `MapCollisionData.updateMain`'s
/// ten-second wake-up).
#[derive(Clone, Copy, Debug, Default)]
pub struct PopulationWorker {
    /// `thread.isAlive()` (started by `startGame`).
    pub started: bool,
    /// `waiting`: blocked on the notifier.
    pub waiting: bool,
    /// When the next pass may run (the `Sync` frame limit).
    pub next_pass_ms: i64,
    /// Passes run so far.
    pub passes: u64,
}

impl PopulationWorker {
    /// `notifyThread()`.
    pub fn notify(&mut self) {
        self.waiting = false;
    }

    /// Runs the passes the thread would have run by `now`: at most one, as
    /// the thread runs at 10 fps and the game at 30 or more.
    pub fn run<H: Copy + Eq>(&mut self, pm: &mut ZombiePopulationManager<H>, now: i64) {
        if !self.started || self.waiting || now < self.next_pass_ms {
            return;
        }
        self.next_pass_ms = now + WORKER_PERIOD_MS;
        self.passes += 1;
        self.waiting = pm.thread_step();
    }
}

/// `ZombiePopulationManager.instance` and `MapCollisionData.instance` of a
/// loaded world, with the worker thread they share.
pub struct WorldPopulation<H> {
    pub manager: ZombiePopulationManager<H>,
    pub worker: PopulationWorker,
}

impl<H: Copy + Eq> WorldPopulation<H> {
    /// `IsoWorld.init`'s `MapCollisionData.init(metaGrid)` (game state,
    /// map `chunkdata` files and zombie intensity per chunk) then
    /// `ZombiePopulationManager.init(metaGrid)` (`n_init`, which reads
    /// `zpop_ids.bin` and `zpop_virtual.bin`, the sandbox `ZombieConfig`
    /// and `PersistentOutfits.getOutfitNames()`). `gs` is
    /// [`save_game_state`] plus the sandbox and mode fields.
    pub fn init(
        native: Native,
        meta: &IsoMetaGrid,
        gs: &McdGameState,
        config: &ZombieConfig,
        outfit_names: &[String],
    ) -> Self {
        Self::init_with_spawn_origins(native, meta, gs, config, outfit_names, &[])
    }

    /// [`WorldPopulation::init`] with the spawn origins Lua registered
    /// before (`IsoWorld.registerSpawnOrigin` on `OnLoadMapZones`), which
    /// `init` hands to the native side: `(x, y, w, h)`.
    pub fn init_with_spawn_origins(
        native: Native,
        meta: &IsoMetaGrid,
        gs: &McdGameState,
        config: &ZombieConfig,
        outfit_names: &[String],
        spawn_origins: &[(i32, i32, i32, i32)],
    ) -> Self {
        let mut pm = ZombiePopulationManager::new(native);
        for &(x, y, w, h) in spawn_origins {
            pm.register_spawn_origin(x, y, w, h);
        }
        pm.last_stand = gs.last_stand;
        let mi = meta_init(meta);
        let client = pm.client;
        pm.mcd.init(&mut pm.native, gs, &mi, client);
        pm.init(
            (meta.min_x, meta.min_y, meta.width, meta.height),
            config,
            outfit_names,
        );
        WorldPopulation {
            manager: pm,
            worker: PopulationWorker::default(),
        }
    }

    fn now(&self) -> i64 {
        self.manager.native.gs.now()
    }

    /// `doLoadGridsquare`'s `MapCollisionData.addChunkToWorld(chunk)` then
    /// `ZombiePopulationManager.addChunkToWorld(chunk)` for chunk
    /// `(wx, wy)` of `cell` (`isNewChunk()` is the chunk's `add_zombies`).
    pub fn add_chunk_to_world(
        &mut self,
        cell: &IsoCell,
        meta: &IsoMetaGrid,
        m: &IsoSpriteManager,
        wx: i32,
        wy: i32,
    ) {
        let Some(chunk) = cell.chunks.get(&(wx, wy)) else {
            return;
        };
        let new_chunk = chunk.add_zombies;
        let bits = chunk_collision_bits(cell, meta, m, wx, wy);
        let pm = &mut self.manager;
        pm.mcd.add_chunk_to_world(&mut pm.native, wx, wy, &bits);
        pm.add_chunk_to_world(wx, wy, new_chunk);
    }

    /// `IsoChunk.removeFromWorld`'s `ZombiePopulationManager.
    /// removeChunkFromWorld(chunk)` (virtualizes `zombies`, the chunk's real
    /// zombies in square order) then `requestSaveCell(floor(wx / 32),
    /// floor(wy / 32))`. `MapCollisionData.removeChunkFromWorld` does
    /// nothing.
    pub fn remove_chunk_from_world<P: PopulationHost<Zombie = H>>(
        &mut self,
        host: &mut P,
        wx: i32,
        wy: i32,
        zombies: &[ChunkZombie<H>],
    ) {
        let pm = &mut self.manager;
        pm.remove_chunk_from_world(host, wx, wy, zombies);
        if !pm.client {
            pm.request_save_cell(wx.div_euclid(32), wy.div_euclid(32));
        }
    }

    /// `MapCollisionData.startGame()`: `updateMain`, the population's
    /// `updateMain`, `n_update`, `updateThread`, `updateMain` and the
    /// population's `updateMain` again, then the thread starts.
    /// `Err` is the exception creating a real zombie threw.
    pub fn start_game<P: PopulationHost<Zombie = H>>(
        &mut self,
        host: &mut P,
        frame: &PopulationFrame<'_, H>,
    ) -> Result<(), String> {
        if self.manager.client {
            return Ok(());
        }
        self.main_update(host, frame)?;
        let pm = &mut self.manager;
        pm.mcd.thread_update(&mut pm.native);
        pm.update_thread();
        self.main_update(host, frame)?;
        self.worker.started = true;
        self.manager.mcd.thread_alive = true;
        self.worker.next_pass_ms = self.now();
        Ok(())
    }

    /// `MapCollisionData.updateGameState()`: `paused` is
    /// `GameTime.isGamePaused()` or the current state is not `IngameState`.
    pub fn update_game_state(&mut self, no_save: bool, paused: bool) {
        let pm = &mut self.manager;
        pm.mcd.update_game_state(&mut pm.native, no_save, paused);
    }

    fn main_update<P: PopulationHost<Zombie = H>>(
        &mut self,
        host: &mut P,
        frame: &PopulationFrame<'_, H>,
    ) -> Result<(), String> {
        let now = self.now();
        let pm = &mut self.manager;
        if pm.mcd.update_main(now) {
            self.worker.notify();
        }
        let maps = frame.chunk_maps;
        if pm.update_main(
            host,
            frame.multiplier,
            frame.world_age_hours,
            &maps.origins,
            maps.width,
            frame.real_zombies,
        )? {
            self.worker.notify();
        }
        Ok(())
    }

    /// One frame of `IngameState.update`: `MapCollisionData.updateMain()`
    /// then `ZombiePopulationManager.updateMain()` (zombies the library
    /// hands over become real through `host`), then the worker thread's
    /// pass if it is due. `Err` is the exception creating a real zombie
    /// threw: the rest of `updateMain` is skipped, but the worker pass still
    /// runs, because in the original it is a separate thread.
    pub fn update<P: PopulationHost<Zombie = H>>(
        &mut self,
        host: &mut P,
        frame: &PopulationFrame<'_, H>,
    ) -> Result<(), String> {
        if self.manager.client {
            return Ok(());
        }
        let main = self.main_update(host, frame);
        let now = self.now();
        self.worker.run(&mut self.manager, now);
        main
    }

    /// `MapCollisionData.save()` (the game's periodic and quit saves):
    /// `updateRealZombies()` with `real_zombies`, then the collision grid's
    /// and the population's saves.
    pub fn save(&mut self, real_zombies: &[RealZombieInfo<H>]) {
        let pm = &mut self.manager;
        if pm.client {
            return;
        }
        pm.update_real_zombies(real_zombies);
        pm.native.mcd_save();
        pm.save();
    }

    /// `MapCollisionData.stop()` then `ZombiePopulationManager.stop()`.
    pub fn stop(&mut self) {
        let pm = &mut self.manager;
        pm.mcd.stop(&mut pm.native);
        pm.stop();
        self.worker.started = false;
    }
}
