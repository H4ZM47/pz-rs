//! Host references retired when the engine explicitly rebuilds Lua after
//! leaving a world. This is separate from LuaManager::init so creating a
//! temporary/synthetic manager does not tear down a running session.

/// Retire session UI, callbacks and canonical handles before bootstrap
/// replaces ScriptManager/Characters and constructs the fresh Lua manager.
/// Call only after the active world has stopped and no Lua callback runs.
/// The engine must reinstall text rendering and load shared/client Lua.
/// Core preferences, the texture manager and persistent renderer owners
/// stay with the native application.
pub fn reset_session_bindings() {
    crate::bind_uitextbox2::set_current(None);
    pz_ui::UIManager::reset();
    crate::bind_map_objects::reset();
    for side in [crate::global_objects::Side::Client, crate::global_objects::Side::Server] {
        crate::global_objects::reset(side);
    }

    crate::bind_players::reset();
    crate::bind_character_sim::reset_session_handles();
    crate::bind_states::reset_session_handles();
    crate::bind_body::reset_session_handles();
    crate::bind_nutrition::reset_session_handles();
    crate::bind_characters::reset_session_handles();
    crate::bind_scripts::reset_session_handles();
    crate::bind_registry::reset_session_handles();
    crate::bind_item_visual::reset_session_handles();
    crate::chat::reset_session_callbacks();
    crate::bind_climate::reset_session_handles();
    crate::bind_puddles::reset_session_handles();

    crate::bind_tile_overlays::shared_tile_overlays().borrow_mut().reset();
    crate::bind_tile_overlays::shared_container_overlays().borrow_mut().reset();
    // The binding's BentFences.Reset clears these definition collections;
    // per-world sandbox values are populated when the next world loads.
    {
        let fences = crate::bind_bent_fences::shared_bent_fences();
        let mut fences = fences.borrow_mut();
        fences.entries.clear();
        fences.fence_map.clear();
    }
    crate::bind_broken_fences::shared_broken_fences().borrow_mut().reset();
    crate::bind_basements::before_on_load_map_zones();
    crate::bind_sprites::install_sprite_manager(None);
    crate::bind_erosion::set_instance(None);
    crate::bind_world::set_cell(None);
    crate::bind_world::set_meta_grid(None);
}
