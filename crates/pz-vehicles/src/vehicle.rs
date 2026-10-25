//! `BaseVehicle` (with `VehicleParts`, `VehicleAlarm` and the lightbar
//! modes): script binding, parts and their Lua functions, the engine,
//! keys, alarm, lights, mass and `Bullet` parameters, crash damage and the
//! per-frame `update`.
//!
//! What the original does through rendering, sound emitters, the network
//! and the animal/zombie code is reported as events (see
//! [`crate::env::VehicleEvents`]) or left out; each such place says so.

use crate::controller::CarController;
use crate::engine::{EngineReason, EngineState, Transmission, UpdateLimit, VehicleEngine};
use crate::env;
use crate::items::{self, ItemRef};
use crate::jmath::{self, fastfloor, Quaternionf, Transform, Vector3f};
use crate::lua::{self, LuaArg};
use crate::part::{
    number_by_condition, PartItem, PartRef, VehicleDoor, VehiclePart, VehicleWindow,
};
use crate::physics::{Bullet, WHEEL_PARAMS};
use crate::poly::VehiclePoly;
use crate::rand;
use pz_inventory::ItemContainer;
use pz_scripts::manager::ScriptManager;
use pz_scripts::objects::vehicles::{Model, VehicleScript};
use pz_scripts::script_type::ScriptType;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::rc::{Rc, Weak};

pub type VehicleRef = Rc<BaseVehicle>;
pub type ScriptRc = Rc<RefCell<VehicleScript>>;

/// `BaseVehicle.PHYSICS_Z_SCALE * 3`: physics units per level.
pub const LEVEL_HEIGHT: f32 = 2.449_49;

/// A character in a seat (`IsoGameCharacter` as the vehicle code sees it).
pub trait VehicleCharacter {
    /// `getMoodles().getMoodleLevel(MoodleType.DRUNK)`.
    fn drunk_level(&self) -> i32 {
        0
    }
    /// `hasTrait(trait)` (`SPEED_DEMON`, `SUNDAY_DRIVER`, ...).
    fn has_trait(&self, _trait_name: &str) -> bool {
        false
    }
    /// `getInventory().haveThisKeyId(keyId)`.
    fn have_key(&self, _key_id: i32) -> Option<ItemRef> {
        None
    }
    /// Takes a key out of the inventory (or its key ring).
    fn remove_item(&self, _item: &ItemRef) {}
    /// `getInventory().addItem(item)`.
    fn add_item(&self, _item: ItemRef) {}
    /// `isGodMod()`.
    fn is_god_mod(&self) -> bool {
        false
    }
    /// `isBlockMovement()`.
    fn is_block_movement(&self) -> bool {
        false
    }
    /// `setX/setY/setZ` while seated.
    fn set_position(&self, _x: f32, _y: f32, _z: f32) {}
    /// `instanceof IsoPlayer` (one of `IsoPlayer.players`).
    fn is_player(&self) -> bool {
        false
    }
    /// `exit`: `setVehicle(null)`, `savedVehicleSeat = -1`,
    /// `setCollidable(true)`.
    fn exited_vehicle(&self) {}
}

pub type CharRef = Rc<dyn VehicleCharacter>;

fn same_char(a: &CharRef, b: &CharRef) -> bool {
    std::ptr::eq(Rc::as_ptr(a) as *const (), Rc::as_ptr(b) as *const ())
}

/// `BaseVehicle.Passenger`.
#[derive(Clone, Default)]
pub struct Passenger {
    pub character: Option<CharRef>,
    pub offset: Vector3f,
}

/// `BaseVehicle.WheelInfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelInfo {
    pub steering: f32,
    pub rotation: f32,
    pub skid_info: f32,
    pub suspension_length: f32,
}

/// `BaseVehicle.VehicleImpulse`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleImpulse {
    pub impulse: Vector3f,
    pub rel_pos: Vector3f,
    pub enable: bool,
    pub applied: bool,
}

/// `BaseVehicle.Authorization`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Authorization {
    #[default]
    Server,
    LocalCollide,
    RemoteCollide,
    Local,
    Remote,
}

/// `VehicleAlarm`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleAlarm {
    pub start_time: f64,
    pub accumulator: f32,
    pub chosen_sound: Option<String>,
    pub sound_on: bool,
    pub lights_on: bool,
    pub world_sound_update_limit: UpdateLimit,
}

/// `LightbarLightsMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightbarLightsMode {
    pub start_time: i64,
    pub light: i32,
    pub mode: i32,
}

impl LightbarLightsMode {
    /// `set(v)`.
    pub fn set(&mut self, v: i32) {
        if v > 3 {
            self.mode = 3;
            return;
        }
        if v < 0 {
            self.mode = 0;
            return;
        }
        self.mode = v;
        if self.mode != 0 {
            self.start_time = env::now_ms();
        }
    }

    /// `update()`.
    pub fn update(&mut self) {
        let d = env::now_ms() - self.start_time;
        self.light = match self.mode {
            1 => {
                let d = d % 1000;
                if d < 50 {
                    0
                } else if d < 450 {
                    1
                } else if d < 550 {
                    0
                } else if d < 950 {
                    2
                } else {
                    0
                }
            }
            2 => {
                let d = d % 1000;
                if d < 50 {
                    0
                } else if d < 250 {
                    1
                } else if d < 300 {
                    0
                } else if d < 500 {
                    1
                } else if d < 550 {
                    0
                } else if d < 750 {
                    2
                } else if d < 800 {
                    0
                } else {
                    2
                }
            }
            3 => {
                let d = d % 300;
                if d < 25 {
                    0
                } else if d < 125 {
                    1
                } else if d < 175 {
                    0
                } else if d < 275 {
                    2
                } else {
                    0
                }
            }
            _ => 0,
        };
    }

    pub fn is_enable(&self) -> bool {
        self.mode != 0
    }
}

/// `LightbarSirenMode.set`.
pub fn clamp_siren_mode(v: i32) -> i32 {
    v.clamp(0, 3)
}

/// A visible part model (`BaseVehicle.ModelInfo`, without the render
/// state).
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub part: Weak<VehiclePart>,
    /// Index of the model in the part's script models.
    pub model_index: usize,
    pub model_id: Option<String>,
    pub wheel_index: i32,
}

/// `BaseVehicle` state other than the parts (see [`BaseVehicle::st`]).
#[derive(Clone)]
pub struct VehicleState {
    // IsoMovingObject
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `square` / `current` (the square coordinates).
    pub square: Option<(i32, i32, i32)>,
    pub current: Option<(i32, i32, i32)>,
    pub key_id: i32,
    /// `chunk` (wx, wy).
    pub chunk: Option<(i32, i32)>,
    pub poly_dirty: bool,
    pub poly_garage_check: bool,
    pub radius_reduction_in_garage: f32,
    pub vehicle_id: i16,
    pub sql_id: i32,
    pub throttle: f32,
    pub transmission_number: Transmission,
    pub transmission_change_time: UpdateLimit,
    pub has_extend_offset: bool,
    pub has_extend_offset_exiting: bool,
    pub saved_physics_z: f32,
    pub saved_rot: Quaternionf,
    pub jni_transform: Transform,
    pub jni_speed: f32,
    pub jni_is_collide: bool,
    pub jni_linear_velocity: Vector3f,
    pub last_linear_velocity: Vector3f,
    pub net_player_authorization: Authorization,
    pub front_end_durability: i32,
    pub rear_end_durability: i32,
    pub rust: f32,
    pub color_hue: f32,
    pub color_saturation: f32,
    pub color_value: f32,
    pub current_front_end_durability: i32,
    pub current_rear_end_durability: i32,
    pub collide_x: f32,
    pub collide_y: f32,
    pub wheel_info: [WheelInfo; 4],
    pub ram_sound: i64,
    pub ram_sound_time: i64,
    pub engine_rpm: Option<[Option<pz_scripts::objects::models::EngineRPMData>; 8]>,
    pub headlights_on: bool,
    pub stoplights_on: bool,
    pub window_lights_on: bool,
    pub alarm: VehicleAlarm,
    pub sound_horn_on: bool,
    pub sound_back_move_on: bool,
    pub previously_entered: bool,
    pub previously_moved: bool,
    pub lightbar_lights_mode: LightbarLightsMode,
    pub lightbar_siren_mode: i32,
    pub passengers: Vec<Passenger>,
    pub script_name: String,
    pub script: Option<ScriptRc>,
    pub models: Vec<ModelInfo>,
    pub created_model: bool,
    pub skin_index: i32,
    pub ctrl: Option<CarController>,
    pub created: bool,
    pub poly: VehiclePoly,
    pub poly_plus_radius: VehiclePoly,
    pub shadow_coord: VehiclePoly,
    pub do_damage_overlay: bool,
    pub loaded: bool,
    pub update_flags: i16,
    pub limit_crash: UpdateLimit,
    pub added_to_world: bool,
    pub removed_from_world: bool,
    pub poly_plus_radius_min_x: f32,
    pub poly_plus_radius_min_y: f32,
    pub poly_plus_radius_max_x: f32,
    pub poly_plus_radius_max_y: f32,
    pub max_speed: f32,
    pub key_is_on_door: bool,
    pub hotwired: bool,
    pub hotwired_broken: bool,
    pub keys_in_ignition: bool,
    /// `ignitionSwitch.getItems()`.
    pub ignition_switch: Vec<ItemRef>,
    pub keys_container_id: i32,
    /// `choosenParts` (a Java `HashMap`; Lua reads and writes it).
    pub choosen_parts: Vec<(String, String)>,
    pub vehicle_type: String,
    pub respawn_zone: String,
    pub mass: f32,
    pub initial_mass: f32,
    pub braking_force: f32,
    pub base_quality: f32,
    pub current_steering: f32,
    pub is_braking: bool,
    pub mechanical_id: i32,
    pub need_parts_update: bool,
    pub alarmed: bool,
    pub siren_start_time: f64,
    /// While a [`crate::virtual_vehicle::VirtualVehicle`] updates the
    /// vehicle: the record's headlights, which the parts read
    /// (`getOwner().getHeadlightsOn()`) and the alarm's light flashes
    /// switch instead of the vehicle's.
    pub meta_headlights: Option<bool>,
    pub mechanic_ui_open: bool,
    pub is_good_car: bool,
    pub current_key: Option<ItemRef>,
    pub do_color: bool,
    pub breaking_slow_factor: f32,
    pub key_spawned: i8,
    pub brake_between_updates_speed: f32,
    pub physic_active_check: i64,
    pub constraint_changed_time: i64,
    pub specific_distribution_id: Option<String>,
    pub add_thump_world_sound: bool,
    /// `surroundVehicle`.
    pub surround: crate::surround::SurroundVehicle,
    pub regulator: bool,
    pub regulator_speed: f32,
    /// `bloodIntensity` (insertion order; saved in `HashMap` order).
    pub blood_intensity: Vec<(String, i8)>,
    pub vehicle_towing: Option<Weak<BaseVehicle>>,
    pub vehicle_towed_by: Option<Weak<BaseVehicle>>,
    pub constraint_towing: i32,
    pub begin_attach_trailer_ms: i64,
    pub vehicle_towing_id: i32,
    pub vehicle_towed_by_id: i32,
    pub tow_attachment_self: Option<String>,
    pub tow_attachment_other: Option<String>,
    pub row_constraint_z_offset: f32,
    pub is_active: bool,
    pub is_static: bool,
    /// The trailer's animals as saved (`IsoAnimal` is not ported).
    pub animals: Vec<u8>,
    pub key_spawn_chanced_d100: f32,
    pub time_since_last_auth: f32,
    pub disable_simulation_due_to_lack_of_surrounding_chunks: bool,
    pub desire_physics_active: bool,
    pub impulse_from_server: VehicleImpulse,
    pub impulses_from_squished_bodies: [Option<VehicleImpulse>; 4],
    pub impulses_from_hit_objects: Vec<VehicleImpulse>,
    pub create_physics_recursion: i32,
    /// The `IsoObject`/`IsoMovingObject` fields `save` writes that the
    /// runtime does not otherwise keep (`offsetX`, `offsetY`, `dir`, the
    /// mod-data `table`), as `load` read them; its position is not used.
    /// `None` for a vehicle made by `new BaseVehicle` (the constructor's
    /// `32 * Core.tileScale`, `96 * Core.tileScale`, `N`, no table).
    pub iso_object: Option<pz_world::save::visual::MovingBase>,
}

/// `BaseVehicle`.
pub struct BaseVehicle {
    me: Weak<BaseVehicle>,
    st: RefCell<VehicleState>,
    /// `parts` (`VehicleParts.parts`).
    parts: RefCell<Vec<PartRef>>,
    /// `lights`: the parts with a light, after `createPhysics`.
    lights: RefCell<Vec<PartRef>>,
    missing_engine_part: RefCell<Option<PartRef>>,
    pub(crate) lua: RefCell<Option<Weak<dyn pz_kahlua::value::LuaObject>>>,
    in_update: Cell<bool>,
}

impl std::fmt::Debug for BaseVehicle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.st();
        write!(f, "BaseVehicle({} id={})", st.script_name, st.vehicle_id)
    }
}

thread_local! {
    static SCRIPTS: RefCell<Option<Rc<ScriptManager>>> = const { RefCell::new(None) };
}

/// Installs the `ScriptManager` vehicles take their scripts from.
pub fn set_script_manager(sm: Rc<ScriptManager>) {
    SCRIPTS.with(|s| *s.borrow_mut() = Some(sm));
}

/// The installed `ScriptManager`.
pub fn script_manager() -> Option<Rc<ScriptManager>> {
    SCRIPTS.with(|s| s.borrow().clone())
}

/// `ScriptManager.instance.getVehicle(name)`.
pub fn find_vehicle_script(name: &str) -> Option<ScriptRc> {
    script_manager()?.get::<VehicleScript>(ScriptType::Vehicle, name)
}

/// `ScriptManager.instance.getAllVehicleScripts()`.
pub fn all_vehicle_scripts() -> Vec<ScriptRc> {
    script_manager()
        .map(|sm| sm.get_all::<VehicleScript>(ScriptType::Vehicle))
        .unwrap_or_default()
}

