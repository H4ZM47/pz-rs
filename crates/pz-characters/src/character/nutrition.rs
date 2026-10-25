//! `zombie.characters.BodyDamage.Nutrition` (players only, owned by
//! [`super::GameCharacter::nutrition`]).
//!
//! The original holds its `IsoPlayer`; here the state lives in
//! [`Nutrition`] and the methods that read or change the owner are
//! [`GameCharacter`] methods (`nutrition_update`, `apply_trait_from_weight`)
//! or take the owner's state as arguments.
//!
//! TODO(BodyDamage): `setWeight` below 35 also reduces the general health by
//! `getHealthReductionFromSevereBadMoodles() * multiplier` and fires
//! `OnPlayerGetDamage(player, "LOWWEIGHT", damage)`; the weight is clamped
//! but the damage is not applied yet.
//! TODO(saves): `save`/`load`.

use pz_scripts::objects::items::CharacterTrait;
use pz_scripts::{ScriptManager, ScriptResult};

use super::context::SimContext;
use super::stats::clamp_f32;
use super::GameCharacter;
use crate::Characters;

/// `Nutrition`.
#[derive(Clone, Debug)]
pub struct Nutrition {
    carbohydrates: f32,
    lipids: f32,
    proteins: f32,
    calories: f32,
    weight: f64,
    updated_weight: i32,
    calories_max: f32,
    calories_min: f32,
    inc_weight: bool,
    inc_weight_lot: bool,
    dec_weight: bool,
}

impl Default for Nutrition {
    /// The constructor: weight 80, 800 calories.
    fn default() -> Self {
        let mut n = Nutrition {
            carbohydrates: 0.0,
            lipids: 0.0,
            proteins: 0.0,
            calories: 0.0,
            weight: 60.0,
            updated_weight: 0,
            calories_max: 0.0,
            calories_min: 0.0,
            inc_weight: false,
            inc_weight_lot: false,
            dec_weight: false,
        };
        n.set_weight(80.0);
        n.set_calories(800.0);
        n
    }
}

/// `v` clamped as the macronutrient setters clamp it.
fn clamp_nutrient(v: f32) -> f32 {
    clamp_f32(v, -500.0, 1000.0)
}

impl Nutrition {
    /// `getCarbohydrates`.
    pub fn carbohydrates(&self) -> f32 {
        self.carbohydrates
    }
    /// `setCarbohydrates`: clamped to -500..1000.
    pub fn set_carbohydrates(&mut self, v: f32) {
        self.carbohydrates = clamp_nutrient(v);
    }
    /// `getProteins`.
    pub fn proteins(&self) -> f32 {
        self.proteins
    }
    /// `setProteins`: clamped to -500..1000.
    pub fn set_proteins(&mut self, v: f32) {
        self.proteins = clamp_nutrient(v);
    }
    /// `getLipids`.
    pub fn lipids(&self) -> f32 {
        self.lipids
    }
    /// `setLipids`: clamped to -500..1000.
    pub fn set_lipids(&mut self, v: f32) {
        self.lipids = clamp_nutrient(v);
    }
    /// `getCalories`.
    pub fn calories(&self) -> f32 {
        self.calories
    }
    /// `setCalories`: clamped to -2200..3700.
    pub fn set_calories(&mut self, v: f32) {
        self.calories = clamp_f32(v, -2200.0, 3700.0);
    }
    /// `getWeight`.
    pub fn weight(&self) -> f64 {
        self.weight
    }
    /// `setWeight`: at least 35. Returns true when it was clamped (the
    /// original then damages the owner; see the module TODO).
    pub fn set_weight(&mut self, mut weight: f64) -> bool {
        let clamped = weight < 35.0;
        if clamped {
            weight = 35.0;
        }
        self.weight = weight;
        clamped
    }
    /// `isIncWeight`.
    pub fn is_inc_weight(&self) -> bool {
        self.inc_weight
    }
    /// `setIncWeight`.
    pub fn set_inc_weight(&mut self, v: bool) {
        self.inc_weight = v;
    }
    /// `isIncWeightLot`.
    pub fn is_inc_weight_lot(&self) -> bool {
        self.inc_weight_lot
    }
    /// `setIncWeightLot`.
    pub fn set_inc_weight_lot(&mut self, v: bool) {
        self.inc_weight_lot = v;
    }
    /// `isDecWeight`.
    pub fn is_dec_weight(&self) -> bool {
        self.dec_weight
    }
    /// `setDecWeight`.
    pub fn set_dec_weight(&mut self, v: bool) {
        self.dec_weight = v;
    }
    /// `caloriesMax` (the largest calories seen by the update).
    pub fn calories_max(&self) -> f32 {
        self.calories_max
    }
    /// `caloriesMin`.
    pub fn calories_min(&self) -> f32 {
        self.calories_min
    }
    /// `updatedWeight`: updates since the traits were last derived from
    /// the weight.
    pub fn updated_weight(&self) -> i32 {
        self.updated_weight
    }

