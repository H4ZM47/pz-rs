//! Map objects: `IsoObject` and the `zombie.iso.objects` classes that
//! `CellLoader.DoTileObjectCreation` creates, with the fields map loading
//! sets.
//!
//! Each object is one [`IsoObject`] holding the fields every object has and
//! an [`ObjectKind`] with the class-specific state. Rendering, sound,
//! networking and update-loop state are not represented.

use crate::defs::RoomDef;
use crate::save::items::ItemBlob;
use crate::save::lua_table::LuaTable;
use crate::save::movers::{AnimalTracks, IsoPushableObject};
use crate::save::visual::{HumanVisual, ItemVisual};
use crate::sprite::SpriteIdx;

/// Identity of an object within its square (stands in for Java object
/// identity, e.g. for `specialObjects`).
pub type ObjectId = u32;

pub use pz_inventory::container::ItemContainer;

/// `IsoLightSource` (map-loading fields).
#[derive(Clone, Debug, PartialEq)]
pub struct IsoLightSource {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub radius: i32,
    pub active: bool,
    pub hydro_powered: bool,
}

/// `IsoDoor`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IsoDoor {
    pub north: bool,
    pub open: bool,
    pub locked: bool,
    pub locked_by_key: bool,
    pub health: i32,
    pub max_health: i32,
    pub pushed_strength: i32,
    pub pushed_max_strength: i32,
    pub open_sprite: Option<SpriteIdx>,
    pub closed_sprite: Option<SpriteIdx>,
    pub has_curtain: bool,
    pub curtain_open: bool,
    pub curtain_inside: bool,
}

/// `IsoCurtain`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IsoCurtain {
    pub north: bool,
    pub open: bool,
    pub health: i32,
    pub max_health: i32,
    pub pushed_strength: i32,
    pub pushed_max_strength: i32,
    pub open_sprite: Option<SpriteIdx>,
    pub closed_sprite: Option<SpriteIdx>,
    pub barricade_strength: i32,
}

/// `IsoWindow`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IsoWindow {
    pub north: bool,
    pub locked: bool,
    pub perma_locked: bool,
    pub open: bool,
    pub destroyed: bool,
    pub glass_removed: bool,
    pub health: i32,
    pub max_health: i32,
    pub open_sprite: Option<SpriteIdx>,
    pub closed_sprite: Option<SpriteIdx>,
    pub smashed_sprite: Option<SpriteIdx>,
    pub glass_removed_sprite: Option<SpriteIdx>,
}

/// `IsoLightSwitch`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoLightSwitch {
    pub activated: bool,
    pub light_room: bool,
    pub room_id: i64,
    pub street_light: bool,
    pub can_be_modified: bool,
    pub primary_r: f32,
    pub primary_g: f32,
    pub primary_b: f32,
    pub lights: Vec<IsoLightSource>,
    pub use_battery: bool,
    pub has_battery: bool,
    /// `bulbItem`; `hasLightBulb()` is `bulbItem != null`.
    pub bulb_item: Option<String>,
    pub power: f32,
    pub delta: f32,
    pub last_minute_stamp: i64,
    pub bulb_burn_minutes: i32,
}

impl Default for IsoLightSwitch {
    /// Field initialisers of `IsoLightSwitch`.
    fn default() -> Self {
        IsoLightSwitch {
            activated: false,
            light_room: false,
            room_id: -1,
            street_light: false,
            can_be_modified: false,
            primary_r: 1.0,
            primary_g: 1.0,
            primary_b: 1.0,
            lights: Vec::new(),
            use_battery: false,
            has_battery: false,
            bulb_item: Some("Base.LightBulb".to_string()),
            power: 0.0,
            delta: 2.5e-4,
            last_minute_stamp: -1,
            bulb_burn_minutes: -1,
        }
    }
}

/// `IsoBarbecue`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoBarbecue {
    pub has_propane_tank: bool,
    pub fuel_amount_minutes: i32,
    pub normal_sprite: Option<SpriteIdx>,
    pub no_tank_sprite: Option<SpriteIdx>,
    pub lit: bool,
    pub last_update_time: f32,
    pub minutes_since_extinguished: i32,
    /// `isSmouldering` (not saved).
    pub smouldering: bool,
    /// `minuteAccumulator` (not saved).
    pub minute_accumulator: f32,
}

