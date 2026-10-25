//! `popman::*`: the population simulation (cells, chunks, zombie records,
//! virtual zombies, repopulation, redistribution, hordes, world sounds) and
//! its `zpop_*.bin` files, plus the JNI entry points of both halves.

use super::flood_fill::FloodFill;
use super::mcd::{type_of, Mcd, NativeMetaGrid, BLOCK_MASK};
use super::rand::NativeRand;
use super::stream::{DataInput, DataOutput};
use super::wall_follower::{self, WallFollower};
use super::{ffloor, GameState};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io;
use std::path::PathBuf;

/// `ZombiePopulationManager.INVALID_PATH_XY`.
pub const NO_PATH: i32 = i32::MIN;

/// Index into [`Native::zombies`].
pub type ZId = u32;
/// Index into [`Native::vzs`].
pub type VzId = u32;
type CellIdx = usize;
type PtwId = usize;
type PtmId = usize;
type SoundId = u32;

/// `popman::Zombie`: one stored zombie.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Zombie {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `IsoDirections` ordinal.
    pub dir: u8,
    /// Persistent outfit id (outfit index in bits 16..30).
    pub outfit: i32,
    /// `ZombieStateFlags`.
    pub state: i32,
    pub path_x: i32,
    pub path_y: i32,
    pub id: u32,
}

/// What `n_getAddZombieData` hands to Java for one zombie.
pub type ZombieRecord = Zombie;

impl Zombie {
    /// `Zombie::save`.
    fn save(&self, out: &mut DataOutput) {
        out.write_float(self.x);
        out.write_float(self.y);
        out.write_byte(self.z as i32 as i8 as i32);
        out.write_byte(self.dir as i8 as i32);
        out.write_int(self.state);
        out.write_int(self.outfit);
        out.write_int(self.id as i32);
    }
}

/// `popman::Chunk`.
#[derive(Clone, Default)]
pub struct Chunk {
    pub wx: i16,
    pub wy: i16,
    /// `calcDesiredPopulation` result.
    pub desired: i16,
    pub zombies: Vec<ZId>,
    /// World hours when a player last had the chunk loaded.
    pub last_seen: f32,
    /// World hours of the last repopulation.
    pub last_repop: f32,
}

/// `popman::Cell` (one 256x256 map cell).
#[derive(Clone)]
pub struct Cell {
    pub x: i16,
    pub y: i16,
    pub chunks: Vec<Chunk>,
    /// Zombies stored in the chunks (kept incrementally).
    pub stored: i16,
    /// Sum of the chunks' desired population.
    pub desired: i16,
    /// Virtual zombies currently in the cell.
    pub vzs: Vec<VzId>,
    pub last_repop: f32,
    pub pending_tasks: i32,
    pub repop_target: i32,
    pub repop_per: i32,
    pub repop_done: i32,
    pub loaded: bool,
    pub from_file: bool,
    pub dirty: bool,
    pub last_seen_ms: i64,
    /// Zombies to write into the next save that live elsewhere (in flight).
    pub save_pending: Vec<ZId>,
    pub chunk_loaded: [u32; 32],
    pub last_redistribute: f32,
}

impl Cell {
    fn new(x: i32, y: i32) -> Cell {
        Cell {
            x: x as i16,
            y: y as i16,
            chunks: Vec::new(),
            stored: 0,
            desired: 0,
            vzs: Vec::new(),
            last_repop: 0.0,
            pending_tasks: 0,
            repop_target: -1,
            repop_per: 0,
            repop_done: 0,
            loaded: false,
            from_file: false,
            dirty: false,
            last_seen_ms: 0,
            save_pending: Vec::new(),
            chunk_loaded: [0; 32],
            last_redistribute: 0.0,
        }
    }

    /// `getChunkFromWorldPos`.
    pub fn chunk_index_world(&self, x: i32, y: i32) -> Option<usize> {
        let ux = x.wrapping_sub((self.x as i32).wrapping_mul(256)) as u32;
        let uy = y.wrapping_sub((self.y as i32).wrapping_mul(256)) as u32;
        if (ux | uy) < 256 && self.loaded {
            Some(((uy * 4) & 0x3e0 | ux >> 3) as usize)
        } else {
            None
        }
    }

    /// `getChunkFromChunkPos`.
    pub fn chunk_index_chunk(&self, wx: i32, wy: i32) -> Option<usize> {
        if !self.loaded {
            return None;
        }
        let ux = wx.wrapping_sub((self.x as i32).wrapping_mul(32)) as u32;
        let uy = wy.wrapping_sub((self.y as i32).wrapping_mul(32)) as u32;
        (ux < 32 && uy < 32).then_some((ux | (uy * 32)) as usize)
    }

    fn set_chunk_loaded(&mut self, wx: i32, wy: i32, loaded: bool) {
        let ux = wx.wrapping_sub((self.x as i32).wrapping_mul(32));
        let uy = wy.wrapping_sub((self.y as i32).wrapping_mul(32));
        if (0..32).contains(&ux) && (0..32).contains(&uy) {
            let bit = 1u32 << (wx & 31);
            let w = &mut self.chunk_loaded[uy as usize];
            if loaded {
                *w |= bit;
            } else {
                *w &= !bit;
            }
        }
    }

    /// `isChunkLoadedWorldPos`.
    pub fn is_chunk_loaded_world(&self, x: i32, y: i32) -> bool {
        let lx = x.wrapping_sub((self.x as i32).wrapping_mul(256));
        let ly = y.wrapping_sub((self.y as i32).wrapping_mul(256));
        let cx = (if lx < 0 { lx + 7 } else { lx }) >> 3;
        let cy = (if ly < 0 { ly + 7 } else { ly }) >> 3;
        if lx > -8 && cx < 32 && ly > -8 && cy < 32 {
            let ux = cx as u32;
            if ux >> 5 < 1 {
                return (self.chunk_loaded[cy as usize] >> (ux & 31)) & 1 != 0;
            }
        }
        false
    }
}

/// `popman::VirtualZombie`: a walking group of stored zombies.
#[derive(Clone, Default)]
pub struct VirtualZombie {
    pub zombie: Option<ZId>,
    /// Group members (the leader first) when more than one.
    pub group: Vec<ZId>,
    pub last: (f32, f32),
    pub next: (f32, f32),
    pub vel: (f32, f32),
    pub toggle: bool,
    pub is_real: bool,
    pub cell: Option<CellIdx>,
    pub sound: Option<SoundId>,
    pub sound_steps: i16,
}

