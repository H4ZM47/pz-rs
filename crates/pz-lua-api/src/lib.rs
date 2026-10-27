//! The Lua-facing game API: `LuaManager` (environment, load order,
//! `require`), `LuaEventManager`/`LuaHookManager`, the
//! `LuaManager.GlobalObject` global functions and the classes
//! `LuaManager.Exposer.exposeAll` publishes, with the original names, arities
//! and behaviour so vanilla scripts and mods run unchanged.
//!
//! Each exposed class gets a global table (statics, `new`, `class`) and a
//! metatable in `Thread::class_metatables` keyed by its Java class name.

pub mod bind_appearance;
pub mod bind_game_window;
pub mod bind_attached_items;
pub mod bind_body;
pub mod bind_body_damage;
pub mod bind_character_sim;
pub mod bind_characters;
pub mod bind_audio;
pub mod bind_erosion;
pub mod bind_colors;
pub mod bind_core;
pub mod bind_date;
pub mod bind_credits;
pub mod bind_debug;
pub mod bind_climate;
pub mod bind_puddles;
pub mod bind_process_items;
pub mod item_sound;
pub mod bind_combat;
pub mod bind_input;
pub mod bind_joypad;
pub mod bind_fs;
pub mod bind_global_objects;
pub mod bind_globals;
pub mod bind_item_container;
pub mod bind_world_object;
pub(crate) mod bind_item_age;
pub mod bind_item_name;
pub mod bind_item_visual;
pub mod bind_items;
pub mod bind_fluid_container;
pub mod component_gc;
pub mod attribute_templates;
pub mod item_tooltip;
pub mod bind_iso_enums;
pub mod bind_java_util;
pub mod bind_misc;
pub mod bind_misc_classes;
pub mod bind_mods;
pub mod bind_nutrition;
pub mod bind_players;
pub mod bind_popman;
pub mod bind_registry;
pub mod bind_render;
pub mod bind_sandbox;
pub mod bind_save;
pub mod bind_scripts;
pub mod bind_search_mode;
pub mod bind_skills;
pub mod bind_survival_guide;
pub mod bind_states;
pub mod bind_zombie;
pub mod bind_zombie_outfits;
pub mod chat;
pub mod bind_text;
pub mod bind_ui;
pub mod gizmo;
pub mod hud;
pub mod bind_ui3d;
pub mod bind_uitextbox2;
pub mod bind_voice;
pub mod bind_map_symbols;
pub mod bind_server;
pub mod bind_server_list;
pub mod bind_map_groups;
pub mod bind_world;
pub mod bind_basements;
pub mod bind_bent_fences;
pub mod bind_map_objects;
pub mod bind_tile_overlays;
pub mod bind_broken_fences;
pub mod bind_key_namer;
pub mod bind_world_gen_utils;
pub mod bind_world_gen_params;
pub mod bind_randomized_world;
pub mod bind_sprites;
pub mod bind_vehicles;
pub mod bind_video;
pub mod events;
pub mod expose;
pub mod global_objects;
pub mod global_objects_save;
pub mod lua_manager;
pub mod player_db;
pub mod savefile;
pub mod session_reset;
pub mod world_save;
pub mod world_mannequins;
pub mod timed_actions;
pub mod worldmap;

pub use lua_manager::LuaManager;
pub use session_reset::reset_session_bindings;

use std::rc::Rc;

fn component_debug_query() -> pz_entity::runtime_load::HostResult<bool> {
    let core = bind_core::shared_core();
    let core = core.try_borrow().map_err(|_| {
        pz_entity::runtime_load::RuntimeHostError::Borrowed("Core.debug")
    })?;
    Ok(core.get_debug())
}

fn component_native_notice(notice: &pz_entity::runtime_fluid::RuntimeNativeNotice) -> pz_entity::runtime_load::HostResult<()> {
    match notice {
        pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralLog(message) => pz_core::debug::log(message),
        pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralError(message) =>
            pz_core::debug::DebugType::General.error(None, message),
        pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralException(message) =>
            pz_core::debug::with(|log| log.print_exception(
                pz_core::debug::DebugType::General,
                Some(&pz_core::debug::LogThrowable::new(message)),
                None,
                "",
                pz_core::debug::LogSeverity::Error,
            )),
        pz_entity::runtime_fluid::RuntimeNativeNotice::ExceptionLogger(message) =>
            pz_core::debug::log_exception(&pz_core::debug::LogThrowable::new(message), None),
    }
    Ok(())
}

