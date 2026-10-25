//! What `Item.InstanceItem` copies from an item script into a new
//! `InventoryItem`, reduced to the values item saving, loading and the
//! gameplay getters read.
//!
//! An [`ItemTemplate`] is built once per script item (see
//! [`ItemTemplate::from_script`]) and shared by every item of that type.

use std::rc::Rc;

use std::cell::RefCell;

use pz_scripts::objects::items::{Item, ItemReplacement, ModelWeaponPart};
use pz_scripts::ScriptManager;

/// A shared script item (`ScriptManager.FindItem`), compared by identity.
#[derive(Clone)]
pub struct ScriptItemRef(pub Rc<RefCell<Item>>);

impl PartialEq for ScriptItemRef {
    fn eq(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0)
    }
}

impl std::fmt::Debug for ScriptItemRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.try_borrow() {
            Ok(i) => write!(f, "ScriptItemRef({})", i.module_dot_type),
            Err(_) => write!(f, "ScriptItemRef(..)"),
        }
    }
}

/// The `InventoryItem` subclass `Item.InstanceItem` creates for the
/// script's `ItemType`, which decides what `save`/`load` write and read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemClass {
    /// `ComboItem` (`ItemType.NORMAL`).
    Normal,
    /// `Key`.
    Key,
    /// `WeaponPart`.
    WeaponPart,
    /// `InventoryContainer`.
    Container,
    /// `Food`.
    Food,
    /// `Literature`.
    Literature,
    /// `AlarmClock`.
    AlarmClock,
    /// `AlarmClockClothing`.
    AlarmClockClothing,
    /// `HandWeapon`.
    Weapon,
    /// `Clothing`.
    Clothing,
    /// `DrainableComboItem`.
    Drainable,
    /// `Radio` (a `Moveable`).
    Radio,
    /// `Moveable`.
    Moveable,
    /// `MapItem`.
    Map,
    /// `AnimalInventoryItem`.
    Animal,
}

impl ItemClass {
    /// The `ItemType` registry field name (`ItemType.NORMAL`, ...).
    pub fn from_item_type_field(field: &str) -> Option<Self> {
        Some(match field {
            "NORMAL" => ItemClass::Normal,
            "KEY" => ItemClass::Key,
            "WEAPON_PART" => ItemClass::WeaponPart,
            "CONTAINER" => ItemClass::Container,
            "FOOD" => ItemClass::Food,
            "LITERATURE" => ItemClass::Literature,
            "ALARM_CLOCK" => ItemClass::AlarmClock,
            "ALARM_CLOCK_CLOTHING" => ItemClass::AlarmClockClothing,
            "WEAPON" => ItemClass::Weapon,
            "CLOTHING" => ItemClass::Clothing,
            "DRAINABLE" => ItemClass::Drainable,
            "RADIO" => ItemClass::Radio,
            "MOVEABLE" => ItemClass::Moveable,
            "MAP" => ItemClass::Map,
            "ANIMAL" => ItemClass::Animal,
            _ => return None,
        })
    }

    /// `getClass().getSimpleName()`.
    pub fn java_name(self) -> &'static str {
        match self {
            ItemClass::Normal => "ComboItem",
            ItemClass::Key => "Key",
            ItemClass::WeaponPart => "WeaponPart",
            ItemClass::Container => "InventoryContainer",
            ItemClass::Food => "Food",
            ItemClass::Literature => "Literature",
            ItemClass::AlarmClock => "AlarmClock",
            ItemClass::AlarmClockClothing => "AlarmClockClothing",
            ItemClass::Weapon => "HandWeapon",
            ItemClass::Clothing => "Clothing",
            ItemClass::Drainable => "DrainableComboItem",
            ItemClass::Radio => "Radio",
            ItemClass::Moveable => "Moveable",
            ItemClass::Map => "MapItem",
            ItemClass::Animal => "AnimalInventoryItem",
        }
    }
}