impl VirtualZombie {
    fn group_size(&self) -> usize {
        self.group.len().max(1)
    }
}

/// `popman::WorldSound`.
#[derive(Clone, Copy, Default)]
struct WorldSound {
    x: i32,
    y: i32,
    radius: i32,
    volume: i32,
    created_ms: i64,
    applied_ms: i64,
    hours: i32,
}

/// `popman::Horde` (`n_spawnHorde`).
#[derive(Clone, Copy, Debug)]
pub struct Horde {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub tx: f32,
    pub ty: f32,
    pub count: i32,
}

/// `popman::LoadedAreas`.
#[derive(Clone, Default, Debug)]
pub struct NativeLoadedAreas {
    pub areas: Vec<[i32; 4]>,
    pub changed: bool,
}

impl NativeLoadedAreas {
    fn copy_from(&mut self, o: &NativeLoadedAreas) {
        self.areas.clone_from(&o.areas);
        self.changed |= o.changed;
    }

    fn clear(&mut self) {
        self.areas.clear();
        self.changed = false;
    }

    /// `isOnEdge(x, y)`.
    pub fn is_on_edge(&self, x: i32, y: i32) -> bool {
        let edge = |v: i32| v == ffloor(v as f32 / 8.0).wrapping_mul(8);
        if !(edge(x) || edge(x.wrapping_add(1)) || edge(y) || edge(y.wrapping_add(1))) || self.areas.is_empty() {
            return false;
        }
        for a in &self.areas {
            let x0 = a[0].wrapping_mul(8);
            let y0 = a[1].wrapping_mul(8);
            let x1 = a[2].wrapping_add(a[0]).wrapping_mul(8);
            let y1 = a[3].wrapping_add(a[1]).wrapping_mul(8);
            if (x0 <= x && x < x1 && (y == y0 || y == y1 - 1)) || (y0 <= y && y < y1 && (x == x0 || x == x1 - 1)) {
                return true;
            }
        }
        false
    }
}

/// A real zombie's report (`n_updateRealZombies`).
#[derive(Clone, Copy, Debug)]
pub struct RealUpdate {
    pub id: u32,
    pub state: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// `DebugCommands.n_debugCommand` commands.
#[derive(Clone, Copy, Debug)]
pub enum DebugCmd {
    SpawnTimeToZero(i32, i32),
    ClearZombies(i32, i32),
    SpawnNow(i32, i32),
}

/// `popman::PassToWorker`.
#[derive(Clone, Default)]
struct PassToWorker {
    loaded_areas: NativeLoadedAreas,
    server_cells: NativeLoadedAreas,
    hours: f64,
    multiplier: f32,
    time_changed: bool,
    config_float: BTreeMap<Vec<u8>, f32>,
    config_int: BTreeMap<Vec<u8>, i32>,
    config_changed: bool,
    aggro: Vec<(i32, i32, i32)>,
    chunks: Vec<(i32, i32, bool)>,
    add: Vec<ZId>,
    register: Vec<ZId>,
    release: Vec<u32>,
    real: Vec<RealUpdate>,
    hordes: Vec<Horde>,
    sounds: Vec<SoundId>,
    debug: Vec<DebugCmd>,
    radar_request: bool,
    debug_request: bool,
    recycled: Vec<PtmId>,
}

impl PassToWorker {
    fn is_empty(&self) -> bool {
        !(self.loaded_areas.changed
            || self.server_cells.changed
            || self.time_changed
            || self.config_changed
            || !self.aggro.is_empty()
            || !self.chunks.is_empty()
            || !self.add.is_empty()
            || !self.register.is_empty()
            || !self.release.is_empty()
            || !self.real.is_empty()
            || !self.hordes.is_empty()
            || !self.sounds.is_empty()
            || !self.debug.is_empty()
            || self.radar_request
            || self.debug_request
            || !self.recycled.is_empty())
    }

    /// `clear()` (keeps the time fields).
    fn clear(&mut self) {
        self.loaded_areas.clear();
        self.server_cells.clear();
        self.time_changed = false;
        self.config_float.clear();
        self.config_int.clear();
        self.config_changed = false;
        self.aggro.clear();
        self.chunks.clear();
        self.add.clear();
        self.register.clear();
        self.release.clear();
        self.real.clear();
        self.hordes.clear();
        self.sounds.clear();
        self.debug.clear();
        self.radar_request = false;
        self.debug_request = false;
    }

    /// `combine(o)` (moves `o`'s lists).
    fn combine(&mut self, o: &mut PassToWorker) {
        if o.loaded_areas.changed {
            self.loaded_areas.copy_from(&o.loaded_areas);
        }
        if o.server_cells.changed {
            self.server_cells.copy_from(&o.server_cells);
        }
        if o.time_changed {
            self.hours = o.hours;
            self.multiplier = o.multiplier;
            self.time_changed = true;
        }
        if o.config_changed {
            self.config_float.clone_from(&o.config_float);
            self.config_int.clone_from(&o.config_int);
            self.config_changed = o.config_changed;
        }
        self.aggro.extend_from_slice(&o.aggro);
        self.chunks.extend_from_slice(&o.chunks);
        self.add.append(&mut o.add);
        self.register.append(&mut o.register);
        self.release.extend_from_slice(&o.release);
        if !o.real.is_empty() {
            self.real.clone_from(&o.real);
        }
        self.hordes.extend_from_slice(&o.hordes);
        self.sounds.append(&mut o.sounds);
        self.debug.extend_from_slice(&o.debug);
        self.radar_request |= o.radar_request;
        self.debug_request |= o.debug_request;
    }
}

/// `popman::MPDebugInfo` (what `MPDebugInfo.n_*` read).
#[derive(Clone, Default, Debug)]
pub struct MpDebugInfo {
    /// Loaded cells: x, y, effective population, target population and
    /// last repopulation hours (as the library packs them).
    pub cells: Vec<(i16, i16, i16, i16, f32)>,
    pub loaded_areas: NativeLoadedAreas,
    pub server_cells: NativeLoadedAreas,
    pub repop_events: Vec<(i32, i32, i32)>,
    pub has_data: bool,
}

/// `popman::PassToMain`.
#[derive(Clone, Default)]
struct PassToMain {
    add: Vec<ZId>,
    radar: Vec<(f32, f32)>,
    has_radar: bool,
    debug: MpDebugInfo,
    repop_events: Vec<(i32, i32, i32)>,
    recycled: Vec<PtwId>,
}

impl PassToMain {
    fn is_empty(&self) -> bool {
        self.add.is_empty() && !self.has_radar && !self.debug.has_data && self.repop_events.is_empty() && self.recycled.is_empty()
    }