impl Default for IsoBarbecue {
    fn default() -> Self {
        IsoBarbecue {
            has_propane_tank: false,
            fuel_amount_minutes: 0,
            normal_sprite: None,
            no_tank_sprite: None,
            lit: false,
            last_update_time: -1.0,
            minutes_since_extinguished: -1,
            smouldering: false,
            minute_accumulator: 0.0,
        }
    }
}

/// `IsoFireplace`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoFireplace {
    pub fuel_amount_minutes: i32,
    pub lit: bool,
    pub last_update_time: f32,
    pub minutes_since_extinguished: i32,
    /// `smouldering` (not saved).
    pub smouldering: bool,
    /// `minuteAccumulator` (not saved).
    pub minute_accumulator: f32,
}

impl Default for IsoFireplace {
    fn default() -> Self {
        IsoFireplace {
            fuel_amount_minutes: 0,
            lit: false,
            last_update_time: -1.0,
            minutes_since_extinguished: -1,
            smouldering: false,
            minute_accumulator: 0.0,
        }
    }
}

/// `IsoStove`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoStove {
    pub activated: bool,
    pub seconds_timer: i32,
    pub max_temperature: f32,
    pub first_turn_on: bool,
    pub broken: bool,
}

impl Default for IsoStove {
    fn default() -> Self {
        IsoStove {
            activated: false,
            seconds_timer: -1,
            max_temperature: 0.0,
            first_turn_on: true,
            broken: false,
        }
    }
}

/// `ClothingWasherLogic`.
#[derive(Clone, Debug, PartialEq)]
pub struct WasherLogic {
    pub activated: bool,
    pub last_update: f32,
}

impl Default for WasherLogic {
    fn default() -> Self {
        WasherLogic {
            activated: false,
            last_update: -1.0,
        }
    }
}

/// `ClothingDryerLogic`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DryerLogic {
    pub activated: bool,
}

/// `IsoCombinationWasherDryer` / `IsoStackedWasherDryer`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WasherDryer {
    /// `logic == dryer` (combination machines only).
    pub dryer_mode: bool,
    pub washer: WasherLogic,
    pub dryer: DryerLogic,
}

/// `IsoTree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoTree {
    pub size: i32,
    pub log_yield: i32,
    pub damage: i32,
}

pub use pz_inventory::device::{DeviceData, DevicePresets, PresetEntry};

/// `IsoWorldInventoryObject`: an item lying on the floor.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoWorldInventoryObject {
    pub xoff: f32,
    pub yoff: f32,
    pub zoff: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub item: crate::floor_item::FloorItem,
    pub drop_time: f64,
    pub ignore_remove_sandbox: bool,
    pub extended_placement: bool,
}

/// `IsoThumpable` (player-built objects).
#[derive(Clone, Debug, PartialEq)]
pub struct IsoThumpable {
    pub open: bool,
    pub locked: bool,
    pub north: bool,
    pub max_health: i32,
    pub health: i32,
    pub closed_sprite: Option<SpriteIdx>,
    pub open_sprite: Option<SpriteIdx>,
    pub thump_dmg: i32,
    pub is_door: bool,
    pub is_door_frame: bool,
    pub is_corner: bool,
    pub is_stairs: bool,
    pub is_container: bool,
    pub is_floor: bool,
    pub can_barricade: bool,
    pub can_pass_through: bool,
    pub dismantable: bool,
    pub can_be_plastered: bool,
    pub paintable: bool,
    pub cross_speed: f32,
    /// `table` (separate from the `IsoObject` mod data).
    pub table: Option<LuaTable>,
    /// `modData`.
    pub mod_data: Option<LuaTable>,
    pub block_all_the_square: bool,
    pub is_thumpable: bool,
    pub is_hoppable: bool,
    pub light_source_life: i32,
    pub light_source_radius: i32,
    pub light_source_x_offset: i32,
    pub light_source_y_offset: i32,
    /// `lightSourceFuel` as its item registry id.
    pub light_source_fuel: Option<i16>,
    pub life_delta: f32,
    pub life_left: f32,
    pub locked_by_key: bool,
    pub locked_by_padlock: bool,
    pub can_be_lock_by_padlock: bool,
    pub locked_by_code: i32,
    pub thump_sound: String,
    pub last_update_hours: f32,
    pub have_fuel: bool,
    pub light_source_on: bool,
    pub build_materials: Option<LuaTable>,
}

