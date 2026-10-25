//! `InventoryItem` and the subclasses chunk files hold, with their
//! `save`/`load`.
//!
//! An item is created from its script ([`ItemTemplate`], what
//! `Item.InstanceItem` sets) and then loaded: `load` resets most saved
//! fields to their defaults and reads those the file flags. Fields that
//! `load` does not reset keep their creation value, which is why some
//! re-saves differ from the input (see [`ItemEnv`]).

use std::rc::Rc;

use crate::component::ComponentBlob;
use crate::container::ItemContainer;
use crate::device::DeviceData;
use crate::env::ItemEnv;
use crate::io::{err, has, read_header, BitHeaderWrite, Reader, SaveError, SaveResult, Writer};
use crate::lua_table::{present, LuaKey, LuaTable, LuaValue};
use crate::shared::{ContainerRef, SharedContainer};
use crate::template::{ItemClass, ItemTemplate};
use crate::visual::ItemVisual;
use pz_scripts::objects::items::ModelWeaponPart;

/// `Color` (`col`): the item's custom colour.
pub type Rgba = [f32; 4];

/// `AnimalTracks` stored on an item.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalTracks {
    pub animal_type: String,
    pub track_type: String,
    pub x: i32,
    pub y: i32,
    /// `dir.ordinal()`, if set.
    pub dir: Option<i32>,
    pub added_time: i64,
    pub added_to_world: bool,
}

impl AnimalTracks {
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        Ok(AnimalTracks {
            animal_type: r.string()?,
            track_type: r.string()?,
            x: r.i32()?,
            y: r.i32()?,
            dir: if r.bool()? { Some(r.i32()?) } else { None },
            added_time: r.i64()?,
            added_to_world: r.bool()?,
        })
    }

    pub fn save(&self, w: &mut Writer) {
        w.string(&self.animal_type);
        w.string(&self.track_type);
        w.i32(self.x);
        w.i32(self.y);
        match self.dir {
            Some(d) => {
                w.i8(1);
                w.i32(d);
            }
            None => w.i8(0),
        }
        w.i64(self.added_time);
        w.bool(self.added_to_world);
    }
}

/// `Clothing.ClothingPatch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClothingPatch {
    pub tailor_lvl: i8,
    pub fabric_type: i8,
    pub scratch_defense: i8,
    pub bite_defense: i8,
    pub has_hole: bool,
    pub condition_gain: i16,
}

/// `Clothing` (and the clothing half of `AlarmClockClothing`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingData {
    pub sprite_name: Option<String>,
    pub dirtyness: f32,
    pub blood_level: f32,
    pub wetness: f32,
    pub last_wetness_update: f32,
    /// `patches`: body part index and patch, in `HashMap` order.
    pub patches: Option<Vec<(i8, ClothingPatch)>>,
    /// Gameplay state not saved here.
    pub spiked_front: bool,
    pub spiked_behind: bool,
}

/// `AlarmClock` / `AlarmClockClothing` alarm fields (see
/// [`crate::alarm`] for their methods).
#[derive(Clone, Debug, PartialEq)]
pub struct AlarmData {
    pub alarm_hour: i32,
    pub alarm_minutes: i32,
    pub alarm_set: bool,
    /// `ringSince` (a double in memory, saved as a float): the world age
    /// in hours the alarm started ringing, -1 when silent.
    pub ring_since: f64,
    pub force_dont_ring: i32,
}

impl Default for AlarmData {
    /// The field initializers (`alarmHour = -1`, `ringSince = -1.0`, ...).
    fn default() -> Self {
        AlarmData {
            alarm_hour: -1,
            alarm_minutes: -1,
            alarm_set: false,
            ring_since: -1.0,
            force_dont_ring: -1,
        }
    }
}

impl AlarmData {
    fn load(r: &mut Reader<'_>, wv: i32) -> SaveResult<Self> {
        // `forceDontRing` keeps the new item's -1 before world version 205.
        let mut a = AlarmData {
            alarm_hour: r.i32()?,
            alarm_minutes: r.i32()?,
            alarm_set: r.bool()?,
            ring_since: r.f32()? as f64,
            force_dont_ring: -1,
        };
        if wv >= 205 {
            a.force_dont_ring = r.i32()?;
        }
        Ok(a)
    }

    fn save(&self, w: &mut Writer) {
        w.i32(self.alarm_hour);
        w.i32(self.alarm_minutes);
        w.bool(self.alarm_set);
        w.f32(self.ring_since as f32);
        w.i32(self.force_dont_ring);
    }
}

/// `AnimalAllele` (saved in a fertilized egg's genome).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalAllele {
    pub name: String,
    pub current_value: f32,
    pub true_ratio_value: f32,
    pub dominant: bool,
    pub genetic_disorder: String,
}

/// `AnimalGene`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalGene {
    pub id: i32,
    pub name: String,
    pub allele1: AnimalAllele,
    pub allele2: AnimalAllele,
}

impl AnimalAllele {
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        Ok(AnimalAllele {
            name: r.string()?,
            current_value: r.f32()?,
            true_ratio_value: r.f32()?,
            dominant: r.bool()?,
            genetic_disorder: r.string()?,
        })
    }

    pub fn save(&self, w: &mut Writer) {
        w.string(&self.name);
        w.f32(self.current_value);
        w.f32(self.true_ratio_value);
        w.bool(self.dominant);
        w.string(&self.genetic_disorder);
    }
}

impl AnimalGene {
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        Ok(AnimalGene {
            id: r.i32()?,
            name: r.string()?,
            allele1: AnimalAllele::load(r)?,
            allele2: AnimalAllele::load(r)?,
        })
    }

    pub fn save(&self, w: &mut Writer) {
        w.i32(self.id);
        w.string(&self.name);
        self.allele1.save(w);
        self.allele2.save(w);
    }
}

/// A fertilized egg's hatch data.
#[derive(Clone, Debug, PartialEq)]
pub struct EggData {
    pub time_to_hatch: i32,
    pub fertilized_time: i32,
    pub animal_hatch: String,
    pub animal_hatch_breed: String,
    /// `eggGenome` in `HashMap<String, AnimalGene>` iteration order.
    pub genome: Vec<AnimalGene>,
    pub mother_id: i32,
}

/// `Food`.
#[derive(Clone, Debug, PartialEq)]
pub struct FoodData {
    pub age: f32,
    pub last_aged: f32,
    pub calories: f32,
    pub proteins: f32,
    pub lipids: f32,
    pub carbohydrates: f32,
    pub hung_change: f32,
    pub base_hunger: f32,
    pub unhappy_change: f32,
    pub boredom_change: f32,
    pub thirst_change: f32,
    pub heat: f32,
    pub last_cook_minute: i32,
    pub cooking_time: f32,
    pub cooked: bool,
    pub burnt: bool,
    pub is_cookable: bool,
    pub dangerous_uncooked: bool,
    pub poison_detection_level: i32,
    pub spices: Option<Vec<String>>,
    pub poison_power: i32,
    pub chef: Option<String>,
    pub off_age: i32,
    pub off_age_max: i32,
    pub pain_reduction: f32,
    pub flu_reduction: i32,
    pub food_sickness_change: i32,
    pub poison: bool,
    pub use_for_poison: i32,
    pub freezing_time: f32,
    pub frozen: bool,
    pub last_frozen_update: f32,
    pub rotten_time: f32,
    pub compost_time: f32,
    pub cooked_in_microwave: bool,
    pub fatigue_change: f32,
    pub end_change: f32,
    pub stress_change: f32,
    pub milk_qty: i32,
    pub milk_type: Option<String>,
    /// `fertilized`: set by a load that read egg data (or `setFertilized`).
    pub fertilized: bool,
    /// The egg fields (`timeToHatch`, `fertilizedTime`, `eggGenome`, ...),
    /// saved while `fertilized`.
    pub egg: Option<EggData>,
    /// `isTainted` (runtime, not saved).
    pub tainted: bool,
}