    fn clear(&mut self) {
        self.add.clear();
        self.radar.clear();
        self.has_radar = false;
        self.debug.cells.clear();
        self.debug.loaded_areas.clear();
        self.debug.server_cells.clear();
        self.repop_events.clear();
        self.debug.has_data = false;
    }
}

/// A repopulation waiting for its path (`popman::RepopulateTask`).
#[derive(Clone, Copy)]
struct RepopTask {
    cell: CellIdx,
    count: i32,
}

/// The `PopulationConfig` the worker applies.
#[derive(Clone, Copy, Debug)]
pub struct PopulationConfig {
    pub population_multiplier: f32,
    pub population_start_multiplier: f32,
    pub population_peak_multiplier: f32,
    pub population_peak_day: i32,
    pub respawn_unseen_hours: f32,
    pub respawn_hours: f32,
    pub respawn_multiplier: f32,
    pub follow_sound_distance: i32,
    pub min_zombies_per_chunk: f32,
    pub max_zombies_per_chunk: f32,
    pub uniform_zombies_per_chunk: f32,
    pub redistribute_hours: f32,
}

impl Default for PopulationConfig {
    fn default() -> Self {
        PopulationConfig {
            population_multiplier: 1.0,
            population_start_multiplier: 0.8,
            population_peak_multiplier: f32::from_bits(0x3f99_999a),
            population_peak_day: 28,
            respawn_unseen_hours: 1.0,
            respawn_hours: 1.0,
            respawn_multiplier: 10.0,
            follow_sound_distance: 100,
            min_zombies_per_chunk: f32::from_bits(0x3d1d_4951),
            max_zombies_per_chunk: f32::from_bits(0x40f5_c28f),
            uniform_zombies_per_chunk: f32::from_bits(0x3e4c_cccd),
            redistribute_hours: 1.0,
        }
    }
}

const FLOAT_KEYS: [&str; 10] = [
    "PopulationMultiplier",
    "PopulationStartMultiplier",
    "PopulationPeakMultiplier",
    "RespawnHours",
    "RespawnUnseenHours",
    "RespawnMultiplier",
    "RedistributeHours",
    "MinZombiesPerChunk",
    "MaxZombiesPerChunk",
    "UniformZombiesPerChunk",
];
const INT_KEYS: [&str; 2] = ["PopulationPeakDay", "FollowSoundDistance"];

/// The worker's collision flags (`VirtualZombie::collided*`).
#[derive(Default, Clone, Copy)]
struct Collided {
    this_frame: bool,
    n: bool,
    s: bool,
    w: bool,
    e: bool,
}

/// `popman::ManagerWorker` fields.
pub struct Worker {
    pub client: bool,
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
    pub cells: Vec<Cell>,
    pub loaded_cells: Vec<CellIdx>,
    pub vzs: Vec<VzId>,
    pub vz_dirty: bool,
    pub real_zombies: HashMap<u32, VzId>,
    last_reported_unregistered: i32,
    pub aggro_targets: Vec<(i32, i32, i32)>,
    sounds: Vec<SoundId>,
    pub loaded_areas: NativeLoadedAreas,
    pub server_cells: NativeLoadedAreas,
    cur_ptm: PtmId,
    pub paused: bool,
    pub next_save_ms: i64,
    pub spawn_origins: Vec<[i32; 4]>,
    ptm_free: Vec<PtmId>,
    pub config: PopulationConfig,
}

/// The library's main-thread state (`popman::ManagerMain`).
struct MainSide {
    combined: PassToMain,
    cur_ptw: PtwId,
    ptw_free: Vec<PtwId>,
    radar: Vec<(f32, f32)>,
}

/// The whole native library.
pub struct Native {
    pub gs: GameState,
    pub rand: NativeRand,
    pub meta: NativeMetaGrid,
    pub mcd: Mcd,
    pub zombies: Vec<Zombie>,
    zombie_free: Vec<ZId>,
    pub vzs: Vec<VirtualZombie>,
    vz_free: Vec<VzId>,
    ptws: Vec<Option<PassToWorker>>,
    ptms: Vec<Option<PassToMain>>,
    q_to_worker: VecDeque<PtwId>,
    q_to_main: VecDeque<PtmId>,
    main: MainSide,
    pub worker: Worker,
    /// `ZombieIDs::counter`.
    pub id_counter: u32,
    /// `ZombieIDFile::saved`.
    pub id_saved: u32,
    /// Outfit names from `n_setOutfitNames` and their index map.
    pub outfit_names: Vec<Vec<u8>>,
    outfit_index: BTreeMap<Vec<u8>, i32>,
    collided: Collided,
    vz_wall_follower: Box<WallFollower>,
    path_wall_follower: Box<WallFollower>,
    ff_horde: FloodFill,
    ff_repop: FloodFill,
    ff_expand: FloodFill,
    ff_redistribute: FloodFill,
    ff_sound: FloodFill,
    task_id: u32,
    repop_tasks: HashMap<u32, RepopTask>,
    /// `WorldSound` objects by address: a freed sound's slot is the next one
    /// handed out (the allocator's LIFO reuse), with its stale fields, so a
    /// virtual zombie still pointing at a freed sound sees the new one as its
    /// own and `lastApplied` starts from the old object's value.
    sounds: HashMap<SoundId, WorldSound>,
    sound_free: Vec<SoundId>,
    /// Whether the next `n_worldSound` may take a freed sound's slot (reset
    /// to `true` after each call). Which block `malloc` hands back is not
    /// deterministic in the original; clearing this models a fresh block,
    /// which a differential harness uses to follow a particular run.
    pub reuse_freed_sound: bool,
    next_sound_id: SoundId,
    /// `writeToLog` messages (severity, text).
    pub log: Vec<(i32, String)>,
    /// Debug `MPDebugInfo` the main side last received.
    pub mp_debug: MpDebugInfo,
}

impl Default for Native {
    fn default() -> Self {
        Native::new(5489)
    }
}

impl Native {
    /// A fresh library whose generator is `std::mt19937(seed)`.
    pub fn new(seed: u32) -> Native {
        let mut n = Native {
            gs: GameState::default(),
            rand: NativeRand::new(seed),
            meta: NativeMetaGrid::default(),
            mcd: Mcd::default(),
            zombies: Vec::new(),
            zombie_free: Vec::new(),
            vzs: Vec::new(),
            vz_free: Vec::new(),
            ptws: Vec::new(),
            ptms: Vec::new(),
            q_to_worker: VecDeque::new(),
            q_to_main: VecDeque::new(),
            main: MainSide { combined: PassToMain::default(), cur_ptw: 0, ptw_free: Vec::new(), radar: Vec::new() },
            worker: Worker {
                client: false,
                min_x: 0,
                min_y: 0,
                width: 0,
                height: 0,
                cells: Vec::new(),
                loaded_cells: Vec::new(),
                vzs: Vec::new(),
                vz_dirty: true,
                real_zombies: HashMap::new(),
                last_reported_unregistered: 0,
                aggro_targets: Vec::new(),
                sounds: Vec::new(),
                loaded_areas: NativeLoadedAreas::default(),
                server_cells: NativeLoadedAreas::default(),
                cur_ptm: 0,
                paused: false,
                next_save_ms: 0,
                spawn_origins: Vec::new(),
                ptm_free: Vec::new(),
                config: PopulationConfig::default(),
            },
            id_counter: 1,
            id_saved: 0,
            outfit_names: Vec::new(),
            outfit_index: BTreeMap::new(),
            collided: Collided::default(),
            vz_wall_follower: Box::default(),
            path_wall_follower: Box::default(),
            ff_horde: FloodFill::new(40),
            ff_repop: FloodFill::new(40),
            ff_expand: FloodFill::new(8),
            ff_redistribute: FloodFill::new(8),
            ff_sound: FloodFill::new(8),
            task_id: 0,
            repop_tasks: HashMap::new(),
            sounds: HashMap::new(),
            sound_free: Vec::new(),
            reuse_freed_sound: true,
            next_sound_id: 1,
            log: Vec::new(),
            mp_debug: MpDebugInfo::default(),
        };
        n.main.cur_ptw = n.new_ptw();
        n.worker.cur_ptm = n.new_ptm();
        n
    }