impl Default for IsoThumpable {
    /// `new IsoThumpable(cell)`.
    fn default() -> Self {
        IsoThumpable {
            open: false,
            locked: false,
            north: false,
            max_health: 500,
            health: 500,
            closed_sprite: None,
            open_sprite: None,
            thump_dmg: 8,
            is_door: false,
            is_door_frame: false,
            is_corner: false,
            is_stairs: false,
            is_container: false,
            is_floor: false,
            can_barricade: false,
            can_pass_through: false,
            dismantable: false,
            can_be_plastered: false,
            paintable: false,
            cross_speed: 1.0,
            table: None,
            mod_data: None,
            block_all_the_square: false,
            is_thumpable: true,
            is_hoppable: false,
            light_source_life: -1,
            light_source_radius: -1,
            light_source_x_offset: 0,
            light_source_y_offset: 0,
            light_source_fuel: None,
            life_delta: 0.0,
            life_left: -1.0,
            locked_by_key: false,
            locked_by_padlock: false,
            can_be_lock_by_padlock: false,
            locked_by_code: 0,
            thump_sound: "ZombieThumpGeneric".to_string(),
            last_update_hours: -1.0,
            have_fuel: false,
            light_source_on: false,
            build_materials: None,
        }
    }
}

/// `IsoCompost`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoCompost {
    pub compost: f32,
    pub last_updated: f32,
    pub health: i32,
    pub max_health: i32,
}

/// `IsoFeedingTrough`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoFeedingTrough {
    /// `feedingTypes` (a `HashMap<String, Float>`) in insertion order.
    pub feeding_types: Vec<(String, f32)>,
    pub water: f32,
    pub linked_x: i32,
    pub linked_y: i32,
    pub north: bool,
    /// `maxWater` and `maxFeed` from `FeedingTroughDef` (not saved; 0 when
    /// the definition is unknown or the trough is a slave).
    pub max_water: f32,
    pub max_feed: i32,
}

/// `IsoBarricade` (saved without the `IsoObject` fields).
#[derive(Clone, Debug, PartialEq)]
pub struct IsoBarricade {
    /// `getForwardIsoDirection().ordinal()`.
    pub dir: i8,
    pub plank_health: [i32; 4],
    pub metal_health: i32,
    pub metal_bar_health: i32,
}

/// `IsoHutch.NestBox`.
#[derive(Clone, Debug, PartialEq)]
pub struct NestBox {
    /// Eggs (`Food` items).
    pub eggs: Vec<ItemBlob>,
    /// The animal laying in it.
    pub animal: Option<crate::save::animal::IsoAnimal>,
}

/// `IsoHutch` state saved by the master square.
#[derive(Clone, Debug, PartialEq)]
pub struct HutchMaster {
    pub sprite_name: String,
    pub open: bool,
    pub open_egg_hatch: bool,
    pub saved_x: i32,
    pub saved_y: i32,
    pub saved_z: i32,
    /// `animalInside`: hutch position and animal, in insertion order (saved
    /// in `HashMap<Integer, IsoAnimal>` order).
    pub animals: Vec<(i32, crate::save::animal::IsoAnimal)>,
    pub hutch_dirt: f32,
    pub nest_box_dirt: f32,
    pub nest_boxes: Vec<NestBox>,
}

/// `IsoHutch`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoHutch {
    pub linked_x: i32,
    pub linked_y: i32,
    pub linked_z: i32,
    /// `None` for a slave square (`linkedX > 0 && linkedY > 0`).
    pub master: Option<HutchMaster>,
}

/// `IsoGenerator`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoGenerator {
    pub connected: bool,
    pub activated: bool,
    pub fuel: f32,
    pub condition: i32,
    pub last_hour: i32,
    pub total_power_using: f32,
}

/// `IsoTrap`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoTrap {
    pub sensor_range: i32,
    pub fire_starting_chance: i32,
    pub fire_starting_energy: i32,
    pub fire_range: i32,
    pub explosion_power: i32,
    pub explosion_range: i32,
    pub explosion_duration: i32,
    pub explosion_start_time: f32,
    pub smoke_range: i32,
    pub noise_range: i32,
    pub noise_duration: i32,
    pub noise_start_time: f32,
    pub extra_damage: f32,
    pub remote_control_id: i32,
    pub timer_before_explosion: i32,
    pub count_down_sound: String,
    pub explosion_sound: String,
    pub weapon: Option<ItemBlob>,
}

/// `IsoCarBatteryCharger`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoCarBatteryCharger {
    pub item: Option<ItemBlob>,
    pub battery: Option<ItemBlob>,
    pub activated: bool,
    pub last_update: f32,
    pub charge_rate: f32,
}