/// `HandWeapon`.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponData {
    pub max_range: f32,
    pub min_range_ranged: f32,
    pub clip_size: i32,
    pub min_damage: f32,
    pub max_damage: f32,
    pub recoil_delay: i32,
    pub aiming_time: i32,
    pub reload_time: i32,
    pub hit_chance: i32,
    pub min_angle: f32,
    /// `attachmentList`: attached weapon parts.
    pub attachments: Vec<InventoryItem>,
    pub fire_mode: Option<String>,
    pub cyclic_rate_multiplier: f32,
    pub explosion_timer: i32,
    pub max_angle: f32,
    pub blood_level: f32,
    pub contains_clip: bool,
    pub round_chambered: bool,
    pub is_jammed: bool,
    /// `spentRoundChambered` (runtime only: not saved).
    pub spent_round_chambered: bool,
    /// `spentRoundCount` (runtime only: not saved).
    pub spent_round_count: i32,
    pub weapon_sprite: Option<String>,
    pub min_sight_range: f32,
    pub max_sight_range: f32,
    /// `originalWeaponSprite` (the script's, not saved).
    pub original_weapon_sprite: Option<String>,
    /// `weaponSpritesByIndex` (the script's list, not saved).
    pub weapon_sprites_by_index: Option<Rc<Vec<String>>>,
    /// `modelWeaponPart` (the script's list, not saved).
    pub model_weapon_part: Option<Rc<Vec<ModelWeaponPart>>>,
}

/// `Literature`.
#[derive(Clone, Debug, PartialEq)]
pub struct LiteratureData {
    pub number_of_pages: i32,
    pub already_read_pages: i32,
    pub can_be_write: bool,
    /// `customPages` values in key order `1..=n` (`HashMap` order for
    /// these small keys).
    pub custom_pages: Option<Vec<String>>,
    pub locked_by: Option<String>,
    pub learned_recipes: Option<Vec<String>>,
}

/// `Moveable` light fields.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveableLight {
    pub use_battery: bool,
    pub has_battery: bool,
    pub bulb_item: Option<String>,
    pub power: f32,
    pub delta: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// `Moveable`'s default `movableFullName` and `customNameFull`.
pub const MOVEABLE_OBJECT: &str = "Moveable Object";

/// `Moveable` (and the moveable half of `Radio`).
#[derive(Clone, Debug, PartialEq)]
pub struct MoveableData {
    pub world_sprite: String,
    pub light: Option<MoveableLight>,
    /// `customItem` (from the sprite's `CustomItem` property).
    pub custom_item: Option<String>,
    pub has_read_world_sprite: bool,
    /// `movableFullName` (the sprite's group and custom name), which
    /// `getName` translates.
    pub movable_full_name: String,
    /// `customNameFull` (with the sprite grid's `(i/n)`).
    pub custom_name_full: String,
}

/// `InventoryContainer`: `weightReduction` and the bag's inventory (a
/// shared container whose `containingItem` is the bag once the item is a
/// shared object, see [`crate::shared::new_item`]).
#[derive(Debug)]
pub struct ContainerData {
    pub weight_reduction: i32,
    pub container: ContainerRef,
}

impl Clone for ContainerData {
    /// A copy of the bag holds a copy of its contents.
    fn clone(&self) -> Self {
        ContainerData {
            weight_reduction: self.weight_reduction,
            container: self.container.borrow().deep_clone(),
        }
    }
}

impl PartialEq for ContainerData {
    fn eq(&self, o: &Self) -> bool {
        self.weight_reduction == o.weight_reduction
            && (Rc::ptr_eq(&self.container, &o.container)
                || *self.container.borrow() == *o.container.borrow())
    }
}

/// The subclass part of an item.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// `ComboItem`, `DrainableComboItem`, `WeaponPart`: nothing beyond
    /// `InventoryItem`.
    Plain,
    Food(Box<FoodData>),
    Clothing(Box<ClothingData>),
    AlarmClockClothing(Box<ClothingData>, AlarmData),
    Weapon(Box<WeaponData>),
    Literature(Box<LiteratureData>),
    AlarmClock(AlarmData),
    /// `Key`: its own `keyId` (the base field stays -1) and
    /// `numberOfKey`.
    Key {
        key_id: i32,
        number_of_key: i8,
    },
    Moveable(Box<MoveableData>),
    Radio(Box<MoveableData>, Box<DeviceData>),
    Container(Box<ContainerData>),
    /// `MapItem`: `mapId` and its `WorldMapSymbols`, kept as bytes (the
    /// symbols run to the end of the item record).
    Map {
        map_id: String,
        symbols: Vec<u8>,
    },
}

/// `InventoryItem` state that saving and loading touch.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryItem {
    pub template: Rc<ItemTemplate>,
    /// `getRegistry_id()`.
    pub registry_id: i16,
    /// `type` (a `Moveable` takes its world sprite name).
    pub type_: String,
    pub id: i32,
    pub uses: i32,
    pub condition: i32,
    pub condition_max: i32,
    pub visual: Option<ItemVisual>,
    pub custom_color: bool,
    pub col: Rgba,
    pub item_capacity: f32,
    pub table: Option<LuaTable>,
    pub activated: bool,
    pub have_been_repaired: i32,
    pub name: String,
    pub original_name: String,
    pub byte_data: Option<Vec<u8>>,
    /// `extraItems` as `WorldDictionary` item ids.
    pub extra_items: Option<Vec<i16>>,
    pub custom_name: bool,
    pub custom_weight: bool,
    pub actual_weight: f32,
    pub weight: f32,
    pub key_id: i32,
    pub remote_control_id: i32,
    pub remote_range: i32,
    pub color_red: f32,
    pub color_green: f32,
    pub color_blue: f32,
    pub worker: Option<String>,
    pub wet_cooldown: f32,
    pub favorite: bool,
    pub stash_map: Option<String>,
    pub zombie_infected: bool,
    pub current_ammo_count: i32,
    pub attached_slot: i32,
    pub attached_slot_type: Option<String>,
    pub attached_to_model: Option<String>,
    pub max_capacity: i32,
    pub recorded_media_index: i16,
    pub world_scale: f32,
    pub is_initialised: bool,
    /// ECS components in entity order (see [`crate::component`]), as
    /// saved.
    pub components: Vec<ComponentBlob>,
    /// The decoded components of a newly made item (or one changed through
    /// [`set_components`](InventoryItem::set_components)), kept beside the
    /// blobs: in memory the original keeps values the save rounds (fluid
    /// colours are saved as bytes). `None` after a load, whose blobs decode
    /// exactly. Code that edits `components` directly must clear it.
    pub component_cache: Option<Box<pz_entity::Components>>,
    pub animal_tracks: Option<AnimalTracks>,
    /// A texture name saved because it differs from the script icon.
    pub texture_override: Option<String>,
    pub model_index: i32,
    /// `staticModelsByIndex` / `worldStaticModelsByIndex`: the script's
    /// lists when not empty (`Item.InstanceItem`), not saved.
    pub static_models_by_index: Option<Rc<Vec<String>>>,
    pub world_static_models_by_index: Option<Rc<Vec<String>>>,
    pub world_x_rotation: f32,
    pub world_y_rotation: f32,
    pub world_z_rotation: f32,
    /// `canBeActivated`, `lightStrength`, `isTorchCone`, `lightDistance`:
    /// set from the script at creation (`Item.InstanceItem`), not saved.
    pub can_be_activated: bool,
    pub light_strength: f32,
    pub torch_cone: bool,
    pub light_distance: i32,
    /// `brakeForce`, `durability`, ... (from the script at creation; not
    /// saved).
    pub vehicle_part: crate::template::VehiclePartStats,
    pub kind: ItemKind,
}

const WHITE: Rgba = [1.0, 1.0, 1.0, 1.0];