    fn new_ptw(&mut self) -> PtwId {
        if let Some(i) = self.ptws.iter().position(|p| p.is_none()) {
            self.ptws[i] = Some(PassToWorker::default());
            return i;
        }
        self.ptws.push(Some(PassToWorker::default()));
        self.ptws.len() - 1
    }

    fn new_ptm(&mut self) -> PtmId {
        if let Some(i) = self.ptms.iter().position(|p| p.is_none()) {
            self.ptms[i] = Some(PassToMain::default());
            return i;
        }
        self.ptms.push(Some(PassToMain::default()));
        self.ptms.len() - 1
    }

    fn ptw(&mut self, i: PtwId) -> &mut PassToWorker {
        self.ptws[i].as_mut().unwrap()
    }

    fn ptm(&mut self, i: PtmId) -> &mut PassToMain {
        self.ptms[i].as_mut().unwrap()
    }

    fn cur(&mut self) -> &mut PassToWorker {
        let i = self.main.cur_ptw;
        self.ptw(i)
    }

    fn cur_ptm(&mut self) -> &mut PassToMain {
        let i = self.worker.cur_ptm;
        self.ptm(i)
    }

    fn free_sound(&mut self, s: SoundId) {
        self.sound_free.push(s);
    }

    fn write_log(&mut self, severity: i32, msg: String) {
        self.log.push((severity, msg));
    }

    // ---- zombie records ----------------------------------------------

    fn alloc_zombie(&mut self) -> ZId {
        if let Some(z) = self.zombie_free.pop() {
            return z;
        }
        self.zombies.push(Zombie { dir: 8, ..Default::default() });
        (self.zombies.len() - 1) as ZId
    }

    fn release_zombie(&mut self, z: ZId) {
        self.zombie_free.push(z);
    }

    /// `ZombieIDs::next()`.
    pub fn next_zombie_id(&mut self) -> u32 {
        let v = self.id_counter;
        self.id_counter = self.id_counter.wrapping_add(1);
        if v != 0 {
            return v;
        }
        self.id_counter = 2;
        1
    }

    /// `Zombie::init`.
    fn zombie_init(&mut self, z: ZId, x: f32, y: f32, zz: f32, px: i32, py: i32) {
        let dir = self.rand.next(8);
        let id = self.next_zombie_id();
        let r = &mut self.zombies[z as usize];
        r.x = x;
        r.y = y;
        r.z = zz;
        r.dir = if (dir as u32) > 7 { 8 } else { dir as u8 };
        r.outfit = 0;
        r.state = 4;
        r.path_x = px;
        r.path_y = py;
        r.id = id;
    }

    /// `Zombie::load`.
    fn zombie_load(&mut self, z: ZId, inp: &mut DataInput, version: i32) -> io::Result<()> {
        let x = inp.read_float()?;
        let y = inp.read_float()?;
        let zz = inp.read_byte()? as f32;
        let dir = inp.read_byte()? as u8;
        let mut state;
        let outfit;
        let mut id = None;
        if version < 5 {
            state = 0;
            let old = inp.read_byte()? as u8;
            if old.wrapping_sub(1) < 4 {
                state |= [6, 8, 2, 4][old.wrapping_sub(1) as usize];
            }
            outfit = if version == 4 {
                inp.read_int()?
            } else {
                if self.gs.server {
                    inp.read_short()?;
                }
                0
            };
        } else {
            state = inp.read_int()?;
            outfit = inp.read_int()?;
            if version > 6 {
                let v = inp.read_int()? as u32;
                if v != 0 {
                    if self.id_counter < v.wrapping_add(1) {
                        self.id_counter = v.wrapping_add(1);
                    }
                    id = Some(v);
                }
            }
        }
        let id = match id {
            Some(v) => v,
            None => self.next_zombie_id(),
        };
        let r = &mut self.zombies[z as usize];
        r.x = x;
        r.y = y;
        r.z = zz;
        r.dir = if dir > 7 { 8 } else { dir };
        r.state = state;
        r.outfit = outfit;
        r.id = id;
        r.path_x = NO_PATH;
        r.path_y = NO_PATH;
        Ok(())
    }