pub use crate::fire::IsoFire;

/// `IsoMannequin`.
#[derive(Debug)]
pub struct IsoMannequin {
    pub dir: i8,
    pub init: bool,
    pub female: bool,
    pub zombie: bool,
    pub skeleton: bool,
    pub mannequin_script_name: Option<String>,
    pub pose: Option<String>,
    pub human_visual: HumanVisual,
    /// Runtime script/model properties resolved by `initOutfit` and
    /// `syncModel`; the chunk codec does not write these fields separately.
    pub model_script_name: Option<String>,
    pub texture_name: Option<String>,
    pub anim_set: Option<String>,
    pub anim_state: Option<String>,
    pub outfit: Option<String>,
    pub item_visuals: Vec<ItemVisual>,
    /// A loaded visual without an explicit texture needs the real host skin
    /// list/RNG getter before activation. No placeholder style is selected.
    pub pending_loaded_skin_resolution: bool,
    /// The mannequin's container (saved after the visual, not with the
    /// `IsoObject` containers) and its worn items as
    /// `(body location, index into the saved items)`.
    pub container: Option<ItemContainer>,
    pub worn_items: Vec<(String, i16)>,
    /// Once installed this is authoritative; legacy codec fields above are
    /// cleared. Normal factory-created clothing keeps its actual owner.
    pub runtime_container: Option<pz_inventory::shared::ContainerRef>,
    /// The real host attaches its canonical Human-group worn collection.
    /// It remains absent until that dependency is available, rather than
    /// creating a separate body-location registry in the world codec.
    pub runtime_worn_items: Option<pz_inventory::worn_items::InventoryWornItemsRef>,
}

/// The class of a map object and its class-specific state.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// Plain `IsoObject` (floors, walls, stairs, furniture, ...).
    Object,
    Door(IsoDoor),
    LightSwitch(IsoLightSwitch),
    Curtain(IsoCurtain),
    Window(IsoWindow),
    WindowFrame {
        north: bool,
    },
    Barbecue(IsoBarbecue),
    /// `IsoFireplace`: fireplaces, campfires, braziers and wood stoves.
    Fireplace(IsoFireplace),
    CombinationWasherDryer(WasherDryer),
    StackedWasherDryer(WasherDryer),
    ClothingDryer(DryerLogic),
    ClothingWasher(WasherLogic),
    Stove(IsoStove),
    Jukebox,
    /// `IsoRadio` (an `IsoWaveSignal`).
    Radio(DeviceData),
    /// `IsoTelevision` (an `IsoWaveSignal`).
    Television(DeviceData),
    /// `IsoMannequin` always owns its constructor visual/worn state.
    /// `init` distinguishes whether `initOutfit` has run.
    Mannequin(Box<IsoMannequin>),
    Tree(IsoTree),
    WorldInventory(crate::floor_object::FloorObject),
    Thumpable(Box<IsoThumpable>),
    BrokenGlass,
    ButcherHook,
    Compost(IsoCompost),
    FeedingTrough(IsoFeedingTrough),
    Barricade(IsoBarricade),
    Hutch(Box<IsoHutch>),
    Generator(IsoGenerator),
    Trap(Box<IsoTrap>),
    CarBatteryCharger(Box<IsoCarBatteryCharger>),
    Fire(IsoFire),
    /// `IsoAnimalTrack`.
    AnimalTrack(Box<AnimalTracks>),
    /// `IsoPushableObject` or `IsoWheelieBin` in a square's object list
    /// (saved as a moving object; the base `IsoObject` fields other than
    /// `sprite` and `container` are unused).
    Pushable(Box<IsoPushableObject>),
}

