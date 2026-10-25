//! Game state outside `zombie.vehicles` that the vehicle code reads:
//! `System.currentTimeMillis`, `GameTime`, `SandboxOptions`, the cell's
//! squares and chunks, sounds/events, and item creation. Each is a
//! thread-local the engine (or a parity harness) installs, the way the
//! original reads static singletons.

use pz_inventory::InventoryItem;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The `GameTime` values the vehicle code reads, set once per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameTimeState {
    /// `getMultiplier()`.
    pub multiplier: f32,
    /// `getTimeDelta()`.
    pub time_delta: f32,
    /// `getThirtyFPSMultiplier()`.
    pub thirty_fps_multiplier: f32,
    /// `getInvMultiplier()`.
    pub inv_multiplier: f32,
    /// `getUnmoddedMultiplier()`.
    pub unmodded_multiplier: f32,
    /// `getPhysicsSecondsSinceLastUpdate()`.
    pub physics_seconds_since_last_update: f32,
    /// `getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `getDeltaMinutesPerDay()`.
    pub delta_minutes_per_day: f32,
    /// `GameTime.getServerTimeMills()`.
    pub server_time_mills: i64,
}

impl Default for GameTimeState {
    fn default() -> Self {
        GameTimeState {
            multiplier: 1.0,
            time_delta: 1.0 / 0.8 / 60.0,
            thirty_fps_multiplier: 1.0 / 1.6,
            inv_multiplier: 1.0,
            unmodded_multiplier: 1.0,
            physics_seconds_since_last_update: 1.0 / 30.0,
            world_age_hours: 0.0,
            delta_minutes_per_day: 30.0,
            server_time_mills: 0,
        }
    }
}

/// `SandboxOptions` values the vehicle code reads.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxVehicle {
    pub car_general_condition: i32,
    pub recently_survivor_vehicles: i32,
    pub vehicle_easy_use: bool,
    pub zombie_attraction_multiplier: f64,
    pub car_damage_on_impact: i32,
    pub player_damage_from_crash: bool,
    pub key_loot_new: f64,
    pub siren_effects_zombies: bool,
    pub siren_shutoff_hours: f64,
    pub injury_severity: i32,
    pub bone_fracture: bool,
}

impl Default for SandboxVehicle {
    fn default() -> Self {
        SandboxVehicle {
            car_general_condition: 3,
            recently_survivor_vehicles: 2,
            vehicle_easy_use: false,
            zombie_attraction_multiplier: 1.0,
            car_damage_on_impact: 3,
            player_damage_from_crash: true,
            key_loot_new: 0.6,
            siren_effects_zombies: true,
            siren_shutoff_hours: 0.0,
            injury_severity: 2,
            bone_fracture: true,
        }
    }
}

/// What the vehicle code asks of an `IsoGridSquare`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SquareInfo {
    /// `getFloor() != null`.
    pub has_floor: bool,
    /// The floor's sprite name (`isDoingOffroad`).
    pub floor_sprite: Option<String>,
    /// `getRoom().roomDef` (the room name).
    pub room_def: Option<String>,
    /// `isInARoom()`.
    pub in_room: bool,
    /// `getZone().getType()`.
    pub zone_type: Option<String>,
    /// The chunk's `refs` are not empty.
    pub chunk_has_refs: bool,
}

