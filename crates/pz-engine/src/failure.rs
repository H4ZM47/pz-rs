//! Owned engine failure transport. Reporting never invokes a Lua formatter.
use pz_kahlua::{LuaError, Value};
use std::fmt;

pub enum EngineError {
    Report(String),
    Lua { operation: &'static str, fault: LuaError },
}

pub type EngineResult<T> = Result<T, EngineError>;

impl EngineError {
    pub fn lua(operation: &'static str, fault: LuaError) -> Self {
        Self::Lua { operation, fault }
    }
    pub fn lua_error(&self) -> Option<&LuaError> {
        match self { Self::Lua { fault, .. } => Some(fault), _ => None }
    }
    pub fn report_text(&self) -> Option<&str> {
        match self { Self::Report(text) => Some(text), _ => None }
    }
}

// An error can be replaced/rejected while a state or other host guard is held.
// Its actual Lua payload must not final-drop there, including unformatted tails.
impl Drop for EngineError {
    fn drop(&mut self) {
        if let Self::Lua { fault: LuaError::Kahlua(value), .. } = self {
            let mut retired = pz_kahlua::native_list::NativeListRetired::new();
            retired.keep_value(value);
        }
    }
}

impl From<String> for EngineError {
    fn from(text: String) -> Self { Self::Report(text) }
}
impl From<&str> for EngineError {
    fn from(text: &str) -> Self { Self::Report(text.into()) }
}
impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report(text) => f.write_str(text),
            Self::Lua { operation, fault } => {
                write!(f, "{operation}: ")?;
                match fault {
                    LuaError::Runtime { message, class } => write!(f, "{class}: {message}"),
                    LuaError::Kahlua(Value::Str(text)) => f.write_str(text),
                    // These are diagnostic labels, not String.valueOf or
                    // KahluaException.getMessage implementations.
                    LuaError::Kahlua(value) => f.write_str(match value {
                        Value::Nil => "Lua error (nil payload)",
                        Value::Bool(_) => "Lua error (boolean payload)",
                        Value::Num(_) => "Lua error (number payload)",
                        Value::Table(_) => "Lua error (table payload)",
                        Value::Closure(_) | Value::Native(_) => "Lua error (function payload)",
                        Value::Coroutine(_) => "Lua error (coroutine payload)",
                        Value::Object(_) => "Lua error (userdata payload)",
                        Value::Str(_) => unreachable!(),
                    }),
                }
            }
        }
    }
}
impl fmt::Debug for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Display::fmt(self, f) }
}
impl std::error::Error for EngineError {}

/// Synchronous first-failure handoff. Callers return immediately after recording
/// a failure; this is not an unchecked pending-error queue or catch policy.
pub(crate) fn record_first(slot: &mut Option<EngineError>, error: EngineError) {
    if slot.is_none() { *slot = Some(error); }
}

#[cfg(test)]
mod tests;
