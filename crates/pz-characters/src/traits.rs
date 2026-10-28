//! `zombie.characters.traits.CharacterTraits`: a character's trait flags.

use std::rc::Rc;

use pz_scripts::objects::items::CharacterTrait;
use pz_inventory::io::{Reader, Writer};
use pz_scripts::registry::{RegistryKey, ResourceLocation};
use pz_scripts::{ScriptManager, ScriptResult};

use crate::character::save::io_error;
use crate::npe;

/// `CharacterTraits`.
#[derive(Clone, Debug)]
pub struct CharacterTraits {
    /// `traits`: a `LinkedHashMap` filled in `Registries.CHARACTER_TRAIT`
    /// key order.
    traits: Vec<(Rc<RegistryKey>, bool)>,
    known_traits: Vec<Rc<RegistryKey>>,
}

impl CharacterTraits {
    pub const OBESE_STRENGTH_PENALTY: i32 = 2;
    pub const OVERWEIGHT_STRENGTH_PENALTY: i32 = 1;
    pub const ALL_THUMBS_STRENGTH_PENALTY: i32 = 1;
    pub const DEXTROUS_STRENGTH_BONUS: i32 = 1;
    pub const BURGLAR_STRENGTH_BONUS: i32 = 1;
    pub const GYMNAST_STRENGTH_BONUS: i32 = 1;
    pub const OBESE_CLIMBING_PENALTY: f32 = -25.0;
    pub const OVERWEIGHT_CLIMBING_PENALTY: f32 = -15.0;
    pub const CLUMSY_CLIMBING_PENALTY_DIVISOR: f32 = 2.0;
    pub const AWKWARD_GLOVES_CLIMBING_PENALTY_DIVISOR: f32 = 2.0;
    pub const REGULAR_GLOVES_CLIMBING_BONUS: f32 = 4.0;
    pub const PERK_CLIMBING_BONUS_MULTIPLIER: f32 = 2.0;
    pub const ENDURANCE_CLIMBING_PENALTY_MULTIPLIER: f32 = -5.0;
    pub const DRUNK_CLIMBING_PENALTY_MULTIPLIER: f32 = -8.0;
    pub const HEAVY_LOAD_CLIMBING_PENALTY_MULTIPLIER: f32 = -8.0;
    pub const PAIN_CLIMBING_PENALTY_MULTIPLIER: f32 = -5.0;
    pub const ALL_THUMBS_CLIMBING_PENALTY: f32 = -4.0;
    pub const DEXTROUS_CLIMBING_BONUS: f32 = 4.0;
    pub const BURGLAR_CLIMBING_BONUS: f32 = 4.0;
    pub const GYMNAST_CLIMBING_BONUS: f32 = 4.0;
    pub const HEALTH_REDUCTION_MULTIPLIER_MODERATE: f32 = 1.2;
    pub const HEALTH_REDUCTION_MULTIPLIER_SEVERE: f32 = 1.4;
    pub const BASE_DETECTION_RANGE: f32 = 3.5;
    pub const FATIGUE_THRESHOLD: f32 = 0.6;
    pub const FATIGUE_SCALE: f32 = 2.5;
    pub const HARD_OF_HEARING_RANGE_PENALTY: f32 = 1.0;
    pub const DEAF_DETECTION_RANGE: f32 = 2.0;
    pub const KEEN_HEARING_RANGE_BONUS: f32 = 3.0;

