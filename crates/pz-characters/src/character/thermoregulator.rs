//! `zombie.characters.BodyDamage.Thermoregulator`.
//!
//! **Slot for the Thermoregulator port** (players only, owned by
//! [`super::body_damage::BodyDamage::thermoregulator`]). It will take the
//! climate through [`super::context::SimContext`] (add the
//! `ClimateManager` readings there). Only what the ported code reads exists
//! so far; the port keeps these method names.

/// `Thermoregulator`.
#[derive(Clone, Debug)]
pub struct Thermoregulator {
    /// `thermalChevronUp`.
    pub thermal_chevron_up: bool,
    /// TODO(Thermoregulator): `thermalChevronCount()` derives this from
    /// `coreRateOfChange`.
    pub thermal_chevron_count: i32,
    /// `fluidsMultiplier`.
    pub fluids_multiplier: f64,
    /// `fatigueMultiplier`.
    pub fatigue_multiplier: f64,
}

impl Default for Thermoregulator {
    fn default() -> Self {
        Thermoregulator {
            thermal_chevron_up: true,
            thermal_chevron_count: 0,
            fluids_multiplier: 1.0,
            fatigue_multiplier: 1.0,
        }
    }
}

impl Thermoregulator {
    /// `thermalChevronUp`.
    pub fn thermal_chevron_up(&self) -> bool {
        self.thermal_chevron_up
    }
    /// `thermalChevronCount`.
    pub fn thermal_chevron_count(&self) -> i32 {
        self.thermal_chevron_count
    }
    /// `getFluidsMultiplier`.
    pub fn fluids_multiplier(&self) -> f64 {
        self.fluids_multiplier
    }
    /// `getFatigueMultiplier`.
    pub fn fatigue_multiplier(&self) -> f64 {
        self.fatigue_multiplier
    }
}
