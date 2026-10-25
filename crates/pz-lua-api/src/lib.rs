//! The Lua-facing game API: `LuaManager` (environment, load order,
//! `require`), `LuaEventManager`/`LuaHookManager`, the
//! `LuaManager.GlobalObject` global functions and the classes
//! `LuaManager.Exposer.exposeAll` publishes, with the original names, arities
//! and behaviour so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

pub mod bind_appearance;
pub mod bind_attached_items;
pub mod bind_body;
pub mod bind_body_damage;
pub mod bind_character_sim;
pub mod bind_characters;
pub mod bind_audio;
pub mod bind_erosion;
pub mod bind_core;
pub mod bind_debug;
pub mod bind_climate;
pub mod bind_combat;
pub mod bind_input;
pub mod bind_joypad;
pub mod bind_fs;
pub mod bind_global_objects;
pub mod bind_globals;
pub mod bind_items;
pub mod bind_iso_enums;
pub mod bind_java_util;
pub mod bind_misc;
pub mod bind_misc_classes;
pub mod bind_mods;
pub mod bind_nutrition;
pub mod bind_popman;
pub mod bind_registry;
pub mod bind_render;
pub mod bind_sandbox;
pub mod bind_save;
pub mod bind_scripts;
pub mod bind_skills;
pub mod bind_states;
pub mod bind_zombie;
pub mod bind_text;
pub mod bind_ui;
pub mod bind_map_symbols;
pub mod bind_server;
pub mod bind_map_groups;
pub mod bind_world;
pub mod events;
pub mod expose;
pub mod global_objects;
pub mod global_objects_save;
pub mod lua_manager;
pub mod player_db;
pub mod savefile;

pub use lua_manager::LuaManager;

use std::rc::Rc;

/// Register all globals and exposed classes.
pub fn register_all(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    let events = mgr.events.clone();
    let sink = move |t: &mut pz_kahlua::Thread, name: &str, a: &[pz_kahlua::Value]| {
        events.trigger(t, name, a);
    };
    let sink: bind_climate::EventSink = Rc::new(sink);
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
    bind_input::register(mgr);
    bind_joypad::register(mgr);
    bind_erosion::register(&mut mgr.thread);
    bind_items::register(&mut mgr.thread);
    bind_misc::register(&mut mgr.thread, mgr.fs.clone(), mgr.events.clone());
    bind_sandbox::register(&mut mgr.thread, mgr.fs.clone(), mgr.loader.clone());
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        bind_misc_classes::register(&mut mgr.thread, jar.as_ref());
    }
    // Scripts: binds the shared ScriptManager (`bind_registry::install_script_manager` before init).
    bind_registry::register(&mut mgr.thread, bind_registry::shared_script_manager());
    // Before bind_scripts, whose enum fallbacks skip classes registered here.
    bind_body_damage::register_shared(&mut mgr.thread);
    bind_states::register_shared(&mut mgr.thread);
    bind_scripts::register(&mut mgr.thread, bind_registry::shared_script_manager());
    bind_characters::register_shared(&mut mgr.thread);
    bind_skills::register_shared(&mut mgr.thread);
    bind_character_sim::register_shared(&mut mgr.thread);
    bind_body::register_shared(&mut mgr.thread);
    bind_nutrition::register_shared(&mut mgr.thread);
    bind_attached_items::register_shared(&mut mgr.thread);
    bind_appearance::register(&mut mgr.thread);
    bind_climate::register(mgr);
    bind_mods::register(mgr);
    bind_map_symbols::register(mgr);
    bind_world::register(&mut mgr.thread);
    bind_map_groups::register(&mut mgr.thread, move || fs.borrow().media_file("maps"));
    bind_server::register(&mut mgr.thread, mgr.fs.clone(), mgr.loader.clone());
    bind_audio::register(&mut mgr.thread);
    bind_iso_enums::register(mgr);
    bind_global_objects::register(mgr);
    bind_popman::register(&mut mgr.thread);
    bind_zombie::register_shared(&mut mgr.thread);
    bind_ui::register(&mut mgr.thread);
    bind_ui::install_event_sink(mgr.events.clone());
    bind_character_sim::install_event_sink(sink.clone());
    bind_climate::install_event_sink(sink);
}
