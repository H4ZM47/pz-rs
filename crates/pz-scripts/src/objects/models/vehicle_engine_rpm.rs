//! `zombie.vehicles.VehicleEngineRPM` and `EngineRPMData`.

use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::{try_parse_float, try_parse_int};

pub const MAX_GEARS: usize = 8;
const VERSION: i32 = 1;

/// `zombie.vehicles.EngineRPMData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EngineRPMData {
    pub gear_change: f32,
    pub after_gear_change: f32,
}

impl EngineRPMData {
    pub fn reset(&mut self) {
        *self = EngineRPMData::default();
    }
}

#[derive(Debug)]
pub struct VehicleEngineRPM {
    pub base: BaseScriptObject,
    name: Option<String>,
    pub rpm_data: [Option<EngineRPMData>; MAX_GEARS],
}

impl Default for VehicleEngineRPM {
    fn default() -> Self {
        Self::new()
    }
}

fn runtime(msg: String) -> ScriptError {
    ScriptError::new(format!("java.lang.RuntimeException: {msg}"))
}

/// `LoadData`.
fn load_data(block: &Block, data: &mut EngineRPMData) -> ScriptResult {
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        match k {
            "afterGearChange" => data.after_gear_change = try_parse_float(v, 0.0),
            "gearChange" => data.gear_change = try_parse_float(v, 0.0),
            _ => return Err(runtime(format!("unknown value vehicleEngineRPM.data.{}", value.string))),
        }
    }
    for child in &block.children {
        if child.type_.as_deref() != Some("xxx") {
            return Err(runtime(format!("unknown block vehicleEngineRPM.data.{}", child.type_.as_deref().unwrap_or("null"))));
        }
    }
    Ok(())
}

impl VehicleEngineRPM {
    pub fn new() -> Self {
        VehicleEngineRPM { base: BaseScriptObject::new(ScriptType::VehicleEngineRPM), name: None, rpm_data: [None; MAX_GEARS] }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl ScriptObject for VehicleEngineRPM {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.vehicles.VehicleEngineRPM"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.name = Some(name.to_string());
        let mut version = -1;
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if k != "VERSION" {
                continue;
            }
            version = try_parse_int(v, -1);
            if !(0..=VERSION).contains(&version) {
                return Err(runtime(format!("unknown vehicleEngineRPM VERSION \"{v}\"")));
            }
        }
        if version == -1 {
            return Err(runtime(format!("unknown vehicleEngineRPM VERSION \"{}\"", block.type_.as_deref().unwrap_or("null"))));
        }
        for (index, child) in block.children.iter().enumerate() {
            if child.type_.as_deref() != Some("data") {
                return Err(runtime(format!("unknown block vehicleEngineRPM.{}", child.type_.as_deref().unwrap_or("null"))));
            }
            if index == MAX_GEARS {
                return Err(runtime(format!("too many vehicleEngineRPM.data blocks, max is {MAX_GEARS}")));
            }
            let data = self.rpm_data[index].insert(EngineRPMData::default());
            load_data(child, data)?;
        }
        Ok(())
    }

    /// `reset`: zeroes existing entries without removing them.
    fn reset(&mut self) {
        for d in self.rpm_data.iter_mut().flatten() {
            d.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_gears() {
        let body = "vehicleEngineRPM E { VERSION = 1, data { gearChange = 4000, afterGearChange = 2000, } data { gearChange = 4500, } }";
        let (e, r) = load(VehicleEngineRPM::new(), "E", body);
        r.unwrap();
        assert_eq!(e.rpm_data[0], Some(EngineRPMData { gear_change: 4000.0, after_gear_change: 2000.0 }));
        assert_eq!(e.rpm_data[1].unwrap().gear_change, 4500.0);
        assert_eq!(e.rpm_data[2], None);
    }

    #[test]
    fn rejects_bad_scripts() {
        let (_, r) = load(VehicleEngineRPM::new(), "E", "vehicleEngineRPM E { }");
        assert_eq!(r.unwrap_err().0, "java.lang.RuntimeException: unknown vehicleEngineRPM VERSION \"vehicleEngineRPM\"");
        let (_, r) = load(VehicleEngineRPM::new(), "E", "vehicleEngineRPM E { VERSION = 2, }");
        assert!(r.is_err());
        let (_, r) = load(VehicleEngineRPM::new(), "E", "vehicleEngineRPM E { VERSION = 1, data { rpm = 1, } }");
        assert_eq!(r.unwrap_err().0, "java.lang.RuntimeException: unknown value vehicleEngineRPM.data. rpm = 1");
    }
}