impl BaseVehicle {
    /// `new BaseVehicle(cell)`: draws the key id and the rust from `Rand`.
    pub fn new() -> VehicleRef {
        let now_limits = |ms| UpdateLimit::new(ms);
        let key_spawn = env::sandbox().key_loot_new as f32 * 25.0;
        let st = VehicleState {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            square: None,
            current: None,
            key_id: -1,
            chunk: None,
            poly_dirty: true,
            poly_garage_check: true,
            radius_reduction_in_garage: 0.0,
            vehicle_id: -1,
            sql_id: -1,
            throttle: 0.0,
            transmission_number: Transmission::N,
            transmission_change_time: now_limits(1000),
            has_extend_offset: true,
            has_extend_offset_exiting: false,
            saved_physics_z: f32::NAN,
            saved_rot: Quaternionf::default(),
            jni_transform: Transform::default(),
            jni_speed: 0.0,
            jni_is_collide: false,
            jni_linear_velocity: Vector3f::default(),
            last_linear_velocity: Vector3f::default(),
            net_player_authorization: Authorization::Server,
            front_end_durability: 100,
            rear_end_durability: 100,
            rust: 0.0,
            color_hue: 0.0,
            color_saturation: 0.0,
            color_value: 0.0,
            current_front_end_durability: 100,
            current_rear_end_durability: 100,
            collide_x: -1.0,
            collide_y: -1.0,
            wheel_info: [WheelInfo::default(); 4],
            ram_sound: 0,
            ram_sound_time: 0,
            engine_rpm: None,
            headlights_on: false,
            stoplights_on: false,
            window_lights_on: false,
            alarm: VehicleAlarm {
                start_time: 0.0,
                accumulator: 0.0,
                chosen_sound: None,
                sound_on: false,
                lights_on: false,
                world_sound_update_limit: now_limits(1000),
            },
            sound_horn_on: false,
            sound_back_move_on: false,
            previously_entered: false,
            previously_moved: false,
            lightbar_lights_mode: LightbarLightsMode::default(),
            lightbar_siren_mode: 0,
            passengers: vec![Passenger::default()],
            script_name: "Base.PickUpTruck".into(),
            script: None,
            models: Vec::new(),
            created_model: false,
            skin_index: -1,
            ctrl: None,
            created: false,
            poly: VehiclePoly::default(),
            poly_plus_radius: VehiclePoly::default(),
            shadow_coord: VehiclePoly::default(),
            do_damage_overlay: false,
            loaded: false,
            update_flags: 0,
            limit_crash: now_limits(600),
            added_to_world: false,
            removed_from_world: false,
            poly_plus_radius_min_x: -123.0,
            poly_plus_radius_min_y: 0.0,
            poly_plus_radius_max_x: 0.0,
            poly_plus_radius_max_y: 0.0,
            max_speed: 0.0,
            key_is_on_door: false,
            hotwired: false,
            hotwired_broken: false,
            keys_in_ignition: false,
            ignition_switch: Vec::new(),
            keys_container_id: -1,
            choosen_parts: Vec::new(),
            vehicle_type: String::new(),
            respawn_zone: String::new(),
            mass: 0.0,
            initial_mass: 0.0,
            braking_force: 0.0,
            base_quality: 0.0,
            current_steering: 0.0,
            is_braking: false,
            mechanical_id: 0,
            need_parts_update: false,
            alarmed: false,
            siren_start_time: 0.0,
            meta_headlights: None,
            mechanic_ui_open: false,
            is_good_car: false,
            current_key: None,
            do_color: true,
            breaking_slow_factor: 0.0,
            key_spawned: 0,
            brake_between_updates_speed: 0.0,
            physic_active_check: -1,
            constraint_changed_time: -1,
            specific_distribution_id: None,
            add_thump_world_sound: false,
            surround: Default::default(),
            regulator: false,
            regulator_speed: 0.0,
            blood_intensity: Vec::new(),
            vehicle_towing: None,
            vehicle_towed_by: None,
            constraint_towing: -1,
            begin_attach_trailer_ms: 0,
            vehicle_towing_id: -1,
            vehicle_towed_by_id: -1,
            tow_attachment_self: None,
            tow_attachment_other: None,
            row_constraint_z_offset: 0.0,
            is_active: false,
            is_static: false,
            animals: vec![0],
            key_spawn_chanced_d100: key_spawn,
            time_since_last_auth: 30.0,
            disable_simulation_due_to_lack_of_surrounding_chunks: false,
            desire_physics_active: false,
            impulse_from_server: VehicleImpulse::default(),
            impulses_from_squished_bodies: [None; 4],
            impulses_from_hit_objects: Vec::new(),
            create_physics_recursion: 0,
            iso_object: None,
        };
        let v = Rc::new_cyclic(|me| BaseVehicle {
            me: me.clone(),
            st: RefCell::new(st),
            parts: RefCell::new(Vec::new()),
            lights: RefCell::new(Vec::new()),
            missing_engine_part: RefCell::new(None),
            lua: RefCell::new(None),
            in_update: Cell::new(false),
        });
        // setKeyId(Rand.Next(100000000)); rust = Rand.Next(0, 2).
        let key = rand::next(100_000_000);
        let rust = rand::next_range(0, 2) as f32;
        {
            let mut st = v.st_mut();
            st.key_id = key;
            st.rust = rust;
        }
        v
    }

    /// This vehicle as a shared reference.
    pub fn rc(&self) -> VehicleRef {
        self.me.upgrade().expect("vehicle dropped")
    }

