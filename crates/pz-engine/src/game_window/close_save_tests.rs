//! Device-free tests of the actual App.close save and post-save dispatch path.
//! A marker state selects that path and observes its prefix before teardown;
//! these tests do not claim coverage of IngameState.exit's other systems.

use super::*;
use crate::game_state::{state, GameState};
use pz_core::game_time::GameTime;
use pz_kahlua::{NativeFunction, Value};
use pz_lua_api::bind_world::{cell_generation, set_cell, set_meta_grid, with_world, WorldState};
use pz_world::{IsoCell, IsoMetaGrid};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ExitWitness {
    phases: Rc<RefCell<Vec<String>>>,
    errors: Rc<RefCell<Vec<String>>>,
}

impl GameState for ExitWitness {
    fn class_name(&self) -> &'static str {
        "zombie.gameStates.IngameState"
    }

    fn exit(&mut self, cx: &mut StateCx<'_>) {
        self.phases.borrow_mut().push("exit".into());
        if let Some(world) = cx.world.as_ref() {
            *self.errors.borrow_mut() = world.errors.clone();
        }
    }
}

struct Fixture {
    app: App,
    dir: PathBuf,
    old_core: Rc<RefCell<Core>>,
    old_time: Rc<RefCell<GameTime>>,
    old_world: WorldState,
    phases: Rc<RefCell<Vec<String>>>,
    errors: Rc<RefCell<Vec<String>>>,
}

