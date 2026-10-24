//! `zombie.characters.BodyDamage.Metabolics`: the constants and their `met`
//! values come from the installed jar
//! ([`pz_scripts::registry::builtin::BuiltinTables::metabolics`]); code names
//! them by their Java field.

use pz_scripts::registry::builtin::BuiltinTables;
use pz_scripts::{ScriptError, ScriptResult};

/// `Metabolics.values()` with their `met`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metabolics {
    entries: Vec<(String, f32)>,
}

impl Metabolics {
    pub fn from_tables(tables: &BuiltinTables) -> Self {
        Metabolics { entries: tables.metabolics.clone() }
    }

    /// The constants in declaration order.
    pub fn values(&self) -> &[(String, f32)] {
        &self.entries
    }

    /// `Metabolics.<field>.getMet()`.
    pub fn met(&self, field: &str) -> ScriptResult<f32> {
        self.entries
            .iter()
            .find(|(n, _)| n == field)
            .map(|(_, m)| *m)
            .ok_or_else(|| ScriptError::new(format!("java.lang.NullPointerException: Metabolics.{field}")))
    }

    /// `getMet` of `field`, 0 when the table lacks it (tables without
    /// `Metabolics`, as in tests).
    pub fn met_or_zero(&self, field: &str) -> f32 {
        self.met(field).unwrap_or(0.0)
    }
}

/// `Metabolics.MetToWm2`.
pub fn met_to_wm2(met: f32) -> f32 {
    58.0 * met
}

/// `Metabolics.MetToW`.
pub fn met_to_w(met: f32) -> f32 {
    met_to_wm2(met) * 1.8
}

/// `Metabolics.MetToBtuHr`.
pub fn met_to_btu_hr(met: f32) -> f32 {
    356.0 * met
}
