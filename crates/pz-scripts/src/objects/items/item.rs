//! `zombie.scripting.objects.Item`: the script side of an inventory item
//! (fields, `Load`/`DoParam`, `OnScriptsLoaded`, `resolveItemTypes`).
//!
//! Field defaults and parsing follow the original exactly. Engine-side state
//! (textures, the clothing asset, the Lua mod-data table) is kept as the data
//! it is built from. TODO(inventory): `InstanceItem` (creating the
//! `InventoryItem` subclasses).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use super::keys::{
    self, AmmoType, BookSubject, ItemBodyLocation, ItemTag, ItemType, MagazineSubject,
    WeaponCategory,
};
use super::parts::{ItemReplacement, ModelWeaponPart, VehiclePartModel};
use super::{
    at, discard_null_or_whitespace, exception_message, invalid_parameter, parse_byte, parse_float,
    parse_int, try_get_texture,
};
use crate::base::{
    index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptRef, ScriptResult,
};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::objects::entity::components::attributes::attribute_type_from_name;
use crate::objects::entity::{CraftRecipe, GameEntityScript};
use crate::objects::recipes::{self, ItemRecipe};
use crate::parser::{Block, BlockElement};
use crate::registry::RegistryKey;
use crate::script_type::{ScriptLoadMode, ScriptType};

/// A `HashSet` of registry keys (`Set<ItemTag>`, `Set<WeaponCategory>`).
/// The original's iteration order follows identity hashes; this keeps
/// insertion order. `None` stands for a `null` added for an unknown id.
#[derive(Clone, Debug, Default)]
pub struct KeySet(Vec<Option<Rc<RegistryKey>>>);

impl KeySet {
    /// `add`.
    pub fn add(&mut self, key: Option<Rc<RegistryKey>>) {
        if !self.contains(key.as_ref()) {
            self.0.push(key);
        }
    }