impl Fixture {
    fn new(no_save: bool, has_cell: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pz-engine-close-cell-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let old_core = pz_lua_api::bind_core::shared_core();
        let old_time = pz_lua_api::bind_climate::shared_game_time();
        let old_world = with_world(std::mem::take);
        let mut core = Core::default();
        core.set_no_save(no_save);
        core.set_game_mode("Sandbox");
        let core = Rc::new(RefCell::new(core));
        pz_lua_api::bind_core::install_core(core.clone());
        pz_lua_api::bind_climate::install_game_time(Rc::new(RefCell::new(GameTime::default())));
        pz_lua_api::bind_players::reset();
        set_meta_grid(Some(IsoMetaGrid::with_bounds(0, 0, 1, 1)));
        set_cell(has_cell.then(IsoCell::default));
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&dir)));
        fs.borrow_mut().set_cache_dir(&dir);
        let lua = LuaManager::init(fs).unwrap();
        let phases = Rc::new(RefCell::new(Vec::new()));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mut states = GameStateMachine::default();
        states.current = Some(state(ExitWitness {
            phases: phases.clone(),
            errors: errors.clone(),
        }));
        let world = IsoWorld::new(&dir, &dir, dir.clone());
        let app = App {
            launch: Launch {
                game_dir: dir.clone(),
                cache_dir: dir.clone(),
                start: Start::MainMenu,
            },
            window: None,
            gpu: None,
            lua: Some(lua),
            core,
            states,
            world: Some(world),
            error: None,
            retired_models: Vec::new(),
            last_frame: Instant::now(),
            gamepads: None,
            audio: None,
            ui_models: None,
        };
        Self {
            app,
            dir,
            old_core,
            old_time,
            old_world,
            phases,
            errors,
        }
    }

    fn lua(&mut self, source: &str) {
        let lua = self.app.lua.as_mut().unwrap();
        let function = pz_kahlua::stdlib::load_string(
            source,
            Some("close-current-cell-test"),
            lua.thread.globals().clone(),
        )
        .unwrap();
        lua.thread.pcall(&function, &[]).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        with_world(|world| *world = std::mem::take(&mut self.old_world));
        pz_lua_api::bind_core::install_core(self.old_core.clone());
        pz_lua_api::bind_climate::install_game_time(self.old_time.clone());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[derive(Clone, Copy, Debug)]
enum FirstCallback {
    Keep,
    Clear,
    Replace,
}

#[test]
fn close_rereads_current_cell_for_each_post_save_block() {
    for (has_cell, action, expected) in [
        (true, FirstCallback::Keep, 2),
        (true, FirstCallback::Clear, 1),
        (true, FirstCallback::Replace, 2),
        (false, FirstCallback::Keep, 0),
    ] {
        let _deferral = pz_kahlua::gc::defer_collection();
        let mut fixture = Fixture::new(true, has_cell);
        let calls = Rc::new(Cell::new(0));
        let observed_generations = Rc::new(RefCell::new(Vec::new()));
        let calls_in = calls.clone();
        let generations_in = observed_generations.clone();
        let phases = fixture.phases.clone();
        fixture.app.lua.as_ref().unwrap().thread.globals().set_str(
            "observePostSave",
            Value::Native(NativeFunction::new("observePostSave", move |_| {
                // Reentry and mutation would fail if the presence-test guard
                // crossed the real event dispatch boundary.
                assert!(with_world(|world| world.cell.is_some()));
                generations_in.borrow_mut().push(cell_generation());
                let count = calls_in.get() + 1;
                calls_in.set(count);
                phases.borrow_mut().push(format!("post{count}"));
                if count == 1 {
                    match action {
                        FirstCallback::Keep => {}
                        FirstCallback::Clear => set_cell(None),
                        FirstCallback::Replace => set_cell(Some(IsoCell::default())),
                    }
                }
                Ok(0)
            })),
        );
        fixture.lua("Events.OnPostSave.Add(function() observePostSave() end)");
        fixture.app.close();
        assert!(fixture.app.error.is_none(), "{action:?}");
        assert_eq!(calls.get(), expected, "{has_cell} {action:?}");
        let mut expected_phases: Vec<String> = (1..=expected).map(|n| format!("post{n}")).collect();
        expected_phases.push("exit".into());
        assert_eq!(*fixture.phases.borrow(), expected_phases);
        if matches!(action, FirstCallback::Replace) {
            let generations = observed_generations.borrow();
            assert_ne!(
                generations[0], generations[1],
                "second dispatch observes the replacement field"
            );
        }
        assert!(fixture.app.world.is_none());
        assert!(!with_world(|world| world.cell.is_some()));
        fixture.app.close();
        assert_eq!(calls.get(), expected, "second close must not resave");
    }
}

#[test]
fn close_post_save_preserves_successful_and_failed_file_prefixes() {
    for fail_prefix in [false, true] {
        let _deferral = pz_kahlua::gc::defer_collection();
        let mut fixture = Fixture::new(false, true);
        if fail_prefix {
            std::fs::create_dir(fixture.dir.join("map_ver.bin")).unwrap();
        }
        let calls = Rc::new(Cell::new(0));
        let dir = fixture.dir.clone();
        let phases = fixture.phases.clone();
        fixture.app.lua.as_ref().unwrap().thread.globals().set_str(
            "observeSave",
            Value::Native(NativeFunction::new("observeSave", move |_| {
                assert!(!fail_prefix);
                assert!(dir.join("map_ver.bin").is_file());
                assert!(dir.join("statistics.bin").is_file());
                assert!(!dir.join("map.bin").exists());
                phases.borrow_mut().push("save".into());
                Ok(0)
            })),
        );
        let dir = fixture.dir.clone();
        let phases = fixture.phases.clone();
        let calls_in = calls.clone();
        fixture.app.lua.as_ref().unwrap().thread.globals().set_str(
            "observePostSave",
            Value::Native(NativeFunction::new("observePostSave", move |_| {
                assert!(with_world(|world| world.cell.is_some()));
                if fail_prefix {
                    assert!(dir.join("map_ver.bin").is_dir());
                    assert!(!dir.join("statistics.bin").exists());
                    assert!(!dir.join("map.bin").exists());
                } else {
                    assert!(dir.join("statistics.bin").is_file());
                    assert!(dir.join("map.bin").is_file());
                }
                let count = calls_in.get() + 1;
                calls_in.set(count);
                phases.borrow_mut().push(format!("post{count}"));
                Ok(0)
            })),
        );
        fixture.lua("Events.OnSave.Add(function() observeSave() end); Events.OnPostSave.Add(function() observePostSave() end)");
        fixture.app.close();
        assert!(fixture.app.error.is_none());
        assert_eq!(calls.get(), 2);
        let expected = if fail_prefix {
            vec!["post1", "post2", "exit"]
        } else {
            vec!["save", "post1", "post2", "exit"]
        };
        assert_eq!(*fixture.phases.borrow(), expected);
        assert_eq!(
            fixture
                .errors
                .borrow()
                .iter()
                .any(|e| e.starts_with("GameWindow.save:")),
            fail_prefix
        );
    }
}
