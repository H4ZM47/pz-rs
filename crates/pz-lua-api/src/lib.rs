//! The Lua-facing game API: `LuaManager.GlobalObject` global functions and
//! every class `LuaManager.Exposer.exposeAll` publishes, with the same names,
//! arities and behaviour, so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

use pz_kahlua::Thread;

/// Register all globals and exposed classes.
pub fn register_all(_thread: &mut Thread) {}