    pub fn st(&self) -> Ref<'_, VehicleState> {
        self.st.borrow()
    }

    pub fn st_mut(&self) -> RefMut<'_, VehicleState> {
        self.st.borrow_mut()
    }

    // ---- simple accessors --------------------------------------------------

    /// `getId()` (`vehicleId`).
    pub fn id(&self) -> i16 {
        self.st().vehicle_id
    }
    pub fn x(&self) -> f32 {
        self.st().x
    }
    pub fn y(&self) -> f32 {
        self.st().y
    }
    pub fn z(&self) -> f32 {
        self.st().z
    }
    pub fn xi(&self) -> i32 {
        fastfloor(self.x())
    }
    pub fn yi(&self) -> i32 {
        fastfloor(self.y())
    }
    pub fn zi(&self) -> i32 {
        fastfloor(self.z())
    }
    pub fn set_position(&self, x: f32, y: f32, z: f32) {
        let mut st = self.st_mut();
        st.x = x;
        st.y = y;
        st.z = z;
    }
    /// `getScript()`.
    pub fn script(&self) -> Option<ScriptRc> {
        self.st().script.clone()
    }
    /// `getScriptName()`.
    pub fn script_name(&self) -> String {
        self.st().script_name.clone()
    }
    /// `setScriptName(name)`.
    pub fn set_script_name(&self, name: &str) {
        self.st_mut().script_name = name.to_string();
    }
    pub fn key_id(&self) -> i32 {
        self.st().key_id
    }
    pub fn set_key_id(&self, id: i32) {
        self.st_mut().key_id = id;
    }
    /// `getVehicleType()`.
    pub fn vehicle_type(&self) -> String {
        self.st().vehicle_type.clone()
    }
    pub fn set_vehicle_type(&self, t: &str) {
        self.st_mut().vehicle_type = t.to_string();
    }
    pub fn is_good_car(&self) -> bool {
        self.st().is_good_car
    }
    pub fn set_good_car(&self, b: bool) {
        self.st_mut().is_good_car = b;
    }
    pub fn is_alarmed(&self) -> bool {
        self.st().alarmed
    }
    /// `setAlarmed(alarmed)`.
    pub fn set_alarmed(&self, alarmed: bool) {
        let mut st = self.st_mut();
        st.alarmed = alarmed;
        if alarmed {
            st.previously_entered = false;
        }
    }
    pub fn mass(&self) -> f32 {
        let m = self.st().mass;
        if m < 0.0 {
            1.0
        } else {
            m
        }
    }
    pub fn max_speed(&self) -> f32 {
        self.st().max_speed
    }
    pub fn braking_force(&self) -> f32 {
        self.st().braking_force
    }
    pub fn transmission(&self) -> Transmission {
        self.st().transmission_number
    }
    /// `getTransmissionNumber()`.
    pub fn transmission_number(&self) -> i32 {
        self.st().transmission_number.index()
    }
    pub fn set_transmission(&self, t: Transmission) {
        self.st_mut().transmission_number = t;
    }
    /// `getHeadlightsOn()`; while a virtual record updates the vehicle
    /// (the parts' owner is the record), the record's headlights.
    pub fn headlights_on(&self) -> bool {
        let st = self.st();
        st.meta_headlights.unwrap_or(st.headlights_on)
    }
    /// `setHeadlightsOn(on)`.
    pub fn set_headlights_on(&self, on: bool) {
        if self.st().headlights_on == on {
            return;
        }
        self.st_mut().headlights_on = on;
        env::events().event(if on {
            "playSound VehicleHeadlightsOn"
        } else {
            "playSound VehicleHeadlightsOff"
        });
    }
    pub fn stoplights_on(&self) -> bool {
        self.st().stoplights_on
    }
    pub fn set_stoplights_on(&self, on: bool) {
        self.st_mut().stoplights_on = on;
    }
    pub fn is_regulator(&self) -> bool {
        self.st().regulator
    }
    pub fn set_regulator(&self, b: bool) {
        self.st_mut().regulator = b;
    }
    pub fn regulator_speed(&self) -> f32 {
        self.st().regulator_speed
    }
    pub fn set_regulator_speed(&self, s: f32) {
        self.st_mut().regulator_speed = s;
    }
    /// `getCurrentSpeedForRegulator()`.
    pub fn current_speed_for_regulator(&self) -> f32 {
        (5.0 * ((self.current_speed_kmh() / 5.0) as f64).floor()).max(5.0) as f32
    }
    pub fn need_parts_update(&self) -> bool {
        self.st().need_parts_update
    }
    pub fn set_need_parts_update(&self, b: bool) {
        self.st_mut().need_parts_update = b;
    }
    pub fn set_current_steering(&self, s: f32) {
        self.st_mut().current_steering = s;
    }
    pub fn current_steering(&self) -> f32 {
        self.st().current_steering
    }
    /// `setBraking(isBraking)`.
    pub fn set_braking(&self, braking: bool) {
        let speed = self.current_absolute_speed_kmh();
        let mut st = self.st_mut();
        st.is_braking = braking;
        if braking && st.brake_between_updates_speed == 0.0 {
            st.brake_between_updates_speed = speed;
        }
    }
    pub fn brake_speed_between_update(&self) -> f32 {
        self.st().brake_between_updates_speed
    }
    pub fn is_hotwired(&self) -> bool {
        self.st().hotwired
    }
    pub fn set_hotwired(&self, b: bool) {
        self.st_mut().hotwired = b;
    }
    pub fn is_hotwired_broken(&self) -> bool {
        self.st().hotwired_broken
    }
    pub fn set_hotwired_broken(&self, b: bool) {
        self.st_mut().hotwired_broken = b;
    }
    pub fn set_previously_moved(&self, b: bool) {
        self.st_mut().previously_moved = b;
    }
    /// `isKeysInIgnition()`.
    pub fn is_keys_in_ignition(&self) -> bool {
        !self.st().ignition_switch.is_empty()
    }
    /// `isBurnt()`.
    pub fn is_burnt(&self) -> bool {
        self.st().script_name.contains("Burnt")
    }
    pub fn is_smashed(&self) -> bool {
        self.st().script_name.contains("Smashed")
    }
    pub fn lightbar_lights_mode(&self) -> i32 {
        self.st().lightbar_lights_mode.mode
    }
    /// `setLightbarLightsMode(mode)`.
    pub fn set_lightbar_lights_mode(&self, mode: i32) {
        self.st_mut().lightbar_lights_mode.set(mode);
    }
    pub fn lightbar_siren_mode(&self) -> i32 {
        self.st().lightbar_siren_mode
    }
    /// `setLightbarSirenMode(mode)`.
    pub fn set_lightbar_siren_mode(&self, mode: i32) {
        self.st_mut().lightbar_siren_mode = clamp_siren_mode(mode);
    }
    /// `hasLightbar()` / `hasSiren()`.
    pub fn has_lightbar(&self) -> bool {
        self.script().is_some_and(|s| s.borrow().lightbar().enable)
    }
    /// `hasAlarm()`.
    pub fn has_alarm(&self) -> bool {
        self.script()
            .is_some_and(|s| s.borrow().sounds().alarm_enable)
    }
    /// `sirenShutoffTimeExpired()`.
    pub fn siren_shutoff_time_expired(&self) -> bool {
        let shutoff = env::sandbox().siren_shutoff_hours;
        if shutoff <= 0.0 {
            return false;
        }
        let world_age = env::time().world_age_hours;
        let start = env::min_hours(self.st().siren_start_time, world_age);
        self.st_mut().siren_start_time = start;
        start + shutoff < world_age
    }
    /// `getMaxPassengers()`.
    pub fn max_passengers(&self) -> i32 {
        self.st().passengers.len() as i32
    }
    /// `getCharacter(seat)`.
    pub fn character(&self, seat: i32) -> Option<CharRef> {
        if seat < 0 {
            return None;
        }
        self.st()
            .passengers
            .get(seat as usize)
            .and_then(|p| p.character.clone())
    }
    /// `getDriver()`.
    pub fn driver(&self) -> Option<CharRef> {
        self.character(0)
    }
    /// `getVehicleTowing()`.
    pub fn vehicle_towing(&self) -> Option<VehicleRef> {
        self.st().vehicle_towing.as_ref().and_then(Weak::upgrade)
    }
    /// `getVehicleTowedBy()`.
    pub fn vehicle_towed_by(&self) -> Option<VehicleRef> {
        self.st().vehicle_towed_by.as_ref().and_then(Weak::upgrade)
    }
    /// `getDriverRegardlessOfTow()`.
    pub fn driver_regardless_of_tow(&self) -> Option<CharRef> {
        if let Some(d) = self.driver() {
            return Some(d);
        }
        self.vehicle_towed_by().and_then(|v| v.driver())
    }
    /// `getSeat(chr)`.
    pub fn seat_of(&self, chr: &CharRef) -> i32 {
        let st = self.st();
        for (i, p) in st.passengers.iter().enumerate() {
            if p.character.as_ref().is_some_and(|c| same_char(c, chr)) {
                return i as i32;
            }
        }
        -1
    }
    /// `setPassenger(seat, chr, offset)`.
    pub fn set_passenger(&self, seat: i32, chr: Option<CharRef>, offset: Vector3f) -> bool {
        if seat < 0 || seat >= self.max_passengers() {
            return false;
        }
        if seat == 0 {
            self.set_need_parts_update(true);
        }
        let mut st = self.st_mut();
        st.passengers[seat as usize].character = chr;
        st.passengers[seat as usize].offset = offset;
        true
    }
    /// `clearPassenger(seat)`.
    pub fn clear_passenger(&self, seat: i32) -> bool {
        if seat >= 0 && seat < self.max_passengers() {
            let mut st = self.st_mut();
            st.passengers[seat as usize].character = None;
            st.passengers[seat as usize].offset = Vector3f::default();
            return true;
        }
        false
    }

    // ---- parts ------------------------------------------------------------

    /// `getParts()` (a snapshot of the list).
    pub fn parts(&self) -> Vec<PartRef> {
        self.parts.borrow().clone()
    }
    /// `getPartCount()`.
    pub fn part_count(&self) -> usize {
        self.parts.borrow().len()
    }
    /// `getPartByIndex(i)`.
    pub fn part_by_index(&self, i: i32) -> Option<PartRef> {
        if i < 0 {
            return None;
        }
        self.parts.borrow().get(i as usize).cloned()
    }
    /// `getPartById(id)`: the first part with that id.
    pub fn part_by_id(&self, id: &str) -> Option<PartRef> {
        self.parts.borrow().iter().find(|p| p.id_is(id)).cloned()
    }
    /// The shared reference of a part of this vehicle.
    pub fn part_ref(&self, part: &VehiclePart) -> Option<PartRef> {
        if let Some(p) = self
            .parts
            .borrow()
            .iter()
            .find(|p| std::ptr::eq(p.as_ref(), part))
        {
            return Some(p.clone());
        }
        self.missing_engine_part
            .borrow()
            .as_ref()
            .filter(|p| std::ptr::eq(p.as_ref(), part))
            .cloned()
    }
    /// `getPartIndex(id)` (by script part id).
    pub fn part_index(&self, id: &str) -> i32 {
        self.parts
            .borrow()
            .iter()
            .position(|p| {
                p.st()
                    .script_part
                    .as_ref()
                    .is_some_and(|sp| sp.id.as_deref() == Some(id))
            })
            .map_or(-1, |i| i as i32)
    }
    /// `getBattery()`.
    pub fn battery(&self) -> Option<PartRef> {
        self.part_by_id("Battery")
    }
    /// `getEngine()` (the `Engine` part).
    pub fn engine_part(&self) -> Option<PartRef> {
        self.part_by_id("Engine")
    }
    /// `getHeater()`.
    pub fn heater(&self) -> Option<PartRef> {
        self.part_by_id("Heater")
    }
    /// `getGasTank()`.
    pub fn gas_tank(&self) -> Option<PartRef> {
        self.part_by_id("GasTank")
    }
    /// `getBatteryCharge()`.
    pub fn battery_charge(&self) -> f32 {
        let Some(b) = self.battery() else { return 0.0 };
        match b.item() {
            Some(i) if items::is_drainable(&i.borrow()) => items::current_uses_float(&i.borrow()),
            _ => 0.0,
        }
    }
    /// `getGasRemaining()`.
    pub fn gas_remaining(&self) -> f32 {
        self.gas_tank()
            .map_or(0.0, |g| g.container_content_amount())
    }
    /// `hasEnoughGasToRun()`: `StringLib.roundToPrecision(gas, 3) > 0`.
    pub fn has_enough_gas_to_run(&self) -> bool {
        let gas = self.gas_remaining() as f64;
        let r = (gas * 1000.0 + 0.5).floor() / 1000.0;
        r > 0.0
    }
    /// `getEngineCondition()`.
    pub fn engine_condition(&self) -> i32 {
        self.engine_part().map_or(0, |p| p.condition())
    }
    /// `windowsOpen()`.
    pub fn windows_open(&self) -> i32 {
        self.parts
            .borrow()
            .iter()
            .filter(|p| p.st().window.is_some_and(|w| w.open))
            .count() as i32
    }
    /// `getPassengerDoor(seat)`.
    pub fn passenger_door(&self, seat: i32) -> Option<PartRef> {
        let door = self.script_passenger(seat, |p| p.door.clone())??;
        self.part_by_id(&door)
    }
    /// `getPassengerDoor2(seat)`.
    pub fn passenger_door2(&self, seat: i32) -> Option<PartRef> {
        let door = self.script_passenger(seat, |p| p.door2.clone())??;
        self.part_by_id(&door)
    }
    /// `getPassengerArea(seat)`.
    pub fn passenger_area(&self, seat: i32) -> Option<String> {
        self.script_passenger(seat, |p| p.area.clone())?
    }
    /// `getScriptPassenger(seat)` mapped through `f`.
    pub fn script_passenger<R>(
        &self,
        seat: i32,
        f: impl FnOnce(&pz_scripts::objects::vehicles::Passenger) -> R,
    ) -> Option<R> {
        let script = self.script()?;
        let s = script.borrow();
        if seat < 0 || seat as usize >= s.passenger_count() {
            return None;
        }
        Some(f(s.passenger(seat as usize)))
    }
    /// `getPartForSeatContainer(seat)`.
    pub fn part_for_seat_container(&self, seat: i32) -> Option<PartRef> {
        if self.script().is_none() || seat < 0 || seat >= self.max_passengers() {
            return None;
        }
        self.parts
            .borrow()
            .iter()
            .find(|p| p.container_seat_number() == seat)
            .cloned()
    }
    /// `getLightCount()`.
    pub fn light_count(&self) -> usize {
        self.lights.borrow().len()
    }
    /// `getLightByIndex(i)`.
    pub fn light_by_index(&self, i: i32) -> Option<PartRef> {
        if i < 0 {
            return None;
        }
        self.lights.borrow().get(i as usize).cloned()
    }

    /// `VehicleParts.add(part)`.
    pub fn add_part(&self, part: PartRef) {
        self.parts.borrow_mut().push(part);
    }

    /// `VehicleParts.setScript(script)`.
    fn parts_set_script(&self, script: Option<&ScriptRc>) {
        let Some(script) = script else {
            self.parts.borrow_mut().clear();
            return;
        };
        let old: Vec<PartRef> = std::mem::take(&mut *self.parts.borrow_mut());
        let script_parts: Vec<pz_scripts::objects::vehicles::Part> = {
            let s = script.borrow();
            (0..s.part_count()).map(|i| s.part(i).clone()).collect()
        };
        for sp in &script_parts {
            let sp_id = sp.id.clone().unwrap_or_default();
            let mut found = None;
            for o in &old {
                let ost = o.st();
                if ost
                    .script_part
                    .as_ref()
                    .is_some_and(|p| p.id.as_deref() == Some(sp_id.as_str()))
                {
                    found = Some(o.clone());
                    break;
                }
                if ost.part_id.as_deref() == Some(sp_id.as_str()) {
                    found = Some(o.clone());
                    break;
                }
            }
            let part = found.unwrap_or_else(|| VehiclePart::new_weak(self.me.clone()));
            {
                let mut pst = part.st_mut();
                pst.script_part = Some(Rc::new(sp.clone()));
                pst.category = sp.category.clone();
                pst.specific_item = sp.specific_item;
                if sp.durability() > 0.0 {
                    pst.durability = sp.durability();
                }
                match &sp.container {
                    Some(c) if c.content_type.is_none() => {
                        let cont = pst.container.get_or_insert_with(|| {
                            let mut c = ItemContainer::new(Some(&sp_id));
                            c.id = 0;
                            c
                        });
                        cont.capacity = c.capacity;
                    }
                    _ => pst.container = None,
                }
                if sp.door.is_none() {
                    pst.door = None;
                } else if pst.door.is_none() {
                    pst.door = Some(VehicleDoor::default());
                }
                if sp_id == "Engine" {
                    if pst.engine.is_none() {
                        pst.engine = Some(VehicleEngine::default());
                    }
                } else {
                    pst.engine = None;
                }
                match &sp.window {
                    None => pst.window = None,
                    Some(w) => match pst.window.as_mut() {
                        None => {
                            pst.window = Some(VehicleWindow {
                                openable: w.openable,
                                open: false,
                                open_delta: 0.0,
                            })
                        }
                        Some(win) => win.openable = w.openable,
                    },
                }
                pst.parent = None;
                pst.children.clear();
            }
            self.parts.borrow_mut().push(part);
        }
        let parts = self.parts();
        for p in &parts {
            let parent_id = p.st().script_part.as_ref().and_then(|sp| sp.parent.clone());
            if let Some(pid) = parent_id {
                if let Some(parent) = self.part_by_id(&pid) {
                    p.st_mut().parent = Some(Rc::downgrade(&parent));
                    parent.st_mut().children.push(Rc::downgrade(p));
                }
            }
        }
        for p in &parts {
            let item = p.st().item.clone();
            p.set_inventory_item(item, 0);
        }
    }

    /// `VehicleParts.createParts()`.
    fn parts_create(&self) {
        let me = self.rc();
        for part in self.parts() {
            let item_type = part.item_type();
            let has_types = item_type.as_ref().is_some_and(|t| !t.is_empty());
            if part.st().created && has_types && !part.has_item() && part.table("install").is_none()
            {
                part.st_mut().created = false;
            }
            if !has_types {
                part.st_mut().item = None;
            }
            if part.st().created {
                continue;
            }
            part.st_mut().created = true;
            match part.lua_function("create") {
                None => part.set_random_condition(None),
                Some(f) => {
                    lua::call_void(
                        &f,
                        &[LuaArg::Vehicle(me.clone()), LuaArg::Part(part.clone())],
                    );
                    if part.condition() == -1 {
                        part.set_random_condition(None);
                    }
                }
            }
        }
    }

    /// `VehicleParts.initParts()`.
    fn parts_init(&self) {
        let me = self.rc();
        for part in self.parts() {
            if let Some(f) = part.lua_function("init") {
                lua::call_void(
                    &f,
                    &[LuaArg::Vehicle(me.clone()), LuaArg::Part(part.clone())],
                );
            }
        }
    }

    /// `VehicleParts.updatePart(part)`.
    pub fn update_part(&self, part: &PartRef) -> bool {
        self.update_signal_device(part);
        let is_headlight =
            part.st().light.is_some() && part.id().is_some_and(|i| i.contains("Headlight"));
        if is_headlight {
            let active = self.headlights_on() && part.has_item() && self.battery_charge() > 0.0;
            part.set_light_active(active);
        }
        let Some(f) = part.lua_function("update") else {
            return false;
        };
        let world_age = env::time().world_age_hours as f32;
        let last = env::check_hours(part.st().last_updated, world_age);
        part.st_mut().last_updated = last;
        let elapsed = world_age - last;
        if (elapsed * 60.0) as i32 > 0 {
            part.st_mut().last_updated = world_age;
            let minutes = elapsed * 60.0;
            lua::call_void(
                &f,
                &[
                    LuaArg::Vehicle(self.rc()),
                    LuaArg::Part(part.clone()),
                    LuaArg::Num(minutes as f64),
                ],
            );
            return true;
        }
        false
    }

    /// `VehiclePart.updateSignalDevice()`: the radio simulation runs
    /// through the installed [`env::VehicleDevices`]; without a device env
    /// only the uninstalled check is made. The device data is taken out of
    /// the part while it updates.
    fn update_signal_device(&self, part: &PartRef) {
        if part.st().device_data.is_none() {
            return;
        }
        let devices = env::devices();
        let Some(mut inner) = devices.device_env(self, part) else {
            let turned_on = part
                .st()
                .device_data
                .as_ref()
                .is_some_and(|d| d.is_turned_on);
            if turned_on && part.is_inventory_item_uninstalled() {
                if let Some(d) = part.st_mut().device_data.as_mut() {
                    d.is_turned_on = false;
                }
            }
            part.st_mut().has_player_in_range = false;
            return;
        };
        let Some(mut dd) = part.st_mut().device_data.take() else {
            return;
        };
        let uninstalled = part.is_inventory_item_uninstalled();
        let mut penv = crate::devices::PartDeviceEnv {
            inner: &mut *inner,
            uninstalled,
            has_power: self.battery_charge() > 0.0,
            pos: [self.x(), self.y(), self.z()],
        };
        let in_range = part.st().has_player_in_range;
        let players = devices.local_players();
        let r = pz_inventory::device_sim::update_vehicle_part_device(
            &mut dd,
            &mut penv,
            uninstalled,
            in_range,
            &players,
        );
        let mut st = part.st_mut();
        st.has_player_in_range = r;
        st.device_data = Some(dd);
    }

    /// `VehicleParts.update()`.
    fn parts_update(&self) -> bool {
        let mut did = false;
        for part in self.parts() {
            if self.update_part(&part) && !did {
                did = true;
            }
        }
        did
    }

    /// `VehicleParts.isEngineWorking()` / `isEngineWorking()`.
    pub fn is_engine_working(&self) -> bool {
        let me = self.rc();
        for part in self.parts() {
            if let Some(f) = part.lua_function("checkEngine") {
                if lua::call_bool(
                    &f,
                    &[LuaArg::Vehicle(me.clone()), LuaArg::Part(part.clone())],
                ) != Some(true)
                {
                    return false;
                }
            }
        }
        true
    }

    /// `isOperational()`.
    pub fn is_operational(&self) -> bool {
        let me = self.rc();
        for part in self.parts() {
            if let Some(f) = part.lua_function("checkOperate") {
                if lua::call_bool(
                    &f,
                    &[LuaArg::Vehicle(me.clone()), LuaArg::Part(part.clone())],
                ) != Some(true)
                {
                    return false;
                }
            }
        }
        true
    }

    /// `isDriveable()`.
    pub fn is_driveable(&self) -> bool {
        self.is_engine_working() && self.is_operational()
    }

    /// `setModelVisible(part, scriptModel, visible)`.
    pub fn set_model_visible(
        &self,
        part: &PartRef,
        model_index: usize,
        model: &Model,
        visible: bool,
    ) {
        let pos = self
            .st()
            .models
            .iter()
            .position(|m| m.part.ptr_eq(&Rc::downgrade(part)) && m.model_index == model_index);
        if let Some(i) = pos {
            if !visible {
                self.st_mut().models.remove(i);
                part.st_mut().update_flags |= 0x40;
                self.st_mut().update_flags |= 0x40;
            }
            return;
        }
        if !visible {
            return;
        }
        // getModelScriptNameForPart: a model without a file takes its file
        // from the installed item's `VehiclePartModel`s (item scripts are
        // not consulted here: such models are kept as visible).
        if model.file.is_none() && !part.has_item() {
            return;
        }
        let wheel_index = part.wheel_index();
        self.st_mut().models.push(ModelInfo {
            part: Rc::downgrade(part),
            model_index,
            model_id: model.id.clone(),
            wheel_index,
        });
        part.st_mut().update_flags |= 0x40;
        self.st_mut().update_flags |= 0x40;
    }

    // ---- script -------------------------------------------------------------

    /// `setScript(name)`.
    pub fn set_script(&self, name: &str) {
        if name.trim().is_empty() {
            return;
        }
        self.st_mut().script_name = name.to_string();
        let had_script = self.st().script.is_some();
        let mut script = find_vehicle_script(name);
        if script.is_none() {
            script = self.choose_random_script();
        }
        self.st_mut().script = script.clone();
        self.st_mut().models.clear();
        if let Some(s) = &script {
            let (full, pcount, front, rear) = {
                let s = s.borrow();
                (
                    s.full_name(),
                    s.passenger_count(),
                    s.front_end_health(),
                    s.rear_end_health(),
                )
            };
            let loaded = self.st().loaded;
            let mut st = self.st_mut();
            st.script_name = full;
            let old = std::mem::take(&mut st.passengers);
            st.passengers = (0..pcount)
                .map(|i| old.get(i).cloned().unwrap_or_default())
                .collect();
            if !had_script && !loaded {
                st.rear_end_durability = 99999;
                st.front_end_durability = 99999;
            }
            st.front_end_durability = st.front_end_durability.min(front);
            st.rear_end_durability = st.rear_end_durability.min(rear);
            st.current_front_end_durability = st.front_end_durability;
            st.current_rear_end_durability = st.rear_end_durability;
        }
        self.parts_set_script(script.as_ref());
        if self.is_alarm_active() && !self.has_alarm() {
            self.alarm_stop();
        }
        if self.st().lightbar_lights_mode.is_enable() && !self.has_lightbar() {
            self.st_mut().lightbar_lights_mode.set(0);
        }
        if self.st().lightbar_siren_mode != 0 && !self.has_lightbar() {
            self.st_mut().lightbar_siren_mode = 0;
        }
        let (loaded, h, s, v) = {
            let st = self.st();
            (st.loaded, st.color_hue, st.color_saturation, st.color_value)
        };
        if !loaded || (h == 0.0 && s == 0.0 && v == 0.0) {
            self.do_vehicle_color();
        }
        self.st_mut().surround.reset();
        self.choose_alarm_sound();
    }

    /// `chooseRandomScript()`.
    fn choose_random_script(&self) -> Option<ScriptRc> {
        let mut scripts = all_vehicle_scripts();
        if scripts.is_empty() {
            return None;
        }
        let mut burnt = Vec::new();
        let mut i = 0;
        while i < scripts.len() {
            if scripts[i].borrow().wheel_count() == 0 {
                burnt.push(scripts.remove(i));
                continue;
            }
            i += 1;
        }
        let is_burnt = (self.st().loaded && self.parts.borrow().is_empty())
            || self.st().script_name.contains("Burnt");
        let script = if is_burnt && !burnt.is_empty() {
            Some(burnt[rand::next(burnt.len() as i32) as usize].clone())
        } else if !scripts.is_empty() {
            Some(scripts[rand::next(scripts.len() as i32) as usize].clone())
        } else {
            None
        };
        if let Some(s) = &script {
            self.st_mut().script_name = s.borrow().full_name();
        }
        script
    }

    /// `doVehicleColor()`.
    fn do_vehicle_color(&self) {
        if !self.st().do_color {
            let mut st = self.st_mut();
            st.color_saturation = 0.1;
            st.color_value = 0.9;
            return;
        }
        let (mut h, mut s, mut v);
        h = rand::next_float(0.0, 0.0);
        s = 0.5;
        v = rand::next_float(0.3, 0.6);
        let _ = (h, s, v);
        let rng = rand::next(100);
        if rng < 20 {
            h = rand::next_float(0.0, 0.03);
            s = rand::next_float(0.85, 1.0);
            v = rand::next_float(0.55, 0.85);
        } else if rng < 32 {
            h = rand::next_float(0.55, 0.61);
            s = rand::next_float(0.85, 1.0);
            v = rand::next_float(0.65, 0.75);
        } else if rng < 67 {
            h = 0.15;
            s = rand::next_float(0.0, 0.1);
            v = rand::next_float(0.7, 0.8);
        } else if rng < 89 {
            h = rand::next_float(0.0, 1.0);
            s = rand::next_float(0.0, 0.1);
            v = rand::next_float(0.1, 0.25);
        } else {
            h = rand::next_float(0.0, 1.0);
            s = rand::next_float(0.6, 0.75);
            v = rand::next_float(0.3, 0.7);
        }
        if let Some(script) = self.script() {
            let sc = script.borrow();
            if sc.forced_hue() > -1.0 {
                h = sc.forced_hue();
            }
            if sc.forced_sat() > -1.0 {
                s = sc.forced_sat();
            }
            if sc.forced_val() > -1.0 {
                v = sc.forced_val();
            }
        }
        let mut st = self.st_mut();
        st.color_hue = h;
        st.color_saturation = s;
        st.color_value = v;
    }

    /// `createPhysics(spawnSwap)`.
    pub fn create_physics(&self, spawn_swap: bool) {
        if self.st().vehicle_id == -1 {
            let id = crate::manager::id_map_allocate();
            self.st_mut().vehicle_id = id;
            crate::manager::id_map_put(id, &self.rc());
        }
        if self.st().script.is_none() {
            let name = self.script_name();
            self.set_script(&name);
        }
        {
            self.st_mut().create_physics_recursion += 1;
            let first = self.st().create_physics_recursion == 1;
            if first && !spawn_swap {
                env::events().event("OnSpawnVehicleStart");
            }
            let has_physics = self.st().ctrl.is_some();
            self.st_mut().create_physics_recursion -= 1;
            if first && has_physics {
                return;
            }
        }
        let Some(script) = self.script() else { return };
        if self.st().skin_index == -1 {
            let n = script.borrow().skin_count() as i32;
            self.set_skin_index(rand::next(n));
        }
        crate::world_sim::create();
        {
            let (ox, oy) = env::offset();
            let mut st = self.st_mut();
            let py = if st.saved_physics_z.is_nan() {
                st.z
            } else {
                st.saved_physics_z
            };
            st.jni_transform.origin = Vector3f::new(st.x - ox, py, st.y - oy);
        }
        let ctrl = CarController::new(self);
        self.st_mut().ctrl = Some(ctrl);
        let remote = self.st().net_player_authorization == Authorization::Remote;
        self.set_physics_active(!remote, true);
        self.st_mut().saved_physics_z = f32::NAN;
        if !self.st().created {
            self.st_mut().created = true;
            let chance = match env::sandbox().recently_survivor_vehicles {
                1 => 0,
                2 => 10,
                4 => 50,
                _ => 30,
            };
            if rand::next(100) < chance {
                self.set_good_car(true);
            }
        }
        self.parts_create();
        self.parts_init();
        self.st_mut().created_model = true;
        {
            let lights: Vec<PartRef> = self
                .parts()
                .into_iter()
                .filter(|p| p.st().light.is_some())
                .collect();
            *self.lights.borrow_mut() = lights;
        }
        let (max_speed, mass) = {
            let s = script.borrow();
            (s.max_speed, s.mass())
        };
        self.st_mut().max_speed = max_speed;
        self.st_mut().initial_mass = mass;
        crate::manager::cell_add(&self.rc());
        {
            let (x, y, z) = (self.x(), self.y(), self.z());
            let sq = env::world()
                .square(fastfloor(x), fastfloor(y), fastfloor(z))
                .map(|_| (fastfloor(x), fastfloor(y), fastfloor(z)));
            self.st_mut().square = sq;
        }
        if !self.should_not_have_loot() {
            self.randomize_containers();
        }
        if self.engine_state() == EngineState::Running {
            self.engine_do_running();
        }
        self.update_total_mass();
        self.update_damage_overlay_later();
        self.update_part_stats();
        self.st_mut().mechanical_id = rand::next(100000);
        env::events().event("OnSpawnVehicleEnd");
    }

    /// `setSkinIndex(index)`.
    pub fn set_skin_index(&self, index: i32) {
        let count = self.script().map_or(0, |s| s.borrow().skin_count() as i32);
        if index < 0 || index > count {
            return;
        }
        self.st_mut().skin_index = index;
    }

    /// `shouldNotHaveLoot()`: in a `junkyard` or `luxuryDealership` zone.
    pub fn should_not_have_loot(&self) -> bool {
        let Some((x, y, z)) = self.square_coords() else {
            return false;
        };
        matches!(
            env::world().vehicle_zone_name(x, y, z).as_deref(),
            Some("junkyard") | Some("luxuryDealership")
        )
    }

    /// `isInTrafficJam()`.
    pub fn is_in_traffic_jam(&self) -> bool {
        let Some((x, y, z)) = self.square_coords() else {
            return false;
        };
        env::world()
            .vehicle_zone_name(x, y, z)
            .is_some_and(|n| n.contains("trafficjam") || n.contains("burnt"))
    }

    /// `randomizeContainers()`: the loot distributions (`ItemPickerJava`)
    /// are not ported; containers are left as they are and the attempt is
    /// reported.
    fn randomize_containers(&self) {
        let any = self
            .parts
            .borrow()
            .iter()
            .any(|p| p.st().container.as_ref().is_some_and(|c| !c.explored));
        if any {
            env::events().event("randomizeContainers");
        }
    }

    /// `getSquare()` coordinates (`getCell().getGridSquare(x, y, z)`).
    pub fn square_coords(&self) -> Option<(i32, i32, i32)> {
        let (x, y, z) = (self.xi(), self.yi(), self.zi());
        env::world().square(x, y, z).map(|_| (x, y, z))
    }

    /// `scriptReloaded(spawnSwap)`.
    pub fn script_reloaded(&self, spawn_swap: bool) {
        if self.st().ctrl.is_some() {
            let q = self.st().jni_transform.rotation();
            self.st_mut().saved_rot = q;
            self.break_constraint(false, false);
            let id = self.id() as i32;
            Bullet::call(|b| b.remove_vehicle(id));
            self.st_mut().ctrl = None;
        }
        self.st_mut().created_model = false;
        self.st_mut().engine_rpm = None;
        for p in self.parts() {
            p.set_inventory_item(None, 0);
            p.st_mut().created = false;
        }
        let name = self.script_name();
        self.set_script(&name);
        self.create_physics(spawn_swap);
        if self.script().is_some() {
            for seat in 0..self.max_passengers() {
                if self.character(seat).is_none() {
                    continue;
                }
                if let Some(off) = self
                    .script_passenger(seat, |p| p.position_by_id("inside").map(|q| q.offset))
                    .flatten()
                {
                    self.st_mut().passengers[seat as usize].offset =
                        Vector3f::new(off.x, off.y, off.z);
                }
            }
        }
        self.st_mut().poly_dirty = true;
        if self.engine_part().is_some() && self.with_engine(|e| e.is_running()) {
            self.engine_do_shutting_down(EngineReason::ScriptReloaded);
            let state = EngineState::Idle;
            self.with_engine_mut(|e| e.state = state);
            self.on_engine_state_changed(state, state, Some(EngineReason::ScriptReloaded));
        }
    }

    // ---- engine -------------------------------------------------------------

    /// `getVehicleEngine()`: the `Engine` part's engine, or the engine of a
    /// detached part when there is none.
    fn engine_holder(&self) -> PartRef {
        if let Some(p) = self.engine_part() {
            if p.st().engine.is_some() {
                return p;
            }
        }
        let mut m = self.missing_engine_part.borrow_mut();
        m.get_or_insert_with(|| {
            let p = VehiclePart::new_weak(self.me.clone());
            p.st_mut().engine = Some(VehicleEngine::default());
            p
        })
        .clone()
    }

    /// Runs `f` on the engine (`getVehicleEngine()`).
    pub fn with_engine<R>(&self, f: impl FnOnce(&VehicleEngine) -> R) -> R {
        let p = self.engine_holder();
        let st = p.st();
        f(st.engine.as_ref().unwrap())
    }

    pub fn with_engine_mut<R>(&self, f: impl FnOnce(&mut VehicleEngine) -> R) -> R {
        let p = self.engine_holder();
        let mut st = p.st_mut();
        f(st.engine.as_mut().unwrap())
    }

    /// `getEngineState()`.
    pub fn engine_state(&self) -> EngineState {
        if self.engine_part().is_none() {
            return EngineState::Idle;
        }
        self.with_engine(|e| e.state)
    }
    /// `isEngineRunning()`.
    pub fn is_engine_running(&self) -> bool {
        self.engine_part().is_some() && self.with_engine(|e| e.is_running())
    }
    /// `isEngineStarted()`.
    pub fn is_engine_started(&self) -> bool {
        self.with_engine(|e| e.is_started())
    }
    /// `isStarting()`.
    pub fn is_starting(&self) -> bool {
        self.with_engine(|e| e.is_starting())
    }
    /// `getEngineSpeed()`.
    pub fn engine_speed(&self) -> f64 {
        if self.engine_part().is_none() {
            return 0.0;
        }
        self.with_engine(|e| e.speed)
    }
    /// `setEngineSpeed(speed)`.
    pub fn set_engine_speed(&self, speed: f64) {
        self.with_engine_mut(|e| e.speed = speed);
    }
    /// `addEngineSpeed(speed)`.
    pub fn add_engine_speed(&self, speed: f64) {
        self.with_engine_mut(|e| e.speed += speed);
    }
    /// `getEngineQuality()`.
    pub fn engine_quality(&self) -> i32 {
        if self.engine_part().is_none() {
            0
        } else {
            self.with_engine(|e| e.quality)
        }
    }
    /// `getEngineLoudness()`.
    pub fn engine_loudness(&self) -> i32 {
        if self.engine_part().is_none() {
            0
        } else {
            self.with_engine(|e| e.loudness)
        }
    }
    /// `getEnginePower()`.
    pub fn engine_power(&self) -> i32 {
        if self.engine_part().is_none() {
            0
        } else {
            self.with_engine(|e| e.power)
        }
    }
    /// `setEngineFeature(quality, loudness, power)`.
    pub fn set_engine_feature(&self, quality: i32, loudness: i32, power: i32) {
        self.with_engine_mut(|e| e.set_features(quality, loudness, power));
    }

    fn engine_transition(&self, state: EngineState, reason: Option<EngineReason>, notify: bool) {
        let old = self.with_engine_mut(|e| {
            if reason.is_some() {
                e.reason = reason;
            }
            e.transition(state)
        });
        if notify {
            self.on_engine_state_changed(old, state, reason);
        }
    }

    /// `engineDoIdle()` (`VehicleEngine.doIdle`: no listener call).
    pub fn engine_do_idle(&self) {
        self.engine_transition(EngineState::Idle, None, false);
    }
    pub fn engine_do_starting(&self) {
        self.engine_transition(EngineState::Starting, None, true);
    }
    pub fn engine_do_retrying_starting(&self) {
        self.engine_transition(EngineState::RetryingStarting, None, true);
    }
    pub fn engine_do_starting_success(&self) {
        self.engine_transition(EngineState::StartingSuccess, None, true);
    }
    pub fn engine_do_starting_failed(&self, reason: EngineReason) {
        self.engine_transition(EngineState::StartingFailed, Some(reason), true);
    }
    pub fn engine_do_starting_failed_no_power(&self) {
        self.engine_do_starting_failed(EngineReason::NoPower);
    }
    pub fn engine_do_running(&self) {
        self.engine_transition(EngineState::Running, None, true);
    }
    pub fn engine_do_stalling(&self) {
        self.engine_transition(EngineState::Stalling, None, true);
    }
    pub fn engine_do_shutting_down(&self, reason: EngineReason) {
        self.engine_transition(EngineState::ShuttingDown, Some(reason), true);
    }
    /// `shutOff()` / `VehicleEngine.shutOff(reason)`.
    pub fn shut_off(&self, reason: EngineReason) {
        if !self.has_enough_gas_to_run() {
            self.engine_do_stalling();
            return;
        }
        self.engine_do_shutting_down(reason);
    }

    /// `VehicleEngine.update()`.
    fn engine_update(&self) {
        let now = env::now_ms();
        let check = self.with_engine(|e| now - e.engine_check_time > 1000);
        if check {
            self.with_engine_mut(|e| e.engine_check_time = now);
            let should_shut =
                self.with_engine(|e| e.state == EngineState::Running) && !self.is_engine_working();
            if should_shut {
                let reason = if !self.has_enough_gas_to_run() {
                    EngineReason::OutOfFuel
                } else {
                    EngineReason::EngineNotWorking
                };
                self.shut_off(reason);
            }
        }
        let (state, ust) = self.with_engine(|e| (e.state, e.update_state_time));
        if state == EngineState::Starting {
            self.engine_update_starting();
        }
        let state = self.with_engine(|e| e.state);
        if state == EngineState::RetryingStarting && now - ust > 2000 {
            self.engine_do_starting();
        }
        let (state, ust) = self.with_engine(|e| (e.state, e.update_state_time));
        if state == EngineState::StartingSuccess && now - ust > 1500 {
            self.engine_do_running();
        }
        let (state, ust) = self.with_engine(|e| (e.state, e.update_state_time));
        if state == EngineState::StartingFailed && now - ust > 1500 {
            self.engine_do_idle();
        }
        let (state, ust) = self.with_engine(|e| (e.state, e.update_state_time));
        if state == EngineState::Stalling && now - ust > 3000 {
            self.engine_do_idle();
        }
        let (state, ust) = self.with_engine(|e| (e.state, e.update_state_time));
        if state == EngineState::ShuttingDown && now - ust > 2000 {
            self.engine_do_idle();
        }
        if self.with_engine(|e| e.state) != EngineState::Idle {
            self.engine_update_world_sounds();
        }
        // shouldAutoStall: the chance is computed first, Rand only when
        // running below condition 50.
        let cond = self.engine_holder().condition();
        let chance = rand::adjust_for_framerate(cond * 12);
        if self.with_engine(|e| e.state == EngineState::Running)
            && cond < 50
            && rand::next(chance) == 0
        {
            self.shut_off(EngineReason::EngineConditionLow);
        }
    }

    /// `VehicleEngine.updateStarting()`.
    fn engine_update_starting(&self) {
        if self.battery_charge() < 0.1 {
            self.engine_do_starting_failed_no_power();
            return;
        }
        if !self.has_enough_gas_to_run() {
            self.engine_do_starting_failed(EngineReason::EngineNotWorking);
            return;
        }
        if self.engine_should_fail_to_start_due_to_cold() {
            self.engine_do_starting_failed(EngineReason::EngineQualityLow);
            return;
        }
        let q = self.with_engine(|e| e.quality);
        if rand::next(q) != 0 {
            self.engine_do_starting_success();
        } else {
            self.engine_do_retrying_starting();
        }
    }

    /// `VehicleEngine.shouldFailToStartDueToCold()`: the air temperature
    /// comes from [`crate::env::VehicleEvents`]-free `ClimateManager`
    /// (not ported; taken as warm).
    fn engine_should_fail_to_start_due_to_cold(&self) -> bool {
        if env::sandbox().vehicle_easy_use {
            return false;
        }
        let q = self.with_engine(|e| e.quality);
        if q >= 100 {
            return false;
        }
        let air_temp = crate::world_sim::air_temperature();
        let mut weather = 0;
        if q < 65 && air_temp <= 2.0 {
            weather = ((2.0 - air_temp) * 2.0).min(30.0) as i32;
        }
        rand::next(q + 50 - weather) <= 30
    }

    /// `VehicleEngine.updateWorldSounds()`: the Rand draws and the sounds
    /// (`WorldSoundManager` is reported as events).
    fn engine_update_world_sounds(&self) {
        let (loudness, speed) = self.with_engine(|e| (e.loudness, e.speed));
        let mut l = (loudness as f64 * speed / 2500.0) as i32;
        let max_speed = speed.min(2000.0);
        l *= (1.0 + max_speed / 4000.0) as i32;
        let base = 120.0f64;
        let inv = env::time().inv_multiplier as f64;
        if rand::next((base * inv) as i32) == 0 {
            env::events().event(&format!("worldSound {}", 8.max(l)));
        }
        if rand::next(((base - 85.0) * inv) as i32) == 0 {
            env::events().event(&format!("worldSound {}", 8.max(l / 2)));
        }
        if rand::next(((base - 110.0) * inv) as i32) == 0 {
            env::events().event(&format!("worldSound {}", 8.max(l / 4)));
        }
        env::events().event(&format!("worldSound {}", 8.max(l / 6)));
    }

    /// `onEngineStateChanged(old, new, reason)` (single player).
    pub fn on_engine_state_changed(
        &self,
        _old: EngineState,
        new: EngineState,
        _reason: Option<EngineReason>,
    ) {
        match new {
            EngineState::Starting => {
                self.set_keys_in_ignition(true);
                self.set_previously_moved(true);
            }
            EngineState::RetryingStarting | EngineState::StartingFailed => {
                env::events().event("engineSound fail");
            }
            EngineState::StartingSuccess => {
                env::events().event("engineSound start");
                self.set_keys_in_ignition(true);
            }
            EngineState::Running => self.set_need_parts_update(true),
            EngineState::Stalling => {
                env::events().event("engineSound outOfGas");
                self.set_keys_in_ignition(false);
            }
            EngineState::ShuttingDown => {
                env::events().event("engineSound turnOff");
                self.set_keys_in_ignition(false);
                if let Some(h) = self.heater() {
                    h.mod_data()
                        .set_str("active", pz_kahlua::Value::Bool(false));
                }
            }
            EngineState::Idle => {}
        }
    }

    /// `tryStartEngine(haveKey)`.
    pub fn try_start_engine(&self, have_key: bool) {
        if self.driver().is_some_and(|d| d.is_block_movement()) {
            return;
        }
        let Some(part) = self.engine_part() else {
            return;
        };
        let start_allowed = env::sandbox().vehicle_easy_use
            || self.is_keys_in_ignition()
            || have_key
            || self.is_hotwired();
        if part.condition() <= 0 || self.engine_quality() <= 0 {
            if start_allowed {
                let r = if part.condition() <= 0 {
                    EngineReason::EngineConditionLow
                } else {
                    EngineReason::EngineQualityLow
                };
                self.engine_do_starting_failed(r);
            } else {
                env::events().event("SayDebug CarKey_none");
            }
            return;
        }
        if self.engine_state() != EngineState::Idle {
            return;
        }
        let Some(battery) = self.battery().and_then(|b| b.item()) else {
            return;
        };
        if !items::is_drainable(&battery.borrow()) {
            return;
        }
        let charge = items::current_uses_float(&battery.borrow());
        items::set_current_uses_float(
            &mut battery.borrow_mut(),
            jmath::clamp(charge - 0.025, 0.0, 1.0),
        );
        if charge <= 0.1 {
            self.engine_do_starting_failed_no_power();
            return;
        }
        if start_allowed {
            self.engine_do_starting();
        } else {
            env::events().event("SayDebug CarKey_none");
        }
    }

    /// `resumeRunningAfterLoad()`.
    pub fn resume_running_after_load(&self) {
        if !self.is_engine_working() {
            return;
        }
        self.engine_do_starting_success();
    }

    // ---- keys -----------------------------------------------------------------

    /// `setKeysInIgnition(keysOnContact)` (single player).
    pub fn set_keys_in_ignition(&self, keys_on_contact: bool) {
        let Some(driver) = self.driver() else { return };
        self.set_alarmed(false);
        if self.is_hotwired() {
            return;
        }
        if keys_on_contact && !self.is_keys_in_ignition() {
            if let Some(key) = driver.have_key(self.key_id()) {
                self.st_mut().keys_in_ignition = true;
                driver.remove_item(&key);
                self.st_mut().ignition_switch.push(key);
                self.st_mut().keys_container_id = -1;
            }
        }
        if !keys_on_contact && self.is_keys_in_ignition() {
            let key = self.st().ignition_switch[0].clone();
            driver.add_item(key);
            let mut st = self.st_mut();
            st.ignition_switch.clear();
            st.current_key = None;
            st.keys_in_ignition = false;
        }
    }

    /// `getCurrentKey()`.
    pub fn current_key(&self) -> Option<ItemRef> {
        self.st().ignition_switch.first().cloned()
    }

    /// `setCurrentKey(key)`.
    pub fn set_current_key(&self, key: ItemRef) {
        let mut st = self.st_mut();
        st.current_key = Some(key.clone());
        st.ignition_switch.push(key);
    }

    /// `tryHotwire(electricityLevel)`.
    pub fn try_hotwire(&self, electricity_level: i32) {
        let q = (100 - self.engine_quality()).max(5).min(50);
        let chance = q + electricity_level * 4;
        if rand::next(100) <= 11 - electricity_level && self.is_alarmed() {
            self.trigger_alarm();
        }
        if rand::next(100) <= chance {
            self.set_hotwired(true);
            env::events().event("playSound VehicleHotwireSuccess");
        } else {
            if rand::next(100) <= 10 - electricity_level {
                self.set_hotwired_broken(true);
            }
            env::events().event("playSound VehicleHotwireFail");
        }
    }

    // ---- alarm ------------------------------------------------------------

    /// `isAlarmActive()`.
    pub fn is_alarm_active(&self) -> bool {
        self.has_alarm() && self.st().alarm.start_time > 0.0 && self.battery_charge() > 0.0
    }

    /// `VehicleAlarm.chooseSound()`.
    pub fn choose_alarm_sound(&self) {
        let Some(script) = self.script() else {
            self.st_mut().alarm.chosen_sound = None;
            return;
        };
        let (enable, alarm, alarm_loop) = {
            let s = script.borrow();
            let snd = s.sounds();
            (snd.alarm_enable, snd.alarm.clone(), snd.alarm_loop.clone())
        };
        if !enable {
            self.st_mut().alarm.chosen_sound = None;
            return;
        }
        let chosen = self.st().alarm.chosen_sound.clone();
        let ok = chosen
            .as_ref()
            .is_some_and(|c| alarm.contains(c) || alarm_loop.contains(c));
        if !ok {
            let choices: Vec<String> = alarm.into_iter().chain(alarm_loop).collect();
            let pick = if choices.is_empty() {
                None
            } else {
                Some(choices[rand::next(choices.len() as i32) as usize].clone())
            };
            self.st_mut().alarm.chosen_sound = pick;
        }
    }

    /// `VehicleAlarm.getChosenSound()`.
    pub fn chosen_alarm_sound(&self) -> Option<String> {
        self.choose_alarm_sound();
        self.st().alarm.chosen_sound.clone()
    }

    fn alarm_is_looping(&self) -> bool {
        let Some(script) = self.script() else {
            return false;
        };
        self.choose_alarm_sound();
        let chosen = self.st().alarm.chosen_sound.clone();
        let s = script.borrow();
        chosen.is_some_and(|c| s.sounds().alarm_loop.contains(&c))
    }

    fn alarm_start(&self) {
        self.st_mut().alarm.sound_on = true;
        env::events().event("alarmStart");
    }

    fn alarm_stop(&self) {
        self.st_mut().alarm.sound_on = false;
        env::events().event("alarmStop");
    }

    fn alarm_lights(&self, on: bool) {
        self.st_mut().alarm.lights_on = on;
        // `onVehicleAlarmEvent(LIGHTS_ON/OFF)` of the listener: the
        // vehicle, or its virtual record while in the meta.
        let in_meta = self.st().meta_headlights.is_some();
        if in_meta {
            self.st_mut().meta_headlights = Some(on);
        } else {
            self.set_headlights_on(on);
        }
    }

    /// `VirtualVehicle.update`'s `getParts().update()`.
    pub(crate) fn update_parts_in_meta(&self) {
        self.parts_update();
    }

    /// `VirtualVehicle.update`'s `getVehicleEngine().update()`.
    pub(crate) fn update_engine_in_meta(&self) {
        self.engine_update();
    }

    /// `VirtualVehicle.update`'s `vehicleAlarm.update()`.
    pub(crate) fn update_alarm_in_meta(&self) {
        self.alarm_update();
    }

    /// `triggerAlarm()`.
    pub fn trigger_alarm(&self) {
        let (alarmed, prev) = (self.st().alarmed, self.st().previously_entered);
        self.st_mut().alarmed = false;
        if !alarmed || prev {
            return;
        }
        self.st_mut().alarm.start_time = env::time().world_age_hours;
        self.st_mut().alarm.accumulator = 0.0;
        if self.alarm_is_looping() {
            self.alarm_start();
        }
    }

    /// `VehicleAlarm.update()`.
    fn alarm_update(&self) {
        if self.st().alarm.start_time <= 0.0 {
            return;
        }
        if self.battery_charge() <= 0.0 {
            if self.st().alarm.sound_on {
                self.alarm_stop();
            }
            self.st_mut().alarm.start_time = 0.0;
            return;
        }
        let t = env::time();
        let start = env::min_hours(self.st().alarm.start_time, t.world_age_hours);
        self.st_mut().alarm.start_time = start;
        if t.world_age_hours >= start + 0.66 * t.delta_minutes_per_day as f64 {
            self.alarm_stop();
            self.alarm_lights(false);
            self.st_mut().alarm.start_time = 0.0;
            return;
        }
        self.choose_alarm_sound();
        let looping = self.alarm_is_looping();
        if looping && !self.st().alarm.sound_on {
            self.alarm_start();
        }
        self.st_mut().alarm.accumulator += t.thirty_fps_multiplier;
        let n = self.st().alarm.accumulator as i32 / 24;
        if !self.st().alarm.lights_on && n % 2 == 0 {
            if !looping {
                self.alarm_start();
            }
            self.alarm_lights(true);
        }
        if self.st().alarm.lights_on && n % 2 == 1 {
            if !looping {
                self.alarm_stop();
            }
            self.alarm_lights(false);
        }
        let sound_on = self.st().alarm.sound_on;
        if sound_on
            && self.alarm_is_looping()
            && self.st_mut().alarm.world_sound_update_limit.check()
        {
            env::events().event("alarmWorldSound");
        }
    }

    // ---- stats ------------------------------------------------------------

    /// `updateDamageOverlayLater()`.
    pub fn update_damage_overlay_later(&self) {
        self.st_mut().do_damage_overlay = true;
    }

    /// `updateTotalMass()`.
    pub fn update_total_mass(&self) {
        let mut plus = 0.0f32;
        for p in self.parts() {
            let st = p.st();
            if let Some(c) = &st.container {
                plus += items::capacity_weight(c);
            }
            if let Some(PartItem::Item(i)) = &st.item {
                plus += items::weight(&i.borrow());
            }
        }
        let m = jmath::java_round_f(self.st().initial_mass + plus) as f32;
        self.st_mut().mass = m;
        if self.st().ctrl.is_some() {
            let (id, m) = (self.id() as i32, self.mass());
            Bullet::call(|b| b.set_vehicle_mass(id, m));
        }
    }

    /// `updatePartStats()`.
    pub fn update_part_stats(&self) {
        self.st_mut().braking_force = 0.0;
        let Some(script) = self.script() else { return };
        let mut loudness = (script.borrow().engine_loudness() as f64
            * env::sandbox().zombie_attraction_multiplier
            / 2.0) as f32;
        let mut found_muffler = false;
        for part in self.parts() {
            let Some(item) = part.item() else { continue };
            let (cond, s) = {
                let i = item.borrow();
                (i.condition, items::stats(&i))
            };
            let skill = part.st().mechanic_skill_installer;
            if s.brake_force > 0.0 {
                let mut f = number_by_condition(s.brake_force, cond as f32, 5.0);
                f += f / 50.0 * skill as f32;
                let b = self.st().braking_force + f;
                self.st_mut().braking_force = b;
            }
            if s.wheel_friction > 0.0 {
                part.st_mut().wheel_friction = 0.0;
                let mut f = number_by_condition(s.wheel_friction, cond as f32, 0.2);
                f += 0.1 * skill as f32;
                f = f.min(2.3);
                part.st_mut().wheel_friction = f;
            }
            if s.suspension_compression > 0.0 {
                let mut st = part.st_mut();
                st.suspension_compression =
                    number_by_condition(s.suspension_compression, cond as f32, 0.6);
                st.suspension_damping = number_by_condition(s.suspension_damping, cond as f32, 0.6);
            }
            if s.engine_loudness > 0.0 {
                let l = number_by_condition(s.engine_loudness, cond as f32, 10.0);
                part.st_mut().engine_loudness = l;
                loudness *= 1.0 + (100.0 - l) / 100.0;
                found_muffler = true;
            }
            if s.durability > 0.0 {
                part.st_mut().durability = s.durability;
            }
        }
        if self.engine_part().is_some() {
            if !found_muffler {
                loudness *= 2.0;
            }
            self.with_engine_mut(|e| e.loudness = loudness as i32);
        }
    }

    /// `isInForest()`.
    pub fn is_in_forest(&self) -> bool {
        let Some((x, y, z)) = self.square_coords() else {
            return false;
        };
        matches!(
            env::world()
                .square(x, y, z)
                .and_then(|s| s.zone_type)
                .as_deref(),
            Some("Forest") | Some("DeepForest") | Some("FarmLand")
        )
    }

    /// `isDoingOffroad()`.
    pub fn is_doing_offroad(&self) -> bool {
        let Some((x, y, z)) = self.st().current else {
            return false;
        };
        let Some(sq) = env::world().square(x, y, z) else {
            return false;
        };
        if !sq.has_floor {
            return false;
        }
        let Some(name) = sq.floor_sprite else {
            return false;
        };
        !name.contains("carpentry_02")
            && !name.contains("blends_street")
            && !name.contains("floors_exterior_street")
    }

    /// `getOffroadEfficiency()`.
    pub fn offroad_efficiency(&self) -> f32 {
        let e = self
            .script()
            .map_or(0.0, |s| s.borrow().offroad_efficiency());
        if self.is_in_forest() {
            e * 1.5
        } else {
            e * 2.0
        }
    }

    /// `updateBulletStats()`.
    pub fn update_bullet_stats(&self) {
        if self.script_name().contains("Burnt") || !crate::world_sim::created() {
            return;
        }
        let Some(script) = self.script() else { return };
        let mut data = [0.0f32; WHEEL_PARAMS];
        let period = 2.4f64;
        let mut chance_of_bump = 100;
        let mut friction_mul = 1.0f32;
        let speed = self.current_absolute_speed_kmh();
        let susp: f64;
        if self.is_in_forest() && self.is_doing_offroad() && speed > 1.0 {
            susp = rand::next_float(0.04, 0.13) as f64;
            friction_mul = 0.7;
            chance_of_bump = 15;
        } else if self.is_doing_offroad() && speed > 1.0 {
            chance_of_bump = 25;
            susp = rand::next_float(0.02, 0.1) as f64;
            friction_mul = 0.7;
        } else {
            susp = if speed > 1.0 && rand::next(100) < 10 {
                rand::next_float(0.01, 0.05) as f64
            } else {
                0.0
            };
        }
        if env::world().is_raining() {
            friction_mul -= 0.3;
        }
        let wheels: Vec<(Option<String>, Vector3f)> = {
            let s = script.borrow();
            (0..s.wheel_count())
                .map(|i| {
                    let w = s.wheel(i);
                    (
                        w.id.clone(),
                        Vector3f::new(w.offset.x, w.offset.y, w.offset.z),
                    )
                })
                .collect()
        };
        for (i, (wid, off)) in wheels.iter().enumerate() {
            if i >= 4 {
                break;
            }
            let o = i * 6;
            let wheel_pos = self.world_pos(off.x, off.y, off.z);
            let wid = wid.clone().unwrap_or_else(|| "null".into());
            let part = self.part_by_id(&format!("Tire{wid}"));
            let susp_part = self.part_by_id(&format!("Suspension{wid}"));
            match part.filter(|p| p.has_item()) {
                Some(p) => {
                    data[o] = 1.0;
                    data[o + 1] = (p.container_content_amount()
                        / (p.container_capacity() - 10) as f32)
                        .min(1.0);
                    data[o + 2] = friction_mul * p.st().wheel_friction;
                    match susp_part.filter(|s| s.has_item()) {
                        Some(s) => {
                            data[o + 3] = s.st().suspension_damping;
                            data[o + 4] = s.st().suspension_compression;
                        }
                        None => {
                            data[o + 3] = 0.1;
                            data[o + 4] = 0.1;
                        }
                    }
                    data[o + 5] = if chance_of_bump > 0 && rand::next(chance_of_bump) == 0 {
                        ((period * wheel_pos.x as f64).sin()
                            * (period * wheel_pos.y as f64).sin()
                            * susp) as f32
                    } else {
                        0.0
                    };
                }
                None => {
                    data[o] = 0.0;
                    data[o + 1] = 30.0;
                    data[o + 2] = 0.0;
                    data[o + 3] = 2.88;
                    data[o + 4] = 3.83;
                    data[o + 5] = if rand::next(chance_of_bump) == 0 {
                        ((period * wheel_pos.x as f64).sin()
                            * (period * wheel_pos.y as f64).sin()
                            * susp) as f32
                    } else {
                        0.0
                    };
                }
            }
        }
        let id = self.id() as i32;
        crate::world_sim::with_wheel_params(|wp| {
            // The original fills a static array that keeps the values of
            // wheels a vehicle with fewer wheels does not write.
            for (i, v) in data.iter().enumerate().take(wheels.len().min(4) * 6) {
                wp[i] = *v;
            }
            Bullet::call(|b| b.set_vehicle_params(id, &mut wp[..]));
        });
    }

    /// `getFudgedMass()`.
    pub fn fudged_mass(&self) -> f32 {
        let name = self.script_name();
        if name.contains("Trailer") {
            if let Some(by) = self.vehicle_towed_by() {
                if by.is_attaching_trailer() {
                    return self.mass() * 0.2;
                }
            }
        }
        if !self.is_burnt() || name.contains("Trailer") {
            return self.mass();
        }
        if let Some(a) = self.vehicle_towed_by() {
            if a.driver().is_some() && a.is_engine_running() {
                let mut mass = 250.0f32.max(a.mass() / 3.7);
                if self.script().is_some_and(|s| s.borrow().wheel_count() == 0) {
                    mass = mass.min(200.0);
                }
                return mass;
            }
        }
        self.mass()
    }

    /// `isAttachingTrailer()`.
    pub fn is_attaching_trailer(&self) -> bool {
        self.st().begin_attach_trailer_ms != 0
    }

    // ---- physics state ------------------------------------------------------

    /// `getController() != null`.
    pub fn has_controller(&self) -> bool {
        self.st().ctrl.is_some()
    }

    /// `setPhysicsActive(active, setStatic)`.
    pub fn set_physics_active(&self, active: bool, set_static: bool) {
        self.st_mut().desire_physics_active = active;
        let active = if self
            .st()
            .disable_simulation_due_to_lack_of_surrounding_chunks
        {
            false
        } else {
            active
        };
        let enabled = match &self.st().ctrl {
            None => return,
            Some(c) => c.is_enable,
        };
        if active == enabled {
            return;
        }
        self.st_mut().ctrl.as_mut().unwrap().is_enable = active;
        let id = self.id() as i32;
        if set_static && self.st().is_static == active {
            self.st_mut().is_static = !active;
            Bullet::call(|b| b.set_vehicle_static(id, !active));
        }
        if self.st().is_active != active {
            self.st_mut().is_active = active;
            Bullet::call(|b| b.set_vehicle_active(id, active));
        }
        if active {
            self.st_mut().physic_active_check = env::now_ms() + 3000;
        }
        if let Some(t) = self.vehicle_towing() {
            t.set_physics_active(active, set_static);
        }
    }

    /// `isPhysicsActive()`.
    pub fn is_physics_active(&self) -> bool {
        self.st().ctrl.as_ref().is_some_and(|c| c.is_enable)
    }

    /// `setSpeedKmHour`.
    pub fn set_speed_kmh(&self, s: f32) {
        self.st_mut().jni_speed = s;
    }

    /// `getCurrentSpeedKmHour()` (single player).
    pub fn current_speed_kmh(&self) -> f32 {
        let disabled = self.st().ctrl.as_ref().is_some_and(|c| !c.is_enable);
        if disabled {
            self.st_mut().jni_speed = 0.0;
            return 0.0;
        }
        self.st().jni_speed
    }

    /// `getCurrentAbsoluteSpeedKmHour()`.
    pub fn current_absolute_speed_kmh(&self) -> f32 {
        self.current_speed_kmh().abs()
    }

    /// `getSpeed2D()`.
    pub fn speed_2d(&self) -> f32 {
        let v = self.st().jni_linear_velocity;
        jmath::jsqrt(v.x * v.x + v.z * v.z)
    }

    /// `isAtRest()`.
    pub fn is_at_rest(&self) -> bool {
        let st = self.st();
        let Some(ctrl) = &st.ctrl else { return true };
        if !st.impulses_from_hit_objects.is_empty() || st.impulse_from_server.enable {
            return false;
        }
        if ctrl.engine_force.abs() >= 0.01 {
            return false;
        }
        drop(st);
        if self.speed_2d() >= 0.025 {
            return false;
        }
        let (vy, oy) = {
            let st = self.st();
            (st.jni_linear_velocity.y, st.jni_transform.origin.y)
        };
        if vy.abs() >= 0.5 {
            return false;
        }
        let (x, y, z) = (self.xi(), self.yi(), self.zi());
        let Some(sq) = env::world().square(x, y, z) else {
            return false;
        };
        if !sq.has_floor {
            return false;
        }
        let zz = oy / LEVEL_HEIGHT;
        !(zz - z as f32 > 0.2)
    }

    /// `getForwardVector(out)`.
    pub fn forward_vector(&self) -> Vector3f {
        self.st().jni_transform.basis.column(2)
    }

    /// `getUpVector(out)`.
    pub fn up_vector(&self) -> Vector3f {
        self.st().jni_transform.basis.column(1)
    }

    /// `getUpVectorDot()`.
    pub fn up_vector_dot(&self) -> f32 {
        self.up_vector().dot(&Vector3f::new(0.0, 1.0, 0.0))
    }

    /// `getLinearVelocity(out)`.
    pub fn linear_velocity(&self) -> Vector3f {
        self.st().jni_linear_velocity
    }

    /// `getWorldTransform(out)`.
    pub fn world_transform(&self) -> Transform {
        self.st().jni_transform
    }

    /// `setWorldTransform(t)`.
    pub fn set_world_transform(&self, t: Transform) {
        self.st_mut().jni_transform = t;
        let q = t.rotation();
        let (ox, oy) = env::offset();
        let id = self.id() as i32;
        Bullet::call(|b| {
            b.teleport_vehicle(
                id,
                t.origin.x + ox,
                t.origin.z + oy,
                t.origin.y,
                q.x,
                q.y,
                q.z,
                q.w,
            )
        });
    }

    /// `getVehicleEngineRPM().rpmData`.
    pub fn engine_rpm_data(&self) -> [Option<pz_scripts::objects::models::EngineRPMData>; 8] {
        if let Some(d) = self.st().engine_rpm {
            return d;
        }
        let ty = self
            .script()
            .map(|s| s.borrow().engine_rpm_type().to_string())
            .unwrap_or_default();
        let data = script_manager()
            .and_then(|sm| {
                sm.get::<pz_scripts::objects::models::VehicleEngineRPM>(
                    ScriptType::VehicleEngineRPM,
                    &ty,
                )
            })
            .map(|r| r.borrow().rpm_data)
            .unwrap_or([None; 8]);
        self.st_mut().engine_rpm = Some(data);
        data
    }

    // ---- update -----------------------------------------------------------

    /// `updateParts()`.
    pub fn update_parts(&self) {
        if self.parts_update() {
            self.st_mut().brake_between_updates_speed = 0.0;
        }
    }

    /// `drainBatteryUpdateHack()`.
    pub fn drain_battery_update_hack(&self) {
        if self.is_engine_running() {
            return;
        }
        for part in self.parts() {
            let device_on = part
                .st()
                .device_data
                .as_ref()
                .is_some_and(|d| d.is_turned_on);
            if device_on {
                self.update_part(&part);
                continue;
            }
            let light_on = part.st().light.as_ref().is_some_and(|l| l.active);
            if light_on {
                self.update_part(&part);
            }
        }
        if self.has_lightbar()
            && (self.st().lightbar_lights_mode.is_enable() || self.st().lightbar_siren_mode != 0)
        {
            if let Some(b) = self.battery() {
                self.update_part(&b);
            }
        }
    }

    /// `update()` for a vehicle in the cell (single player).
    pub fn update(&self) {
        if self.in_update.replace(true) {
            return;
        }
        self.update_inner();
        self.in_update.set(false);
    }

    fn update_inner(&self) {
        if self.st().removed_from_world {
            return;
        }
        if !crate::manager::cell_contains(self) {
            return;
        }
        if self.st().time_since_last_auth > 0.0 {
            self.st_mut().time_since_last_auth -= 1.0;
        }
        if self.st().ctrl.is_some()
            && self.st().vehicle_towing_id != -1
            && self.vehicle_towing().is_none()
        {
            self.try_reconnect_to_towed_vehicle();
        }
        let mut towed = false;
        let mut towing = false;
        if let Some(by) = self.vehicle_towed_by() {
            if let Some(c) = &by.st().ctrl {
                towed = c.is_enable;
                towing = self.vehicle_towing().is_some_and(|t| t.driver().is_some());
            }
        }
        if self.st().ctrl.is_some() {
            let update_physics = self.driver().is_some() || towed || towing || !self.is_at_rest();
            let now = env::now_ms();
            if self.st().constraint_changed_time != -1 {
                if self.st().constraint_changed_time + 3500 < now {
                    self.st_mut().constraint_changed_time = -1;
                    if !update_physics && self.st().physic_active_check < now {
                        self.set_physics_active(false, true);
                    }
                }
            } else {
                let enabled = self.is_physics_active();
                if self.st().physic_active_check != -1 && (update_physics || !enabled) {
                    self.st_mut().physic_active_check = -1;
                }
                let pac = self.st().physic_active_check;
                if !update_physics && self.is_physics_active() && pac != -1 && pac < now {
                    self.st_mut().physic_active_check = -1;
                    self.set_physics_active(false, true);
                }
            }
            let has_wheels = self.script().is_some_and(|s| s.borrow().wheel_count() > 0);
            if self.vehicle_towed_by().is_some() && has_wheels {
                self.controller_update_trailer();
            } else if self.driver().is_none() {
                self.controller_check_should_be_active();
            }
            self.alarm_update();
            let imp = self.st().impulse_from_server;
            if imp.enable {
                self.st_mut().impulse_from_server.enable = false;
                let id = self.id() as i32;
                let t = imp.rel_pos.cross(&imp.impulse);
                Bullet::call(|b| {
                    b.apply_central_force_to_vehicle(
                        id,
                        imp.impulse.x,
                        imp.impulse.y,
                        imp.impulse.z,
                    )?;
                    b.apply_torque_to_vehicle(id, t.x, t.y, t.z)
                });
            }
            if self.engine_part().is_some() {
                self.engine_update();
            }
            if self.driver().is_none() && !towed {
                self.controller_park();
            }
            let (ox, oy) = env::offset();
            let o = self.st().jni_transform.origin;
            {
                let mut st = self.st_mut();
                st.x = o.x + ox;
                st.y = o.z + oy;
                st.z = 0.0;
            }
            let zi = fastfloor(o.y / LEVEL_HEIGHT + 0.05);
            let (x, y) = (self.x(), self.y());
            let w = env::world();
            let square = w.square(fastfloor(x), fastfloor(y), zi);
            let below = w.square(fastfloor(x), fastfloor(y), zi - 1);
            if square.as_ref().is_some_and(|s| s.has_floor)
                || (square.is_some() && below.as_ref().is_some_and(|b| b.has_floor))
            {
                self.st_mut().z = zi as f32;
            }
            let sq = w.square(self.xi(), self.yi(), self.zi());
            let chunk_refs = self
                .st()
                .chunk
                .is_some_and(|(wx, wy)| w.chunk_loaded(wx, wy));
            if sq.is_none() && chunk_refs {
                let (wx, wy) = self.st().chunk.unwrap();
                let d = 5.0e-4f32;
                let (minx, miny) = ((wx * 8) as f32, (wy * 8) as f32);
                {
                    let mut st = self.st_mut();
                    st.x = st.x.max(minx + d).min(minx + 8.0 - d);
                    st.y = st.y.max(miny + d).min(miny + 8.0 - d);
                    st.z = 0.2;
                }
                let mut t1 = self.world_transform();
                let (x, y, z) = (self.x(), self.y(), self.z());
                t1.origin = Vector3f::new(x - ox, z, y - oy);
                self.set_world_transform(t1);
                let cur = w
                    .square(self.xi(), self.yi(), fastfloor(self.z()))
                    .map(|_| (self.xi(), self.yi(), fastfloor(self.z())));
                self.st_mut().current = cur;
            }
            if let Some((cx, cy, _)) = self.st().current {
                let chunk = (
                    crate::jmath::coorddivision(cx, 8),
                    crate::jmath::coorddivision(cy, 8),
                );
                let old = self.st().chunk;
                if old != Some(chunk) {
                    self.st_mut().chunk = Some(chunk);
                    let me = self.rc();
                    if let Some(from) = old {
                        w.vehicle_chunk_changed(&me, from, chunk);
                    } else {
                        w.add_vehicle_to_chunk(&me, chunk);
                    }
                    crate::spawn::add_from_checked_vehicles(&me);
                }
            }
            let current_velocity = self.st().jni_linear_velocity;
            if self.st().jni_is_collide && self.st_mut().limit_crash.check() {
                self.st_mut().jni_is_collide = false;
                self.st_mut().limit_crash.reset();
                let last = self.st().last_linear_velocity;
                let mut change = current_velocity;
                change.sub(last.x, last.y, last.z);
                change.y = 0.0;
                let mut delta = change.length();
                if current_velocity.length_squared() > last.length_squared() && delta > 6.0 {
                    delta = 6.0;
                }
                if delta > 1.0 {
                    if last.length() < 6.0 {
                        delta /= 3.0;
                    }
                    let fwd = self.forward_vector();
                    let dot = change.dot(&fwd);
                    self.crash(delta * 3.0, dot < 0.0);
                    self.damage_objects(delta * 30.0);
                }
            }
            self.st_mut().last_linear_velocity = current_velocity;
        }
        for imp in self
            .st_mut()
            .impulses_from_squished_bodies
            .iter_mut()
            .flatten()
        {
            imp.enable = false;
        }
        // updateSounds / breakingObjects / scrape sounds: sound emitters and
        // the objects around the vehicle (not ported).
        self.breaking_objects();
        if self.st().add_thump_world_sound {
            self.st_mut().add_thump_world_sound = false;
            env::events().event("thumpWorldSound");
        }
        let lightbar_enable = self.has_lightbar();
        if lightbar_enable
            && self.st().lightbar_lights_mode.is_enable()
            && self.battery_charge() > 0.0
        {
            self.st_mut().lightbar_lights_mode.update();
        }
        self.update_world_lights();
        let pcount = self
            .script()
            .map_or(0, |s| s.borrow().passenger_count() as i32);
        for seat in 0..pcount {
            if let Some(chr) = self.character(seat) {
                if let Some(p) = self.passenger_world_pos(seat) {
                    chr.set_position(p.x, p.y, p.z);
                }
            }
        }
        if let Some(lb) = self.part_by_id("lightbar") {
            if self.st().lightbar_lights_mode.is_enable() && lb.condition() == 0 {
                self.set_lightbar_lights_mode(0);
            }
            if self.st().lightbar_siren_mode != 0 && lb.condition() == 0 {
                self.set_lightbar_siren_mode(0);
            }
        }
        let alarm_active = self.st().alarm.start_time > 0.0;
        if self.need_parts_update() || self.st().mechanic_ui_open || alarm_active {
            self.update_parts();
        } else {
            self.drain_battery_update_hack();
        }
        if self.engine_state() == EngineState::Running || towed {
            self.update_bullet_stats();
        }
        if self.st().do_damage_overlay {
            self.st_mut().do_damage_overlay = false;
        }
        if let Some(gas) = self.gas_tank() {
            let cap = gas.container_capacity();
            if gas.container_content_amount() > cap as f32 {
                gas.set_container_content_amount(cap as f32, false, false);
            }
        }
        if self.st().passengers.iter().any(|p| p.character.is_some()) {
            self.update_surround();
        }
        // checkForIntersectingCrops: farming (not ported).
        if self.st().ctrl.is_some() {
            let (id, m) = (self.id() as i32, self.fudged_mass());
            Bullet::call(|b| b.set_vehicle_mass(id, m));
        }
        self.update_velocity_multiplier();
        self.check_trailer_vertical_alignment();
        self.check_trailer_attach_time();
    }

    /// `updateVelocityMultiplier()`.
    fn update_velocity_multiplier(&self) {
        if self.st().ctrl.is_none() {
            return;
        }
        let Some(script) = self.script() else { return };
        let mut v = self.linear_velocity();
        v.y = 0.0;
        let speed = v.length();
        let mut max_speed = 100000.0f32;
        let mut mul = 1.0f32;
        let slow = self.st().breaking_slow_factor;
        if script.borrow().wheel_count() > 0 {
            if speed > 0.0 && speed > 34.0 - slow {
                max_speed = 34.0 - slow;
                mul = (34.0 - slow) / speed;
            }
        } else if self.vehicle_towed_by().is_none() {
            max_speed = 0.0;
            mul = 0.1;
        }
        let id = self.id() as i32;
        Bullet::call(|b| b.set_vehicle_velocity_multiplier(id, max_speed, mul));
    }

    /// `breakingObjects()`: the vehicle hitting world objects and
    /// characters (the cell's objects are not ported); the slow factor stays
    /// at 0 without objects.
    fn breaking_objects(&self) {
        self.st_mut().breaking_slow_factor = 0.0;
    }

    /// `damageObjects(damage)`: world objects (not ported) and other
    /// vehicles.
    pub fn damage_objects(&self, damage: f32) {
        if !self.is_engine_running() {
            return;
        }
        env::events().event(&format!("damageObjects {:x}", damage.to_bits()));
    }

    /// `updateWorldLights()`: the lightbar's world light sources (only the
    /// index state is kept; light sources belong to the lighting port).
    /// `updateWorldLights()`: the lightbar's world lights belong to
    /// rendering (`pz_vehicle_render::lights::LightbarWorldLights`); the
    /// vehicle reports the lightbar state through
    /// [`env::VehicleEvents::lightbar_world_lights`].
    fn update_world_lights(&self) {
        let (mode_on, light) = {
            let st = self.st();
            (
                st.lightbar_lights_mode.is_enable(),
                st.lightbar_lights_mode.light,
            )
        };
        let state = env::LightbarState {
            has_lightbar: self.has_lightbar(),
            enabled: mode_on && self.battery_charge() > 0.0,
            light,
        };
        env::events().lightbar_world_lights(self, state);
    }

    // ---- damage -------------------------------------------------------------

    fn is_driver_god_mode(&self) -> bool {
        self.driver_regardless_of_tow()
            .is_some_and(|d| d.is_god_mod())
    }

    /// `crash(delta, front)` (single player).
    pub fn crash(&self, delta: f32, front: bool) {
        env::events().event("statistic VehicleCrash");
        let modifier = match env::sandbox().car_damage_on_impact {
            1 => 1.9f32,
            2 => 1.6,
            4 => 1.1,
            5 => 0.9,
            _ => 1.3,
        };
        let dmg = delta;
        let delta = delta.abs() / modifier;
        if front {
            self.add_damage_front(delta as i32);
        } else {
            self.add_damage_rear((delta / modifier).abs() as i32);
        }
        self.damage_players(dmg.abs());
        env::events().event(&format!("crashSound {:x}", dmg.to_bits()));
        if let Some(t) = self.vehicle_towing() {
            t.crash(delta, front);
        }
    }

    /// `damagePlayers(damage)`: injuries are the character port's; the
    /// event carries the damage.
    pub fn damage_players(&self, damage: f32) {
        if !env::sandbox().player_damage_from_crash {
            return;
        }
        for seat in 0..self.max_passengers() {
            if let Some(c) = self.character(seat) {
                if !c.is_god_mod() {
                    env::events().event(&format!(
                        "OnPlayerGetDamage CARCRASHDAMAGE {:x}",
                        damage.to_bits()
                    ));
                }
            }
        }
    }

    fn damage_part(&self, id: &str, amount_min: i32, amount_max: i32) {
        if let Some(p) = self.part_by_id(id).filter(|p| p.has_item()) {
            p.damage(rand::next_range(amount_min, amount_max));
        }
    }

    fn add_damage_front(&self, dmg: i32) {
        if self.is_driver_god_mode() {
            return;
        }
        self.st_mut().current_front_end_durability -= dmg;
        let door = self.part_by_id("EngineDoor");
        if let Some(p) = door.as_ref().filter(|p| p.has_item()) {
            p.damage(rand::next_range(1.max(dmg - 5), dmg + 5));
        }
        let door_ok = door
            .as_ref()
            .is_some_and(|p| p.has_item() && p.condition() >= 25);
        if !door_ok {
            if let Some(e) = self.part_by_id("Engine") {
                e.damage(rand::next_range(1.max(dmg - 3), dmg + 3));
            }
        }
        self.damage_part("Windshield", 1.max(dmg - 5), dmg + 5);
        if rand::next(4) == 0 {
            self.damage_part("DoorFrontLeft", 1.max(dmg - 5), dmg + 5);
            self.damage_part("WindowFrontLeft", 1.max(dmg - 5), dmg + 5);
        }
        if rand::next(4) == 0 {
            self.damage_part("DoorFrontRight", 1.max(dmg - 5), dmg + 5);
            self.damage_part("WindowFrontRight", 1.max(dmg - 5), dmg + 5);
        }
        if rand::next(20) < dmg {
            self.damage_headlight("HeadlightLeft", dmg);
        }
        if rand::next(20) < dmg {
            self.damage_headlight("HeadlightRight", dmg);
        }
    }

    fn add_damage_rear(&self, dmg: i32) {
        if self.is_driver_god_mode() {
            return;
        }
        self.st_mut().current_rear_end_durability -= dmg;
        if let Some(p) = self.part_by_id("TruckBed").filter(|p| p.has_item()) {
            p.set_condition(p.condition() - rand::next_range(1.max(dmg - 5), dmg + 5));
            let item = p.st().item.clone();
            p.do_inventory_item_stats(item.as_ref(), 0);
        }
        self.damage_part("DoorRear", 1.max(dmg - 5), dmg + 5);
        self.damage_part("TrunkDoor", 1.max(dmg - 5), dmg + 5);
        if let Some(p) = self.part_by_id("WindshieldRear").filter(|p| p.has_item()) {
            p.damage(dmg);
        }
        if rand::next(4) == 0 {
            self.damage_part("DoorRearLeft", 1.max(dmg - 5), dmg + 5);
            self.damage_part("WindowRearLeft", 1.max(dmg - 5), dmg + 5);
        }
        if rand::next(4) == 0 {
            self.damage_part("DoorRearRight", 1.max(dmg - 5), dmg + 5);
            self.damage_part("WindowRearRight", 1.max(dmg - 5), dmg + 5);
        }
        if rand::next(20) < dmg {
            self.damage_headlight("HeadlightRearLeft", dmg);
        }
        if rand::next(20) < dmg {
            self.damage_headlight("HeadlightRearRight", dmg);
        }
        if rand::next(20) < dmg {
            self.damage_part("Muffler", 1.max(dmg - 5), dmg + 5);
        }
    }

    fn damage_headlight(&self, id: &str, dmg: i32) {
        if self.is_driver_god_mode() {
            return;
        }
        if let Some(p) = self.part_by_id(id).filter(|p| p.has_item()) {
            p.damage(dmg);
            if p.condition() <= 0 {
                p.set_inventory_item(None, 0);
            }
        }
    }

    /// `getBloodIntensity(id)`.
    pub fn blood_intensity(&self, id: &str) -> f32 {
        let v = self
            .st()
            .blood_intensity
            .iter()
            .find(|(k, _)| k == id)
            .map_or(0, |e| e.1);
        (v as u8) as f32 / 100.0
    }

    /// `setBloodIntensity(id, intensity)`.
    pub fn set_blood_intensity(&self, id: &str, intensity: f32) {
        let v = (jmath::clamp(intensity, 0.0, 1.0) * 100.0) as i32 as i8;
        let mut st = self.st_mut();
        match st.blood_intensity.iter_mut().find(|(k, _)| k == id) {
            Some(e) if e.1 == v => {}
            Some(e) => e.1 = v,
            None => st.blood_intensity.push((id.to_string(), v)),
        }
    }

    // ---- towing (the parts that do not need the attach UI) ----------------

    /// `breakConstraint(forgetID, remote)`.
    pub fn break_constraint(&self, forget_id: bool, _remote: bool) {
        let c = self.st().constraint_towing;
        if c == -1 {
            return;
        }
        Bullet::call(|b| b.remove_constraint(c));
        self.st_mut().constraint_towing = -1;
        if let Some(t) = self.vehicle_towing() {
            t.st_mut().vehicle_towed_by = None;
            t.st_mut().constraint_towing = -1;
            if forget_id {
                self.st_mut().vehicle_towing_id = -1;
                t.st_mut().vehicle_towed_by_id = -1;
            }
            t.constraint_changed();
            self.st_mut().vehicle_towing = None;
        }
        if let Some(by) = self.vehicle_towed_by() {
            by.st_mut().vehicle_towing = None;
            by.st_mut().constraint_towing = -1;
            if forget_id {
                by.st_mut().vehicle_towing_id = -1;
                self.st_mut().vehicle_towed_by_id = -1;
            }
            by.constraint_changed();
            self.st_mut().vehicle_towed_by = None;
        }
        self.constraint_changed();
    }

    /// `constraintChanged()`.
    pub fn constraint_changed(&self) {
        let now = env::now_ms();
        self.set_physics_active(true, true);
        self.st_mut().constraint_changed_time = now;
    }

    /// `tryReconnectToTowedVehicle()`: finding the towed vehicle by its sql
    /// id among the cell's vehicles.
    fn try_reconnect_to_towed_vehicle(&self) {
        if self.vehicle_towing().is_some() {
            return;
        }
        let want = self.st().vehicle_towing_id;
        if want == -1 {
            return;
        }
        let other = crate::manager::cell_vehicles()
            .into_iter()
            .find(|v| v.st().sql_id == want);
        if let Some(o) = other {
            if !Rc::ptr_eq(&o, &self.rc()) {
                env::events().event("tryReconnectToTowedVehicle");
            }
        }
    }

    fn check_trailer_vertical_alignment(&self) {
        if self.st().constraint_towing != -1 && self.up_vector_dot() < 0.8 {
            env::events().event("playSound BreakMetalItem");
            self.break_constraint(true, false);
        }
    }

    fn check_trailer_attach_time(&self) {
        let c = self.st().constraint_towing;
        if c == -1 {
            return;
        }
        let begin = self.st().begin_attach_trailer_ms;
        if begin <= 0 {
            self.st_mut().begin_attach_trailer_ms = 0;
            return;
        }
        if begin + 2000 <= env::now_ms() {
            Bullet::call(|b| {
                for axis in 0..6 {
                    b.set_constraint_erp(c, 0.2, axis)?;
                }
                Ok(())
            });
            self.st_mut().begin_attach_trailer_ms = 0;
        }
    }

    // ---- world --------------------------------------------------------------

    /// `addToWorld()` (single player): `createPhysics`, the engine's idle
    /// speed and the siren start time. Key spawning
    /// (`trySpawnKey`) needs the world's containers and zombies and is not
    /// ported.
    pub fn add_to_world(&self) {
        if self.st().added_to_world {
            return;
        }
        // `VehiclesDB2.setVehicleLoaded`: a vehicle without an id gets one.
        if let Some(ids) = env::vehicle_ids() {
            let id = ids.borrow_mut().set_vehicle_loaded(self.st().sql_id);
            self.st_mut().sql_id = id;
        }
        {
            let mut st = self.st_mut();
            st.added_to_world = true;
            st.removed_from_world = false;
        }
        self.create_physics(false);
        // `VirtualVehicleManager.removeFromMeta` (single player).
        crate::virtual_vehicle::with_manager(|m| m.remove_from_meta(self));
        if self.st().lightbar_siren_mode != 0 {
            let m = self.st().lightbar_siren_mode;
            self.set_lightbar_siren_mode(m);
            if self.st().siren_start_time <= 0.0 {
                self.st_mut().siren_start_time = env::time().world_age_hours;
            }
        }
        if self.engine_part().is_some() && self.with_engine(|e| e.state) != EngineState::Idle {
            let idle = self
                .script()
                .map_or(1000.0, |s| s.borrow().engine_idle_speed() as f64);
            self.with_engine_mut(|e| e.speed = idle);
        }
    }

    /// `removeFromWorld()` (single player).
    pub fn remove_from_world(&self) {
        self.break_constraint(false, false);
        // `VehiclesDB2.setVehicleUnloaded`.
        if let Some(ids) = env::vehicle_ids() {
            ids.borrow_mut().set_vehicle_unloaded(self.st().sql_id);
        }
        // A local player's vehicle stays.
        if self
            .st()
            .passengers
            .iter()
            .any(|p| p.character.as_ref().is_some_and(|c| c.is_player()))
        {
            return;
        }
        crate::spawn::remove_from_checked_vehicles(self);
        if self.st().removed_from_world {
            return;
        }
        {
            let mut st = self.st_mut();
            st.removed_from_world = true;
            st.added_to_world = false;
        }
        // `parts.removeFromWorld()`, `removeVehicleSounds`, the emitter.
        env::events().event("removeVehicleSounds");
        if self.st().ctrl.is_some() {
            let id = self.id() as i32;
            Bullet::call(|b| b.remove_vehicle(id));
            self.st_mut().ctrl = None;
        }
        let id = self.id();
        if id != -1 {
            crate::manager::id_map_remove(id);
        }
        crate::manager::cell_remove(self);
        // `PathfindNative.removeVehicle` / `PolygonalMap2.removeVehicleFromWorld`.
        env::events().event("pathfindRemoveVehicle");
        self.st_mut().surround.reset();
        // `removeWorldLights()`.
        env::events().lightbar_world_lights(self, env::LightbarState::default());
    }

    /// `exit(chr)`.
    pub fn exit(&self, chr: &CharRef) -> bool {
        let seat = self.seat_of(chr);
        if seat == -1 {
            return false;
        }
        if !self.clear_passenger(seat) {
            return false;
        }
        chr.exited_vehicle();
        if self.driver().is_none() && self.st().sound_horn_on {
            self.st_mut().sound_horn_on = false;
        }
        let mut st = self.st_mut();
        st.poly_garage_check = true;
        st.poly_dirty = true;
        true
    }

    /// `permanentlyRemove()`: everyone out, the tow link broken, out of the
    /// world, off its square and chunk, and out of `vehicles.db`.
    pub fn permanently_remove(&self) {
        for seat in 0..self.max_passengers() {
            if let Some(chr) = self.character(seat) {
                self.exit(&chr);
            }
        }
        self.break_constraint(true, false);
        self.remove_from_world();
        // `removeFromSquare()`.
        self.st_mut().current = None;
        if let Some(chunk) = self.st().chunk {
            env::world().remove_vehicle_from_chunk(&self.rc(), chunk);
        }
        env::events().vehicle_removed_from_database(self);
    }

    /// `setAddThumpWorldSound(add)` (a zombie hit the bodywork).
    pub fn set_add_thump_world_sound(&self, add: bool) {
        self.st_mut().add_thump_world_sound = add;
    }

    /// `getNearestBodyworkPart(chr)`: the first door or bodywork part with
    /// condition left whose area holds `(x, y)` (the character's position).
    pub fn nearest_bodywork_part(&self, x: f32, y: f32) -> Option<crate::PartRef> {
        self.parts().into_iter().find(|p| {
            let cat = p.st().category.clone();
            let ok = matches!(cat.as_deref(), Some("door") | Some("bodywork"));
            ok && p.area().is_some_and(|a| self.is_in_area(&a, x, y)) && p.condition() > 0
        })
    }

    /// `transmitPartCondition(part)` / `transmitPartItem(part)`: server
    /// sync only (nothing in single player).
    pub fn transmit_part_condition(&self, _part: &crate::PartRef) {}
    pub fn transmit_part_item(&self, _part: &crate::PartRef) {}

    /// `chooseBestAttackPosition(target, attacker, out)`: `target_vehicle`
    /// is `target.getVehicle()`; animals get `None`.
    pub fn choose_best_attack_position(
        &self,
        target_vehicle: &BaseVehicle,
        attacker: &crate::surround::SurroundZombie,
        attacker_is_animal: bool,
    ) -> Option<(f32, f32, f32)> {
        if attacker_is_animal {
            return None;
        }
        let w = env::world();
        let lookup = |k: u64| w.surround_zombie(k);
        let (x, y) = target_vehicle.surround_position_for_zombie(attacker, &lookup)?;
        Some((x, y, self.z()))
    }

    /// `getSurroundVehicle().getPositionForZombie(zombie, out)`.
    pub fn surround_position_for_zombie(
        &self,
        z: &crate::surround::SurroundZombie,
        lookup: &dyn Fn(u64) -> Option<crate::surround::SurroundZombie>,
    ) -> Option<(f32, f32)> {
        let mut s = std::mem::take(&mut self.st_mut().surround);
        let r = s.position_for_zombie(self, z, lookup);
        self.st_mut().surround = s;
        r
    }

    /// `getSurroundVehicle().update()`.
    pub fn update_surround(&self) {
        let w = env::world();
        let lookup = |k: u64| w.surround_zombie(k);
        let mut s = std::mem::take(&mut self.st_mut().surround);
        s.update(self, &lookup);
        self.st_mut().surround = s;
    }
}

