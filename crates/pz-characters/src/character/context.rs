//! The engine singletons the character simulation reads
//! (`GameTime.instance`, `SandboxOptions.instance`, `ZomboidGlobals`,
//! `GameServer.server`/`GameClient.client`, `Core.debug`/`Core.gameMode`,
//! `LuaEventManager`/`LuaHookManager`, `Translator`), behind [`SimContext`],
//! and the per-character world inputs ([`CharacterEnvironment`]) that come
//! from systems not ported yet (squares, buildings, inventory, sounds).
//!
//! Every default is the value the original has in a fresh single-player
//! game at normal speed with default sandbox options; nothing else is
//! assumed.

use std::rc::Rc;

use pz_scripts::registry::RegistryKey;

use crate::skills::PerkRef;

/// `ZomboidGlobals`' statics. All of them except the ones its static
/// initializer sets come from the Lua table `ZomboidGlobals`
/// (`ZomboidGlobals.Load`), so they default to 0 until loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct ZomboidGlobals {
    pub running_endurance_reduce: f64,
    pub sprinting_endurance_reduce: f64,
    /// `imobileEnduranceReduce` (Lua `ImobileEnduranceIncrease`).
    pub imobile_endurance_reduce: f64,
    /// Static initializer: 5.0.
    pub sitting_endurance_multiplier: f64,
    pub thirst_increase: f64,
    pub thirst_sleeping_increase: f64,
    pub thirst_level_to_auto_drink: f64,
    pub thirst_level_reduction_on_auto_drink: f64,
    pub hunger_increase: f64,
    pub hunger_increase_when_well_fed: f64,
    pub hunger_increase_while_asleep: f64,
    pub hunger_increase_when_exercise: f64,
    pub fatigue_increase: f64,
    /// `stressReduction` (Lua `StressDecrease`).
    pub stress_reduction: f64,
    pub boredom_increase_rate: f64,
    pub boredom_decrease_rate: f64,
    pub unhappiness_increase: f64,
    pub stress_from_sounds_multiplier: f64,
    pub stress_from_bite_or_scratch: f64,
    pub stress_from_hemophobic: f64,
    pub stress_from_discomfort: f64,
    pub anger_decrease: f64,
    pub sleep_fatigue_reduction: f64,
    pub wetness_increase: f64,
    pub wetness_decrease: f64,
    pub catch_a_cold_increase_rate: f64,
    pub catch_a_cold_decrease_rate: f64,
    pub poison_level_decrease: f64,
    pub poison_health_reduction: f64,
    pub food_sickness_decrease: f64,
    pub idle_increase_rate: f64,
    pub idle_decrease_rate: f64,
}

impl Default for ZomboidGlobals {
    fn default() -> Self {
        Self::INITIAL
    }
}

impl ZomboidGlobals {
    /// The values before `ZomboidGlobals.Load`.
    pub const INITIAL: ZomboidGlobals = ZomboidGlobals {
        running_endurance_reduce: 0.0,
        sprinting_endurance_reduce: 0.0,
        imobile_endurance_reduce: 0.0,
        sitting_endurance_multiplier: 5.0,
        thirst_increase: 0.0,
        thirst_sleeping_increase: 0.0,
        thirst_level_to_auto_drink: 0.0,
        thirst_level_reduction_on_auto_drink: 0.0,
        hunger_increase: 0.0,
        hunger_increase_when_well_fed: 0.0,
        hunger_increase_while_asleep: 0.0,
        hunger_increase_when_exercise: 0.0,
        fatigue_increase: 0.0,
        stress_reduction: 0.0,
        boredom_increase_rate: 0.0,
        boredom_decrease_rate: 0.0,
        unhappiness_increase: 0.0,
        stress_from_sounds_multiplier: 0.0,
        stress_from_bite_or_scratch: 0.0,
        stress_from_hemophobic: 0.0,
        stress_from_discomfort: 0.0,
        anger_decrease: 0.0,
        sleep_fatigue_reduction: 0.0,
        wetness_increase: 0.0,
        wetness_decrease: 0.0,
        catch_a_cold_increase_rate: 0.0,
        catch_a_cold_decrease_rate: 0.0,
        poison_level_decrease: 0.0,
        poison_health_reduction: 0.0,
        food_sickness_decrease: 0.0,
        idle_increase_rate: 0.0,
        idle_decrease_rate: 0.0,
    };
}

