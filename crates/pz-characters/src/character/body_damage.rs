//! `zombie.characters.BodyDamage.BodyDamage`.
//!
//! [`BodyDamage`] holds the state (the body parts, their last state, the
//! infection, cold and sneeze timers, the tuning fields). The original's
//! `BodyDamage` holds its owner and the owner's `Stats`; the methods that
//! read or change the owner (`Update` and everything it calls, the injury
//! entry points, food and pills) are methods of [`GameCharacter`] here,
//! named after the original method (`Update` is
//! [`GameCharacter::body_damage_update`]).
//!
//! Side effects on unported systems (blood splats, clothing holes, sounds,
//! Lua `OnPlayerGetDamage`, network sync) are [`SimEvent`]s in the
//! original's order; inputs from them come from
//! [`super::CharacterEnvironment`], the [`SimContext`] and the attacker
//! structs ([`ZombieAttack`], [`WeaponUser`]). Food, pills, literature and
//! weapons are `pz-inventory` items; the clothing is the character's worn
//! items ([`super::items`]).
//!
//! TODO(MusicIntensityConfig): `RestoreToFullHealth` does not notify it.
//! TODO(Thermoregulator): its part of `save`/`load` is the caller's.
//! TODO(IsoAnimal): `DamageFromAnimal`.

use pz_inventory::gameplay::UseOutcome;
use pz_inventory::{InventoryItem, ItemOwner};
use pz_scripts::objects::items::WeaponCategory;
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::body_part::{
    adjust_for_framerate, java_min, next_bool, per_thirtieth_of_a_second, BodyPart, BodyPartLast, BodyPartTypes,
    InjuryEnv,
};
use super::context::{SimContext, SimEvent};
use super::item_owner::CharacterItemContext;
use super::items::{has_tag, is_hand_weapon};
use super::stats::{clamp_f32, CharacterStatRef, CharacterStats, Stats};
use super::thermoregulator::Thermoregulator;
use super::GameCharacter;
use crate::Characters;

fn npe(what: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.NullPointerException: {what}"))
}

fn class_cast() -> ScriptError {
    ScriptError::new(
        "java.lang.ClassCastException: class zombie.characters.IsoGameCharacter cannot be cast to class \
         zombie.characters.IsoPlayer",
    )
}

/// Java's `Math.round(float)`.
fn java_round(x: f32) -> i32 {
    if x.is_nan() {
        0
    } else {
        (x as f64 + 0.5).floor() as i32
    }
}

/// `PZMath.fastfloor(float)`.
fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `BodyDamage`.
#[derive(Clone, Debug)]
pub struct BodyDamage {
    /// `bodyParts`, in `BodyPartType` order.
    pub parts: Vec<BodyPart>,
    /// `bodyPartsLastState`.
    pub last_state: Vec<BodyPartLast>,
    pub damage_mod_count: i32,
    pub infection_growth_rate: f32,
    pub is_infected: bool,
    pub infection_time: f32,
    pub infection_mortality_duration: f32,
    pub is_fake_infected: bool,
    pub overall_body_health: f32,
    pub standard_health_addition: f32,
    pub reduced_health_addition: f32,
    pub severly_reduced_health_addition: f32,
    pub sleeping_health_addition: f32,
    pub health_from_food: f32,
    pub health_reduction_from_severe_bad_moodles: f32,
    pub standard_health_from_food_time: i32,
    pub health_from_food_timer: f32,
    pub boredom_decrease_from_reading: f32,
    pub initial_thump_pain: f32,
    pub initial_scratch_pain: f32,
    pub initial_bite_pain: f32,
    pub initial_wound_pain: f32,
    pub continual_pain_increase: f32,
    pub pain_reduction_from_meds: f32,
    pub standard_pain_reduction_when_well: f32,
    pub old_num_zombies_visible: i32,
    pub current_num_zombies_visible: i32,
    pub panic_increase_value: f32,
    pub panic_reduction_value: f32,
    pub drunk_increase_value: f32,
    pub drunk_reduction_value: f32,
    pub is_on_fire: bool,
    pub burnt_to_death: bool,
    pub catch_a_cold: f32,
    pub has_a_cold: bool,
    pub cold_strength: f32,
    pub cold_progression_rate: f32,
    pub time_to_sneeze_or_cough: f32,
    pub mild_cold_sneeze_timer_min: i32,
    pub mild_cold_sneeze_timer_max: i32,
    pub cold_sneeze_timer_min: i32,
    pub cold_sneeze_timer_max: i32,
    pub nasty_cold_sneeze_timer_min: i32,
    pub nasty_cold_sneeze_timer_max: i32,
    pub sneeze_cough_active: i32,
    pub sneeze_cough_time: i32,
    pub sneeze_cough_delay: i32,
    pub cold_damage_stage: f32,
    pub remote_pain_level: i32,
    pub reduce_fake_infection: bool,
    pub pain_reduction: f32,
    pub cold_reduction: f32,
    /// `thermoregulator` (players only).
    pub thermoregulator: Option<Thermoregulator>,
    pub was_dragging_corpse: bool,
    pub started_dragging_corpse: bool,
}

/// `BodyDamage.InfectionLevelToZombify`.
pub const INFECTION_LEVEL_TO_ZOMBIFY: f32 = 0.001;
const SMOKER_SNEEZE_TIMER_MIN: i32 = 43200;
const SMOKER_SNEEZE_TIMER_MAX: i32 = 129600;
const PANIC_INCREASE_VALUE_FRAME: f32 = 0.035;

impl BodyDamage {
    /// `new BodyDamage(parent)`: one part per `BodyPartType` below `MAX`,
    /// restored to full health (before the owner is set, so the owner's
    /// stats are untouched), and a `Thermoregulator` for players.
    pub fn new(sm: &ScriptManager, player: bool) -> Self {
        let types = &BodyPartTypes::from_tables(&sm.builtin_tables.borrow());
        let parts: Vec<BodyPart> =
            types.decls.iter().take(types.max_index()).enumerate().map(|(i, d)| BodyPart::new(d, i)).collect();
        let last_state = vec![BodyPartLast::default(); parts.len()];
        let mut bd = BodyDamage {
            parts,
            last_state,
            damage_mod_count: 60,
            infection_growth_rate: 0.001,
            is_infected: false,
            infection_time: -1.0,
            infection_mortality_duration: -1.0,
            is_fake_infected: false,
            overall_body_health: 100.0,
            standard_health_addition: 0.002,
            reduced_health_addition: 0.0013,
            severly_reduced_health_addition: 8.0e-4,
            sleeping_health_addition: 0.02,
            health_from_food: 0.015,
            health_reduction_from_severe_bad_moodles: 0.0165,
            standard_health_from_food_time: 1600,
            health_from_food_timer: 0.0,
            boredom_decrease_from_reading: 0.5,
            initial_thump_pain: 14.0,
            initial_scratch_pain: 18.0,
            initial_bite_pain: 25.0,
            initial_wound_pain: 80.0,
            continual_pain_increase: 0.001,
            pain_reduction_from_meds: 30.0,
            standard_pain_reduction_when_well: 0.01,
            old_num_zombies_visible: 0,
            current_num_zombies_visible: 0,
            panic_increase_value: 7.0,
            panic_reduction_value: 0.06,
            drunk_increase_value: 400.0,
            drunk_reduction_value: 0.0042,
            is_on_fire: false,
            burnt_to_death: false,
            catch_a_cold: 0.0,
            has_a_cold: false,
            cold_strength: 0.0,
            cold_progression_rate: 0.0112,
            time_to_sneeze_or_cough: -1.0,
            mild_cold_sneeze_timer_min: 600,
            mild_cold_sneeze_timer_max: 800,
            cold_sneeze_timer_min: 300,
            cold_sneeze_timer_max: 600,
            nasty_cold_sneeze_timer_min: 200,
            nasty_cold_sneeze_timer_max: 300,
            sneeze_cough_active: 0,
            sneeze_cough_time: 0,
            sneeze_cough_delay: 25,
            cold_damage_stage: 0.0,
            remote_pain_level: 0,
            reduce_fake_infection: false,
            pain_reduction: 0.0,
            cold_reduction: 0.0,
            thermoregulator: None,
            was_dragging_corpse: false,
            started_dragging_corpse: false,
        };
        bd.restore_parts_to_full_health();
        if player {
            bd.thermoregulator = Some(Thermoregulator::new(sm));
        }
        bd.set_body_parts_last_state();
        bd
    }

    /// The owner-independent part of `RestoreToFullHealth`.
    fn restore_parts_to_full_health(&mut self) {
        for p in &mut self.parts {
            p.restore_to_full_health();
        }
        self.is_infected = false;
        self.set_is_fake_infected(false);
        self.overall_body_health = 100.0;
        self.catch_a_cold = 0.0;
        self.has_a_cold = false;
        self.cold_strength = 0.0;
        self.sneeze_cough_active = 0;
        self.sneeze_cough_time = 0;
        self.infection_time = -1.0;
        self.infection_mortality_duration = -1.0;
    }

    /// `getBodyPart(type)` by index.
    pub fn body_part(&self, index: usize) -> Option<&BodyPart> {
        self.parts.get(index)
    }
    /// `getBodyPart(type)` by index, mutably.
    pub fn body_part_mut(&mut self, index: usize) -> Option<&mut BodyPart> {
        self.parts.get_mut(index)
    }
    /// `getBodyPartsLastState`.
    pub fn body_parts_last_state(&self, index: usize) -> Option<&BodyPartLast> {
        self.last_state.get(index)
    }
    /// `setBodyPartsLastState`.
    pub fn set_body_parts_last_state(&mut self) {
        for (l, p) in self.last_state.iter_mut().zip(&self.parts) {
            l.copy(p);
        }
    }

    /// `getOverallBodyHealth`.
    pub fn overall_body_health(&self) -> f32 {
        self.overall_body_health
    }
    /// `getHealth`: the overall body health.
    pub fn health(&self) -> f32 {
        self.overall_body_health()
    }
    /// `getNumPartsBleeding`.
    pub fn num_parts_bleeding(&self) -> i32 {
        self.parts.iter().filter(|p| p.bleeding).count() as i32
    }
    /// `isNeckBleeding` (`false` without a `Neck` part).
    pub fn is_neck_bleeding(&self) -> bool {
        self.parts.iter().any(|p| p.type_field == "Neck" && p.bleeding)
    }
    /// `getNumPartsBitten`.
    pub fn num_parts_bitten(&self) -> i32 {
        self.parts.iter().filter(|p| p.bitten).count() as i32
    }
    /// `getNumPartsScratched`.
    pub fn num_parts_scratched(&self) -> i32 {
        self.parts.iter().filter(|p| p.scratched).count() as i32
    }
    /// `HasInjury`.
    pub fn has_injury(&self) -> bool {
        self.parts.iter().any(BodyPart::has_injury)
    }
    /// `IsInfected`.
    pub fn is_infected(&self) -> bool {
        self.is_infected
    }
    /// `IsFakeInfected`.
    pub fn is_fake_infected(&self) -> bool {
        self.is_fake_infected
    }
    /// `setIsFakeInfected`: also the first part's flag.
    pub fn set_is_fake_infected(&mut self, v: bool) {
        self.is_fake_infected = v;
        if let Some(p) = self.parts.first_mut() {
            p.is_fake_infected = v;
        }
    }
    /// `getColdStrength`: 0 without a cold.
    pub fn cold_strength(&self) -> f32 {
        if self.has_a_cold {
            self.cold_strength
        } else {
            0.0
        }
    }
    /// `getHealthFromFoodTimer`.
    pub fn health_from_food_timer(&self) -> f32 {
        self.health_from_food_timer
    }
    /// `getStandardHealthFromFoodTime`.
    pub fn standard_health_from_food_time(&self) -> i32 {
        self.standard_health_from_food_time
    }
    /// `getThermoregulator`.
    pub fn thermoregulator(&self) -> Option<&Thermoregulator> {
        self.thermoregulator.as_ref()
    }
    /// `getThermoregulator` (mutable).
    pub fn thermoregulator_mut(&mut self) -> Option<&mut Thermoregulator> {
        self.thermoregulator.as_mut()
    }
    /// `setColdDamageStage`.
    pub fn set_cold_damage_stage(&mut self, stage: f32) {
        self.cold_damage_stage = stage;
    }
    /// `getBodyPart(BodyPartType.FromIndex(index)).getWetness()` (0 without
    /// the part).
    pub fn body_part_wetness(&self, index: usize) -> f32 {
        self.parts.get(index).map_or(0.0, |p| p.wetness)
    }
    /// `getBodyPart(BodyPartType.FromIndex(index)).getStiffness()` (0
    /// without the part).
    pub fn body_part_stiffness(&self, index: usize) -> f32 {
        self.parts.get(index).map_or(0.0, |p| p.stiffness)
    }
    /// `getBodyPart(BodyPartType.FromIndex(index)).setStiffness(v)`:
    /// clamped to 0..100 (nothing without the part).
    pub fn set_body_part_stiffness(&mut self, index: usize, v: f32) {
        if let Some(p) = self.parts.get_mut(index) {
            p.set_stiffness(v);
        }
    }
    /// `getApparentInfectionLevel`: the largest of zombie fever, zombie
    /// infection and food sickness.
    pub fn apparent_infection_level(&self, stats: &Stats, defs: &CharacterStats) -> ScriptResult<f32> {
        let infection = stats.get(&defs.field("ZOMBIE_FEVER")?).max(stats.get(&defs.field("ZOMBIE_INFECTION")?));
        Ok(stats.get(&defs.field("FOOD_SICKNESS")?).max(infection))
    }
    /// `IsSneezingCoughing`.
    pub fn is_sneezing_coughing(&self) -> i32 {
        self.sneeze_cough_active
    }
    /// `getPanicIncreaseValueFrame`.
    pub fn panic_increase_value_frame(&self) -> f32 {
        PANIC_INCREASE_VALUE_FRAME
    }
    /// `getSmokerSneezeTimerMin`.
    pub fn smoker_sneeze_timer_min(&self) -> i32 {
        SMOKER_SNEEZE_TIMER_MIN
    }
    /// `getSmokerSneezeTimerMax`.
    pub fn smoker_sneeze_timer_max(&self) -> i32 {
        SMOKER_SNEEZE_TIMER_MAX
    }

