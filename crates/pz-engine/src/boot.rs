//! Pieces of `GameWindow.init` shared by every client mode.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// The development game entry skips `MainScreen.create`, which normally
/// runs `MainOptions.loadKeys` before a game can start. Run that same Lua
/// loader so defaults, the user's saved bindings and keyboard layout are
/// applied before the first gameplay input snapshot.
pub fn load_key_bindings(lua: &mut pz_lua_api::LuaManager) -> Result<(), String> {
    let options = lua.thread.globals().get_str("MainOptions");
    let options = options.as_table().ok_or("MainOptions is unavailable while loading key bindings")?;
    let load = options.get_str("loadKeys");
    if load.is_nil() {
        return Err("MainOptions.loadKeys is unavailable while loading key bindings".into());
    }
    lua.thread
        .pcall(&load, &[])
        .map(|_| ())
        .map_err(|(error, trace)| format!("MainOptions.loadKeys: {error:?}\n{trace}"))
}

/// The `Registries` bootstrap, `PerkFactory.init()`,
/// `ScriptManager.instance.Load()` and the clothing loads after it
/// (`ClothingDecals`, `BeardStyles`, `HairStyles`, `OutfitManager`,
/// `VoiceStyles`, with `ModelManager`'s `PopTemplateManager.init`) of
/// `GameWindow.initShared`, shared with the Lua API; then the `ClothingItem`
/// assets item visuals read. TODO: mod script folders, `CustomPerks`.
pub fn load_scripts(game_dir: &Path, fs: &mut pz_fs::GameFs) -> Result<(), String> {
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game_dir.join("projectzomboid.jar"))
        .map_err(|e| format!("built-in registries: {e}"))?;
    let mut chars = pz_characters::Characters::default();
    {
        let tables = sm.builtin_tables.borrow();
        chars.perks = pz_characters::PerkFactory::from_tables(&tables).map_err(|e| format!("PerkFactory.Perks: {e}"))?;
        chars.perks.init(&tables, &pz_characters::Characters::translate).map_err(|e| format!("PerkFactory.init: {e}"))?;
        chars.sim = pz_characters::SimStatics::from_tables(&tables).map_err(|e| format!("character statics: {e}"))?;
    }
    pz_lua_api::bind_characters::install_characters(Rc::new(RefCell::new(chars)));
    sm.load(fs, &[]).map_err(|e| format!("ScriptManager.Load failed: {e}"))?;
    // The entity registries items' factory components (`FluidContainer`)
    // resolve against.
    let entity = pz_lua_api::bind_items::item_entity_context(&game_dir.join("projectzomboid.jar"), &sm)
        .map_err(|e| eprintln!("item entity registries: {e}"))
        .ok();
    pz_lua_api::bind_registry::install_script_manager(Rc::new(sm));
    let mut pop = pz_characters::population::Population::default();
    pop.init(fs);
    pz_lua_api::bind_appearance::install_population(Rc::new(RefCell::new(pop)));
    // The `ClothingItem` assets item visuals read (`instanceItem`).
    let assets = pz_inventory::ItemAssets::load(game_dir)
        .map_err(|e| eprintln!("item assets: {e}"))
        .ok();
    pz_lua_api::bind_items::install_item_runtime(assets, entity);
    Ok(())
}