impl ObjectKind {
    /// The Java class name.
    pub fn class_name(&self) -> &'static str {
        match self {
            ObjectKind::Object => "IsoObject",
            ObjectKind::Door(_) => "IsoDoor",
            ObjectKind::LightSwitch(_) => "IsoLightSwitch",
            ObjectKind::Curtain(_) => "IsoCurtain",
            ObjectKind::Window(_) => "IsoWindow",
            ObjectKind::WindowFrame { .. } => "IsoWindowFrame",
            ObjectKind::Barbecue(_) => "IsoBarbecue",
            ObjectKind::Fireplace(_) => "IsoFireplace",
            ObjectKind::CombinationWasherDryer(_) => "IsoCombinationWasherDryer",
            ObjectKind::StackedWasherDryer(_) => "IsoStackedWasherDryer",
            ObjectKind::ClothingDryer(_) => "IsoClothingDryer",
            ObjectKind::ClothingWasher(_) => "IsoClothingWasher",
            ObjectKind::Stove(_) => "IsoStove",
            ObjectKind::Jukebox => "IsoJukebox",
            ObjectKind::Radio(_) => "IsoRadio",
            ObjectKind::Television(_) => "IsoTelevision",
            ObjectKind::Mannequin(_) => "IsoMannequin",
            ObjectKind::Tree(_) => "IsoTree",
            ObjectKind::WorldInventory(_) => "IsoWorldInventoryObject",
            ObjectKind::Thumpable(_) => "IsoThumpable",
            ObjectKind::BrokenGlass => "IsoBrokenGlass",
            ObjectKind::ButcherHook => "IsoButcherHook",
            ObjectKind::Compost(_) => "IsoCompost",
            ObjectKind::FeedingTrough(_) => "IsoFeedingTrough",
            ObjectKind::Barricade(_) => "IsoBarricade",
            ObjectKind::Hutch(_) => "IsoHutch",
            ObjectKind::Generator(_) => "IsoGenerator",
            ObjectKind::Trap(_) => "IsoTrap",
            ObjectKind::CarBatteryCharger(_) => "IsoCarBatteryCharger",
            ObjectKind::Fire(_) => "IsoFire",
            ObjectKind::AnimalTrack(_) => "IsoAnimalTrack",
            ObjectKind::Pushable(p) if p.wheelie_bin => "IsoWheelieBin",
            ObjectKind::Pushable(_) => "IsoPushableObject",
        }
    }

    /// `getObjectName()` of the subclasses that override it; `None` for a
    /// plain `IsoObject`, whose name comes from `name` or the sprite.
    pub fn object_name(&self) -> Option<&'static str> {
        Some(match self {
            ObjectKind::Object => return None,
            ObjectKind::Door(_) => "Door",
            ObjectKind::LightSwitch(_) => "LightSwitch",
            ObjectKind::Curtain(_) => "Curtain",
            ObjectKind::Window(_) => "Window",
            ObjectKind::WindowFrame { .. } => "IsoWindowFrame",
            ObjectKind::Barbecue(_) => "Barbecue",
            ObjectKind::Fireplace(_) => "Fireplace",
            ObjectKind::CombinationWasherDryer(_) => "CombinationWasherDryer",
            ObjectKind::StackedWasherDryer(_) => "StackedWasherDryer",
            ObjectKind::ClothingDryer(_) => "ClothingDryer",
            ObjectKind::ClothingWasher(_) => "ClothingWasher",
            ObjectKind::Stove(_) => "Stove",
            ObjectKind::Jukebox => "Jukebox",
            ObjectKind::Radio(_) => "Radio",
            ObjectKind::Television(_) => "Television",
            ObjectKind::Mannequin(_) => "Mannequin",
            ObjectKind::Tree(_) => "Tree",
            ObjectKind::WorldInventory(_) => "WorldInventoryItem",
            ObjectKind::Thumpable(_) => "Thumpable",
            ObjectKind::BrokenGlass => "IsoBrokenGlass",
            ObjectKind::ButcherHook => "ButcherHook",
            ObjectKind::Compost(_) => "IsoCompost",
            ObjectKind::FeedingTrough(_) => "FeedingTrough",
            ObjectKind::Barricade(_) => "Barricade",
            ObjectKind::Hutch(_) => "IsoHutch",
            ObjectKind::Generator(_) => "IsoGenerator",
            ObjectKind::Trap(_) => "IsoTrap",
            ObjectKind::CarBatteryCharger(_) => "IsoCarBatteryCharger",
            ObjectKind::Fire(_) => "Fire",
            ObjectKind::AnimalTrack(_) => "IsoAnimalTrack",
            ObjectKind::Pushable(p) => p.object_name(),
        })
    }

    /// `instanceof IsoWaveSignal`: the device data.
    pub fn device_data(&self) -> Option<&DeviceData> {
        match self {
            ObjectKind::Radio(d) | ObjectKind::Television(d) => Some(d),
            _ => None,
        }
    }
}

