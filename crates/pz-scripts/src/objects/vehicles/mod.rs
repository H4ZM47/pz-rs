//! Script classes of the `vehicles` group: `VehicleScript` (with its nested
//! classes), `VehicleTemplate`, and the vehicle id enums.

mod ids;
mod inner;
mod template;
mod vehicle_script;

use std::rc::Rc;

pub use ids::*;
pub use inner::{
    Anim, Area, Container, Door, ImmutableColor, LightBar, LuaKey, LuaTable, LuaValue, Model, Part, Passenger,
    PhysicsShape, Position, Skin, Sounds, SwitchSeat, Wheel, Window, PHYSICS_SHAPE_BOX, PHYSICS_SHAPE_MESH,
    PHYSICS_SHAPE_SPHERE,
};
pub use template::VehicleTemplate;
pub use vehicle_script::VehicleScript;

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::{ScriptLoadMode, ScriptType};

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(type_: ScriptType, module: &Rc<ScriptModule>, name: &str, token: &str) -> Option<ScriptRef> {
    match type_ {
        ScriptType::Vehicle => Some(ScriptRef::new(VehicleScript::new())),
        ScriptType::VehicleTemplate => Some(ScriptRef::new(VehicleTemplate::new(module, name, token))),
        _ => None,
    }
}

/// The `vehicles` bucket's `onScriptLoad` override: `script.Loaded()`.
pub fn on_script_load(sm: &ScriptManager, _mode: ScriptLoadMode, script: &ScriptRef) -> ScriptResult {
    match script.downcast::<VehicleScript>() {
        Some(v) => v.borrow_mut().loaded(sm),
        None => Ok(()),
    }
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`).
/// `VehicleScript` and `VehicleTemplate` have only constants.
#[derive(Default)]
pub struct Statics {}

/// Group steps run at the end of `ScriptManager.Load`.
pub fn post_load(_sm: &ScriptManager) {}

/// `Float.parseFloat`, with the original's exception text.
fn parse_float(s: &str) -> ScriptResult<f32> {
    java::parse_f32(s).ok_or_else(|| {
        if java::trim(s).is_empty() {
            ScriptError::new("java.lang.NumberFormatException: empty String")
        } else {
            ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\""))
        }
    })
}

/// `Integer.parseInt`, with the original's exception text.
fn parse_int(s: &str) -> ScriptResult<i32> {
    java::parse_i32(s)
        .ok_or_else(|| ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\"")))
}

/// `array[i]` on the result of `String.split`.
fn at<'a>(ss: &[&'a str], i: usize) -> ScriptResult<&'a str> {
    ss.get(i).copied().ok_or_else(|| {
        ScriptError::new(format!(
            "java.lang.ArrayIndexOutOfBoundsException: Index {i} out of bounds for length {}",
            ss.len()
        ))
    })
}

/// A `NullPointerException` from dereferencing `what`.
fn npe(what: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.NullPointerException: {what} is null"))
}

/// `StringUtils.discardNullOrWhitespace`.
fn discard_null_or_whitespace(s: &str) -> Option<String> {
    (!java::is_null_or_whitespace(Some(s))).then(|| s.to_string())
}

/// `v.isEmpty() ? null : v`.
fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests;
