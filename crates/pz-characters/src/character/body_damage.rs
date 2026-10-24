//! `zombie.characters.BodyDamage.BodyDamage` (and later `BodyPart`).
//!
//! **Slot for the BodyDamage port.** Only the state the ported moodle and
//! stat code reads exists so far, as plain fields with the original's
//! getter names; the full port replaces the fields with the body parts and
//! keeps these method names and signatures (the callers in
//! [`super::moodles`] and [`super::update`] use only the methods). Values
//! default to a healthy character.
//!
//! The original's `BodyDamage` holds its owner; here methods that need the
//! owner's state take it as arguments (`&Stats`, the stat constants).

use pz_scripts::ScriptResult;

use super::stats::{CharacterStats, Stats};
use super::thermoregulator::Thermoregulator;

/// `BodyDamage`.
#[derive(Clone, Debug)]
pub struct BodyDamage {
    /// TODO(BodyDamage): `getOverallBodyHealth()` from the body parts.
    pub overall_body_health: f32,
    /// TODO(BodyDamage): `getNumPartsBleeding()` from the body parts.
    pub num_parts_bleeding: i32,
    /// TODO(BodyDamage): `isNeckBleeding()`.
    pub neck_bleeding: bool,
    /// TODO(BodyDamage): `getNumPartsBitten()`.
    pub num_parts_bitten: i32,
    /// TODO(BodyDamage): `getNumPartsScratched()`.
    pub num_parts_scratched: i32,
    /// `IsInfected()`.
    pub infected: bool,
    /// `IsFakeInfected()`.
    pub fake_infected: bool,
    /// `getColdStrength()`.
    pub cold_strength: f32,
    /// `healthFromFoodTimer`.
    pub health_from_food_timer: f32,
    /// `standardHealthFromFoodTime`.
    pub standard_health_from_food_time: i32,
    /// `thermoregulator` (players only).
    pub thermoregulator: Option<Thermoregulator>,
}

impl Default for BodyDamage {
    fn default() -> Self {
        BodyDamage {
            overall_body_health: 100.0,
            num_parts_bleeding: 0,
            neck_bleeding: false,
            num_parts_bitten: 0,
            num_parts_scratched: 0,
            infected: false,
            fake_infected: false,
            cold_strength: 0.0,
            health_from_food_timer: 0.0,
            standard_health_from_food_time: 1600,
            thermoregulator: None,
        }
    }
}

impl BodyDamage {
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
        self.num_parts_bleeding
    }
    /// `isNeckBleeding`.
    pub fn is_neck_bleeding(&self) -> bool {
        self.neck_bleeding
    }
    /// `getNumPartsBitten`.
    pub fn num_parts_bitten(&self) -> i32 {
        self.num_parts_bitten
    }
    /// `getNumPartsScratched`.
    pub fn num_parts_scratched(&self) -> i32 {
        self.num_parts_scratched
    }
    /// `IsInfected`.
    pub fn is_infected(&self) -> bool {
        self.infected
    }
    /// `IsFakeInfected`.
    pub fn is_fake_infected(&self) -> bool {
        self.fake_infected
    }
    /// `getColdStrength`.
    pub fn cold_strength(&self) -> f32 {
        self.cold_strength
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
    /// `getApparentInfectionLevel`: the largest of zombie fever, zombie
    /// infection and food sickness.
    pub fn apparent_infection_level(&self, stats: &Stats, defs: &CharacterStats) -> ScriptResult<f32> {
        let infection = stats.get(&defs.field("ZOMBIE_FEVER")?).max(stats.get(&defs.field("ZOMBIE_INFECTION")?));
        Ok(stats.get(&defs.field("FOOD_SICKNESS")?).max(infection))
    }
}
