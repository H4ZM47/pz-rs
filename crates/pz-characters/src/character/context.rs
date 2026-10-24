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
    pub vehicle_discomfort_when_over_encumbered: f64,
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
    /// A tissue in hand was used by a sneeze (`setCurrentUses(uses - 1)`,
    /// and `Use()` when it reaches 0).
    TissueUsed { primary: bool, used_up: bool },
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
    /// `splatBloodFloor()` (TODO(IsoChunk): consumes `Rand` when the square
    /// has a chunk).
    SplatBloodFloor,
    /// `splatBloodFloorBig()`.
    SplatBloodFloorBig,
    /// `addBlood(BloodBodyPartType.FromIndex(part), scratched, bitten,
    /// allLayers)`.
    AddBlood { part: i32, scratched: bool, bitten: bool, all_layers: bool },
    /// `addHole(BloodBodyPartType.FromIndex(part))` /
    /// `addHole(part, true)`.
    AddHole { part: i32, all_layers: bool },
    /// `addHoleFromZombieAttacks(BloodBodyPartType.FromIndex(part),
    /// scratchOrBite)`.
    AddHoleFromZombieAttacks { part: i32, scratch_or_bite: bool },
    /// `playSound(name)` / `getEmitter().playSoundImpl(name, null)`.
    PlaySound(String),
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
    /// `Rand.Next(int min, int max)`.
    fn rand_next_range(&self, min: i32, max: i32) -> i32 {
        pz_core::random::next_range(min, max)
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
    /// `GameTime.instance.getGameWorldSecondsSinceLastUpdate()`:
    /// `getTimeDelta() * (1440 / minutesPerDay)` (`multiplierBias` 1).
    fn game_world_seconds_since_last_update(&self) -> f32 {
        let time_delta = self.multiplier() / 0.8 / 1.0 / 60.0;
        time_delta * (1440.0 / self.minutes_per_day())
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
        false
    }
    /// `ClimateManager.getInstance().getRainIntensity()`.
    fn rain_intensity(&self) -> f32 {
        0.0
    }
    /// `FliesSound.maxCorpseCount`.
    fn flies_max_corpse_count(&self) -> i32 {
        25
    }
    /// Receives the side effects (Lua events, sounds, UI).
    fn event(&self, _event: SimEvent) {}
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
    /// `isDraggingCorpse()`.
    pub dragging_corpse: bool,
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
    /// `ClimateManager.getInstance().getAirTemperatureForCharacter(chr)`.
    pub air_temperature: f32,
    /// `IsSpeaking()`.
    pub speaking: bool,
    /// `callOut`.
    pub call_out: bool,
    /// The first queued action is not pathfinding (`isCurrentlyBusy`).
    pub busy_with_action: bool,
    /// `CorpseCount.instance.getCorpseCount(chr)`.
    pub corpse_count: i32,
    /// `getCorpseSicknessDefense(rate, true)` (TODO(Clothing): it also
    /// drains gas mask filters).
    pub corpse_sickness_defense: f32,
    /// `hasDirtyClothing(part)`, by body part index (missing: false).
    pub dirty_clothing: Vec<bool>,
    /// `hasBloodyClothing(part)`, by body part index (missing: false).
    pub bloody_clothing: Vec<bool>,
    /// The primary hand item's `getCurrentUses()` when it is a `Tissue`,
    /// `ToiletPaper` or has the `MUFFLE_SNEEZE` tag.
    pub primary_hand_tissue_uses: Option<i32>,
    /// The same for the secondary hand item.
    pub secondary_hand_tissue_uses: Option<i32>,
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
            dragging_corpse: false,
            square_have_roof: false,
            vehicle_has_roof: false,
            vehicle_windshield_destroyed: false,
            vehicle_speed_kmh: 0.0,
            bed_is_tent: false,
            air_temperature: 0.0,
            speaking: false,
            call_out: false,
            busy_with_action: false,
            corpse_count: 0,
            corpse_sickness_defense: 0.0,
            dirty_clothing: Vec::new(),
            bloody_clothing: Vec::new(),
            primary_hand_tissue_uses: None,
            secondary_hand_tissue_uses: None,
        }
    }
}