/// `IsoSpriteInstance` (the saved fields).
#[derive(Clone, Debug, PartialEq)]
pub struct IsoSpriteInstance {
    pub sprite: SpriteIdx,
    pub off_x: f32,
    pub off_y: f32,
    pub off_z: f32,
    pub tint_r: f32,
    pub tint_g: f32,
    pub tint_b: f32,
    pub alpha: f32,
    pub target_alpha: f32,
    pub flip: bool,
    pub copy_target_alpha: bool,
    pub multiply_object_alpha: bool,
}

impl IsoSpriteInstance {
    /// `IsoSpriteInstance.get(spr)` (after `reset()`).
    pub fn new(sprite: SpriteIdx) -> Self {
        IsoSpriteInstance {
            sprite,
            off_x: 0.0,
            off_y: 0.0,
            off_z: 0.0,
            tint_r: 1.0,
            tint_g: 1.0,
            tint_b: 1.0,
            alpha: 1.0,
            target_alpha: 1.0,
            flip: false,
            copy_target_alpha: true,
            multiply_object_alpha: false,
        }
    }
}

/// `IsoObject.name` as stored in a chunk file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectName {
    /// The `"Grass"` shortcut.
    Grass,
    /// A `WorldDictionary` object-name id.
    Dict(i8),
    Str(String),
}

/// A sprite name as stored: a `WorldDictionary` sprite-name id or a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpriteName {
    Dict(i32),
    Str(String),
}

/// `IsoWallBloodSplat`.
#[derive(Clone, Debug, PartialEq)]
pub struct WallBloodSplat {
    pub world_age: f32,
    pub sprite: Option<SpriteIdx>,
}

/// `IsoObject.overlaySprite` and its colour.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlaySprite {
    pub name: SpriteName,
    pub color: Option<[f32; 4]>,
}

pub use pz_inventory::component::ComponentBlob;
pub use pz_inventory::ItemComponents;

/// `IsoObject` plus its class.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoObject {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub sprite: Option<SpriteIdx>,
    /// The id of a sprite the object holds that `IsoSpriteManager` does not
    /// know (`IsoSprite.CreateSprite` gives such sprites id 20000000).
    /// Only used when `sprite` is `None`.
    pub sprite_id_override: Option<i32>,
    /// `tile`: the map tile name the object was created from.
    pub tile: Option<String>,
    pub sx: f32,
    pub attached_anim_sprite: Option<Vec<IsoSpriteInstance>>,
    pub container: Option<ItemContainer>,
    pub secondary_containers: Vec<ItemContainer>,
    pub outline_on_mouseover: bool,
    pub tint_r: f32,
    pub tint_g: f32,
    pub tint_b: f32,
    pub render_y_offset: f32,
    /// Whether the entity hook gave the object components
    /// (`GameEntity.hasComponents`).
    pub has_components: bool,
    /// Sole component authority. Explicit archive imports may remain encoded;
    /// a live component has one actual allocation and mutable body.
    pub components: ItemComponents,
    pub name: Option<ObjectName>,
    pub sprite_name: Option<SpriteName>,
    pub custom_color: Option<[f32; 3]>,
    pub do_not_sync: bool,
    pub wall_blood_splats: Option<Vec<WallBloodSplat>>,
    /// `table` (mod data).
    pub mod_data: Option<LuaTable>,
    pub special_tooltip: bool,
    pub key_id: i32,
    pub uses_external_water_source: bool,
    pub sheet_rope: bool,
    pub sheet_rope_health: f32,
    pub overlay_sprite: Option<OverlaySprite>,
    pub moved_thumpable: bool,
    pub sprite_model_name: Option<String>,
    /// `rerouteCollide`: the window an `IsoWindow` made from the map set
    /// on its square's wall (runtime only, not saved).
    pub reroute_collide: Option<ObjectId>,
}

impl IsoObject {
    pub fn new(id: ObjectId, kind: ObjectKind, sprite: Option<SpriteIdx>) -> Self {
        IsoObject {
            id,
            kind,
            sprite,
            sprite_id_override: None,
            tile: None,
            sx: 0.0,
            attached_anim_sprite: None,
            container: None,
            secondary_containers: Vec::new(),
            outline_on_mouseover: false,
            tint_r: 1.0,
            tint_g: 1.0,
            tint_b: 1.0,
            render_y_offset: 0.0,
            has_components: false,
            components: ItemComponents::new(),
            name: None,
            sprite_name: None,
            custom_color: None,
            do_not_sync: false,
            wall_blood_splats: None,
            mod_data: None,
            special_tooltip: false,
            key_id: -1,
            uses_external_water_source: false,
            sheet_rope: false,
            sheet_rope_health: 100.0,
            overlay_sprite: None,
            moved_thumpable: false,
            sprite_model_name: None,
            reroute_collide: None,
        }
    }