    /// `AddGeneralHealth`: spread over the damaged parts.
    pub fn add_general_health(&mut self, val: f32) {
        let damaged = self.parts.iter().filter(|p| p.health < 100.0).count();
        if damaged > 0 {
            let per = val / damaged as f32;
            for p in &mut self.parts {
                if p.health < 100.0 {
                    p.add_health(per);
                }
            }
        }
    }

    /// `ReduceGeneralHealth` without the owner's `forceAwake`: returns
    /// whether the owner is forced awake (overall health at most 10).
    fn reduce_general_health_parts(&mut self, types: &BodyPartTypes, val: f32) -> bool {
        let wake = self.overall_body_health <= 10.0;
        if val <= 0.0 {
            return wake;
        }
        let per = val / types.max_index() as f32;
        for (i, p) in self.parts.iter_mut().enumerate() {
            p.reduce_health(per / types.damage_modifier(i as i32));
        }
        wake
    }

    /// `calculateOverallHealth`, given `getDamageFromPills()`.
    pub fn calculate_overall_health_with(&mut self, types: &BodyPartTypes, damage_from_pills: f32) {
        let mut total = 0.0f32;
        for (i, p) in self.parts.iter().enumerate() {
            total += (100.0 - p.health) * types.damage_modifier(i as i32);
        }
        total += damage_from_pills;
        if total > 100.0 {
            total = 100.0;
        }
        self.overall_body_health = 100.0 - total;
    }

    /// `UseBandageOnMostNeededPart`.
    pub fn use_bandage_on_most_needed_part(&mut self) -> bool {
        let mut highest = 0;
        let mut part = None;
        for (n, p) in self.parts.iter().enumerate() {
            if p.bandaged {
                continue;
            }
            let mut score = 0;
            if p.bleeding {
                score += 100;
            }
            if p.scratched {
                score += 50;
            }
            if p.bitten {
                score += 50;
            }
            if score <= highest {
                continue;
            }
            highest = score;
            part = Some(n);
        }
        if let (true, Some(n)) = (highest > 0, part) {
            self.parts[n].set_bandaged_simple(true, 10.0);
            return true;
        }
        false
    }

    /// `getGeneralWoundInfectionLevel`.
    pub fn general_wound_infection_level(&self, ctx: &dyn SimContext) -> f32 {
        let factor = ctx.wound_infection_factor();
        if factor <= 0.0 {
            return 0.0;
        }
        let mut level = 0.0f32;
        for p in &self.parts {
            if p.infected_wound {
                level += p.wound_infection_level;
            }
        }
        level *= 10.0;
        level *= factor as f32;
        java_min(level, 100.0)
    }

    /// `ShowDebugInfo`.
    pub fn show_debug_info(&mut self) {
        if self.damage_mod_count > 0 {
            self.damage_mod_count -= 1;
        }
    }

    /// `saveMainFields`.
    pub fn save_main_fields(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.catch_a_cold.to_be_bytes());
        out.push(self.has_a_cold as u8);
        out.extend_from_slice(&self.cold_strength().to_be_bytes());
        out.extend_from_slice(&(self.time_to_sneeze_or_cough as i32).to_be_bytes());
        out.push(self.reduce_fake_infection as u8);
        for f in [
            self.health_from_food_timer,
            self.pain_reduction,
            self.cold_reduction,
            self.infection_time,
            self.infection_mortality_duration,
            self.cold_damage_stage,
        ] {
            out.extend_from_slice(&f.to_be_bytes());
        }
    }

    /// `save` up to the thermoregulator flag (the caller writes
    /// `thermoregulator != null` and the thermoregulator's save).
    pub fn save_parts_and_main_fields(&self, out: &mut Vec<u8>) {
        fn s(out: &mut Vec<u8>, v: &Option<String>) {
            // `GameWindow.WriteString`: a short length then the bytes; null
            // is written as the empty string.
            let b = v.as_deref().unwrap_or("").as_bytes();
            out.extend_from_slice(&(b.len() as i16).to_be_bytes());
            out.extend_from_slice(b);
        }
        for p in &self.parts {
            for b in [
                p.cut,
                p.bitten,
                p.scratched,
                p.bandaged,
                p.bleeding,
                p.deep_wounded,
                p.is_fake_infected,
                p.is_infected,
            ] {
                out.push(b as u8);
            }
            out.extend_from_slice(&p.health.to_be_bytes());
            if p.bandaged {
                out.extend_from_slice(&p.bandage_life.to_be_bytes());
            }
            out.push(p.infected_wound as u8);
            if p.infected_wound {
                out.extend_from_slice(&p.wound_infection_level.to_be_bytes());
            }
            for f in [
                p.cut_time,
                p.bite_time,
                p.scratch_time,
                p.bleeding_time,
                p.alcohol_level,
                p.additional_pain,
                p.deep_wound_time,
            ] {
                out.extend_from_slice(&f.to_be_bytes());
            }
            out.push(p.have_glass as u8);
            out.push(p.get_bandage_xp as u8);
            out.push(p.stitched as u8);
            out.extend_from_slice(&p.stitch_time.to_be_bytes());
            out.push(p.get_stitch_xp as u8);
            out.push(p.get_splint_xp as u8);
            out.extend_from_slice(&p.fracture_time.to_be_bytes());
            out.push(p.splint as u8);
            if p.splint {
                out.extend_from_slice(&p.splint_factor.to_be_bytes());
            }
            out.push(p.have_bullet as u8);
            out.extend_from_slice(&p.burn_time.to_be_bytes());
            out.push(p.need_burn_wash as u8);
            out.extend_from_slice(&p.last_time_burn_wash.to_be_bytes());
            s(out, &p.splint_item);
            s(out, &p.bandage_type);
            for f in [p.cut_time, p.wetness, p.stiffness, p.comfrey_factor, p.garlic_factor, p.plantain_factor] {
                out.extend_from_slice(&f.to_be_bytes());
            }
        }
        self.save_main_fields(out);
    }
}

/// The player using a weapon (`weapon.getUsingPlayer()`), as
/// `DamageFromWeapon` and `damageFromSpikedArmor` read it.
#[derive(Clone, Copy, Debug, Default)]
pub struct WeaponUser<'a> {
    /// `isDoShove()` (`IsoLivingCharacter`).
    pub do_shove: bool,
    /// `isAimAtFloor()`.
    pub aim_at_floor: bool,
    /// `isBehind(target)`.
    pub behind: bool,
    /// `getPrimaryHandItem()`.
    pub primary_hand_item: Option<&'a InventoryItem>,
    /// `getSecondaryHandItem()`.
    pub secondary_hand_item: Option<&'a InventoryItem>,
}

/// `WeaponType.getWeaponType(weapon) == WeaponType.KNIFE` (STUB in
/// `pz-inventory`: never a knife until `get_weapon_type` is ported).
fn is_knife(weapon: &InventoryItem) -> bool {
    weapon.get_weapon_type() == Some("KNIFE")
}

/// `weapon.isOfWeaponCategory(WeaponCategory.<field>)`.
fn is_of_weapon_category(sm: &ScriptManager, weapon: &InventoryItem, field: &str) -> bool {
    WeaponCategory::constant(sm, field)
        .is_some_and(|k| weapon.get_weapon_categories().iter().any(|c| c.eq_ignore_ascii_case(&k.location.to_string())))
}

/// A zombie attack as `AddRandomDamageFromZombie` reads it (the zombie
/// side; the clothing defense and spikes are the character's worn items).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZombieAttack {
    pub hit_reaction: Option<String>,
    pub part_index: i32,
    /// `getMeleeCombatMod()`.
    pub melee_combat_mod: i32,
    /// `testDotSide(zombie)`: `"BEHIND"`, `"LEFT"`, `"RIGHT"` or other.
    pub dot_side: String,
    /// `getSurroundingAttackingZombies()`.
    pub surrounding_attacking_zombies: i32,
    /// `getSurroundingAttackingZombies(zombiesCrawlersDragDown)`.
    pub drag_down_zombies: i32,
    /// The character's `getHitReaction()` before the attack.
    pub current_hit_reaction: String,
    /// `zombie.crawling`.
    pub crawling: bool,
    /// `zombie.inactive`.
    pub inactive: bool,
    /// `zombie.cantBite()`.
    pub cant_bite: bool,
    /// `zombie.getBiteSoundName()`.
    pub bite_sound: String,
}

/// What `AddRandomDamageFromZombie` did to the zombie and the character
/// besides the body damage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZombieAttackOutcome {
    /// The return value.
    pub hit: bool,
    /// `setHitReaction` (when called).
    pub hit_reaction: Option<String>,
    /// `setDeathDragDown(true)`.
    pub death_drag_down: bool,
    /// `zombie.scratch` / `zombie.laceration` after the rolls.
    pub scratch: bool,
    pub laceration: bool,
    /// `zombie.parameterZombieState.setState(...)`.
    pub zombie_state: Option<&'static str>,
    /// `helmetFall(neckOrHead)`.
    pub helmet_fall: Option<bool>,
    /// `zombie.spikePart(type)` calls (`BodyPartType` constant names).
    pub zombie_spiked: Vec<&'static str>,
    /// `zombie.Kill(null)`.
    pub zombie_killed: bool,
    /// `GameClient.sendZombieHit` (the client returns after it).
    pub sent_zombie_hit: bool,
    /// The chosen body part.
    pub part_index: i32,
}

fn stat(chars: &Characters, f: &str) -> ScriptResult<CharacterStatRef> {
    chars.sim.character_stats.field(f)
}

impl<I> GameCharacter<I> {
    fn moodle(&self, sm: &ScriptManager, field: &str) -> ScriptResult<i32> {
        self.moodles.moodle_level_of(sm, field)
    }

