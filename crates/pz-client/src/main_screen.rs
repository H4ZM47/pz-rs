//! Boot sequence and `MainScreenState` frame loop.

use pz_lua_api::LuaManager;
use pz_ui::UIManager;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

/// `GameWindow.init` + `MainScreenState.enter`, then `frames` iterations
/// of `MainScreenState.update`/`render`.
pub fn run_headless(game_dir: &Path, frames: u32) -> Result<(), String> {
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(game_dir)));
    let errors: Rc<RefCell<BTreeMap<String, usize>>> = Rc::default();
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
    for (e, n) in errs.iter().take(30) {
        println!("{n:5}  {e}");
    }
    Ok(())
}
