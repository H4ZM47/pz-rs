//! `zombie.popman.ZombiePopulationManager` (single player and host side),
//! with `LoadedAreas`, `PlayerSpawns` and `RealZombieMap`.
//!
//! The manager owns the [`Native`] library state and the
//! [`MapCollisionData`] front end. What it needs from the live world (grid
//! squares, real `IsoZombie`s, the game's `Rand`) comes through
//! [`PopulationHost`]; the calls into it are made in the original's order,
//! so random draws made by the host (`createRealZombieAlways`,
//! `DoZombieStats`) interleave with the manager's own as in the game.
//!
//! The original runs `updateThread` on the `MapCollisionDataJNI` thread;
//! here [`ZombiePopulationManager::thread_step`] is one pass of that
//! thread's loop, to be called by whoever drives the world.

use crate::map_collision::MapCollisionData;
use crate::native::{Native, Zombie};
use crate::state_flags::{ZombieStateFlags, ZombieStateSource};
use pz_world::java::{coordmodulo, fastfloor, GameRand};
use pz_world::square::IsoDirections;
use pz_world::IsoMetaGrid;
use std::collections::{BTreeSet, HashMap, HashSet};

pub const SQUARES_PER_CHUNK: i32 = 8;
pub const CHUNKS_PER_CELL: i32 = 32;
pub const SQUARES_PER_CELL: i32 = 256;
pub const OLD_ZOMBIE_CRAWLER_CAN_WALK: u8 = 1;
pub const OLD_ZOMBIE_FAKE_DEAD: u8 = 2;
pub const OLD_ZOMBIE_CRAWLER: u8 = 3;
pub const OLD_ZOMBIE_WALKER: u8 = 4;
/// `INVALID_PATH_XY`.
pub const INVALID_PATH_XY: i32 = i32::MIN;
/// `INVALID_PERSISTENT_ID`.
pub const INVALID_PERSISTENT_ID: i32 = 0;
const UNREGISTERED_ZOMBIE_REPORT_THRESHOLD: i32 = 64;
const SAVE_CELL_INTERVAL_MS: i64 = 5000;
const REAL_ZOMBIE_UPDATE_INTERVAL_MS: i64 = 5000;
const PUSH_BEFORE_SAVE_INTERVAL_MS: i64 = 1000;
/// The direct buffer `updateMain` reads zombie records through.
const BYTE_BUFFER_CAPACITY: usize = 1024;

// ---- LoadedAreas -----------------------------------------------------------------

/// `LoadedAreas`: up to 64 chunk rectangles (`x, y, w, h` in chunks).
#[derive(Clone, Debug)]
pub struct LoadedAreas {
    pub areas: Vec<i32>,
    pub count: i32,
    pub changed: bool,
    pub prev_areas: Vec<i32>,
    pub prev_count: i32,
    pub server_cells: bool,
}

impl LoadedAreas {
    /// `MAX_AREAS`.
    pub const MAX_AREAS: i32 = 64;

    pub fn new(server_cells: bool) -> LoadedAreas {
        LoadedAreas { areas: vec![0; 256], count: 0, changed: false, prev_areas: vec![0; 256], prev_count: 0, server_cells }
    }

    /// `set()`: rebuilds from `areas` (the single player branch adds one
    /// `chunkGridWidth` square per non-ignored chunk map, from
    /// `getWorldXMin/YMin`); returns whether anything changed.
    pub fn set(&mut self, areas: impl IntoIterator<Item = (i32, i32, i32, i32)>) -> bool {
        self.set_prev();
        self.clear();
        for (x, y, w, h) in areas {
            self.add(x, y, w, h);
        }
        self.changed = self.compare_with_prev();
        self.changed
    }

    pub fn add(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if self.count >= Self::MAX_AREAS {
            return;
        }
        let n = (self.count * 4) as usize;
        self.areas[n] = x;
        self.areas[n + 1] = y;
        self.areas[n + 2] = width.clamp(1, 128);
        self.areas[n + 3] = height.clamp(1, 128);
        self.count += 1;
    }

    pub fn clear(&mut self) {
        self.count = 0;
        self.changed = false;
    }

    pub fn copy(&mut self, other: &LoadedAreas) {
        self.count = other.count;
        let n = (self.count * 4) as usize;
        self.areas[..n].copy_from_slice(&other.areas[..n]);
    }

    fn set_prev(&mut self) {
        self.prev_count = self.count;
        let n = (self.count * 4) as usize;
        self.prev_areas[..n].copy_from_slice(&self.areas[..n]);
    }

    fn compare_with_prev(&self) -> bool {
        if self.prev_count != self.count {
            return true;
        }
        let n = (self.count * 4) as usize;
        self.prev_areas[..n] != self.areas[..n]
    }

    /// `isOnEdge(x, y)` (square coordinates).
    pub fn is_on_edge(&self, x: i32, y: i32) -> bool {
        if coordmodulo(x, 8) != 0
            && coordmodulo(x + 1, 8) != 0
            && coordmodulo(y, 8) != 0
            && coordmodulo(y + 1, 8) != 0
        {
            return false;
        }
        for i in 0..self.count as usize {
            let a = &self.areas[i * 4..i * 4 + 4];
            let min_x = a[0] * 8;
            let min_y = a[1] * 8;
            let max_x = min_x + a[2] * 8;
            let max_y = min_y + a[3] * 8;
            let in_x = x >= min_x && x < max_x;
            let in_y = y >= min_y && y < max_y;
            if in_x && (y == min_y || y == max_y - 1) {
                return true;
            }
            if in_y && (x == min_x || x == max_x - 1) {
                return true;
            }
        }
        false
    }
}

