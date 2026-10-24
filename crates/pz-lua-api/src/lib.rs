//! The Lua-facing game API: `LuaManager` (environment, load order,
//! `require`), `LuaEventManager`/`LuaHookManager`, the
//! `LuaManager.GlobalObject` global functions and the classes
//! `LuaManager.Exposer.exposeAll` publishes, with the original names, arities
//! and behaviour so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

pub mod bind_attached_items;
pub mod bind_characters;
pub mod bind_core;
pub mod bind_globals;
pub mod bind_registry;
pub mod bind_scripts;
pub mod bind_skills;
pub mod bind_ui;
pub mod events;
pub mod expose;
pub mod lua_manager;

pub use lua_manager::LuaManager;

/// Register all globals and exposed classes.
pub fn register_all(mgr: &mut LuaManager) {
    bind_globals::register(&mut mgr.thread);
    // Core: binds the shared instance (`bind_core::install_core` before init).
    bind_core::register(&mut mgr.thread, Some(mgr.events.clone()), bind_core::shared_core());
    bind_ui::register(&mut mgr.thread);
    // Scripts: binds the shared ScriptManager (`bind_registry::install_script_manager` before init).
    bind_registry::register(&mut mgr.thread, bind_registry::shared_script_manager());
    bind_scripts::register(&mut mgr.thread, bind_registry::shared_script_manager());
    bind_characters::register_shared(&mut mgr.thread);
    bind_skills::register_shared(&mut mgr.thread);
    bind_attached_items::register_shared(&mut mgr.thread);
    bind_ui::install_event_sink(mgr.events.clone());
}