/// The cell, chunks and map zones as the vehicle code sees them.
pub trait VehicleWorld {
    /// `getCell().getGridSquare(x, y, z)`.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo>;
    /// `IsoWorld.instance.getMetaGrid().isValidChunk(wx, wy)`.
    fn is_valid_chunk(&self, wx: i32, wy: i32) -> bool;
    /// `currentCell.getChunk(wx, wy)` exists and is `loaded`.
    fn chunk_loaded(&self, wx: i32, wy: i32) -> bool;
    /// `IsoChunkMap.chunkGridWidth`.
    fn chunk_grid_width(&self) -> i32 {
        13
    }
    /// `metaGrid.getVehicleZoneAt(x, y, z).name`.
    fn vehicle_zone_name(&self, _x: i32, _y: i32, _z: i32) -> Option<String> {
        None
    }
    /// `RainManager.isRaining()`.
    fn is_raining(&self) -> bool {
        false
    }
    /// `chunk.vehicles` of a chunk in the cell (runtime vehicles, in list
    /// order); `None` when the chunk is not loaded. See
    /// [`crate::cell_link::chunk_vehicles`].
    fn chunk_vehicles(&self, _wx: i32, _wy: i32) -> Option<Vec<crate::VehicleRef>> {
        None
    }
    /// `Bullet.updatePhysicsForLevelIfNeeded(wx, wy, level)`: when the
    /// chunk level is marked for a physics check, clear the mark, build its
    /// `ToBullet` buffers (`IsoChunk.updatePhysicsForLevel`, see
    /// [`crate::chunk_physics`]) into `to_bullet` and answer `true`.
    fn update_physics_for_level_if_needed(
        &self,
        _wx: i32,
        _wy: i32,
        _level: i32,
        _to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool {
        false
    }
    /// `PolygonalMap2.instance.lineClearCollide(x1, y1, x2, y2, z, vehicle)`
    /// (`SurroundVehicle`'s blocked spots).
    fn line_clear_collide(
        &self,
        _x1: f32,
        _y1: f32,
        _x2: f32,
        _y2: f32,
        _z: i32,
        _vehicle: &crate::BaseVehicle,
    ) -> bool {
        false
    }
    /// `PolygonalMap2.instance.canStandAt(x, y, z, vehicle, false, false)`.
    fn can_stand_at(&self, _x: f32, _y: f32, _z: i32, _vehicle: &crate::BaseVehicle) -> bool {
        true
    }
    /// The zombie with this key as `SurroundVehicle` reads it (`None` when
    /// it is gone).
    fn surround_zombie(&self, _key: u64) -> Option<crate::surround::SurroundZombie> {
        None
    }
    /// `addToWorld`: `chunk.vehicles.add(this)` unless already there.
    fn add_vehicle_to_chunk(&self, _v: &crate::VehicleRef, _chunk: (i32, i32)) {}
    /// `update`: the vehicle moved from chunk `from` to `to`.
    fn vehicle_chunk_changed(&self, _v: &crate::VehicleRef, _from: (i32, i32), _to: (i32, i32)) {}
    /// `permanentlyRemove`: `chunk.vehicles.remove(this)`.
    fn remove_vehicle_from_chunk(&self, _v: &crate::VehicleRef, _chunk: (i32, i32)) {}
}

/// A flat world of loaded chunks without floors.
pub struct OpenWorld;

impl VehicleWorld for OpenWorld {
    fn square(&self, _x: i32, _y: i32, _z: i32) -> Option<SquareInfo> {
        Some(SquareInfo {
            chunk_has_refs: true,
            ..Default::default()
        })
    }
    fn is_valid_chunk(&self, _wx: i32, _wy: i32) -> bool {
        true
    }
    fn chunk_loaded(&self, _wx: i32, _wy: i32) -> bool {
        true
    }
}

/// Side effects the vehicle code has outside the vehicle (sounds, world
/// sounds, Lua events, music events). The default ignores them.
/// What `updateWorldLights` reads for the lightbar's two world lights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightbarState {
    /// The vehicle has a lightbar part.
    pub has_lightbar: bool,
    /// `lightbarLightsMode.isEnable() && getBatteryCharge() > 0`.
    pub enabled: bool,
    /// `lightbarLightsMode.getLightTexIndex()` (0: both lights off).
    pub light: i32,
}

pub trait VehicleEvents {
    fn event(&self, _what: &str) {}
    /// `VehiclesDB2.instance.removeVehicle(vehicle)` (`permanentlyRemove`).
    fn vehicle_removed_from_database(&self, _vehicle: &crate::BaseVehicle) {}
    /// `updateWorldLights()`: rendering places or removes the lightbar's
    /// world lights from this state.
    fn lightbar_world_lights(&self, _vehicle: &crate::BaseVehicle, _state: LightbarState) {}
    /// `getEmitter().playSoundImpl(name, null)`: the sound's handle (0 when
    /// sound is disabled, as with `DummySoundEmitter`).
    fn play_sound(&self, _name: &str) -> i64 {
        1
    }
}

struct NoEvents;
impl VehicleEvents for NoEvents {}

/// `InventoryItemFactory.CreateItem(fullType)` and the script values the
/// vehicle code reads from items.
pub trait ItemFactory {
    /// Creates an item (drawing its id from `Rand` like the original).
    fn create_item(&self, full_type: &str) -> Option<InventoryItem>;
    /// The item context weights and scripts are read through.
    fn context(&self) -> &dyn pz_inventory::ItemContext {
        &pz_inventory::api::Offline
    }
    /// `ZomboidRadio.GetChannelList(category)`.
    fn channel_lists(&self) -> &dyn pz_inventory::device::ChannelLists {
        &pz_inventory::device::NoChannels
    }
}