impl BaseVehicle {
    /// `applyAccumulatedImpulsesFromHitObjectsToPhysics()`.
    pub fn apply_accumulated_impulses_from_hit_objects(&self) {
        let imps = std::mem::take(&mut self.st_mut().impulses_from_hit_objects);
        if imps.is_empty() {
            return;
        }
        let mut force = Vector3f::default();
        let mut torque = Vector3f::default();
        for i in &imps {
            force.add(i.impulse.x, i.impulse.y, i.impulse.z);
            let c = i.rel_pos.cross(&i.impulse);
            torque.add(c.x, c.y, c.z);
        }
        let limit = 1500.0 * self.fudged_mass();
        if force.length_squared() > limit * limit {
            let s = limit / force.length();
            force.mul(s);
            torque.mul(s);
        }
        let id = self.id() as i32;
        Bullet::call(|b| {
            b.apply_central_force_to_vehicle(id, force.x * 30.0, force.y * 30.0, force.z * 30.0)?;
            b.apply_torque_to_vehicle(id, torque.x * 30.0, torque.y * 30.0, torque.z * 30.0)
        });
    }

    /// `applyAllImpulsesFromProneCharacters()`.
    pub fn apply_all_impulses_from_prone_characters(&self) {
        let has = self
            .st()
            .impulses_from_squished_bodies
            .iter()
            .flatten()
            .any(|i| i.enable);
        if !has {
            return;
        }
        let mut force = Vector3f::default();
        let mut torque = Vector3f::default();
        for i in self
            .st_mut()
            .impulses_from_squished_bodies
            .iter_mut()
            .flatten()
        {
            if !i.enable || i.applied {
                continue;
            }
            force.add(i.impulse.x, i.impulse.y, i.impulse.z);
            let c = i.rel_pos.cross(&i.impulse);
            torque.add(c.x, c.y, c.z);
            i.applied = true;
        }
        if force.length_squared() > 0.0 {
            let limit = self.fudged_mass() * 0.15;
            if force.length_squared() > limit * limit {
                force.mul(limit / force.length());
            }
            let id = self.id() as i32;
            Bullet::call(|b| {
                b.apply_central_force_to_vehicle(
                    id,
                    force.x * 30.0,
                    force.y * 30.0,
                    force.z * 30.0,
                )?;
                b.apply_torque_to_vehicle(id, torque.x * 30.0, torque.y * 30.0, torque.z * 30.0)
            });
        }
    }

