//! Port of `zombie.scripting`: the parser for the game's script files
//! (`media/scripts/**/*.txt`) and `ScriptManager`, which loads items,
//! recipes, vehicles, models, sounds and the other script types from the
//! game and its mods, with the original's lookup and override rules.

pub mod base;
pub mod bucket;
pub mod java;
pub mod manager;
pub mod math;
pub mod module;
pub mod objects;
pub use pz_script_parser as parser;
pub mod registry;
pub mod script_type;

pub use base::{BaseScriptObject, ScriptError, ScriptObject, ScriptRef, ScriptResult};
pub use manager::{ModScriptDirs, ScriptHooks, ScriptManager};
pub use module::ScriptModule;
pub use script_type::{Flag, ScriptLoadMode, ScriptType};
