//! The engine singletons the character simulation reads
//! (`GameTime.instance`, `SandboxOptions.instance`, `ZomboidGlobals`,
//! `GameServer.server`/`GameClient.client`, `Core.debug`/`Core.gameMode`,
//! `LuaEventManager`/`LuaHookManager`, `Translator`), behind [`SimContext`],
//! and the per-character world inputs ([`CharacterEnvironment`]) that come
//! from systems not ported yet (squares, buildings, sounds).
//!
//! Every default is the value the original has in a fresh single-player
//! game at normal speed with default sandbox options; nothing else is
//! assumed.

use std::cell::Ref;
use std::rc::Rc;

use pz_core::weather::{AirTemperatureQuery, ClimateManager, HeatSource};
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
    pub vehicle_discomfort_when_over_encumbered: f64,
    pub equipped_or_worn_encumbrance_multiplier: f64,
    pub corpse_sickness_filter_drain_multiplier: f64,
    /// Static initializer: 25.0.
    pub basic_mask_defense: f32,
    /// Static initializer: 85.0.
    pub improvised_gas_mask_defense: f32,
    /// Static initializer: 100.0.
    pub standard_gas_mask_defense: f32,
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
        vehicle_discomfort_when_over_encumbered: 0.0,
        equipped_or_worn_encumbrance_multiplier: 0.0,
        corpse_sickness_filter_drain_multiplier: 0.0,
        basic_mask_defense: 25.0,
        improvised_gas_mask_defense: 85.0,
        standard_gas_mask_defense: 100.0,
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
    /// `LuaEventManager.triggerEvent("OnPlayerGetDamage", chr, kind, amount)`
    /// (`INFECTION` with the `Integer` 110 is `110.0` here).
    PlayerGetDamage { kind: &'static str, amount: f32 },
    /// `getCurrentSquare().getChunk().addBloodSplat(x, y, z, type)` from
    /// `splatBloodFloor()`/`splatBloodFloorBig()` (the draws are made by
    /// [`super::GameCharacter::splat_blood_floor`]).
    AddBloodSplat { kind: i32 },
    /// `addBlood(BloodBodyPartType.FromIndex(part), scratched, bitten,
    /// allLayers)`.
    AddBlood { part: i32, scratched: bool, bitten: bool, all_layers: bool },
    /// `addHole(BloodBodyPartType.FromIndex(part))` /
    /// `addHole(part, true)`: the worn items already have the hole
    /// (`CharacterItems::add_hole`); the human visual's is the renderer's.
    AddHole { part: i32, all_layers: bool },
    /// `addHoleFromZombieAttacks(BloodBodyPartType.FromIndex(part),
    /// scratchOrBite)`, already applied to the worn items.
    AddHoleFromZombieAttacks { part: i32, scratch_or_bite: bool },
    /// `playSound(name)` / `getEmitter().playSoundImpl(name, null)`.
    PlaySound(String),
    /// `getEmitter().stopOrTriggerSoundByName(name)`.
    StopSound(&'static str),
    /// `LuaEventManager.triggerEvent("OnZombieDead", zombie)`.
    ZombieDead,
    /// `getEmitter().playSound(name)` (the emitter's own path; the
    /// headless original's dummy emitter draws `Rand.Next(Integer.MAX_VALUE)`
    /// for the sound id).
    EmitterPlaySound(String),
    /// `playWeaponHitArmourSound(part, bullet)`.
    WeaponHitArmourSound { part: i32, bullet: bool },
    /// `WorldSoundManager.instance.addSound(chr, x, y, z, radius, volume,
    /// false)` (with `stressAnimals = false`), from a sneeze or cough.
    SneezeCoughSound { radius: i32, volume: i32 },
    /// `GameServer.sendSneezingCoughing(player, sneezingCoughing, var)`.
    SendSneezingCoughing { sneezing_coughing: i32, var: i32 },
    /// `LuaManager.caller.pcallvoid(thread, OnEat function, pill, chr)`.
    CallOnEat(String),
    /// `getNetworkCharacterAI().syncDamage()` / `syncVisuals()` on a server.
    SyncDamage,
    /// `LuaEventManager.triggerEvent("OnAIStateChange", chr, current,
    /// previous)`.
    AiStateChange {
        current: Option<crate::character::state::StateId>,
        previous: Option<crate::character::state::StateId>,
    },
    /// `DoFootstepSound(type)`.
    Footstep(String),
    /// `IsoPlayer.playerVoiceSound(name)` with a name from data.
    PlayerVoiceSoundNamed(String),
    /// `Kill(killer)` from the `KilledByAttacker` animation event (the
    /// killer by id).
    Kill { attacker: Option<i32> },
    /// `onKilled(killer, weapon, isGory)` (the subclass reaction, run by
    /// the owner of the corpse code).
    Killed { attacker: Option<i32> },
    /// `LuaEventManager.triggerEvent("OnWeaponHitCharacter", wielder,
    /// target, weapon, damageSplit)` (characters by id).
    WeaponHitCharacter { wielder: i32, target: i32, damage: f32 },
    /// `addWorldSoundUnlessInvisible(radius, volume, false)`.
    WorldSound { radius: i32, volume: i32 },
    /// `splatBlood(amount, speed)`.
    SplatBlood { amount: i32, speed: f32 },
    /// `die()` after `Kill`: the corpse (`becomeCorpse`).
    Die,
    /// `getCharacterActions().get(0).forceStop()`.
    ForceStopAction,
    /// `LuaEventManager.triggerEvent("OnWeaponSwing", chr, weapon)`.
    WeaponSwing,
    /// `LuaEventManager.triggerEvent("OnWeaponSwingHitPoint", chr, weapon)`.
    WeaponSwingHitPoint,
    /// `LuaEventManager.triggerEvent("OnWeaponHitXp", chr, weapon, target,
    /// damageSplit, 1)` (the target by id).
    WeaponHitXp { target: i32, damage: f32 },
    /// `LuaEventManager.triggerEvent("OnPlayerAttackFinished", chr, weapon)`.
    PlayerAttackFinished,
    /// The weapon's swing sound for a sound id (`getSoundByID`, else
    /// `getSwingSound`).
    SwingSound(String),
    /// An error the original would log (`DebugType.printException`).
    Error(String),
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
    /// `ClimateManager.getInstance()`; `None` when no climate runs (the
    /// climate getters then read 0 and the air temperatures come from
    /// [`CharacterEnvironment`]).
    fn climate(&self) -> Option<Ref<'_, ClimateManager>> {
        None
    }
    /// `IsoWorld.instance.isHydroPowerOn()`.
    fn hydro_power_on(&self) -> bool {
        false
    }
    /// `ClimateManager.getInstance().getHumidity()`.
    fn climate_humidity(&self) -> f32 {
        self.climate().map_or(0.0, |c| c.get_humidity())
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
    /// `Rand.Next(int max)` (`RandStandard.INSTANCE`).
    fn rand_next(&self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    /// `Rand.Next(float min, float max)`.
    fn rand_next_float(&self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
    /// `PerformanceSettings.getLockFPS()`.
    fn lock_fps(&self) -> i32 {
        60
    }
    /// `GameTime.instance.getInvMultiplier()`.
    fn inv_multiplier(&self) -> f32 {
        1.0 / self.multiplier()
    }
    /// `SandboxOptions.instance.lore.transmission.getValue()`.
    fn lore_transmission(&self) -> i32 {
        1
    }
    /// `SandboxOptions.instance.lore.mortality.getValue()`.
    fn lore_mortality(&self) -> i32 {
        5
    }
    /// `SandboxOptions.instance.lore.strength.getValue()`.
    fn lore_strength(&self) -> i32 {
        2
    }
    /// `SandboxOptions.instance.lore.zombiesDragDown.getValue()`.
    fn lore_zombies_drag_down(&self) -> bool {
        true
    }
    /// `SandboxOptions.instance.lore.zombiesCrawlersDragDown.getValue()`.
    fn lore_zombies_crawlers_drag_down(&self) -> bool {
        false
    }
    /// `Core.getInstance().isOptionAutoProneAtk()`.
    fn option_auto_prone_attack(&self) -> bool {
        true
    }
    /// `SandboxOptions.instance.multiHitZombies.getValue()`.
    fn multi_hit_zombies(&self) -> bool {
        false
    }
    /// `SandboxOptions.instance.attackBlockMovements.getValue()`.
    fn attack_block_movements(&self) -> bool {
        true
    }
    /// `SandboxOptions.instance.lore.toughness.getValue()`.
    fn lore_toughness(&self) -> i32 {
        2
    }
    /// `SandboxOptions.instance.lore.zombiesArmorFactor.getValue()`.
    fn lore_zombies_armor_factor(&self) -> f64 {
        2.0
    }
    /// `SandboxOptions.instance.lore.zombiesMaxDefense.getValue()`.
    fn lore_zombies_max_defense(&self) -> i32 {
        85
    }
    /// `SandboxOptions.instance.injurySeverity.getValue()` (1 low, 2
    /// normal, 3 high).
    fn injury_severity(&self) -> i32 {
        2
    }
    /// `SandboxOptions.instance.boneFracture.getValue()`.
    fn bone_fracture(&self) -> bool {
        true
    }
    /// `SandboxOptions.instance.woundInfectionFactor.getValue()`.
    fn wound_infection_factor(&self) -> f64 {
        0.0
    }
    /// `SandboxOptions.instance.rearVulnerability.getValue()`.
    fn rear_vulnerability(&self) -> i32 {
        3
    }
    /// `ServerOptions.instance.pvpMeleeDamageModifier.getValue()`.
    fn pvp_melee_damage_modifier(&self) -> f64 {
        30.0
    }
    /// `ServerOptions.instance.pvpFirearmDamageModifier.getValue()`.
    fn pvp_firearm_damage_modifier(&self) -> f64 {
        50.0
    }
    /// `SandboxOptions.instance.muscleStrainFactor.getValue()`.
    fn muscle_strain_factor(&self) -> f64 {
        1.0
    }
    /// `GameServer.fastForward`.
    fn server_fast_forward(&self) -> bool {
        false
    }
    /// `ClimateManager.getInstance().isRaining()`.
    fn is_raining(&self) -> bool {
        self.climate().is_some_and(|c| c.is_raining())
    }
    /// `ClimateManager.getInstance().getRainIntensity()`.
    fn rain_intensity(&self) -> f32 {
        self.climate().map_or(0.0, |c| c.get_rain_intensity())
    }
    /// `FliesSound.maxCorpseCount`.
    fn flies_max_corpse_count(&self) -> i32 {
        25
    }
    // ---- zombie lore (`SandboxOptions.instance.lore`) and the clock ----
    /// `lore.speed` (1 sprinters, 2 fast shamblers, 3 shamblers, 4 random).
    fn lore_speed(&self) -> i32 {
        2
    }
    /// `lore.sprinterPercentage`.
    fn lore_sprinter_percentage(&self) -> i32 {
        33
    }
    /// `lore.cognition` (1 navigate and use doors, 2 navigate, 3 basic
    /// navigation, 4 random).
    fn lore_cognition(&self) -> i32 {
        3
    }
    /// `lore.doorOpeningPercentage`.
    fn lore_door_opening_percentage(&self) -> i32 {
        33
    }
    /// `lore.crawlUnderVehicle` (1 never .. 7 always).
    fn lore_crawl_under_vehicle(&self) -> i32 {
        5
    }
    /// `lore.memory` (1 long .. 4 none, 5 random, 6 random between normal
    /// and none).
    fn lore_memory(&self) -> i32 {
        2
    }
    /// `lore.sight` (1 eagle, 2 normal, 3 poor, 4 random, 5 random between
    /// normal and poor).
    fn lore_sight(&self) -> i32 {
        2
    }
    /// `lore.hearing` (1 pinpoint, 2 normal, 3 poor, 4 random, 5 random
    /// between normal and poor).
    fn lore_hearing(&self) -> i32 {
        2
    }
    /// `lore.reanimate`.
    fn lore_reanimate(&self) -> i32 {
        3
    }
    /// `lore.spottedLogic`.
    fn lore_spotted_logic(&self) -> bool {
        true
    }
    /// `lore.thumpNoChasing`.
    fn lore_thump_no_chasing(&self) -> bool {
        false
    }
    /// `lore.thumpOnConstruction`.
    fn lore_thump_on_construction(&self) -> bool {
        true
    }
    /// `lore.activeOnly` (1 both, 2 night, 3 day).
    fn lore_active_only(&self) -> i32 {
        1
    }
    /// `lore.zombiesFenceLunge`.
    fn lore_zombies_fence_lunge(&self) -> bool {
        true
    }
    /// `lore.zombiesFallDamage`.
    fn lore_zombies_fall_damage(&self) -> f64 {
        1.0
    }
    /// `lore.disableFakeDead` (1 world zombies, 2 world and combat, 3 never).
    fn lore_disable_fake_dead(&self) -> i32 {
        1
    }
    /// `GameTime.isNight()` (for `activeOnly`).
    fn is_night(&self) -> bool {
        false
    }
    /// `GameTime.isZombieInactivityPhase()`.
    fn is_zombie_inactivity_phase(&self) -> bool {
        let active = match self.lore_active_only() {
            1 => true,
            2 => self.is_night(),
            3 => !self.is_night(),
            _ => false,
        };
        !active
    }
    /// `GameTime.getViewDist()`: the climate's view distance.
    fn view_dist(&self) -> f32 {
        self.climate().map_or(42.0, |c| c.get_view_distance())
    }
    /// `GameTime.getViewDistMax()`.
    fn view_dist_max(&self) -> f32 {
        42.0
    }
    /// `GameTime.getRealworldSecondsSinceLastUpdate()`.
    fn realworld_seconds_since_last_update(&self) -> f32 {
        0.016_666_668 * self.multiplier()
    }
    /// `GameTime.getTrueMultiplier()`.
    fn true_multiplier(&self) -> f32 {
        self.multiplier()
    }
    /// `ClimateManager.getInstance().getFogIntensity()`.
    fn fog_intensity(&self) -> f32 {
        self.climate().map_or(0.0, |c| c.get_fog_intensity())
    }
    /// `ClimateManager.getInstance().getDayLightStrength()`.
    fn day_light_strength(&self) -> f32 {
        self.climate().map_or(0.0, |c| c.get_day_light_strength())
    }
    /// `ClimateManager.getInstance().getAmbient()`.
    fn ambient(&self) -> f32 {
        self.climate().map_or(0.0, |c| c.get_ambient())
    }
    /// `Core.lastStand`.
    fn is_last_stand(&self) -> bool {
        self.game_mode() == "LastStand"
    }
    /// `Core.tutorial`.
    fn is_tutorial(&self) -> bool {
        self.game_mode() == "Tutorial"
    }
    /// `SystemDisabler.zombiesDontAttack`.
    fn zombies_dont_attack(&self) -> bool {
        false
    }
    /// `java.lang.Math.random()`.
    fn math_random(&self) -> f64 {
        math_random()
    }
    /// `System.currentTimeMillis()`.
    fn current_time_millis(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }
    /// Receives the side effects (Lua events, sounds, UI).
    fn event(&self, _event: SimEvent) {}
}

/// `java.lang.Math.random()`: one `java.util.Random` seeded from the clock
/// (the original's `RandomNumberGeneratorHolder`).
pub fn math_random() -> f64 {
    thread_local! {
        static R: pz_core::random::JavaRandom = pz_core::random::JavaRandom::new(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0),
        );
    }
    R.with(|r| r.next_double())
}

/// A [`SimContext`] with every default.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultSimContext;

impl SimContext for DefaultSimContext {}

/// What the original reads per character from systems that are not ported
/// yet (`IsoGridSquare`, `IsoBuilding`, `WorldSoundManager`,
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
    /// `getCurrentBuilding() != null && getCurrentBuilding().isToxic()`.
    pub building_toxic: bool,
    /// `Temperature.getWindChillAmountForPlayer(player)` when no climate
    /// runs ([`SimContext::climate`] is `None`).
    pub wind_chill_amount: f32,
    /// `isDraggingCorpse()`.
    pub dragging_corpse: bool,
    /// `isFallOnFront()`.
    pub fall_on_front: bool,
    /// `getCurrentSquare().haveRoof`.
    pub square_have_roof: bool,
    /// `vehicle.hasRoof(vehicle.getSeat(chr))`.
    pub vehicle_has_roof: bool,
    /// The vehicle's `Windshield` part has a destroyed window.
    pub vehicle_windshield_destroyed: bool,
    /// `vehicle.getCurrentSpeedKmHour()`.
    pub vehicle_speed_kmh: f32,
    /// `getBed() != null && getBed().getSprite() != null && getBed().isTent()`.
    pub bed_is_tent: bool,
    /// `IsSpeaking()`.
    pub speaking: bool,
    /// `callOut`.
    pub call_out: bool,
    /// The first queued action is not pathfinding (`isCurrentlyBusy`).
    pub busy_with_action: bool,
    /// `CorpseCount.instance.getCorpseCount(chr)`.
    pub corpse_count: i32,
    /// `ClimateManager.getAirTemperatureForCharacter(chr, false)` when no
    /// climate runs (default 0); with one it is computed.
    pub air_temperature: f32,
    /// `ClimateManager.getAirTemperatureForCharacter(chr, true)` when no
    /// climate runs (default 0).
    pub air_and_wind_temperature: f32,
    /// `getCurrentSquare().getZ()`.
    pub square_z: i32,
    /// `getVehicle().getInsideTemperature()`.
    pub vehicle_inside_temperature: f32,
    /// The cell's heat sources on the square's level
    /// (`IsoCell.getHeatSourceHighestTemperature`).
    pub heat_sources: Vec<HeatSource>,
    /// `sprinting` (input and movement).
    pub sprinting: bool,
    /// `IsoPlayer.isAllowSprint()`.
    pub allow_sprint: bool,
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
    /// `isNearSirenVehicle()`.
    pub near_siren_vehicle: bool,
    /// `getCurrentSquare().getChunk() != null` (blood splats draw `Rand`
    /// only then).
    pub square_has_chunk: bool,
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
            building_toxic: false,
            wind_chill_amount: 0.0,
            dragging_corpse: false,
            fall_on_front: false,
            square_have_roof: false,
            vehicle_has_roof: false,
            vehicle_windshield_destroyed: false,
            vehicle_speed_kmh: 0.0,
            bed_is_tent: false,
            speaking: false,
            call_out: false,
            busy_with_action: false,
            corpse_count: 0,
            air_temperature: 0.0,
            air_and_wind_temperature: 0.0,
            square_z: 0,
            vehicle_inside_temperature: 0.0,
            heat_sources: Vec::new(),
            sprinting: false,
            allow_sprint: true,
            sneaking: false,
            current_speed: 0.0,
            attacking_weapon_type: None,
            climbing_state: false,
            first_action_calories_modifier: None,
            near_siren_vehicle: false,
            square_has_chunk: false,
        }
    }
}