    /// `Zombie::fixOutfitNames(names)`: remaps a file's outfit index to the
    /// current outfit list.
    fn zombie_fix_outfit(&mut self, z: ZId, names: &[Vec<u8>]) {
        let o = self.zombies[z as usize].outfit as u32;
        if o == 0 {
            return;
        }
        let o = o & 0x7fff_ffff;
        self.zombies[z as usize].outfit = o as i32;
        let idx = ((o >> 16) & 0x7fff) as usize;
        let new = if idx >= names.len() {
            0
        } else {
            match self.outfit_index.get(&names[idx]) {
                Some(&i) => (o & 0x8000_ffff) | ((i as u32) << 16),
                None => 0,
            }
        };
        self.zombies[z as usize].outfit = new as i32;
    }

    // ---- virtual zombies -----------------------------------------------

    fn alloc_vz(&mut self) -> VzId {
        if let Some(v) = self.vz_free.pop() {
            return v;
        }
        self.vzs.push(VirtualZombie::default());
        (self.vzs.len() - 1) as VzId
    }

    /// `VirtualZombiePool::release`.
    fn release_vz(&mut self, v: VzId) {
        let vz = &mut self.vzs[v as usize];
        vz.zombie = None;
        vz.group.clear();
        vz.sound = None;
        vz.is_real = false;
        self.vz_free.push(v);
    }

    /// `VirtualZombie::init(zombie, px, py)`.
    fn vz_init(&mut self, v: VzId, z: ZId, path: Option<(i32, i32)>) {
        if let Some((px, py)) = path {
            self.zombies[z as usize].path_x = px;
            self.zombies[z as usize].path_y = py;
        }
        let vz = &mut self.vzs[v as usize];
        vz.zombie = Some(z);
        vz.cell = None;
        vz.sound = None;
    }

    fn leader(&self, v: VzId) -> ZId {
        self.vzs[v as usize].zombie.unwrap()
    }

    fn leader_pos(&self, v: VzId) -> (f32, f32) {
        let z = &self.zombies[self.leader(v) as usize];
        (z.x, z.y)
    }

    /// `updateCurrentCell`.
    fn vz_update_current_cell(&mut self, v: VzId) {
        let (x, y) = self.leader_pos(v);
        let c = self.cell_from_world(ffloor(x), ffloor(y));
        let old = self.vzs[v as usize].cell;
        if c == old {
            return;
        }
        if let Some(o) = old {
            let cell = &mut self.worker.cells[o];
            if let Some(i) = cell.vzs.iter().position(|&e| e == v) {
                cell.vzs.remove(i);
            }
            cell.dirty = true;
        }
        if let Some(n) = c {
            let cell = &mut self.worker.cells[n];
            cell.vzs.push(v);
            cell.dirty = true;
        }
        self.vzs[v as usize].cell = c;
    }

    /// `clearCurrentCell`.
    fn vz_clear_current_cell(&mut self, v: VzId) {
        if let Some(o) = self.vzs[v as usize].cell {
            let cell = &mut self.worker.cells[o];
            if let Some(i) = cell.vzs.iter().position(|&e| e == v) {
                cell.vzs.remove(i);
            }
            cell.dirty = true;
            self.vzs[v as usize].cell = None;
        }
    }

    // ---- cells ------------------------------------------------------------

    fn cell_from_world(&self, x: i32, y: i32) -> Option<CellIdx> {
        let w = &self.worker;
        let ux = x.wrapping_sub(w.min_x.wrapping_mul(256));
        if ux < 0 {
            return None;
        }
        let uy = y.wrapping_sub(w.min_y.wrapping_mul(256));
        if uy < 0 {
            return None;
        }
        let cx = ux >> 8;
        let cy = uy >> 8;
        if cx >= w.width || cy >= w.height {
            return None;
        }
        Some((cx + w.width * cy) as usize)
    }

    fn cell_from_cell_pos(&self, cx: i32, cy: i32) -> Option<CellIdx> {
        let w = &self.worker;
        let x = cx.wrapping_sub(w.min_x);
        let y = cy.wrapping_sub(w.min_y);
        (x >= 0 && x < w.width && y >= 0 && y < w.height).then(|| (x + w.width * y) as usize)
    }

    fn cell_from_chunk(&self, wx: i32, wy: i32) -> Option<CellIdx> {
        self.cell_from_world(wx.wrapping_mul(8), wy.wrapping_mul(8))
    }

    /// `ManagerWorker::loadIfNeeded`.
    fn load_if_needed(&mut self, c: CellIdx) {
        self.worker.cells[c].last_seen_ms = self.gs.now();
        if !self.worker.cells[c].loaded {
            self.cell_load(c);
            self.cell_init_population(c);
            self.cell_calc_population(c);
            self.worker.loaded_cells.push(c);
        }
    }

    fn zpop_file(&self, name: &str) -> PathBuf {
        self.gs.save_file(&self.gs.subdir_zpop, name)
    }

    fn cell_file(&self, c: CellIdx) -> PathBuf {
        let cell = &self.worker.cells[c];
        self.zpop_file(&format!("zpop_{}_{}.bin", cell.x, cell.y))
    }

    /// `Cell::load()`.
    fn cell_load(&mut self, c: CellIdx) {
        {
            let cell = &mut self.worker.cells[c];
            cell.loaded = true;
            cell.chunks = (0..1024)
                .map(|i| Chunk {
                    wx: (i % 32 + cell.x as i32 * 32) as i16,
                    wy: (i / 32 + cell.y as i32 * 32) as i16,
                    ..Default::default()
                })
                .collect();
        }
        let path = self.cell_file(c);
        let ok = match DataInput::open(&path) {
            Ok(mut inp) => match self.cell_read(c, &mut inp, &path) {
                Ok(()) => true,
                Err(e) => {
                    self.write_log(2, format!("ERROR loading {}: {e}", path.display()));
                    false
                }
            },
            Err(_) => false,
        };
        self.worker.cells[c].from_file = ok;
    }