    /// `setTireRemoved(wheelIndex, removed)`.
    pub fn set_tire_removed(&self, wheel_index: i32, removed: bool) {
        let id = self.id() as i32;
        Bullet::call(|b| b.set_tire_removed(id, wheel_index, removed));
    }

    /// `setTireInflation(wheelIndex, inflation)`.
    pub fn set_tire_inflation(&self, wheel_index: i32, inflation: f32) {
        let id = self.id() as i32;
        Bullet::call(|b| b.set_tire_inflation(id, wheel_index, inflation));
    }
}

#[cfg(test)]
mod removal_tests {
    use super::*;
    use std::cell::RefCell;

    struct World(RefCell<Vec<(i32, i32)>>);
    impl env::VehicleWorld for World {
        fn square(&self, _: i32, _: i32, _: i32) -> Option<env::SquareInfo> {
            None
        }
        fn is_valid_chunk(&self, _: i32, _: i32) -> bool {
            false
        }
        fn chunk_loaded(&self, _: i32, _: i32) -> bool {
            false
        }
        fn remove_vehicle_from_chunk(&self, _v: &VehicleRef, chunk: (i32, i32)) {
            self.0.borrow_mut().push(chunk);
        }
    }

    #[test]
    fn permanently_remove_leaves_world_and_chunk() {
        let w = Rc::new(World(RefCell::new(Vec::new())));
        env::set_world(w.clone());
        let v = BaseVehicle::new();
        {
            let mut st = v.st_mut();
            st.added_to_world = true;
            st.chunk = Some((3, 4));
            st.current = Some((24, 32, 0));
        }
        crate::manager::add_to_world(&v);
        v.permanently_remove();
        assert!(v.st().removed_from_world);
        assert!(!v.st().added_to_world);
        assert!(v.st().current.is_none());
        assert_eq!(*w.0.borrow(), vec![(3, 4)]);
        assert!(!crate::manager::cell_vehicles()
            .iter()
            .any(|o| Rc::ptr_eq(o, &v)));
        env::set_world(Rc::new(env::OpenWorld));
    }
}