/// `HaloTextHelper` calls the simulation makes.
#[derive(Clone, Debug, PartialEq)]
pub enum HaloText {
    /// `addGoodText(player, text, "[br/]")`.
    Good(String),
    /// `addTextWithArrow(player, text, "[br/]", true, getGoodColor())`.
    GoodWithArrowUp(String),
}

/// Side effects the original performs through other singletons, in the
/// order it performs them.
#[derive(Clone, Debug)]
pub enum SimEvent {
    /// `LuaEventManager.triggerEvent("LevelPerk", chr, perk, level, increased)`.
    LevelPerk { perk: PerkRef, level: i32, increased: bool },
    /// `LuaEventManager.triggerEvent("LogLevelPerk", chr, perk, level, false)`
    /// (`XP.load`).
    LogLevelPerk { perk: PerkRef, level: i32 },
    /// `LuaEventManager.triggerEventGarbage("AddXP", chr, perk, amount)`.
    AddXp { perk: PerkRef, amount: f32 },
    /// `IsoPlayer.playGainExperienceLevelSound` (or, on a server,
    /// `sendObjectChange(PLAY_GAIN_EXPERIENCE_LEVEL_SOUND)`).
    GainExperienceLevelSound,
    /// `IsoPlayer.playerVoiceSound(name)`.
    PlayerVoiceSound(&'static str),
    /// `MoodlesUI.getInstance().wiggle(type)`.
    MoodleWiggle(Rc<RegistryKey>),
    /// `HaloTextHelper`.
    Halo(HaloText),
    /// `GameClient.sendPerks(player)`.
    SendPerks,
    /// `StopAllActionQueue()`.
    StopAllActionQueue,
}

/// The singletons the simulation reads. Every method has the default a
/// fresh single-player game has.
pub trait SimContext {
    /// `ZomboidGlobals`.
    fn zomboid_globals(&self) -> &ZomboidGlobals {
        &ZomboidGlobals::INITIAL
    }
    /// `GameServer.server`.
    fn is_server(&self) -> bool {
        false
    }
    /// `GameClient.client`.
    fn is_client(&self) -> bool {
        false
    }
    /// `Core.debug`.
    fn is_debug(&self) -> bool {
        false
    }
    /// `Core.gameMode`.
    fn game_mode(&self) -> &str {
        "Sandbox"
    }
    /// `GameTime.instance.getMultiplier()`.
    fn multiplier(&self) -> f32 {
        1.0
    }
    /// `GameTime.instance.getMinutesPerDay()`.
    fn minutes_per_day(&self) -> f32 {
        30.0
    }
    /// `GameTime.instance.getDeltaMinutesPerDay()`: `30 / minutesPerDay`.
    fn delta_minutes_per_day(&self) -> f32 {
        30.0 / self.minutes_per_day()
    }
    /// `GameTime.instance.getThirtyFPSMultiplier()`: `getMultiplier() / 1.6f`.
    fn thirty_fps_multiplier(&self) -> f32 {
        self.multiplier() / 1.6
    }
    /// `GameTime.instance.multiplierBias`.
    fn multiplier_bias(&self) -> f32 {
        1.0
    }
    /// `GameTime.getTimeDelta()`: `getMultiplier() / 0.8f / multiplierBias
    /// / 60.0f`.
    fn time_delta(&self) -> f32 {
        self.multiplier() / 0.8 / self.multiplier_bias() / 60.0
    }
    /// `GameTime.getGameWorldSecondsSinceLastUpdate()`: `getTimeDelta() *
    /// (1440.0f / getMinutesPerDay())`.
    fn game_world_seconds_since_last_update(&self) -> f32 {
        let dif = 1440.0 / self.minutes_per_day();
        self.time_delta() * dif
    }
    /// `GameTime.getMinutes()`: the minutes of `timeOfDay` (9.0 in a fresh
    /// game).
    fn minutes(&self) -> i32 {
        0
    }
    /// `GameTime.getCalender().getTimeInMillis()`. The default is 0: the
    /// owner of `GameTime` supplies the calendar.
    fn calendar_time_in_millis(&self) -> i64 {
        0
    }
    /// `GameTime.instance.getWorldAgeHours()`.
    fn world_age_hours(&self) -> f64 {
        0.0
    }
    /// `IsoPlayer.allPlayersAsleep()`.
    fn all_players_asleep(&self) -> bool {
        false
    }
    /// `SandboxOptions.instance.getStatsDecreaseMultiplier()`.
    fn stats_decrease_multiplier(&self) -> f64 {
        1.0
    }
    /// `SandboxOptions.instance.getEnduranceRegenMultiplier()`.
    fn endurance_regen_multiplier(&self) -> f64 {
        1.0
    }
    /// `SandboxOptions.instance.decayingCorpseHealthImpact.getValue()`.
    fn decaying_corpse_health_impact(&self) -> i32 {
        3
    }
    /// `SandboxOptions.instance.nutrition.getValue()` (the option's
    /// default is `false`; the presets turn it on).
    fn nutrition_enabled(&self) -> bool {
        false
    }
    /// `SystemDisabler.doCharacterStats`.
    fn do_character_stats(&self) -> bool {
        true
    }
    /// `ClimateManager.getInstance().getHumidity()`. The default is 0: the
    /// owner of the climate supplies it.
    fn climate_humidity(&self) -> f32 {
        0.0
    }
    /// `Rand.Next(min, max)`.
    fn rand_next_range(&self, min: i32, max: i32) -> i32 {
        pz_core::random::next_range(min, max)
    }
    /// `multipliersConfig.xpMultiplierGlobalToggle.getValue()`.
    fn xp_multiplier_global_toggle(&self) -> bool {
        true
    }
    /// `multipliersConfig.xpMultiplierGlobal.getValue()`.
    fn xp_multiplier_global(&self) -> f64 {
        1.0
    }
    /// `Float.parseFloat(getOptionByName("MultiplierConfig." + perk)
    /// .asConfigOption().getValueAsString())`.
    fn xp_multiplier_for(&self, _perk_id: &str) -> f32 {
        1.0
    }
    /// `ServerOptions.instance.sleepAllowed && sleepNeeded` (only read on a
    /// server).
    fn server_sleep_allowed_and_needed(&self) -> bool {
        false
    }
    /// `LuaHookManager.TriggerHook(name, chr)`: true when a hook took over.
    fn trigger_hook(&self, _name: &str) -> bool {
        false
    }
    /// `Translator.getText(key)`.
    fn get_text(&self, key: &str) -> String {
        key.to_string()
    }
    /// Receives the side effects (Lua events, sounds, UI).
    fn event(&self, _event: SimEvent) {}
}

/// One worn `Clothing` item as the thermoregulator reads it (an
/// `ItemVisual` of `getItemVisuals` whose `getInventoryItem()` is a
/// `Clothing`), in `getItemVisuals` order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WornClothing {
    /// `getInsulation()`.
    pub insulation: f32,
    /// `getWindresistance()`.
    pub windresistance: f32,
    /// `getWetness()`.
    pub wetness: f32,
    /// `getCurrentCondition()`.
    pub current_condition: f32,
    /// `BloodClothingType.getCoveredParts(getBloodClothingType())` as
    /// `BloodBodyPartType` indices; `None` when `getBloodClothingType()` is
    /// null.
    pub covered_parts: Option<Vec<i32>>,
    /// `getBodyLocation()` is `ItemBodyLocation.HAT` or `MASK`.
    pub hat_or_mask: bool,
    /// `getVisual().getHole(part)` by `BloodBodyPartType` index (missing
    /// entries are 0).
    pub holes: Vec<f32>,
}