    /// `characterHaveWeightTrouble`; `has` is the owner's `hasTrait` by
    /// `CharacterTrait` field.
    pub fn character_have_weight_trouble(has: &dyn Fn(&str) -> ScriptResult<bool>) -> ScriptResult<bool> {
        Ok(has("EMACIATED")? || has("OBESE")? || has("VERY_UNDERWEIGHT")? || has("OVERWEIGHT")?)
    }

    /// `canAddFitnessXp`, given the owner's Fitness level and traits.
    pub fn can_add_fitness_xp(
        &self,
        fitness_level: i32,
        has: &dyn Fn(&str) -> ScriptResult<bool>,
    ) -> ScriptResult<bool> {
        if fitness_level >= 9 && Self::character_have_weight_trouble(has)? {
            return Ok(false);
        }
        if fitness_level >= 6 {
            return Ok(!has("EMACIATED")? && !has("OBESE")? && !has("VERY_UNDERWEIGHT")?);
        }
        Ok(true)
    }

    /// `applyWeightFromTraits` (called by `IsoGameCharacter.applyTraits`
    /// for players); `has` is the owner's `hasTrait`.
    pub fn apply_weight_from_traits(&mut self, has: &dyn Fn(&str) -> ScriptResult<bool>) -> ScriptResult {
        let weights = [
            ("EMACIATED", 50.0),
            ("VERY_UNDERWEIGHT", 60.0),
            ("UNDERWEIGHT", 70.0),
            ("OVERWEIGHT", 95.0),
            ("OBESE", 105.0),
        ];
        for (t, w) in weights {
            if has(t)? {
                self.set_weight(w);
            }
        }
        Ok(())
    }

    /// `updateCalories`, given the owner's state.
    fn update_calories(&mut self, s: &CaloriesInputs, ctx: &dyn SimContext) {
        let mut modifier = 1.0f32;
        if let Some(m) = s.first_action_calories_modifier {
            modifier = m;
        }
        if s.swipe_or_climb {
            modifier = 8.0;
        }
        let cold_multi = s.energy_multiplier.map_or(1.0, |e| e as f32);
        let calories_delta = (self.weight() / 80.0) as f32;
        let secs = ctx.game_world_seconds_since_last_update();
        if s.running && s.player_moving {
            modifier = 1.0;
            self.set_calories(self.calories() - 0.13 * modifier * calories_delta * secs);
        } else if s.sprinting && s.player_moving {
            modifier = 1.3;
            self.set_calories(self.calories() - 0.13 * modifier * calories_delta * secs);
        } else if s.player_moving {
            modifier = 0.6;
            self.set_calories(self.calories() - 0.13 * modifier * calories_delta * secs);
        } else if s.asleep {
            self.set_calories(self.calories() - 0.003 * modifier * cold_multi * calories_delta * secs);
        } else {
            self.set_calories(self.calories() - 0.016 * modifier * cold_multi * calories_delta * secs);
        }
        if self.calories() > self.calories_max {
            self.calories_max = self.calories();
        }
        if self.calories() < self.calories_min {
            self.calories_min = self.calories();
        }
    }