// ---- PlayerSpawns ----------------------------------------------------------------

/// The building and room a player spawned in (`PlayerSpawn`).
#[derive(Clone, Debug)]
struct PlayerSpawn {
    counter: i64,
    /// `BuildingDef` id and bounds (`x, y, x2, y2`).
    building: (i64, i32, i32, i32, i32),
    room: i64,
}

/// What `PlayerSpawn.allowZombie` reads from a square.
#[derive(Clone, Copy, Debug, Default)]
pub struct SquarePlace {
    pub x: i32,
    pub y: i32,
    /// `getBuilding().getDef()` id.
    pub building: Option<i64>,
    /// `getRoom().getRoomDef()` id.
    pub room: Option<i64>,
}

/// `PlayerSpawns`: for ten seconds after a player spawns, zombies are not
/// made real in or around their building (`Lore.PlayerSpawnZombieRemoval`).
#[derive(Clone, Debug, Default)]
pub struct PlayerSpawns {
    spawns: Vec<PlayerSpawn>,
}

impl PlayerSpawns {
    /// `addSpawn(x, y, z)`: only spawns inside a room of a building count.
    pub fn add_spawn(&mut self, meta: &IsoMetaGrid, x: i32, y: i32, z: i32) {
        let Some(room) = meta.get_room_at(x, y, z) else { return };
        let Some(b) = room.building else { return };
        let bd = &meta.building_defs[b.0 as usize];
        self.spawns.push(PlayerSpawn { counter: -1, building: (bd.id, bd.x, bd.y, bd.x2, bd.y2), room: room.id });
    }

    /// `update()`.
    pub fn update(&mut self, now_ms: i64) {
        let mut i = 0;
        while i < self.spawns.len() {
            let ps = &mut self.spawns[i];
            if ps.counter == -1 {
                ps.counter = now_ms;
            }
            if ps.counter + 10000 > now_ms {
                i += 1;
                continue;
            }
            self.spawns.remove(i);
        }
    }

    /// `allowZombie(sq)`; `removal` is `Lore.PlayerSpawnZombieRemoval`.
    pub fn allow_zombie(&self, sq: &SquarePlace, removal: i32) -> bool {
        self.spawns.iter().all(|ps| ps.allow_zombie(sq, removal))
    }

    pub fn len(&self) -> usize {
        self.spawns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spawns.is_empty()
    }
}

impl PlayerSpawn {
    fn allow_zombie(&self, sq: &SquarePlace, removal: i32) -> bool {
        let (id, bx, by, bx2, by2) = self.building;
        match removal {
            1 => {
                if sq.building == Some(id) {
                    return false;
                }
                !(sq.x >= bx - 15 && sq.x < bx2 + 15 && sq.y >= by - 15 && sq.y < by2 + 15)
            }
            2 => sq.building != Some(id),
            3 => sq.room != Some(self.room),
            _ => true,
        }
    }
}

// ---- RealZombieMap ---------------------------------------------------------------

/// `RealZombieMap`: the real zombies by persistent id.
#[derive(Clone, Debug)]
pub struct RealZombieMap<H> {
    by_id: HashMap<i32, H>,
}

impl<H> Default for RealZombieMap<H> {
    fn default() -> Self {
        RealZombieMap { by_id: HashMap::new() }
    }
}

impl<H: Copy + Eq> RealZombieMap<H> {
    pub fn register(&mut self, zombie: H, persistent_id: i32) {
        if persistent_id == 0 {
            return;
        }
        self.by_id.insert(persistent_id, zombie);
    }

    /// `unregister`: only when `persistent_id` still maps to this zombie.
    pub fn unregister(&mut self, zombie: H, persistent_id: i32) -> bool {
        if self.by_id.get(&persistent_id) != Some(&zombie) {
            return false;
        }
        self.by_id.remove(&persistent_id);
        true
    }

    pub fn is_live(&self, persistent_id: i32) -> bool {
        persistent_id != 0 && self.by_id.contains_key(&persistent_id)
    }

    pub fn clear(&mut self) {
        self.by_id.clear();
    }
}

// ---- host --------------------------------------------------------------------------

/// A grid square as `addZombieStanding/Moving` and `updateMain` see it.
#[derive(Clone, Copy, Debug, Default)]
pub struct SquareInfo {
    /// `solidFloorCached ? solidFloor : TreatAsSolidFloor()`.
    pub solid_floor: bool,
    /// `roomId` (-1 outside).
    pub room_id: i64,
    pub place: SquarePlace,
}