#[cfg(test)]
mod signal_device_tests {
    use super::*;
    use pz_inventory::device::DeviceData;
    use pz_inventory::device_sim::{
        DeviceEnv, DeviceEvent, IsoObjectPos, MainsPower, MediaLine, ParentKind,
    };
    use std::cell::RefCell;

    type Log = Rc<RefCell<Vec<DeviceEvent>>>;

    struct World(Log);
    impl DeviceEnv for World {
        fn minutes_stamp(&self) -> i64 {
            60
        }
        fn multiplier(&self) -> f32 {
            1.0
        }
        fn parent(&self) -> ParentKind {
            ParentKind::None
        }
        fn next_int(&mut self, _bound: i32) -> i32 {
            0
        }
        fn next_float(&mut self) -> f32 {
            0.0
        }
        fn mains_power(&self) -> MainsPower {
            MainsPower::Square(None)
        }
        fn channel_list(&self, _c: &str) -> Option<Vec<(i32, String)>> {
            None
        }
        fn media_exists(&self, _i: i16) -> bool {
            false
        }
        fn media_line(&self, _i: i16, _l: i32) -> Option<MediaLine> {
            None
        }
        fn iso_object(&self) -> Option<IsoObjectPos> {
            None
        }
        fn closest_listener_distance_sq(&self, _x: f32, _y: f32, _z: f32) -> f32 {
            f32::MAX
        }
        fn parent_pos(&self) -> [f32; 3] {
            [0.0; 3]
        }
        fn event(&mut self, e: DeviceEvent) {
            self.0.borrow_mut().push(e);
        }
    }

