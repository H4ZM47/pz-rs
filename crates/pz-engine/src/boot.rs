//! Pieces of `GameWindow.init` shared by every client mode.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

thread_local! {
    // Attribute's actual class-static registry survives menu/script reloads.
    // AttributeFactory's instance-pool reset is a separate native operation.
    static ATTRIBUTE_TYPES: RefCell<Option<pz_entity::AttributeTypes>> = const { RefCell::new(None) };
}

fn native_attribute_types(jar: &pz_classfile::Jar) -> Result<pz_entity::AttributeTypes, String> {
    if let Some(types) = ATTRIBUTE_TYPES.with(|slot| slot.borrow().clone()) {
        return Ok(types);
    }
    let types = pz_entity::AttributeTypes::from_jar(jar)
        .map_err(|error| format!("attribute registry: {error}"))?;
    ATTRIBUTE_TYPES.with(|slot| *slot.borrow_mut() = Some(types.clone()));
    Ok(types)
}

/// The development game entry skips `MainScreen.create`, which normally
/// runs `MainOptions.loadKeys` before a game can start. Run that same Lua
/// loader so defaults, the user's saved bindings and keyboard layout are
/// applied before the first gameplay input snapshot.
pub fn load_key_bindings(lua: &mut pz_lua_api::LuaManager) -> Result<(), String> {
    let options = lua.thread.globals().get_str("MainOptions");
    let options = options
        .as_table()
        .ok_or("MainOptions is unavailable while loading key bindings")?;
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
        chars.perks = pz_characters::PerkFactory::from_tables(&tables)
            .map_err(|e| format!("PerkFactory.Perks: {e}"))?;
        chars
            .perks
            .init(&tables, &pz_characters::Characters::translate)
            .map_err(|e| format!("PerkFactory.init: {e}"))?;
        chars.sim = pz_characters::SimStatics::from_tables(&tables)
            .map_err(|e| format!("character statics: {e}"))?;
    }
    pz_lua_api::bind_characters::install_characters(Rc::new(RefCell::new(chars)));
    let jar = pz_classfile::Jar::open(game_dir.join("projectzomboid.jar"))
        .map_err(|error| format!("attribute registry jar: {error}"))?;
    let attributes = native_attribute_types(&jar)?;
    pz_lua_api::attribute_templates::install(attributes.clone());
    let loaded = sm.load(fs, &[]);
    pz_lua_api::attribute_templates::flush_notices()
        .map_err(|error| format!("attribute template diagnostics: {error:?}"))?;
    loaded.map_err(|e| format!("ScriptManager.Load failed: {e}"))?;
    // The entity registries items' factory components (`FluidContainer`)
    // resolve against.
    let entity = Some(pz_entity::BasicContext {
        attributes,
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm)
            .map_err(|error| format!("item fluid registry: {error}"))?),
        ..Default::default()
    });
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

/// `IngameState.exit` rebuilds the menu's script/Lua environment before
/// `MainScreenState.enter`. Dictionary ids belong to the discarded world;
/// fresh item/entity definitions start unregistered, retaining the normal
/// write-once `set_registry_id` contract.
/// On failure the caller must stop: shared hosts may already be retired.
pub fn reload_main_menu_environment(
    lua: &mut pz_lua_api::LuaManager,
    launch: &crate::game_window::Launch,
) -> Result<(), String> {
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&launch.game_dir)));
    fs.borrow_mut().set_cache_dir(&launch.cache_dir);
    // Read the installation before retiring the current Lua/UI owners.
    // Later failures retain the LuaManager value but cannot roll back
    // every shared host registry changed during initialization.
    load_scripts(&launch.game_dir, &mut fs.borrow_mut())?;
    replace_menu_environment(lua, fs, |fresh| {
        pz_lua_api::bind_text::boot(
            &fresh.fs.borrow(),
            &pz_lua_api::bind_core::shared_core().borrow(),
        )
        .map_err(|e| format!("text init: {e}"))?;
        pz_lua_api::hud::init_ui(&mut fresh.thread);
        fresh.load_all_client();
        fresh.trigger_event("OnGameBoot", &[]);
        Ok(())
    })
}

/// Keep the old LuaManager value until initialization succeeds. Shared
/// session bindings are reset first; this is not a transactional rollback.
/// The initializer is also used by synthetic
/// lifecycle tests, which provide authored Lua rather than game assets.
fn replace_menu_environment(
    lua: &mut pz_lua_api::LuaManager,
    fs: Rc<RefCell<pz_fs::GameFs>>,
    initialize: impl FnOnce(&mut pz_lua_api::LuaManager) -> Result<(), String>,
) -> Result<(), String> {
    pz_lua_api::session_reset::reset_session_bindings();
    let mut fresh =
        pz_lua_api::LuaManager::init(fs).map_err(|e| format!("LuaManager.init: {e}"))?;
    initialize(&mut fresh)?;
    *lua = fresh;
    Ok(())
}