    /// The public constants as `(Java name, value)`, ints first.
    pub const INT_CONSTANTS: &'static [(&'static str, i32)] = &[
        ("ObeseStrengthPenalty", Self::OBESE_STRENGTH_PENALTY),
        ("OverweightStrengthPenalty", Self::OVERWEIGHT_STRENGTH_PENALTY),
        ("AllThumbsStrengthPenalty", Self::ALL_THUMBS_STRENGTH_PENALTY),
        ("DextrousStrengthBonus", Self::DEXTROUS_STRENGTH_BONUS),
        ("BurglarStrengthBonus", Self::BURGLAR_STRENGTH_BONUS),
        ("GymnastStrengthBonus", Self::GYMNAST_STRENGTH_BONUS),
    ];
    pub const FLOAT_CONSTANTS: &'static [(&'static str, f32)] = &[
        ("ObeseClimbingPenalty", Self::OBESE_CLIMBING_PENALTY),
        ("OverweightClimbingPenalty", Self::OVERWEIGHT_CLIMBING_PENALTY),
        ("ClumsyClimbingPenaltyDivisor", Self::CLUMSY_CLIMBING_PENALTY_DIVISOR),
        ("AwkwardGlovesClimbingPenaltyDivisor", Self::AWKWARD_GLOVES_CLIMBING_PENALTY_DIVISOR),
        ("RegularGlovesClimbingBonus", Self::REGULAR_GLOVES_CLIMBING_BONUS),
        ("PerkClimbingBonusMultiplier", Self::PERK_CLIMBING_BONUS_MULTIPLIER),
        ("EnduranceClimbingPenaltyMultiplier", Self::ENDURANCE_CLIMBING_PENALTY_MULTIPLIER),
        ("DrunkClimbingPenaltyMultiplier", Self::DRUNK_CLIMBING_PENALTY_MULTIPLIER),
        ("HeavyLoadClimbingPenaltyMultiplier", Self::HEAVY_LOAD_CLIMBING_PENALTY_MULTIPLIER),
        ("PainClimbingPenaltyMultiplier", Self::PAIN_CLIMBING_PENALTY_MULTIPLIER),
        ("AllThumbsClimbingPenalty", Self::ALL_THUMBS_CLIMBING_PENALTY),
        ("DextrousClimbingBonus", Self::DEXTROUS_CLIMBING_BONUS),
        ("BurglarClimbingBonus", Self::BURGLAR_CLIMBING_BONUS),
        ("GymnastClimbingBonus", Self::GYMNAST_CLIMBING_BONUS),
        ("HealthReductionMultiplierModerate", Self::HEALTH_REDUCTION_MULTIPLIER_MODERATE),
        ("HealthReductionMultiplierSevere", Self::HEALTH_REDUCTION_MULTIPLIER_SEVERE),
        ("BASE_DETECTION_RANGE", Self::BASE_DETECTION_RANGE),
        ("FATIGUE_THRESHOLD", Self::FATIGUE_THRESHOLD),
        ("FATIGUE_SCALE", Self::FATIGUE_SCALE),
        ("HARD_OF_HEARING_RANGE_PENALTY", Self::HARD_OF_HEARING_RANGE_PENALTY),
        ("DEAF_DETECTION_RANGE", Self::DEAF_DETECTION_RANGE),
        ("KEEN_HEARING_RANGE_BONUS", Self::KEEN_HEARING_RANGE_BONUS),
    ];

    /// The constructor: every registered trait, unset.
    pub fn new(sm: &ScriptManager) -> Self {
        let reg = sm.registries.get(CharacterTrait::REGISTRY).expect("known registry").borrow();
        CharacterTraits { traits: reg.iter().map(|k| (k.clone(), false)).collect(), known_traits: Vec::new() }
    }

    fn position(&self, t: Option<&Rc<RegistryKey>>) -> Option<usize> {
        let t = t?;
        self.traits.iter().position(|(k, _)| Rc::ptr_eq(k, t) || k.location == t.location)
    }

    /// `get`; a trait not in the map unboxes `null` (`NullPointerException`).
    pub fn get(&self, t: Option<&Rc<RegistryKey>>) -> ScriptResult<bool> {
        self.position(t).map(|i| self.traits[i].1).ok_or_else(npe)
    }

    /// `set`: whether the value changed. Setting `true` appends to the known
    /// traits even if already there; `false` removes the first occurrence.
    pub fn set(&mut self, t: Option<&Rc<RegistryKey>>, value: bool) -> ScriptResult<bool> {
        let old = self.get(t)?;
        let t = t.expect("present after get");
        let i = self.position(Some(t)).expect("present after get");
        self.traits[i].1 = value;
        if value {
            self.known_traits.push(t.clone());
        } else if let Some(k) = self.known_traits.iter().position(|k| k.location == t.location) {
            self.known_traits.remove(k);
        }
        Ok(value != old)
    }

    /// `add`.
    pub fn add(&mut self, t: Option<&Rc<RegistryKey>>) -> ScriptResult {
        self.set(t, true).map(|_| ())
    }

    /// `remove`.
    pub fn remove(&mut self, t: Option<&Rc<RegistryKey>>) -> ScriptResult {
        self.set(t, false).map(|_| ())
    }

    /// `getTraits`.
    pub fn traits(&self) -> &[(Rc<RegistryKey>, bool)] {
        &self.traits
    }

    /// `getKnownTraits` (a copy).
    pub fn known_traits(&self) -> Vec<Rc<RegistryKey>> {
        self.known_traits.clone()
    }

    /// `reset`.
    pub fn reset(&mut self) {
        self.known_traits.clear();
        for e in &mut self.traits {
            e.1 = false;
        }
    }

    /// `load`: `reset`, then every saved trait (`CharacterTrait.get` of the
    /// saved location) added.
    pub fn load(&mut self, sm: &ScriptManager, r: &mut Reader<'_>) -> ScriptResult {
        self.reset();
        let n = r.i32().map_err(io_error)?;
        for _ in 0..n {
            let id = r.string().map_err(io_error)?;
            let t = CharacterTrait::get(sm, &ResourceLocation::of(&id)?);
            self.add(t.as_ref())?;
        }
        Ok(())
    }

    /// `save`: the known traits' registry locations.
    pub fn save(&self, w: &mut Writer) {
        w.i32(self.known_traits.len() as i32);
        for t in &self.known_traits {
            w.string(&t.location.id());
        }
    }

    fn constant(&self, sm: &ScriptManager, field: &str) -> ScriptResult<bool> {
        self.get(CharacterTrait::constant(sm, field).as_ref())
    }

    /// `getTraitDamageDealtReductionModifier`.
    pub fn trait_damage_dealt_reduction_modifier(&self, sm: &ScriptManager) -> ScriptResult<f32> {
        let mut m = 1.0f32;
        if self.constant(sm, "UNDERWEIGHT")? {
            m *= 0.8;
        }
        if self.constant(sm, "VERY_UNDERWEIGHT")? {
            m *= 0.6;
        }
        if self.constant(sm, "EMACIATED")? {
            m *= 0.4;
        }
        Ok(m)
    }

    /// `getTraitEnduranceLossModifier`.
    pub fn trait_endurance_loss_modifier(&self, sm: &ScriptManager) -> ScriptResult<f32> {
        Ok(if self.constant(sm, "ASTHMATIC")? { 1.2 } else { 1.0 })
    }

    /// `getTraitWeatherPenaltyModifier`.
    pub fn trait_weather_penalty_modifier(&self, sm: &ScriptManager) -> ScriptResult<f32> {
        Ok(if self.constant(sm, "OUTDOORSMAN")? { 1.0 } else { 1.5 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_known() {
        let sm = ScriptManager::new();
        let a = CharacterTrait::register(&sm, "t:alpha").unwrap();
        let b = CharacterTrait::register(&sm, "t:beta").unwrap();
        let mut t = CharacterTraits::new(&sm);
        assert_eq!(t.traits().len(), 2);
        assert!(!t.get(Some(&a)).unwrap());
        assert!(t.set(Some(&a), true).unwrap());
        assert!(!t.set(Some(&a), true).unwrap());
        t.add(Some(&b)).unwrap();
        assert_eq!(t.known_traits().len(), 3);
        t.remove(Some(&a)).unwrap();
        let known: Vec<_> = t.known_traits().iter().map(|k| k.path().to_string()).collect();
        assert_eq!(known, ["alpha", "beta"]);
        assert!(t.get(None).is_err());
        let other = ScriptManager::new();
        let c = CharacterTrait::register(&other, "t:gamma").unwrap();
        assert!(t.set(Some(&c), true).is_err());
        assert!(t.trait_endurance_loss_modifier(&sm).is_err());
        t.reset();
        assert!(t.known_traits().is_empty() && !t.get(Some(&b)).unwrap());
    }
}
