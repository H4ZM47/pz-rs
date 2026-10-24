//! `zombie.characters.BodyDamage.Nutrition`.
//!
//! **Slot for the Nutrition port** (players only, owned by
//! [`super::GameCharacter::nutrition`]). Only what the ported XP and stat
//! code reads exists so far; the port keeps these method names. Methods the
//! original runs against its owner take the owner's state as arguments.

use pz_scripts::ScriptResult;

/// `Nutrition`.
#[derive(Clone, Debug, Default)]
pub struct Nutrition {
    /// `carbohydrates`.
    pub carbohydrates: f32,
    /// `lipids`.
    pub lipids: f32,
    /// `proteins`.
    pub proteins: f32,
    /// `calories`.
    pub calories: f32,
}

impl Nutrition {
    /// `getProteins`.
    pub fn proteins(&self) -> f32 {
        self.proteins
    }
    /// `getLipids`.
    pub fn lipids(&self) -> f32 {
        self.lipids
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

    /// TODO(Nutrition): `applyWeightFromTraits` (called by
    /// `IsoGameCharacter.applyTraits` for players).
    pub fn apply_weight_from_traits(&mut self) {}
}