    /// The owner state `BodyPart` injuries read.
    pub fn injury_env<'a>(&self, sm: &ScriptManager, ctx: &'a dyn SimContext) -> ScriptResult<InjuryEnv<'a>> {
        Ok(InjuryEnv {
            ctx,
            fast_healer: self.has_trait(sm, "FAST_HEALER")?,
            slow_healer: self.has_trait(sm, "SLOW_HEALER")?,
        })
    }

    /// `isInvulnerable`: the god mode cheat.
    pub fn is_invulnerable(&self, ctx: &dyn SimContext) -> bool {
        self.is_god_mod(ctx)
    }

    /// `forceAwake`.
    pub fn force_awake(&mut self) {
        if self.asleep {
            self.force_wake_up = true;
        }
    }

    /// `setHealth`: 0 is ignored while invulnerable.
    pub fn set_health(&mut self, ctx: &dyn SimContext, health: f32) {
        if health == 0.0 && self.is_invulnerable(ctx) {
            return;
        }
        self.health = health;
    }

    /// `getWeightMod`.
    pub fn weight_mod(&self, chars: &Characters) -> f32 {
        match self.perk_level(chars.perks.constant("Strength").as_ref()) {
            1 => 0.9,
            2 => 1.07,
            3 => 1.24,
            4 => 1.41,
            5 => 1.58,
            6 => 1.75,
            7 => 1.92,
            8 => 2.09,
            9 => 2.26,
            10 => 2.5,
            _ => 0.8,
        }
    }

    /// `SleepingTablet`.
    pub fn sleeping_tablet(&mut self, delta: f32) {
        self.sleeping_tablet_effect = 6600.0;
        self.sleeping_tablet_delta += delta;
    }
    /// `BetaBlockers`.
    pub fn beta_blockers(&mut self, delta: f32) {
        self.beta_effect = 6600.0;
        self.beta_delta += delta;
    }
    /// `BetaAntiDepress`.
    pub fn beta_anti_depress(&mut self, delta: f32) {
        if self.depress_effect == 0.0 {
            self.depress_first_take_time = 10000.0;
        }
        self.depress_effect = 6600.0;
        self.depress_delta += delta;
    }
    /// `PainMeds`.
    pub fn pain_meds(&mut self, delta: f32) {
        self.pain_effect = 5400.0;
        self.pain_delta += delta;
    }

    /// `isCurrentlyBusy`.
    pub fn is_currently_busy(&self) -> bool {
        if !self.is_player() {
            return true;
        }
        self.asleep || self.is_reading || self.environment.busy_with_action || self.is_in_combat()
    }

    /// `addStiffness(type, stiffness)` on the part named `field`.
    fn add_stiffness_to(&mut self, chars: &Characters, field: &str, v: f32) -> ScriptResult {
        let i = chars.sim.body_part_types.index(field)?;
        if let Some(p) = self.body_damage.parts.get_mut(i) {
            p.add_stiffness(v);
        }
        Ok(())
    }

    /// `addBackMuscleStrain`.
    pub fn add_back_muscle_strain(&mut self, chars: &Characters, ctx: &dyn SimContext, mut f: f32) -> ScriptResult {
        if ctx.muscle_strain_factor() > 0.0 {
            f *= 2.5;
            f = (f as f64 * ctx.muscle_strain_factor()) as f32;
            self.add_stiffness_to(chars, "Torso_Upper", f)?;
            self.add_stiffness_to(chars, "Torso_Lower", f)?;
        }
        Ok(())
    }

    /// `BodyDamage.ReduceGeneralHealth`.
    pub fn reduce_general_health(&mut self, chars: &Characters, val: f32) {
        if self.body_damage.reduce_general_health_parts(&chars.sim.body_part_types, val) {
            self.force_awake();
        }
    }

    /// `BodyDamage.getDamageFromPills`.
    fn damage_from_pills(&self) -> f32 {
        if self.is_player() {
            match self.sleeping_pills_taken {
                10 => return 40.0,
                11 => return 80.0,
                n if n >= 12 => return 100.0,
                _ => {}
            }
        }
        0.0
    }

    /// `BodyDamage.calculateOverallHealth`.
    pub fn calculate_overall_health(&mut self, chars: &Characters) {
        let pills = self.damage_from_pills();
        self.body_damage.calculate_overall_health_with(&chars.sim.body_part_types, pills);
    }

    /// `BodyDamage.decreaseBodyWetness`.
    pub fn decrease_body_wetness(&mut self, chars: &Characters, amount: f32) -> ScriptResult {
        for p in &mut self.body_damage.parts {
            p.set_wetness(p.wetness - amount);
        }
        self.stats.remove(&stat(chars, "WETNESS")?, amount);
        Ok(())
    }

    /// `BodyDamage.increaseBodyWetness`.
    pub fn increase_body_wetness(&mut self, chars: &Characters, amount: f32) -> ScriptResult {
        for p in &mut self.body_damage.parts {
            p.set_wetness(p.wetness + amount);
        }
        self.stats.add(&stat(chars, "WETNESS")?, amount);
        Ok(())
    }

    /// `BodyDamage.RestoreToFullHealth`.
    pub fn restore_to_full_health(&mut self, chars: &Characters) -> ScriptResult {
        for p in &mut self.body_damage.parts {
            p.restore_to_full_health();
        }
        self.stats.reset_stats(&chars.sim.character_stats);
        self.corpse_sickness_rate = 0.0;
        let bd = &mut self.body_damage;
        bd.is_infected = false;
        bd.set_is_fake_infected(false);
        bd.overall_body_health = 100.0;
        bd.catch_a_cold = 0.0;
        bd.has_a_cold = false;
        bd.cold_strength = 0.0;
        bd.sneeze_cough_active = 0;
        bd.sneeze_cough_time = 0;
        bd.infection_time = -1.0;
        bd.infection_mortality_duration = -1.0;
        if let Some(t) = &mut bd.thermoregulator {
            t.reset()?;
        }
        Ok(())
    }

    /// `BodyDamage.loadMainFields`.
    pub fn load_body_damage_main_fields(
        &mut self,
        chars: &Characters,
        r: &mut SaveReader<'_>,
        world_version: i32,
    ) -> ScriptResult {
        let bd = &mut self.body_damage;
        bd.catch_a_cold = r.get_float()?;
        bd.has_a_cold = r.get_bool()?;
        bd.cold_strength = r.get_float()?;
        if world_version >= 222 {
            bd.time_to_sneeze_or_cough = r.get_int()? as f32;
        }
        bd.reduce_fake_infection = r.get_bool()?;
        bd.health_from_food_timer = r.get_float()?;
        bd.pain_reduction = r.get_float()?;
        bd.cold_reduction = r.get_float()?;
        bd.infection_time = r.get_float()?;
        bd.infection_mortality_duration = r.get_float()?;
        bd.cold_damage_stage = r.get_float()?;
        self.calculate_overall_health(chars);
        Ok(())
    }

    /// `BodyDamage.load` up to the thermoregulator flag (the caller reads the
    /// flag and the thermoregulator).
    pub fn load_body_damage(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        r: &mut SaveReader<'_>,
        world_version: i32,
    ) -> ScriptResult {
        let env = self.injury_env(sm, ctx)?;
        for p in &mut self.body_damage.parts {
            let cut = r.get_bool()?;
            // `setCut(cut, false)`, `SetBitten(bitten)`, `setScratched(.., false)`.
            p.set_cut(&env, cut, false);
            let bitten = r.get_bool()?;
            p.set_bitten(&env, bitten);
            let scratched = r.get_bool()?;
            p.set_scratched(&env, scratched, false);
            let bandaged = r.get_bool()?;
            p.set_bandaged_simple(bandaged, 0.0);
            p.set_bleeding(r.get_bool()?);
            p.set_deep_wounded(r.get_bool()?);
            p.is_fake_infected = r.get_bool()?;
            p.is_infected = r.get_bool()?;
            p.set_health(r.get_float()?);
            if p.bandaged {
                p.set_bandage_life(r.get_float()?);
            }
            p.infected_wound = r.get_bool()?;
            if p.infected_wound {
                p.set_wound_infection_level(r.get_float()?);
            }
            p.set_cut_time(r.get_float()?);
            p.bite_time = r.get_float()?;
            p.set_scratch_time(r.get_float()?);
            p.set_bleeding_time(r.get_float()?);
            p.alcohol_level = r.get_float()?;
            p.additional_pain = r.get_float()?;
            p.deep_wound_time = r.get_float()?;
            p.have_glass = r.get_bool()?;
            p.get_bandage_xp = r.get_bool()?;
            p.set_stitched(&env, r.get_bool()?);
            p.stitch_time = r.get_float()?;
            p.get_stitch_xp = r.get_bool()?;
            p.get_splint_xp = r.get_bool()?;
            p.fracture_time = r.get_float()?;
            p.set_splint(r.get_bool()?, 0.0);
            if p.splint {
                p.splint_factor = r.get_float()?;
            }
            p.set_have_bullet(&env, r.get_bool()?, 0);
            p.burn_time = r.get_float()?;
            p.set_need_burn_wash(r.get_bool()?);
            p.last_time_burn_wash = r.get_float()?;
            p.splint_item = Some(r.read_string()?);
            p.bandage_type = Some(r.read_string()?);
            p.set_cut_time(r.get_float()?);
            p.set_wetness(r.get_float()?);
            p.set_stiffness(r.get_float()?);
            if world_version < 227 {
                continue;
            }
            p.set_comfrey_factor(r.get_float()?);
            p.set_garlic_factor(r.get_float()?);
            p.set_plantain_factor(r.get_float()?);
        }
        self.body_damage.set_body_parts_last_state();
        self.load_body_damage_main_fields(chars, r, world_version)
    }

    // ---- panic ----------------------------------------------------------

    fn panic_trait_factor(&self, sm: &ScriptManager) -> ScriptResult<f32> {
        let mut del = 1.0f32;
        if self.beta_effect > 0.0 {
            del -= self.beta_delta;
            del = del.clamp(0.0, 1.0);
        }
        if self.has_trait(sm, "COWARDLY")? {
            del *= 2.0;
        }
        if self.has_trait(sm, "BRAVE")? {
            del *= 0.3;
        }
        if self.has_trait(sm, "DESENSITIZED")? {
            del *= 0.15;
        }
        Ok(del)
    }

    /// `BodyDamage.IncreasePanicFloat`.
    pub fn increase_panic_float(&mut self, sm: &ScriptManager, chars: &Characters, delta: f32) -> ScriptResult {
        let del = self.panic_trait_factor(sm)?;
        let panic = stat(chars, "PANIC")?;
        self.stats.add(&panic, PANIC_INCREASE_VALUE_FRAME * delta * del);
        if self.has_trait(sm, "DESENSITIZED")? {
            self.stats.reset(&panic);
        }
        Ok(())
    }

    /// `BodyDamage.IncreasePanic`.
    pub fn increase_panic(&mut self, sm: &ScriptManager, chars: &Characters, mut n: i32) -> ScriptResult {
        if self.environment.in_vehicle {
            n /= 2;
        }
        let del = self.panic_trait_factor(sm)?;
        let panic = stat(chars, "PANIC")?;
        self.stats.add(&panic, self.body_damage.panic_increase_value * n as f32 * del);
        if self.has_trait(sm, "DESENSITIZED")? {
            self.stats.reset(&panic);
        }
        Ok(())
    }

    /// `BodyDamage.ReducePanic`.
    pub fn reduce_panic(&mut self, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let panic = stat(chars, "PANIC")?;
        if self.stats.is_at_minimum(&panic) {
            return Ok(());
        }
        let r = self.body_damage.panic_reduction_value;
        let mut delta = r * ctx.thirty_fps_multiplier();
        let mut months = fastfloor((self.hours_survived(ctx) as i32 / 24 / 30) as f32);
        if months > 5 {
            months = 5;
        }
        delta += r * months as f32;
        if self.asleep {
            delta *= 2.0;
        }
        self.stats.remove(&panic, delta);
        Ok(())
    }

    /// `BodyDamage.UpdateDraggingCorpse`.
    pub fn update_dragging_corpse(&mut self) {
        let dragging = self.environment.dragging_corpse;
        let bd = &mut self.body_damage;
        if dragging != bd.was_dragging_corpse {
            bd.started_dragging_corpse = dragging;
            bd.was_dragging_corpse = dragging;
        } else {
            bd.started_dragging_corpse = false;
        }
    }

    /// `BodyDamage.UpdatePanicState`.
    pub fn update_panic_state(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let visible = self.stats.num_visible_zombies();
        let old = self.body_damage.old_num_zombies_visible;
        self.body_damage.old_num_zombies_visible = visible;
        let new = visible - old;
        if self.has_trait(sm, "DESENSITIZED")? {
            self.stats.reset(&stat(chars, "PANIC")?);
            return Ok(());
        }
        let count = new.max(0);
        if count > 0 {
            self.increase_panic(sm, chars, count)
        } else {
            self.reduce_panic(chars, ctx)
        }
    }

    // ---- food, drink, pills ------------------------------------------------

    /// `BodyDamage.JustDrankBooze(food, percentage)`.
    pub fn just_drank_booze(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        food: &InventoryItem,
        mut percentage: f32,
    ) -> ScriptResult {
        let mut del = 1.0f32;
        if food.get_base_hunger() != 0.0 {
            percentage = food.get_hung_change() * percentage / food.get_base_hunger() * 2.0;
        }
        del *= percentage;
        if food.get_name().to_lowercase().contains("beer") || has_tag(sm, food, "LOW_ALCOHOL") {
            del *= 0.25;
        }
        let hunger = self.stats.get(&stat(chars, "HUNGER")?);
        if hunger > 0.8 {
            del *= 1.25;
        } else if hunger > 0.6 {
            del *= 1.1;
        }
        self.stats.add(&stat(chars, "INTOXICATION")?, self.body_damage.drunk_increase_value * del);
        self.sleeping_tablet(0.02 * percentage);
        self.beta_anti_depress(0.4 * percentage);
        self.beta_blockers(0.2 * percentage);
        self.pain_meds(0.2 * percentage);
        Ok(())
    }

    /// `BodyDamage.JustDrankBoozeFluid`.
    pub fn just_drank_booze_fluid(&mut self, chars: &Characters, alcohol: f32) -> ScriptResult {
        let mut del = 1.0f32;
        del *= alcohol;
        let hunger = self.stats.get(&stat(chars, "HUNGER")?);
        if hunger > 0.8 {
            del *= 1.1;
        } else if hunger > 0.6 {
            del *= 1.25;
        }
        self.stats.add(&stat(chars, "INTOXICATION")?, self.body_damage.drunk_increase_value * del);
        self.sleeping_tablet(0.02 * alcohol);
        self.beta_anti_depress(0.4 * alcohol);
        self.beta_blockers(0.2 * alcohol);
        self.pain_meds(0.2 * alcohol);
        Ok(())
    }

    /// `BodyDamage.JustTookPill(pill)` without its final `UseAndSync`.
    fn pill_effects(&mut self, chars: &Characters, ctx: &dyn SimContext, pill: &InventoryItem) -> ScriptResult {
        let drunk = self.stats.get(&stat(chars, "INTOXICATION")?) > 10.0;
        match pill.get_type() {
            "PillsBeta" => self.beta_blockers(if drunk { 0.15 } else { 0.3 }),
            "PillsAntiDep" => self.beta_anti_depress(if drunk { 0.15 } else { 0.3 }),
            "PillsSleepingTablets" => {
                self.sleeping_tablet(0.1);
                if self.is_player() {
                    let n = self.sleeping_pills_taken + 1;
                    self.set_sleeping_pills_taken(chars, ctx, n)?;
                }
            }
            "Pills" => self.pain_meds(if drunk { 0.15 } else { 0.45 }),
            "PillsVitamins" => {
                let f = stat(chars, "FATIGUE")?;
                if drunk {
                    self.stats.add(&f, pill.get_fatigue_change() / 2.0);
                } else {
                    self.stats.add(&f, pill.get_fatigue_change());
                }
            }
            _ => {}
        }
        self.stats.add(&stat(chars, "STRESS")?, pill.get_stress_change());
        if let Some(f) = pill.get_on_eat() {
            ctx.event(SimEvent::CallOnEat(f.to_string()));
        }
        Ok(())
    }

    /// `BodyDamage.JustTookPill(pill)`: ends with `pill.UseAndSync()`
    /// (`Use(false, false, GameServer.server)`) for a pill held by `owner`;
    /// the caller applies the returned outcome to the pill's container
    /// (see [`Self::just_took_held_pill`] for a pill in this character's
    /// inventory).
    pub fn just_took_pill(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        pill: &mut InventoryItem,
        owner: &dyn ItemOwner,
    ) -> ScriptResult<UseOutcome> {
        self.pill_effects(chars, ctx, pill)?;
        let ictx = CharacterItemContext::new(sm, ctx);
        Ok(pill.use_with(false, false, ctx.is_server(), owner, &ictx))
    }

    /// [`Self::just_took_pill`] for the item `id` of this character's
    /// inventory, with the `Use` applied. `None` when it holds no such item.
    pub fn just_took_held_pill(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        id: i32,
    ) -> ScriptResult<Option<UseOutcome>> {
        let Some(pill) = self.items.item(id).cloned() else { return Ok(None) };
        self.pill_effects(chars, ctx, &pill)?;
        Ok(self.use_held_item(sm, ctx, id, false, false, ctx.is_server()))
    }

    /// `BodyDamage.JustAteFood(food, percentage, useUtensil)`.
    pub fn just_ate_food(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        food: &InventoryItem,
        percentage: f32,
        use_utensil: bool,
    ) -> ScriptResult {
        let poison = stat(chars, "POISON")?;
        let pain = stat(chars, "PAIN")?;
        if food.get_poison_power() > 0 {
            let mut power = food.get_poison_power() as f32 * percentage;
            if self.has_trait(sm, "IRON_GUT")? && food.get_type() != "Bleach" {
                power /= 2.0;
            }
            if self.has_trait(sm, "WEAK_STOMACH")? {
                power *= 2.0;
            }
            self.stats.add(&poison, power);
            self.stats.add(&pain, food.get_poison_power() as f32 * percentage / 6.0);
        }
        if food.is_tainted() {
            self.stats.add(&poison, 20.0 * percentage);
            self.stats.add(&pain, 10.0 * percentage / 6.0);
        }
        if food.get_reduce_infection_power() > 0.0 {
            self.reduce_infection_power = food.get_reduce_infection_power();
        }
        let modifier = |change: f32| {
            if !use_utensil {
                1.0f32
            } else if change * percentage < 0.0 {
                1.25
            } else {
                0.75
            }
        };
        let boredom = food.get_boredom_change();
        self.stats.add(&stat(chars, "BOREDOM")?, boredom * percentage * modifier(boredom));
        let unhappy = food.get_unhappy_change();
        self.stats.add(&stat(chars, "UNHAPPINESS")?, unhappy * percentage * modifier(unhappy));
        if food.is_alcoholic() {
            self.just_drank_booze(sm, chars, food, percentage)?;
        }
        if self.stats.is_at_minimum(&stat(chars, "HUNGER")?) {
            let change = food.get_hunger_change().abs() * percentage;
            let bd = &mut self.body_damage;
            bd.health_from_food_timer = (bd.health_from_food_timer + change * 13000.0) as i32 as f32;
            if food.is_cooked() {
                bd.health_from_food_timer = (bd.health_from_food_timer + change * 13000.0) as i32 as f32;
            }
            if bd.health_from_food_timer > 11000.0 {
                bd.health_from_food_timer = 11000.0;
            }
        }
        if ctx.game_mode() == "Tutorial" {
            return Ok(());
        }
        let egg = has_tag(sm, food, "EGG");
        if !food.is_cooked() && food.is_b_dangerous_uncooked() {
            self.body_damage.health_from_food_timer = 0.0;
            let mut chance = 75;
            if egg {
                chance = 5;
            }
            if self.has_trait(sm, "IRON_GUT")? {
                chance /= 2;
                if egg {
                    chance = 0;
                }
            }
            if self.has_trait(sm, "WEAK_STOMACH")? {
                chance *= 2;
            }
            if chance > 0 && !self.body_damage.is_infected && !food.is_burnt() {
                self.stats.add(&poison, 15.0 * percentage);
            }
        }
        let (age, off_age, off_age_max) = (food.get_age(), food.get_off_age(), food.get_off_age_max());
        if age >= off_age_max as f32 {
            let mut off = age - off_age_max as f32;
            if off == 0.0 {
                off = 1.0;
            }
            if off > 5.0 {
                off = 5.0;
            }
            let mut chance = if off_age_max > off_age {
                (off / (off_age_max - off_age) as f32 * 100.0) as i32
            } else {
                100
            };
            if self.has_trait(sm, "IRON_GUT")? {
                chance /= 2;
            }
            if self.has_trait(sm, "WEAK_STOMACH")? {
                chance *= 2;
            }
            if !self.body_damage.is_infected {
                if ctx.rand_next(100) < chance {
                    self.stats.add(&poison, 5.0 * (food.get_hung_change() * 10.0).abs() * percentage);
                } else {
                    self.stats.add(&poison, 2.0 * (food.get_hung_change() * 10.0).abs() * percentage);
                }
            }
        }
        Ok(())
    }

    /// `BodyDamage.JustReadSomething(literature)`.
    pub fn just_read_something(&mut self, chars: &Characters, literature: &InventoryItem) -> ScriptResult {
        self.stats.add(&stat(chars, "BOREDOM")?, literature.get_boredom_change());
        self.stats.add(&stat(chars, "UNHAPPINESS")?, literature.get_unhappy_change());
        Ok(())
    }

    /// `BodyDamage.JustTookPainMeds`.
    pub fn just_took_pain_meds(&mut self, chars: &Characters) -> ScriptResult {
        self.stats.remove(&stat(chars, "PAIN")?, self.body_damage.pain_reduction_from_meds);
        Ok(())
    }

    // ---- cold and wetness ------------------------------------------------

    /// `BodyDamage.UpdateWetness`.
    pub fn update_wetness(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let env = &self.environment;
        let mut outside = !env.has_current_square || !env.square_in_room && !env.square_have_roof;
        if env.in_vehicle && env.vehicle_has_roof {
            outside = false;
        }
        let mut increase = 0.0f32;
        let mut decrease = 0.0f32;
        let mut windshield = 0.0f32;
        if env.in_vehicle && ctx.is_raining() && env.vehicle_windshield_destroyed {
            let mut v = ctx.rain_intensity();
            v *= v;
            v *= env.vehicle_speed_kmh / 50.0;
            if v < 0.1 {
                v = 0.0;
            }
            if v > 1.0 {
                v = 1.0;
            }
            windshield = v * 3.0;
            increase = v;
        }
        if outside
            && (self.asleep || self.sit_on_ground || self.sitting_on_furniture || self.is_resting)
            && env.bed_is_tent
        {
            outside = false;
        }
        if outside && ctx.is_raining() {
            let mut v = ctx.rain_intensity();
            if v < 0.1 {
                v = 0.0;
            }
            increase = v;
        } else {
            let t = env.air_temperature_for_character(ctx, false);
            let mut v = 0.1f32;
            if t > 5.0 {
                v += (t - 5.0) / 10.0;
            }
            v -= windshield;
            if v < 0.0 {
                v = 0.0;
            }
            decrease = v;
        }
        if let Some(cw) = self.clothing_wetness.as_mut() {
            let thermo = self.body_damage.thermoregulator.as_ref().ok_or_else(|| npe("getThermoregulator()"))?;
            cw.update_wetness(
                sm,
                ctx,
                &chars.sim.body_part_types,
                &mut self.items,
                &mut self.body_damage.parts,
                thermo,
                increase,
                decrease,
            );
            // TODO(network): `getClothingWetnessSync().update()` on a server.
        }
        let wet = stat(chars, "WETNESS")?;
        let current = self.stats.get(&wet);
        let mut average = 0.0f32;
        let parts = &mut self.body_damage.parts;
        if !parts.is_empty() {
            for p in parts.iter() {
                average += p.wetness;
            }
            average /= parts.len() as f32;
        }
        let target = average + (current - average) * 0.1;
        for p in parts.iter_mut() {
            p.set_wetness(target);
        }
        self.stats.set(&wet, target);
        let hypothermia = self.moodle(sm, "HYPOTHERMIA")?;
        let mut delta = self.body_damage.thermoregulator.as_ref().map_or(0.0, |t| t.catch_a_cold_delta(hypothermia));
        if !self.body_damage.has_a_cold && delta > 0.1 {
            if self.has_trait(sm, "PRONE_TO_ILLNESS")? {
                delta *= 1.7;
            }
            if self.has_trait(sm, "RESILIENT")? {
                delta *= 0.45;
            }
            if self.has_trait(sm, "OUTDOORSMAN")? {
                delta *= 0.25;
            }
            let bd = &mut self.body_damage;
            bd.catch_a_cold += ctx.zomboid_globals().catch_a_cold_increase_rate as f32 * delta * ctx.multiplier();
            if bd.catch_a_cold >= 100.0 {
                bd.catch_a_cold = 0.0;
                bd.has_a_cold = true;
                bd.cold_strength = 20.0;
                bd.time_to_sneeze_or_cough = 0.0;
            }
        }
        if delta <= 0.1 {
            let bd = &mut self.body_damage;
            bd.catch_a_cold -= ctx.zomboid_globals().catch_a_cold_decrease_rate as f32;
            if bd.catch_a_cold <= 0.0 {
                bd.catch_a_cold = 0.0;
            }
        }
        Ok(())
    }

    /// `BodyDamage.TriggerSneezeCough`.
    pub fn trigger_sneeze_cough(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult {
        if self.body_damage.sneeze_cough_active > 0 {
            return Ok(());
        }
        let cold = self.moodle(sm, "HAS_A_COLD")?;
        let smoker = cold < 1 && self.has_trait(sm, "SMOKER")?;
        let bd = &mut self.body_damage;
        bd.sneeze_cough_active = if ctx.rand_next(100) > 50 && !smoker { 1 } else { 2 };
        if cold == 2 {
            bd.sneeze_cough_active = 1;
        }
        bd.sneeze_cough_time = bd.sneeze_cough_delay;
        let timer = |min: i32, max: i32| (min + ctx.rand_next(max - min)) as f32;
        if cold == 4 {
            bd.time_to_sneeze_or_cough = timer(bd.nasty_cold_sneeze_timer_min, bd.nasty_cold_sneeze_timer_max);
        } else if cold == 3 {
            bd.time_to_sneeze_or_cough = timer(bd.cold_sneeze_timer_min, bd.cold_sneeze_timer_max);
        } else if cold == 2 {
            bd.time_to_sneeze_or_cough = timer(bd.mild_cold_sneeze_timer_min, bd.mild_cold_sneeze_timer_max);
        } else if smoker {
            bd.time_to_sneeze_or_cough = timer(SMOKER_SNEEZE_TIMER_MIN, SMOKER_SNEEZE_TIMER_MAX);
        }
        let consumed = self.use_tissue(sm, ctx);
        let bd = &mut self.body_damage;
        if consumed {
            bd.sneeze_cough_active += 2;
        } else {
            let (mut radius, mut volume) = (20, 20);
            if bd.sneeze_cough_active == 1 {
                radius = 20;
                volume = 25;
            }
            if bd.sneeze_cough_active == 2 {
                radius = 35;
                volume = 40;
            }
            ctx.event(SimEvent::SneezeCoughSound { radius, volume });
        }
        if ctx.is_server() && self.is_player() {
            let s = self.body_damage.sneeze_cough_active;
            let var = if s == 1 || s == 3 { ctx.rand_next(2) + 1 } else { 0 };
            ctx.event(SimEvent::SendSneezingCoughing { sneezing_coughing: s, var });
        }
        Ok(())
    }

    /// `BodyDamage.UpdateCold`.
    pub fn update_cold(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let mult = ctx.multiplier();
        if self.body_damage.has_a_cold {
            let mut recovering = !(!self.environment.has_current_square
                || !self.environment.square_in_room
                || self.moodle(sm, "WET")? > 0
                || self.moodle(sm, "HYPOTHERMIA")? >= 1
                || self.stats.get(&stat(chars, "FATIGUE")?) > 0.5
                || self.stats.get(&stat(chars, "HUNGER")?) > 0.25
                || self.stats.get(&stat(chars, "THIRST")?) > 0.25);
            let bd = &mut self.body_damage;
            if bd.cold_reduction > 0.0 {
                recovering = true;
                bd.cold_reduction -= 0.005 * mult;
                if bd.cold_reduction < 0.0 {
                    bd.cold_reduction = 0.0;
                }
            }
            if recovering {
                let mut delta = 1.0f32;
                if self.has_trait(sm, "PRONE_TO_ILLNESS")? {
                    delta = 0.5;
                }
                if self.has_trait(sm, "RESILIENT")? {
                    delta = 1.5;
                }
                let bd = &mut self.body_damage;
                bd.cold_strength = bd.cold_strength() - bd.cold_progression_rate * delta * mult;
                if bd.cold_reduction > 0.0 {
                    bd.cold_strength = bd.cold_strength() - bd.cold_progression_rate * delta * mult;
                }
                if bd.cold_strength() < 0.0 {
                    bd.cold_strength = 0.0;
                    bd.has_a_cold = false;
                    bd.catch_a_cold = 0.0;
                }
            } else {
                let mut delta = 1.0f32;
                if self.has_trait(sm, "PRONE_TO_ILLNESS")? {
                    delta = 1.2;
                }
                if self.has_trait(sm, "RESILIENT")? {
                    delta = 0.8;
                }
                let bd = &mut self.body_damage;
                bd.cold_strength = bd.cold_strength() + bd.cold_progression_rate * delta * mult;
                if bd.cold_strength() > 100.0 {
                    bd.cold_strength = 100.0;
                }
            }
            let bd = &mut self.body_damage;
            if bd.sneeze_cough_time > 0 {
                bd.sneeze_cough_time -= 1;
                if bd.sneeze_cough_time == 0 {
                    bd.sneeze_cough_active = 0;
                }
            }
            if self.moodle(sm, "HAS_A_COLD")? > 1
                && self.body_damage.time_to_sneeze_or_cough >= 0.0
                && !self.environment.speaking
            {
                self.body_damage.time_to_sneeze_or_cough -= 1.0;
                if self.body_damage.time_to_sneeze_or_cough <= 0.0 {
                    self.trigger_sneeze_cough(sm, ctx)?;
                }
            }
        } else if self.has_trait(sm, "SMOKER")? {
            let bd = &mut self.body_damage;
            if bd.sneeze_cough_time > 0 {
                bd.sneeze_cough_time -= 1;
                if bd.sneeze_cough_time == 0 {
                    bd.sneeze_cough_active = 0;
                }
            }
            if bd.time_to_sneeze_or_cough >= 0.0 {
                if !self.environment.speaking {
                    bd.time_to_sneeze_or_cough -= ctx.game_world_seconds_since_last_update();
                    if bd.time_to_sneeze_or_cough <= 0.0 {
                        self.trigger_sneeze_cough(sm, ctx)?;
                    }
                }
            } else {
                bd.time_to_sneeze_or_cough =
                    (SMOKER_SNEEZE_TIMER_MIN + ctx.rand_next(SMOKER_SNEEZE_TIMER_MAX - SMOKER_SNEEZE_TIMER_MIN)) as f32;
            }
        }
        Ok(())
    }

    // ---- the other per-tick updates ----------------------------------------

    /// `BodyDamage.UpdateBoredom`.
    pub fn update_boredom(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.kind == super::CharacterKind::Survivor {
            return Ok(());
        }
        if self.is_player() && self.asleep {
            return Ok(());
        }
        if !self.environment.has_current_square {
            return Err(npe("getCurrentSquare()"));
        }
        let g = ctx.zomboid_globals();
        let mult = ctx.multiplier() as f64;
        let boredom = stat(chars, "BOREDOM")?;
        let idleness = stat(chars, "IDLENESS")?;
        if self.environment.square_in_room || self.idle_square_time >= 1800.0 {
            let idle = self.stats.get(&idleness) as f64;
            if self.environment.currently_idle {
                self.stats.add(&boredom, (g.boredom_increase_rate * idle * mult) as f32);
            } else {
                self.stats.add(&boredom, (g.boredom_increase_rate / 10.0 * idle * mult) as f32);
            }
            if self.environment.speaking && !self.environment.call_out {
                self.stats.remove(&boredom, (g.boredom_decrease_rate * mult) as f32);
            }
            if self.num_survivors_in_vicinity > 0 {
                self.stats.remove(&boredom, (g.boredom_decrease_rate * 0.1f32 as f64 * mult) as f32);
            }
            if self.is_currently_busy() && self.stats.get(&idleness) < 0.1 {
                self.stats.remove(&boredom, (g.boredom_decrease_rate * 0.5 * mult) as f32);
            }
        } else if self.environment.in_vehicle {
            let speed = self.environment.vehicle_speed_kmh;
            if speed.abs() <= 0.1 {
                if self.is_reading {
                    self.stats.add(&boredom, (g.boredom_increase_rate / 5.0 * mult) as f32);
                } else {
                    self.stats.add(&boredom, (g.boredom_increase_rate * mult) as f32);
                }
            } else {
                self.stats.remove(&boredom, (g.boredom_decrease_rate * 0.5 * mult) as f32);
            }
        } else {
            self.stats.remove(&boredom, (g.boredom_decrease_rate * 0.1f32 as f64 * mult) as f32);
        }
        if self.stats.get(&stat(chars, "INTOXICATION")?) > 20.0 {
            self.stats.remove(&boredom, (g.boredom_decrease_rate * 2.0 * mult) as f32);
        }
        if self.stats.get(&stat(chars, "PANIC")?) > 5.0 {
            self.stats.reset(&boredom);
        }
        let unhappy = stat(chars, "UNHAPPINESS")?;
        let bored = self.moodle(sm, "BORED")?;
        if bored > 1 && !self.is_reading {
            self.stats.add(&unhappy, (g.unhappiness_increase * bored as f64 * mult) as f32);
        }
        let stressed = self.moodle(sm, "STRESS")?;
        if stressed > 1 && !self.is_reading {
            self.stats.add(&unhappy, (g.unhappiness_increase / 2.0 * stressed as f64 * mult) as f32);
        }
        if self.has_trait(sm, "SMOKER")? {
            self.time_since_last_smoke += 1.0e-4 * ctx.multiplier();
            if self.time_since_last_smoke > 1.0 {
                let mut last = (fastfloor(self.time_since_last_smoke / 10.0) as f32 + 1.0) as f64;
                if last > 10.0 {
                    last = 10.0;
                }
                self.stats.add(
                    &stat(chars, "NICOTINE_WITHDRAWAL")?,
                    (g.stress_from_bite_or_scratch / 8.0 * last * mult) as f32,
                );
            }
        }
        Ok(())
    }

    /// `BodyDamage.UpdateStrength`.
    pub fn update_strength(&mut self, sm: &ScriptManager, chars: &Characters) -> ScriptResult {
        let mut n = 0;
        let table: [(&str, [i32; 3]); 5] = [
            ("HUNGRY", [1, 2, 2]),
            ("THIRST", [1, 2, 2]),
            ("SICK", [1, 2, 3]),
            ("BLEEDING", [1, 1, 1]),
            ("INJURED", [1, 2, 3]),
        ];
        for (m, add) in table {
            let l = self.moodle(sm, m)?;
            if (2..=4).contains(&l) {
                n += add[(l - 2) as usize];
            }
        }
        self.max_weight = (self.max_weight_base as f32 * self.weight_mod(chars)) as i32 - n;
        if self.max_weight < 0 {
            self.max_weight = 0;
        }
        if self.is_player() {
            self.max_weight = (self.max_weight as f32 * self.max_weight_delta) as i32;
        }
        Ok(())
    }

    /// `BodyDamage.UpdateTemperatureState`.
    pub fn update_temperature_state(&mut self, chars: &Characters, ctx: &dyn SimContext) {
        if self.is_player() {
            let stage = self.body_damage.cold_damage_stage;
            if stage > 0.0 {
                let max = 100.0 - stage * 100.0;
                if max <= 0.0 {
                    self.set_health(ctx, 0.0);
                    return;
                }
                if self.body_damage.overall_body_health > max {
                    let v = self.body_damage.overall_body_health - max;
                    self.reduce_general_health(chars, v);
                }
            }
            self.move_speed = 0.06;
        }
    }

    /// `BodyDamage.UpdateDiscomfort`.
    pub fn update_discomfort(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let dragging = if self.environment.dragging_corpse { 0.3f32 } else { 0.0 };
        let clothing = self.clothing_discomfort_modifier;
        let mut bed = 0.0f32;
        if self.asleep {
            bed = match self.bed_type.as_str() {
                "badBed" => 0.3,
                "badBedPillow" => 0.2,
                "floor" => 0.5,
                "floorPillow" => 0.4,
                _ => 0.0,
            };
        }
        let discomfort = stat(chars, "DISCOMFORT")?;
        let drunk = 1.0 - 0.5 * (self.stats.get(&stat(chars, "INTOXICATION")?) / 100.0);
        let hypo = 0.1 * self.moodle(sm, "HYPOTHERMIA")? as f32;
        let hyper = 0.1 * self.moodle(sm, "HYPERTHERMIA")? as f32;
        let wet = 0.1 * self.moodle(sm, "WET")? as f32;
        let vehicle = if self.environment.in_vehicle {
            (self.moodle(sm, "HEAVY_LOAD")? as f64 * ctx.zomboid_globals().vehicle_discomfort_when_over_encumbered)
                as f32
        } else {
            0.0
        };
        let mut m = 0.0f32;
        m += bed;
        m += clothing;
        m += dragging;
        m += hypo;
        m += hyper;
        m += wet;
        m += vehicle;
        m *= drunk;
        let target = clamp_f32(m, 0.0, 1.0) * 100.0;
        let mut step = 0.005 * ctx.multiplier();
        if target > self.stats.get(&discomfort) {
            step *= 0.025;
        }
        if self.asleep {
            self.stats.set(&discomfort, target);
        } else {
            let d = self.stats.get(&discomfort);
            // `!PZMath.equal(d, target, step)`.
            if (target - d).abs() >= step || (target - d).is_nan() {
                self.stats.set(&discomfort, d + (target - d) * step);
            } else if d != target {
                self.stats.set(&discomfort, target);
            }
        }
        let stress = stat(chars, "STRESS")?;
        if self.moodle(sm, "UNCOMFORTABLE")? >= 1 && self.stats.get(&stress) < stress.maximum_value() {
            let malus = if m > 1.0 { 3.0 + (m - 1.0) * 9.0 } else { 3.0f32 };
            let v = ctx.zomboid_globals().stress_from_discomfort
                * (self.stats.get(&discomfort) * malus) as f64
                * ctx.multiplier() as f64
                * ctx.delta_minutes_per_day() as f64;
            self.stats.add(&stress, v as f32);
        }
        Ok(())
    }

    /// `BodyDamage.getSicknessFromCorpsesRate`.
    pub fn sickness_from_corpses_rate(ctx: &dyn SimContext, corpse_count: i32) -> f32 {
        let impact = ctx.decaying_corpse_health_impact();
        if impact == 1 {
            return 0.0;
        }
        if corpse_count > 5 {
            let d = ctx.zomboid_globals().food_sickness_decrease as f32;
            let inc = match impact {
                2 => d * 0.01,
                4 => d * 0.11,
                5 => d,
                _ => d * 0.07,
            };
            let cap = (corpse_count - 5).min(ctx.flies_max_corpse_count() - 5);
            return inc * cap as f32;
        }
        0.0
    }

    /// `BodyDamage.GetBaseCorpseSickness`.
    pub fn base_corpse_sickness(&self, ctx: &dyn SimContext) -> f32 {
        Self::sickness_from_corpses_rate(ctx, self.environment.corpse_count)
    }

    /// `BodyDamage.UpdateIllness`.
    pub fn update_illness(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let food_sickness = stat(chars, "FOOD_SICKNESS")?;
        if ctx.decaying_corpse_health_impact() != 1 {
            let mut rate = self.base_corpse_sickness(ctx);
            if rate > 0.0 {
                let defense = self.items.corpse_sickness_defense(sm, ctx, rate, true)?;
                if defense > 0.0 {
                    let m = 0.0f32.max(1.0 - defense / 100.0);
                    rate *= m;
                }
                if self.has_trait(sm, "RESILIENT")? {
                    rate *= 0.75;
                } else if self.has_trait(sm, "PRONE_TO_ILLNESS")? {
                    rate *= 1.25;
                }
                if rate > 0.0 {
                    self.stats.add(&food_sickness, rate * ctx.multiplier());
                    self.corpse_sickness_rate = rate;
                    return Ok(());
                }
            }
        }
        self.corpse_sickness_rate = 0.0;
        if self.stats.is_at_minimum(&stat(chars, "POISON")?) && self.stats.is_above_minimum(&food_sickness) {
            let v = ctx.zomboid_globals().food_sickness_decrease as f32 * ctx.multiplier();
            self.stats.remove(&food_sickness, v);
        }
        Ok(())
    }

    /// `BodyDamage.pickMortalityDuration`.
    pub fn pick_mortality_duration(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult<f32> {
        let mut del = 1.0f32;
        if self.has_trait(sm, "RESILIENT")? {
            del = 1.25;
        }
        if self.has_trait(sm, "PRONE_TO_ILLNESS")? {
            del = 0.75;
        }
        Ok(match ctx.lore_mortality() {
            1 => 0.0,
            2 => ctx.rand_next_float(0.0, 30.0) / 3600.0 * del,
            3 => ctx.rand_next_float(0.5, 1.0) / 60.0 * del,
            4 => ctx.rand_next_float(3.0, 12.0) * del,
            5 => ctx.rand_next_float(2.0, 3.0) * 24.0 * del,
            6 => ctx.rand_next_float(1.0, 2.0) * 7.0 * 24.0 * del,
            _ => -1.0,
        })
    }

    /// `BodyDamage.getCurrentTimeForInfection`.
    fn current_time_for_infection(&self, ctx: &dyn SimContext) -> f32 {
        if self.is_player() {
            self.hours_survived(ctx) as f32
        } else {
            ctx.world_age_hours() as f32
        }
    }

    // ---- BodyPart.DamageUpdate ---------------------------------------------

    /// `BodyPart.DamageUpdate` of part `i`.
    pub fn body_part_damage_update(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        i: usize,
    ) -> ScriptResult {
        if ctx.is_client() && self.is_local_player(ctx) {
            return Ok(());
        }
        let mult = ctx.multiplier();
        let invulnerable = self.is_invulnerable(ctx);
        // `CombatManager.applyDamage(part, damage)`.
        let apply = |p: &mut BodyPart, d: f32| {
            if !invulnerable {
                p.reduce_health(d);
            }
        };
        let p = &mut self.body_damage.parts[i];
        let s = p.damage_scaler;
        if p.deep_wound_time > 0.0 && !p.stitched {
            let d = if p.bandaged { 1.5625 * s * mult } else { 3.125 * s * mult };
            apply(p, d);
        }
        if p.scratch_time > 0.0 && !p.bandaged {
            apply(p, 0.9375 * s * mult);
        }
        if p.cut_time > 0.0 && !p.bandaged {
            apply(p, 1.875 * s * mult);
        }
        if p.bite_time > 0.0 && !p.bandaged {
            apply(p, 2.1875 * s * mult);
        }
        if p.bleeding_time > 0.0 && !p.bandaged {
            let d = 0.285_714_3 * s * mult * (p.bleeding_time / 10.0);
            self.reduce_general_health(chars, d);
            ctx.event(SimEvent::PlayerGetDamage { kind: "BLEEDING", amount: d });
            if next_bool(ctx, adjust_for_framerate(ctx, 1000)) {
                ctx.event(SimEvent::AddBlood { part: i as i32, scratched: false, bitten: false, all_layers: true });
            }
        }
        let p = &mut self.body_damage.parts[i];
        if p.have_bullet {
            let d = if p.bandaged { 1.5625 * s * mult } else { 3.125 * s * mult };
            apply(p, d);
        }
        if p.burn_time > 0.0 && !p.bandaged {
            apply(p, 3.75 * s * mult);
        }
        if p.fracture_time > 0.0 && !p.splint {
            apply(p, 3.125 * s * mult);
        }
        let dirty = self.items.has_dirty_clothing(sm, i as i32);
        let bloody = self.items.has_bloody_clothing(sm, i as i32);
        let stiffness = if self.is_player() { self.fitness.as_ref().map(|f| f.on_going_stiffness()) } else { None };
        let mut rip = self.reduce_infection_power;
        self.body_damage.parts[i].heal_update(ctx, &mut rip, dirty, bloody, stiffness);
        self.reduce_infection_power = rip;
        Ok(())
    }

    // ---- Update ------------------------------------------------------------

    /// `BodyDamage.Update`.
    pub fn body_damage_update(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.is_zombie() || self.is_animal() {
            return Ok(());
        }
        if ctx.is_client() && self.is_player() && !self.is_dead() {
            if !self.is_local_player(ctx) {
                self.restore_to_full_health(chars)?;
            }
            return Ok(());
        }
        if self.is_god_mod(ctx) {
            self.restore_to_full_health(chars)?;
            if !self.is_player() {
                return Err(class_cast());
            }
            self.bleeding_level = 0;
            return Ok(());
        }
        let types = &chars.sim.body_part_types;
        let max = self.body_damage.parts.len();
        let mult = ctx.multiplier();
        let pain = stat(chars, "PAIN")?;
        let last_pain = self.stats.get(&pain);
        let bd = &self.body_damage;
        let mut n = bd.num_parts_bleeding() * 2;
        n += bd.num_parts_scratched();
        // The bytecode adds the bites before the test (the decompiled
        // `getHealth() >= 60 && (n += bitten * 6) <= 3` reads otherwise).
        n += bd.num_parts_bitten() * 6;
        if bd.health() >= 60.0 && n <= 3 {
            n = 0;
        }
        if !self.is_player() {
            return Err(class_cast());
        }
        self.bleeding_level = n as i8;
        if n > 0 {
            let chance = 1.0 / n as f32 * 200.0 * ctx.inv_multiplier();
            if (ctx.rand_next(chance as i32) as f32) < chance * 0.3 {
                ctx.event(SimEvent::SplatBloodFloor);
            }
            if ctx.rand_next(chance as i32) == 0 {
                ctx.event(SimEvent::SplatBloodFloor);
            }
        }
        self.thermoregulator_update(sm, chars, ctx)?;
        self.update_dragging_corpse();
        self.update_wetness(sm, chars, ctx)?;
        self.update_cold(sm, chars, ctx)?;
        self.update_boredom(sm, chars, ctx)?;
        self.update_strength(sm, chars)?;
        self.update_panic_state(sm, chars, ctx)?;
        self.update_temperature_state(chars, ctx);
        self.update_discomfort(sm, chars, ctx)?;
        self.update_illness(sm, chars, ctx)?;
        if self.body_damage.overall_body_health == 0.0 {
            return Ok(());
        }
        let fever = stat(chars, "ZOMBIE_FEVER")?;
        let infection = stat(chars, "ZOMBIE_INFECTION")?;
        if !self.body_damage.is_infected {
            for i in 0..max {
                if !self.body_damage.parts[i].is_infected {
                    continue;
                }
                self.body_damage.is_infected = true;
                if !self.body_damage.parts[i].is_fake_infected {
                    continue;
                }
                self.body_damage.parts[i].disable_fake_infection();
                let f = self.stats.get(&fever);
                self.stats.set(&infection, f);
                self.stats.reset(&fever);
                self.body_damage.set_is_fake_infected(false);
                self.body_damage.reduce_fake_infection = false;
            }
            if self.body_damage.is_infected && self.body_damage.infection_time < 0.0 && ctx.lore_mortality() != 7 {
                self.body_damage.infection_time = self.current_time_for_infection(ctx);
                self.body_damage.infection_mortality_duration = self.pick_mortality_duration(sm, ctx)?;
            }
        }
        if !self.body_damage.is_infected
            && !self.body_damage.is_fake_infected
            && self.body_damage.parts.iter().any(|p| p.is_fake_infected)
        {
            self.body_damage.set_is_fake_infected(true);
        }
        let growth = self.body_damage.infection_growth_rate;
        if self.body_damage.is_fake_infected
            && !self.body_damage.reduce_fake_infection
            && self.reduce_infection_power == 0.0
        {
            self.stats.add(&fever, growth * mult);
            if self.stats.is_at_maximum(&fever) {
                self.body_damage.reduce_fake_infection = true;
            }
        }
        self.stats.remove(&stat(chars, "INTOXICATION")?, self.body_damage.drunk_reduction_value * mult);
        let mut health_to_add = 0.0f32;
        if self.body_damage.health_from_food_timer > 0.0 {
            health_to_add += self.body_damage.health_from_food * mult;
            self.body_damage.health_from_food_timer -= 1.0 * mult;
        }
        let hungry = self.moodle(sm, "HUNGRY")?;
        let sick = self.moodle(sm, "SICK")?;
        let thirst = self.moodle(sm, "THIRST")?;
        let mut reduced = 0;
        if hungry == 2 || sick == 2 || thirst == 2 {
            reduced = 1;
        }
        if hungry == 3 || sick == 3 || thirst == 3 {
            reduced = 2;
        }
        if hungry == 4 || thirst == 4 {
            reduced = 3;
        }
        if self.asleep {
            reduced = -1;
        }
        let bd = &self.body_damage;
        match reduced {
            0 => health_to_add += bd.standard_health_addition * mult,
            1 => health_to_add += bd.reduced_health_addition * mult,
            2 => health_to_add += bd.severly_reduced_health_addition * mult,
            3 => health_to_add += 0.0,
            _ => {}
        }
        if self.asleep {
            if ctx.is_client() {
                health_to_add += 15.0 * ctx.game_world_seconds_since_last_update() / 3600.0;
            } else {
                health_to_add += bd.sleeping_health_addition * mult;
            }
            if hungry == 4 || thirst == 4 {
                health_to_add = 0.0;
            }
        }
        self.body_damage.add_general_health(health_to_add);
        let mut reduction = 0.0f32;
        let mut poison_damage = 0.0f32;
        let mut hungry_damage = 0.0f32;
        let mut sick_damage = 0.0f32;
        let mut bleeding_damage = 0.0f32;
        let mut thirst_damage = 0.0f32;
        let mut heavy_damage = 0.0f32;
        let severe = self.body_damage.health_reduction_from_severe_bad_moodles;
        let poison = stat(chars, "POISON")?;
        let food_sickness = stat(chars, "FOOD_SICKNESS")?;
        let pv = self.stats.get(&poison);
        if pv > 0.0 {
            if pv > 10.0 && sick >= 1 {
                poison_damage = 0.0035 * java_min(pv / 10.0, 3.0) * mult;
                reduction += poison_damage;
            }
            let mut well_fed = 0.0f32;
            let fed = self.moodle(sm, "FOOD_EATEN")?;
            if fed > 0 {
                well_fed = 1.5e-4 * fed as f32;
            }
            let v = (well_fed as f64 + ctx.zomboid_globals().poison_level_decrease * mult as f64) as f32;
            self.stats.remove(&poison, v);
            let r = java_round(self.stats.get(&poison) / 10.0) as f32;
            self.stats.add(&food_sickness, growth * (2.0 + r) * mult);
        }
        if hungry == 4 {
            hungry_damage = severe / 50.0 * mult;
            reduction += hungry_damage;
        }
        if sick == 4
            && (self.stats.get(&food_sickness) > self.stats.get(&infection)
                || ctx.wound_infection_factor() > 0.0
                    && self.body_damage.general_wound_infection_level(ctx) > self.stats.get(&infection))
        {
            sick_damage = severe * mult;
            reduction += sick_damage;
        }
        if self.moodle(sm, "BLEEDING")? == 4 {
            bleeding_damage = severe * mult;
            reduction += bleeding_damage;
        }
        if thirst == 4 {
            thirst_damage = severe / 10.0 * mult;
            reduction += thirst_damage;
        }
        let heavy = self.moodle(sm, "HEAVY_LOAD")?;
        let heavy_hurts = || -> ScriptResult<bool> {
            if heavy <= 2 || self.environment.in_vehicle || self.asleep || self.is_sitting() {
                return Ok(false);
            }
            let t = self.body_damage.thermoregulator.as_ref().ok_or_else(|| npe("getThermoregulator()"))?;
            let seated = sm
                .builtin_tables
                .borrow()
                .metabolics
                .iter()
                .find(|(n, _)| n == "SeatedResting")
                .map(|(_, m)| *m)
                .ok_or_else(|| ScriptError::new("Metabolics.SeatedResting is not a constant"))?;
            if t.metabolic_target() == seated
                || ctx.is_server() && ctx.server_fast_forward()
                || self.body_damage.health() <= 75.0
            {
                return Ok(false);
            }
            Ok(per_thirtieth_of_a_second(ctx, 10))
        };
        if heavy_hurts()? {
            heavy_damage = severe / ((5 - heavy) as f32 / 10.0) * mult;
            reduction += heavy_damage;
            self.add_back_muscle_strain(chars, ctx, heavy_damage / 2.0)?;
        }
        self.reduce_general_health(chars, reduction);
        for (kind, d) in [
            ("POISON", poison_damage),
            ("HUNGRY", hungry_damage),
            ("SICK", sick_damage),
            ("BLEEDING", bleeding_damage),
            ("THIRST", thirst_damage),
            ("HEAVYLOAD", heavy_damage),
        ] {
            if d > 0.0 {
                ctx.event(SimEvent::PlayerGetDamage { kind, amount: d });
            }
        }
        let fps = ctx.thirty_fps_multiplier();
        if self.pain_effect > 0.0 {
            self.stats.remove(&pain, 0.023_333_333 * fps);
            self.pain_effect -= fps;
        } else {
            self.pain_delta = 0.0;
            let mut total = 0.0f32;
            for (i, p) in self.body_damage.parts.iter().enumerate() {
                total += p.pain(ctx) * types.pain_modifier(i as i32);
            }
            total -= self.body_damage.pain_reduction;
            let cur = self.stats.get(&pain);
            if total > cur {
                self.stats.add(&pain, (total - cur) / 500.0);
            } else {
                self.stats.set(&pain, total);
            }
        }
        self.body_damage.pain_reduction -= 0.005 * mult;
        if self.body_damage.pain_reduction < 0.0 {
            self.body_damage.pain_reduction = 0.0;
        }
        if self.body_damage.is_infected {
            let mortality = ctx.lore_mortality();
            if mortality == 1 {
                self.reduce_general_health(chars, 110.0);
                ctx.event(SimEvent::PlayerGetDamage { kind: "INFECTION", amount: 110.0 });
                self.stats.set(&infection, infection.maximum_value());
            } else if mortality != 7 {
                let now = self.current_time_for_infection(ctx);
                if self.body_damage.infection_mortality_duration < 0.0 {
                    self.body_damage.infection_mortality_duration = self.pick_mortality_duration(sm, ctx)?;
                }
                let bd = &mut self.body_damage;
                // `GameTime.checkHours(infectionTime, now)`.
                bd.infection_time =
                    if bd.infection_time < 0.0 || bd.infection_time > now { now } else { bd.infection_time };
                let mut percent = java_min((now - bd.infection_time) / bd.infection_mortality_duration, 1.0);
                self.stats.set(&infection, percent * 100.0);
                if percent == 1.0 {
                    self.reduce_general_health(chars, 110.0);
                    ctx.event(SimEvent::PlayerGetDamage { kind: "INFECTION", amount: 110.0 });
                } else {
                    percent *= percent;
                    percent *= percent;
                    let max_health = (1.0 - percent) * 100.0;
                    let excess = self.body_damage.overall_body_health - max_health;
                    if excess > 0.0 && max_health <= 99.0 {
                        self.reduce_general_health(chars, excess);
                        ctx.event(SimEvent::PlayerGetDamage { kind: "INFECTION", amount: excess });
                    }
                }
            }
        }
        for i in 0..max {
            self.body_part_damage_update(sm, chars, ctx, i)?;
        }
        self.calculate_overall_health(chars);
        if self.body_damage.overall_body_health <= 0.0 {
            if self.body_damage.is_on_fire {
                self.body_damage.burnt_to_death = true;
                for i in 0..max {
                    let h = ctx.rand_next(90) as f32;
                    self.body_damage.parts[i].set_health(h);
                }
            } else {
                self.body_damage.burnt_to_death = false;
            }
        }
        if self.body_damage.reduce_fake_infection && self.body_damage.overall_body_health > 0.0 {
            self.stats.remove(&fever, growth * mult * 2.0);
        }
        if self.reduce_infection_power > 0.0 && self.body_damage.overall_body_health > 0.0 {
            self.stats.remove(&fever, growth * mult);
            self.reduce_infection_power -= growth * mult;
            if self.reduce_infection_power < 0.0 {
                self.reduce_infection_power = 0.0;
            }
        }
        if self.stats.get(&fever) <= 0.0 {
            for p in &mut self.body_damage.parts {
                p.is_fake_infected = false;
            }
            self.body_damage.set_is_fake_infected(false);
            self.stats.reset(&fever);
            self.body_damage.reduce_fake_infection = false;
        }
        if last_pain == self.stats.get(&pain) {
            self.stats.remove(&pain, 0.25 * fps);
        }
        Ok(())
    }

    // ---- injury entry points -------------------------------------------------

    /// `BodyDamage.AddDamage(index, val)`.
    pub fn body_damage_add_damage(&mut self, index: usize, val: f32) -> ScriptResult {
        let n = self.body_damage.parts.len();
        let p = self.body_damage.parts.get_mut(index).ok_or_else(|| {
            ScriptError::new(format!("java.lang.IndexOutOfBoundsException: Index {index} out of bounds for length {n}"))
        })?;
        p.add_damage(val);
        Ok(())
    }

    /// The slot of `getBodyPart(BodyPartType.FromIndex(index))` (an error
    /// for `MAX`, which has no part).
    pub fn part_slot(&self, chars: &Characters, index: i32) -> ScriptResult<usize> {
        let field = chars.sim.body_part_types.field_of(index);
        let i = chars.sim.body_part_types.index(field)?;
        if i < self.body_damage.parts.len() {
            Ok(i)
        } else {
            Err(ScriptError::new(format!(
                "java.lang.IndexOutOfBoundsException: Index {i} out of bounds for length {}",
                self.body_damage.parts.len()
            )))
        }
    }

    /// `BodyDamage.applyDamageFromWeapon`.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_damage_from_weapon(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        part_index: i32,
        damage: f32,
        damage_type: i32,
        pain: f32,
    ) -> ScriptResult {
        let i = self.part_slot(chars, part_index)?;
        let env = self.injury_env(sm, ctx)?;
        let p = &mut self.body_damage.parts[i];
        match damage_type {
            1 => p.generate_deep_wound(&env),
            2 | 4 => p.set_cut(&env, true, true),
            3 | 5 => p.set_scratched(&env, true, true),
            6 => p.set_have_bullet(&env, true, 0),
            _ => {}
        }
        self.body_damage_add_damage(part_index as usize, damage)?;
        self.stats.add(&stat(chars, "PAIN")?, pain);
        if ctx.is_server() {
            ctx.event(SimEvent::SyncDamage);
        }
        Ok(())
    }

    /// `BodyDamage.isSpikedPart(owner, this, part)`.
    fn is_spiked_part(&self, sm: &ScriptManager, owner: &WeaponUser<'_>, part_index: i32) -> bool {
        let behind = if !owner.aim_at_floor { owner.behind } else { self.environment.fall_on_front };
        if behind {
            self.items.body_part_is_spiked_behind(sm, part_index)
        } else {
            self.items.body_part_is_spiked(sm, part_index)
        }
    }

    /// `BodyDamage.DamageFromWeapon(weapon, part)`; `owner` is
    /// `weapon.getUsingPlayer()`. The owner's `spikePart` calls are the
    /// owner's.
    pub fn damage_from_weapon(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        weapon: &InventoryItem,
        owner: Option<&WeaponUser<'_>>,
        mut part_index: i32,
    ) -> ScriptResult {
        if ctx.is_client() && self.is_player() && !self.is_local_player(ctx) {
            return Ok(());
        }
        let types = &chars.sim.body_part_types;
        let (mut blunt, mut blade, mut bullet) = (false, false, false);
        if is_of_weapon_category(sm, weapon, "BLUNT") || is_of_weapon_category(sm, weapon, "SMALL_BLUNT") {
            blunt = true;
        } else if !weapon.is_aimed_firearm() {
            blade = true;
        } else {
            bullet = true;
        }
        if part_index == -1 {
            part_index = ctx.rand_next_range(types.index("Hand_L")? as i32, types.max_index() as i32);
        }
        let slot = self.part_slot(chars, part_index)?;
        let def = self.items.body_part_clothing_defense(sm, part_index, blade, bullet);
        if (ctx.rand_next(100) as f32) < def {
            if owner.is_some_and(|o| {
                is_knife(weapon) && !has_tag(sm, weapon, "HANDGUARD") && self.is_spiked_part(sm, o, part_index)
            }) {
                ctx.event(SimEvent::AddBlood { part: part_index, scratched: true, bitten: false, all_layers: false });
            }
            ctx.event(SimEvent::AddHoleFromZombieAttacks { part: part_index, scratch_or_bite: false });
            ctx.event(SimEvent::WeaponHitArmourSound { part: part_index, bullet });
            return Ok(());
        }
        ctx.event(SimEvent::AddHole { part: part_index, all_layers: false });
        for _ in 0..3 {
            ctx.event(SimEvent::SplatBloodFloorBig);
        }
        let env = self.injury_env(sm, ctx)?;
        let bd = &mut self.body_damage;
        let mut damage_type = 0;
        let mut pain = 0.0f32;
        let p = &mut bd.parts[slot];
        if blade {
            if next_bool(ctx, 6) {
                damage_type = 1;
                p.generate_deep_wound(&env);
            } else if next_bool(ctx, 3) {
                damage_type = 2;
                p.set_cut(&env, true, true);
            } else {
                damage_type = 3;
                p.set_scratched(&env, true, true);
            }
            pain = bd.initial_scratch_pain * types.pain_modifier(part_index);
        } else if blunt {
            if next_bool(ctx, 4) {
                damage_type = 4;
                p.set_cut(&env, true, true);
            } else {
                damage_type = 5;
                p.set_scratched(&env, true, true);
            }
            pain = bd.initial_thump_pain * types.pain_modifier(part_index);
        } else if bullet {
            damage_type = 6;
            p.set_have_bullet(&env, true, 0);
            pain = bd.initial_bite_pain * types.pain_modifier(part_index);
        }
        let mut damage = ctx.rand_next_float(weapon.get_min_damage(), weapon.get_max_damage()) * 15.0;
        if part_index == types.index("Head")? as i32 {
            damage *= 4.0;
        }
        if part_index == types.index("Neck")? as i32 {
            damage *= 4.0;
        }
        if part_index == types.index("Torso_Upper")? as i32 {
            damage *= 2.0;
        }
        if ctx.is_server() {
            let m = if weapon.is_ranged() { ctx.pvp_firearm_damage_modifier() } else { ctx.pvp_melee_damage_modifier() };
            damage = (damage as f64 * m) as f32;
        }
        // `damageFromSpikedArmor(owner, this, part, weapon)`.
        if let Some(o) = owner.filter(|o| (o.do_shove || is_knife(weapon)) && self.is_spiked_part(sm, o, part_index)) {
            let free = |i: Option<&InventoryItem>| i.is_none_or(is_hand_weapon);
            let foot = o.aim_at_floor && o.do_shove;
            let primary = !foot && free(o.primary_hand_item);
            let secondary = !foot && free(o.secondary_hand_item) && o.do_shove;
            for hit in [foot, primary, secondary] {
                if hit {
                    ctx.event(SimEvent::AddBlood {
                        part: part_index,
                        scratched: true,
                        bitten: false,
                        all_layers: false,
                    });
                }
            }
        }
        self.apply_damage_from_weapon(sm, chars, ctx, part_index, damage, damage_type, pain)?;
        ctx.event(SimEvent::WeaponHitArmourSound { part: part_index, bullet });
        Ok(())
    }

    /// `BodyDamage.setScratchedWindow`: the scratched part's index (`None`
    /// on a client).
    pub fn set_scratched_window(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult<Option<usize>> {
        if ctx.is_client() {
            return Ok(None);
        }
        let types = &chars.sim.body_part_types;
        let i = ctx.rand_next_range(types.index("Hand_L")? as i32, types.index("ForeArm_R")? as i32 + 1);
        let slot = self.part_slot(chars, i)?;
        let env = self.injury_env(sm, ctx)?;
        let p = &mut self.body_damage.parts[slot];
        p.add_damage(10.0);
        p.set_scratched_window(&env, true);
        Ok(Some(slot))
    }

    /// `BodyDamage.AddRandomDamage`.
    pub fn add_random_damage(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult {
        let env = self.injury_env(sm, ctx)?;
        let n = self.body_damage.parts.len() as i32;
        let p = &mut self.body_damage.parts[ctx.rand_next(n) as usize];
        match ctx.rand_next(4) {
            0 => {
                p.generate_deep_wound(&env);
                if ctx.rand_next(4) == 0 {
                    p.infected_wound = true;
                }
            }
            1 => {
                p.generate_deep_shard_wound(&env);
                if ctx.rand_next(4) == 0 {
                    p.infected_wound = true;
                }
            }
            2 => p.fracture_time = ctx.rand_next_range(30, 50) as f32,
            3 => p.burn_time = ctx.rand_next_range(30, 50) as f32,
            _ => {}
        }
        Ok(())
    }

    /// `BodyDamage.AddRandomDamageFromZombie`.
    pub fn add_random_damage_from_zombie(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        z: &ZombieAttack,
    ) -> ScriptResult<ZombieAttackOutcome> {
        let types = &chars.sim.body_part_types;
        let idx = |f: &str| types.index(f).map(|i| i as i32);
        let hit_reaction = z.hit_reaction.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "Bite".to_string());
        let mut out = ZombieAttackOutcome::default();
        let mut pain_type = 0;
        let mut base = 15 + z.melee_combat_mod;
        let mut bite = 85;
        let mut lacer = 65;
        let behind = z.dot_side == "BEHIND";
        let side = z.dot_side == "LEFT" || z.dot_side == "RIGHT";
        let attacking = z.surrounding_attacking_zombies.max(1);
        base -= (attacking - 1) * 10;
        bite -= (attacking - 1) * 30;
        lacer -= (attacking - 1) * 15;
        let mut needed = 3;
        if ctx.lore_strength() == 1 {
            needed = 2;
        }
        if ctx.lore_strength() == 3 {
            needed = 6;
        }
        if self.has_trait(sm, "THICK_SKINNED")? {
            base = (base as f64 * 1.3) as i32;
        }
        if self.has_trait(sm, "THIN_SKINNED")? {
            base = (base as f64 / 1.3) as i32;
        }
        if z.current_hit_reaction != "EndDeath" {
            if !self.is_god_mod(ctx)
                && z.drag_down_zombies >= needed
                && ctx.lore_zombies_drag_down()
                && !self.sit_on_ground
            {
                bite = 0;
                lacer = 0;
                base = 0;
                out.hit_reaction = Some("EndDeath".to_string());
                out.death_drag_down = true;
            } else {
                out.hit_reaction = Some(hit_reaction.clone());
            }
        }
        let rear = ctx.rear_vulnerability();
        if behind {
            base -= 15;
            bite -= 25;
            lacer -= 35;
            if rear == 1 {
                base += 15;
                bite += 25;
                lacer += 35;
            }
            if rear == 2 {
                base += 7;
                bite += 17;
                lacer += 23;
            }
            if attacking > 2 {
                bite -= 15;
                lacer -= 15;
            }
        }
        if side {
            base -= 30;
            bite -= 7;
            lacer -= 27;
            if rear == 1 {
                base += 30;
                bite += 7;
                lacer += 27;
            }
            if rear == 2 {
                base += 15;
                bite += 4;
                lacer += 15;
            }
        }
        let mut part = z.part_index;
        if part < 0 {
            if !z.crawling {
                part = if ctx.rand_next(10) == 0 {
                    ctx.rand_next_range(idx("Hand_L")?, idx("Groin")? + 1)
                } else {
                    ctx.rand_next_range(idx("Hand_L")?, idx("Neck")? + 1)
                };
                let mut neck_chance = 10.0 * attacking as f32;
                if behind {
                    neck_chance += 5.0;
                }
                if side {
                    neck_chance += 2.0;
                }
                if behind && (ctx.rand_next(100) as f32) < neck_chance {
                    part = idx("Neck")?;
                }
                if part == idx("Head")? || part == idx("Neck")? {
                    let mut percent = 70;
                    if behind {
                        percent = 90;
                    }
                    if side {
                        percent = 80;
                    }
                    if ctx.rand_next(100) > percent {
                        loop {
                            part = ctx.rand_next(idx("Torso_Lower")? + 1);
                            if part != idx("Head")? && part != idx("Neck")? && part != idx("Groin")? {
                                break;
                            }
                        }
                    }
                }
            } else if ctx.rand_next(2) == 0 {
                part = if ctx.rand_next(10) == 0 {
                    ctx.rand_next_range(idx("Groin")?, types.max_index() as i32)
                } else {
                    ctx.rand_next_range(idx("UpperLeg_L")?, types.max_index() as i32)
                };
            } else {
                out.part_index = part;
                return Ok(out);
            }
        }
        out.part_index = part;
        if z.inactive {
            base += 20;
            bite += 20;
            lacer += 20;
        }
        let mut damage = ctx.rand_next(1000) as f32 / 1000.0;
        damage *= (ctx.rand_next(10) + 10) as f32;
        let mut hole_done = false;
        let mut scratch_or_bite = true;
        let behind_hit = behind || self.environment.fall_on_front;
        let player = self.is_player();
        if ctx.rand_next(100) > base {
            let spiked = if behind_hit {
                self.items.body_part_is_spiked_behind(sm, part)
            } else {
                self.items.body_part_is_spiked(sm, part)
            };
            out.scratch = true;
            out.helmet_fall = Some(part == idx("Neck")? || part == idx("Head")?);
            if ctx.rand_next(100) > lacer {
                out.scratch = false;
                out.laceration = true;
            }
            if ctx.rand_next(100) > bite && !z.cant_bite {
                out.scratch = false;
                out.laceration = false;
                scratch_or_bite = false;
            }
            let alive = self.body_damage.health() > 0.0;
            if out.scratch || out.laceration {
                let defense = self.items.body_part_clothing_defense(sm, part, false, false);
                out.zombie_state = Some(if out.scratch { "AttackScratch" } else { "AttackLacerate" });
                if alive {
                    ctx.event(SimEvent::PlaySound("ZombieScratch".to_string()));
                }
                if alive && spiked {
                    let hand = if next_bool(ctx, 2) { "Hand_L" } else { "Hand_R" };
                    ctx.event(SimEvent::AddBlood { part, scratched: true, bitten: false, all_layers: false });
                    out.zombie_spiked.push(hand);
                }
                if (ctx.rand_next(100) as f32) < defense {
                    ctx.event(SimEvent::AddHoleFromZombieAttacks { part, scratch_or_bite });
                    return Ok(out);
                }
                ctx.event(SimEvent::AddHole { part, all_layers: true });
                hole_done = true;
                pain_type = 1;
                if player {
                    let voice = if out.scratch { "PainFromScratch" } else { "PainFromLacerate" };
                    ctx.event(SimEvent::PlayerVoiceSound(voice));
                    if ctx.is_client() {
                        out.sent_zombie_hit = true;
                        out.hit = true;
                        return Ok(out);
                    }
                }
                self.body_damage_add_damage(part as usize, damage)?;
                let env = self.injury_env(sm, ctx)?;
                let p = &mut self.body_damage.parts[part as usize];
                if out.scratch {
                    p.set_scratched(&env, true, false);
                } else {
                    p.set_cut(&env, true, false);
                }
                ctx.event(SimEvent::AddBlood { part, scratched: true, bitten: false, all_layers: true });
            } else {
                let defense = self.items.body_part_clothing_defense(sm, part, true, false);
                out.zombie_state = Some("AttackBite");
                if alive {
                    let sound = if part == idx("Neck")? { "NeckBite".to_string() } else { z.bite_sound.clone() };
                    ctx.event(SimEvent::PlaySound(sound));
                }
                if (ctx.rand_next(100) as f32) < defense {
                    ctx.event(SimEvent::AddHoleFromZombieAttacks { part, scratch_or_bite });
                    if spiked {
                        ctx.event(SimEvent::AddBlood { part, scratched: false, bitten: true, all_layers: false });
                        out.zombie_spiked.push("Head");
                    }
                    return Ok(out);
                }
                ctx.event(SimEvent::AddHole { part, all_layers: true });
                hole_done = true;
                pain_type = 2;
                if player {
                    ctx.event(SimEvent::PlayerVoiceSound("PainFromBite"));
                    if ctx.is_client() {
                        out.sent_zombie_hit = true;
                        out.hit = true;
                        return Ok(out);
                    }
                }
                self.body_damage_add_damage(part as usize, damage)?;
                let env = self.injury_env(sm, ctx)?;
                self.body_damage.parts[part as usize].set_bitten(&env, true);
                if part == idx("Neck")? {
                    ctx.event(SimEvent::AddBlood { part, scratched: false, bitten: true, all_layers: true });
                    ctx.event(SimEvent::AddBlood { part, scratched: false, bitten: true, all_layers: true });
                    ctx.event(SimEvent::AddBlood {
                        part: idx("Torso_Upper")?,
                        scratched: false,
                        bitten: true,
                        all_layers: false,
                    });
                    for _ in 0..3 {
                        ctx.event(SimEvent::SplatBloodFloorBig);
                    }
                }
                ctx.event(SimEvent::AddBlood { part, scratched: false, bitten: true, all_layers: true });
                for _ in 0..3 {
                    ctx.event(SimEvent::SplatBloodFloorBig);
                }
                if spiked {
                    ctx.event(SimEvent::AddBlood { part, scratched: false, bitten: true, all_layers: false });
                    out.zombie_spiked.push("Head");
                    out.zombie_killed = true;
                }
            }
        }
        if !hole_done {
            ctx.event(SimEvent::AddHoleFromZombieAttacks { part, scratch_or_bite });
        }
        let bd = &self.body_damage;
        let p = match pain_type {
            0 => bd.initial_thump_pain,
            1 => bd.initial_scratch_pain,
            _ => bd.initial_bite_pain,
        } * types.pain_modifier(part);
        self.stats.add(&stat(chars, "PAIN")?, p);
        if ctx.is_server() && player {
            ctx.event(SimEvent::SyncDamage);
        }
        out.hit = true;
        Ok(out)
    }
}

/// A big-endian reader over a save buffer (`ByteBuffer`).
pub struct SaveReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl SaveReader<'_> {
    fn take<const N: usize>(&mut self) -> ScriptResult<[u8; N]> {
        let end = self.pos + N;
        let b = self.data.get(self.pos..end).ok_or_else(|| ScriptError::new("java.nio.BufferUnderflowException"))?;
        self.pos = end;
        Ok(b.try_into().unwrap())
    }
    pub fn get(&mut self) -> ScriptResult<u8> {
        Ok(self.take::<1>()?[0])
    }
    pub fn get_bool(&mut self) -> ScriptResult<bool> {
        Ok(self.get()? != 0)
    }
    pub fn get_float(&mut self) -> ScriptResult<f32> {
        Ok(f32::from_be_bytes(self.take()?))
    }
    pub fn get_int(&mut self) -> ScriptResult<i32> {
        Ok(i32::from_be_bytes(self.take()?))
    }
    /// `GameWindow.ReadString`.
    pub fn read_string(&mut self) -> ScriptResult<String> {
        let n = i16::from_be_bytes(self.take()?);
        if n <= 0 {
            return Ok(String::new());
        }
        let n = n as usize;
        let end = self.pos + n;
        let b = self.data.get(self.pos..end).ok_or_else(|| ScriptError::new("java.nio.BufferUnderflowException"))?;
        self.pos = end;
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}
