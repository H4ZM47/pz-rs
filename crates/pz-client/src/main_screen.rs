//! Boot sequence and `MainScreenState` frame loop.

use pz_lua_api::LuaManager;
use pz_ui::UIManager;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

/// The `Registries` bootstrap, `PerkFactory.init()` and
/// `ScriptManager.instance.Load()` of `GameWindow.init`, shared with the Lua
/// API. TODO: mod script folders, `CustomPerks`.
fn load_scripts(game_dir: &Path, fs: &pz_fs::GameFs) -> Result<(), String> {
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game_dir.join("projectzomboid.jar"))
        .map_err(|e| format!("built-in registries: {e}"))?;
    let mut chars = pz_characters::Characters::default();
    {
        let tables = sm.builtin_tables.borrow();
        chars.perks = pz_characters::PerkFactory::from_tables(&tables).map_err(|e| format!("PerkFactory.Perks: {e}"))?;
        chars.perks.init(&tables, &pz_characters::Characters::translate).map_err(|e| format!("PerkFactory.init: {e}"))?;
    }
    pz_lua_api::bind_characters::install_characters(Rc::new(RefCell::new(chars)));
    sm.load(fs, &[]).map_err(|e| format!("ScriptManager.Load failed: {e}"))?;
    pz_lua_api::bind_registry::install_script_manager(Rc::new(sm));
    Ok(())
}

/// `GameWindow.init` + `MainScreenState.enter`, then `frames` iterations
/// of `MainScreenState.update`/`render`.
pub fn run_headless(game_dir: &Path, frames: u32) -> Result<(), String> {
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(game_dir)));
    let errors: Rc<RefCell<BTreeMap<String, usize>>> = Rc::default();
    // GameWindow.init: the UI texture packs, then the pack lookup.
    pz_lua_api::bind_render::textures().borrow_mut().load_ui_packs(&fs.borrow());
    load_scripts(game_dir, &fs.borrow())?;
    let mut mgr = LuaManager::init(fs).map_err(|e| format!("LuaManager.init failed: {e}"))?;
    let e2 = errors.clone();
    mgr.thread.log = Box::new(move |s| {
        let first = s.lines().next().unwrap_or("").to_string();
        *e2.borrow_mut().entry(first).or_default() += 1;
    });
    mgr.thread.out = Box::new(|s| println!("LOG  : {s}"));
    UIManager::init();
    mgr.load_all_client();
    let load_failures = mgr.loader.borrow().errors.len();
    mgr.trigger_event("OnGameBoot", &[]);
    // MainScreenState.enter
    mgr.trigger_event("OnMainMenuEnter", &[]);
    for _ in 0..frames {
        // MainScreenState.update
        mgr.trigger_event("OnFETick", &[0.0.into()]);
        UIManager::update(&mut mgr.thread);
        // MainScreenState.render
        UIManager::render(&mut mgr.thread);
    }
    let (n_ui, n_cmds) = pz_ui::with_ui(|m| (m.ui.len(), m.sprite.cmds.len()));
    println!("script files failing at load: {load_failures}");
    println!("UI elements: {n_ui}, draw commands last frame: {n_cmds}");
    let mut errs: Vec<_> = errors.borrow().clone().into_iter().collect();
    errs.sort_by_key(|e| std::cmp::Reverse(e.1));
    println!("distinct Lua errors: {}", errs.len());
    for (e, n) in errs.iter().take(100000) {
        println!("{n:5}  {e}");
    }
    Ok(())
}