/// A [`SimContext`] with every default.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultSimContext;

impl SimContext for DefaultSimContext {}

/// What the original reads per character from systems that are not ported
/// yet (`IsoGridSquare`, `IsoBuilding`, `ItemContainer`, `WorldSoundManager`,
/// `Temperature`, the state machine). The owner of those systems fills it
/// before the update; each field names the original call.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterEnvironment {
    /// `getCurrentSquare() != null`.
    pub has_current_square: bool,
    /// `getCurrentSquare() == getLastSquare()`.
    pub on_last_square: bool,
    /// `getCurrentSquare().isInARoom()`.
    pub square_in_room: bool,
    /// `getCurrentSquare().getRoomSize()`.
    pub room_size: i32,
    /// `vehicle != null`.
    pub in_vehicle: bool,
    /// `isCurrentlyIdle()`.
    pub currently_idle: bool,
    /// `isCurrentState(SwipeStatePlayer.instance())`.
    pub swipe_state: bool,
    /// `WorldSoundManager.instance.getStressFromSounds(x, y, z)`.
    pub stress_from_sounds: f32,
    /// `getTotalBlood()`.
    pub total_blood: f32,
    /// `getInventory().getCapacityWeight()`.
    pub inventory_capacity_weight: f32,
    /// `getCurrentBuilding() != null && getCurrentBuilding().isToxic()`.
    pub building_toxic: bool,
    /// `isProtectedFromToxic(false)`.
    pub protected_from_toxic: bool,
    /// `Temperature.getWindChillAmountForPlayer(player)`.
    pub wind_chill_amount: f32,
    /// `ClimateManager.getAirTemperatureForCharacter(chr, false)`. The
    /// owner of the climate supplies it (default 0).
    pub air_temperature: f32,
    /// `ClimateManager.getAirTemperatureForCharacter(chr, true)` (default 0).
    pub air_and_wind_temperature: f32,
    /// The worn clothing (`getItemVisuals`).
    pub worn_clothing: Vec<WornClothing>,
    /// `sprinting` (input and movement).
    pub sprinting: bool,
    /// `IsoPlayer.isAllowSprint()`.
    pub allow_sprint: bool,
    /// The primary or secondary hand item `isEquippedNoSprint()`.
    pub hand_item_equipped_no_sprint: bool,
    /// `sneaking`.
    pub sneaking: bool,
    /// `IsoPlayer.currentSpeed`.
    pub current_speed: f32,
    /// `WeaponType.getWeaponType(player)` (the constant's name) while
    /// `IsoPlayer.isAttacking()`; `None` when not attacking.
    pub attacking_weapon_type: Option<String>,
    /// `isCurrentState(ClimbOverFenceState)` or
    /// `isCurrentState(ClimbThroughWindowState)`.
    pub climbing_state: bool,
    /// `getCharacterActions().get(0).caloriesModifier`; `None` when there
    /// is no action.
    pub first_action_calories_modifier: Option<f32>,
}

impl Default for CharacterEnvironment {
    fn default() -> Self {
        CharacterEnvironment {
            has_current_square: false,
            on_last_square: true,
            square_in_room: false,
            room_size: 0,
            in_vehicle: false,
            currently_idle: false,
            swipe_state: false,
            stress_from_sounds: 0.0,
            total_blood: 0.0,
            inventory_capacity_weight: 0.0,
            building_toxic: false,
            protected_from_toxic: false,
            wind_chill_amount: 0.0,
            air_temperature: 0.0,
            air_and_wind_temperature: 0.0,
            worn_clothing: Vec::new(),
            sprinting: false,
            allow_sprint: true,
            hand_item_equipped_no_sprint: false,
            sneaking: false,
            current_speed: 0.0,
            attacking_weapon_type: None,
            climbing_state: false,
            first_action_calories_modifier: None,
        }
    }
}
