//! Opt-in regression through the actual Rust session owners. Installed
//! media is read at runtime; saves always go into a fresh test cache.

mod queue;
mod cell_drag;
mod mannequin;

use super::{headless_app, run_frames, DevGame, Launch, NewGameSetup, Start};
use pz_core::core::Core;
use pz_lua_api::{bind_core, bind_players, player_db::PlayerDb, LuaManager};
use pz_characters::character::player::PlayerInput;
use pz_anim::AnimationVariableSource;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

const INGAME: &str = "zombie.gameStates.IngameState";

struct RestoreCore(Rc<RefCell<Core>>);

impl Drop for RestoreCore {
    fn drop(&mut self) {
        bind_core::install_core(self.0.clone());
    }
}

fn new_cache(game_dir: &Path) -> PathBuf {
    let base = std::env::var_os("PZ_SESSION_CHECK_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&base).expect("create session-check cache parent");
    let base = base.canonicalize().expect("resolve session-check cache parent");
    assert!(
        !base.starts_with(game_dir),
        "session-check output must be outside the installed game directory"
    );
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let cache = base.join(format!("pz-rs-session-check-{}-{stamp}", std::process::id()));
    // create_dir deliberately refuses to reuse any existing cache/save.
    std::fs::create_dir(&cache).expect("create fresh session-check cache");
    cache
}

fn lua_check(lua: &mut LuaManager, source: &str) {
    let f = pz_kahlua::stdlib::load_string(
        source,
        Some("rust-session-regression"),
        lua.thread.globals().clone(),
    )
    .expect("compile session-check Lua");
    lua.thread.pcall(&f, &[]).unwrap_or_else(|(error, trace)| {
        panic!("session-check Lua failed: {error:?}\n{trace}")
    });
}

fn save_counts(lua: &LuaManager) -> (Option<f64>, Option<f64>) {
    let globals = lua.thread.globals();
    (
        globals.get_str("rustSessionSaveCount").as_num(),
        globals.get_str("rustSessionPostSaveCount").as_num(),
    )
}

fn inventory_ui_check(lua: &mut LuaManager) {
    lua_check(lua, r#"
        local page = getPlayerInventory(0)
        assert(page and page.inventoryPane, "shipped player inventory pane was not created")
        local pane = page.inventoryPane
        local before = pane.refreshContainerCount or 0
        pane:refreshContainer()
        assert(pane.refreshContainerCount == before + 1, "inventory refresh did not finish")
        local foundKeyRing = false
        for _, row in ipairs(pane.itemslist) do
            for _, item in ipairs(row.items) do
                if item:isItemType(ItemType.KEY_RING) or item:hasTag(ItemTag.KEY_RING) then
                    foundKeyRing = true
                end
            end
        end
        assert(foundKeyRing, "starting key ring is missing from the actual inventory rows")
    "#);
}

fn global_systems_check(lua: &mut LuaManager, saved_world: bool) {
    lua.thread.globals().set_str("rustSessionLoadedGlobalSystems", saved_world);
    lua_check(lua, r#"
        local cell = getCell()
        assert(cell and cell == IsoCell.getInstance(), "global getCell does not return the live world cell")
        assert(cell:getDrag(0) == nil, "freshly started session retained a drag cursor")
        local systems = {
            {SCampfireSystem, 'campfire', CCampfireSystem},
            {SFarmingSystem, 'farming', CFarmingSystem},
            {STrapSystem, 'trap', CTrapSystem},
            {SRainBarrelSystem, 'rainbarrel', CRainBarrelSystem},
        }
        for _, entry in ipairs(systems) do
            local class, name = entry[1], entry[2]
            assert(class and class.instance, name .. ' Lua system was not initialized')
            local system = SGlobalObjects.getSystemByName(name)
            assert(system and class.instance.system == system,
                name .. ' Lua system does not retain the live native system')
            assert(system:getModData() == class.instance,
                name .. ' Lua/native mod-data identity changed')
            local version = system:loadedWorldVersion()
            assert(version == (rustSessionLoadedGlobalSystems and 249 or -1),
                name .. ' did not follow the new/saved global-system lifecycle')
            local clientClass, client = entry[3], CGlobalObjects.getSystemByName(name)
            assert(clientClass and clientClass.instance and client,
                name .. ' client system was not initialized')
            assert(clientClass.instance.system == client and client:getModData() == clientClass.instance,
                name .. ' client Lua/native system identity changed')
            assert(client:getObjectCount() == system:getObjectCount(),
                name .. ' client did not receive the initial objects')
        end
    "#);
}

fn close_survival_guide(lua: &mut LuaManager) {
    lua_check(lua, r#"
        if SurvivalGuide and SurvivalGuide.instance then
            SurvivalGuide.instance:onClose()
        end
        assert(UIManager.getSpeedControls():getCurrentGameSpeed() == 1,
            "closing the actual survival guide did not resume play")
    "#);
}

fn movement_diagnostics(app: &mut super::App, label: &str) {
    let input = pz_player_input::EnginePlayerInput::capture();
    let binding = app.core.borrow().get_key_binding("Forward");
    let raw = pz_lua_api::bind_input::is_key_down_raw(17);
    eprintln!("movement {label}: Forward={binding:?}, rawW={raw}, namedForward={}, textEntry={}, luaKeys={}",
        input.is_key_down("Forward"), input.is_doing_text_entry(), input.do_lua_key_pressed());
    let Some(player) = bind_players::player(0) else { return };
    let p = player.borrow();
    eprintln!("movement {label}: id={}, position=({},{},{}), action={}, children={:?}, moving={}, pressedMovement={}, isMoving={}, blockMovement={}, ignoreMovement={}, deferredEnabled={}, moveDelta={}",
        p.id, p.vars.x, p.vars.y, p.vars.z, p.state.action_context.current_state_name(),
        p.state.action_context.child_states().iter().map(|s| s.name()).collect::<Vec<_>>(),
        p.player_moving, p.get_variable_boolean("pressedMovement"), p.get_variable_boolean("isMoving"),
        p.vars.block_movement, p.vars.ignore_movement, p.vars.deferred_movement_enabled, p.vars.move_delta);
    if let Some(models) = app.world.as_mut().and_then(|w| w.models.as_mut()) {
        if let Some((animation, animator)) = models.animation_mut(p.id) {
            let root = animation.deferred_movement(false);
            let tracks: Vec<_> = animation.tracks().iter().map(|t| (t.name(), t.current_track_time(), t.blend_weight(), t.deferred_movement_bone_idx(), t.use_deferred_movement(), t.deferred_movement_diff())).collect();
            eprintln!("movement {label}: animState={:?}, root=({},{}), tracks={tracks:?}", animator.current_state_name(), root.x, root.y);
        } else {
            eprintln!("movement {label}: no live character animation");
        }
    }
}

#[derive(Debug, PartialEq)]
struct InventoryRecord {
    full_type: String,
    name: String,
    item_id: i32,
    registry_id: i16,
    key_id: i32,
    contents: Vec<InventoryRecord>,
}

fn inventory_records(container: &pz_inventory::shared::ContainerRef) -> Vec<InventoryRecord> {
    container.borrow().items.iter().map(|item| {
        let item = item.borrow();
        InventoryRecord {
            full_type: item.get_full_type(),
            name: item.get_name().to_string(),
            item_id: item.id,
            registry_id: item.registry_id,
            key_id: item.get_key_id(),
            contents: pz_inventory::shared::inventory_of(&item)
                .map_or_else(Vec::new, |bag| inventory_records(&bag)),
        }
    }).collect()
}

#[test]
fn paused_window_frames_keep_ui_callbacks_but_skip_state_updates_until_next_frame() {
    use crate::game_state::{state, GameState, GameStateMachine, StateAction, StateCx};
    use pz_core::game_time::GameTimeContext;
    use std::cell::Cell;

    struct Probe(&'static str, Rc<Cell<usize>>);
    impl GameState for Probe {
        fn class_name(&self) -> &'static str { self.0 }
        fn update(&mut self, _: &mut StateCx<'_>) -> StateAction {
            self.1.set(self.1.get() + 1);
            StateAction::Remain
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
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let base = TempDir(std::env::temp_dir().join(format!("pz-rs-paused-frame-{}-{stamp}", std::process::id())));
    std::fs::create_dir(&base.0).unwrap();
    std::fs::create_dir(base.0.join("game")).unwrap();
    std::fs::create_dir(base.0.join("cache")).unwrap();
    let _restore_core = RestoreCore(bind_core::shared_core());
    let _restore_ui = RestoreUi(Some(pz_ui::with_ui(std::mem::take)));
    bind_core::install_core(Rc::new(RefCell::new(Core::default())));
    assert!(!pz_lua_api::bind_climate::is_game_paused(), "missing speed controls must not pause");
    let mut app = headless_app(Launch {
        game_dir: base.0.join("game"), cache_dir: base.0.join("cache"), start: Start::MainMenu,
    }).unwrap();
    app.lua = Some(LuaManager::init(Rc::new(RefCell::new(pz_fs::GameFs::new(&app.launch.game_dir)))).unwrap());
    pz_lua_api::hud::init_ui(&mut app.lua.as_mut().unwrap().thread);
    let updates = Rc::new(Cell::new(0));
    let probe = state(Probe(INGAME, updates.clone()));
    app.states = GameStateMachine::default();
    app.states.firstrun = false;
    app.states.states = vec![probe.clone()];
    app.states.current = Some(probe);
    let context = pz_lua_api::bind_climate::LuaClimateContext::new();
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let packet_player = Rc::new(RefCell::new(pz_lua_api::bind_character_sim::LuaGameCharacter::new(
        &sm, pz_characters::CharacterKind::Player,
    )));
    app.lua.as_mut().unwrap().thread.globals().set_str(
        "packetPlayer", pz_lua_api::bind_character_sim::character_value(&packet_player),
    );
    lua_check(app.lua.as_mut().unwrap(), r#"
        -- This authored fixture has no installed stdlib.lua.
        function assert(value, message)
            if not value then error(message or "assertion failed") end
            return value
        end
        pumpOrder = {}
        local server = SGlobalObjects.registerSystem('paused-frame-probe')
        server:getModData().OnClientCommand = function(self, command, player, args)
            assert(command == 'ping' and player == packetPlayer and args.value == 7)
            table.insert(pumpOrder, 'server')
            packetClock = getGameTime():getRealworldSecondsSinceLastUpdate()
            server:sendCommand('reply', {value=args.value})
        end
        local client = CGlobalObjects.registerSystem('paused-frame-probe')
        client:getModData().OnServerCommand = function(self, command, args)
            assert(command == 'reply' and args.value == 7)
            table.insert(pumpOrder, 'client')
        end
        client:sendCommand('ping', packetPlayer, {value=7})
        pausedEvents = 0
        Events.OnTickEvenPaused.Add(function(tick)
            assert(tick == 0)
            table.insert(pumpOrder, 'tick')
            pausedEvents = pausedEvents + 1
            if resumeDuringPausedEvent then setGameSpeed(1) end
        end)
        setGameSpeed(0)
    "#);
    assert!(context.is_game_paused(), "a retained context must read current speed controls");
    // A prior-frame value must be replaced before packet Lua observes time.
    pz_lua_api::bind_climate::shared_game_time().borrow_mut().fps_multiplier = 0.0;
    app.frame();
    assert_eq!(updates.get(), 0);
    lua_check(app.lua.as_mut().unwrap(), "assert(table.concat(pumpOrder, ',') == 'server,client,tick')");
    let current_clock = pz_lua_api::bind_climate::shared_game_time().borrow().realworld_seconds_since_last_update();
    assert!(current_clock > 0.0);
    assert_eq!(app.lua.as_ref().unwrap().thread.globals().get_str("packetClock").as_num(), Some(current_clock as f64));
    lua_check(app.lua.as_mut().unwrap(), "assert(pausedEvents == 1); resumeDuringPausedEvent = true");
    app.frame();
    assert_eq!(updates.get(), 0, "unpause callback must not replay state update in this frame");
    assert!(!context.is_game_paused());
    app.frame();
    assert_eq!(updates.get(), 1);
    lua_check(app.lua.as_mut().unwrap(), "assert(pausedEvents == 2); setGameSpeed(0)");
    let menu = state(Probe("test.PauseMenu", updates.clone()));
    app.states.current = Some(menu);
    app.frame();
    assert_eq!(updates.get(), 1);
    lua_check(app.lua.as_mut().unwrap(), "assert(pausedEvents == 2)");
    pz_ui::with_ui(|ui| ui.speed_controls = None);
    app.frame();
    assert_eq!(updates.get(), 2);
    assert!(app.error.is_none());
    pz_lua_api::global_objects::reset_single_player_packets();
    for side in [pz_lua_api::global_objects::Side::Client, pz_lua_api::global_objects::Side::Server] {
        pz_lua_api::global_objects::reset(side);
    }
}

#[test]
#[ignore = "requires PZ_GAME_DIR; boots installed media in the Rust engine, saves, and reloads a fresh test world"]
fn installed_assets_new_game_moves_animates_and_saves_on_close() {
    let game_dir = PathBuf::from(std::env::var_os("PZ_GAME_DIR").expect("set PZ_GAME_DIR to installed game assets"))
        .canonicalize()
        .expect("resolve installed game directory");
    assert!(game_dir.join("projectzomboid.jar").is_file(), "PZ_GAME_DIR must contain projectzomboid.jar");
    assert!(game_dir.join("media").is_dir(), "PZ_GAME_DIR must contain media");
    let cache_dir = new_cache(&game_dir);
    eprintln!("session-check cache: {}", cache_dir.display());

    let _restore_core = RestoreCore(bind_core::shared_core());
    bind_core::install_core(Rc::new(RefCell::new(Core::default())));
    let mut app = headless_app(Launch {
        game_dir: game_dir.clone(),
        cache_dir: cache_dir.clone(),
        start: Start::Game(DevGame {
            game_mode: "Sandbox".into(),
            world: "rust-session-regression".into(),
            new_game: Some(NewGameSetup { spawn: Some((10695, 9383, 0)), seed: "RustSessionRegression".into() }),
            map: "Muldraugh, KY".into(),
        }),
    })
    .expect("create Rust session");
    app.core.borrow_mut().set_no_save(false);
    app.boot().expect("boot Rust session from installed media");
    movement_diagnostics(&mut app, "boot");

    // Use the real loading/state-machine path, including population startup.
    for _ in 0..1100 {
        run_frames(&mut app, 1);
        if app.current_state() == Some(INGAME) {
            break;
        }
    }
    assert_eq!(app.current_state(), Some(INGAME), "session never entered gameplay; cache: {}", cache_dir.display());
    run_frames(&mut app, 4);
    movement_diagnostics(&mut app, "before W");
    let player = bind_players::player(0).expect("new session created local player 0");
    let (player_id, before) = {
        let p = player.borrow();
        (p.id, (p.vars.x, p.vars.y, p.vars.z))
    };
    let starting_inventory = inventory_records(&player.borrow().items.inventory);
    assert!(
        starting_inventory.iter().any(|item| item.full_type.starts_with("Base.KeyRing")),
        "shipped OnNewGame starting-item script did not create a key ring: {starting_inventory:?}"
    );
    inventory_ui_check(app.lua.as_mut().unwrap());
    global_systems_check(app.lua.as_mut().unwrap(), false);

    // The shipped guide pauses through the real speed controls. Held input
    // must not advance simulation until its actual close handler resumes it.
    lua_check(app.lua.as_mut().unwrap(), r#"
        setGameSpeed(0)
        rustSessionPausedTicks = 0
        rustSessionPausedTick = function(tick)
            assert(tick == 0, "paused event must carry constant zero")
            rustSessionPausedTicks = rustSessionPausedTicks + 1
        end
        Events.OnTickEvenPaused.Add(rustSessionPausedTick)
    "#);
    let paused_clock = pz_lua_api::bind_climate::shared_game_time().borrow().time_of_day;
    let paused_world_frame = app.world.as_ref().unwrap().frame_no;
    pz_input::with_input(|i| i.key(17, 1));
    run_frames(&mut app, 3);
    pz_input::with_input(|i| i.key(17, 0));
    assert_eq!((player.borrow().vars.x, player.borrow().vars.y, player.borrow().vars.z), before);
    assert_eq!(app.world.as_ref().unwrap().frame_no, paused_world_frame);
    assert_eq!(pz_lua_api::bind_climate::shared_game_time().borrow().time_of_day, paused_clock);
    lua_check(app.lua.as_mut().unwrap(), r#"
        assert(rustSessionPausedTicks == 3)
        Events.OnTickEvenPaused.Remove(rustSessionPausedTick)
    "#);
    close_survival_guide(app.lua.as_mut().unwrap());

    // Keyboard scan code 17 is W. It enters the same input queue as the
    // native window; no character coordinate or movement flag is set here.
    pz_input::with_input(|i| i.key(17, 1));
    let mut late_start = None;
    for frame in 1..=60 {
        run_frames(&mut app, 1);
        if frame <= 5 || frame >= 56 || [10, 20, 40].contains(&frame) {
            movement_diagnostics(&mut app, &format!("W frame {frame}"));
        }
        if frame == 10 || frame == 40 {
            let p = player.borrow();
            let position = (p.vars.x, p.vars.y, p.vars.z);
            if frame == 10 {
                assert!(
                    (position.0 - before.0).abs() + (position.1 - before.1).abs() > 0.0001,
                    "real W input did not move the player during its first ten frames: {before:?} -> {position:?}; cache: {}",
                    cache_dir.display()
                );
            } else {
                late_start = Some(position);
            }
        }
    }
    let late_start = late_start.expect("captured held-W frame 40");
    let held_end = {
        let p = player.borrow();
        (p.vars.x, p.vars.y, p.vars.z)
    };
    assert!(
        (held_end.0 - late_start.0).abs() + (held_end.1 - late_start.1).abs() > 0.0001,
        "held W stopped moving the player during frames 41-60: {late_start:?} -> {held_end:?}; cache: {}",
        cache_dir.display()
    );
    pz_input::with_input(|i| i.key(17, 0));
    run_frames(&mut app, 1);
    queue::wait_for_idle(&mut app);
    let after = {
        let p = player.borrow();
        (p.vars.x, p.vars.y, p.vars.z)
    };
    assert!(after.0.is_finite() && after.1.is_finite() && after.2.is_finite(), "player position became non-finite");
    assert!(
        (after.0 - before.0).abs() + (after.1 - before.1).abs() > 0.0001,
        "real W input did not move the player: {before:?} -> {after:?}; cache: {}",
        cache_dir.display()
    );
    let world = app.world.as_ref().expect("live gameplay world");
    let models = world.models.as_ref().expect("world model owner exists");
    let (animation, _) = models.animation(player_id).expect("live player has an animation player");
    assert!(animation.num_bones() > 0, "player animation has no loaded rig");
    assert!(animation.track_count() > 0, "player animation has no live clip tracks");
    let save_dir = world.save_dir.clone();
    let sql_id = world.player_sql_ids[0];
    assert!(save_dir.starts_with(&cache_dir), "session save escaped its fresh cache");
    assert!(sql_id > 0, "local player has no persistent database id");

    queue::check(&mut app);
    lua_check(app.lua.as_mut().unwrap(), r#"
        local data = getPlayer():getModData()
        data.rustSessionRegression = { revision = 7, nested = { value = "saved from Lua" } }
        local retained = data.rustSessionRegression.nested
        retained.value = "retained nested mutation"
        assert(getPlayer():getModData() == data)
        rustSessionSaveCount, rustSessionPostSaveCount = 0, 0
        Events.OnSave.Add(function() rustSessionSaveCount = rustSessionSaveCount + 1 end)
        Events.OnPostSave.Add(function() rustSessionPostSaveCount = rustSessionPostSaveCount + 1 end)
    "#);
    // This is the native CloseRequested path, not a direct save helper.
    let parts = app.world.as_ref().unwrap().save_tables.as_ref().unwrap().blood_body_parts;
    let sprites = app.world.as_ref().unwrap().sprites.as_ref().unwrap().clone();
    mannequin::install_outfit_fixtures(app.lua.as_mut().unwrap(), &game_dir, parts, &sprites);
    let expected_world_chunks = mannequin::check_live(true);
    let expected_mannequins = mannequin::saved_state();
    app.close();
    assert!(app.world.is_none(), "native close retained the gameplay world");
    assert_eq!(app.current_state(), None);
    // `GameWindow.exit` delivers `OnPostSave` twice.
    assert_eq!(save_counts(app.lua.as_ref().unwrap()), (Some(1.0), Some(2.0)));
    app.close();
    assert_eq!(save_counts(app.lua.as_ref().unwrap()), (Some(1.0), Some(2.0)), "second close saved twice");
    mannequin::check_saved(&save_dir, &expected_world_chunks);

    let db_path = save_dir.join("players.db");
    assert!(db_path.is_file(), "native close did not create players.db");
    let mut db = PlayerDb::new(false, &save_dir.to_string_lossy());
    let saved = db.load_player(sql_id).expect("native close persisted the local player");
    assert!(!saved.bytes.is_empty(), "saved player has no serialized gameplay data");
    assert!(saved.world_version > 0);
    assert!((saved.x - after.0).abs() < 0.0001, "saved x differs: {} versus {}", saved.x, after.0);
    assert!((saved.y - after.1).abs() < 0.0001, "saved y differs: {} versus {}", saved.y, after.1);
    assert!((saved.z - after.2).abs() < 0.0001, "saved z differs: {} versus {}", saved.z, after.2);
    db.close();
    drop(db);
    assert!(save_dir.join("WorldDictionary.bin").is_file(), "new world has no persistent item dictionary; cache: {}", cache_dir.display());
    for name in ["campfire", "farming", "trap", "rainbarrel"] {
        assert!(pz_lua_api::global_objects_save::file_name(&save_dir, name).is_file(),
            "native close did not save the real {name} global system");
    }
    assert!(pz_lua_api::global_objects_save::save_dir().is_none(), "native close retained the global-system save directory");
    assert!(bind_players::player(0).is_none(), "native close retained player 0");
    drop(app);

    // A row readback cannot establish that a session is loadable. Reboot
    // from the same files with fresh App/Core owners and no new-game setup.
    bind_core::install_core(Rc::new(RefCell::new(Core::default())));
    let mut reloaded = headless_app(Launch {
        game_dir,
        cache_dir: cache_dir.clone(),
        start: Start::Game(DevGame {
            game_mode: "Sandbox".into(),
            world: "rust-session-regression".into(),
            new_game: None,
            map: "Muldraugh, KY".into(),
        }),
    })
    .expect("create Rust saved session");
    reloaded.core.borrow_mut().set_no_save(false);
    reloaded.boot().expect("boot Rust saved session from installed media");
    lua_check(reloaded.lua.as_mut().unwrap(), r#"
        rustSessionReloadNewGameCount = 0
        Events.OnNewGame.Add(function() rustSessionReloadNewGameCount = rustSessionReloadNewGameCount + 1 end)
    "#);
    for _ in 0..1100 {
        run_frames(&mut reloaded, 1);
        if reloaded.current_state() == Some(INGAME) {
            break;
        }
    }
    assert_eq!(reloaded.current_state(), Some(INGAME), "saved session never entered gameplay; cache: {}", cache_dir.display());
    mannequin::check_live(false);
    assert_eq!(mannequin::saved_state(), expected_mannequins, "mannequin outfit/identity/pose/skin changed across close and fresh reload");
    let loaded_player = bind_players::player(0).expect("saved session reconstructed local player 0");
    assert!(!Rc::ptr_eq(&player, &loaded_player), "saved session reused the closed session's player");
    let loaded_position = {
        let p = loaded_player.borrow();
        (p.vars.x, p.vars.y, p.vars.z)
    };
    let loaded_world = reloaded.world.as_ref().expect("saved session has a live gameplay world");
    assert_eq!(loaded_world.save_dir, save_dir, "saved session opened another world");
    assert_eq!(loaded_world.player_sql_ids[0], sql_id, "saved session did not decode the persisted player");
    assert_eq!(
        inventory_records(&loaded_player.borrow().items.inventory), starting_inventory,
        "starting items or their nested contents/IDs changed across native close and reload"
    );
    inventory_ui_check(reloaded.lua.as_mut().unwrap());
    global_systems_check(reloaded.lua.as_mut().unwrap(), true);
    close_survival_guide(reloaded.lua.as_mut().unwrap());
    assert!((loaded_position.0 - after.0).abs() < 0.0001, "reloaded x differs: {} versus {}", loaded_position.0, after.0);
    assert!((loaded_position.1 - after.1).abs() < 0.0001, "reloaded y differs: {} versus {}", loaded_position.1, after.1);
    assert!((loaded_position.2 - after.2).abs() < 0.0001, "reloaded z differs: {} versus {}", loaded_position.2, after.2);
    assert_eq!(reloaded.lua.as_ref().unwrap().thread.globals().get_str("rustSessionReloadNewGameCount").as_num(), Some(0.0), "saved session started a new game");
    lua_check(reloaded.lua.as_mut().unwrap(), r#"
        local saved = getPlayer():getModData().rustSessionRegression
        assert(saved.revision == 7)
        assert(saved.nested.value == "retained nested mutation")
        rustSessionOldEnvironment = true
        rustSessionOldMenuCount = 0
        Events.OnMainMenuEnter.Add(function() rustSessionOldMenuCount = rustSessionOldMenuCount + 1 end)
    "#);
    movement_diagnostics(&mut reloaded, "reloaded");
    let old_scripts = pz_lua_api::bind_registry::shared_script_manager();
    let old_globals = reloaded.lua.as_ref().unwrap().thread.globals().clone();
    reloaded.core.borrow_mut().exiting = true;
    run_frames(&mut reloaded, 1);
    assert_eq!(reloaded.current_state(), Some("zombie.gameStates.MainScreenState"), "quit did not return to the main menu");
    assert!(reloaded.world.is_none(), "menu return retained the old world");
    assert!(!Rc::ptr_eq(&old_scripts, &pz_lua_api::bind_registry::shared_script_manager()), "menu retained the old script definitions");
    assert!(reloaded.lua.as_ref().unwrap().thread.globals().get_str("rustSessionOldEnvironment").is_nil(), "menu retained old Lua globals");
    assert_eq!(old_globals.get_str("rustSessionOldMenuCount").as_num(), Some(0.0), "old menu callback ran after reset");
    assert!(matches!(reloaded.launch.start, Start::MainMenu), "startup game settings leaked into future menu loads");
    reloaded.close();
    assert!(reloaded.world.is_none(), "native close retained the reloaded world");
    assert_eq!(reloaded.current_state(), None);
    assert!(bind_players::player(0).is_none(), "native close retained the reloaded player");
    eprintln!("session-check passed: {before:?} -> {after:?} -> reloaded {loaded_position:?}; save: {}", db_path.display());
}