    fn cell_read(&mut self, c: CellIdx, inp: &mut DataInput, path: &std::path::Path) -> io::Result<()> {
        let version = inp.read_short()? as i32;
        if !(1..=7).contains(&version) {
            return Err(io::Error::other(format!("unknown file version \"{version}\"")));
        }
        let mut names = Vec::new();
        if version > 3 {
            let n = inp.read_short()?;
            for _ in 0..n.max(0) {
                names.push(inp.read_utf()?);
            }
        }
        self.worker.cells[c].last_repop = inp.read_float()?;
        if version > 2 {
            self.worker.cells[c].last_redistribute = inp.read_float()?;
        }
        let mut legacy = LegacyZombieSet::default();
        for i in 0..1024 {
            self.chunk_load(c, i, inp, version, &mut legacy)?;
            let zs = self.worker.cells[c].chunks[i].zombies.clone();
            for z in zs {
                self.zombie_fix_outfit(z, &names);
            }
        }
        if legacy.dropped > 0 {
            self.write_log(1, format!("ZPOP: dropped {} duplicate zombie records from {}", legacy.dropped, path.display()));
            self.worker.cells[c].dirty = true;
        }
        Ok(())
    }

    /// `Chunk::load`.
    fn chunk_load(
        &mut self,
        c: CellIdx,
        i: usize,
        inp: &mut DataInput,
        version: i32,
        legacy: &mut LegacyZombieSet,
    ) -> io::Result<()> {
        let desired = inp.read_short()?;
        self.worker.cells[c].chunks[i].desired = desired;
        let n = inp.read_short()?;
        for _ in 0..n.max(0) {
            let z = self.alloc_zombie();
            self.zombie_load(z, inp, version)?;
            if version < 7 && !legacy.insert(&self.zombies[z as usize]) {
                self.release_zombie(z);
                continue;
            }
            self.worker.cells[c].chunks[i].zombies.push(z);
        }
        let chunk = &mut self.worker.cells[c].chunks[i];
        chunk.last_seen = inp.read_float()?;
        chunk.last_repop = inp.read_float()?;
        Ok(())
    }

    /// `Cell::unload()`.
    fn cell_unload(&mut self, c: CellIdx) {
        if self.worker.cells[c].dirty {
            self.worker.cells[c].dirty = false;
            self.cell_save(c, false);
        }
        let chunks = std::mem::take(&mut self.worker.cells[c].chunks);
        for ch in chunks {
            for z in ch.zombies {
                self.release_zombie(z);
            }
        }
        self.worker.cells[c].loaded = false;
    }

    /// `Cell::save(force)`.
    fn cell_save(&mut self, c: CellIdx, force: bool) {
        if !self.worker.cells[c].loaded || self.gs.no_save {
            return;
        }
        if !force && !(self.id_file_save_if_advanced() && self.virtual_file_save()) {
            self.worker.cells[c].dirty = true;
            return;
        }
        let cell = &self.worker.cells[c];
        // Extra zombies to write: pending saves, then real zombies in the
        // cell, each with the chunk it falls in.
        let mut extra: Vec<(ZId, Option<usize>)> = Vec::new();
        for &z in &cell.save_pending {
            let r = &self.zombies[z as usize];
            extra.push((z, cell.chunk_index_world(ffloor(r.x), ffloor(r.y))));
        }
        for &v in &cell.vzs {
            let vz = &self.vzs[v as usize];
            if vz.is_real {
                let z = vz.zombie.unwrap();
                let r = &self.zombies[z as usize];
                extra.push((z, cell.chunk_index_world(ffloor(r.x), ffloor(r.y))));
            }
        }
        let mut out = DataOutput::default();
        out.write_short(7);
        out.write_short(self.outfit_names.len() as i16);
        for n in &self.outfit_names {
            out.write_utf(n);
        }
        out.write_float(cell.last_repop);
        out.write_float(cell.last_redistribute);
        for (i, ch) in cell.chunks.iter().enumerate() {
            let mut list: Vec<ZId> = ch.zombies.clone();
            for &(z, ci) in &extra {
                if ci == Some(i) {
                    list.push(z);
                }
            }
            out.write_short(ch.desired);
            out.write_short(list.len() as i16);
            for z in list {
                self.zombies[z as usize].save(&mut out);
            }
            out.write_float(ch.last_seen);
            out.write_float(ch.last_repop);
        }
        let path = self.cell_file(c);
        if let Err(e) = out.save(&path) {
            self.write_log(2, format!("failed to open for writing: {} ({e})", path.display()));
        }
    }

    /// `Chunk::calcDesiredPopulation`.
    fn chunk_calc_desired(&mut self, c: CellIdx, i: usize) -> i16 {
        self.worker.cells[c].chunks[i].desired = 0;
        let (wx, wy) = {
            let ch = &self.worker.cells[c].chunks[i];
            (ch.wx as i32, ch.wy as i32)
        };
        if self.gs.zombies_disabled || self.gs.last_stand || self.mcd.is_chunk_blocked(wx, wy, BLOCK_MASK, &self.gs, &self.meta) {
            return 0;
        }
        let cfg = self.worker.config;
        let mut f = 0.0f32;
        if let Some(intensity) = self.meta.get_chunk_data(wx, wy) {
            let mut v = intensity as f32;
            if self.gs.distribution == 2 {
                v = cfg.uniform_zombies_per_chunk;
            }
            if v != 0.0 {
                f = cfg.min_zombies_per_chunk
                    + ((v * cfg.population_multiplier) / 255.0)
                        * (cfg.max_zombies_per_chunk - cfg.min_zombies_per_chunk);
                if f >= 1.0 {
                    let n = f as i32;
                    if n != 1 {
                        self.worker.cells[c].chunks[i].desired = n as i16;
                    } else if self.rand.next(100) < 50 {
                        self.worker.cells[c].chunks[i].desired = 1;
                    }
                    return self.worker.cells[c].chunks[i].desired;
                }
            }
        }
        let r = self.rand.next(100);
        if f * 100.0 > r as f32 {
            self.worker.cells[c].chunks[i].desired = 1;
        }
        self.worker.cells[c].chunks[i].desired
    }

