//! The per-tick stat update: `IsoGameCharacter.calculateStats` (with the
//! `IsoPlayer` overrides of `calculateStats` and `updateStats_Sleeping`),
//! the stat part of `IsoGameCharacter.updateInternal`, and
//! `IsoPlayer.updateEnduranceWhileSitting`.
//!
//! Arithmetic follows the original: `float` fields and `double`
//! expressions (`ZomboidGlobals`, sandbox multipliers) cast to `float` at
//! the same points.
//!
//! TODO(IsoPlayer.updateEndurance): the movement-based endurance update
//! (needs movement speed, pacing and hyperthermia modifiers).
//! TODO(autoDrink): needs the inventory and fluid containers.

use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::context::{SimContext, SimEvent};
use super::stats::{clamp_f32, CharacterStatRef};
use super::GameCharacter;
use crate::Characters;

impl<I> GameCharacter<I> {
    fn stat_ref(chars: &Characters, field: &str) -> ScriptResult<CharacterStatRef> {
        chars.sim.character_stats.field(field)
    }

    /// `calculateStats` (`IsoPlayer`: only when not a client).
    pub fn calculate_stats(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.is_player() && ctx.is_client() {
            return Ok(());
        }
        if self.is_animal() {
            return Ok(());
        }
        if ctx.is_server() && !ctx.server_sleep_allowed_and_needed() {
            self.stats.reset(&Self::stat_ref(chars, "FATIGUE")?);
        }
        if ctx.trigger_hook("CalculateStats") {
            return Ok(());
        }
        self.update_endurance_base(chars, ctx)?;
        self.update_tripping();
        self.update_thirst(sm, chars, ctx)?;
        self.update_stress(sm, chars, ctx)?;
        self.update_stats_wake_state(sm, chars, ctx)?;
        self.update_morale(chars)?;
        self.update_fitness(chars)
    }

    /// `IsoGameCharacter.updateEndurance`.
    fn update_endurance_base(&mut self, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let e = Self::stat_ref(chars, "ENDURANCE")?;
        self.stats.set_last_endurance(self.stats.get(&e));
        if self.is_unlimited_endurance(ctx) {
            self.stats.reset(&e);
        }
        Ok(())
    }

    /// `updateTripping`.
    fn update_tripping(&mut self) {
        if self.stats.is_tripping() {
            self.stats.add_tripping_rot_angle(0.06);
        }
    }

    /// `getRunningThirstReduction`.
    fn running_thirst_reduction(&self, sm: &ScriptManager) -> f64 {
        if self.player_instance && self.is_running(sm) {
            1.2
        } else {
            1.0
        }
    }

    /// `updateThirst` (without `autoDrink`).
    fn update_thirst(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let g = ctx.zomboid_globals();
        let mut trait_mod = 1.0f32;
        if self.has_trait(sm, "HIGH_THIRST")? {
            trait_mod *= 2.0;
        }
        if self.has_trait(sm, "LOW_THIRST")? {
            trait_mod *= 0.5;
        }
        let skip = !ctx.is_server() && (ctx.is_client() || !self.player_instance)
            || self.is_player() && self.is_ghost_mode(ctx);
        if !skip {
            let thirst = Self::stat_ref(chars, "THIRST")?;
            let mult = ctx.multiplier() as f64;
            let delta = ctx.delta_minutes_per_day() as f64;
            let sd = ctx.stats_decrease_multiplier();
            if self.asleep {
                self.stats.add(&thirst, (g.thirst_sleeping_increase * sd * mult * delta * trait_mod as f64) as f32);
            } else {
                let v = g.thirst_increase
                    * sd
                    * mult
                    * self.running_thirst_reduction(sm)
                    * delta
                    * trait_mod as f64
                    * self.thirst_multiplier();
                self.stats.add(&thirst, v as f32);
            }
        }
        Ok(())
    }