/// Script values an item reads after creation.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemTemplate {
    pub class: ItemClass,
    /// `getModule().name`.
    pub module: String,
    /// `name` (the item's `type`).
    pub type_: String,
    /// `displayName`: the item's `originalName`.
    pub display_name: String,
    pub icon: Option<String>,
    pub condition_max: i32,
    /// `Item.spriteName` (`Clothing.spriteName` at creation).
    pub sprite_name: Option<String>,
    pub weapon_sprite: Option<String>,
    pub fire_mode: Option<String>,
    pub cyclic_rate_multiplier: f32,
    pub min_sight_range: f32,
    pub max_sight_range: f32,
    pub magazine_type: Option<String>,
    /// `Item.getStaticModel()` / `worldStaticModel` (resolved model script
    /// names).
    pub static_model: Option<String>,
    pub world_static_model: Option<String>,
    /// `staticModelsByIndex` / `worldStaticModelsByIndex` /
    /// `weaponSpritesByIndex` (shared with every instance, as the original
    /// shares the script's lists).
    pub static_models_by_index: Option<Rc<Vec<String>>>,
    pub world_static_models_by_index: Option<Rc<Vec<String>>>,
    pub weapon_sprites_by_index: Option<Rc<Vec<String>>>,
    /// `modelWeaponPart` (the script's entries, in script order).
    pub model_weapon_part: Option<Rc<Vec<ModelWeaponPart>>>,
    /// The script item itself, for `GameEntityFactory` (set by
    /// [`ItemTemplate::with_script`]; `None` builds no components).
    pub script: Option<ScriptItemRef>,
    /// `createItemInternal`'s `<type>Empty` fallback: the item is created
    /// from the base script and its `FluidContainer` emptied.
    pub empty_fluid_container: bool,
    /// `padlock` (a new padlock key draws its key id).
    pub padlock: bool,
    /// `paletteChoices.size()` (clothing draws one at creation).
    pub palette_choices: usize,
    pub world_object_sprite: Option<String>,
    /// For `Moveables.<sprite>` items: the sprite `InventoryItemFactory`
    /// reads (`ReadFromWorldSprite(movName)`).
    pub moveable_sprite: Option<String>,
    /// `getTags()` as registry ids (`base:egg`, ...).
    pub tags: Vec<String>,
    pub weapon_categories: Vec<String>,
    pub display_category: Option<String>,
    // Gameplay values read through the getters.
    pub actual_weight: f32,
    pub alcoholic: bool,
    pub alcohol_power: f32,
    pub reduce_infection_power: f32,
    pub boredom_change: f32,
    pub unhappy_change: f32,
    pub stress_change: f32,
    pub fatigue_change: f32,
    pub hunger_change: f32,
    pub thirst_change: f32,
    pub endurance_change: f32,
    pub poison_power: i32,
    pub on_eat: Option<String>,
    pub days_fresh: i32,
    pub days_totally_rotten: i32,
    pub is_cookable: bool,
    pub dangerous_uncooked: bool,
    pub good_hot: bool,
    pub bad_cold: bool,
    /// The script has component blocks (`GameEntityFactory` adds them).
    pub has_components: bool,
    pub ranged: bool,
    pub is_aimed_firearm: bool,
    pub min_damage: f32,
    pub max_damage: f32,
    pub bite_defense: f32,
    pub scratch_defense: f32,
    pub bullet_defense: f32,
    pub corpse_sickness_defense: f32,
    pub insulation: f32,
    pub windresist: f32,
    pub waterresist: f32,
    pub weight_wet: f32,
    /// `wetCooldown` (`WetCooldown`; 0 when the script has none), which
    /// `InstanceItem` gives every new item.
    pub wet_cooldown: f32,
    pub capacity: i32,
    pub weight_reduction: i32,
    /// `maxItemSize` (an `InventoryContainer`'s per-item weight limit).
    pub max_item_size: f32,
    /// `PutInSound`, `OpenSound`, `CloseSound`, `OnlyAcceptCategory`,
    /// `AcceptItemFunction`: what `InstanceItem` gives a bag's inventory.
    pub put_in_sound: Option<String>,
    pub open_sound: Option<String>,
    pub close_sound: Option<String>,
    pub only_accept_category: Option<String>,
    pub accept_item_function: Option<String>,
    pub use_delta: f32,
    pub count: i32,
    // Values the gameplay methods (`gameplay.rs`) read from the script
    // (`getScriptItem()`), resolved once when the template is built.
    /// `clothingItem` (the `ClothingItem` asset name).
    pub clothing_item: Option<String>,
    /// `replacePrimaryHand` (clothing item name, mask variable name/value,
    /// attachment).
    pub replace_primary_hand: Option<ItemReplacement>,
    /// `replaceSecondHand`.
    pub replace_second_hand: Option<ItemReplacement>,
    /// `getWithoutDrainable()`.
    pub without_drainable: Option<String>,
    /// The `clothingItem` of the `withoutDrainable` script, if it exists.
    pub without_drainable_clothing_item: Option<String>,
    /// `bloodClothingType` names, in script order.
    pub blood_clothing_type: Option<Vec<String>>,
    /// `getBodyLocation()` as its registry id (`base:hat`, ...).
    pub body_location: Option<String>,
    pub equipped_no_sprint: bool,
    pub protect_from_rain_when_equipped: bool,
    pub neck_protection_modifier: f32,
    pub disappear_on_use: bool,
    pub replace_on_use: Option<String>,
    pub replace_on_deplete: Option<String>,
    pub weight_empty: f32,
    pub swing_anim: Option<String>,
    pub two_hand_weapon: bool,
    pub have_chamber: bool,
    /// `ammoType.getItemKey()`.
    pub ammo_item: Option<String>,
    pub cosmetic: bool,
    pub remove_on_broken: bool,
    pub can_have_holes: bool,
    pub break_sound: Option<String>,
    pub damaged_sound: Option<String>,
    pub on_break: Option<String>,
    /// `isItemType(ItemType.KEY_RING)`.
    pub key_ring_type: bool,
    /// `WeaponPart.getWeightModifier()` (`weightModifier`).
    pub weight_modifier: f32,
    /// `activatedItem` (`InventoryItem.canBeActivated` at creation).
    pub activated_item: bool,
    /// `lightStrength`, `torchCone`, `lightDistance` (copied to the item at
    /// creation) and `torchDot` (read through the script).
    pub light_strength: f32,
    pub torch_cone: bool,
    pub light_distance: i32,
    pub torch_dot: f32,
    /// `itemCapacity` (`setItemCapacity` at creation).
    pub item_capacity: i32,
    /// `Food.setCalories` / `setProteins` / `setLipids` /
    /// `setCarbohydrates` at creation.
    pub calories: f32,
    pub proteins: f32,
    pub lipids: f32,
    pub carbohydrates: f32,
    /// `Literature.setNumberOfPages(numberOfPages)` at creation.
    pub number_of_pages: i32,
    /// `maxCapacity` (`setMaxCapacity` at creation).
    pub max_capacity: i32,
    /// `conditionAffectsCapacity` (read through `getScriptItem()`).
    pub condition_affects_capacity: bool,
    /// The vehicle-part values `InstanceItem` copies to every item.
    pub vehicle_part: VehiclePartStats,
    /// The `DeviceData` values `InstanceItem` sets on a `Radio`.
    pub radio: RadioScript,
    /// `map` (`MapItem.setMapID`; a random map definition when blank).
    pub map: Option<String>,
}