impl CharacterEnvironment {
    fn with_query<R>(&self, ctx: &dyn SimContext, f: impl FnOnce(Option<&AirTemperatureQuery<'_>>) -> R) -> R {
        if !self.has_current_square {
            return f(None);
        }
        let heat = |t: f32| pz_core::weather::heat_source_highest_temperature(t, &self.heat_sources);
        let q = AirTemperatureQuery {
            in_room: self.square_in_room,
            z: self.square_z,
            vehicle_inside_temperature: self.in_vehicle.then_some(self.vehicle_inside_temperature),
            hydro_power_on: ctx.hydro_power_on(),
            heat_source_highest: if self.heat_sources.is_empty() { None } else { Some(&heat) },
        };
        f(Some(&q))
    }

    /// `ClimateManager.getAirTemperatureForCharacter(chr, doWindChill)`:
    /// from the climate when one runs, else the supplied value.
    pub fn air_temperature_for_character(&self, ctx: &dyn SimContext, wind_chill: bool) -> f32 {
        match ctx.climate() {
            Some(c) => self.with_query(ctx, |q| c.get_air_temperature_for_square(q, wind_chill)),
            None if wind_chill => self.air_and_wind_temperature,
            None => self.air_temperature,
        }
    }

    /// `Temperature.getWindChillAmountForPlayer(player)`.
    pub fn wind_chill_amount_for_player(&self, ctx: &dyn SimContext) -> f32 {
        match ctx.climate() {
            Some(c) => self.with_query(ctx, |q| c.wind_chill_amount(q, self.in_vehicle)),
            None => self.wind_chill_amount,
        }
    }
}

#[cfg(test)]
mod climate_tests {
    use super::*;
    use std::cell::RefCell;