/// A real zombie as `packRealZombies` / `removeChunkFromWorld` read it.
#[derive(Clone, Copy, Debug)]
pub struct RealZombieInfo<H> {
    pub handle: H,
    pub persistent_id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getForwardIsoDirection()`.
    pub dir: IsoDirections,
    /// `getPersistentOutfitID()`.
    pub outfit: i32,
    pub state: ZombieStateSource,
    pub path_target_x: i32,
    pub path_target_y: i32,
    pub dead: bool,
    pub reanimated_player: bool,
    /// The current state is `WalkTowardState` or `PathFindState`.
    pub walking: bool,
}

/// One real zombie on a chunk being unloaded (`removeChunkFromWorld`).
#[derive(Clone, Copy, Debug)]
pub struct ChunkZombie<H> {
    pub info: RealZombieInfo<H>,
    /// The square's level (`z` of the loop).
    pub level: i32,
    /// `sq.getRoom() != null`.
    pub in_room: bool,
}

/// What the population manager needs from the live world.
pub trait PopulationHost {
    /// A real zombie (`IsoZombie`).
    type Zombie: Copy + Eq;

    /// `IsoWorld.instance.currentCell.getGridSquare(x, y, z)`.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo>;
    /// `getSquareForSittingZombie`'s test of one square: `isFree(true)`,
    /// no building, a non-zero `getWallType()`, and no
    /// `PolygonalMap2.lineClearCollide(fromX, fromY, sq.x + 0.5, sq.y + 0.5,
    /// sq.z, null, false, true)`. Returns the wall type when it passes.
    fn sitting_wall_type(&self, x: i32, y: i32, z: i32, from_x: f32, from_y: f32) -> Option<i32>;
    /// The game's `Rand`.
    fn rand(&mut self) -> &mut dyn GameRand;
    /// `VirtualZombieManager.createRealZombieAlways(outfit, dir, false,
    /// persistentId)` with `choices = [square]`.
    fn create_real_zombie(&mut self, outfit: i32, dir: IsoDirections, persistent_id: i32, square: (i32, i32, i32)) -> Option<Self::Zombie>;
    /// The zombie as `registerZombie` reads it at the end of
    /// `createRealZombieAlways` (its position in the chosen square, its
    /// fresh state flags and `getPersistentOutfitID()`).
    fn real_zombie_info(&self, z: Self::Zombie) -> Option<RealZombieInfo<Self::Zombie>>;
    /// `zombie.persistentId = id`.
    fn set_persistent_id(&mut self, z: Self::Zombie, id: i32);
    /// `setX(x); setY(y)`.
    fn set_position(&mut self, z: Self::Zombie, x: f32, y: f32);
    /// `sitAgainstWall`'s effect: position, `setSitAgainstWall(true)`,
    /// `setDir`, `setForwardDirection` and the animation player's direction.
    /// `dir` is `None` when the square has no wall type.
    fn sit_against_wall(&mut self, z: Self::Zombie, x: f32, y: f32, dir: Option<IsoDirections>);
    /// `setHealth(health); sprite = legsSprite; setFakeDead(true)`.
    fn make_fake_dead(&mut self, z: Self::Zombie, health: f32);
    /// `setCrawler(true); setCanWalk(canWalk); setOnFloor(true);
    /// setFallOnFront(true); walkVariant = "ZombieWalk"; DoZombieStats()`.
    fn make_crawler(&mut self, z: Self::Zombie, can_walk: bool);
    fn set_can_crawl_under_vehicle(&mut self, z: Self::Zombie, v: bool);
    fn set_reanimated_for_grapple_only(&mut self, z: Self::Zombie, v: bool);
    /// `allowRepathDelay = -1; pathToLocation(x, y, 0)`.
    fn path_to(&mut self, z: Self::Zombie, x: i32, y: i32);
    /// `die()` (a dead zombie found on an unloading chunk).
    fn die(&mut self, z: Self::Zombie);
    /// `removeFromWorld(); removeFromSquare()`.
    fn remove_zombie(&mut self, z: Self::Zombie);
}

/// `SandboxOptions.ZombieConfig` values `onConfigReloaded` sends.
#[derive(Clone, Copy, Debug)]
pub struct ZombieConfig {
    pub population_multiplier: f64,
    pub population_start_multiplier: f64,
    pub population_peak_multiplier: f64,
    pub population_peak_day: i32,
    pub respawn_hours: f64,
    pub respawn_unseen_hours: f64,
    pub respawn_multiplier: f64,
    pub redistribute_hours: f64,
    pub follow_sound_distance: i32,
}

impl Default for ZombieConfig {
    /// The sandbox defaults.
    fn default() -> Self {
        ZombieConfig {
            population_multiplier: 0.65,
            population_start_multiplier: 1.0,
            population_peak_multiplier: 1.5,
            population_peak_day: 28,
            respawn_hours: 72.0,
            respawn_unseen_hours: 16.0,
            respawn_multiplier: 0.1,
            redistribute_hours: 12.0,
            follow_sound_distance: 100,
        }
    }
}

/// `ChooseGameInfo.SpawnOrigin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnOrigin {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// `ZombiePopulationManager`.
pub struct ZombiePopulationManager<H> {
    pub native: Native,
    pub mcd: MapCollisionData,
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
    pub stopped: bool,
    /// `GameClient.client`.
    pub client: bool,
    /// `GameServer.server`.
    pub server: bool,
    /// `Core.bLastStand`.
    pub last_stand: bool,
    /// `Lore.PlayerSpawnZombieRemoval`.
    pub player_spawn_zombie_removal: i32,
    pub debug_logging_enabled: bool,
    pending_save_cells: BTreeSet<i32>,
    last_save_cell_ms: HashMap<i32, i64>,
    real_zombies: RealZombieMap<H>,
    pub loaded_areas: LoadedAreas,
    pub loaded_server_cells: LoadedAreas,
    pub player_spawns: PlayerSpawns,
    real_zombie_update_ms: i64,
    last_reported_unregistered: i32,
    new_chunks: HashSet<i32>,
    spawn_origins: Vec<SpawnOrigin>,
    pub zombies_min_per_chunk: f32,
    pub zombies_max_per_chunk: f32,
    pub radar_xy: Option<Vec<f32>>,
    pub radar_count: i32,
    pub radar_render_flag: bool,
    pub radar_request_flag: bool,
    /// `MCDThread.save`.
    pub thread_save_requested: bool,
    /// `noise(...)` lines (only while `debug_logging_enabled`).
    pub log: Vec<String>,
}

impl<H: Copy + Eq> ZombiePopulationManager<H> {
    pub fn new(native: Native) -> Self {
        ZombiePopulationManager {
            native,
            mcd: MapCollisionData::default(),
            min_x: 0,
            min_y: 0,
            width: 0,
            height: 0,
            stopped: false,
            client: false,
            server: false,
            last_stand: false,
            player_spawn_zombie_removal: 1,
            debug_logging_enabled: false,
            pending_save_cells: BTreeSet::new(),
            last_save_cell_ms: HashMap::new(),
            real_zombies: RealZombieMap::default(),
            loaded_areas: LoadedAreas::new(false),
            loaded_server_cells: LoadedAreas::new(true),
            player_spawns: PlayerSpawns::default(),
            real_zombie_update_ms: 0,
            last_reported_unregistered: 0,
            new_chunks: HashSet::new(),
            spawn_origins: Vec::new(),
            zombies_min_per_chunk: 0.0,
            zombies_max_per_chunk: 255.0,
            radar_xy: None,
            radar_count: 0,
            radar_render_flag: false,
            radar_request_flag: false,
            thread_save_requested: false,
            log: Vec::new(),
        }
    }