/// `InventoryItem` fields `Item.InstanceItem` sets for vehicle parts
/// (`setBrakeForce`, `setDurability`, ...). They are not saved: a loaded
/// item keeps the values of the item `CreateItem` made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehiclePartStats {
    pub brake_force: f32,
    pub durability: f32,
    pub chance_to_spawn_damaged: i32,
    pub condition_lower_normal: f32,
    pub condition_lower_offroad: f32,
    pub wheel_friction: f32,
    pub suspension_compression: f32,
    pub engine_loudness: f32,
    pub suspension_damping: f32,
}

impl VehiclePartStats {
    /// The values of an item script.
    pub fn from_script(item: &Item) -> Self {
        VehiclePartStats {
            brake_force: item.brake_force,
            durability: item.durability,
            chance_to_spawn_damaged: item.chance_to_spawn_damaged,
            condition_lower_normal: item.condition_lower_normal,
            condition_lower_offroad: item.condition_lower_offroad,
            wheel_friction: item.wheel_friction,
            suspension_compression: item.suspension_compression,
            engine_loudness: item.engine_loudness,
            suspension_damping: item.suspension_damping,
        }
    }
}

/// Item script values `Item.InstanceItem` gives a `Radio`'s `DeviceData`.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioScript {
    pub two_way: bool,
    pub transmit_range: i32,
    pub mic_range: i32,
    pub base_volume_range: f32,
    pub is_portable: bool,
    pub is_television: bool,
    pub min_channel: i32,
    pub max_channel: i32,
    pub uses_battery: bool,
    pub is_high_tier: bool,
    pub use_delta: f32,
    pub accept_media_type: i8,
    pub no_transmit: bool,
}

