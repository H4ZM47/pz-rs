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
pub mod bind_java_util;
pub mod bind_render;
pub mod bind_text;
pub mod bind_map_groups;
pub mod bind_ui;
pub mod bind_world;
pub mod events;
pub mod expose;
pub mod lua_manager;

pub use lua_manager::LuaManager;

/// Register all globals and exposed classes.
pub fn register_all(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    bind_java_util::register(&mut mgr.thread);
    bind_globals::register(&mut mgr.thread);
    // Core: binds the shared instance (`bind_core::install_core` before init).
    bind_core::register(&mut mgr.thread, Some(mgr.events.clone()), bind_core::shared_core());
    bind_fs::register(mgr);
    bind_render::register(mgr);
    bind_text::register(&mut mgr.thread);
    bind_debug::register(&mut mgr.thread);
    bind_world::register(&mut mgr.thread);
    bind_map_groups::register(&mut mgr.thread, move || fs.borrow().media_file("maps"));
    bind_ui::register(&mut mgr.thread);
    bind_ui::install_event_sink(mgr.events.clone());
}