    /// `updateStress`.
    fn update_stress(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.is_animal() {
            return Ok(());
        }
        let g = ctx.zomboid_globals();
        let stress = Self::stat_ref(chars, "STRESS")?;
        let mult = ctx.multiplier();
        let delta = ctx.delta_minutes_per_day() as f64;
        if !self.has_trait(sm, "DEAF")? {
            self.stats
                .add(&stress, (self.environment.stress_from_sounds as f64 * g.stress_from_sounds_multiplier) as f32);
        }
        let bite = (g.stress_from_bite_or_scratch * mult as f64 * delta) as f32;
        if self.body_damage.num_parts_bitten() > 0 {
            self.stats.add(&stress, bite);
        }
        if self.body_damage.num_parts_scratched() > 0 {
            self.stats.add(&stress, bite);
        }
        if self.body_damage.is_infected() || self.body_damage.is_fake_infected() {
            self.stats.add(&stress, bite);
        }
        if self.has_trait(sm, "HEMOPHOBIC")? {
            let v = self.environment.total_blood as f64 * g.stress_from_hemophobic * (mult / 0.8) as f64 * delta;
            self.stats.add(&stress, v as f32);
        }
        self.stats.remove(&Self::stat_ref(chars, "ANGER")?, (g.anger_decrease * mult as f64 * delta) as f32);
        Ok(())
    }

    /// `updateStats_WakeState`.
    fn update_stats_wake_state(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        if self.is_animal() {
            return Ok(());
        }
        if ctx.is_server() || !ctx.is_client() && self.player_instance {
            if self.asleep {
                if self.is_player() {
                    self.update_stats_sleeping_player(sm, chars, ctx)?;
                }
            } else {
                self.update_stats_awake(sm, chars, ctx)?;
            }
        }
        Ok(())
    }

    /// `getAppetiteMultiplier`.
    pub fn appetite_multiplier(&self, sm: &ScriptManager, chars: &Characters) -> ScriptResult<f32> {
        let mut m = 1.0 - self.stats.get(&Self::stat_ref(chars, "HUNGER")?);
        if self.has_trait(sm, "HEARTY_APPETITE")? {
            m *= 1.5;
        }
        if self.has_trait(sm, "LIGHT_EATER")? {
            m *= 0.75;
        }
        Ok(m)
    }

    /// `updateIdleSquareTime`.
    fn update_idle_square_time(&mut self, ctx: &dyn SimContext) {
        if self.environment.on_last_square {
            if self.idle_square_time <= 3600.0 {
                self.idle_square_time += 1.0 * ctx.multiplier() * ctx.delta_minutes_per_day();
            }
        } else {
            self.idle_square_time = 0.0;
        }
    }

    /// `updateStats_Awake`.
    fn update_stats_awake(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let g = ctx.zomboid_globals();
        let mult = ctx.multiplier() as f64;
        let delta = ctx.delta_minutes_per_day() as f64;
        let sd = ctx.stats_decrease_multiplier();
        self.stats.remove(&Self::stat_ref(chars, "STRESS")?, (g.stress_reduction * mult * delta) as f32);
        let mut fatiguemod = 1.0 - self.stats.get(&Self::stat_ref(chars, "ENDURANCE")?);
        if fatiguemod < 0.3 {
            fatiguemod = 0.3;
        }
        let mut mul = 1.0f32;
        if self.has_trait(sm, "NEEDS_LESS_SLEEP")? {
            mul = 0.7;
        }
        if self.has_trait(sm, "NEEDS_MORE_SLEEP")? {
            mul = 1.3;
        }
        let mut m = 1.0f32;
        if self.is_sitting() || self.is_resting {
            m = 1.5;
        }
        let fatigue =
            g.fatigue_increase * sd * fatiguemod as f64 * mult * delta * mul as f64 * self.fatique_multiplier()
                / m as f64;
        self.stats.add(&Self::stat_ref(chars, "FATIGUE")?, fatigue as f32);
        let hunger_mult = self.appetite_multiplier(sm, chars)? as f64;
        let hunger = Self::stat_ref(chars, "HUNGER")?;
        let well_fed = self.moodles.moodle_level_of(sm, "FOOD_EATEN")? != 0;
        let exercising = self.is_player() && self.is_running(sm) && self.player_moving || self.environment.swipe_state;
        let hm = self.hunger_multiplier();
        let v = if exercising {
            if !well_fed {
                g.hunger_increase_when_exercise / 3.0 * sd * hunger_mult * mult * delta * hm
            } else {
                g.hunger_increase_when_exercise * sd * hunger_mult * mult * delta * hm
            }
        } else if !well_fed {
            g.hunger_increase * sd * hunger_mult * mult * delta * hm
        } else {
            g.hunger_increase_when_well_fed * sd * mult * delta * hm
        };
        self.stats.add(&hunger, v as f32);
        self.update_idle_square_time(ctx);
        let idle = Self::stat_ref(chars, "IDLENESS")?;
        if self.is_in_combat() {
            self.stats.reset(&idle);
        } else if self.environment.currently_idle && self.environment.has_current_square {
            if self.environment.on_last_square && self.idle_square_time >= 1800.0 {
                let v = self.stats.get(&idle) as f64 + g.idle_increase_rate * mult * delta;
                self.stats.set(&idle, v as f32);
            }
            if self.environment.square_in_room {
                let v = self.stats.get(&idle) as f64 + g.idle_increase_rate / 3.0 * mult * delta;
                self.stats.set(&idle, v as f32);
            }
        } else if !self.is_sitting() {
            let v = self.stats.get(&idle) as f64 - g.idle_decrease_rate * mult * delta;
            self.stats.set(&idle, v as f32);
        }
        Ok(())
    }