impl Default for RadioScript {
    /// The `Item` field defaults.
    fn default() -> Self {
        RadioScript {
            two_way: false,
            transmit_range: 0,
            mic_range: 0,
            base_volume_range: 0.0,
            is_portable: false,
            is_television: false,
            min_channel: 88000,
            max_channel: 108000,
            uses_battery: false,
            is_high_tier: false,
            use_delta: 0.03125,
            accept_media_type: -1,
            no_transmit: false,
        }
    }
}

impl RadioScript {
    /// The values of an item script.
    pub fn from_script(item: &Item) -> Self {
        RadioScript {
            two_way: item.two_way,
            transmit_range: item.transmit_range,
            mic_range: item.mic_range,
            base_volume_range: item.base_volume_range,
            is_portable: item.is_portable,
            is_television: item.is_television,
            min_channel: item.min_channel,
            max_channel: item.max_channel,
            uses_battery: item.uses_battery,
            is_high_tier: item.is_high_tier,
            use_delta: item.use_delta,
            accept_media_type: item.accept_media_type,
            no_transmit: item.no_transmit,
        }
    }
}

impl ItemTemplate {
    /// A template with `InventoryItem`'s defaults (for tests and items
    /// built without a script).
    pub fn new(class: ItemClass, module: &str, type_: &str) -> Self {
        ItemTemplate {
            class,
            module: module.into(),
            type_: type_.into(),
            display_name: type_.into(),
            icon: None,
            condition_max: 10,
            sprite_name: None,
            weapon_sprite: None,
            fire_mode: None,
            cyclic_rate_multiplier: 0.0,
            min_sight_range: 2.0,
            max_sight_range: 6.0,
            magazine_type: None,
            static_model: None,
            world_static_model: None,
            static_models_by_index: None,
            world_static_models_by_index: None,
            weapon_sprites_by_index: None,
            model_weapon_part: None,
            script: None,
            empty_fluid_container: false,
            padlock: false,
            palette_choices: 0,
            world_object_sprite: None,
            moveable_sprite: None,
            tags: Vec::new(),
            weapon_categories: Vec::new(),
            display_category: None,
            actual_weight: 1.0,
            alcoholic: false,
            alcohol_power: 0.0,
            reduce_infection_power: 0.0,
            boredom_change: 0.0,
            unhappy_change: 0.0,
            stress_change: 0.0,
            fatigue_change: 0.0,
            hunger_change: 0.0,
            thirst_change: 0.0,
            endurance_change: 0.0,
            poison_power: 0,
            on_eat: None,
            days_fresh: 1_000_000_000,
            days_totally_rotten: 1_000_000_000,
            is_cookable: false,
            dangerous_uncooked: false,
            good_hot: false,
            bad_cold: false,
            has_components: false,
            ranged: false,
            is_aimed_firearm: false,
            min_damage: 0.4,
            max_damage: 1.5,
            bite_defense: 0.0,
            scratch_defense: 0.0,
            bullet_defense: 0.0,
            corpse_sickness_defense: 0.0,
            insulation: 0.0,
            windresist: 0.0,
            waterresist: 0.0,
            weight_wet: 0.0,
            wet_cooldown: 0.0,
            capacity: 0,
            weight_reduction: 0,
            max_item_size: 0.0,
            put_in_sound: None,
            open_sound: None,
            close_sound: None,
            only_accept_category: None,
            accept_item_function: None,
            use_delta: 0.03125,
            count: 1,
            clothing_item: None,
            replace_primary_hand: None,
            replace_second_hand: None,
            without_drainable: None,
            without_drainable_clothing_item: None,
            blood_clothing_type: None,
            body_location: None,
            equipped_no_sprint: false,
            protect_from_rain_when_equipped: false,
            neck_protection_modifier: 1.0,
            disappear_on_use: true,
            replace_on_use: None,
            replace_on_deplete: None,
            weight_empty: 0.0,
            swing_anim: None,
            two_hand_weapon: false,
            have_chamber: true,
            ammo_item: None,
            cosmetic: false,
            remove_on_broken: true,
            can_have_holes: true,
            break_sound: None,
            damaged_sound: None,
            on_break: None,
            key_ring_type: false,
            weight_modifier: 0.0,
            activated_item: false,
            light_strength: 0.0,
            torch_cone: false,
            light_distance: 0,
            torch_dot: 0.96,
            item_capacity: -1,
            calories: 0.0,
            proteins: 0.0,
            lipids: 0.0,
            carbohydrates: 0.0,
            number_of_pages: -1,
            max_capacity: -1,
            condition_affects_capacity: false,
            vehicle_part: VehiclePartStats::default(),
            radio: RadioScript::default(),
            map: None,
        }
    }

