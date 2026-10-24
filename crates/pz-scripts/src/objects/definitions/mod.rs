//! Script classes of the `definitions` group: energy and fluid definitions,
//! fluid filters, timed actions and the XUI scripts.

/// Declares a Java enum with its constant names, `name()`/`toString()`,
/// `values()` and `valueOf` (with the original exception text).
macro_rules! java_enum {
    ($(#[$m:meta])* $name:ident, $class:literal, [$($v:ident),* $(,)?]) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($v),*
        }

        impl $name {
            /// `values()`, in ordinal order.
            pub const ALL: &'static [$name] = &[$($name::$v),*];

            /// `name()`.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$v => stringify!($v)),*
                }
            }

            /// `valueOf`: exact, case-sensitive constant name.
            pub fn value_of(s: &str) -> $crate::base::ScriptResult<$name> {
                Self::ALL.iter().copied().find(|v| v.name() == s).ok_or_else(|| {
                    $crate::base::ScriptError::new(format!(
                        "java.lang.IllegalArgumentException: No enum constant {}.{}",
                        $class, s
                    ))
                })
            }
        }
    };
}

mod color;
mod energy_definition;
mod fluid_definition;
mod fluid_filter;
mod timed_action;
mod xui_colors;
mod xui_config;
mod xui_layout;
mod xui_skin;

use std::rc::Rc;

pub use color::Color;
pub use energy_definition::{EnergyDefinitionScript, EnergyType};
pub use fluid_definition::{FluidCategory, FluidDefinitionScript, FluidType, PoisonEffect};
pub use fluid_filter::{FilterType, FluidFilter, FluidFilterScript};
pub use timed_action::{ActionSoundTime, TimedActionScript};
pub use xui_colors::XuiColorsScript;
pub use xui_config::{XuiConfigScript, XuiVarType, ALLOWED_VAR_TYPES};
pub use xui_layout::{read_lua_class_value, XuiLayoutScript, XuiScriptType};
pub use xui_skin::{ComponentType, ComponentUiScript, EntityUiScript, StyleInfoScript, XuiSkinScript};

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(type_: ScriptType, _module: &Rc<ScriptModule>, _name: &str, _token: &str) -> Option<ScriptRef> {
    Some(match type_ {
        ScriptType::EnergyDefinition => ScriptRef::new(EnergyDefinitionScript::new()),
        ScriptType::FluidDefinition => ScriptRef::new(FluidDefinitionScript::new()),
        ScriptType::FluidFilter => ScriptRef::new(FluidFilterScript::new()),
        ScriptType::TimedAction => ScriptRef::new(TimedActionScript::new()),
        ScriptType::XuiColor => ScriptRef::new(XuiColorsScript::new()),
        ScriptType::XuiConfig => ScriptRef::new(XuiConfigScript::new()),
        ScriptType::XuiLayout => ScriptRef::new(XuiLayoutScript::new(type_, XuiScriptType::Layout)),
        ScriptType::XuiStyle => ScriptRef::new(XuiLayoutScript::new(type_, XuiScriptType::Style)),
        ScriptType::XuiDefaultStyle => ScriptRef::new(XuiLayoutScript::new(type_, XuiScriptType::DefaultStyle)),
        ScriptType::XuiSkin => ScriptRef::new(XuiSkinScript::new()),
        _ => return None,
    })
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`).
/// The only ones are `EnergyDefinitionScript`'s lazily loaded default
/// textures. TODO(Texture)
#[derive(Default)]
pub struct Statics {}

/// Group steps run at the end of `ScriptManager.Load`.
pub fn post_load(_sm: &crate::manager::ScriptManager) {}

/// `new Exception(msg)` as logged by `ExceptionLogger`.
pub(crate) fn exception(msg: impl std::fmt::Display) -> ScriptError {
    ScriptError::new(format!("java.lang.Exception: {msg}"))
}

/// `Float.parseFloat`, with its `NumberFormatException` text.
pub(crate) fn parse_float(s: &str) -> ScriptResult<f32> {
    java::parse_f32(s).ok_or_else(|| {
        let t = java::trim(s);
        if t.is_empty() {
            ScriptError::new("java.lang.NumberFormatException: empty String")
        } else {
            ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{t}\""))
        }
    })
}

/// `Integer.parseInt`, with its `NumberFormatException` text.
pub(crate) fn parse_int(s: &str) -> ScriptResult<i32> {
    java::parse_i32(s)
        .ok_or_else(|| ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\"")))
}

/// `array[index]` past the end of a `String[]`.
pub(crate) fn array_index_error(index: usize, len: usize) -> ScriptError {
    ScriptError::new(format!("java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {len}"))
}

#[cfg(test)]
mod tests;