    fn now(&self) -> i64 {
        self.native.gs.now()
    }

    fn noise(&mut self, s: String) {
        if self.debug_logging_enabled {
            self.log.push(format!("ZPOP: {s}"));
        }
    }

    /// `registerZombie(zombie, persistentId)`: returns the id the library
    /// assigned (0 on a client).
    pub fn register_zombie(&mut self, z: &RealZombieInfo<H>, persistent_id: i32) -> i32 {
        if self.client {
            return 0;
        }
        let state = ZombieStateFlags::int_from_zombie(&z.state);
        let id = self.native.n_register_zombie(z.x, z.y, z.z, z.dir as u8, z.outfit, state, persistent_id as u32) as i32;
        self.real_zombies.register(z.handle, id);
        id
    }

    /// The end of `createRealZombieAlways`: `zombie.persistentId =
    /// registerZombie(zombie, persistentId)`.
    fn register_created<P: PopulationHost<Zombie = H>>(&mut self, host: &mut P, z: H, persistent_id: i32) {
        if let Some(info) = host.real_zombie_info(z) {
            let id = self.register_zombie(&info, persistent_id);
            host.set_persistent_id(z, id);
        }
    }

    /// `releaseZombie(zombie)`.
    pub fn release_zombie(&mut self, zombie: H, persistent_id: i32) {
        if self.client || persistent_id == 0 {
            return;
        }
        if !self.real_zombies.unregister(zombie, persistent_id) {
            return;
        }
        self.native.n_release_zombie(persistent_id as u32);
    }

    /// `requestSaveCell`.
    pub fn request_save_cell(&mut self, cx: i32, cy: i32) {
        if self.client || self.stopped {
            return;
        }
        self.pending_save_cells.insert(pack_cell(cx, cy));
    }

    /// `processPendingSaveCells` (on the collision thread).
    pub fn process_pending_save_cells(&mut self) {
        if self.client || self.pending_save_cells.is_empty() {
            return;
        }
        let now = self.now();
        let keys: Vec<i32> = self.pending_save_cells.iter().copied().collect();
        for key in keys {
            if now - self.last_save_cell_ms.get(&key).copied().unwrap_or(0) < SAVE_CELL_INTERVAL_MS {
                continue;
            }
            self.pending_save_cells.remove(&key);
            self.last_save_cell_ms.insert(key, now);
            self.native.n_save_cell(key >> 16, (key & 0xFFFF) as i16 as i32);
        }
    }

    /// `init(metaGrid)`: `bounds` is the meta grid's `getMinX/MinY/Width/
    /// Height` (in cells) and `outfit_names` is
    /// `PersistentOutfits.getOutfitNames()`.
    pub fn init(&mut self, bounds: (i32, i32, i32, i32), config: &ZombieConfig, outfit_names: &[String]) {
        if self.client {
            return;
        }
        (self.min_x, self.min_y, self.width, self.height) = bounds;
        self.stopped = false;
        self.native.n_init(self.client, self.server, self.min_x, self.min_y, self.width, self.height);
        self.on_config_reloaded(config);
        let names: Vec<String> = outfit_names.iter().map(|s| s.to_lowercase()).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        self.native.n_set_outfit_names(&refs);
        let origins: Vec<i32> = self.spawn_origins.iter().flat_map(|o| [o.x, o.y, o.w, o.h]).collect();
        let _ = self.native.n_set_spawn_origins(&origins);
    }