    fn days_lerp(&self) -> Option<f32> {
        let days = self.gs.world_age_hours as f32 / 24.0;
        let peak = self.worker.config.population_peak_day as f32;
        (days < peak).then(|| days / peak)
    }

    /// `Cell::targetPopulation(n)`.
    fn cell_target_population(&self, n: i32) -> i32 {
        let cfg = &self.worker.config;
        let nf = n as f32;
        match self.days_lerp() {
            Some(t) => {
                let s = cfg.population_start_multiplier * nf;
                (cfg.population_peak_multiplier * nf - s).mul_add(t, s) as i32
            }
            None => (cfg.population_peak_multiplier * nf) as i32,
        }
    }

    /// `Chunk::targetPopulation()`.
    fn chunk_target_population(&self, desired: i16) -> i32 {
        let cfg = &self.worker.config;
        let nf = desired as i32 as f32;
        match self.days_lerp() {
            Some(t) => {
                let s = (cfg.population_start_multiplier * nf).ceil();
                let p = (cfg.population_peak_multiplier * nf).ceil();
                (p - s).mul_add(t, s) as i32
            }
            None => (cfg.population_peak_multiplier * nf).ceil() as i32,
        }
    }

    /// `Chunk::addRandomZombie`.
    fn chunk_add_random_zombie(&mut self, c: CellIdx, i: usize) -> bool {
        let (wx, wy) = {
            let ch = &self.worker.cells[c].chunks[i];
            (ch.wx as i32, ch.wy as i32)
        };
        for _ in 0..100 {
            let x = self.rand.next(8) + wx * 8;
            let y = self.rand.next(8) + wy * 8;
            if self.can_spawn_at(x, y) {
                let z = self.alloc_zombie();
                self.zombie_init(z, x as f32 + 0.5, y as f32 + 0.5, 0.0, NO_PATH, NO_PATH);
                self.worker.cells[c].chunks[i].zombies.push(z);
                self.worker.cells[c].dirty = true;
                return true;
            }
        }
        false
    }

    /// `Cell::initPopulation`.
    fn cell_init_population(&mut self, c: CellIdx) {
        if self.worker.cells[c].from_file {
            return;
        }
        let hours = self.gs.world_age_hours as f32;
        let mut list: Vec<usize> = Vec::new();
        let mut total = 0i32;
        for i in 0..1024 {
            let d = self.chunk_calc_desired(c, i);
            if d > 0 {
                list.push(i);
                total += d as i32;
            }
            self.worker.cells[c].chunks[i].last_repop = hours;
        }
        if !list.is_empty() {
            let target = self.cell_target_population(total);
            let totalf = total as f32;
            let targetf = target as f32;
            let mut added = 0i32;
            if total > target {
                for &i in &list {
                    let d = self.worker.cells[c].chunks[i].desired as i32 as f32;
                    let n = ((d / totalf) * targetf) as i32;
                    for _ in 0..n.max(0) {
                        added += self.chunk_add_random_zombie(c, i) as i32;
                    }
                }
            } else {
                for &i in &list {
                    let d = self.worker.cells[c].chunks[i].desired as i32 as f32;
                    let n = (((d / totalf) * targetf) as i32).max(1);
                    for _ in 0..n {
                        added += self.chunk_add_random_zombie(c, i) as i32;
                    }
                }
            }
            let remaining = target - added;
            if remaining >= 1 {
                for _ in 0..remaining {
                    let mut tries = 0u32;
                    loop {
                        let k = self.rand.next(list.len() as i32) as usize;
                        let ok = self.chunk_add_random_zombie(c, list[k]);
                        let ok = ok || tries > 0x382;
                        tries += 1;
                        if ok {
                            break;
                        }
                    }
                }
            }
        }
        let cell = &mut self.worker.cells[c];
        cell.last_repop = hours;
        cell.last_redistribute = hours;
        cell.dirty = true;
    }

    /// `Cell::calcPopulation`.
    fn cell_calc_population(&mut self, c: CellIdx) {
        let cell = &mut self.worker.cells[c];
        let mut desired = 0i16;
        let mut stored = 0i16;
        for ch in &cell.chunks {
            desired = desired.wrapping_add(ch.desired);
            stored = stored.wrapping_add(ch.zombies.len() as i16);
        }
        cell.desired = desired;
        cell.stored = stored;
    }

    /// `Cell::effectivePopulation`.
    fn cell_effective_population(&self, c: CellIdx) -> i32 {
        let cell = &self.worker.cells[c];
        let mut sum = 0u16;
        for &v in &cell.vzs {
            sum = sum.wrapping_add(self.vzs[v as usize].group_size() as u16);
        }
        (sum as i16 as i32) + cell.stored as i32
    }

    // ---- spawning checks ----------------------------------------------------

    fn bits(&mut self, x: i32, y: i32) -> u8 {
        self.mcd.get_bits(x, y, &self.gs, &self.meta)
    }

    fn blocked(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
        self.mcd.is_blocked(x1, y1, x2, y2, BLOCK_MASK, &self.gs, &self.meta)
    }

    fn can_path_only(&mut self, x: i32, y: i32, dir: u8) -> bool {
        if self.bits(x, y) & BLOCK_MASK != 0 {
            return false;
        }
        let n = self.blocked(x, y, x, y - 1);
        let s = self.blocked(x, y, x, y + 1);
        let w = self.blocked(x - 1, y, x, y);
        let e = self.blocked(x + 1, y, x, y);
        match dir {
            b'N' => !n && s && w && e,
            b'S' => n && !s && w && e,
            b'W' => n && s && !w && e,
            _ => n && s && w && !e,
        }
    }