    struct Ctx(RefCell<ClimateManager>);

    impl SimContext for Ctx {
        fn climate(&self) -> Option<Ref<'_, ClimateManager>> {
            Some(self.0.borrow())
        }
    }

    #[test]
    fn climate_drives_air_temperature() {
        let mut c = ClimateManager::default();
        c.vars.floats[pz_core::weather::manager::FLOAT_TEMPERATURE].final_value = 30.0;
        c.vars.floats[pz_core::weather::manager::FLOAT_HUMIDITY].final_value = 0.75;
        c.vars.floats[pz_core::weather::manager::FLOAT_PRECIPITATION_INTENSITY].final_value = 0.5;
        c.wind_power = 0.4;
        let ctx = Ctx(RefCell::new(c));
        let mut env = CharacterEnvironment { air_temperature: -5.0, ..CharacterEnvironment::default() };
        // No square: the climate's temperature.
        assert_eq!(env.air_temperature_for_character(&ctx, false), 30.0);
        env.has_current_square = true;
        env.square_in_room = true;
        // Above 22 indoors on the ground floor: 15% of the excess at 40%.
        assert_eq!(env.air_temperature_for_character(&ctx, false), 30.0 - (8.0f32 * 0.85) * 0.4);
        env.square_in_room = false;
        env.heat_sources.push(HeatSource { dist_sq: 4.0, radius: 4, temperature: 60, source_in_room: Some(false), line_clear: true });
        assert!(env.air_temperature_for_character(&ctx, false) > 30.0);
        assert_eq!(env.wind_chill_amount_for_player(&ctx), 0.0);
        assert_eq!(ctx.climate_humidity(), 0.75);
        assert!(ctx.is_raining());
        assert_eq!(ctx.rain_intensity(), 0.5);
        // Without a climate the supplied values are used.
        assert_eq!(env.air_temperature_for_character(&DefaultSimContext, false), -5.0);
    }
}