    pub fn full_type(&self) -> String {
        format!("{}.{}", self.module, self.type_)
    }

    /// `hasTag(ItemTag.X)` by registry id (e.g. `base:egg`).
    pub fn has_tag(&self, id: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(id))
    }

    /// [`from_script`](Self::from_script) keeping the script item (for the
    /// factory-made ECS components).
    pub fn from_script_ref(sm: &ScriptManager, item: &Rc<RefCell<Item>>) -> Option<Rc<Self>> {
        let t = Self::from_script(sm, &item.borrow())?;
        let mut t = Rc::try_unwrap(t).unwrap_or_else(|t| (*t).clone());
        t.script = Some(ScriptItemRef(item.clone()));
        Some(Rc::new(t))
    }

    /// Reads the values from a loaded script item.
    pub fn from_script(sm: &ScriptManager, item: &Item) -> Option<Rc<Self>> {
        const TYPES: [&str; 15] = [
            "NORMAL",
            "KEY",
            "WEAPON_PART",
            "CONTAINER",
            "FOOD",
            "LITERATURE",
            "ALARM_CLOCK",
            "ALARM_CLOCK_CLOTHING",
            "WEAPON",
            "CLOTHING",
            "DRAINABLE",
            "RADIO",
            "MOVEABLE",
            "MAP",
            "ANIMAL",
        ];
        let class = TYPES
            .iter()
            .find(|f| item.is_item_type(sm, f))
            .and_then(|f| ItemClass::from_item_type_field(f))?;
        let full = item.module_dot_type.clone();
        let (module, type_) = full.split_once('.').unwrap_or(("Base", full.as_str()));
        let keys = |k: &pz_scripts::objects::items::KeySet| {
            k.iter()
                .flatten()
                .map(|k| k.location.to_string())
                .collect::<Vec<_>>()
        };
        Some(Rc::new(ItemTemplate {
            class,
            module: module.to_string(),
            type_: type_.to_string(),
            display_name: item.display_name.clone().unwrap_or_default(),
            icon: item.icon.clone(),
            condition_max: item.condition_max,
            sprite_name: item.sprite_name.clone(),
            weapon_sprite: item.weapon_sprite.clone(),
            fire_mode: item.fire_mode.clone(),
            cyclic_rate_multiplier: item.cyclic_rate_multiplier,
            min_sight_range: item.min_sight_range,
            max_sight_range: item.max_sight_range,
            magazine_type: item.magazine_type.clone(),
            static_model: item.static_model.clone(),
            world_static_model: item.world_static_model.clone(),
            static_models_by_index: item.static_models_by_index.clone().map(Rc::new),
            world_static_models_by_index: item.world_static_models_by_index.clone().map(Rc::new),
            weapon_sprites_by_index: item.weapon_sprites_by_index.clone().map(Rc::new),
            model_weapon_part: item
                .model_weapon_part
                .as_ref()
                .map(|v| Rc::new(v.iter().map(|m| m.borrow().clone()).collect())),
            script: None,
            empty_fluid_container: false,
            padlock: item.padlock,
            palette_choices: item.palette_choices.len(),
            world_object_sprite: item.world_object_sprite.clone(),
            moveable_sprite: None,
            tags: keys(&item.item_tags),
            weapon_categories: keys(&item.weapon_categories),
            display_category: item.display_category.clone(),
            actual_weight: item.actual_weight,
            alcoholic: item.alcoholic,
            alcohol_power: item.alcohol_power,
            reduce_infection_power: item.reduce_infection_power,
            boredom_change: item.boredom_change,
            unhappy_change: item.unhappy_change,
            stress_change: item.stress_change,
            fatigue_change: item.fatigue_change,
            hunger_change: item.hunger_change,
            thirst_change: item.thirst_change,
            endurance_change: item.endurance_change,
            poison_power: item.poison_power,
            on_eat: item.on_eat.clone(),
            days_fresh: item.days_fresh,
            days_totally_rotten: item.days_totally_rotten,
            is_cookable: item.is_cookable,
            dangerous_uncooked: item.dangerous_uncooked,
            good_hot: item.good_hot,
            bad_cold: item.bad_cold,
            has_components: item.entity.has_components(),
            ranged: item.ranged,
            is_aimed_firearm: item.is_aimed_firearm,
            min_damage: item.min_damage,
            max_damage: item.max_damage,
            bite_defense: item.bite_defense,
            scratch_defense: item.scratch_defense,
            bullet_defense: item.bullet_defense,
            corpse_sickness_defense: item.corpse_sickness_defense,
            insulation: item.insulation,
            windresist: item.windresist,
            waterresist: item.waterresist,
            weight_wet: item.weight_wet,
            wet_cooldown: item.wet_cooldown,
            capacity: item.capacity,
            weight_reduction: item.weight_reduction,
            max_item_size: item.max_item_size,
            put_in_sound: item.put_in_sound.clone(),
            open_sound: item.open_sound.clone(),
            close_sound: item.close_sound.clone(),
            only_accept_category: item.only_accept_category.clone(),
            accept_item_function: item.accept_item_function.clone(),
            use_delta: item.use_delta,
            count: item.count,
            clothing_item: item.clothing_item.clone(),
            replace_primary_hand: item.replace_primary_hand.clone(),
            replace_second_hand: item.replace_second_hand.clone(),
            without_drainable: item.without_drainable.clone(),
            without_drainable_clothing_item: item
                .without_drainable
                .as_deref()
                .and_then(|n| pz_scripts::objects::items::get_item(sm, n))
                .and_then(|i| i.borrow().clothing_item.clone()),
            blood_clothing_type: item.blood_clothing_type.clone(),
            body_location: item.body_location.as_ref().map(|k| k.location.to_string()),
            equipped_no_sprint: item.equipped_no_sprint,
            protect_from_rain_when_equipped: item.protect_from_rain_when_equipped,
            neck_protection_modifier: item.neck_protection_modifier,
            disappear_on_use: item.disappear_on_use,
            replace_on_use: item.replace_on_use.clone(),
            replace_on_deplete: item.replace_on_deplete.clone(),
            weight_empty: item.weight_empty,
            swing_anim: item.swing_anim.clone(),
            two_hand_weapon: item.two_hand_weapon,
            have_chamber: item.have_chamber,
            ammo_item: item.ammo_type.as_ref().and_then(|k| k.item_key.clone()),
            cosmetic: item.cosmetic,
            remove_on_broken: item.remove_on_broken,
            can_have_holes: item.can_have_holes,
            break_sound: item.break_sound.clone(),
            damaged_sound: item.damaged_sound.clone(),
            on_break: item.on_break.clone(),
            key_ring_type: item.is_item_type(sm, "KEY_RING"),
            weight_modifier: item.weight_modifier,
            activated_item: item.activated_item,
            light_strength: item.light_strength,
            torch_cone: item.torch_cone,
            light_distance: item.light_distance,
            torch_dot: item.torch_dot,
            item_capacity: item.item_capacity,
            calories: item.calories,
            proteins: item.proteins,
            lipids: item.lipids,
            carbohydrates: item.carbohydrates,
            number_of_pages: item.number_of_pages,
            max_capacity: item.max_capacity,
            condition_affects_capacity: item.condition_affects_capacity,
            vehicle_part: VehiclePartStats::from_script(item),
            radio: RadioScript::from_script(item),
            map: item.map.clone(),
        }))
    }
}