    /// `IsoPlayer.updateStats_Sleeping`.
    fn update_stats_sleeping_player(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let g = ctx.zomboid_globals();
        let mut end_mult = 2.0f32;
        if ctx.all_players_asleep() {
            end_mult *= ctx.delta_minutes_per_day();
        }
        let v = g.imobile_endurance_reduce
            * ctx.endurance_regen_multiplier()
            * self.recovery_mod(sm, chars)? as f64
            * ctx.multiplier() as f64
            * end_mult as f64;
        self.stats.add(&Self::stat_ref(chars, "ENDURANCE")?, v as f32);
        let fatigue = Self::stat_ref(chars, "FATIGUE")?;
        if self.stats.is_above_minimum(&fatigue) {
            let mut mul = 1.0f32;
            if self.has_trait(sm, "INSOMNIAC")? {
                mul *= 0.5;
            }
            if self.has_trait(sm, "NIGHT_OWL")? {
                mul *= 1.4;
            }
            let mut bed = 1.0f32;
            if self.bed_type == "averageBedPillow" {
                bed = 1.05;
            }
            match self.bed_type.as_str() {
                "goodBed" => bed = 1.1,
                "goodBedPillow" => bed = 1.15,
                "badBed" => bed = 0.9,
                "badBedPillow" => bed = 0.95,
                "floor" => bed = 0.6,
                "floorPillow" => bed = 0.75,
                _ => {}
            }
            let elapsed_hours = 1.0 / ctx.minutes_per_day() / 60.0 * ctx.multiplier() / 2.0;
            self.time_of_sleep += elapsed_hours;
            if self.time_of_sleep > self.delay_to_actually_sleep {
                let mut trait_mul = 1.0f32;
                if self.has_trait(sm, "NEEDS_LESS_SLEEP")? {
                    trait_mul *= 0.75;
                } else if self.has_trait(sm, "NEEDS_MORE_SLEEP")? {
                    trait_mul *= 1.18;
                }
                if self.stats.get(&fatigue) <= 0.3 {
                    let hours_needed = 7.0 * trait_mul;
                    self.stats.remove(&fatigue, elapsed_hours / hours_needed * 0.3 * mul * bed);
                } else {
                    let hours_needed = 5.0 * trait_mul;
                    self.stats.remove(&fatigue, elapsed_hours / hours_needed * 0.7 * mul * bed);
                }
            }
        }
        let hunger = Self::stat_ref(chars, "HUNGER")?;
        let sd = ctx.stats_decrease_multiplier();
        let mult = ctx.multiplier() as f64;
        let delta = ctx.delta_minutes_per_day() as f64;
        if self.moodles.moodle_level_of(sm, "FOOD_EATEN")? == 0 {
            let hunger_mult = self.appetite_multiplier(sm, chars)? as f64;
            let v = g.hunger_increase_while_asleep * sd * hunger_mult * mult * delta * self.hunger_multiplier();
            self.stats.add(&hunger, v as f32);
        } else {
            let v = g.hunger_increase_when_well_fed
                * sd
                * g.hunger_increase_while_asleep
                * sd
                * mult
                * self.hunger_multiplier()
                * delta;
            self.stats.add(&hunger, v as f32);
        }
        Ok(())
    }

    /// `updateMorale`.
    fn update_morale(&mut self, chars: &Characters) -> ScriptResult {
        let mut m = 1.0 - self.stats.nicotine_stress(&chars.sim.character_stats)? - 0.5;
        m *= 1.0e-4;
        if m > 0.0 {
            m += 0.5;
        }
        self.stats.add(&Self::stat_ref(chars, "MORALE")?, clamp_f32(m, 0.0, 1.0));
        Ok(())
    }