struct NoItems;
impl ItemFactory for NoItems {
    fn create_item(&self, _full_type: &str) -> Option<InventoryItem> {
        None
    }
}

thread_local! {
    static NOW_MS: Cell<i64> = const { Cell::new(0) };
    static TIME: Cell<GameTimeState> = Cell::new(GameTimeState::default());
    static SANDBOX: RefCell<SandboxVehicle> = RefCell::new(SandboxVehicle::default());
    static WORLD: RefCell<Rc<dyn VehicleWorld>> = RefCell::new(Rc::new(OpenWorld));
    static EVENTS: RefCell<Rc<dyn VehicleEvents>> = RefCell::new(Rc::new(NoEvents));
    static ITEMS: RefCell<Rc<dyn ItemFactory>> = RefCell::new(Rc::new(NoItems));
    static OFFSET: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
}

/// `System.currentTimeMillis()`.
pub fn now_ms() -> i64 {
    NOW_MS.with(|c| c.get())
}

/// Sets `System.currentTimeMillis()`.
pub fn set_now_ms(ms: i64) {
    NOW_MS.with(|c| c.set(ms));
}

/// The current `GameTime` values.
pub fn time() -> GameTimeState {
    TIME.with(|c| c.get())
}

pub fn set_time(t: GameTimeState) {
    TIME.with(|c| c.set(t));
}

/// `SandboxOptions.instance`.
pub fn sandbox() -> SandboxVehicle {
    SANDBOX.with(|s| s.borrow().clone())
}

pub fn set_sandbox(s: SandboxVehicle) {
    SANDBOX.with(|c| *c.borrow_mut() = s);
}

/// The installed world.
pub fn world() -> Rc<dyn VehicleWorld> {
    WORLD.with(|w| w.borrow().clone())
}

pub fn set_world(w: Rc<dyn VehicleWorld>) {
    WORLD.with(|c| *c.borrow_mut() = w);
}

pub fn events() -> Rc<dyn VehicleEvents> {
    EVENTS.with(|w| w.borrow().clone())
}

pub fn set_events(e: Rc<dyn VehicleEvents>) {
    EVENTS.with(|c| *c.borrow_mut() = e);
}

pub fn items() -> Rc<dyn ItemFactory> {
    ITEMS.with(|w| w.borrow().clone())
}

pub fn set_items(f: Rc<dyn ItemFactory>) {
    ITEMS.with(|c| *c.borrow_mut() = f);
}

/// `WorldSimulation.instance.offsetX/offsetY`.
pub fn offset() -> (f32, f32) {
    OFFSET.with(|c| c.get())
}

pub fn set_offset(x: f32, y: f32) {
    OFFSET.with(|c| c.set((x, y)));
}

/// `GameTime.minHours`/`checkHours`: a stored time later than now is reset
/// to now.
pub fn check_hours(hours: f32, world_age_hours: f32) -> f32 {
    if hours < 0.0 {
        world_age_hours
    } else {
        hours.min(world_age_hours)
    }
}

/// `GameTime.minHours(double, double)`.
pub fn min_hours(hours: f64, world_age_hours: f64) -> f64 {
    hours.min(world_age_hours)
}

/// `VehiclesDB2`'s main-thread id sets, shared between the vehicle runtime
/// (`addToWorld` / `removeFromWorld`) and the streamer
/// (`StreamCtx::vehicle_ids`, borrowed while it runs).
pub type VehicleIdsRef = Rc<RefCell<pz_world::save::vehicles_db::VehicleIds>>;

thread_local! {
    static VEHICLE_IDS: RefCell<Option<VehicleIdsRef>> = const { RefCell::new(None) };
}

/// Installs the ids `VehiclesDB2.init` made (`None`: no store, ids are not
/// tracked and new vehicles keep `sqlId` -1).
pub fn set_vehicle_ids(ids: Option<VehicleIdsRef>) {
    VEHICLE_IDS.with(|v| *v.borrow_mut() = ids);
}

/// The installed ids.
pub fn vehicle_ids() -> Option<VehicleIdsRef> {
    VEHICLE_IDS.with(|v| v.borrow().clone())
}