fn component_script_cache(is_item: bool, name: &str) -> pz_entity::runtime_load::HostResult<Option<pz_entity::EntityOwner>> {
    let scripts = bind_registry::shared_script_manager();
    let owner = if is_item {
        scripts.get::<pz_scripts::objects::items::Item>(pz_scripts::ScriptType::Item, name)
            .map(|script| pz_entity::EntityOwner::from_ref(&script))
    } else {
        scripts.get::<pz_scripts::objects::entity::GameEntityScript>(pz_scripts::ScriptType::Entity, name)
            .map(|script| pz_entity::EntityOwner::from_ref(&script))
    };
    Ok(owner)
}

fn component_fluid_owner(owner: &pz_entity::EntityOwner) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    if let Some(item) = owner.try_handle::<pz_inventory::InventoryItem>() {
        return bind_item_container::on_fluid_update(&item);
    }
    if let Some(object) = owner.try_handle::<pz_world::IsoObject>() {
        let trough = {
            let object = object.try_borrow().map_err(|_| RuntimeHostError::Borrowed("fluid world owner class"))?;
            matches!(object.kind, pz_world::ObjectKind::FeedingTrough(_))
        };
        if trough {
            // The native override runs real container/event/linked-world/
            // overlay/network phases. Do not substitute the inherited no-op.
            return Err(RuntimeHostError::Unsupported {
                operation: "IsoFeedingTrough.onFluidContainerUpdate container/overlay phases",
                kind: Some(pz_entity::ComponentType::FluidContainer),
            });
        }
        // The authenticated current JAR declares no other world-object
        // override of GameEntity's exact return-only implementation.
        return Ok(());
    }
    Err(RuntimeHostError::Unsupported {
        operation: "actual fluid component owner type",
        kind: Some(pz_entity::ComponentType::FluidContainer),
    })
}