    /// `updateFitness`.
    fn update_fitness(&mut self, chars: &Characters) -> ScriptResult {
        let level = self.perk_level(chars.perks.constant("Fitness").as_ref());
        self.stats.set(&Self::stat_ref(chars, "FITNESS")?, level as f32 / 5.0 - 1.0);
        Ok(())
    }

    /// `IsoPlayer.updateEnduranceWhileSitting`.
    pub fn update_endurance_while_sitting(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let g = ctx.zomboid_globals();
        let mut mul = g.sitting_endurance_multiplier as f32;
        mul *= 1.0 - self.stats.get(&Self::stat_ref(chars, "FATIGUE")?) * 0.8;
        mul *= ctx.multiplier();
        let v = g.imobile_endurance_reduce
            * ctx.endurance_regen_multiplier()
            * self.recovery_mod(sm, chars)? as f64
            * mul as f64;
        self.stats.add(&Self::stat_ref(chars, "ENDURANCE")?, v as f32);
        Ok(())
    }

    /// The stat and moodle part of `updateInternal` (from the client moodle
    /// update to the sanity changes).
    pub fn update_internal_stats(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let in_vehicle = self.environment.in_vehicle;
        if ctx.is_client() && !self.is_zombie() {
            self.update_moodles(sm, chars, ctx)?;
        }
        if ctx.is_client() || self.is_zombie() {
            return Ok(());
        }
        let panic = Self::stat_ref(chars, "PANIC")?;
        let fps = ctx.thirty_fps_multiplier();
        let square = |c: &Self| {
            if c.environment.has_current_square {
                Ok(())
            } else {
                Err(ScriptError::new("java.lang.NullPointerException: getCurrentSquare()"))
            }
        };
        if self.has_trait(sm, "AGORAPHOBIC")? {
            square(self)?;
            if !self.environment.square_in_room {
                self.stats.add(&panic, 0.5 * fps);
            }
        }
        if self.has_trait(sm, "CLAUSTROPHOBIC")? {
            if !in_vehicle {
                square(self)?;
            }
            if self.environment.square_in_room || in_vehicle {
                let n = if in_vehicle { 60 } else { self.environment.room_size };
                if n > 0 && n < 70 {
                    let del = (1.0 - n as f32 / 70.0).max(0.0);
                    self.stats.add(&panic, 0.6 * del * fps);
                }
            }
        }
        if self.body_damage.num_parts_bleeding() > 0 {
            let del =
                (1.0 - self.body_damage.overall_body_health() / 100.0) * self.body_damage.num_parts_bleeding() as f32;
            let k = if self.has_trait(sm, "HEMOPHOBIC")? { 0.4 } else { 0.2 };
            self.stats.add(&panic, k * del * fps);
        }
        self.update_moodles(sm, chars, ctx)?;
        if self.asleep {
            self.beta_effect = 0.0;
            self.sleeping_tablet_effect = 0.0;
            ctx.event(SimEvent::StopAllActionQueue);
        }
        if self.beta_effect > 0.0 {
            self.beta_effect -= fps;
            self.stats.remove(&panic, 0.6 * fps);
        } else {
            self.beta_delta = 0.0;
        }
        if self.depress_first_take_time > 0.0 || self.depress_effect > 0.0 {
            self.depress_first_take_time -= fps;
            if self.depress_first_take_time < 0.0 {
                self.depress_first_take_time = -1.0;
                self.depress_effect -= fps;
                self.stats.remove(&Self::stat_ref(chars, "UNHAPPINESS")?, 0.03 * fps);
            }
        }
        if self.depress_effect < 0.0 {
            self.depress_effect = 0.0;
        }
        if self.sleeping_tablet_effect > 0.0 {
            self.sleeping_tablet_effect -= fps;
            self.stats.add(&Self::stat_ref(chars, "FATIGUE")?, 0.001_666_666_7 * self.sleeping_tablet_delta * fps);
        } else {
            self.sleeping_tablet_delta = 0.0;
        }
        let sanity = Self::stat_ref(chars, "SANITY")?;
        match self.moodles.moodle_level_of(sm, "PANIC")? {
            2 => {
                self.stats.remove(&sanity, 3.2e-7);
            }
            3 => {
                self.stats.remove(&sanity, 4.800_000_4e-7);
            }
            4 => {
                self.stats.remove(&sanity, 8.0e-7);
            }
            0 => {
                self.stats.add(&sanity, 1.0e-7);
            }
            _ => {}
        }
        if self.moodles.moodle_level_of(sm, "TIRED")? == 4 {
            self.stats.remove(&sanity, 2.0e-6);
        }
        Ok(())
    }
}
