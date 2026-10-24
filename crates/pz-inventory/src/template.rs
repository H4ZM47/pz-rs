//! What `Item.InstanceItem` copies from an item script into a new
//! `InventoryItem`, reduced to the values item saving, loading and the
//! gameplay getters read.
//!
//! An [`ItemTemplate`] is built once per script item (see
//! [`ItemTemplate::from_script`]) and shared by every item of that type.

use std::rc::Rc;

use pz_scripts::objects::items::Item;
use pz_scripts::ScriptManager;

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
    pub weapon_sprites_by_index: bool,
    pub static_models_by_index: bool,
    pub world_object_sprite: Option<String>,
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
    pub capacity: i32,
    pub weight_reduction: i32,
    pub use_delta: f32,
    pub count: i32,
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
            weapon_sprites_by_index: false,
            static_models_by_index: false,
            world_object_sprite: None,
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
            capacity: 0,
            weight_reduction: 0,
            use_delta: 0.03125,
            count: 1,
        }
    }

    pub fn full_type(&self) -> String {
        format!("{}.{}", self.module, self.type_)
    }

    /// `hasTag(ItemTag.X)` by registry id (e.g. `base:egg`).
    pub fn has_tag(&self, id: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(id))
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
            weapon_sprites_by_index: item.weapon_sprites_by_index.is_some(),
            static_models_by_index: item
                .static_models_by_index
                .as_ref()
                .is_some_and(|v| !v.is_empty())
                || item
                    .world_static_models_by_index
                    .as_ref()
                    .is_some_and(|v| !v.is_empty()),
            world_object_sprite: item.world_object_sprite.clone(),
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
            capacity: item.capacity,
            weight_reduction: item.weight_reduction,
            use_delta: item.use_delta,
            count: item.count,
        }))
    }
}
