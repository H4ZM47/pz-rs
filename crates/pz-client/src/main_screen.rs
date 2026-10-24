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
pub fn run_headless(game_dir: &Path, cache_dir: &Path, frames: u32) -> Result<(), String> {
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(game_dir)));
    // Never fall back to the original game's ~/Zomboid.
    fs.borrow_mut().set_cache_dir(cache_dir);
    pz_lua_api::bind_core::shared_core().borrow_mut().set_cache_dir(cache_dir);
    load_scripts(game_dir, &fs.borrow())?;
    let errors: Rc<RefCell<BTreeMap<String, usize>>> = Rc::default();
    // GameWindow.init: the UI texture packs, then the pack lookup.
    pz_lua_api::bind_render::textures().borrow_mut().load_ui_packs(&fs.borrow());
    let mut mgr = LuaManager::init(fs).map_err(|e| format!("LuaManager.init failed: {e}"))?;
    let e2 = errors.clone();
    mgr.thread.log = Box::new(move |s| {
        let first = s.lines().next().unwrap_or("").to_string();
        *e2.borrow_mut().entry(first).or_default() += 1;
    });
    mgr.thread.out = Box::new(|s| println!("LOG  : {s}"));
    // GameWindow.init: Translator.loadFiles and TextManager.Init.
    {
        let core = pz_lua_api::bind_core::shared_core();
        let fs = mgr.fs.borrow();
        pz_lua_api::bind_text::boot(&fs, &core.borrow())
            .map_err(|e| format!("text init failed: {e}"))?;
    }
    UIManager::init();
    mgr.load_all_client();
    let load_failures = mgr.loader.borrow().errors.len();
    mgr.thread.log = Box::new(|s| eprintln!("LUAERR: {s}"));
    mgr.thread.on_missing_global = Some(Box::new(|n| eprintln!("MISSING-GLOBAL: {n}")));
    mgr.thread.on_missing_member = Some(Box::new(|c, k| eprintln!("MISSING-MEMBER: {c}.{k}")));
    if std::env::var_os("PZ_AUTOSTUB").is_some() {
        // Discovery aid: unknown globals resolve to a chainable stub.
        let src = r#"
            local seen = {}
            local stubmt = {}
            local function mk(path) local s = setmetatable({__stubpath = path}, stubmt) return s end
            stubmt.__index = function(t, k) if k == "__stubpath" then return nil end local p = rawget(t, "__stubpath") .. "." .. tostring(k) if not seen[p] then seen[p] = true print("STUB " .. p) end return mk(p) end
            stubmt.__call = function(t, ...) return mk(rawget(t, "__stubpath") .. "()") end
            setmetatable(_G, {__index = function(t, k) if type(k) == "string" and k:sub(1,1):match("[A-Za-z]") then local p = k if not seen[p] then seen[p] = true print("STUB " .. p) end return mk(p) end end})
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("autostub"), mgr.thread.globals().clone()).unwrap();
        mgr.thread.call(&f, &[]).unwrap();
    }
    mgr.trigger_event("OnGameBoot", &[]);
    // MainScreenState.enter
    mgr.trigger_event("OnMainMenuEnter", &[]);
    for _ in 0..frames {
        // MainScreenState.update
        mgr.trigger_event("OnFETick", &[0.0.into()]);
        // UIManager.update starts with UITransition.UpdateAll().
        pz_lua_api::bind_misc::ui_transition_update_all();
        UIManager::update(&mut mgr.thread);
        // MainScreenState.render
        UIManager::render(&mut mgr.thread);
    }
    let (n_ui, n_cmds) = pz_ui::with_ui(|m| (m.ui.len(), m.sprite.cmds.len()));
    println!("script files failing at load: {load_failures}");
    println!("UI elements: {n_ui}, draw commands last frame: {n_cmds}");
    let mut errs: Vec<_> = errors.borrow().clone().into_iter().collect();
    errs.sort_by(|a, b| b.1.cmp(&a.1));
    println!("distinct Lua errors: {}", errs.len());
    for (e, n) in errs.iter().take(100000) {
        println!("{n:5}  {e}");
    }
    Ok(())
}