/// Register all globals and exposed classes.
pub fn register_all(mgr: &mut LuaManager) {
    component_gc::install_observers();
    pz_inventory::component_host::install_component_debug_query(Some(component_debug_query));
    pz_inventory::component_host::install_native_notice_logger(Some(component_native_notice));
    pz_inventory::component_host::install_script_cache_resolver(Some(component_script_cache));
    pz_inventory::component_host::install_item_fluid_observer(Some(bind_item_container::on_fluid_update));
    pz_inventory::component_host::install_fluid_owner_observer(Some(component_fluid_owner));
    // A new Lua session must not retain the previous session's sandbox
    // provider. Native generator range latches have a separate lifetime.
    bind_world::power::install_sandbox(None);
    let fs = mgr.fs.clone();
    let events = mgr.events.clone();
    let sink = move |t: &mut pz_kahlua::Thread, name: &str, a: &[pz_kahlua::Value]| {
        events.trigger(t, name, a);
    };
    let sink: bind_climate::EventSink = Rc::new(sink);
    bind_java_util::register(&mut mgr.thread);
    bind_globals::register(&mut mgr.thread);
    bind_players::register(&mut mgr.thread);
    bind_search_mode::register(&mut mgr.thread);
    // Core: binds the shared instance (`bind_core::install_core` before init).
    bind_core::register(&mut mgr.thread, Some(mgr.events.clone()), bind_core::shared_core());
    bind_core::register(
        &mut mgr.thread,
        Some(mgr.events.clone()),
        bind_core::shared_core(),
    );
    bind_fs::register(mgr);
    bind_render::register(mgr);
    bind_video::register(mgr);
    bind_save::register(mgr);
    // Text before UI: bind_text's UIFont is canonical.
    bind_text::register(&mut mgr.thread);
    bind_date::register(&mut mgr.thread);
    bind_credits::register(mgr);
    bind_debug::register(&mut mgr.thread);
    bind_input::register(mgr);
    bind_joypad::register(mgr);
    bind_erosion::register(&mut mgr.thread);
    bind_fluid_container::register(&mut mgr.thread);
    bind_items::register(&mut mgr.thread);
    bind_item_age::install_events(mgr.events.clone());
    bind_item_container::register(&mut mgr.thread);
    bind_misc::register(&mut mgr.thread, mgr.fs.clone(), mgr.events.clone());
    bind_sandbox::register(&mut mgr.thread, mgr.fs.clone(), mgr.loader.clone());
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        bind_misc_classes::register(&mut mgr.thread, jar.as_ref());
        if let Some(jar) = jar.as_ref() { timed_actions::register_action_anims(&mut mgr.thread, jar); }
    }
    // Scripts: binds the shared ScriptManager (`bind_registry::install_script_manager` before init).
    bind_registry::register(&mut mgr.thread, bind_registry::shared_script_manager());
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        bind_survival_guide::register(&mut mgr.thread, jar.as_ref());
    }
    // Before bind_scripts, whose enum fallbacks skip classes registered here.
    bind_body_damage::register_shared(&mut mgr.thread);
    bind_states::register_shared(&mut mgr.thread);
    bind_scripts::register(&mut mgr.thread, bind_registry::shared_script_manager());
    bind_characters::register_shared(&mut mgr.thread);
    bind_skills::register_shared(&mut mgr.thread);
    bind_character_sim::register_shared(&mut mgr.thread);
    timed_actions::register(&mut mgr.thread);
    bind_body::register_shared(&mut mgr.thread);
    bind_nutrition::register_shared(&mut mgr.thread);
    bind_attached_items::register_shared(&mut mgr.thread);
    bind_appearance::register(&mut mgr.thread);
    bind_item_visual::register(&mut mgr.thread);
    bind_climate::register(mgr);
    bind_puddles::register(&mut mgr.thread);
    bind_mods::register(mgr);
    bind_map_symbols::register(mgr);
    bind_world::register_with_events(&mut mgr.thread, mgr.events.clone());
    bind_world::lua_world::register(&mut mgr.thread, mgr.fs.clone());
    bind_basements::register(&mut mgr.thread);
    bind_bent_fences::register(&mut mgr.thread);
    bind_map_objects::register(&mut mgr.thread);
    bind_tile_overlays::register(&mut mgr.thread);
    bind_broken_fences::register(&mut mgr.thread);
    bind_key_namer::register(&mut mgr.thread);
    bind_world_gen_utils::register(&mut mgr.thread, mgr.fs.clone());
    bind_world_gen_params::register(&mut mgr.thread);
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        bind_randomized_world::register_from_jar(&mut mgr.thread, jar.as_ref());
    }
    bind_sprites::register(&mut mgr.thread);
    bind_map_groups::register(&mut mgr.thread, move || fs.borrow().media_file("maps"));
    bind_server::register(&mut mgr.thread, mgr.fs.clone(), mgr.loader.clone());
    bind_server_list::register(&mut mgr.thread, mgr.fs.clone());
    bind_audio::register(&mut mgr.thread);
    bind_iso_enums::register(mgr);
    bind_global_objects::register(mgr);
    bind_popman::register(&mut mgr.thread);
    bind_zombie::register_shared(&mut mgr.thread);
    bind_vehicles::register(&mut mgr.thread);
    bind_ui::register(&mut mgr.thread);
    hud::register(&mut mgr.thread);
    worldmap::register(&mut mgr.thread);
    worldmap::install_fs(mgr.fs.clone());
    gizmo::register(&mut mgr.thread);
    hud::characters::install_character_hud_hosts();
    hud::alarm_clocks::install_alarm_clock_host();
    bind_ui3d::register(&mut mgr.thread);
    bind_uitextbox2::register(&mut mgr.thread);
    bind_voice::register(&mut mgr.thread);
    bind_voice::register_input_globals(&mut mgr.thread);
    chat::lua::register(&mut mgr.thread);
    chat::characters::register(&mut mgr.thread);
    chat::characters::install_character_chat_players();
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        chat::characters::set_static_sounds(jar.as_ref().map(chat::characters::static_sounds_from_jar).unwrap_or_default());
    }
    {
        let jar = pz_classfile::Jar::open(mgr.fs.borrow().base.join("projectzomboid.jar")).ok();
        let cache = mgr.fs.borrow().cache_dir().to_path_buf();
        bind_game_window::register(&mut mgr.thread, jar.as_ref(), &cache);
    }
    bind_ui::install_event_sink(mgr.events.clone());
    bind_character_sim::install_event_sink(sink.clone());
    chat::install_event_sink(sink.clone());
    bind_climate::install_event_sink(sink);
}