    struct Devices(Log);
    impl env::VehicleDevices for Devices {
        fn device_env(&self, _v: &BaseVehicle, _p: &PartRef) -> Option<Box<dyn DeviceEnv>> {
            Some(Box::new(World(self.0.clone())))
        }
    }

    #[test]
    fn unpowered_vehicle_radio_turns_off() {
        let log: Log = Rc::default();
        env::set_devices(Rc::new(Devices(log.clone())));
        let v = BaseVehicle::new();
        let part = crate::part::VehiclePart::new(&v);
        part.st_mut().device_data = Some(DeviceData {
            is_turned_on: true,
            is_battery_powered: false,
            ..Default::default()
        });
        v.update_signal_device(&part);
        let d = part.st().device_data.clone().unwrap();
        assert!(!d.is_turned_on);
        assert_eq!(d.last_minute_stamp, -1);
        assert!(!part.st().has_player_in_range);
        // Installed (no item type), so `update` turns it off without
        // `setIsTurnedOn`; the chat element still updates.
        assert_eq!(
            *log.borrow(),
            vec![DeviceEvent::ChatUpdate {
                history_range: 0.6 * 20.0
            }]
        );
        env::set_devices(Rc::new(NoDevicesForTest));
    }

    struct NoDevicesForTest;
    impl env::VehicleDevices for NoDevicesForTest {}
}
