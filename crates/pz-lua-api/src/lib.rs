//! The Lua-facing game API: `LuaManager` (environment, load order,
//! `require`), `LuaEventManager`/`LuaHookManager`, the
//! `LuaManager.GlobalObject` global functions and the classes
//! `LuaManager.Exposer.exposeAll` publishes, with the original names, arities
//! and behaviour so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

pub mod bind_characters;
pub mod bind_core;
pub mod bind_fs;
pub mod bind_debug;
pub mod bind_globals;
pub mod bind_java_util;
pub mod bind_misc;
pub mod bind_misc_classes;
pub mod bind_mods;
pub mod bind_render;
pub mod bind_sandbox;
pub mod bind_text;
pub mod bind_map_groups;
pub mod bind_registry;
pub mod bind_save;
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
    bind_save::register(mgr);
    // Text before UI: bind_text's UIFont is canonical.
    bind_text::register(&mut mgr.thread);
    bind_debug::register(&mut mgr.thread);
    bind_mods::register(mgr);
    bind_world::register(&mut mgr.thread);
    bind_map_groups::register(&mut mgr.thread, move || fs.borrow().media_file("maps"));
    bind_misc::register(&mut mgr.thread, mgr.fs.clone(), mgr.events.clone());
    bind_sandbox::register(&mut mgr.thread, mgr.fs.clone(), mgr.loader.clone());
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        bind_misc_classes::register(&mut mgr.thread, jar.as_ref());
    }
    // Scripts: binds the shared ScriptManager (`bind_registry::install_script_manager` before init).
    bind_registry::register(&mut mgr.thread, bind_registry::shared_script_manager());
    bind_characters::register_shared(&mut mgr.thread);
    bind_ui::register(&mut mgr.thread);
    bind_ui::install_event_sink(mgr.events.clone());
}