    /// `attachedAnimSprite.add(IsoSpriteInstance.get(spr))`.
    pub fn attach_anim_sprite(&mut self, s: SpriteIdx) {
        self.attached_anim_sprite
            .get_or_insert_with(|| Vec::with_capacity(4))
            .push(IsoSpriteInstance::new(s));
    }

    /// The sprites of `attachedAnimSprite`.
    pub fn attached_sprites(&self) -> Option<Vec<SpriteIdx>> {
        self.attached_anim_sprite
            .as_ref()
            .map(|a| a.iter().map(|i| i.sprite).collect())
    }

    /// `getContainerCount()`.
    pub fn container_count(&self) -> usize {
        self.container.is_some() as usize + self.secondary_containers.len()
    }

    /// `getContainerByIndex` order.
    pub fn containers(&self) -> impl Iterator<Item = &ItemContainer> {
        self.container
            .iter()
            .chain(self.secondary_containers.iter())
    }
}

/// Lookup tables defined in game code that object creation needs. Not part
/// of this repository: read from the installed game at runtime.
#[derive(Clone, Debug)]
pub struct ObjectTables {
    /// `IsoTree.LOGS_PER_SIZE`, indexed by `size - 1` (sizes 1..=8).
    pub logs_per_size: Vec<i32>,
    /// `IsoWindow.LockedHouseFrequency` constants in ordinal order as
    /// `(value, lockChance)`. The first constant is `Never`; the last is
    /// `VeryOften`, which `fromValue` falls back to.
    pub locked_house_frequency: Vec<(i32, i32)>,
}

impl ObjectTables {
    /// Reads the tables from the installed game's `projectzomboid.jar`
    /// (see [`ObjectTables::from_jar`]).
    pub fn from_game_jar(jar: &std::path::Path) -> Result<Self, String> {
        let jar = pz_classfile::Jar::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
        Self::from_jar(&jar)
    }

