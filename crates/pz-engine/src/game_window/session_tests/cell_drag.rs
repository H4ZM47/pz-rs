use super::*;
use crate::{game_loading::GameLoadingState, game_state::state, iso_world::IsoWorld};
use pz_kahlua::{value::LuaObject, NativeFunction, TableRef, Value};
use std::any::Any;
use std::cell::Cell;

struct RetiredCursor(Rc<Cell<bool>>);
impl LuaObject for RetiredCursor {
    fn class_name(&self) -> &str {
        "authored.PartialLoadCursor"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for RetiredCursor {
    fn drop(&mut self) {
        self.0
            .set(pz_lua_api::bind_world::with_world(|w| w.cell.is_none()));
    }
}

struct RestoreWorld(Option<pz_lua_api::bind_world::WorldState>);
impl Drop for RestoreWorld {
    fn drop(&mut self) {
        pz_lua_api::bind_world::set_cell(None);
        let old = self.0.take().unwrap();
        let retired = pz_lua_api::bind_world::with_world(|w| std::mem::replace(w, old));
        drop(retired);
    }
}
struct RestoreUi(Option<pz_ui::UIManager>);
impl Drop for RestoreUi {
    fn drop(&mut self) {
        let old = self.0.take().unwrap();
        let retired = pz_ui::with_ui(|ui| std::mem::replace(ui, old));
        drop(retired);
    }
}
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn closing_during_loading_retires_real_cell_and_cursor_without_deactivate_or_save() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = TempDir(std::env::temp_dir().join(format!(
        "pz-rs-partial-close-{}-{stamp}",
        std::process::id()
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let game = dir.0.join("game");
    let cache = dir.0.join("cache");
    std::fs::create_dir(&game).unwrap();
    std::fs::create_dir(&cache).unwrap();
    let _core = RestoreCore(bind_core::shared_core());
    let _world = RestoreWorld(Some(pz_lua_api::bind_world::with_world(std::mem::take)));
    let _ui = RestoreUi(Some(pz_ui::with_ui(std::mem::take)));
    bind_core::install_core(Rc::new(RefCell::new(Core::default())));
    let mut app = headless_app(Launch {
        game_dir: game,
        cache_dir: cache,
        start: Start::MainMenu,
    })
    .unwrap();
    app.lua = Some(
        LuaManager::init(Rc::new(RefCell::new(pz_fs::GameFs::new(
            &app.launch.game_dir,
        ))))
        .unwrap(),
    );
    app.states.current = Some(state(GameLoadingState::default()));
    let save_dir = dir.0.join("partial-save");
    app.world = Some(IsoWorld::new(
        &app.launch.game_dir,
        &app.launch.cache_dir,
        save_dir.clone(),
    ));
    pz_lua_api::global_objects_save::set_save_dir(Some(save_dir.clone()));
    pz_lua_api::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    let dropped = Rc::new(Cell::new(false));
    let deactivations = Rc::new(Cell::new(0));
    let calls = deactivations.clone();
    let cursor = TableRef::new();
    cursor.set_str(
        "owner",
        Value::Object(Rc::new(RetiredCursor(dropped.clone()))),
    );
    cursor.set_str(
        "deactivate",
        Value::Native(NativeFunction::new("authored-deactivate", move |_| {
            calls.set(calls.get() + 1);
            Ok(0)
        })),
    );
    pz_lua_api::bind_world::with_world(|w| {
        w.cell.as_ref().unwrap().drag.replace(0, Some(cursor));
    });
    app.close();
    assert!(app.world.is_none() && app.current_state().is_none());
    assert!(pz_lua_api::bind_world::current_cell().is_nil());
    assert!(pz_lua_api::global_objects_save::save_dir().is_none());
    assert!(
        dropped.get(),
        "final cursor owner did not observe retired cell outside world borrow"
    );
    assert_eq!(
        deactivations.get(),
        0,
        "cell retirement invented a cursor deactivate callback"
    );
    assert!(
        !save_dir.join("players.db").exists(),
        "partial close saved an unstarted session"
    );
    app.close();
    assert_eq!(deactivations.get(), 0);
}
