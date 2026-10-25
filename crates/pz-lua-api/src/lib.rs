//! The Lua-facing game API: `LuaManager` (environment, load order,
//! `require`), `LuaEventManager`/`LuaHookManager`, the
//! `LuaManager.GlobalObject` global functions and the classes
//! `LuaManager.Exposer.exposeAll` publishes, with the original names, arities
//! and behaviour so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

pub mod bind_core;
pub mod bind_debug;
pub mod bind_fs;
pub mod bind_globals;
pub mod bind_items;
pub mod bind_java_util;
pub mod bind_render;
pub mod bind_ui;
pub mod bind_vehicles;
pub mod events;
pub mod expose;
pub mod lua_manager;

pub use lua_manager::LuaManager;

/// Register all globals and exposed classes.
pub fn register_all(mgr: &mut LuaManager) {
    bind_java_util::register(&mut mgr.thread);
    bind_globals::register(&mut mgr.thread);
    // Core: binds the shared instance (`bind_core::install_core` before init).
    bind_core::register(
        &mut mgr.thread,
        Some(mgr.events.clone()),
        bind_core::shared_core(),
    );
    bind_debug::register(&mut mgr.thread);
    bind_items::register(&mut mgr.thread);
    bind_ui::register(&mut mgr.thread);
    bind_ui::install_event_sink(mgr.events.clone());
    bind_vehicles::register(&mut mgr.thread);
    bind_fs::register(mgr);
    bind_render::register(mgr);
}