    /// `onConfigReloaded`.
    pub fn on_config_reloaded(&mut self, cfg: &ZombieConfig) {
        let n = &mut self.native;
        let _ = n.n_config_float("PopulationMultiplier", cfg.population_multiplier as f32);
        let _ = n.n_config_float("PopulationStartMultiplier", cfg.population_start_multiplier as f32);
        let _ = n.n_config_float("PopulationPeakMultiplier", cfg.population_peak_multiplier as f32);
        let _ = n.n_config_int("PopulationPeakDay", cfg.population_peak_day);
        let _ = n.n_config_float("RespawnHours", cfg.respawn_hours as f32);
        let _ = n.n_config_float("RespawnUnseenHours", cfg.respawn_unseen_hours as f32);
        let _ = n.n_config_float("RespawnMultiplier", cfg.respawn_multiplier as f32);
        let _ = n.n_config_float("RedistributeHours", cfg.redistribute_hours as f32);
        let _ = n.n_config_int("FollowSoundDistance", cfg.follow_sound_distance);
        let _ = n.n_config_float("MinZombiesPerChunk", self.zombies_min_per_chunk);
        let _ = n.n_config_float("MaxZombiesPerChunk", self.zombies_max_per_chunk);
        let _ = n.n_config_float("UniformZombiesPerChunk", 0.2);
    }

    /// `registerSpawnOrigin` (negative values are ignored).
    pub fn register_spawn_origin(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if x < 0 || y < 0 || w < 0 || h < 0 {
            return;
        }
        self.spawn_origins.push(SpawnOrigin { x, y, w, h });
    }

    /// `playerSpawnedAt`.
    pub fn player_spawned_at(&mut self, meta: &IsoMetaGrid, x: i32, y: i32, z: i32) {
        self.player_spawns.add_spawn(meta, x, y, z);
    }

    pub fn set_zombies_min_per_chunk(&mut self, f: f32) {
        self.zombies_min_per_chunk = f;
    }

    pub fn set_zombies_max_per_chunk(&mut self, f: f32) {
        self.zombies_max_per_chunk = f;
    }

    /// `addChunkToWorld(chunk)`.
    pub fn add_chunk_to_world(&mut self, wx: i32, wy: i32, new_chunk: bool) {
        if self.client {
            return;
        }
        if new_chunk {
            self.new_chunks.insert(wy << 16 | wx);
        }
        self.native.n_load_chunk(wx, wy, true);
    }

    /// `removeChunkFromWorld(chunk)`: `zombies` are the chunk's real
    /// zombies in square order (level, then y, then x).
    pub fn remove_chunk_from_world<P: PopulationHost<Zombie = H>>(&mut self, host: &mut P, wx: i32, wy: i32, zombies: &[ChunkZombie<H>]) {
        if self.client || self.stopped {
            return;
        }
        self.native.n_load_chunk(wx, wy, false);
        for cz in zombies {
            let z = &cz.info;
            if z.reanimated_player {
                continue;
            }
            if z.dead {
                host.die(z.handle);
                continue;
            }
            let state = ZombieStateFlags::int_from_zombie(&z.state);
            let (px, py) = if cz.level == 0 && !cz.in_room && z.walking {
                (z.path_target_x, z.path_target_y)
            } else {
                (INVALID_PATH_XY, INVALID_PATH_XY)
            };
            self.native.n_add_zombie(z.x, z.y, z.z, z.dir as u8, z.outfit, state, px, py, z.persistent_id as u32);
            host.remove_zombie(z.handle);
        }
        self.new_chunks.remove(&(wy << 16 | wx));
    }

    /// `virtualizeZombie(realZombie)`.
    pub fn virtualize_zombie<P: PopulationHost<Zombie = H>>(&mut self, host: &mut P, z: &RealZombieInfo<H>) {
        let state = ZombieStateFlags::int_from_zombie(&z.state);
        self.native.n_add_zombie(
            z.x,
            z.y,
            z.z,
            z.dir as u8,
            z.outfit,
            state,
            z.path_target_x,
            z.path_target_y,
            z.persistent_id as u32,
        );
        host.remove_zombie(z.handle);
    }

    pub fn set_aggro_target(&mut self, id: i32, x: i32, y: i32) {
        self.native.n_aggro_target(id, x, y);
    }