    /// `updateWeight` up to the trait update: returns true when
    /// `applyTraitFromWeight` is due.
    fn update_weight(&mut self, has: &dyn Fn(&str) -> ScriptResult<bool>, ctx: &dyn SimContext) -> ScriptResult<bool> {
        self.set_inc_weight(false);
        self.set_inc_weight_lot(false);
        self.set_dec_weight(false);
        let mut calories_to_gain_weight = 1000.0f32;
        let calories_to_gain_weight_max = 4000.0f32;
        if self.weight() < 90.0 && has("WEIGHT_GAIN")? {
            calories_to_gain_weight = 700.0;
        }
        if self.weight() > 70.0 && has("WEIGHT_LOSS")? {
            calories_to_gain_weight = 1800.0;
        }
        let calories_diff = ((self.weight() - 80.0) * 40.0) as f32;
        calories_to_gain_weight += calories_diff;
        let mut calories_to_lose_weight = ((self.weight() - 70.0) * 30.0) as f32;
        if calories_to_lose_weight > 0.0 {
            calories_to_lose_weight = 0.0;
        }
        let secs = ctx.game_world_seconds_since_last_update();
        let weight = if self.calories() > calories_to_gain_weight {
            self.set_inc_weight(true);
            let mut delta = self.calories() / calories_to_gain_weight_max;
            if delta > 1.0 {
                delta = 1.0;
            }
            let mut real_weight_gain = 1.3e-5f32;
            if self.carbohydrates() > 700.0 || self.lipids() > 700.0 {
                real_weight_gain *= 3.0;
                self.set_inc_weight_lot(true);
            } else if self.carbohydrates() > 400.0 || self.lipids() > 400.0 {
                real_weight_gain *= 2.0;
                self.set_inc_weight_lot(true);
            }
            self.weight() + (real_weight_gain * delta * secs) as f64
        } else if self.calories() < calories_to_lose_weight {
            self.set_dec_weight(true);
            let mut delta = self.calories().abs() / 2500.0;
            if delta > 1.0 {
                delta = 1.0;
            }
            self.weight() - (8.5e-6f32 * delta * secs) as f64
        } else {
            self.weight()
        };
        if !ctx.is_client() {
            self.set_weight(weight);
            self.updated_weight += 1;
            if self.updated_weight >= 2000 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// What `updateCalories` reads from the owner.
struct CaloriesInputs {
    first_action_calories_modifier: Option<f32>,
    swipe_or_climb: bool,
    /// The thermoregulator's `getEnergyMultiplier()`, if there is one.
    energy_multiplier: Option<f64>,
    running: bool,
    sprinting: bool,
    player_moving: bool,
    asleep: bool,
}

impl<I: 'static> GameCharacter<I> {
    /// `Nutrition.update` (players only; nothing without a `Nutrition`).
    pub fn nutrition_update(&mut self, sm: &ScriptManager, _chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if !ctx.nutrition_enabled() {
            return Ok(());
        }
        if self.nutrition.is_none() || self.is_dead() {
            return Ok(());
        }
        if self.is_god_mod(ctx) {
            return Ok(());
        }
        let inputs = CaloriesInputs {
            first_action_calories_modifier: self.environment.first_action_calories_modifier,
            swipe_or_climb: self.environment.swipe_state || self.environment.climbing_state,
            energy_multiplier: self.body_damage.thermoregulator().map(|t| t.energy_multiplier()),
            running: self.is_running(sm),
            sprinting: self.is_sprinting(sm, ctx),
            player_moving: self.player_moving,
            asleep: self.asleep,
        };
        let Some(mut n) = self.nutrition.take() else {
            return Ok(());
        };
        if !ctx.is_client() {
            let secs = ctx.game_world_seconds_since_last_update();
            n.set_carbohydrates(n.carbohydrates() - 0.0035 * secs);
            n.set_lipids(n.lipids() - 0.00113 * secs);
            n.set_proteins(n.proteins() - 8.6e-4 * secs);
            n.update_calories(&inputs, ctx);
        }
        let due = n.update_weight(&|f| self.has_trait(sm, f), ctx);
        self.nutrition = Some(n);
        if due? {
            self.apply_trait_from_weight(sm)?;
            if let Some(n) = &mut self.nutrition {
                n.updated_weight = 0;
            }
        }
        Ok(())
    }

    /// `Nutrition.applyTraitFromWeight`: replaces the weight traits by the
    /// one the weight falls in.
    pub fn apply_trait_from_weight(&mut self, sm: &ScriptManager) -> ScriptResult {
        let Some(weight) = self.nutrition.as_ref().map(Nutrition::weight) else {
            return Ok(());
        };
        let key = |f: &str| CharacterTrait::constant(sm, f);
        for f in ["UNDERWEIGHT", "VERY_UNDERWEIGHT", "EMACIATED", "OVERWEIGHT", "OBESE"] {
            self.character_traits.remove(key(f).as_ref())?;
        }
        if weight >= 100.0 {
            self.character_traits.add(key("OBESE").as_ref())?;
        }
        if (85.0..100.0).contains(&weight) {
            self.character_traits.add(key("OVERWEIGHT").as_ref())?;
        }
        if weight > 65.0 && weight <= 75.0 {
            self.character_traits.add(key("UNDERWEIGHT").as_ref())?;
        }
        if weight > 50.0 && weight <= 65.0 {
            self.character_traits.add(key("VERY_UNDERWEIGHT").as_ref())?;
        }
        if weight <= 50.0 {
            self.character_traits.add(key("EMACIATED").as_ref())?;
        }
        Ok(())
    }
}
