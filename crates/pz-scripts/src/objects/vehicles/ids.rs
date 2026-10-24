//! Key records of `zombie.scripting.objects` naming vehicle scripts. The
//! vehicle enums (`VehicleArea`, `VehiclePart`, ...) are read from the game
//! jar at runtime ([`crate::registry::builtin::BuiltinTables`]).

use std::fmt;

/// `record VehicleKey(String id)`. The original's constants for vanilla
/// vehicles are not copied here; read them from the game jar if needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleKey(pub &'static str);

impl VehicleKey {
    /// `id()` / `toString`.
    pub fn id(self) -> &'static str {
        self.0
    }

    /// `toStringBase`: `"Base." + id`.
    pub fn to_string_base(self) -> String {
        format!("Base.{}", self.0)
    }
}

impl fmt::Display for VehicleKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// `record VehicleTemplateKey(String id)` (constants not copied, as above).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleTemplateKey(pub &'static str);

impl VehicleTemplateKey {
    /// `id()` / `toString`.
    pub fn id(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for VehicleTemplateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// `VehiclePartModel`: a part's model reference, used from Lua.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VehiclePartModel {
    pub part_id: Option<String>,
    pub part_model_id: Option<String>,
    pub model_id: Option<String>,
}

