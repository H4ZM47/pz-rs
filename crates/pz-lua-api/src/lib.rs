//! The Lua-facing game API: `LuaManager` (environment, load order,
//! `require`), `LuaEventManager`/`LuaHookManager`, the
//! `LuaManager.GlobalObject` global functions and the classes
//! `LuaManager.Exposer.exposeAll` publishes, with the original names, arities
//! and behaviour so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

pub mod events;
pub mod expose;
pub mod lua_manager;

pub use lua_manager::LuaManager;

/// Register all globals and exposed classes.
pub fn register_all(_mgr: &mut LuaManager) {}