impl InventoryItem {
    /// `Item.InstanceItem` for the state saving reads (the item id is 0;
    /// `InventoryItemFactory` draws it from `Rand`).
    pub fn create(template: Rc<ItemTemplate>, registry_id: i16) -> Self {
        let t = &template;
        let kind = match t.class {
            ItemClass::Normal | ItemClass::Drainable | ItemClass::WeaponPart => ItemKind::Plain,
            ItemClass::Food => ItemKind::Food(Box::new(FoodData::from_template(t))),
            ItemClass::Clothing => ItemKind::Clothing(Box::new(ClothingData::from_template(t))),
            ItemClass::AlarmClockClothing => ItemKind::AlarmClockClothing(
                Box::new(ClothingData::from_template(t)),
                AlarmData::default(),
            ),
            ItemClass::Weapon => ItemKind::Weapon(Box::new(WeaponData::from_template(t))),
            ItemClass::Literature => ItemKind::Literature(Box::new(LiteratureData {
                number_of_pages: t.number_of_pages,
                already_read_pages: 0,
                can_be_write: false,
                custom_pages: None,
                locked_by: None,
                learned_recipes: None,
            })),
            ItemClass::AlarmClock => ItemKind::AlarmClock(AlarmData::default()),
            ItemClass::Key => ItemKind::Key {
                key_id: -1,
                number_of_key: 0,
            },
            ItemClass::Moveable => ItemKind::Moveable(Box::new(MoveableData {
                world_sprite: String::new(),
                light: None,
                custom_item: None,
                has_read_world_sprite: false,
                movable_full_name: MOVEABLE_OBJECT.into(),
                custom_name_full: MOVEABLE_OBJECT.into(),
            })),
            ItemClass::Radio => ItemKind::Radio(
                Box::new(MoveableData {
                    world_sprite: String::new(),
                    light: None,
                    custom_item: None,
                    has_read_world_sprite: false,
                    movable_full_name: MOVEABLE_OBJECT.into(),
                    custom_name_full: MOVEABLE_OBJECT.into(),
                }),
                Box::default(),
            ),
            ItemClass::Container => ItemKind::Container(Box::new(ContainerData {
                weight_reduction: t.weight_reduction.clamp(0, 100),
                container: SharedContainer::new_bag(t).into_ref(),
            })),
            ItemClass::Map => ItemKind::Map {
                map_id: String::new(),
                symbols: Vec::new(),
            },
            ItemClass::Animal => ItemKind::Plain,
        };
        InventoryItem {
            registry_id,
            type_: t.type_.clone(),
            id: 0,
            uses: 1,
            condition: t.condition_max,
            condition_max: t.condition_max,
            visual: None,
            custom_color: false,
            col: WHITE,
            item_capacity: t.item_capacity as f32,
            table: None,
            activated: false,
            have_been_repaired: 0,
            name: t.display_name.clone(),
            original_name: t.display_name.clone(),
            byte_data: None,
            extra_items: None,
            custom_name: false,
            custom_weight: false,
            actual_weight: t.actual_weight,
            weight: t.actual_weight,
            key_id: -1,
            remote_control_id: -1,
            remote_range: 0,
            color_red: 1.0,
            color_green: 1.0,
            color_blue: 1.0,
            worker: None,
            // `item.setWetCooldown(this.wetCooldown)`.
            wet_cooldown: t.wet_cooldown,
            favorite: false,
            stash_map: None,
            zombie_infected: false,
            current_ammo_count: 0,
            attached_slot: -1,
            attached_slot_type: None,
            attached_to_model: None,
            max_capacity: t.max_capacity,
            recorded_media_index: -1,
            world_scale: 1.0,
            is_initialised: false,
            components: Vec::new(),
            component_cache: None,
            animal_tracks: None,
            texture_override: None,
            model_index: -1,
            static_models_by_index: t.static_models_by_index.clone().filter(|l| !l.is_empty()),
            world_static_models_by_index: t
                .world_static_models_by_index
                .clone()
                .filter(|l| !l.is_empty()),
            world_x_rotation: 0.0,
            world_y_rotation: 0.0,
            world_z_rotation: -1.0,
            can_be_activated: t.activated_item,
            light_strength: t.light_strength,
            torch_cone: t.torch_cone,
            light_distance: t.light_distance,
            vehicle_part: t.vehicle_part.clone(),
            kind,
            template,
        }
    }

    /// `getModule()`.
    pub fn get_module(&self) -> &str {
        &self.template.module
    }

    /// `getType()`.
    pub fn get_type(&self) -> &str {
        &self.type_
    }

    /// `getFullType()`.
    pub fn get_full_type(&self) -> String {
        format!("{}.{}", self.template.module, self.type_)
    }

