//! Pieces of `GameWindow.init` shared by every client mode.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// The `Registries` bootstrap, `PerkFactory.init()` and
/// `ScriptManager.instance.Load()` of `GameWindow.init`, shared with the Lua
/// API. TODO: mod script folders, `CustomPerks`.
pub fn load_scripts(game_dir: &Path, fs: &pz_fs::GameFs) -> Result<(), String> {
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
    pz_lua_api::bind_registry::install_script_manager(Rc::new(sm));
    // The `ClothingItem` assets item visuals read (`instanceItem`).
    match pz_inventory::ItemAssets::load(game_dir) {
        Ok(a) => pz_lua_api::bind_items::install_item_assets(a),
        Err(e) => eprintln!("item assets: {e}"),
    }
    Ok(())
}