    /// `ManagerWorker::canSpawnAt(x, y, 0)`.
    fn can_spawn_at(&mut self, x: i32, y: i32) -> bool {
        if self.bits(x, y) & BLOCK_MASK != 0 {
            return false;
        }
        let n = self.blocked(x, y, x, y - 1);
        let s = self.blocked(x, y, x, y + 1);
        let w = self.blocked(x - 1, y, x, y);
        let e = self.blocked(x + 1, y, x, y);
        if n && s && w && e {
            return false;
        }
        if self.can_path_only(x, y, b'E') && self.can_path_only(x + 1, y, b'W') {
            return false;
        }
        if self.can_path_only(x - 1, y, b'E') && self.can_path_only(x, y, b'W') {
            return false;
        }
        if self.can_path_only(x, y, b'S') && self.can_path_only(x, y + 1, b'N') {
            return false;
        }
        if self.can_path_only(x, y - 1, b'S') && self.can_path_only(x, y, b'N') {
            return false;
        }
        true
    }

    // ---- files ------------------------------------------------------------

    /// `ZombieIDFile::load`.
    fn id_file_load(&mut self) {
        self.id_saved = 0;
        let Ok(mut inp) = DataInput::open(&self.zpop_file("zpop_ids.bin")) else { return };
        let Ok(v) = inp.read_short() else { return };
        if !(1..=7).contains(&v) {
            self.write_log(2, format!("ZPOP: cannot read zpop_ids.bin: unknown file version \"{v}\""));
            return;
        }
        if let Ok(n) = inp.read_int() {
            let n = n as u32;
            if self.id_counter < n {
                self.id_counter = n;
            }
            self.id_saved = n;
        }
    }

    /// `ZombieIDFile::saveIfAdvanced`.
    fn id_file_save_if_advanced(&mut self) -> bool {
        if self.gs.no_save || self.id_counter == self.id_saved {
            return true;
        }
        let v = self.id_counter;
        let path = self.zpop_file("zpop_ids.bin");
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        let mut out = DataOutput::default();
        out.write_short(7);
        out.write_int(v as i32);
        if out.save(std::path::Path::new(&tmp)).is_err() || std::fs::rename(&tmp, &path).is_err() {
            self.write_log(2, format!("ZPOP: cannot write {}", path.display()));
            return false;
        }
        self.id_saved = v;
        true
    }

    /// `VirtualZombieFile::save`.
    fn virtual_file_save(&mut self) -> bool {
        if self.gs.no_save {
            return true;
        }
        if !self.id_file_save_if_advanced() {
            return false;
        }
        if !self.worker.vz_dirty {
            return true;
        }
        let mut out = DataOutput::default();
        out.write_short(7);
        out.write_short(self.outfit_names.len() as i16);
        for n in &self.outfit_names {
            out.write_utf(n);
        }
        out.write_int(self.worker.vzs.len() as i32);
        for &v in &self.worker.vzs {
            let vz = &self.vzs[v as usize];
            let n = vz.group.len().clamp(1, 0x7fff) as i16;
            out.write_short(n);
            let lead = &self.zombies[vz.zombie.unwrap() as usize];
            out.write_int(lead.path_x);
            out.write_int(lead.path_y);
            if n > 1 {
                for &z in &vz.group[..n as usize] {
                    self.zombies[z as usize].save(&mut out);
                }
            } else {
                lead.save(&mut out);
            }
        }
        if out.save(&self.zpop_file("zpop_virtual.bin")).is_err() {
            return false;
        }
        self.worker.vz_dirty = false;
        true
    }

    /// `VirtualZombieFile::load`.
    fn virtual_file_load(&mut self) {
        let path = self.zpop_file("zpop_virtual.bin");
        let Ok(mut inp) = DataInput::open(&path) else { return };
        if let Err(e) = self.virtual_file_read(&mut inp) {
            self.write_log(2, format!("ZPOP: cannot read zpop_virtual.bin: {e}"));
        }
    }

    fn virtual_file_read(&mut self, inp: &mut DataInput) -> io::Result<()> {
        let version = inp.read_short()? as i32;
        if !(1..=7).contains(&version) {
            return Err(io::Error::other(format!("unknown file version \"{version}\"")));
        }
        let mut names = Vec::new();
        if version > 3 {
            let n = inp.read_short()?;
            for _ in 0..n.max(0) {
                names.push(inp.read_utf()?);
            }
        }
        let n = inp.read_int()?;
        for _ in 0..n.max(0) {
            let z = self.alloc_zombie();
            let v = self.alloc_vz();
            self.vz_init(v, z, Some((NO_PATH, NO_PATH)));
            self.vz_load(v, inp, version)?;
            let members: Vec<ZId> = if self.vzs[v as usize].group.len() > 1 {
                self.vzs[v as usize].group.clone()
            } else {
                vec![self.leader(v)]
            };
            for m in members {
                self.zombie_fix_outfit(m, &names);
            }
            self.worker.vzs.push(v);
            self.worker.vz_dirty = true;
        }
        Ok(())
    }

    /// `VirtualZombie::load`.
    fn vz_load(&mut self, v: VzId, inp: &mut DataInput, version: i32) -> io::Result<()> {
        let n = inp.read_short()? as i32;
        let (px, py) = if version < 6 {
            (inp.read_short()? as i32, inp.read_short()? as i32)
        } else {
            (inp.read_int()?, inp.read_int()?)
        };
        let lead = self.leader(v);
        self.zombie_load(lead, inp, version)?;
        if n >= 2 {
            self.vzs[v as usize].group.push(lead);
            for _ in 0..n - 1 {
                let z = self.alloc_zombie();
                self.zombie_load(z, inp, version)?;
                self.vzs[v as usize].group.push(z);
            }
        }
        let keep = version > 5 || px != -1;
        let r = &mut self.zombies[lead as usize];
        r.path_x = if keep { px } else { NO_PATH };
        r.path_y = if keep { py } else { NO_PATH };
        Ok(())
    }
}

/// `popman::LegacyZombieSet`: drops duplicate records in pre-7 files.
#[derive(Default)]
struct LegacyZombieSet {
    seen: HashSet<u64>,
    dropped: i32,
}

impl LegacyZombieSet {
    fn insert(&mut self, z: &Zombie) -> bool {
        if z.state & 1 == 0 {
            return true;
        }
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [z.x.to_bits(), z.y.to_bits(), z.z.to_bits(), z.dir as u32, z.state as u32, z.outfit as u32] {
            h = (h ^ v as u64).wrapping_mul(0x100_0000_01b3);
        }
        if self.seen.insert(h) {
            return true;
        }
        self.dropped += 1;
        false
    }
}

include!("popman_worker.rs");
include!("popman_jni.rs");