#[cfg(test)]
mod session_reset_tests {
    use super::*;
    use pz_inventory::dictionary::{DictionaryInfo, WorldDictionary};
    use pz_kahlua::{stdlib, Value};
    use pz_lua_api::LuaManager;
    use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptType};

    fn eval(lua: &mut LuaManager, source: &str) {
        let function = stdlib::load_string(
            source,
            Some("menu-reset-fixture"),
            lua.thread.globals().clone(),
        )
        .unwrap();
        lua.thread.call(&function, &[]).unwrap();
    }

    fn scripts(sm: &ScriptManager) {
        sm.parse_script(
            ScriptLoadMode::Init,
            "module Example { item Pebble { Weight = 1, } entity Bench { } }",
        )
        .unwrap();
        let module = sm.get_module("Example").unwrap();
        for kind in [ScriptType::Item, ScriptType::Entity] {
            module.bucket(kind).load_scripts(sm, ScriptLoadMode::Init);
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pz-rs-menu-reset-{}-{stamp}-{number}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn menu_reset_reloads_definitions_and_isolates_lua_ui_and_world_dictionary_ids() {
        let game = temp_dir();
        let scripts_dir = game.join("media/lua/client");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(
            scripts_dir.join("menu.lua"),
            r#"
            newBootCount = 0
            newMenuCount = 0
            menuObservedBootCount = -1
            Events.OnGameBoot.Add(function() newBootCount = newBootCount + 1 end)
            Events.OnMainMenuEnter.Add(function()
                menuObservedBootCount = newBootCount
                newMenuCount = newMenuCount + 1
            end)
        "#,
        )
        .unwrap();
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&game)));
        let sm = Rc::new(ScriptManager::new());
        scripts(&sm);
        pz_lua_api::bind_registry::install_script_manager(sm.clone());
        let mut lua = LuaManager::init(fs.clone()).unwrap();
        eval(
            &mut lua,
            r#"
            legacyMarker = {}
            legacyBootCount = 0
            legacyMenuCount = 0
            Events.OnGameBoot.Add(function() legacyBootCount = legacyBootCount + 1 end)
            Events.OnMainMenuEnter.Add(function() legacyMenuCount = legacyMenuCount + 1 end)
        "#,
        );
        let old_globals = lua.thread.globals().clone();
        pz_ui::with_ui(|m| {
            m.ui.push(pz_ui::UIElement::new(Some(old_globals.clone())));
            m.to_add.push(pz_ui::UIElement::new(None));
        });
        let first_world = game.join("world-a");
        let first = crate::world_dictionary::init(&first_world, false, true, &sm, &[])
            .unwrap()
            .unwrap();
        assert_eq!(first.item_id("Example.Pebble"), Some(0));
        let old_item = pz_scripts::objects::items::get_all_items(&sm)
            .pop()
            .unwrap();
        assert_eq!(old_item.borrow().entity.registry_id(), 0);
        assert!(
            old_item.borrow_mut().entity.set_registry_id(7).is_err(),
            "the setter remains write-once"
        );

        // The original keeps ScriptManager.instance but discards all its
        // definitions during exit, then loads fresh ones for the menu.
        sm.reset();
        scripts(&sm);
        replace_menu_environment(&mut lua, fs, |fresh| {
            fresh.load_all_client();
            fresh.trigger_event("OnGameBoot", &[]);
            Ok(())
        })
        .unwrap();
        assert!(Rc::ptr_eq(
            &sm,
            &pz_lua_api::bind_registry::shared_script_manager()
        ));
        assert!(!Value::Table(old_globals.clone())
            .raw_equals(&Value::Table(lua.thread.globals().clone())));
        assert!(lua.thread.globals().get_str("legacyMarker").is_nil());
        assert_eq!(old_globals.get_str("legacyBootCount").as_num(), Some(0.0));
        assert!(pz_ui::with_ui(|m| m.ui.is_empty() && m.to_add.is_empty()));
        // UI-triggered events must also target the fresh EventManager.
        pz_ui::UIManager::trigger(&mut lua.thread, "OnMainMenuEnter", &[]);
        assert_eq!(
            lua.thread.globals().get_str("newBootCount").as_num(),
            Some(1.0)
        );
        assert_eq!(
            lua.thread.globals().get_str("newMenuCount").as_num(),
            Some(1.0)
        );
        assert_eq!(
            lua.thread
                .globals()
                .get_str("menuObservedBootCount")
                .as_num(),
            Some(1.0)
        );
        assert_eq!(old_globals.get_str("legacyMenuCount").as_num(), Some(0.0));
        let new_item = pz_scripts::objects::items::get_all_items(&sm)
            .pop()
            .unwrap();
        assert!(!Rc::ptr_eq(&old_item, &new_item));
        assert_eq!(new_item.borrow().entity.registry_id(), -1);

        let second_world = game.join("world-b");
        std::fs::create_dir(&second_world).unwrap();
        let mut second = WorldDictionary::new();
        second.items.insert(
            7,
            DictionaryInfo {
                registry_id: 7,
                module_name: "Example".into(),
                name: "Pebble".into(),
                full_type: "Example.Pebble".into(),
                mod_id: "pz-vanilla".into(),
                is_modded: false,
                exists_as_vanilla: true,
                obsolete: false,
                removed: false,
                mod_overrides: None,
            },
        );
        second.next_info_id = 8;
        std::fs::write(
            second_world.join("WorldDictionary.bin"),
            second.save().unwrap(),
        )
        .unwrap();
        let loaded = crate::world_dictionary::init(&second_world, false, false, &sm, &[])
            .unwrap()
            .unwrap();
        assert_eq!(loaded.item_id("Example.Pebble"), Some(7));
        assert_eq!(new_item.borrow().entity.registry_id(), 7);
        assert_eq!(old_item.borrow().entity.registry_id(), 0);
        std::fs::remove_dir_all(game).unwrap();
    }

    #[test]
    fn missing_installation_keeps_existing_lua_environment() {
        let game = temp_dir();
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&game)));
        let mut lua = LuaManager::init(fs).unwrap();
        eval(&mut lua, "oldEnvironment = 42");
        let launch = crate::game_window::Launch {
            game_dir: game.clone(),
            cache_dir: game.clone(),
            start: Default::default(),
        };
        assert!(reload_main_menu_environment(&mut lua, &launch).is_err());
        assert_eq!(
            lua.thread.globals().get_str("oldEnvironment").as_num(),
            Some(42.0)
        );
        std::fs::remove_dir_all(game).unwrap();
    }

    #[test]
    fn late_menu_initialization_failure_stops_before_entering_old_main_menu() {
        use crate::game_state::{state, GameState, GameStateMachine, StateCx, StateRef};
        use std::cell::Cell;

        struct FailingMenuRebuild {
            fs: Rc<RefCell<pz_fs::GameFs>>,
            initializer_called: Rc<Cell<bool>>,
        }

        impl GameState for FailingMenuRebuild {
            fn class_name(&self) -> &'static str {
                "fixture.FailingMenuRebuild"
            }

            fn exit(&mut self, cx: &mut StateCx<'_>) {
                crate::ingame::rebuild_main_menu_with(cx, |lua, _| {
                    replace_menu_environment(lua, self.fs.clone(), |fresh| {
                        self.initializer_called.set(true);
                        assert!(fresh.thread.globals().get_str("oldEnvironment").is_nil());
                        Err("forced late initializer failure".into())
                    })
                });
            }

            fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
                Some(state(crate::main_screen::MainScreenState::default()))
            }
        }

        let game = temp_dir();
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&game)));
        let mut lua = LuaManager::init(fs.clone()).unwrap();
        eval(&mut lua, r#"
            oldEnvironment = 42
            oldMainMenuCount = 0
            Events.OnMainMenuEnter.Add(function() oldMainMenuCount = oldMainMenuCount + 1 end)
        "#);
        let old_globals = lua.thread.globals().clone();
        pz_ui::with_ui(|ui| ui.ui.push(pz_ui::UIElement::new(Some(old_globals.clone()))));
        let initializer_called = Rc::new(Cell::new(false));
        let mut machine = GameStateMachine::default();
        machine.states.push(state(FailingMenuRebuild { fs, initializer_called: initializer_called.clone() }));
        let launch = crate::game_window::Launch { game_dir: game.clone(), cache_dir: game.clone(), start: Default::default() };
        let mut world = None;
        let mut error = None;
        let mut retired_models = Vec::new();
        let mut cx = StateCx {
            lua: &mut lua, launch: &launch, screen_w: 1, screen_h: 1,
            multiplier: 1.0, time_delta: 1.0 / 30.0, world: &mut world,
            shutdown: false, error: &mut error, retired_models: &mut retired_models,
        };
        machine.update(&mut cx);
        assert!(initializer_called.get(), "failure did not reach the late initialization boundary");
        assert_eq!(cx.error.as_deref(), Some("Unable to rebuild main menu: forced late initializer failure"));
        assert!(machine.current.is_none());
        assert!(!machine.looping);
        assert!(pz_ui::with_ui(|ui| ui.ui.is_empty()), "old UI was not retired before the forced failure");
        // Updating or rendering again cannot call the old environment's
        // menu callbacks after the shared owners have been replaced.
        machine.update(&mut cx);
        machine.render(&mut cx, &mut pz_render::DrawList::default());
        assert_eq!(old_globals.get_str("oldMainMenuCount").as_num(), Some(0.0));
        std::fs::remove_dir_all(game).unwrap();
    }
}