    pub fn create_horde_from_to(&mut self, sx: i32, sy: i32, tx: i32, ty: i32, count: i32) {
        self.native.n_spawn_horde(sx, sy, 0, 0, tx as f32, ty as f32, count);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_horde_in_area_to(&mut self, sx: i32, sy: i32, sw: i32, sh: i32, tx: i32, ty: i32, count: i32) {
        self.native.n_spawn_horde(sx, sy, sw, sh, tx as f32, ty as f32, count);
    }

    pub fn ready_to_pause(&mut self) -> bool {
        self.native.n_ready_to_pause()
    }

    /// `addWorldSound(sound, doSend)`: `hearing_multiplier` is
    /// `WorldSoundManager.getHearingMultiplier(hearing)` for
    /// `Lore.Hearing` (4 and 5 count as 2).
    #[allow(clippy::too_many_arguments)]
    pub fn add_world_sound(
        &mut self,
        x: i32,
        y: i32,
        radius: i32,
        volume: i32,
        source_is_zombie: bool,
        stress_zombies: bool,
        hearing_multiplier: f32,
    ) {
        if self.client || radius < 50 || source_is_zombie || !stress_zombies {
            return;
        }
        let r = (radius as f32 * hearing_multiplier).ceil() as i32;
        self.native.n_world_sound(x, y, r, volume);
    }

    /// `packRealZombies` + `n_updateRealZombies`.
    pub fn update_real_zombies(&mut self, zombies: &[RealZombieInfo<H>]) {
        if self.client {
            return;
        }
        let mut ids = Vec::with_capacity(zombies.len() * 2);
        let mut pos = Vec::with_capacity(zombies.len() * 3);
        let mut unregistered = 0;
        for z in zombies {
            if z.persistent_id == 0 {
                if !z.reanimated_player && !z.dead {
                    unregistered += 1;
                }
                continue;
            }
            ids.push(z.persistent_id);
            ids.push(ZombieStateFlags::int_from_zombie(&z.state));
            pos.extend_from_slice(&[z.x, z.y, z.z]);
        }
        if unregistered > UNREGISTERED_ZOMBIE_REPORT_THRESHOLD && unregistered > self.last_reported_unregistered {
            self.last_reported_unregistered = unregistered;
            self.log.push(format!("ZPOP: {unregistered} zombies have no persistent id and will not be saved"));
        }
        let count = (ids.len() / 2) as i32;
        let _ = self.native.n_update_real_zombies(count, &ids, &pos);
    }

    /// `updateMain()`. `chunk_maps` are the non-ignored players' chunk map
    /// origins (`getWorldXMin/YMin`) and `chunk_grid_width` is
    /// `IsoChunkMap.chunkGridWidth`; `real_zombies` is the cell's zombie
    /// list, packed when due. Returns whether it notified the worker
    /// thread (`MapCollisionData.notifyThread()`: the library or the
    /// collision grid has data for it).
    pub fn update_main<P: PopulationHost<Zombie = H>>(
        &mut self,
        host: &mut P,
        multiplier: f32,
        world_age_hours: f64,
        chunk_maps: &[(i32, i32)],
        chunk_grid_width: i32,
        real_zombies: &[RealZombieInfo<H>],
    ) -> bool {
        if self.client {
            return false;
        }
        let now = self.now();
        self.native.n_update_main(multiplier, world_age_hours);
        let total = self.native.n_get_add_zombie_count();
        let mut offset = 0;
        let (mut standing, mut moving) = (0, 0);
        while offset < total {
            let mut recs: Vec<Zombie> = Vec::new();
            let count = self.native.n_get_add_zombie_data(offset, BYTE_BUFFER_CAPACITY, &mut recs).unwrap_or(0);
            offset += count;
            for r in recs {
                let (fx, fy, fz) = (fastfloor(r.x), fastfloor(r.y), fastfloor(r.z));
                let key = (fy / 8) << 16 | (fx / 8);
                if self.new_chunks.contains(&key) && host.square(fx, fy, fz).is_some_and(|sq| sq.room_id != -1) {
                    continue;
                }
                let dir = IsoDirections::ALL.get(r.dir as usize).copied().unwrap_or(IsoDirections::N);
                let state = ZombieStateFlags::from_int(r.state);
                let mut path = (r.path_x, r.path_y);
                if path.0 != INVALID_PATH_XY && self.loaded_areas.is_on_edge(fx, fy) {
                    path = (INVALID_PATH_XY, INVALID_PATH_XY);
                }
                if path.0 == INVALID_PATH_XY {
                    self.add_zombie_standing(host, &r, dir, state);
                    standing += 1;
                } else {
                    self.add_zombie_moving(host, &r, dir, state, path);
                    moving += 1;
                }
            }
        }
        if standing > 0 {
            self.noise(format!("unloaded -> real {total}"));
        }
        if moving > 0 {
            self.noise(format!("virtual -> real {total}"));
        }
        if let Some(xy) = self.radar_xy.as_mut().filter(|_| self.radar_render_flag) {
            if self.radar_request_flag {
                if self.native.n_has_radar_data() {
                    self.radar_count = self.native.n_get_radar_zombie_data(xy).unwrap_or(0);
                    self.radar_render_flag = false;
                    self.radar_request_flag = false;
                }
            } else {
                self.native.n_request_radar_data();
                self.radar_request_flag = true;
            }
        }
        self.update_loaded_areas(chunk_maps, chunk_grid_width);
        let push = if self.pending_save_cells.is_empty() { REAL_ZOMBIE_UPDATE_INTERVAL_MS } else { PUSH_BEFORE_SAVE_INTERVAL_MS };
        if self.real_zombie_update_ms + push < now {
            self.real_zombie_update_ms = now;
            self.update_real_zombies(real_zombies);
        }
        let d1 = self.native.n_has_data_for_thread();
        let d2 = self.mcd.has_data_for_thread(&mut self.native);
        self.player_spawns.update(now);
        d1 || d2
    }

    fn is_stale_record(&mut self, persistent_id: i32) -> bool {
        if !self.real_zombies.is_live(persistent_id) {
            return false;
        }
        self.log.push(format!("ZPOP: dropped a stale record for zombie {persistent_id}, it is already alive"));
        true
    }

    /// Shared head of `addZombieStanding/Moving`: the square, or `None`
    /// after handing the record back to the library (no floor) or dropping
    /// it (stale, or near a fresh player spawn).
    fn real_square<P: PopulationHost<Zombie = H>>(&mut self, host: &mut P, r: &Zombie, path: (i32, i32), what: &str) -> Option<SquareInfo> {
        if self.is_stale_record(r.id as i32) {
            return None;
        }
        let (fx, fy, fz) = (fastfloor(r.x), fastfloor(r.y), fastfloor(r.z));
        let sq = host.square(fx, fy, fz);
        let Some(sq) = sq.filter(|s| s.solid_floor) else {
            self.noise(what.to_string());
            self.native.n_add_zombie(r.x, r.y, r.z, r.dir, r.outfit, r.state, path.0, path.1, r.id);
            return None;
        };
        if !self.last_stand && !self.player_spawns.allow_zombie(&sq.place, self.player_spawn_zombie_removal) {
            self.noise(format!("removed zombie near player spawn {fx},{fy},{fz}"));
            return None;
        }
        Some(sq)
    }

    fn add_zombie_standing<P: PopulationHost<Zombie = H>>(&mut self, host: &mut P, r: &Zombie, dir: IsoDirections, state: ZombieStateFlags) {
        let Some(sq) = self.real_square(host, r, (INVALID_PATH_XY, INVALID_PATH_XY), "real -> unloaded") else {
            return;
        };
        let fz = fastfloor(r.z);
        let mut wall = None;
        if !state.is_crawling() && !state.is_fake_dead() && host.rand().next(3) == 0 {
            wall = Self::square_for_sitting_zombie(host, r.x, r.y, fz);
        }
        let square = match wall {
            Some((wx, wy, _)) => (wx, wy, fz),
            None => (sq.place.x, sq.place.y, fz),
        };
        let Some(z) = host.create_real_zombie(r.outfit, dir, r.id as i32, square) else {
            return;
        };
        self.register_created(host, z, r.id as i32);
        match wall {
            Some((wx, wy, wall_type)) => {
                let d = Self::sitting_direction(host, wall_type, self.client, wx, wy);
                host.sit_against_wall(z, wx as f32 + 0.5, wy as f32 + 0.5, d);
            }
            None => host.set_position(z, r.x, r.y),
        }
        if state.is_fake_dead() {
            let h = 0.5 + host.rand().next_float_range(0.0, 0.3);
            host.make_fake_dead(z, h);
        } else if state.is_crawling() {
            host.make_crawler(z, state.is_can_walk());
        }
        if state.is_initialized() {
            host.set_can_crawl_under_vehicle(z, state.is_can_crawl_under_vehicle());
        }
        host.set_reanimated_for_grapple_only(z, state.is_reanimated_for_grapple_only());
    }

    /// `getSquareForSittingZombie(x, y, z)`: `(x, y, wallType)`.
    fn square_for_sitting_zombie<P: PopulationHost<Zombie = H>>(host: &P, x: f32, y: f32, z: i32) -> Option<(i32, i32, i32)> {
        for dx in -3..3 {
            for dy in -3..3 {
                let (sx, sy) = (fastfloor(x) + dx, fastfloor(y) + dy);
                if let Some(w) = host.sitting_wall_type(sx, sy, z, x, y) {
                    return Some((sx, sy, w));
                }
            }
        }
        None
    }

    /// The direction `sitAgainstWall` picks for a wall type.
    fn sitting_direction<P: PopulationHost<Zombie = H>>(host: &mut P, wall_type: i32, client: bool, x: i32, y: i32) -> Option<IsoDirections> {
        use IsoDirections::*;
        if wall_type == 0 {
            return None;
        }
        let w = wall_type;
        let mut dirs = Vec::new();
        if w & 1 != 0 && w & 4 != 0 {
            dirs.push(SE);
        }
        if w & 1 != 0 && w & 8 != 0 {
            dirs.push(SW);
        }
        if w & 2 != 0 && w & 4 != 0 {
            dirs.push(NE);
        }
        if w & 2 != 0 && w & 8 != 0 {
            dirs.push(NW);
        }
        if w & 1 != 0 {
            dirs.push(S);
        }
        if w & 2 != 0 {
            dirs.push(N);
        }
        if w & 4 != 0 {
            dirs.push(E);
        }
        if w & 8 != 0 {
            dirs.push(W);
        }
        if dirs.is_empty() {
            // `pickRandom` of an empty list is null; `setDir(null)` follows.
            return None;
        }
        let mut d = dirs[host.rand().next(dirs.len() as i32) as usize];
        if client {
            d = dirs[(((x & 1) + (y & 1)) as usize) % dirs.len()];
        }
        Some(d)
    }

    fn add_zombie_moving<P: PopulationHost<Zombie = H>>(
        &mut self,
        host: &mut P,
        r: &Zombie,
        dir: IsoDirections,
        state: ZombieStateFlags,
        path: (i32, i32),
    ) {
        let what = format!("real -> virtual {},{}", r.x, r.y);
        let Some(sq) = self.real_square(host, r, path, &what) else {
            return;
        };
        let square = (sq.place.x, sq.place.y, fastfloor(r.z));
        let Some(z) = host.create_real_zombie(r.outfit, dir, r.id as i32, square) else {
            return;
        };
        self.register_created(host, z, r.id as i32);
        host.set_position(z, r.x, r.y);
        if state.is_crawling() {
            host.make_crawler(z, state.is_can_walk());
        }
        if state.is_initialized() {
            host.set_can_crawl_under_vehicle(z, state.is_can_crawl_under_vehicle());
        }
        if (path.0 as f32 - r.x).abs() > 1.0 || (path.1 as f32 - r.y).abs() > 1.0 {
            host.path_to(z, path.0, path.1);
        }
        host.set_reanimated_for_grapple_only(z, state.is_reanimated_for_grapple_only());
    }

    /// `updateLoadedAreas()` (single player areas).
    pub fn update_loaded_areas(&mut self, chunk_maps: &[(i32, i32)], chunk_grid_width: i32) {
        let areas = chunk_maps.iter().map(|&(x, y)| (x, y, chunk_grid_width, chunk_grid_width));
        if self.loaded_areas.set(areas) {
            let n = (self.loaded_areas.count * 4) as usize;
            let _ = self.native.n_loaded_areas(self.loaded_areas.count, &self.loaded_areas.areas[..n], false);
        }
    }

    /// `updateThread()`.
    pub fn update_thread(&mut self) {
        self.native.n_update_thread();
    }

    /// `shouldWait()`.
    pub fn should_wait(&mut self) -> bool {
        self.native.n_should_wait()
    }

    /// One pass of `MCDThread.runInner` (single player queues no path
    /// tasks from Java): a requested save, the collision update, the
    /// population's worker update and the due cell saves. Returns whether
    /// the thread would now wait.
    pub fn thread_step(&mut self) -> bool {
        if self.thread_save_requested {
            self.native.mcd_save();
            self.save();
            self.thread_save_requested = false;
        }
        self.mcd.thread_update(&mut self.native);
        self.update_thread();
        self.process_pending_save_cells();
        self.native.mcd_should_wait() && self.should_wait()
    }

    /// `dbgSpawnTimeToZero` (single player).
    pub fn dbg_spawn_time_to_zero(&mut self, cx: i32, cy: i32) {
        self.native.n_debug_command(3, cx, cy);
    }

    /// `dbgClearZombies` (single player).
    pub fn dbg_clear_zombies(&mut self, cx: i32, cy: i32) {
        self.native.n_debug_command(4, cx, cy);
    }

    /// `dbgSpawnNow` (single player).
    pub fn dbg_spawn_now(&mut self, cx: i32, cy: i32) {
        self.native.n_debug_command(5, cx, cy);
    }

    /// `save()`.
    pub fn save(&mut self) {
        if self.client {
            return;
        }
        self.native.n_save();
    }

    /// `stop()`.
    pub fn stop(&mut self) {
        if self.client {
            return;
        }
        self.stopped = true;
        self.native.n_stop();
        self.real_zombies.clear();
        self.pending_save_cells.clear();
        self.last_save_cell_ms.clear();
        self.last_reported_unregistered = 0;
        self.loaded_areas.clear();
        self.new_chunks.clear();
        self.spawn_origins.clear();
        self.radar_xy = None;
        self.radar_count = 0;
        self.radar_render_flag = false;
        self.radar_request_flag = false;
        self.zombies_min_per_chunk = 0.0;
        self.zombies_max_per_chunk = 255.0;
    }

    pub fn real_zombies(&self) -> &RealZombieMap<H> {
        &self.real_zombies
    }

    pub fn spawn_origins(&self) -> &[SpawnOrigin] {
        &self.spawn_origins
    }
}

/// `packCell`.
pub fn pack_cell(cx: i32, cy: i32) -> i32 {
    cx << 16 | (cy & 0xFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_areas_edges() {
        let mut a = LoadedAreas::new(false);
        assert!(a.set([(10, 20, 3, 300)]));
        assert_eq!(&a.areas[..4], &[10, 20, 3, 128]);
        assert!(!a.set([(10, 20, 3, 300)]));
        assert!(a.is_on_edge(80, 170));
        assert!(a.is_on_edge(103, 170));
        assert!(!a.is_on_edge(104, 170));
        assert!(a.is_on_edge(90, 160));
        assert!(!a.is_on_edge(90, 170));
        assert!(!a.is_on_edge(85, 165));
    }

    #[test]
    fn real_zombie_map() {
        let mut m = RealZombieMap::default();
        m.register(1u32, 5);
        m.register(2u32, 0);
        assert!(m.is_live(5) && !m.is_live(0));
        assert!(!m.unregister(2, 5));
        assert!(m.unregister(1, 5));
        assert!(!m.is_live(5));
    }

    #[test]
    fn player_spawn_rules() {
        let ps = PlayerSpawn { counter: -1, building: (7, 100, 100, 110, 110), room: 3 };
        let inside = SquarePlace { x: 105, y: 105, building: Some(7), room: Some(3) };
        let near = SquarePlace { x: 120, y: 105, building: None, room: None };
        let far = SquarePlace { x: 125, y: 105, building: None, room: None };
        assert!(!ps.allow_zombie(&inside, 1));
        assert!(!ps.allow_zombie(&near, 1));
        assert!(ps.allow_zombie(&far, 1));
        assert!(ps.allow_zombie(&near, 2) && !ps.allow_zombie(&inside, 2));
        assert!(!ps.allow_zombie(&inside, 3) && ps.allow_zombie(&inside, 4));
        let mut s = PlayerSpawns { spawns: vec![ps] };
        s.update(1000);
        s.update(10999);
        assert_eq!(s.len(), 1);
        s.update(11000);
        assert!(s.is_empty());
    }

    #[test]
    fn pack_cell_round_trip() {
        let k = pack_cell(41, -3);
        assert_eq!(k >> 16, 41);
        assert_eq!((k & 0xFFFF) as i16 as i32, -3);
    }
}