    /// `contains`.
    pub fn contains(&self, key: Option<&Rc<RegistryKey>>) -> bool {
        self.0.iter().any(|k| match (k, key) {
            (Some(a), Some(b)) => a.location == b.location,
            (None, None) => true,
            _ => false,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = Option<&Rc<RegistryKey>>> {
        self.0.iter().map(Option::as_ref)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A value of the `defaultModData` table: unknown params whose value parses
/// as a double are stored as numbers, the rest as strings.
#[derive(Clone, Debug, PartialEq)]
pub enum ModDataValue {
    Number(f64),
    String(String),
}

#[derive(Debug)]
pub struct Item {
    /// The `GameEntityScript` this class extends (its base holds the
    /// `BaseScriptObject` state).
    pub entity: GameEntityScript,
    pub clothing_extra_submenu: Option<String>,
    pub display_name: Option<String>,
    pub hidden: bool,
    pub cant_eat: bool,
    pub icon: Option<String>,
    normal_texture: Option<pz_render::TextureRef>,
    world_texture: Option<pz_render::TextureRef>,
    special_textures: Vec<Option<pz_render::TextureRef>>,
    texture_capture_complete: bool,
    pub medical: bool,
    pub canned_food: bool,
    pub survival_gear: bool,
    pub mechanics_item: bool,
    pub use_world_item: bool,
    pub scale_world_icon: f32,
    pub close_kill_move: Option<String>,
    pub weapon_length: f32,
    pub actual_weight: f32,
    pub weight_wet: f32,
    pub weight_empty: f32,
    pub hunger_change: f32,
    pub critical_chance: f32,
    pub count: i32,
    pub days_fresh: i32,
    pub days_totally_rotten: i32,
    pub minutes_to_cook: i32,
    pub minutes_to_burn: i32,
    pub is_cookable: bool,
    pub cooking_sound: Option<String>,
    pub stress_change: f32,
    pub boredom_change: f32,
    pub unhappy_change: f32,
    pub always_welcome_gift: bool,
    pub replace_on_deplete: Option<String>,
    pub replace_on_extinguish: Option<String>,
    pub ranged: bool,
    pub can_store_water: bool,
    pub max_range: f32,
    pub min_range: f32,
    pub thirst_change: f32,
    pub fatigue_change: f32,
    pub min_angle: f32,
    pub requires_equipped_both_hands: bool,
    pub max_damage: f32,
    pub min_damage: f32,
    pub minimum_swing_time: f32,
    pub swing_sound: Option<String>,
    pub weapon_sprite: Option<String>,
    pub angle_falloff: bool,
    pub sound_volume: i32,
    pub to_hit_modifier: f32,
    pub sound_radius: i32,
    pub other_character_volume_boost: f32,
    pub weapon_categories: KeySet,
    pub item_tags: KeySet,
    pub impact_sound: Option<String>,
    pub swing_time: f32,
    pub knock_back_on_no_death: bool,
    pub splat_blood_on_no_death: bool,
    pub swing_amount_before_impact: f32,
    pub ammo_type: Option<Rc<RegistryKey>>,
    pub max_ammo: i32,
    pub gun_type: Option<Vec<String>>,
    pub door_damage: i32,
    pub condition_lower_chance: i32,
    pub head_condition_lower_chance_multiplier: f32,
    pub condition_max: i32,
    pub can_bandage: bool,
    pub name: String,
    pub module_dot_type: String,
    pub max_hit_count: i32,
    pub use_self: bool,
    pub other_hand_use: bool,
    pub other_hand_require: Option<Rc<RegistryKey>>,
    pub physics_object: Option<String>,
    pub swing_anim: Option<String>,
    pub weapon_weight: f32,
    pub endurance_change: f32,
    pub idle_anim: Option<String>,
    pub run_anim: Option<String>,
    pub attachment_type: Option<String>,
    pub make_up_type: Option<String>,
    pub consolidate_option: Option<String>,
    pub require_in_hand_or_inventory: Option<Vec<Option<String>>>,
    pub door_hit_sound: Option<String>,
    pub replace_on_use: Option<String>,
    pub dangerous_uncooked: bool,
    pub alcoholic: bool,
    pub push_back_mod: f32,
    pub splat_number: i32,
    pub npc_sound_boost: f32,
    pub range_falloff: bool,
    pub use_endurance: bool,
    pub multiple_hit_condition_affected: bool,
    pub share_endurance: bool,
    pub can_barricade: bool,
    pub use_while_equipped: bool,
    pub use_while_unequipped: bool,
    pub ticks_per_equip_use: i32,
    pub disappear_on_use: bool,
    pub use_delta: f32,
    pub always_knockdown: bool,
    pub endurance_mod: f32,
    pub knockdown_mod: f32,
    pub cant_attack_with_lowest_endurance: bool,
    pub replace_on_use_on: Option<String>,
    pub replace_types: Option<String>,
    pub replace_types_map: Option<JHashMap<Option<String>>>,
    pub attachments_provided: Option<Vec<String>>,
    pub food_type: Option<String>,
    pub poison: bool,
    pub poison_detection_level: i32,
    pub poison_power: i32,
    /// `defaultModData`: unknown params in the order they were set.
    /// TODO(Lua): the original stores them in a `KahluaTable`.
    pub default_mod_data: Option<Vec<(String, ModDataValue)>>,
    pub is_aimed_firearm: bool,
    pub is_aimed_hand_weapon: bool,
    pub can_stack: bool,
    pub aiming_mod: f32,
    pub projectile_count: i32,
    pub projectile_spread: f32,
    pub projectile_weight_center: f32,
    pub hit_angle_mod: f32,
    pub splat_size: f32,
    pub temperature: f32,
    pub number_of_pages: i32,
    pub lvl_skill_trained: i32,
    pub num_levels_trained: i32,
    pub skill_trained: Option<String>,
    pub capacity: i32,
    pub max_item_size: f32,
    pub weight_reduction: i32,
    pub sub_category: Option<String>,
    pub activated_item: bool,
    pub light_strength: f32,
    pub torch_cone: bool,
    pub light_distance: i32,
    pub can_be_equipped: Option<Rc<RegistryKey>>,
    pub two_hand_weapon: bool,
    pub custom_context_menu: Option<String>,
    pub tooltip: Option<String>,
    pub replace_on_cooked: Option<Vec<String>>,
    pub display_category: Option<String>,
    pub trap: bool,
    pub obsolete: bool,
    pub fishing_lure: bool,
    pub can_be_write: bool,
    pub aiming_perk_crit_modifier: i32,
    pub aiming_perk_range_modifier: f32,
    pub aiming_perk_hit_chance_modifier: f32,
    pub hit_chance: i32,
    pub aiming_perk_min_angle_modifier: f32,
    pub recoil_delay: i32,
    pub piercing_bullets: bool,
    pub sound_gain: f32,
    pub protect_from_rain_when_equipped: bool,
    pub max_range_modifier: f32,
    pub min_sight_range: f32,
    pub max_sight_range: f32,
    pub low_light_bonus: f32,
    pub min_range_ranged_modifier: f32,
    pub damage_modifier: f32,
    pub recoil_delay_modifier: f32,
    pub clip_size_modifier: i32,
    pub mount_on: Option<Vec<String>>,
    pub part_type: Option<String>,
    pub can_attach_callback: Option<String>,
    pub can_detach_callback: Option<String>,
    pub on_attach_callback: Option<String>,
    pub on_detach_callback: Option<String>,
    pub clip_size: i32,
    pub reload_time: i32,
    pub reload_time_modifier: i32,
    pub aiming_time: i32,
    pub aiming_time_modifier: i32,
    pub hit_chance_modifier: i32,
    pub angle_modifier: f32,
    pub projectile_spread_modifier: f32,
    pub weight_modifier: f32,
    pub page_to_write: i32,
    pub remove_negative_effect_on_cooked: bool,
    pub tree_damage: i32,
    pub alcohol_power: f32,
    pub put_in_sound: Option<String>,
    pub place_one_sound: Option<String>,
    pub place_multiple_sound: Option<String>,
    pub open_sound: Option<String>,
    pub close_sound: Option<String>,
    pub break_sound: Option<String>,
    pub custom_eat_sound: Option<String>,
    pub fill_from_dispenser_sound: Option<String>,
    pub fill_from_lake_sound: Option<String>,
    pub fill_from_tap_sound: Option<String>,
    pub fill_from_toilet_sound: Option<String>,
    pub bullet_out_sound: Option<String>,
    pub shell_fall_sound: Option<String>,
    pub drop_sound: Option<String>,
    pub sound_map: Option<JHashMap<String>>,
    pub bandage_power: f32,
    pub reduce_infection_power: f32,
    pub on_cooked: Option<String>,
    pub only_accept_category: Option<String>,
    pub accept_item_function: Option<String>,
    pub padlock: bool,
    pub digital_padlock: bool,
    pub learned_recipes: Option<Vec<String>>,
    pub trigger_explosion_timer: i32,
    pub can_be_placed: bool,
    pub explosion_range: i32,
    pub explosion_power: i32,
    pub fire_range: i32,
    pub fire_starting_chance: i32,
    pub fire_starting_energy: i32,
    pub smoke_range: i32,
    pub noise_range: i32,
    pub noise_duration: i32,
    pub extra_damage: f32,
    pub explosion_timer: i32,
    pub explosion_duration: i32,
    pub placed_sprite: Option<String>,
    pub can_be_reused: bool,
    pub sensor_range: i32,
    pub can_be_remote: bool,
    pub remote_controller: bool,
    pub remote_range: i32,
    pub count_down_sound: Option<String>,
    pub explosion_sound: Option<String>,
    pub flu_reduction: i32,
    pub food_sickness_change: i32,
    pub inverse_cough_probability: i32,
    pub inverse_cough_probability_smoker: i32,
    pub pain_reduction: i32,
    pub torch_dot: f32,
    pub color_red: i32,
    pub color_green: i32,
    pub color_blue: i32,
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
    pub herbalist_type: Option<String>,
    pub carbohydrates: f32,
    pub lipids: f32,
    pub proteins: f32,
    pub calories: f32,
    pub packaged: bool,
    pub cant_be_frozen: bool,
    pub evolved_recipe_name: Option<String>,
    pub replace_on_rotten: Option<String>,
    pub metal_value: f32,
    pub alarm_sound: Option<String>,
    pub item_when_dry: Option<String>,
    pub wet_cooldown: f32,
    pub is_wet: bool,
    pub on_eat: Option<String>,
    pub cant_be_consolided: bool,
    pub bad_in_microwave: bool,
    pub good_hot: bool,
    pub bad_cold: bool,
    pub map: Option<String>,
    pub vehicle_type: i32,
    pub vehicle_part_models: Option<Vec<VehiclePartModel>>,
    pub max_capacity: i32,
    pub item_capacity: i32,
    pub condition_affects_capacity: bool,
    pub brake_force: f32,
    pub durability: f32,
    pub chance_to_spawn_damaged: i32,
    pub condition_lower_normal: f32,
    pub condition_lower_offroad: f32,
    pub wheel_friction: f32,
    pub suspension_damping: f32,
    pub suspension_compression: f32,
    pub engine_loudness: f32,
    /// TODO(ClothingItem): the resolved `clothingItemAsset`.
    pub clothing_item: Option<String>,
    pub static_model: Option<String>,
    pub primary_anim_mask: Option<String>,
    pub secondary_anim_mask: Option<String>,
    pub primary_anim_mask_attachment: Option<String>,
    pub secondary_anim_mask_attachment: Option<String>,
    pub replace_in_second_hand: Option<String>,
    pub replace_in_primary_hand: Option<String>,
    pub replace_when_unequip: Option<String>,
    pub replace_primary_hand: Option<ItemReplacement>,
    pub replace_second_hand: Option<ItemReplacement>,
    pub world_object_sprite: Option<String>,
    /// `"Item_" + icon`, the name of the item's texture.
    /// TODO(Texture): `normalTexture`, `specialTextures`, `worldTexture`.
    pub item_name: Option<String>,
    pub special_world_texture_names: Vec<String>,
    pub world_texture_name: Option<String>,
    pub eat_type: Option<String>,
    pub pour_type: Option<String>,
    pub read_type: Option<String>,
    pub dig_type: Option<String>,
    pub icons_for_texture: Option<Vec<String>>,
    pub base_speed: f32,
    /// `BloodClothingType` names; `BloodClothingType.fromString` maps each
    /// to its constant. TODO(BloodClothingType).
    pub blood_clothing_type: Option<Vec<String>>,
    pub stomp_power: f32,
    pub run_speed_modifier: f32,
    pub combat_speed_modifier: f32,
    pub clothing_item_extra: Option<Vec<String>>,
    pub clothing_item_extra_option: Option<Vec<String>>,
    pub remove_on_broken: bool,
    pub can_have_holes: bool,
    pub cosmetic: bool,
    pub ammo_box: Option<String>,
    pub insert_ammo_start_sound: Option<String>,
    pub insert_ammo_sound: Option<String>,
    pub insert_ammo_stop_sound: Option<String>,
    pub eject_ammo_start_sound: Option<String>,
    pub eject_ammo_sound: Option<String>,
    pub eject_ammo_stop_sound: Option<String>,
    pub rack_sound: Option<String>,
    pub click_sound: Option<String>,
    pub equip_sound: Option<String>,
    pub unequip_sound: Option<String>,
    pub bring_to_bear_sound: Option<String>,
    pub aim_release_sound: Option<String>,
    pub magazine_type: Option<String>,
    pub weapon_reload_type: Option<String>,
    pub rack_after_shoot: bool,
    pub jam_gun_chance: f32,
    /// The original's list can hold the same part twice (an updated part is
    /// added again), so entries are shared.
    pub model_weapon_part: Option<Vec<Rc<RefCell<ModelWeaponPart>>>>,
    pub have_chamber: bool,
    pub need_to_be_closed_once_reload: bool,
    pub manually_remove_spent_rounds: bool,
    pub bite_defense: f32,
    pub scratch_defense: f32,
    pub corpse_sickness_defense: f32,
    pub bullet_defense: f32,
    pub damage_category: Option<String>,
    pub damage_make_hole: bool,
    pub neck_protection_modifier: f32,
    pub attachment_replacement: Option<String>,
    pub insert_all_bullets_reload: bool,
    pub chance_to_fall: i32,
    pub fabric_type: Option<String>,
    pub equipped_no_sprint: bool,
    pub world_static_model: Option<String>,
    pub crit_dmg_multiplier: f32,
    pub is_dung: bool,
    pub insulation: f32,
    pub windresist: f32,
    pub waterresist: f32,
    pub fire_mode: Option<String>,
    pub fire_mode_possibilities: Option<Vec<String>>,
    pub cyclic_rate_multiplier: f32,
    pub remove_unhappiness_when_cooked: bool,
    pub stop_power: f32,
    pub recorded_media_cat: Option<String>,
    pub accept_media_type: i8,
    pub no_transmit: bool,
    pub world_render: bool,
    pub lua_create: Option<String>,
    pub sound_parameter_map: Option<JHashMap<String>>,
    pub milk_replace_item: Option<String>,
    pub max_milk: i32,
    pub animal_feed_type: Option<String>,
    pub evolved_recipe: Vec<String>,
    pub item_config_key: Option<String>,
    /// The `ItemConfig` script named by `item_config_key`.
    pub item_config: Option<ScriptRef>,
    pub icon_color_mask: Option<String>,
    pub icon_fluid_mask: Option<String>,
    pub with_drainable: Option<String>,
    pub without_drainable: Option<String>,
    pub static_models_by_index: Option<Vec<String>>,
    pub world_static_models_by_index: Option<Vec<String>>,
    pub weapon_sprites_by_index: Option<Vec<String>>,
    pub spawn_with: Option<String>,
    pub vision_modifier: f32,
    pub hearing_modifier: f32,
    pub strain_modifier: f32,
    pub on_break: Option<String>,
    pub damaged_sound: Option<String>,
    pub bullet_hit_armour_sound: Option<String>,
    pub weapon_hit_armour_sound: Option<String>,
    pub shout_type: Option<String>,
    pub shout_multiplier: f32,
    pub eat_time: i32,
    pub visual_aid: bool,
    pub discomfort_modifier: f32,
    pub fire_fuel_ratio: f32,
    pub item_after_cleaning: Option<String>,
    pub is_craft_recipe_product: bool,
    pub is_craft_recipe_product_checked_extra: bool,
    pub can_be_foraged: bool,
    pub can_be_foraged_checked_extra: bool,
    pub forage_focus_categories: HashSet<String>,
    pub can_spawn_as_loot: bool,
    pub can_spawn_as_loot_checked_extra: bool,
    pub researchable_recipes: Vec<String>,
    pub is_researchable_recipes_checked_extra: bool,
    pub used_in_recipes: Vec<String>,
    pub file_name: Option<String>,
    pub is_used_in_build_recipes: bool,
    pub opening_recipe: Option<String>,
    pub double_click_recipe: Option<String>,
    /// `ModelKey(id)`.
    pub muzzle_flash_model_key: Option<String>,
    pub book_subjects: Vec<Option<Rc<RegistryKey>>>,
    pub magazine_subjects: Vec<Option<Rc<RegistryKey>>>,
    pub hit_sound: Option<String>,
    pub hit_floor_sound: Option<String>,
    pub body_location: Option<Rc<RegistryKey>>,
    pub palette_choices: Vec<String>,
    pub sprite_name: Option<String>,
    pub palettes_start: Option<String>,
    pub item_type: Option<Rc<RegistryKey>>,
    pub spice: bool,
    pub use_for_poison: i32,
    pub item_recipe_map: JHashMap<ItemRecipe>,
}

impl Default for Item {
    fn default() -> Self {
        Self::new()
    }
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

/// `val.split(";")` as owned strings, untrimmed.
fn split_owned(v: &str, sep: char) -> Vec<String> {
    java::split_char(v, sep)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// `val.split(";")` with each part trimmed.
fn split_trimmed(v: &str, sep: char) -> Vec<String> {
    java::split_char(v, sep)
        .into_iter()
        .map(|p| java::trim(p).to_string())
        .collect()
}

fn eq_true(v: &str) -> bool {
    java::eq_ignore_case(v, "true")
}

impl Item {
    pub fn normal_texture_slot(&self) -> Option<&pz_render::TextureRef> {
        self.normal_texture.as_ref()
    }
    pub fn world_texture_slot(&self) -> Option<&pz_render::TextureRef> {
        self.world_texture.as_ref()
    }
    pub fn special_texture_slots(&self) -> &[Option<pz_render::TextureRef>] {
        &self.special_textures
    }
    pub fn texture_capture_complete(&self) -> bool {
        self.texture_capture_complete
    }
    /// Collector-only physical detach, with no native reload/reset behavior.
    pub fn take_texture_slots_for_gc(
        &mut self,
    ) -> (
        Option<pz_render::TextureRef>,
        Option<pz_render::TextureRef>,
        Vec<Option<pz_render::TextureRef>>,
    ) {
        (
            self.normal_texture.take(),
            self.world_texture.take(),
            std::mem::take(&mut self.special_textures),
        )
    }

    fn replace_normal_texture(&mut self, texture: Option<pz_render::TextureRef>) {
        if let Some(old) = std::mem::replace(&mut self.normal_texture, texture) {
            crate::texture_capture::retire_texture(old);
        }
    }
    fn replace_world_texture(&mut self, texture: Option<pz_render::TextureRef>) {
        if let Some(old) = std::mem::replace(&mut self.world_texture, texture) {
            crate::texture_capture::retire_texture(old);
        }
    }
    pub fn new() -> Self {
        let mut entity = GameEntityScript::new();
        entity.base = BaseScriptObject::new(ScriptType::Item);
        Item {
            entity,
            clothing_extra_submenu: None,
            display_name: None,
            hidden: false,
            cant_eat: false,
            icon: s("None"),
            normal_texture: None,
            world_texture: None,
            special_textures: Vec::new(),
            texture_capture_complete: false,
            medical: false,
            canned_food: false,
            survival_gear: false,
            mechanics_item: false,
            use_world_item: false,
            scale_world_icon: 1.0,
            close_kill_move: None,
            weapon_length: 0.4,
            actual_weight: 1.0,
            weight_wet: 0.0,
            weight_empty: 0.0,
            hunger_change: 0.0,
            critical_chance: 20.0,
            count: 1,
            days_fresh: Self::MAXIMUM_FOOD_AGE,
            days_totally_rotten: Self::MAXIMUM_FOOD_AGE,
            minutes_to_cook: 60,
            minutes_to_burn: 120,
            is_cookable: false,
            cooking_sound: None,
            stress_change: 0.0,
            boredom_change: 0.0,
            unhappy_change: 0.0,
            always_welcome_gift: false,
            replace_on_deplete: None,
            replace_on_extinguish: None,
            ranged: false,
            can_store_water: false,
            max_range: 1.0,
            min_range: 0.0,
            thirst_change: 0.0,
            fatigue_change: 0.0,
            min_angle: 1.0,
            requires_equipped_both_hands: false,
            max_damage: 1.5,
            min_damage: 0.0,
            minimum_swing_time: 0.0,
            swing_sound: s("BaseballBatSwing"),
            weapon_sprite: None,
            angle_falloff: false,
            sound_volume: 0,
            to_hit_modifier: 1.0,
            sound_radius: 0,
            other_character_volume_boost: 0.0,
            weapon_categories: KeySet::default(),
            item_tags: KeySet::default(),
            impact_sound: s("BaseballBatHit"),
            swing_time: 1.0,
            knock_back_on_no_death: true,
            splat_blood_on_no_death: false,
            swing_amount_before_impact: 0.0,
            ammo_type: None,
            max_ammo: 0,
            gun_type: None,
            door_damage: 1,
            condition_lower_chance: 10,
            head_condition_lower_chance_multiplier: 1.0,
            condition_max: 10,
            can_bandage: false,
            name: String::new(),
            module_dot_type: String::new(),
            max_hit_count: 1000,
            use_self: false,
            other_hand_use: false,
            other_hand_require: None,
            physics_object: None,
            swing_anim: s("Rifle"),
            weapon_weight: 1.0,
            endurance_change: 0.0,
            idle_anim: s("Idle"),
            run_anim: s("Run"),
            attachment_type: None,
            make_up_type: None,
            consolidate_option: None,
            require_in_hand_or_inventory: None,
            door_hit_sound: s("BaseballBatHit"),
            replace_on_use: None,
            dangerous_uncooked: false,
            alcoholic: false,
            push_back_mod: 1.0,
            splat_number: 2,
            npc_sound_boost: 1.0,
            range_falloff: false,
            use_endurance: true,
            multiple_hit_condition_affected: true,
            share_endurance: false,
            can_barricade: false,
            use_while_equipped: true,
            use_while_unequipped: false,
            ticks_per_equip_use: 30,
            disappear_on_use: true,
            use_delta: 0.03125,
            always_knockdown: false,
            endurance_mod: 1.0,
            knockdown_mod: 1.0,
            cant_attack_with_lowest_endurance: false,
            replace_on_use_on: None,
            replace_types: None,
            replace_types_map: None,
            attachments_provided: None,
            food_type: None,
            poison: false,
            poison_detection_level: 0,
            poison_power: 0,
            default_mod_data: None,
            is_aimed_firearm: false,
            is_aimed_hand_weapon: false,
            can_stack: true,
            aiming_mod: 1.0,
            projectile_count: 1,
            projectile_spread: 0.0,
            projectile_weight_center: 1.0,
            hit_angle_mod: 0.0,
            splat_size: 1.0,
            temperature: 0.0,
            number_of_pages: -1,
            lvl_skill_trained: -1,
            num_levels_trained: 1,
            skill_trained: s(""),
            capacity: 0,
            max_item_size: 0.0,
            weight_reduction: 0,
            sub_category: s(""),
            activated_item: false,
            light_strength: 0.0,
            torch_cone: false,
            light_distance: 0,
            can_be_equipped: None,
            two_hand_weapon: false,
            custom_context_menu: None,
            tooltip: None,
            replace_on_cooked: None,
            display_category: None,
            trap: false,
            obsolete: false,
            fishing_lure: false,
            can_be_write: false,
            aiming_perk_crit_modifier: 0,
            aiming_perk_range_modifier: 0.0,
            aiming_perk_hit_chance_modifier: 0.0,
            hit_chance: 0,
            aiming_perk_min_angle_modifier: 0.0,
            recoil_delay: 0,
            piercing_bullets: false,
            sound_gain: 1.0,
            protect_from_rain_when_equipped: false,
            max_range_modifier: 0.0,
            min_sight_range: 0.0,
            max_sight_range: 0.0,
            low_light_bonus: 0.0,
            min_range_ranged_modifier: 0.0,
            damage_modifier: 0.0,
            recoil_delay_modifier: 0.0,
            clip_size_modifier: 0,
            mount_on: None,
            part_type: None,
            can_attach_callback: None,
            can_detach_callback: None,
            on_attach_callback: None,
            on_detach_callback: None,
            clip_size: 0,
            reload_time: 0,
            reload_time_modifier: 0,
            aiming_time: 0,
            aiming_time_modifier: 0,
            hit_chance_modifier: 0,
            angle_modifier: 0.0,
            projectile_spread_modifier: 0.0,
            weight_modifier: 0.0,
            page_to_write: 0,
            remove_negative_effect_on_cooked: false,
            tree_damage: 0,
            alcohol_power: 0.0,
            put_in_sound: None,
            place_one_sound: None,
            place_multiple_sound: None,
            open_sound: None,
            close_sound: None,
            break_sound: None,
            custom_eat_sound: None,
            fill_from_dispenser_sound: None,
            fill_from_lake_sound: None,
            fill_from_tap_sound: None,
            fill_from_toilet_sound: None,
            bullet_out_sound: None,
            shell_fall_sound: None,
            drop_sound: None,
            sound_map: None,
            bandage_power: 0.0,
            reduce_infection_power: 0.0,
            on_cooked: None,
            only_accept_category: None,
            accept_item_function: None,
            padlock: false,
            digital_padlock: false,
            learned_recipes: None,
            trigger_explosion_timer: 0,
            can_be_placed: false,
            explosion_range: 0,
            explosion_power: 0,
            fire_range: 0,
            fire_starting_chance: 0,
            fire_starting_energy: 0,
            smoke_range: 0,
            noise_range: 0,
            noise_duration: 0,
            extra_damage: 0.0,
            explosion_timer: 0,
            explosion_duration: 0,
            placed_sprite: None,
            can_be_reused: false,
            sensor_range: 0,
            can_be_remote: false,
            remote_controller: false,
            remote_range: 0,
            count_down_sound: None,
            explosion_sound: None,
            flu_reduction: 0,
            food_sickness_change: 0,
            inverse_cough_probability: 0,
            inverse_cough_probability_smoker: 0,
            pain_reduction: 0,
            torch_dot: 0.96,
            color_red: 255,
            color_green: 255,
            color_blue: 255,
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
            herbalist_type: None,
            carbohydrates: 0.0,
            lipids: 0.0,
            proteins: 0.0,
            calories: 0.0,
            packaged: false,
            cant_be_frozen: false,
            evolved_recipe_name: None,
            replace_on_rotten: None,
            metal_value: 0.0,
            alarm_sound: None,
            item_when_dry: None,
            wet_cooldown: 0.0,
            is_wet: false,
            on_eat: None,
            cant_be_consolided: false,
            bad_in_microwave: false,
            good_hot: false,
            bad_cold: false,
            map: None,
            vehicle_type: 0,
            vehicle_part_models: None,
            max_capacity: -1,
            item_capacity: -1,
            condition_affects_capacity: false,
            brake_force: 0.0,
            durability: 0.0,
            chance_to_spawn_damaged: 0,
            condition_lower_normal: 0.0,
            condition_lower_offroad: 0.0,
            wheel_friction: 0.0,
            suspension_damping: 0.0,
            suspension_compression: 0.0,
            engine_loudness: 0.0,
            clothing_item: None,
            static_model: None,
            primary_anim_mask: None,
            secondary_anim_mask: None,
            primary_anim_mask_attachment: None,
            secondary_anim_mask_attachment: None,
            replace_in_second_hand: None,
            replace_in_primary_hand: None,
            replace_when_unequip: None,
            replace_primary_hand: None,
            replace_second_hand: None,
            world_object_sprite: None,
            item_name: None,
            special_world_texture_names: Vec::new(),
            world_texture_name: None,
            eat_type: None,
            pour_type: None,
            read_type: None,
            dig_type: None,
            icons_for_texture: None,
            base_speed: 1.0,
            blood_clothing_type: None,
            stomp_power: 1.0,
            run_speed_modifier: 1.0,
            combat_speed_modifier: 1.0,
            clothing_item_extra: None,
            clothing_item_extra_option: None,
            remove_on_broken: true,
            can_have_holes: true,
            cosmetic: false,
            ammo_box: None,
            insert_ammo_start_sound: None,
            insert_ammo_sound: None,
            insert_ammo_stop_sound: None,
            eject_ammo_start_sound: None,
            eject_ammo_sound: None,
            eject_ammo_stop_sound: None,
            rack_sound: None,
            click_sound: s("Stormy9mmClick"),
            equip_sound: None,
            unequip_sound: None,
            bring_to_bear_sound: None,
            aim_release_sound: None,
            magazine_type: None,
            weapon_reload_type: None,
            rack_after_shoot: false,
            jam_gun_chance: 1.0,
            model_weapon_part: None,
            have_chamber: true,
            need_to_be_closed_once_reload: false,
            manually_remove_spent_rounds: false,
            bite_defense: 0.0,
            scratch_defense: 0.0,
            corpse_sickness_defense: 0.0,
            bullet_defense: 0.0,
            damage_category: None,
            damage_make_hole: false,
            neck_protection_modifier: 1.0,
            attachment_replacement: None,
            insert_all_bullets_reload: false,
            chance_to_fall: 0,
            fabric_type: None,
            equipped_no_sprint: false,
            world_static_model: None,
            crit_dmg_multiplier: 0.0,
            is_dung: false,
            insulation: 0.0,
            windresist: 0.0,
            waterresist: 0.0,
            fire_mode: None,
            fire_mode_possibilities: None,
            cyclic_rate_multiplier: 0.0,
            remove_unhappiness_when_cooked: false,
            stop_power: 5.0,
            recorded_media_cat: None,
            accept_media_type: -1,
            no_transmit: false,
            world_render: true,
            lua_create: None,
            sound_parameter_map: None,
            milk_replace_item: None,
            max_milk: 0,
            animal_feed_type: None,
            evolved_recipe: Vec::new(),
            item_config_key: None,
            item_config: None,
            icon_color_mask: None,
            icon_fluid_mask: None,
            with_drainable: None,
            without_drainable: None,
            static_models_by_index: None,
            world_static_models_by_index: None,
            weapon_sprites_by_index: None,
            spawn_with: None,
            vision_modifier: 1.0,
            hearing_modifier: 1.0,
            strain_modifier: 1.0,
            on_break: None,
            damaged_sound: None,
            bullet_hit_armour_sound: None,
            weapon_hit_armour_sound: None,
            shout_type: None,
            shout_multiplier: 1.0,
            eat_time: 0,
            visual_aid: false,
            discomfort_modifier: 0.0,
            fire_fuel_ratio: 0.0,
            item_after_cleaning: None,
            is_craft_recipe_product: false,
            is_craft_recipe_product_checked_extra: false,
            can_be_foraged: false,
            can_be_foraged_checked_extra: false,
            forage_focus_categories: HashSet::new(),
            can_spawn_as_loot: false,
            can_spawn_as_loot_checked_extra: false,
            researchable_recipes: Vec::new(),
            is_researchable_recipes_checked_extra: false,
            used_in_recipes: Vec::new(),
            file_name: None,
            is_used_in_build_recipes: false,
            opening_recipe: None,
            double_click_recipe: None,
            muzzle_flash_model_key: None,
            book_subjects: Vec::new(),
            magazine_subjects: Vec::new(),
            hit_sound: s("BaseballBatHit"),
            hit_floor_sound: s("BatOnFloor"),
            body_location: None,
            palette_choices: Vec::new(),
            sprite_name: None,
            palettes_start: s(""),
            item_type: None,
            spice: false,
            use_for_poison: 0,
            item_recipe_map: JHashMap::new(),
        }
    }

    /// `MAXIMUM_FOOD_AGE`.
    pub const MAXIMUM_FOOD_AGE: i32 = 1_000_000_000;

    fn module(&self) -> Option<Rc<ScriptModule>> {
        self.entity.base.module()
    }

    fn module_name_or_null(&self) -> String {
        self.entity
            .base
            .module_name()
            .unwrap_or_else(|| "null".to_string())
    }

    /// `getName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getModuleName`.
    pub fn module_name(&self) -> Option<String> {
        self.entity.base.module_name()
    }

    /// `getFullName`: `moduleDotType`.
    pub fn full_name(&self) -> String {
        self.module_dot_type.clone()
    }

    /// `getDisplayName`: the full name when no display name is set.
    pub fn display_name(&self) -> String {
        match self.display_name.as_deref() {
            Some(d) if !d.is_empty() => d.to_string(),
            _ => self.full_name(),
        }
    }

    /// `getActualWeight`.
    pub fn actual_weight(&self) -> f32 {
        self.actual_weight.max(0.0)
    }

    /// `setActualWeight`.
    pub fn set_actual_weight(&mut self, w: f32) {
        self.actual_weight = if w < 0.0 { 0.0 } else { w };
    }

    /// `getTicksPerEquipUse` (returned as a float).
    pub fn ticks_per_equip_use(&self) -> f32 {
        self.ticks_per_equip_use as f32
    }

    /// `isKeepOnDeplete`.
    pub fn is_keep_on_deplete(&self) -> bool {
        !self.disappear_on_use
    }

    /// `setKeepOnDeplete`.
    pub fn set_keep_on_deplete(&mut self, keep: bool) {
        self.disappear_on_use = !keep;
    }

    /// `getStaticModelException`: items tagged `UseWorldStaticModel` use their
    /// world model.
    pub fn static_model_exception(&self, sm: &ScriptManager) -> Option<&str> {
        if self.has_tag_constant(sm, "USE_WORLD_STATIC_MODEL") {
            return self.world_static_model.as_deref();
        }
        self.static_model.as_deref()
    }

    /// `getSoundByID(String)`.
    pub fn sound_by_id(&self, id: &str) -> Option<&str> {
        self.sound_map.as_ref()?.get(id).map(String::as_str)
    }

    /// `getReplaceType`.
    pub fn replace_type(&self, key: &str) -> Option<&str> {
        self.replace_types_map.as_ref()?.get(key)?.as_deref()
    }

    /// `hasReplaceType`.
    pub fn has_replace_type(&self, key: &str) -> bool {
        self.replace_type(key).is_some()
    }

    /// `getSoundParameter`.
    pub fn sound_parameter(&self, parameter_name: &str) -> Option<&str> {
        self.sound_parameter_map
            .as_ref()?
            .get(parameter_name)
            .map(String::as_str)
    }

    /// `getMaxLevelTrained`.
    pub fn max_level_trained(&self) -> i32 {
        if self.lvl_skill_trained == -1 {
            return -1;
        }
        self.lvl_skill_trained + self.num_levels_trained
    }

    /// `getR` / `getColorRed`.
    pub fn r(&self) -> f32 {
        self.color_red as f32 / 255.0
    }

    /// `getG` / `getColorGreen`.
    pub fn g(&self) -> f32 {
        self.color_green as f32 / 255.0
    }

    /// `getB` / `getColorBlue`.
    pub fn b(&self) -> f32 {
        self.color_blue as f32 / 255.0
    }

    /// `containsWeaponCategory`.
    pub fn contains_weapon_category(&self, category: Option<&Rc<RegistryKey>>) -> bool {
        self.weapon_categories.contains(category)
    }

    /// `hasTag(ItemTag)`.
    pub fn has_tag(&self, tag: Option<&Rc<RegistryKey>>) -> bool {
        self.item_tags.contains(tag)
    }

    /// `hasTag(ItemTag...)`.
    pub fn has_any_tag(&self, tags: &[Option<&Rc<RegistryKey>>]) -> bool {
        tags.iter().any(|t| self.has_tag(*t))
    }

    /// `hasTag(ItemTag.<field>)` for a built-in tag constant.
    pub fn has_tag_constant(&self, sm: &ScriptManager, field: &str) -> bool {
        self.has_tag(ItemTag::constant(sm, field).as_ref())
    }

    /// `isItemType(ItemType)`.
    pub fn is_item_type_key(&self, item_type: Option<&Rc<RegistryKey>>) -> bool {
        match (&self.item_type, item_type) {
            (Some(a), Some(b)) => a.location == b.location,
            (None, None) => true,
            _ => false,
        }
    }

    /// `isItemType(ItemType.<field>)` for a built-in type constant.
    pub fn is_item_type(&self, sm: &ScriptManager, field: &str) -> bool {
        keys::is_constant(sm, self.item_type.as_ref(), ItemType::REGISTRY, field)
    }

    /// `isBodyLocation`.
    pub fn is_body_location(&self, location: Option<&Rc<RegistryKey>>) -> bool {
        match (&self.body_location, location) {
            (Some(a), Some(b)) => a.location == b.location,
            (None, None) => true,
            _ => false,
        }
    }

    /// `isMementoLoot`.
    pub fn is_memento_loot(&self, sm: &ScriptManager) -> bool {
        self.has_tag_constant(sm, "IS_MEMENTO")
            || self.display_category.as_deref() == Some("Memento")
    }

    fn category_is(&self, names: &[&str]) -> bool {
        self.display_category
            .as_deref()
            .is_some_and(|c| names.contains(&c))
    }

    /// `isCookwareLoot`.
    pub fn is_cookware_loot(&self) -> bool {
        self.category_is(&["CookingWeapon", "Cooking"])
    }

    /// `isMaterialLoot`.
    pub fn is_material_loot(&self) -> bool {
        self.category_is(&["MaterialWeapon", "Material"])
    }

    /// `isFarmingLoot`.
    pub fn is_farming_loot(&self, sm: &ScriptManager) -> bool {
        self.category_is(&["GardeningWeapon", "Gardening"])
            || self.has_tag_constant(sm, "FARMING_LOOT")
    }

    /// `isToolLoot`.
    pub fn is_tool_loot(&self) -> bool {
        self.category_is(&["ToolWeapon", "Tool"])
    }

    /// `isSurvivalGearLoot`.
    pub fn is_survival_gear_loot(&self) -> bool {
        self.survival_gear
            || self.category_is(&[
                "FishingWeapon",
                "Fishing",
                "Trapping",
                "Camping",
                "FireSource",
            ])
    }

    /// `isMedicalLoot`.
    pub fn is_medical_loot(&self) -> bool {
        self.medical || self.category_is(&["FirstAid", "FirstAidWeapon"])
    }

    /// `isMechanicsLoot`.
    pub fn is_mechanics_loot(&self) -> bool {
        self.mechanics_item || self.category_is(&["VehicleMaintenance", "VehicleMaintenanceWeapon"])
    }

    /// `ignoreZombieDensity`.
    pub fn ignore_zombie_density(&self, sm: &ScriptManager) -> bool {
        self.is_item_type(sm, "FOOD")
            || self.has_tag_constant(sm, "IGNORE_ZOMBIE_DENSITY")
            || self.is_memento_loot(sm)
    }

    /// `isSpice`: every food counts, a drainable only when flagged.
    pub fn is_spice(&self, sm: &ScriptManager) -> bool {
        self.is_item_type(sm, "FOOD") || (self.is_item_type(sm, "DRAINABLE") && self.spice)
    }

    /// `getDiscomfortModifier`, given `SandboxOptions.discomfortFactor`.
    pub fn discomfort_modifier(&self, discomfort_factor: f64) -> f32 {
        if discomfort_factor <= 0.0 {
            return 0.0;
        }
        (self.discomfort_modifier as f64 * discomfort_factor) as f32
    }

    /// `hasResearchableRecipes`.
    pub fn has_researchable_recipes(&self) -> bool {
        !self.researchable_recipes.is_empty()
    }

    /// `addResearchableRecipe(String)`: a name that is a craft recipe is
    /// added only if that recipe can be researched.
    pub fn add_researchable_recipe(&mut self, sm: &ScriptManager, recipe_name: &str) {
        if self.researchable_recipes.iter().any(|r| r == recipe_name) {
            return;
        }
        // TODO(entity): `getCraftRecipe` also finds the craft recipe
        // component of an entity with this name.
        if let Some(recipe) = sm.get::<CraftRecipe>(ScriptType::CraftRecipe, recipe_name) {
            self.add_researchable_craft_recipe(&mut recipe.borrow_mut());
            return;
        }
        self.researchable_recipes.push(recipe_name.to_string());
    }

    /// `addResearchableRecipe(CraftRecipe)`.
    pub fn add_researchable_craft_recipe(&mut self, recipe: &mut CraftRecipe) {
        let name = recipe.name().unwrap_or("null").to_string();
        if self.researchable_recipes.contains(&name) || !recipe.can_be_researched() {
            return;
        }
        self.researchable_recipes.push(name);
    }

    /// `addResearchableRecipes`.
    pub fn add_researchable_recipes(&mut self, sm: &ScriptManager, names: &[String]) {
        for n in names {
            self.add_researchable_recipe(sm, n);
        }
    }

    /// `getUsedInRecipes`.
    pub fn used_in_recipes(&self) -> &[String] {
        &self.used_in_recipes
    }

    /// `isUsedInRecipes`.
    pub fn is_used_in_recipes(&self) -> bool {
        !self.used_in_recipes.is_empty()
    }

    /// `getObsolete`.
    pub fn obsolete(&self) -> bool {
        self.obsolete
    }

    /// `toString`.
    pub fn to_java_string(&self) -> String {
        format!(
            "Item{{Module: {}, Name:{}, ItemType:{}}}",
            self.module_name_or_null(),
            self.name,
            self.item_type
                .as_ref()
                .map_or("null".to_string(), |t| t.location.id())
        )
    }

    /// `DoParam(String)`: one `key = value` line (also how `Load` reads
    /// each value). Only the text up to a second `=` is the value.
    pub fn do_param_str(&mut self, sm: &ScriptManager, s: &str) -> ScriptResult {
        if java::trim(s).is_empty() {
            return Ok(());
        }
        self.do_param_line(sm, s)
            .map_err(|e| invalid_parameter(exception_message(&e)))
    }

    fn do_param_line(&mut self, sm: &ScriptManager, s: &str) -> ScriptResult {
        let p = java::split_char(s, '=');
        let param = java::trim(at(&p, 0)?);
        let val = java::trim(at(&p, 1)?);
        self.do_param(sm, param, val)
    }

    /// `DoParam(String, String)`: any failure is reported as an invalid
    /// parameter of this item.
    pub fn do_param(&mut self, sm: &ScriptManager, param: &str, val: &str) -> ScriptResult {
        self.do_param_inner(sm, param, val).map_err(|_| {
            invalid_parameter(Some(&format!(
                "Error: {} is not a valid parameter in item: {}",
                java::trim(param),
                self.name
            )))
        })
    }

    fn do_param_inner(&mut self, sm: &ScriptManager, param: &str, val: &str) -> ScriptResult {
        let p = java::trim(param);
        let is = |k: &str| java::eq_ignore_case(p, k);
        let is_raw = |k: &str| java::eq_ignore_case(param, k);
        let vt = java::trim(val);
        if let Some(attribute) = attribute_type_from_name(p) {
            self.entity.load_attribute(sm, p, vt)?;
            if attribute == "HeadCondition" {
                self.entity.load_attribute(sm, "TimesHeadRepaired", "0")?;
            }
        } else if is("BodyLocation") {
            self.body_location = ItemBodyLocation::get_id(sm, vt)?;
        } else if is("Palettes") {
            for part in java::split_char(val, '/') {
                self.palette_choices.push(java::trim(part).to_string());
            }
        } else if is("HitSound") {
            self.hit_sound = (vt != "null").then(|| vt.to_string());
        } else if is("HitFloorSound") {
            self.hit_floor_sound = s(vt);
        } else if is("PalettesStart") {
            self.palettes_start = s(vt);
        } else if is("DisplayName") {
            self.display_name = s(vt);
        } else if is("MetalValue") {
            self.metal_value = parse_float(vt)?;
        } else if is("SpriteName") {
            self.sprite_name = s(vt);
        } else if is("ItemType") {
            self.item_type = ItemType::get_id(sm, vt)?;
        } else if is("SplatSize") {
            self.splat_size = parse_float(val)?;
        } else if is("CanStoreWater") {
            self.can_store_water = eq_true(val);
        } else if is("Poison") {
            self.poison = eq_true(val);
        } else if is("FoodType") {
            self.food_type = s(vt);
        } else if is("PoisonDetectionLevel") {
            self.poison_detection_level = parse_int(val)?;
        } else if is("PoisonPower") {
            self.poison_power = parse_int(val)?;
        } else if is("UseForPoison") {
            self.use_for_poison = parse_int(val)?;
        } else if is("SwingAnim") {
            self.swing_anim = s(val);
        } else if is("Icon") {
            self.do_param_icon(sm, val)?;
        } else if is("UseWorldItem") {
            self.use_world_item = eq_true(val);
        } else if is("Medical") {
            self.medical = eq_true(val);
        } else if is("CannedFood") {
            self.canned_food = eq_true(val);
        } else if is("MechanicsItem") {
            self.mechanics_item = eq_true(val);
        } else if is("SurvivalGear") {
            self.survival_gear = eq_true(val);
        } else if is("ScaleWorldIcon") {
            self.scale_world_icon = parse_float(val)?;
        } else if is("DoorHitSound") {
            self.door_hit_sound = s(val);
        } else if is("Weight") {
            self.actual_weight = parse_float(val)?;
            if self.actual_weight < 0.0 {
                self.actual_weight = 0.0;
            }
        } else if is("WeightWet") {
            self.weight_wet = parse_float(val)?;
        } else if is("WeightEmpty") {
            self.weight_empty = parse_float(val)?;
        } else if is("HungerChange") {
            self.hunger_change = parse_float(val)?;
        } else if is("ThirstChange") {
            self.thirst_change = parse_float(val)?;
        } else if is("FatigueChange") {
            self.fatigue_change = parse_float(val)?;
        } else if is("EnduranceChange") {
            self.endurance_change = parse_float(val)?;
        } else if is("Hidden") {
            self.hidden = eq_true(val);
        } else if is("CriticalChance") {
            self.critical_chance = parse_float(val)?;
        } else if is("critDmgMultiplier") {
            self.crit_dmg_multiplier = parse_float(val)?;
        } else if is("DaysFresh") {
            self.days_fresh = parse_int(val)?;
        } else if is("MilkReplaceItem") {
            self.milk_replace_item = s(val);
        } else if is("MaxMilk") {
            self.max_milk = parse_int(val)?;
        } else if is("AnimalFeedType") {
            self.animal_feed_type = s(val);
        } else if is("DaysTotallyRotten") {
            self.days_totally_rotten = parse_int(val)?;
        } else if is("IsCookable") {
            self.is_cookable = eq_true(val);
        } else if is("CookingSound") {
            self.cooking_sound = s(val);
        } else if is("MinutesToCook") {
            self.minutes_to_cook = parse_int(val)?;
        } else if is("MinutesToBurn") {
            self.minutes_to_burn = parse_int(val)?;
        } else if is("BoredomChange") {
            self.boredom_change = parse_int(val)? as f32;
        } else if is("StressChange") {
            self.stress_change = parse_int(val)? as f32;
        } else if is("UnhappyChange") {
            self.unhappy_change = parse_int(val)? as f32;
        } else if is("RemoveUnhappinessWhenCooked") {
            self.remove_unhappiness_when_cooked = eq_true(val);
        } else if is("ReplaceOnDeplete") {
            self.replace_on_deplete = s(val);
        } else if is("ReplaceOnExtinguish") {
            self.replace_on_extinguish = s(val);
        } else if is("ReplaceOnUseOn") {
            self.replace_on_use_on = s(val);
            if val.contains('-') {
                let ss = java::split_char(val, '-');
                let key = java::trim(at(&ss, 0)?);
                let type_ = java::trim(at(&ss, 1)?);
                if !key.is_empty() && !type_.is_empty() {
                    self.replace_types_map
                        .get_or_insert_with(JHashMap::new)
                        .insert(key.to_string(), s(type_));
                }
            }
        } else if is("ReplaceTypes") {
            self.replace_types = s(val);
            for key_and_type in java::split_char(val, ';') {
                let ss2 = java::split_ws(java::trim(key_and_type));
                if ss2.len() != 2 {
                    continue;
                }
                let key = java::trim(ss2[0]);
                let type_ = java::trim(ss2[1]);
                if key.is_empty() || type_.is_empty() {
                    continue;
                }
                self.replace_types_map
                    .get_or_insert_with(JHashMap::new)
                    .insert(key.to_string(), s(type_));
            }
        } else if is("Ranged") {
            self.ranged = eq_true(val);
        } else if is("UseSelf") {
            self.use_self = eq_true(val);
        } else if is("OtherHandUse") {
            self.other_hand_use = eq_true(val);
        } else if is("DangerousUncooked") {
            self.dangerous_uncooked = eq_true(val);
        } else if is("MaxRange") {
            self.max_range = parse_float(val)?;
        } else if is("MinRange") {
            self.min_range = parse_float(val)?;
        } else if is("MinAngle") {
            self.min_angle = parse_float(val)?;
        } else if is("MaxDamage") {
            self.max_damage = parse_float(val)?;
        } else if is("BaseSpeed") {
            self.base_speed = parse_float(val)?;
        } else if is("stompPower") {
            self.stomp_power = parse_float(val)?;
        } else if is("combatSpeedModifier") {
            self.combat_speed_modifier = parse_float(val)?;
        } else if is("runSpeedModifier") {
            self.run_speed_modifier = parse_float(val)?;
        } else if is("clothingItemExtra") {
            self.clothing_item_extra = Some(split_owned(val, ';'));
        } else if is("clothingExtraSubmenu") {
            self.clothing_extra_submenu = s(val);
        } else if is("removeOnBroken") {
            self.remove_on_broken = eq_true(val);
        } else if is("canHaveHoles") {
            self.can_have_holes = eq_true(val);
        } else if is("Cosmetic") {
            self.cosmetic = eq_true(val);
        } else if is("ammoBox") {
            self.ammo_box = s(val);
        } else if is("InsertAmmoStartSound") {
            self.insert_ammo_start_sound = discard_null_or_whitespace(val);
        } else if is("InsertAmmoSound") {
            self.insert_ammo_sound = discard_null_or_whitespace(val);
        } else if is("InsertAmmoStopSound") {
            self.insert_ammo_stop_sound = discard_null_or_whitespace(val);
        } else if is("EjectAmmoStartSound") {
            self.eject_ammo_start_sound = discard_null_or_whitespace(val);
        } else if is("EjectAmmoSound") {
            self.eject_ammo_sound = discard_null_or_whitespace(val);
        } else if is("EjectAmmoStopSound") {
            self.eject_ammo_stop_sound = discard_null_or_whitespace(val);
        } else if is("rackSound") {
            self.rack_sound = s(val);
        } else if is("clickSound") {
            self.click_sound = s(val);
        } else if is_raw("BringToBearSound") {
            self.bring_to_bear_sound = discard_null_or_whitespace(val);
        } else if is_raw("AimReleaseSound") {
            self.aim_release_sound = discard_null_or_whitespace(val);
        } else if is_raw("EquipSound") {
            self.equip_sound = discard_null_or_whitespace(val);
        } else if is_raw("UnequipSound") {
            self.unequip_sound = discard_null_or_whitespace(val);
        } else if is("magazineType") {
            self.magazine_type = s(val);
        } else if is("jamGunChance") {
            self.jam_gun_chance = parse_float(val)?;
        } else if is("modelWeaponPart") {
            self.do_param_model_weapon_part(val);
        } else if is("rackAfterShoot") {
            self.rack_after_shoot = eq_true(val);
        } else if is("haveChamber") {
            self.have_chamber = eq_true(val);
        } else if is("needtobeclosedoncereload") {
            self.need_to_be_closed_once_reload = eq_true(val);
        } else if is("isDung") {
            self.is_dung = eq_true(val);
        } else if is_raw("ManuallyRemoveSpentRounds") {
            self.manually_remove_spent_rounds = eq_true(val);
        } else if is("biteDefense") {
            self.bite_defense = parse_float(val)?.min(100.0);
        } else if is("bulletDefense") {
            self.bullet_defense = parse_float(val)?.min(100.0);
        } else if is("scratchDefense") {
            self.scratch_defense = parse_float(val)?.min(100.0);
        } else if is("neckProtectionModifier") {
            self.neck_protection_modifier = parse_float(val)?;
        } else if is("damageCategory") {
            self.damage_category = s(val);
        } else if is("fireMode") {
            self.fire_mode = s(val);
        } else if is("damageMakeHole") {
            self.damage_make_hole = eq_true(val);
        } else if is("equippedNoSprint") {
            self.equipped_no_sprint = eq_true(val);
        } else if is("corpseSicknessDefense") {
            self.corpse_sickness_defense = parse_float(val)?;
        } else if is("weaponReloadType") {
            self.weapon_reload_type = s(val);
        } else if is("insertAllBulletsReload") {
            self.insert_all_bullets_reload = eq_true(val);
        } else if is("clothingItemExtraOption") {
            self.clothing_item_extra_option = Some(split_owned(val, ';'));
        } else if is("ConditionLowerChanceOneIn") {
            self.condition_lower_chance = parse_int(val)?;
        } else if is("HeadConditionLowerChanceMultiplier") {
            self.head_condition_lower_chance_multiplier = parse_float(val)?;
        } else if is("MultipleHitConditionAffected") {
            self.multiple_hit_condition_affected = eq_true(val);
        } else if is("CanBandage") {
            self.can_bandage = eq_true(val);
        } else if is("ConditionMax") {
            self.condition_max = parse_int(val)?;
        } else if is("SoundGain") {
            self.sound_gain = parse_float(val)?;
        } else if is("MinDamage") {
            self.min_damage = parse_float(val)?;
        } else if is("MinimumSwingTime") {
            self.minimum_swing_time = parse_float(val)?;
        } else if is("SwingSound") {
            self.swing_sound = s(val);
        } else if is("ReplaceOnUse") {
            self.replace_on_use = s(val);
        } else if is("WeaponSprite") {
            self.weapon_sprite = s(val);
        } else if is("weaponSpritesByIndex") {
            self.weapon_sprites_by_index = Some(split_trimmed(val, ';'));
        } else if is("AimingPerkCritModifier") {
            self.aiming_perk_crit_modifier = parse_int(val)?;
        } else if is("AimingPerkRangeModifier") {
            self.aiming_perk_range_modifier = parse_float(val)?;
        } else if is("AimingPerkHitChanceModifier") {
            self.aiming_perk_hit_chance_modifier = parse_float(val)?;
        } else if is("ProjectileSpreadModifier") {
            self.projectile_spread_modifier = parse_float(val)?;
        } else if is("WeightModifier") {
            self.weight_modifier = parse_float(val)?;
        } else if is("AimingPerkMinAngleModifier") {
            self.aiming_perk_min_angle_modifier = parse_float(val)?;
        } else if is("HitChance") {
            self.hit_chance = parse_int(val)?;
        } else if is("RecoilDelay") {
            self.recoil_delay = parse_int(val)?;
        } else if is("StopPower") {
            self.stop_power = parse_float(val)?;
        } else if is("PiercingBullets") {
            self.piercing_bullets = eq_true(val);
        } else if is("AngleFalloff") {
            self.angle_falloff = eq_true(val);
        } else if is("SoundVolume") {
            self.sound_volume = parse_int(val)?;
        } else if is("ToHitModifier") {
            self.to_hit_modifier = parse_float(val)?;
        } else if is("SoundRadius") {
            self.sound_radius = parse_int(val)?;
        } else if is("Categories") {
            for c in java::split_char(val, ';') {
                let key = WeaponCategory::get_id(sm, java::trim(c))?;
                self.weapon_categories.add(key);
            }
        } else if is("Tags") {
            for t in java::split_char(val, ';') {
                let key = ItemTag::get_id(sm, java::trim(t))?;
                self.item_tags.add(key);
            }
        } else if is("OtherCharacterVolumeBoost") {
            self.other_character_volume_boost = parse_float(val)?;
        } else if is("ImpactSound") {
            self.impact_sound = (val != "null").then(|| val.to_string());
        } else if is("SwingTime") {
            self.swing_time = parse_float(val)?;
        } else if is("KnockBackOnNoDeath") {
            self.knock_back_on_no_death = eq_true(val);
        } else if is("Alcoholic") {
            self.alcoholic = eq_true(val);
        } else if is("SplatBloodOnNoDeath") {
            self.splat_blood_on_no_death = eq_true(val);
        } else if is("SwingAmountBeforeImpact") {
            self.swing_amount_before_impact = parse_float(val)?;
        } else if is("AmmoType") {
            self.ammo_type = AmmoType::get_id(sm, val)?;
        } else if is("maxAmmo") {
            self.max_ammo = parse_int(val)?;
        } else if is("GunType") {
            self.gun_type = Some(split_trimmed(val, ';'));
        } else if is("HitAngleMod") {
            self.hit_angle_mod = parse_float(val)?;
        } else if is("OtherHandRequire") {
            self.other_hand_require = ItemTag::get_id(sm, val)?;
        } else if is("AlwaysWelcomeGift") {
            self.always_welcome_gift = eq_true(val);
        } else if is("CantAttackWithLowestEndurance") {
            self.cant_attack_with_lowest_endurance = eq_true(val);
        } else if is("EnduranceMod") {
            self.endurance_mod = parse_float(val)?;
        } else if is("KnockdownMod") {
            self.knockdown_mod = parse_float(val)?;
        } else if is("DoorDamage") {
            self.door_damage = parse_int(val)?;
        } else if is("MaxHitCount") {
            self.max_hit_count = parse_int(val)?;
        } else if is("PhysicsObject") {
            self.physics_object = discard_null_or_whitespace(val);
        } else if is("Count") {
            self.count = parse_int(val)?;
        } else if is("WeaponWeight") {
            self.weapon_weight = parse_float(val)?;
        } else if is("IdleAnim") {
            self.idle_anim = s(val);
        } else if is("RunAnim") {
            self.run_anim = s(val);
        } else if is("RequireInHandOrInventory") {
            self.require_in_hand_or_inventory = Some(
                java::split_char(val, '/')
                    .into_iter()
                    .map(|t| Some(t.to_string()))
                    .collect(),
            );
        } else if is("fireModePossibilities") {
            self.fire_mode_possibilities = Some(split_owned(val, '/'));
        } else if is("CyclicRateMultiplier") {
            self.cyclic_rate_multiplier = parse_float(val)?;
        } else if is("attachmentsProvided") {
            self.attachments_provided = Some(split_owned(val, ';'));
        } else if is("attachmentReplacement") {
            self.attachment_replacement = s(vt);
        } else if is("PushBackMod") {
            self.push_back_mod = parse_float(val)?;
        } else if is("NPCSoundBoost") {
            self.npc_sound_boost = parse_float(val)?;
        } else if is("SplatNumber") {
            self.splat_number = parse_int(val)?;
        } else if is("RangeFalloff") {
            self.range_falloff = eq_true(val);
        } else if is("UseEndurance") {
            self.use_endurance = eq_true(val);
        } else if is("ShareEndurance") {
            self.share_endurance = eq_true(val);
        } else if is("AlwaysKnockdown") {
            self.always_knockdown = eq_true(val);
        } else if is("IsAimedFirearm") {
            self.is_aimed_firearm = eq_true(val);
        } else if is("bulletOutSound") {
            self.bullet_out_sound = s(vt);
        } else if is("ShellFallSound") {
            self.shell_fall_sound = s(vt);
        } else if is_raw("DropSound") {
            self.drop_sound = discard_null_or_whitespace(val);
        } else if is("SoundMap") {
            let ss = java::split_ws(val);
            if ss.len() == 2 && !java::trim(ss[0]).is_empty() {
                self.sound_map
                    .get_or_insert_with(JHashMap::new)
                    .insert(java::trim(ss[0]).to_string(), java::trim(ss[1]).to_string());
            }
        } else if is("IsAimedHandWeapon") {
            self.is_aimed_hand_weapon = eq_true(val);
        } else if is("AimingMod") {
            self.aiming_mod = parse_float(val)?;
        } else if is("ProjectileCount") {
            self.projectile_count = parse_int(val)?;
        } else if is("ProjectileSpread") {
            self.projectile_spread = parse_float(val)?;
        } else if is("ProjectileWeightCenter") {
            self.projectile_weight_center = parse_float(val)?;
        } else if is("MuzzleFlashModelKey") {
            self.muzzle_flash_model_key = s(val);
        } else if is("CanStack") {
            self.can_stack = eq_true(val);
        } else if is("HerbalistType") {
            self.herbalist_type = s(vt);
        } else if is("CanBarricade") {
            self.can_barricade = eq_true(val);
        } else if is("UseWhileEquipped") {
            self.use_while_equipped = eq_true(val);
        } else if is("UseWhileUnequipped") {
            self.use_while_unequipped = eq_true(val);
        } else if is("TicksPerEquipUse") {
            self.ticks_per_equip_use = parse_int(vt)?;
        } else if is("DisappearOnUse") {
            self.disappear_on_use = eq_true(val);
        } else if is("Temperature") {
            self.temperature = parse_float(val)?;
        } else if is("Insulation") {
            self.insulation = parse_float(val)?;
        } else if is("WindResistance") {
            self.windresist = parse_float(val)?;
        } else if is("WaterResistance") {
            self.waterresist = parse_float(val)?;
        } else if is("CloseKillMove") {
            self.close_kill_move = s(vt);
        } else if is("UseDelta") {
            self.use_delta = parse_float(val)?;
        } else if is("TorchDot") {
            self.torch_dot = parse_float(val)?;
        } else if is("NumberOfPages") {
            self.number_of_pages = parse_int(vt)?;
        } else if is("SkillTrained") {
            self.skill_trained = s(vt);
        } else if is("LvlSkillTrained") {
            self.lvl_skill_trained = parse_int(vt)?;
        } else if is("NumLevelsTrained") {
            self.num_levels_trained = parse_int(vt)?;
        } else if is("Capacity") {
            let mut capacity = parse_int(vt)?;
            if capacity as f32 > 50.0 - self.actual_weight {
                capacity = (50.0 - self.actual_weight) as i32;
            }
            self.capacity = capacity;
        } else if is("MaxItemSize") {
            self.max_item_size = parse_float(vt)?;
        } else if is("MaxCapacity") {
            self.max_capacity = parse_int(vt)?;
        } else if is("ItemCapacity") {
            self.item_capacity = parse_int(vt)?;
        } else if is("ConditionAffectsCapacity") {
            self.condition_affects_capacity = eq_true(vt);
        } else if is("BrakeForce") {
            self.brake_force = parse_int(vt)? as f32;
        } else if is("Durability") {
            self.durability = parse_float(vt)?;
        } else if is("ChanceToSpawnDamaged") {
            self.chance_to_spawn_damaged = parse_int(vt)?;
        } else if is("WeaponLength") {
            self.weapon_length = parse_float(vt)?;
        } else if is("ClipSize") {
            self.clip_size = parse_int(vt)?;
        } else if is("ReloadTime") {
            self.reload_time = parse_int(vt)?;
        } else if is("AimingTime") {
            self.aiming_time = parse_int(vt)?;
        } else if is("AimingTimeModifier") {
            self.aiming_time_modifier = parse_int(vt)?;
        } else if is("ReloadTimeModifier") {
            self.reload_time_modifier = parse_int(vt)?;
        } else if is("HitChanceModifier") {
            self.hit_chance_modifier = parse_int(vt)?;
        } else if is("WeightReduction") {
            self.weight_reduction = parse_int(vt)?;
        } else if is("CanBeEquipped") {
            self.can_be_equipped = ItemBodyLocation::get_id(sm, vt)?;
        } else if is("SubCategory") {
            self.sub_category = s(vt);
        } else if is("ActivatedItem") {
            self.activated_item = eq_true(val);
        } else if is("ProtectFromRainWhenEquipped") {
            self.protect_from_rain_when_equipped = eq_true(val);
        } else if is("LightStrength") {
            self.light_strength = parse_float(vt)?;
        } else if is("TorchCone") {
            self.torch_cone = eq_true(val);
        } else if is("LightDistance") {
            self.light_distance = parse_int(vt)?;
        } else if is("TwoHandWeapon") {
            self.two_hand_weapon = eq_true(val);
        } else if is("Tooltip") {
            self.tooltip = s(vt);
        } else if is("DisplayCategory") {
            self.display_category = s(vt);
        } else if is("BadInMicrowave") {
            self.bad_in_microwave = eq_true(val);
        } else if is("GoodHot") {
            self.good_hot = eq_true(val);
        } else if is("BadCold") {
            self.bad_cold = eq_true(val);
        } else if is("AlarmSound") {
            self.alarm_sound = s(vt);
        } else if is("RequiresEquippedBothHands") {
            self.requires_equipped_both_hands = eq_true(val);
        } else if is("ReplaceOnCooked") {
            self.replace_on_cooked = Some(split_owned(vt, ';'));
        } else if is("CustomContextMenu") {
            self.custom_context_menu = s(vt);
        } else if is("Trap") {
            self.trap = eq_true(vt);
        } else if is("Wet") {
            self.is_wet = eq_true(val);
        } else if is("WetCooldown") {
            self.wet_cooldown = parse_float(vt)?;
        } else if is("ItemWhenDry") {
            self.item_when_dry = s(vt);
        } else if is("FishingLure") {
            self.fishing_lure = eq_true(vt);
        } else if is("CanBeWrite") {
            self.can_be_write = eq_true(vt);
        } else if is("PageToWrite") {
            self.page_to_write = parse_int(vt)?;
        } else if is("Spice") {
            self.spice = eq_true(vt);
        } else if is("RemoveNegativeEffectOnCooked") {
            self.remove_negative_effect_on_cooked = eq_true(vt);
        } else if is("ClipSizeModifier") {
            self.clip_size_modifier = parse_int(val)?;
        } else if is("RecoilDelayModifier") {
            self.recoil_delay_modifier = parse_float(val)?;
        } else if is("MaxRangeModifier") {
            self.max_range_modifier = parse_float(val)?;
        } else if is("MinSightRange") {
            self.min_sight_range = parse_float(val)?;
        } else if is("MaxSightRange") {
            self.max_sight_range = parse_float(val)?;
        } else if is("LowLightBonus") {
            self.low_light_bonus = parse_float(val)?;
        } else if is("DamageModifier") {
            self.damage_modifier = parse_float(val)?;
        } else if is("Map") {
            self.map = s(vt);
        } else if is("PutInSound") {
            self.put_in_sound = discard_null_or_whitespace(vt);
        } else if is("PlaceOneSound") {
            self.place_one_sound = discard_null_or_whitespace(vt);
        } else if is("PlaceMultipleSound") {
            self.place_multiple_sound = discard_null_or_whitespace(vt);
        } else if is("CloseSound") {
            self.close_sound = discard_null_or_whitespace(vt);
        } else if is("OpenSound") {
            self.open_sound = discard_null_or_whitespace(vt);
        } else if is("BreakSound") {
            self.break_sound = s(vt);
        } else if is("TreeDamage") {
            self.tree_damage = parse_int(val)?;
        } else if is("CustomEatSound") {
            self.custom_eat_sound = s(vt);
        } else if is("FillFromDispenserSound") {
            self.fill_from_dispenser_sound = discard_null_or_whitespace(vt);
        } else if is("FillFromLakeSound") {
            self.fill_from_lake_sound = discard_null_or_whitespace(vt);
        } else if is("FillFromTapSound") {
            self.fill_from_tap_sound = discard_null_or_whitespace(vt);
        } else if is("FillFromToiletSound") {
            self.fill_from_toilet_sound = discard_null_or_whitespace(vt);
        } else if is("AlcoholPower") {
            self.alcohol_power = parse_float(vt)?;
        } else if is("BandagePower") {
            self.bandage_power = parse_float(vt)?;
        } else if is("ReduceInfectionPower") {
            self.reduce_infection_power = parse_float(vt)?;
        } else if is("OnCooked") {
            self.on_cooked = s(vt);
        } else if is("OnlyAcceptCategory") {
            self.only_accept_category = discard_null_or_whitespace(val);
        } else if is("AcceptItemFunction") {
            self.accept_item_function = discard_null_or_whitespace(val);
        } else if is("Padlock") {
            self.padlock = eq_true(vt);
        } else if is("DigitalPadlock") {
            self.digital_padlock = eq_true(vt);
        } else if is("triggerExplosionTimer") {
            self.trigger_explosion_timer = parse_int(val)?;
        } else if is("sensorRange") {
            self.sensor_range = parse_int(val)?;
        } else if is("remoteRange") {
            self.remote_range = parse_int(val)?;
        } else if is("CountDownSound") {
            self.count_down_sound = s(vt);
        } else if is("explosionSound") {
            self.explosion_sound = s(vt);
        } else if is("PlacedSprite") {
            self.placed_sprite = s(vt);
        } else if is("explosionTimer") {
            self.explosion_timer = parse_int(val)?;
        } else if is("explosionDuration") {
            self.explosion_duration = parse_int(val)?;
        } else if is("explosionRange") {
            self.explosion_range = parse_int(val)?;
        } else if is("explosionPower") {
            self.explosion_power = parse_int(val)?;
        } else if is("fireRange") {
            self.fire_range = parse_int(val)?;
        } else if is("fireStartingEnergy") {
            self.fire_starting_energy = parse_int(val)?;
        } else if is("fireStartingChance") {
            self.fire_starting_chance = parse_int(val)?;
        } else if is("canBePlaced") {
            self.can_be_placed = eq_true(vt);
        } else if is("CanBeReused") {
            self.can_be_reused = eq_true(vt);
        } else if is("canBeRemote") {
            self.can_be_remote = eq_true(vt);
        } else if is("remoteController") {
            self.remote_controller = eq_true(vt);
        } else if is("smokeRange") {
            self.smoke_range = parse_int(val)?;
        } else if is("noiseRange") {
            self.noise_range = parse_int(val)?;
        } else if is("noiseDuration") {
            self.noise_duration = parse_int(val)?;
        } else if is("extraDamage") {
            self.extra_damage = parse_float(val)?;
        } else if is("TwoWay") {
            self.two_way = eq_true(vt);
        } else if is("TransmitRange") {
            self.transmit_range = parse_int(val)?;
        } else if is("MicRange") {
            self.mic_range = parse_int(val)?;
        } else if is("BaseVolumeRange") {
            self.base_volume_range = parse_float(val)?;
        } else if is("IsPortable") {
            self.is_portable = eq_true(vt);
        } else if is("IsTelevision") {
            self.is_television = eq_true(vt);
        } else if is("MinChannel") {
            self.min_channel = parse_int(val)?;
        } else if is("MaxChannel") {
            self.max_channel = parse_int(val)?;
        } else if is("UsesBattery") {
            self.uses_battery = eq_true(vt);
        } else if is("IsHighTier") {
            self.is_high_tier = eq_true(vt);
        } else if is("WorldObjectSprite") {
            self.world_object_sprite = s(vt);
        } else if is("fluReduction") {
            self.flu_reduction = parse_int(val)?;
        } else if is("FoodSicknessChange") {
            self.food_sickness_change = parse_int(val)?;
        } else if is("InverseCoughProbability") {
            self.inverse_cough_probability = parse_int(val)?;
        } else if is("InverseCoughProbabilitySmoker") {
            self.inverse_cough_probability_smoker = parse_int(val)?;
        } else if is("painReduction") {
            self.pain_reduction = parse_int(val)?;
        } else if is("ColorRed") {
            self.color_red = parse_int(val)?;
        } else if is("ColorGreen") {
            self.color_green = parse_int(val)?;
        } else if is("ColorBlue") {
            self.color_blue = parse_int(val)?;
        } else if is("calories") {
            self.calories = parse_float(val)?;
        } else if is("carbohydrates") {
            self.carbohydrates = parse_float(val)?;
        } else if is("lipids") {
            self.lipids = parse_float(val)?;
        } else if is("proteins") {
            self.proteins = parse_float(val)?;
        } else if is("Packaged") {
            self.packaged = eq_true(vt);
        } else if is("CantBeFrozen") {
            self.cant_be_frozen = eq_true(vt);
        } else if is("EvolvedRecipeName") {
            self.evolved_recipe_name = Some(self.translator_evolved_recipe_name(sm, val));
        } else if is("ReplaceOnRotten") {
            self.replace_on_rotten = s(vt);
        } else if is("CantBeConsolided") {
            self.cant_be_consolided = eq_true(val);
        } else if is("OnEat") {
            self.on_eat = s(vt);
        } else if is("KeepOnDeplete") {
            self.disappear_on_use = !eq_true(val);
        } else if is("VehicleType") {
            self.vehicle_type = parse_int(val)?;
        } else if is("ChanceToFall") {
            self.chance_to_fall = parse_int(val)?;
        } else if is("conditionLowerOffroad") {
            self.condition_lower_offroad = parse_float(val)?;
        } else if is("ConditionLowerStandard") {
            self.condition_lower_normal = parse_float(val)?;
        } else if is("wheelFriction") {
            self.wheel_friction = parse_float(val)?;
        } else if is("suspensionDamping") {
            self.suspension_damping = parse_float(val)?;
        } else if is("suspensionCompression") {
            self.suspension_compression = parse_float(val)?;
        } else if is("engineLoudness") {
            self.engine_loudness = parse_float(val)?;
        } else if is("attachmentType") {
            self.attachment_type = s(vt);
        } else if is("makeUpType") {
            self.make_up_type = s(vt);
        } else if is("consolidateOption") {
            self.consolidate_option = s(vt);
        } else if is("fabricType") {
            self.fabric_type = s(vt);
        } else if is("LearnedRecipes") {
            // TODO(Translator): in debug, `Translator.getRecipeName` for each.
            self.learned_recipes = Some(split_trimmed(val, ';'));
        } else if is("ResearchableRecipes") {
            for name in java::split_char(val, ';') {
                self.add_researchable_recipe(sm, java::trim(name));
            }
        } else if is("MountOn") {
            self.mount_on = Some(split_trimmed(val, ';'));
        } else if is("PartType") {
            self.part_type = s(val);
        } else if is("CanAttach") {
            self.can_attach_callback = s(val);
        } else if is("CanDetach") {
            self.can_detach_callback = s(val);
        } else if is("OnAttach") {
            self.on_attach_callback = s(val);
        } else if is("OnDetach") {
            self.on_detach_callback = s(val);
        } else if is("ClothingItem") {
            self.clothing_item = s(val);
        } else if is("EvolvedRecipe") {
            self.do_param_evolved_recipe(val)?;
        } else if is("StaticModel") {
            self.static_model = s(vt);
        } else if is("worldStaticModel") {
            self.world_static_model = s(vt);
        } else if is("primaryAnimMask") {
            self.primary_anim_mask = s(vt);
        } else if is("secondaryAnimMask") {
            self.secondary_anim_mask = s(vt);
        } else if is("primaryAnimMaskAttachment") {
            self.primary_anim_mask_attachment = s(vt);
        } else if is("secondaryAnimMaskAttachment") {
            self.secondary_anim_mask_attachment = s(vt);
        } else if is("replaceInSecondHand") {
            self.replace_in_second_hand = s(vt);
        } else if is("replaceInPrimaryHand") {
            self.replace_in_primary_hand = s(vt);
        } else if is("replaceWhenUnequip") {
            self.replace_when_unequip = s(vt);
        } else if is("EatType") {
            self.eat_type = s(vt);
        } else if is("PourType") {
            self.pour_type = s(vt);
        } else if is("ReadType") {
            self.read_type = s(vt);
        } else if is("book_subject") {
            for id in java::split_char(vt, ';') {
                self.book_subjects.push(BookSubject::get_id(sm, id)?);
            }
        } else if is("magazine_subject") {
            for id in java::split_char(vt, ';') {
                self.magazine_subjects
                    .push(MagazineSubject::get_id(sm, id)?);
            }
        } else if is("DigType") {
            self.dig_type = s(vt);
        } else if is("IconsForTexture") {
            self.icons_for_texture = Some(split_trimmed(val, ';'));
        } else if is("BloodLocation") {
            self.blood_clothing_type = Some(split_trimmed(val, ';'));
        } else if is("MediaCategory") {
            self.recorded_media_cat = s(vt);
        } else if is("AcceptMediaType") {
            self.accept_media_type = parse_byte(vt)?;
        } else if is("NoTransmit") {
            self.no_transmit = eq_true(vt);
        } else if is("WorldRender") {
            self.world_render = eq_true(vt);
        } else if is("CantEat") {
            self.cant_eat = eq_true(vt);
        } else if is("OBSOLETE") {
            self.obsolete = eq_true(vt);
        } else if is("OnCreate") {
            self.lua_create = s(vt);
        } else if is("OpeningRecipe") {
            self.opening_recipe = s(vt);
        } else if is("DoubleClickRecipe") {
            self.double_click_recipe = s(vt);
        } else if is("ItemConfig") {
            self.item_config_key = s(vt);
        } else if is("IconColorMask") {
            self.icon_color_mask = s(vt);
        } else if is("IconFluidMask") {
            self.icon_fluid_mask = s(vt);
        } else if is("GameEntityScript") {
        } else if is("SoundParameter") {
            let ss = java::split_ws(val);
            let k = java::trim(at(&ss, 0)?).to_string();
            let v = java::trim(at(&ss, 1)?).to_string();
            self.sound_parameter_map
                .get_or_insert_with(JHashMap::new)
                .insert(k, v);
        } else if is("VehiclePartModel") {
            self.do_param_vehicle_part_model(val);
        } else if is("withDrainable") {
            self.with_drainable = s(val);
        } else if is("withoutDrainable") {
            self.without_drainable = s(val);
        } else if is("staticModelsByIndex") {
            self.static_models_by_index = Some(split_trimmed(val, ';'));
        } else if is("worldStaticModelsByIndex") {
            self.world_static_models_by_index = Some(split_trimmed(val, ';'));
        } else if is("spawnWith") {
            self.spawn_with = s(val);
        } else if is("visionModifier") {
            self.vision_modifier = parse_float(val)?;
        } else if is("hearingModifier") {
            self.hearing_modifier = parse_float(val)?;
        } else if is("strainModifier") {
            self.strain_modifier = parse_float(val)?;
        } else if is("OnBreak") {
            self.on_break = s(vt);
        } else if is("DamagedSound") {
            self.damaged_sound = s(vt);
        } else if is("BulletHitArmourSound") {
            self.bullet_hit_armour_sound = discard_null_or_whitespace(vt);
        } else if is("WeaponHitArmourSound") {
            self.weapon_hit_armour_sound = discard_null_or_whitespace(vt);
        } else if is("ShoutType") {
            self.shout_type = s(vt);
        } else if is("ShoutMultiplier") {
            self.shout_multiplier = parse_float(val)?;
        } else if is("EatTime") {
            self.eat_time = parse_int(val)?;
        } else if is("VisualAid") {
            self.visual_aid = eq_true(vt);
        } else if is("DiscomfortModifier") {
            self.discomfort_modifier = parse_float(val)?;
        } else if is("fireFuelRatio") {
            self.fire_fuel_ratio = parse_float(val)?;
        } else if is("ItemAfterCleaning") {
            self.item_after_cleaning = s(vt);
        } else {
            self.add_default_mod_data(sm, p, val);
        }
        Ok(())
    }

    /// The fallback of `DoParam`: an unknown param becomes mod data.
    fn add_default_mod_data(&mut self, sm: &ScriptManager, p: &str, val: &str) {
        let vt = java::trim(val);
        sm.debugln(&format!(
            "adding unknown item param \"{p}\" = \"{vt}\", script: {}, path: {}",
            self.entity.base.full_type().unwrap_or_default(),
            self.entity.file_abs_path().unwrap_or("null")
        ));
        let value = match java::parse_f64(vt) {
            Some(d) => ModDataValue::Number(d),
            None => ModDataValue::String(val.to_string()),
        };
        let data = self.default_mod_data.get_or_insert_with(Vec::new);
        match data.iter_mut().find(|(k, _)| k == p) {
            Some(entry) => entry.1 = value,
            None => data.push((p.to_string(), value)),
        }
    }

    fn capture_icon(&mut self, sm: &ScriptManager, val: &str) -> pz_render::TextureResult<()> {
        self.texture_capture_complete = false;
        self.icon = s(val);
        let item_name = format!("Item_{val}");
        self.item_name = Some(item_name.clone());
        self.replace_normal_texture(pz_render::try_get_texture_ref(&item_name)?);
        if self.normal_texture.is_none() {
            self.replace_normal_texture(pz_render::get_shared_texture_ref(
                "media/inventory/Question_On.png",
            )?);
        }
        let world = format!(
            "{}.png",
            item_name.replace("Item_", "media/inventory/world/WItem_")
        );
        self.world_texture_name = Some(world.clone());
        self.replace_world_texture(pz_render::get_shared_texture_ref(&world)?);
        if self.is_item_type(sm, "FOOD") {
            let mut rotten = pz_render::try_get_texture_ref(&format!("{item_name}Rotten"))?;
            let mut world_rotten = world.replace(".png", "Rotten.png");
            if rotten.is_none() {
                rotten = pz_render::try_get_texture_ref(&format!("{item_name}Spoiled"))?;
                world_rotten = world_rotten.replace("Rotten.png", "Spoiled.png");
            }
            if rotten.is_none() {
                rotten = pz_render::try_get_texture_ref(&format!("{item_name}_Rotten"))?;
                world_rotten = world_rotten.replace("Rotten.png", "_Rotten.png");
            }
            self.special_world_texture_names.push(world_rotten);
            self.special_textures.push(rotten);
            let cooked = pz_render::try_get_texture_ref(&format!("{item_name}Cooked"))?;
            self.special_textures.push(cooked);
            self.special_world_texture_names
                .push(world.replace(".png", "Cooked.png"));
            let mut burnt = pz_render::try_get_texture_ref(&format!("{item_name}Overdone"))?;
            let mut world_burnt = world.replace(".png", "Overdone.png");
            if burnt.is_none() {
                burnt = pz_render::try_get_texture_ref(&format!("{item_name}Burnt"))?;
                world_burnt = world_burnt.replace("Overdone.png", "Burnt.png");
            }
            if burnt.is_none() {
                burnt = pz_render::try_get_texture_ref(&format!("{item_name}_Burnt"))?;
                world_burnt = world_burnt.replace("Overdone.png", "_Burnt.png");
            }
            self.special_textures.push(burnt);
            self.special_world_texture_names.push(world_burnt);
        }
        self.texture_capture_complete = true;
        Ok(())
    }

    /// The `Icon` param: the texture names derived from the icon. A food
    /// item looks for its rotten/cooked/burnt variants under fallback names.
    fn do_param_icon(&mut self, sm: &ScriptManager, val: &str) -> ScriptResult {
        if sm.texture_capture_enabled() {
            if let Err(error) = self.capture_icon(sm, val) {
                sm.record_texture_host_fault(error);
            }
            return Ok(());
        }
        // Explicit offline parser/data profile; actual slots remain uncaptured.
        self.texture_capture_complete = false;
        self.icon = s(val);
        let item_name = format!("Item_{val}");
        // TODO(Texture): normalTexture (falling back to Question_On.png) and
        // worldTexture.
        try_get_texture(sm, &item_name)?;
        let world = format!(
            "{}.png",
            item_name.replace("Item_", "media/inventory/world/WItem_")
        );
        if self.is_item_type(sm, "FOOD") {
            let mut w_rotten = world.replace(".png", "Rotten.png");
            let mut found = try_get_texture(sm, &format!("{item_name}Rotten"))?;
            if !found {
                found = try_get_texture(sm, &format!("{item_name}Spoiled"))?;
                w_rotten = w_rotten.replace("Rotten.png", "Spoiled.png");
            }
            if !found {
                try_get_texture(sm, &format!("{item_name}_Rotten"))?;
                w_rotten = w_rotten.replace("Rotten.png", "_Rotten.png");
            }
            self.special_world_texture_names.push(w_rotten);
            try_get_texture(sm, &format!("{item_name}Cooked"))?;
            self.special_world_texture_names
                .push(world.replace(".png", "Cooked.png"));
            let mut w_overdone = world.replace(".png", "Overdone.png");
            let mut found = try_get_texture(sm, &format!("{item_name}Overdone"))?;
            if !found {
                found = try_get_texture(sm, &format!("{item_name}Burnt"))?;
                w_overdone = w_overdone.replace("Overdone.png", "Burnt.png");
            }
            if !found {
                try_get_texture(sm, &format!("{item_name}_Burnt"))?;
                w_overdone = w_overdone.replace("Overdone.png", "_Burnt.png");
            }
            self.special_world_texture_names.push(w_overdone);
        }
        self.item_name = Some(item_name);
        self.world_texture_name = Some(world);
        Ok(())
    }

    /// The `modelWeaponPart` param: `partType modelName [attachSelf [attachParent]]`.
    fn do_param_model_weapon_part(&mut self, val: &str) {
        let module = self.module_name_or_null();
        let parts = self.model_weapon_part.get_or_insert_with(Vec::new);
        let ss = java::split_ws(val);
        if !(2..=4).contains(&ss.len()) {
            return;
        }
        let mwp = parts
            .iter()
            .find(|m| m.borrow().part_type == ss[0])
            .cloned()
            .unwrap_or_else(|| Rc::new(RefCell::new(ModelWeaponPart::default())));
        {
            let mut m = mwp.borrow_mut();
            m.part_type = ss[0].to_string();
            m.model_name = ss[1].to_string();
            m.attachment_name_self = ss.get(2).map(|s| s.to_string());
            m.attachment_parent = ss.get(3).map(|s| s.to_string());
            if !m.part_type.contains('.') {
                m.part_type = format!("{module}.{}", m.part_type);
            }
            if !m.model_name.contains('.') {
                m.model_name = format!("{module}.{}", m.model_name);
            }
            if m.attachment_name_self
                .as_deref()
                .is_some_and(|a| java::eq_ignore_case(a, "none"))
            {
                m.attachment_name_self = None;
            }
            if m.attachment_parent
                .as_deref()
                .is_some_and(|a| java::eq_ignore_case(a, "none"))
            {
                m.attachment_parent = None;
            }
        }
        parts.push(mwp);
    }

    /// `DoParam_VehiclePartModel`: `partId partModelId modelId`.
    fn do_param_vehicle_part_model(&mut self, val: &str) {
        let models = self.vehicle_part_models.get_or_insert_with(Vec::new);
        let ss = java::split_ws(val);
        if ss.len() != 3 {
            return;
        }
        models.push(VehiclePartModel {
            part_id: ss[0].to_string(),
            part_model_id: ss[1].to_string(),
            model_id: ss[2].to_string(),
        });
    }

    /// The `EvolvedRecipe` param: `Recipe:use[|Cooked];...`, one
    /// `ItemRecipe` per evolved recipe (or template) name.
    fn do_param_evolved_recipe(&mut self, val: &str) -> ScriptResult {
        let split = java::split_char(val, ';');
        for part in &split {
            self.evolved_recipe.push(java::trim(part).to_string());
        }
        let module = self
            .module()
            .map(|m| m.name())
            .unwrap_or_else(|| "null".to_string());
        for recipe in split {
            let mut use_ = 0;
            let mut cooked = false;
            let mut recipe_name = if recipe.contains(':') {
                let parts = java::split_char(recipe, ':');
                let use_str = at(&parts, 1)?;
                if use_str.contains('|') {
                    let splitted = java::split_char(use_str, '|');
                    if splitted.contains(&"Cooked") {
                        cooked = true;
                    }
                    use_ = parse_int(at(&splitted, 0)?)?;
                } else {
                    use_ = parse_int(use_str)?;
                }
                at(&parts, 0)?.to_string()
            } else {
                recipe.to_string()
            };
            match recipe_name.as_str() {
                "RicePot" | "RicePan" => recipe_name = "Rice".to_string(),
                "PastaPot" | "PastaPan" => recipe_name = "Pasta".to_string(),
                "Roasted Vegetables" => recipe_name = "Stir fry".to_string(),
                _ => {}
            }
            let mut item_recipe = ItemRecipe::new(&self.name, &module, use_);
            item_recipe.cooked = cooked;
            self.item_recipe_map.insert(recipe_name, item_recipe);
        }
        Ok(())
    }

    /// `Translator.setDefaultItemEvolvedRecipeName` followed by
    /// `getItemEvolvedRecipeName`: the first English name set for an item
    /// is kept. TODO(Translator): other languages.
    fn translator_evolved_recipe_name(&self, sm: &ScriptManager, english: &str) -> String {
        let full = self.full_name();
        sm.statics
            .items
            .borrow_mut()
            .item_evolved_recipe_name
            .entry(full)
            .or_insert_with(|| english.to_string())
            .clone()
    }

    /// `Translator.getItemNameFromFullType(getFullName())` without loaded
    /// translations: the display name of the item with this full type.
    /// TODO(Translator): translated item names.
    fn translator_item_name(&self, sm: &ScriptManager) -> String {
        let full = self.full_name();
        match sm
            .get_script(ScriptType::Item, &full)
            .and_then(|s| s.downcast::<Item>())
        {
            None => full,
            Some(item) => match item.try_borrow() {
                Ok(other) => other.display_name(),
                Err(_) => self.display_name(),
            },
        }
    }

    /// `resolveItemTypes`: makes item references in this item's fields
    /// module-qualified.
    pub fn resolve_item_types(&mut self, sm: &ScriptManager) {
        let Some(module) = self.module() else { return };
        let resolve = |t: Option<&str>| t.and_then(|t| sm.resolve_item_type(&module, t));
        if let Some(ammo) = &self.ammo_type {
            if !self.has_tag_constant(sm, "FAKE_WEAPON") {
                if let Some(resolved) = resolve(ammo.item_key.as_deref()) {
                    if let Some(_ammo_type) = AmmoType::get_by_item_key(sm, &resolved) {
                        // TODO(IsoBulletTracerEffects): load the tracer effect.
                    }
                }
            }
        }
        self.magazine_type = resolve(self.magazine_type.as_deref());
        self.physics_object = resolve(self.physics_object.as_deref());
        if let Some(list) = &mut self.require_in_hand_or_inventory {
            for t in list.iter_mut() {
                *t = resolve(t.as_deref());
            }
        }
        if let Some(map) = &mut self.replace_types_map {
            let keys: Vec<String> = map.keys().map(str::to_string).collect();
            for k in keys {
                let v = map.get_mut(&k).unwrap();
                *v = resolve(v.as_deref());
            }
        }
    }

    /// `resolveModelScripts`.
    pub fn resolve_model_scripts(&mut self, sm: &ScriptManager) {
        let Some(module) = self.module() else { return };
        self.static_model = self
            .static_model
            .as_deref()
            .and_then(|m| sm.resolve_model_script(&module, m));
        self.world_static_model = self
            .world_static_model
            .as_deref()
            .and_then(|m| sm.resolve_model_script(&module, m));
    }

    /// The tail of `Load`: hand replacements built from the masks.
    fn build_hand_replacements(&mut self) {
        let pair = |v: &Option<String>| -> Option<(String, String)> {
            let v = v
                .as_deref()
                .filter(|v| !java::is_null_or_whitespace(Some(v)))?;
            let ss = java::split_ws(java::trim(v));
            (ss.len() == 2).then(|| (java::trim(ss[0]).to_string(), java::trim(ss[1]).to_string()))
        };
        if let Some((item, value)) = pair(&self.replace_in_primary_hand) {
            self.replace_primary_hand = Some(ItemReplacement {
                clothing_item_name: Some(item),
                mask_variable_value: Some(value),
                mask_variable_name: s("RightHandMask"),
                attachment: None,
            });
        }
        if let Some((item, value)) = pair(&self.replace_in_second_hand) {
            self.replace_second_hand = Some(ItemReplacement {
                clothing_item_name: Some(item),
                mask_variable_value: Some(value),
                mask_variable_name: s("LeftHandMask"),
                attachment: None,
            });
        }
        if !java::is_null_or_whitespace(self.primary_anim_mask.as_deref()) {
            self.replace_primary_hand = Some(ItemReplacement {
                clothing_item_name: None,
                mask_variable_value: self.primary_anim_mask.clone(),
                mask_variable_name: s("RightHandMask"),
                attachment: self.primary_anim_mask_attachment.clone(),
            });
        }
        if !java::is_null_or_whitespace(self.secondary_anim_mask.as_deref()) {
            self.replace_second_hand = Some(ItemReplacement {
                clothing_item_name: None,
                mask_variable_value: self.secondary_anim_mask.clone(),
                mask_variable_name: s("LeftHandMask"),
                attachment: self.secondary_anim_mask_attachment.clone(),
            });
        }
    }

    fn load_block(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        self.entity.base.load_common_block(block);
        if sm.texture_capture_enabled() {
            self.texture_capture_complete = false;
        }
        for element in &block.elements {
            match element {
                BlockElement::Value(i) => self.do_param_str(sm, &block.values[*i].string)?,
                BlockElement::Block(i) => {
                    let child = &block.children[*i];
                    if child
                        .type_
                        .as_deref()
                        .is_some_and(|t| java::eq_ignore_case(t, "component"))
                    {
                        self.entity.load_component_block(sm, child)?;
                    }
                }
            }
            if sm.texture_capture_aborted() {
                return Ok(());
            }
        }
        self.build_hand_replacements();
        if sm.texture_capture_enabled() {
            if self.normal_texture.is_none() {
                if let Some(first) = self
                    .icons_for_texture
                    .as_ref()
                    .and_then(|icons| icons.first())
                {
                    match pz_render::try_get_texture_ref(&format!("Item_{first}")) {
                        Ok(texture) => self.replace_normal_texture(texture),
                        Err(error) => {
                            sm.record_texture_host_fault(error);
                            return Ok(());
                        }
                    }
                }
            }
            self.texture_capture_complete = true;
        }
        Ok(())
    }
}

impl ScriptObject for Item {
    fn base(&self) -> &BaseScriptObject {
        &self.entity.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.entity.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.Item"
    }

    /// `InitLoadPP`: also assigns the item's network id.
    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.file_name = sm.current_file_name();
        self.name = name.to_string();
        self.module_dot_type = format!("{}.{name}", self.module_name_or_null());
        ScriptObject::init_load_pp(&mut self.entity, sm, name);
        let mut st = sm.statics.items.borrow_mut();
        let id = st.id_max;
        st.id_max += 1;
        st.net_id_to_item.insert(id, self.module_dot_type.clone());
        st.net_item_to_id.insert(self.module_dot_type.clone(), id);
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.name = name.to_string();
        self.module_dot_type = format!("{}.{name}", self.module_name_or_null());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.load_block(sm, block)
    }

    fn get_obsolete(&self) -> bool {
        self.obsolete
    }

    fn pre_reload(&mut self) {
        self.entity.pre_reload();
    }

    /// `OnScriptsLoaded`: adds this item to the evolved recipes (and
    /// templates) it names, takes its display name and resolves its item
    /// config.
    fn on_scripts_loaded(&mut self, sm: &ScriptManager, mode: ScriptLoadMode) -> ScriptResult {
        let evolved = recipes::get_all_evolved_recipes(sm);
        for (key, item_recipe) in self.item_recipe_map.iter() {
            let mut found = false;
            if let Some(recipe) = recipes::get_evolved_recipe(sm, key) {
                recipe
                    .borrow_mut()
                    .items_list
                    .insert(self.name.clone(), item_recipe.clone());
                found = true;
            }
            for r in &evolved {
                let mut r = r.borrow_mut();
                let template = r
                    .template
                    .as_deref()
                    .ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?;
                if !java::eq_ignore_case(template, key) {
                    continue;
                }
                r.items_list.insert(self.name.clone(), item_recipe.clone());
                found = true;
            }
            if !found {
                let full = self.full_name();
                sm.script_error(&format!(
                    "Could not find evolved recipe or template: '{key}' in item = {full}"
                ));
                return Err(invalid_parameter(Some(&format!(
                    "Could not find evolved recipe or template: '{key}' in item: {full}"
                ))));
            }
        }
        self.display_name = Some(self.translator_item_name(sm));
        if let Some(key) = self.item_config_key.clone() {
            let config = sm.get_script(ScriptType::ItemConfig, &key).ok_or_else(|| {
                ScriptError::new(format!(
                    "java.lang.Exception: Cannot set item config '{key}' to item: {}",
                    self.full_name()
                ))
            })?;
            self.item_config = Some(config);
        }
        self.entity.on_scripts_loaded(sm, mode)
    }

    fn on_loaded_after_lua(&mut self, sm: &ScriptManager) -> ScriptResult {
        self.entity.on_loaded_after_lua(sm)
    }

    fn on_post_world_dictionary_init(&mut self, sm: &ScriptManager) -> ScriptResult {
        self.entity.on_post_world_dictionary_init(sm)
    }
}