    /// `getName()`.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// `getClass().getSimpleName()`.
    pub fn class_name(&self) -> &'static str {
        self.template.class.java_name()
    }

    /// `InventoryItem.save(output, false)` (the registry id, save type and
    /// the class's fields).
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        w.i16(self.registry_id);
        w.i8(-1);
        w.i32(self.id);
        self.save_fields(w, env)
    }

    /// `saveWithSize`.
    pub fn save_with_size(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        let at = w.pos();
        w.i32(0);
        self.save(w, env)?;
        let len = w.pos() - at - 4;
        w.patch_i32(at, len as i32);
        Ok(())
    }

    fn save_fields(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        let mut header = BitHeaderWrite::alloc(w, 1);
        let uses = self.get_current_uses();
        if uses != 1 {
            header.add(1);
            w.i32(uses);
        }
        if self.condition != self.condition_max {
            header.add(4);
            w.i8(self.condition as i8);
        }
        if let Some(v) = &self.visual {
            header.add(8);
            v.save(w);
        }
        if self.custom_color && self.col != WHITE {
            header.add(16);
            for c in self.col {
                w.unit_byte(c);
            }
        }
        if self.item_capacity != -1.0 {
            header.add(32);
            w.f32(self.item_capacity);
        }
        let mut bits = BitHeaderWrite::alloc(w, 4);
        if let Some(t) = present(&self.table) {
            bits.add(1);
            t.save(w);
        }
        if self.activated {
            bits.add(2);
        }
        if self.have_been_repaired != 0 {
            bits.add(4);
            w.i16(self.have_been_repaired as i16);
        }
        if self.name != self.original_name {
            bits.add(8);
            w.string(&self.name);
        }
        if let Some(b) = &self.byte_data {
            bits.add(16);
            w.i32(b.len() as i32);
            w.bytes(b);
        }
        if let Some(x) = self.extra_items.as_ref().filter(|x| !x.is_empty()) {
            bits.add(32);
            w.i32(x.len() as i32);
            for &id in x {
                w.i16(id);
            }
        }
        if self.custom_name {
            bits.add(64);
        }
        if self.custom_weight {
            bits.add(128);
            w.f32(self.actual_weight);
        }
        if self.key_id != -1 {
            bits.add(256);
            w.i32(self.key_id);
        }
        if self.remote_control_id != -1 || self.remote_range != 0 {
            bits.add(1024);
            w.i32(self.remote_control_id);
            w.i32(self.remote_range);
        }
        if self.color_red != 1.0 || self.color_green != 1.0 || self.color_blue != 1.0 {
            bits.add(2048);
            w.unit_byte(self.color_red);
            w.unit_byte(self.color_green);
            w.unit_byte(self.color_blue);
        }
        if let Some(s) = &self.worker {
            bits.add(4096);
            w.string(s);
        }
        if self.wet_cooldown != -1.0 {
            bits.add(8192);
            w.f32(self.wet_cooldown);
        }
        if self.favorite {
            bits.add(16384);
        }
        if let Some(s) = &self.stash_map {
            bits.add(32768);
            w.string(s);
        }
        if self.zombie_infected {
            bits.add(65536);
        }
        if self.current_ammo_count != 0 {
            bits.add(131072);
            w.i32(self.current_ammo_count);
        }
        if self.attached_slot != -1 {
            bits.add(262144);
            w.i32(self.attached_slot);
        }
        if let Some(s) = &self.attached_slot_type {
            bits.add(524288);
            w.string(s);
        }
        if let Some(s) = &self.attached_to_model {
            bits.add(0x100000);
            w.string(s);
        }
        if self.max_capacity != -1 {
            bits.add(0x200000);
            w.i32(self.max_capacity);
        }
        if self.recorded_media_index >= 0 {
            bits.add(0x400000);
            w.i16(self.recorded_media_index);
        }
        if self.world_scale != 1.0 {
            bits.add(0x1000000);
            w.f32(self.world_scale);
        }
        if self.is_initialised {
            bits.add(0x2000000);
        }
        if !self.components.is_empty() {
            bits.add(0x4000000);
            env.components().save_entity(w, &self.components)?;
        }
        if let Some(t) = &self.animal_tracks {
            bits.add(0x8000000);
            t.save(w);
        }
        if let Some(t) = &self.texture_override {
            bits.add(0x10000000);
            w.string(t);
        }
        if self.model_index > -1 {
            bits.add(0x20000000);
            w.i32(self.model_index);
        }
        if self.world_x_rotation != 0.0
            || self.world_y_rotation != 0.0
            || self.world_z_rotation != -1.0
        {
            bits.add(0x40000000);
            w.f32(self.world_x_rotation);
            w.f32(self.world_y_rotation);
            w.f32(self.world_z_rotation);
        }
        if bits.flags != 0 {
            header.add(64);
            bits.write(w);
        } else {
            bits.discard(w);
        }
        header.write(w);
        self.save_kind(w, env)
    }

    fn save_kind(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        match &self.kind {
            ItemKind::Plain => {}
            ItemKind::Food(f) => f.save(w),
            ItemKind::Clothing(c) => c.save(w),
            ItemKind::AlarmClockClothing(c, a) => {
                c.save(w);
                a.save(w);
            }
            ItemKind::Weapon(wd) => wd.save(w, env, &self.template)?,
            ItemKind::Literature(l) => l.save(w),
            ItemKind::AlarmClock(a) => a.save(w),
            ItemKind::Key {
                key_id,
                number_of_key,
            } => {
                w.i32(*key_id);
                w.i8(*number_of_key);
            }
            ItemKind::Moveable(m) => m.save(w),
            ItemKind::Radio(m, d) => {
                m.save(w);
                w.i8(1);
                d.save(w);
            }
            ItemKind::Map { map_id, symbols } => {
                w.string(map_id);
                w.bytes(symbols);
            }
            ItemKind::Container(c) => {
                let saved = c.container.borrow().to_container();
                w.i32(saved.id);
                w.i32(c.weight_reduction);
                saved.save(w, env)?;
            }
        }
        Ok(())
    }

    /// `InventoryItem.load` and the subclass `load` (after the registry
    /// id and save type).
    pub fn load(&mut self, r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<()> {
        if let ItemKind::Weapon(w) = &mut self.kind {
            // clearAllWeaponParts
            w.attachments.clear();
        }
        self.id = r.i32()?;
        let header = read_header(r, 1)?;
        self.set_current_uses_loading(1, env);
        self.name = self.original_name.clone();
        self.condition = self.condition_max;
        self.custom_color = false;
        self.col = WHITE;
        self.item_capacity = -1.0;
        self.table = None;
        self.activated = false;
        self.have_been_repaired = 0;
        self.custom_name = false;
        self.custom_weight = false;
        self.key_id = -1;
        self.remote_control_id = -1;
        self.remote_range = 0;
        self.color_red = 1.0;
        self.color_green = 1.0;
        self.color_blue = 1.0;
        self.worker = None;
        self.wet_cooldown = -1.0;
        self.favorite = false;
        self.stash_map = None;
        self.zombie_infected = false;
        self.current_ammo_count = 0;
        self.attached_slot = -1;
        self.attached_slot_type = None;
        self.attached_to_model = None;
        self.max_capacity = -1;
        self.recorded_media_index = -1;
        self.world_z_rotation = -1.0;
        self.world_scale = 1.0;
        self.is_initialised = false;
        if header != 0 {
            if wv >= 220 {
                if has(header, 1) {
                    let u = r.i32()?;
                    self.set_current_uses_loading(u, env);
                }
            } else {
                if has(header, 1) {
                    let u = r.i16()? as i32;
                    self.set_current_uses_loading(u, env);
                }
                if has(header, 2) {
                    r.i8()?;
                }
            }
            if has(header, 4) {
                // setConditionWhileLoading: PZMath.clamp(c, 0, conditionMax).
                let c = r.i8()? as i32;
                self.condition = if c < 0 {
                    0
                } else if c > self.condition_max {
                    self.condition_max
                } else {
                    c
                };
            }
            if has(header, 8) {
                self.visual = Some(ItemVisual::load(r, env.blood_body_parts())?);
            }
            if has(header, 16) {
                self.col = [
                    r.unit_byte()?,
                    r.unit_byte()?,
                    r.unit_byte()?,
                    r.unit_byte()?,
                ];
                self.custom_color = true;
            }
            if has(header, 32) {
                self.item_capacity = r.f32()?;
            }
            if has(header, 64) {
                let bits = read_header(r, 4)?;
                if has(bits, 1) {
                    self.table = Some(LuaTable::load(r, wv)?);
                }
                self.activated = has(bits, 2);
                if has(bits, 4) {
                    self.have_been_repaired = r.i16()? as i32;
                }
                if has(bits, 8) {
                    self.name = r.string()?;
                }
                if has(bits, 16) {
                    let n = r.i32()?;
                    if n < 0 {
                        return err("negative byteData size");
                    }
                    self.byte_data = Some(r.bytes(n as usize)?.to_vec());
                }
                if has(bits, 32) {
                    let n = r.i32()?;
                    if n > 0 {
                        let mut v = Vec::with_capacity(n as usize);
                        for _ in 0..n {
                            v.push(env.extra_item_round_trip(r.i16()?));
                        }
                        self.extra_items = Some(v);
                    }
                }
                // setCustomName also stores the name in the mod data.
                self.custom_name = has(bits, 64);
                self.table.get_or_insert_with(LuaTable::default).rawset(
                    LuaKey::Str("customName".into()),
                    Some(LuaValue::Str(self.name.clone())),
                );
                if has(bits, 128) {
                    let weight = r.f32()?;
                    if weight >= 0.0 {
                        self.actual_weight = weight;
                        self.weight = weight;
                        self.custom_weight = true;
                    }
                }
                if has(bits, 256) {
                    let k = r.i32()?;
                    self.set_key_id(k, env);
                }
                if has(bits, 1024) {
                    self.remote_control_id = r.i32()?;
                    self.remote_range = r.i32()?;
                }
                if has(bits, 2048) {
                    self.color_red = r.unit_byte()?;
                    self.color_green = r.unit_byte()?;
                    self.color_blue = r.unit_byte()?;
                    self.col = [self.color_red, self.color_green, self.color_blue, 1.0];
                    self.custom_color = true;
                    self.tint_visual();
                }
                if has(bits, 4096) {
                    self.worker = Some(r.string()?);
                }
                if has(bits, 8192) {
                    self.wet_cooldown = r.f32()?;
                }
                self.favorite = has(bits, 16384);
                if has(bits, 32768) {
                    self.stash_map = Some(r.string()?);
                }
                self.zombie_infected = has(bits, 65536);
                if has(bits, 131072) {
                    self.current_ammo_count = r.i32()?;
                }
                if has(bits, 262144) {
                    self.attached_slot = r.i32()?;
                }
                if has(bits, 524288) {
                    self.attached_slot_type = Some(r.string()?);
                }
                if has(bits, 0x100000) {
                    self.attached_to_model = Some(r.string()?);
                }
                if has(bits, 0x200000) {
                    self.max_capacity = r.i32()?;
                }
                if has(bits, 0x400000) {
                    let idx = r.i16()?;
                    self.set_recorded_media_index(idx, env);
                }
                if wv < 232 && has(bits, 0x800000) {
                    self.world_z_rotation = r.i32()? as f32;
                }
                if has(bits, 0x1000000) {
                    self.world_scale = r.f32()?;
                }
                self.is_initialised = has(bits, 0x2000000);
                if has(bits, 0x4000000) {
                    let loaded = env.components().load_entity(r, wv, &mut self.components)?;
                    // Components the factory made that the file does not hold
                    // are removed, last to first, from the component list (an
                    // unordered `Array`: the last component takes the removed
                    // one's place).
                    for i in (0..self.components.len()).rev() {
                        if !loaded.contains(&self.components[i].type_id) {
                            self.components.swap_remove(i);
                        }
                    }
                    self.component_cache = None;
                }
                if has(bits, 0x8000000) {
                    self.animal_tracks = Some(AnimalTracks::load(r)?);
                }
                if has(bits, 0x10000000) {
                    let name = r.string()?;
                    self.texture_override = env.texture_round_trip(&self.template, &name);
                }
                if has(bits, 0x20000000) {
                    self.model_index = r.i32()?;
                }
                if has(bits, 0x40000000) {
                    if wv >= 232 {
                        self.world_x_rotation = r.f32()?;
                        self.world_y_rotation = r.f32()?;
                        self.world_z_rotation = r.f32()?;
                    } else {
                        self.world_y_rotation = r.i32()? as f32;
                        self.world_x_rotation = r.i32()? as f32;
                    }
                }
            }
        }
        // The end of `InventoryItem.load`: `synchWithVisual` (its model
        // index draw; the visual part runs after the subclass fields here).
        self.pick_model_index(&mut |m| env.rand_next(m));
        self.load_kind(r, wv, env)?;
        if let Some(a) = env.assets() {
            self.synch_with_visual(Some(a), &crate::context::NoOwner, env.visual_rng());
        }
        Ok(())
    }

    /// `setCurrentUses` while `InventoryItem.load` runs: `Food` with a
    /// base hunger (still the script's here) consumes hunger instead, which
    /// `Food.load` then overwrites, so only the uses of other items change.
    fn set_current_uses_loading(&mut self, v: i32, env: &dyn ItemEnv) {
        match &self.kind {
            ItemKind::Food(f) if f.base_hunger != 0.0 => {}
            ItemKind::Food(_) => self.uses = v.max(0),
            _ => self.uses = v,
        }
        // DrainableComboItem.setCurrentUses -> updateWeight.
        self.update_weight_by(|n| env.script_actual_weight(n));
    }

    /// `getVisual().setTint(new ImmutableColor(colorRed, colorGreen,
    /// colorBlue))`.
    fn tint_visual(&mut self) {
        let c = [self.color_red, self.color_green, self.color_blue].map(|v| v.clamp(0.0, 1.0));
        if let Some(v) = &mut self.visual {
            v.tint = Some(c);
        }
    }

    /// `setKeyId` (`Key` overrides it: its own field, and -1 draws a
    /// random id).
    fn set_key_id(&mut self, k: i32, env: &dyn ItemEnv) {
        match &mut self.kind {
            ItemKind::Key { key_id, .. } => {
                *key_id = if k == -1 {
                    env.rand_next(10_000_000)
                } else {
                    k
                };
            }
            _ => self.key_id = k,
        }
    }

    /// `setRecordedMediaIndex` (the media name is looked up through the
    /// environment).
    fn set_recorded_media_index(&mut self, idx: i16, env: &dyn ItemEnv) {
        self.recorded_media_index = idx;
        if idx >= 0 {
            match env.recorded_media_name(idx) {
                Some(Some(name)) => self.name = name,
                Some(None) => {}
                None => self.recorded_media_index = -1,
            }
        } else {
            self.name = self.template.display_name.clone();
        }
    }

    fn load_kind(&mut self, r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<()> {
        let template = self.template.clone();
        match &mut self.kind {
            ItemKind::Plain => {}
            ItemKind::Food(f) => f.load(r)?,
            ItemKind::Clothing(c) => c.load(r, env)?,
            ItemKind::AlarmClockClothing(c, a) => {
                c.load(r, env)?;
                *a = AlarmData::load(r, wv)?;
            }
            ItemKind::Weapon(w) => w.load(r, wv, env, &template)?,
            ItemKind::Literature(l) => l.load(r, wv)?,
            ItemKind::AlarmClock(a) => *a = AlarmData::load(r, wv)?,
            ItemKind::Key { .. } => {
                let k = r.i32()?;
                let n = r.i8()?;
                self.set_key_id(k, env);
                if let ItemKind::Key { number_of_key, .. } = &mut self.kind {
                    *number_of_key = n;
                }
            }
            ItemKind::Moveable(m) => {
                m.load(r)?;
                self.after_moveable_load(env);
            }
            ItemKind::Radio(m, d) => {
                m.load(r)?;
                if r.bool()? {
                    d.load(r)?;
                }
                self.after_moveable_load(env);
            }
            ItemKind::Map { map_id, symbols } => {
                *map_id = r.string()?;
                // WorldMapSymbols.load: a version short, then the symbols.
                // Only version 2 is saved back unchanged.
                let rest = r.bytes(r.remaining())?;
                if rest.len() < 2 || i16::from_be_bytes([rest[0], rest[1]]) != 2 {
                    return Err(SaveError("map symbols version is not 2".into()));
                }
                *symbols = rest.to_vec();
            }
            ItemKind::Container(c) => {
                // The bag keeps its container object: `clear()`, then the
                // weight reduction, capacity and id, then `load`.
                let id = r.i32()?;
                c.weight_reduction = r.i32()?.clamp(0, 100);
                let mut saved = ItemContainer::new(None);
                saved.capacity = template.capacity;
                saved.id = id;
                saved.load(r, wv, env)?;
                c.container.borrow_mut().weight_reduction = c.weight_reduction;
                SharedContainer::fill_from(&c.container, saved);
            }
        }
        Ok(())
    }

    /// `Moveable.load` after the world sprite: `ReadFromWorldSprite`, then
    /// a moveable without a custom item takes its sprite name as its type.
    fn after_moveable_load(&mut self, env: &dyn ItemEnv) {
        let ws = match &self.kind {
            ItemKind::Moveable(m) | ItemKind::Radio(m, _) => m.world_sprite.clone(),
            _ => return,
        };
        self.read_from_world_sprite(&ws, env);
        let custom = match &self.kind {
            ItemKind::Moveable(m) | ItemKind::Radio(m, _) => m.custom_item.is_some(),
            _ => false,
        };
        if !custom && !ws.trim().is_empty() && !self.type_.eq_ignore_ascii_case(&ws) {
            self.type_ = ws;
        }
    }

    /// `Moveable.ReadFromWorldSprite`: the item state the sprite's
    /// properties give, with the flatpack colour of a multi-tile sprite
    /// drawn from `Rand` (textures are not modelled).
    pub fn read_from_world_sprite(&mut self, sprite: &str, env: &dyn ItemEnv) -> bool {
        let (ItemKind::Moveable(m) | ItemKind::Radio(m, _)) = &mut self.kind else {
            return false;
        };
        if m.has_read_world_sprite && m.world_sprite.eq_ignore_ascii_case(sprite) {
            return true;
        }
        m.custom_item = None;
        let Some(info) = env.world_sprite(sprite) else {
            return false;
        };
        if let Some(ci) = info.custom_item {
            if let Some(w) = env.script_actual_weight(&ci) {
                self.weight = w;
                self.actual_weight = w;
            }
            m.custom_item = Some(ci);
            m.world_sprite = sprite.to_string();
            return true;
        }
        m.world_sprite = sprite.to_string();
        let w = info.pick_up_weight.map_or(1.0, |w| w / 10.0);
        self.weight = w;
        self.actual_weight = w;
        self.custom_weight = true;
        let mut name = match (&info.custom_name, &info.group_name) {
            (Some(c), Some(g)) => format!("{g} {c}"),
            (Some(c), None) => c.clone(),
            _ => MOVEABLE_OBJECT.to_string(),
        };
        m.movable_full_name = name.clone();
        if let Some((index, count)) = info.sprite_grid {
            // `setColor(new Color(Rand.Next(0.7f, 1.0f), ...))`, r, g, b.
            let r = env.rand_float(0.7, 1.0);
            let g = env.rand_float(0.7, 1.0);
            let b = env.rand_float(0.7, 1.0);
            self.col = [r, g, b, 1.0];
            name = if info.force_single_item {
                format!("{name} (1/1)")
            } else {
                format!("{name} ({}/{count})", index + 1)
            };
            self.table.get_or_insert_with(LuaTable::default).rawset(
                LuaKey::Str("Flatpack".into()),
                Some(LuaValue::Str("true".into())),
            );
        }
        m.custom_name_full = name.clone();
        self.name = name;
        m.has_read_world_sprite = true;
        true
    }
}

impl ClothingData {
    fn from_template(t: &ItemTemplate) -> Self {
        ClothingData {
            sprite_name: t.sprite_name.clone(),
            dirtyness: 0.0,
            blood_level: 0.0,
            wetness: 0.0,
            last_wetness_update: 0.0,
            patches: None,
            spiked_front: false,
            spiked_behind: false,
        }
    }

    fn save(&self, w: &mut Writer) {
        let mut bits = BitHeaderWrite::alloc(w, 1);
        if let Some(s) = &self.sprite_name {
            bits.add(1);
            w.string(s);
        }
        if self.dirtyness != 0.0 {
            bits.add(2);
            w.f32(self.dirtyness);
        }
        if self.blood_level != 0.0 {
            bits.add(4);
            w.f32(self.blood_level);
        }
        if self.wetness != 0.0 {
            bits.add(8);
            w.f32(self.wetness);
        }
        if self.last_wetness_update != 0.0 {
            bits.add(16);
            w.f32(self.last_wetness_update);
        }
        if let Some(p) = &self.patches {
            bits.add(32);
            w.i8(p.len() as i8);
            for (part, patch) in p {
                w.i8(*part);
                w.i8(patch.tailor_lvl);
                w.i8(patch.fabric_type);
                w.i8(patch.scratch_defense);
                w.i8(patch.bite_defense);
                w.bool(patch.has_hole);
                w.i16(patch.condition_gain);
            }
        }
        bits.write(w);
    }

    fn load(&mut self, r: &mut Reader<'_>, env: &dyn ItemEnv) -> SaveResult<()> {
        let bits = read_header(r, 1)?;
        if bits != 0 {
            if has(bits, 1) {
                self.sprite_name = Some(r.string()?);
            }
            if has(bits, 2) {
                self.dirtyness = r.f32()?;
            }
            if has(bits, 4) {
                self.blood_level = r.f32()?;
            }
            if has(bits, 8) {
                self.wetness = r.f32()?;
            }
            if has(bits, 16) {
                self.last_wetness_update = r.f32()?;
            }
            if has(bits, 32) {
                let n = r.i8()?;
                for _ in 0..n.max(0) {
                    let part = r.i8()?;
                    let patch = ClothingPatch {
                        tailor_lvl: r.i8()?,
                        fabric_type: r.i8()?,
                        scratch_defense: r.i8()?,
                        bite_defense: r.i8()?,
                        has_hole: r.bool()?,
                        condition_gain: r.i16()?,
                    };
                    let list = self.patches.get_or_insert_with(Vec::new);
                    match list.iter_mut().find(|(p, _)| *p == part) {
                        Some(e) => e.1 = patch,
                        None => list.push((part, patch)),
                    }
                }
                if let Some(list) = &mut self.patches {
                    crate::java::sort_int_hash_map(list, |e| e.0 as i32);
                }
            }
        }
        // Clothing.load ends with lastWetnessUpdate = world age.
        self.last_wetness_update = env.world_age_hours() as f32;
        Ok(())
    }
}

impl FoodData {
    fn from_template(t: &ItemTemplate) -> Self {
        FoodData {
            age: 0.0,
            last_aged: -1.0,
            calories: t.calories,
            proteins: t.proteins,
            lipids: t.lipids,
            carbohydrates: t.carbohydrates,
            hung_change: t.hunger_change / 100.0,
            base_hunger: t.hunger_change / 100.0,
            unhappy_change: t.unhappy_change,
            boredom_change: t.boredom_change,
            thirst_change: t.thirst_change / 100.0,
            heat: 1.0,
            last_cook_minute: 0,
            cooking_time: 0.0,
            cooked: false,
            burnt: false,
            is_cookable: t.is_cookable,
            dangerous_uncooked: t.dangerous_uncooked,
            poison_detection_level: -1,
            spices: None,
            poison_power: t.poison_power,
            chef: None,
            off_age: t.days_fresh,
            off_age_max: t.days_totally_rotten,
            pain_reduction: 0.0,
            flu_reduction: 0,
            food_sickness_change: 0,
            poison: false,
            use_for_poison: 0,
            freezing_time: 0.0,
            frozen: false,
            last_frozen_update: 0.0,
            rotten_time: 0.0,
            compost_time: 0.0,
            cooked_in_microwave: false,
            fatigue_change: t.fatigue_change / 100.0,
            end_change: t.endurance_change / 100.0,
            stress_change: t.stress_change / 100.0,
            milk_qty: 0,
            milk_type: None,
            fertilized: false,
            egg: None,
            tainted: false,
        }
    }

    fn save(&self, w: &mut Writer) {
        w.f32(self.age);
        w.f32(self.last_aged);
        let mut header = BitHeaderWrite::alloc(w, 1);
        if self.calories != 0.0
            || self.proteins != 0.0
            || self.lipids != 0.0
            || self.carbohydrates != 0.0
        {
            header.add(1);
            w.f32(self.calories);
            w.f32(self.proteins);
            w.f32(self.lipids);
            w.f32(self.carbohydrates);
        }
        for (flag, v) in [
            (2, self.hung_change),
            (4, self.base_hunger),
            (8, self.unhappy_change),
            (16, self.boredom_change),
            (32, self.thirst_change),
        ] {
            if v != 0.0 {
                header.add(flag);
                w.f32(v);
            }
        }
        let mut bits = BitHeaderWrite::alloc(w, 4);
        if self.heat != 1.0 {
            bits.add(1);
            w.f32(self.heat);
        }
        if self.last_cook_minute != 0 {
            bits.add(2);
            w.i32(self.last_cook_minute);
        }
        if self.cooking_time != 0.0 {
            bits.add(4);
            w.f32(self.cooking_time);
        }
        if self.cooked {
            bits.add(8);
        }
        if self.burnt {
            bits.add(16);
        }
        if self.is_cookable {
            bits.add(32);
        }
        if self.dangerous_uncooked {
            bits.add(64);
        }
        if self.poison_detection_level != -1 {
            bits.add(128);
            w.i8(self.poison_detection_level as i8);
        }
        if let Some(s) = &self.spices {
            bits.add(256);
            w.i8(s.len() as i8);
            for x in s {
                w.string(x);
            }
        }
        if self.poison_power != 0 {
            bits.add(512);
            w.i8(self.poison_power as i8);
        }
        if let Some(c) = &self.chef {
            bits.add(1024);
            w.string(c);
        }
        if self.off_age as f64 != 1.0e9 {
            bits.add(2048);
            w.i32(self.off_age);
        }
        if self.off_age_max as f64 != 1.0e9 {
            bits.add(4096);
            w.i32(self.off_age_max);
        }
        if self.pain_reduction != 0.0 {
            bits.add(8192);
            w.f32(self.pain_reduction);
        }
        if self.flu_reduction != 0 {
            bits.add(16384);
            w.i32(self.flu_reduction);
        }
        if self.food_sickness_change != 0 {
            bits.add(32768);
            w.i32(self.food_sickness_change);
        }
        if self.poison {
            bits.add(65536);
        }
        if self.use_for_poison != 0 {
            bits.add(131072);
            w.i16(self.use_for_poison as i16);
        }
        if self.freezing_time != 0.0 {
            bits.add(262144);
            w.f32(self.freezing_time);
        }
        if self.frozen {
            bits.add(524288);
        }
        if self.last_frozen_update != 0.0 {
            bits.add(0x100000);
            w.f32(self.last_frozen_update);
        }
        if self.rotten_time != 0.0 {
            bits.add(0x200000);
            w.f32(self.rotten_time);
        }
        if self.compost_time != 0.0 {
            bits.add(0x400000);
            w.f32(self.compost_time);
        }
        if self.cooked_in_microwave {
            bits.add(0x800000);
        }
        if self.fatigue_change != 0.0 {
            bits.add(0x1000000);
            w.f32(self.fatigue_change);
        }
        if self.end_change != 0.0 {
            bits.add(0x2000000);
            w.f32(self.end_change);
        }
        if self.milk_qty > 0 {
            bits.add(0x4000000);
            w.i32(self.milk_qty);
            w.opt_string(self.milk_type.as_deref());
        }
        if let Some(e) = self.egg.as_ref().filter(|_| self.fertilized) {
            bits.add(0x8000000);
            w.i32(e.time_to_hatch);
            w.i32(e.fertilized_time);
            w.string(&e.animal_hatch);
            w.string(&e.animal_hatch_breed);
            w.i32(e.genome.len() as i32);
            for g in &e.genome {
                g.save(w);
            }
            w.i32(e.mother_id);
        }
        if self.stress_change != 0.0 {
            bits.add(0x10000000);
            w.f32(self.stress_change);
        }
        if bits.flags != 0 {
            header.add(64);
            bits.write(w);
        } else {
            bits.discard(w);
        }
        header.write(w);
    }

    fn load(&mut self, r: &mut Reader<'_>) -> SaveResult<()> {
        self.calories = 0.0;
        self.proteins = 0.0;
        self.lipids = 0.0;
        self.carbohydrates = 0.0;
        self.hung_change = 0.0;
        self.base_hunger = 0.0;
        self.unhappy_change = 0.0;
        self.boredom_change = 0.0;
        self.thirst_change = 0.0;
        self.heat = 1.0;
        self.last_cook_minute = 0;
        self.cooking_time = 0.0;
        self.cooked = false;
        self.burnt = false;
        self.is_cookable = false;
        self.dangerous_uncooked = false;
        self.poison_detection_level = -1;
        self.spices = None;
        self.poison_power = 0;
        self.chef = None;
        self.off_age = 1_000_000_000;
        self.off_age_max = 1_000_000_000;
        self.pain_reduction = 0.0;
        self.flu_reduction = 0;
        self.food_sickness_change = 0;
        self.poison = false;
        self.use_for_poison = 0;
        self.freezing_time = 0.0;
        self.frozen = false;
        self.last_frozen_update = 0.0;
        self.rotten_time = 0.0;
        self.compost_time = 0.0;
        self.cooked_in_microwave = false;
        self.fatigue_change = 0.0;
        self.stress_change = 0.0;
        self.end_change = 0.0;
        self.age = r.f32()?;
        self.last_aged = r.f32()?;
        let header = read_header(r, 1)?;
        if header == 0 {
            return Ok(());
        }
        if has(header, 1) {
            self.calories = r.f32()?;
            self.proteins = r.f32()?;
            self.lipids = r.f32()?;
            self.carbohydrates = r.f32()?;
        }
        if has(header, 2) {
            self.hung_change = r.f32()?;
        }
        if has(header, 4) {
            self.base_hunger = r.f32()?;
        }
        if has(header, 8) {
            self.unhappy_change = r.f32()?;
        }
        if has(header, 16) {
            self.boredom_change = r.f32()?;
        }
        if has(header, 32) {
            self.thirst_change = r.f32()?;
        }
        if !has(header, 64) {
            return Ok(());
        }
        let bits = read_header(r, 4)?;
        if has(bits, 1) {
            self.heat = r.f32()?;
        }
        if has(bits, 2) {
            self.last_cook_minute = r.i32()?;
        }
        if has(bits, 4) {
            self.cooking_time = r.f32()?;
        }
        self.cooked = has(bits, 8);
        self.burnt = has(bits, 16);
        self.is_cookable = has(bits, 32);
        self.dangerous_uncooked = has(bits, 64);
        if has(bits, 128) {
            self.poison_detection_level = r.i8()? as i32;
        }
        if has(bits, 256) {
            let n = r.i8()?;
            let mut v = Vec::new();
            for _ in 0..n.max(0) {
                v.push(r.string()?);
            }
            self.spices = Some(v);
        }
        if has(bits, 512) {
            self.poison_power = r.i8()? as i32;
        }
        if has(bits, 1024) {
            self.chef = Some(r.string()?);
        }
        if has(bits, 2048) {
            self.off_age = r.i32()?;
        }
        if has(bits, 4096) {
            self.off_age_max = r.i32()?;
        }
        if has(bits, 8192) {
            self.pain_reduction = r.f32()?;
        }
        if has(bits, 16384) {
            self.flu_reduction = r.i32()?;
        }
        if has(bits, 32768) {
            self.food_sickness_change = r.i32()?;
        }
        self.poison = has(bits, 65536);
        if has(bits, 131072) {
            self.use_for_poison = r.i16()? as i32;
        }
        if has(bits, 262144) {
            self.freezing_time = r.f32()?;
        }
        self.frozen = has(bits, 524288);
        if has(bits, 0x100000) {
            self.last_frozen_update = r.f32()?;
        }
        if has(bits, 0x200000) {
            self.rotten_time = r.f32()?;
        }
        if has(bits, 0x400000) {
            self.compost_time = r.f32()?;
        }
        self.cooked_in_microwave = has(bits, 0x800000);
        if has(bits, 0x1000000) {
            self.fatigue_change = r.f32()?;
        }
        if has(bits, 0x2000000) {
            self.end_change = r.f32()?;
        }
        if has(bits, 0x4000000) {
            self.milk_qty = r.i32()?;
            self.milk_type = Some(r.string()?);
        }
        if has(bits, 0x8000000) {
            let time_to_hatch = r.i32()?;
            let fertilized_time = r.i32()?;
            let animal_hatch = r.string()?;
            let animal_hatch_breed = r.string()?;
            let n = r.i32()?;
            let mut genome: Vec<AnimalGene> = Vec::new();
            for _ in 0..n.max(0) {
                let g = AnimalGene::load(r)?;
                match genome.iter_mut().find(|e| e.name == g.name) {
                    Some(e) => *e = g,
                    None => genome.push(g),
                }
            }
            crate::java::sort_string_hash_map(&mut genome, |g| g.name.as_str());
            self.egg = Some(EggData {
                time_to_hatch,
                fertilized_time,
                animal_hatch,
                animal_hatch_breed,
                genome,
                mother_id: r.i32()?,
            });
            self.fertilized = true;
        }
        if has(bits, 0x10000000) {
            self.stress_change = r.f32()?;
        }
        Ok(())
    }
}

impl WeaponData {
    fn from_template(t: &ItemTemplate) -> Self {
        WeaponData {
            max_range: 1.0,
            min_range_ranged: 0.0,
            clip_size: 0,
            min_damage: t.min_damage,
            max_damage: t.max_damage,
            recoil_delay: 0,
            aiming_time: 0,
            reload_time: 0,
            hit_chance: 0,
            min_angle: 0.5,
            attachments: Vec::new(),
            fire_mode: t.fire_mode.clone(),
            cyclic_rate_multiplier: if t.cyclic_rate_multiplier > 0.0 {
                t.cyclic_rate_multiplier
            } else {
                0.0
            },
            explosion_timer: 0,
            max_angle: 1.0,
            blood_level: 0.0,
            contains_clip: false,
            round_chambered: false,
            is_jammed: false,
            spent_round_chambered: false,
            spent_round_count: 0,
            weapon_sprite: t.weapon_sprite.clone(),
            min_sight_range: t.min_sight_range,
            max_sight_range: t.max_sight_range,
            original_weapon_sprite: t.weapon_sprite.clone(),
            weapon_sprites_by_index: t.weapon_sprites_by_index.clone(),
            model_weapon_part: t.model_weapon_part.clone(),
        }
    }

    fn save(&self, w: &mut Writer, env: &dyn ItemEnv, t: &ItemTemplate) -> SaveResult<()> {
        let mut bits = BitHeaderWrite::alloc(w, 4);
        if self.max_range != 1.0 {
            bits.add(1);
            w.f32(self.max_range);
        }
        if self.min_range_ranged != 0.0 {
            bits.add(2);
            w.f32(self.min_range_ranged);
        }
        if self.clip_size != 0 {
            bits.add(4);
            w.i32(self.clip_size);
        }
        if self.min_damage != 0.4 {
            bits.add(8);
            w.f32(self.min_damage);
        }
        if self.max_damage != 1.5 {
            bits.add(16);
            w.f32(self.max_damage);
        }
        for (flag, v) in [
            (32, self.recoil_delay),
            (64, self.aiming_time),
            (128, self.reload_time),
            (256, self.hit_chance),
        ] {
            if v != 0 {
                bits.add(flag);
                w.i32(v);
            }
        }
        if self.min_angle != 0.5 {
            bits.add(512);
            w.f32(self.min_angle);
        }
        if !self.attachments.is_empty() {
            bits.add(1024);
            w.i8(self.attachments.len() as i8);
            for p in &self.attachments {
                p.save(w, env)?;
            }
        }
        if let Some(f) = &self.fire_mode {
            bits.add(2048);
            w.string(f);
        }
        if self.cyclic_rate_multiplier > 0.0 {
            bits.add(4096);
            w.f32(self.cyclic_rate_multiplier);
        }
        if self.explosion_timer != 0 {
            bits.add(65536);
            w.i32(self.explosion_timer);
        }
        if self.max_angle != 1.0 {
            bits.add(131072);
            w.f32(self.max_angle);
        }
        if self.blood_level != 0.0 {
            bits.add(262144);
            w.f32(self.blood_level);
        }
        if self.contains_clip {
            bits.add(524288);
        }
        if self.round_chambered {
            bits.add(0x100000);
        }
        if self.is_jammed {
            bits.add(0x200000);
        }
        if self.weapon_sprite != t.weapon_sprite {
            bits.add(0x400000);
            w.opt_string(self.weapon_sprite.as_deref());
        }
        if self.min_sight_range != 2.0 {
            bits.add(0x800000);
            w.f32(self.min_sight_range);
        }
        if self.max_sight_range != 6.0 {
            bits.add(0x1000000);
            w.f32(self.max_sight_range);
        }
        bits.write(w);
        Ok(())
    }

    fn load(
        &mut self,
        r: &mut Reader<'_>,
        wv: i32,
        env: &dyn ItemEnv,
        t: &ItemTemplate,
    ) -> SaveResult<()> {
        self.max_range = 1.0;
        self.min_range_ranged = 0.0;
        self.clip_size = 0;
        self.min_damage = 0.4;
        self.max_damage = 1.5;
        self.recoil_delay = 0;
        self.aiming_time = 0;
        self.reload_time = 0;
        self.hit_chance = 0;
        self.min_angle = 0.5;
        self.explosion_timer = 0;
        self.max_angle = 1.0;
        self.blood_level = 0.0;
        self.contains_clip = false;
        self.round_chambered = false;
        self.is_jammed = false;
        self.weapon_sprite = t.weapon_sprite.clone();
        let bits = read_header(r, 4)?;
        if bits == 0 {
            return Ok(());
        }
        if has(bits, 1) {
            self.max_range = r.f32()?;
        }
        if has(bits, 2) {
            self.min_range_ranged = r.f32()?;
        }
        if has(bits, 4) {
            self.clip_size = r.i32()?;
        }
        if has(bits, 8) {
            self.min_damage = r.f32()?;
        }
        if has(bits, 16) {
            self.max_damage = r.f32()?;
        }
        if has(bits, 32) {
            self.recoil_delay = r.i32()?;
        }
        if has(bits, 64) {
            self.aiming_time = r.i32()?;
        }
        if has(bits, 128) {
            self.reload_time = r.i32()?;
        }
        if has(bits, 256) {
            self.hit_chance = r.i32()?;
        }
        if has(bits, 512) {
            self.min_angle = r.f32()?;
        }
        if has(bits, 1024) {
            let n = r.i8()?;
            for _ in 0..n.max(0) {
                let reg = r.i16()?;
                let Some(mut part) = crate::env::create_item(env, reg) else {
                    // InventoryItemFactory.CreateItem returned null: the
                    // original throws and the whole item is dropped.
                    return Err(SaveError(format!("weapon part {reg} cannot be created")));
                };
                r.i8()?;
                part.load(r, wv, env)?;
                if part.template.class == ItemClass::WeaponPart {
                    self.attachments.push(part);
                }
            }
        }
        if has(bits, 2048) {
            self.fire_mode = Some(r.string()?);
        }
        if has(bits, 4096) {
            self.cyclic_rate_multiplier = r.f32()?;
        }
        if has(bits, 65536) {
            self.explosion_timer = r.i32()?;
        }
        if has(bits, 131072) {
            self.max_angle = r.f32()?;
        }
        if has(bits, 262144) {
            self.blood_level = r.f32()?;
        }
        self.contains_clip = has(bits, 524288);
        if t.magazine_type
            .as_deref()
            .is_none_or(|m| m.trim().is_empty())
        {
            self.contains_clip = false;
        }
        self.round_chambered = has(bits, 0x100000);
        self.is_jammed = has(bits, 0x200000);
        if has(bits, 0x400000) {
            self.weapon_sprite = Some(r.string()?);
        }
        if has(bits, 0x800000) {
            self.min_sight_range = r.f32()?;
        }
        if has(bits, 0x1000000) {
            self.max_sight_range = r.f32()?;
        }
        if has(bits, 0x2000000) {
            let n = r.i8()?;
            for _ in 0..n.max(0) {
                let reg = r.i16()?;
                if let Some(part) = crate::env::create_item(env, reg) {
                    if part.template.class == ItemClass::WeaponPart {
                        self.attachments.push(part);
                    }
                }
            }
        }
        Ok(())
    }
}

impl LiteratureData {
    fn save(&self, w: &mut Writer) {
        let mut bits = BitHeaderWrite::alloc(w, 2);
        let page_type = if self.number_of_pages >= 127 && self.number_of_pages < i16::MAX as i32 {
            1
        } else if self.number_of_pages >= i16::MAX as i32 {
            2
        } else {
            0
        };
        let put = |w: &mut Writer, v: i32| match page_type {
            1 => w.i16(v as i16),
            2 => w.i32(v),
            _ => w.i8(v as i8),
        };
        if self.number_of_pages != -1 {
            bits.add(1);
            match page_type {
                1 => bits.add(2),
                2 => bits.add(4),
                _ => {}
            }
            put(w, self.number_of_pages);
        }
        if self.already_read_pages != 0 {
            bits.add(8);
            put(w, self.already_read_pages);
        }
        if self.can_be_write {
            bits.add(16);
        }
        if let Some(p) = self.custom_pages.as_ref().filter(|p| !p.is_empty()) {
            bits.add(32);
            w.i32(p.len() as i32);
            for s in p {
                w.string(s);
            }
        }
        if let Some(s) = &self.locked_by {
            bits.add(64);
            w.string(s);
        }
        if let Some(l) = &self.learned_recipes {
            bits.add(128);
            w.i16(l.len() as i16);
            for s in l {
                w.string(s);
            }
        }
        bits.write(w);
    }

    fn load(&mut self, r: &mut Reader<'_>, wv: i32) -> SaveResult<()> {
        self.number_of_pages = -1;
        self.already_read_pages = 0;
        self.can_be_write = false;
        self.custom_pages = None;
        self.locked_by = None;
        self.learned_recipes = None;
        let bits = read_header(r, if wv >= 226 { 2 } else { 1 })?;
        if bits == 0 {
            return Ok(());
        }
        let mut page_type = 0;
        if has(bits, 1) {
            if has(bits, 2) {
                page_type = 1;
                self.number_of_pages = r.i16()? as i32;
            } else if has(bits, 4) {
                page_type = 2;
                self.number_of_pages = r.i32()?;
            } else {
                self.number_of_pages = r.i8()? as i32;
            }
        }
        if has(bits, 8) {
            self.already_read_pages = match page_type {
                1 => r.i16()? as i32,
                2 => r.i32()?,
                _ => r.i8()? as i32,
            };
        }
        self.can_be_write = has(bits, 16);
        if has(bits, 32) {
            let n = r.i32()?;
            if n > 0 {
                let mut v = Vec::new();
                for _ in 0..n {
                    v.push(r.string()?);
                }
                self.custom_pages = Some(v);
            }
        }
        if has(bits, 64) {
            self.locked_by = Some(r.string()?);
        }
        if wv >= 226 && has(bits, 128) {
            let n = r.i16()?;
            let mut v = Vec::new();
            for _ in 0..n.max(0) {
                v.push(r.string()?);
            }
            self.learned_recipes = Some(v);
        }
        Ok(())
    }
}

impl MoveableData {
    fn save(&self, w: &mut Writer) {
        w.string(&self.world_sprite);
        match &self.light {
            None => w.i8(0),
            Some(l) => {
                w.i8(1);
                w.bool(l.use_battery);
                w.bool(l.has_battery);
                match &l.bulb_item {
                    Some(b) => {
                        w.i8(1);
                        w.string(b);
                    }
                    None => w.i8(0),
                }
                for v in [l.power, l.delta, l.r, l.g, l.b] {
                    w.f32(v);
                }
            }
        }
    }

    fn load(&mut self, r: &mut Reader<'_>) -> SaveResult<()> {
        self.world_sprite = r.string()?;
        if r.bool()? {
            let use_battery = r.bool()?;
            let has_battery = r.bool()?;
            let bulb_item = if r.bool()? { Some(r.string()?) } else { None };
            // A light keeps an earlier bulb when none is saved.
            let bulb_item = bulb_item.or(self.light.as_ref().and_then(|l| l.bulb_item.clone()));
            self.light = Some(MoveableLight {
                use_battery,
                has_battery,
                bulb_item,
                power: r.f32()?,
                delta: r.f32()?,
                r: r.f32()?,
                g: r.f32()?,
                b: r.f32()?,
            });
        } else if let Some(l) = &self.light {
            // isLight = false keeps the light fields but they are not saved.
            let _ = l;
            self.light = None;
        }
        Ok(())
    }
}