    /// Reads the tables from an open game jar: the array the static
    /// initialiser of `zombie.iso.objects.IsoTree` stores in
    /// `LOGS_PER_SIZE`, and the `(value, lockChance)` constructor arguments
    /// of the `zombie.iso.objects.IsoWindow$LockedHouseFrequency` constants.
    pub fn from_jar(jar: &pz_classfile::Jar) -> Result<Self, String> {
        use pz_classfile::Value;
        const TREE: &str = "zombie/iso/objects/IsoTree";
        const FREQ: &str = "zombie/iso/objects/IsoWindow$LockedHouseFrequency";
        let int = |what: &str, v: &Value| match v {
            Value::Int(i) => i32::try_from(*i).map_err(|_| format!("{what}: {i} is not an int")),
            other => Err(format!("{what}: expected an int, got {other:?}")),
        };
        let logs = match pz_classfile::static_value(jar, TREE, "LOGS_PER_SIZE")
            .map_err(|e| format!("{TREE}: {e}"))?
        {
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| int("LOGS_PER_SIZE", v))
                .collect::<Result<Vec<_>, _>>()?,
            other => return Err(format!("{TREE}.LOGS_PER_SIZE: {other:?}")),
        };
        let freq = pz_classfile::self_constructed_constants(jar, FREQ)
            .map_err(|e| format!("{FREQ}: {e}"))?
            .iter()
            .map(|(name, args)| {
                // name, ordinal, value, lockChance
                match args.as_slice() {
                    [_, _, value, chance] => Ok((int(name, value)?, int(name, chance)?)),
                    _ => Err(format!(
                        "{FREQ}.{name}: {} constructor arguments",
                        args.len()
                    )),
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::new(logs, freq)
    }

    /// Reads the tables from text: line 1 the comma-separated logs per
    /// size, line 2 comma-separated `value:lockChance` pairs.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let logs = lines
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().parse::<i32>().map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let freq = lines
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|p| {
                let (v, c) = p.trim().split_once(':').ok_or("expected value:chance")?;
                Ok((
                    v.parse::<i32>().map_err(|e| e.to_string())?,
                    c.parse::<i32>().map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::new(logs, freq)
    }

    /// The tables in the [`ObjectTables::from_text`] format.
    pub fn to_text(&self) -> String {
        let logs: Vec<String> = self.logs_per_size.iter().map(i32::to_string).collect();
        let freq: Vec<String> = self
            .locked_house_frequency
            .iter()
            .map(|(v, c)| format!("{v}:{c}"))
            .collect();
        format!("{}\n{}\n", logs.join(","), freq.join(","))
    }

    pub fn new(
        logs_per_size: Vec<i32>,
        locked_house_frequency: Vec<(i32, i32)>,
    ) -> Result<Self, String> {
        if logs_per_size.len() != 8 {
            return Err(format!("{} logs per size, expected 8", logs_per_size.len()));
        }
        if locked_house_frequency.is_empty() {
            return Err("empty LockedHouseFrequency table".into());
        }
        Ok(ObjectTables {
            logs_per_size,
            locked_house_frequency,
        })
    }

    /// `LockedHouseFrequency.fromValue(value).getLockChance()`.
    pub fn window_lock_chance(&self, value: i32) -> i32 {
        self.locked_house_frequency
            .iter()
            .find(|&&(v, _)| v == value)
            .unwrap_or(self.locked_house_frequency.last().unwrap())
            .1
    }

    /// `LockedHouseFrequency.Never.lockChance`.
    pub fn never_lock_chance(&self) -> i32 {
        self.locked_house_frequency[0].1
    }
}

/// Game state object creation reads.
#[derive(Clone, Copy, Debug)]
pub struct ObjectLoadSettings {
    /// `SandboxOptions.lockedHouses`.
    pub locked_houses: i32,
    /// `Core.gameMode.equals("Tutorial")`.
    pub tutorial: bool,
    /// `GameTime.getMinutesStamp()`.
    pub minutes_stamp: i64,
}

impl Default for ObjectLoadSettings {
    fn default() -> Self {
        ObjectLoadSettings {
            locked_houses: 6,
            tutorial: false,
            minutes_stamp: 0,
        }
    }
}

/// Systems object creation calls into that are outside this crate. The
/// defaults behave like a game with no item or entity scripts loaded and no
/// power.
pub trait ObjectLoadHooks {
    /// `IsoGridSquare.haveElectricity() || hasGridPower()` for a square in
    /// `room` (generators, sandbox power-grid timing, derelict and
    /// user-defined rooms, `NoPower` zones).
    fn square_powered(&mut self, _sq: crate::SquareRef, _room: &RoomDef) -> bool {
        false
    }

    /// `GameEntityFactory.CreateIsoEntityFromCellLoading(obj)` for an object
    /// whose sprite has the `EntityScript` flag or both `IsMoveAble` and
    /// `CustomItem`. Returns whether components were added.
    fn create_iso_entity(
        &mut self,
        _obj: crate::IsoObjectRef,
        _rand: &mut dyn crate::GameRand,
    ) -> bool {
        false
    }

    /// `IsoWaveSignal.cloneDeviceDataFromItem(item)` (including the item
    /// creation it does the first time an item type is seen).
    fn device_data_from_item(
        &mut self,
        _item: &str,
        _rand: &mut dyn crate::GameRand,
    ) -> Option<DeviceData> {
        None
    }

    /// `ZomboidRadio.GetChannelList(category)` in iteration order.
    fn channel_list(&self, _category: &str) -> Option<Vec<(i32, String)>> {
        None
    }
}

/// [`ObjectLoadHooks`] with every default.
pub struct NoHooks;
impl ObjectLoadHooks for NoHooks {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_tables() {
        let mut p = DevicePresets {
            max_presets: 2,
            ..Default::default()
        };
        p.add_preset("a", 1);
        p.add_preset("a", 2);
        p.add_preset("b", 1);
        p.add_preset("c", 3);
        p.add_preset("d", 4);
        assert_eq!(p.presets.len(), 2);
        let t = ObjectTables::from_text("2,2,3,3,4,4,5,5\n0:0,1:9,2:7\n").unwrap();
        assert_eq!(t.window_lock_chance(1), 9);
        assert_eq!(t.window_lock_chance(9), 7);
        assert_eq!(t.never_lock_chance(), 0);
        assert!(ObjectTables::from_text("1,2\n0:0").is_err());
        let mut d = DeviceData::default();
        d.set_initial_power(100);
        assert!((d.power_delta - 0.9).abs() < 1e-6);
        d.set_turned_on_raw(false);
        assert_eq!(d.last_minute_stamp, -1);
    }
}
