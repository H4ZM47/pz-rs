//! Opt-in regression through the actual Rust session owners. Installed
//! media is read at runtime; saves always go into a fresh test cache.

mod queue;
mod windows;
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

fn observe_tile_definitions(lua: &mut LuaManager) {
    lua_check(lua, r#"
        rustTileEvents = 0
        local floors = {}
        for _,group in ipairs({"blends_natural_01", "floors_interior_tilesandwood_01", "floors_exterior_natural_01"}) do
            for i=0,63 do table.insert(floors, group .. "_" .. i) end
        end
        rustMapNew, rustMapLoad, rustSquareEvents = 0, 0, 0
        local observed
        MapObjects.OnNewWithSprite(floors, function(o)
            if o:getSquare() then rustMapNew = rustMapNew + 1 end
        end, 1234)
        MapObjects.OnLoadWithSprite(floors, function(o)
            local sq = o:getSquare()
            if sq then
                if not (SFarmingSystem and SFarmingSystem.instance) then rustMapEarly = true end
                rustMapLoad = rustMapLoad + 1
                observed = sq
            end
        end, 1234)
        Events.LoadGridsquare.Add(function(sq)
            if observed == sq then
                rustSquareEvents = rustSquareEvents + 1
                observed = nil
            end
        end)
        Events.OnLoadedTileDefinitions.Add(function(manager)
            assert(manager == IsoSpriteManager.instance)
            local fence = manager:getSprite('fencing_01_1')
            assert(manager:getSprite(fence:getID()) == fence and fence:getName() == 'fencing_01_1')
            local props = manager:getSprite('rust_session_tile_probe'):getProperties()
            assert(not props:has(IsoFlagType.solid))
            props:set(IsoFlagType.solid)
            rustTileEvents = rustTileEvents + 1
        end)
    "#);
}

fn verify_tile_definitions(lua: &mut LuaManager) {
    lua_check(lua, r#"
        assert(rustTileEvents == 1)
        assert(not rustMapEarly, "map observers ran before global system initialization")
        assert(rustMapLoad > 0 and rustSquareEvents > 0, "square and sprite load observers")
        assert(IsoSpriteManager.instance:getSprite('rust_session_tile_probe'):getProperties():has(IsoFlagType.solid))
    "#);
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

fn verify_utilities_and_ambience(app: &mut super::App, streamer: bool) {
    lua_check(app.lua.as_mut().unwrap(), r#"
        local clock, sb = getGameTime(), getSandboxOptions()
        local original = sb:getTimeSinceApo()
        sb:set("TimeSinceApo", 3)
        local age = 60 + clock:getWorldAgeHours() / 24
        assert(math.abs(clock:getWorldAgeDaysSinceBegin() - age) < 0.000001)
        assert(math.abs(getWorld():getWorldAgeDays() - age) < 0.00001)
        assert(sb:doesPowerGridExist() == (age <= sb:getElecShutModifier()))
        assert(sb:doesPowerGridExist(70) == (age <= sb:getElecShutModifier() + 70))
        local world = getWorld()
        local originalPower = world:isHydroPowerOn()
        world:setHydroPowerOn(not originalPower)
        assert(world:isHydroPowerOn() ~= originalPower)
        world:setHydroPowerOn(originalPower)
        sb:set("TimeSinceApo", original)
        rustAmbientPower = originalPower and 1 or 0
        rustAmbientWater = clock:getWorldAgeDaysSinceBegin() < sb:getWaterShutModifier() and 1 or 0
        local square = getPlayer():getCurrentSquare()
        local room = square:getRoomDef()
        rustAmbientRoom = room and room:getArea() or math.max(0, square:getRoomSize())
        assert(square:hasGridPower() == (not square:isNoPower() and sb:doesPowerGridExist()))
        assert(square:hasGridPower(-999) == false)
        assert(square:getFloor():hasGridPower() == square:hasGridPower())
        assert(type(square:isNoWater()) == "boolean")
        local floor = square:getFloor()
        assert(type(square:isNoGas()) == "boolean")
        local data = floor:getModData()
        assert(type(square:HasTree()) == "boolean" and floor:getType() ~= nil)
        assert(type(SFarmingSystem.instance:hasWeeds(square)) == "boolean")
        local oldFuel = data.fuelAmount
        data.fuelAmount = nil
        assert(floor:getPipedFuelAmount() == -1)
        floor:setPipedFuelAmount(17)
        assert(data.fuelAmount == 17 and floor:getPipedFuelAmount() == 17)
        floor:setPipedFuelAmount(-9)
        assert(data.fuelAmount == 0)
        data.fuelAmount = oldFuel
        local parts = ArrayList.new()
        parts:add(floor)
        assert(floor:getSpriteGridObjects(parts) == parts and parts:isEmpty())
        assert(floor:getSpriteGridObjectsIncludingSelf(parts) == parts and parts:get(0) == floor)
        local function findObject(check)
            for y = square:getY()-8, square:getY()+8 do
                for x = square:getX()-8, square:getX()+8 do
                    local sq = getCell():getGridSquare(x, y, square:getZ())
                    local objects = sq and sq:getObjects()
                    if objects then
                        for i = 0, objects:size()-1 do
                            if check(objects:get(i), sq) then return true end
                        end
                    end
                end
            end
        end
        local others = ArrayList.new()
        assert(findObject(function(object)
            object:getSpriteGridObjectsExcludingSelf(others)
            if others:isEmpty() then return end
            object:getSpriteGridObjects(parts, true)
            assert(parts:size() == others:size()+1)
            local index = 0
            for j = 0, parts:size()-1 do
                local part = parts:get(j)
                assert(part:getSquare() ~= nil)
                if part ~= object then
                    assert(others:get(index) == part)
                    index = index + 1
                end
            end
            assert(index == others:size())
            return true
        end), "no multi-square furniture found near the player")
        assert(findObject(function(object, sq)
            local container = object:getContainerByEitherType("fridge", "freezer")
            if not container or not sq:getRoom() or sq:isNoPower() then return end
            local shut = sb:getElecShutModifier()
            sb:set("ElecShutModifier", 365)
            assert(ItemContainer.isObjectPowered(object, false))
            assert(container:isPowered() and object:checkObjectPowered())
            sb:set("ElecShutModifier", -1)
            assert(not ItemContainer.isObjectPowered(object, false))
            assert(container:isPowered(), "object power must be stable during a game tick")
            sb:set("ElecShutModifier", shut)
            return true
        end), "no indoor fridge near player")
        assert(not square:isDerelict() or (square:isNoPower() and square:isNoWater()))
    "#);
    let room = app.lua.as_ref().unwrap().thread.globals().get_str("rustAmbientRoom").as_num().unwrap() as f32;
    let ambient = &app.world.as_ref().unwrap().ambient_parameters;
    for (parameter, global) in [("Water", "rustAmbientWater"), ("Electricity", "rustAmbientPower")] {
        let expected = app.lua.as_ref().unwrap().thread.globals().get_str(global).as_num().unwrap() as f32;
        assert_eq!(ambient.current_value(parameter), Some(expected));
    }
    assert_eq!(ambient.current_value("RoomSize"), Some(room));
    let scripts = pz_lua_api::bind_registry::shared_script_manager();
    let player = bind_players::player(0).unwrap();
    let actor = player.borrow();
    assert_eq!(ambient.current_value("HardOfHearing"), Some(f32::from(actor.has_trait(&scripts, "HARD_OF_HEARING").unwrap())));
    assert_eq!(ambient.current_value("MoodlePanic"), Some(actor.moodles.moodle_level_of(&scripts, "PANIC").unwrap() as f32 / 4.));
    let zoom = {
        let core = app.core.borrow();
        let min = core.offscreen_buffer.get_min_zoom();
        (core.get_zoom(actor.player.player_index) - min) / (core.offscreen_buffer.get_max_zoom() - min)
    };
    assert_eq!(ambient.current_value("CameraZoom").unwrap().to_bits(), zoom.to_bits());
    drop(actor);
    assert_eq!(ambient.current_value("CharacterElevation"), Some(player.borrow().vars.z));
    assert_eq!(ambient.current_value("StreamerMode"), Some(f32::from(streamer)));
    let temperature = pz_lua_api::bind_climate::shared_game_time().borrow().climate.get_temperature();
    assert_eq!(ambient.current_value("Temperature"), Some((temperature * 100.0) as i32 as f32 / 100.0));
}

fn verify_pump_supply(app: &mut super::App) {
    let sprites = app.world.as_ref().unwrap().sprites.as_ref().unwrap().borrow();
    let pump = sprites.sprites.iter().find_map(|s| {
        let capacity: i32 = s.properties.get(&sprites.alias, "fuelAmount")?.parse().ok()?;
        (capacity > 1).then(|| s.name.clone()).flatten()
    }).expect("installed pump sprite");
    drop(sprites);
    let lua = app.lua.as_mut().unwrap();
    lua.thread.globals().set_str("rustPumpSprite", pz_kahlua::Value::str(&pump));
    lua_check(lua, r#"
        local sq = getCell():getGridSquare(10689, 9377, 0)
        local sb = getSandboxOptions()
        local saved = {}
        local function set(name, value)
            if saved[name] == nil then saved[name] = sb:getOptionByName(name):getValue() end
            sb:set(name, value)
        end
        set("AllowExteriorGenerator", false)
        set("ElecShutModifier", -1)
        set("FuelStationGasInfinite", false)
        assert(getSquare(10689.75, 9377.25, 0) == sq)
        local plant = sq:addTileObject('vegetation_indoor_01_7')
        local suffix = tonumber(string.match(plant:getSprite():getName(), 'vegetation_indoor_01_(%d+)'))
        assert(suffix and suffix >= 7 and suffix <= 12)
        local tile = 'fencing_01_1'
        local sign = sq:addTileObject(tile)
        local shared = sign:getSprite()
        sign:setName('runtime-sprite-fixture')
        sign:setSprite(tile)
        local runtime = sign:getSprite()
        assert(shared ~= runtime and runtime == sign:getSprite())
        assert(sign:getSpriteName() == tile and sign:getType() == IsoObjectType.MAX)
        assert(sign:getTileName() == sign:getName())
        local added = sq:addTileObject(rustPumpSprite)
        assert(added:getSquare() == sq and added:getObjectIndex() >= 0)
        assert(added:getPipedFuelAmount() == -1)
        set("FuelStationGasInfinite", true)
        assert(added:getPipedFuelAmount() == 1000)
        set("FuelStationGasInfinite", false)
        set("FuelStationGasMin", 0.5)
        set("FuelStationGasMax", 0.5)
        set("FuelStationGasEmptyChance", 0)
        set("ElecShutModifier", 365)
        local capacity = tonumber(added:getProperty("fuelAmount"))
        assert(added:getPipedFuelAmount() == math.floor(capacity * 0.5))
        added:setPipedFuelAmount(137)
        set("ElecShutModifier", -1)
        assert(added:getPipedFuelAmount() == 137)
        for key,value in pairs(saved) do sb:set(key, value) end
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
    observe_tile_definitions(app.lua.as_mut().unwrap());
    movement_diagnostics(&mut app, "boot");

    // Use the real loading/state-machine path, including population startup.
    for _ in 0..1100 {
        assert!(run_frames(&mut app, 1), "new session stopped at {:?}: {:?}", app.current_state(), app.error);
        if app.current_state() == Some(INGAME) {
            break;
        }
    }
    assert_eq!(app.current_state(), Some(INGAME), "session never entered gameplay; cache: {}, world errors: {:?}", cache_dir.display(), app.world.as_ref().map(|w| &w.errors));
    verify_tile_definitions(app.lua.as_mut().unwrap());
    lua_check(app.lua.as_mut().unwrap(), "assert(rustMapNew > 0, 'new sprite callbacks')");
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
    let ambient_before_pause = app.world.as_ref().unwrap().ambient_parameters.current_value("StreamerMode");
    let streamer = app.core.borrow().get_option_streamer_mode();
    app.core.borrow_mut().set_option_streamer_mode(!streamer);
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
    assert_eq!(app.world.as_ref().unwrap().ambient_parameters.current_value("StreamerMode"), ambient_before_pause);
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
    verify_utilities_and_ambience(&mut app, !streamer);
    verify_pump_supply(&mut app);
    app.core.borrow_mut().set_option_streamer_mode(streamer);
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
    mannequin::check_moveable_items(app.lua.as_mut().unwrap(), app.world.as_ref().unwrap());
    mannequin::install_outfit_fixtures(app.lua.as_mut().unwrap(), &game_dir, parts, &sprites);
    let expected_world_chunks = mannequin::check_live(true);
    let expected_mannequins = mannequin::saved_state();
    assert!(app.error.is_none(), "session failed before native close: {:?}", app.error);
    lua_check(app.lua.as_mut().unwrap(), "getCore():quitToDesktop()");
    run_frames(&mut app, 1);
    assert!(app.error.is_none(), "native close failed: {:?}", app.error);
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
    assert!(pz_lua_api::bind_sprites::sprite_property("rust_session_tile_probe", "solid").is_none(), "closed world retained its sprite manager");
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
    observe_tile_definitions(reloaded.lua.as_mut().unwrap());
    lua_check(reloaded.lua.as_mut().unwrap(), r#"
        rustSessionReloadNewGameCount = 0
        Events.OnNewGame.Add(function() rustSessionReloadNewGameCount = rustSessionReloadNewGameCount + 1 end)
    "#);
    for _ in 0..1100 {
        assert!(run_frames(&mut reloaded, 1), "saved session stopped at {:?}: {:?}", reloaded.current_state(), reloaded.error);
        if reloaded.current_state() == Some(INGAME) {
            break;
        }
    }
    assert_eq!(reloaded.current_state(), Some(INGAME), "saved session never entered gameplay; cache: {}", cache_dir.display());
    verify_tile_definitions(reloaded.lua.as_mut().unwrap());
    lua_check(reloaded.lua.as_mut().unwrap(), r#"
        local objects = getCell():getGridSquare(10689, 9377, 0):getObjects()
        local found, foundSprite = false, false
        for i = 0, objects:size()-1 do
            local obj = objects:get(i)
            if obj:getName() == 'runtime-sprite-fixture' then
                assert(obj:getSpriteName() == 'fencing_01_1' and obj:getType() == IsoObjectType.MAX)
                assert(obj:getSprite() and obj:getSprite() ~= IsoSpriteManager.instance:getSprite('fencing_01_1'))
                assert(obj:getTileName() == 'runtime-sprite-fixture')
                foundSprite = true
            end
            if obj:getProperty("fuelAmount") and obj:getPipedFuelAmount() == 137 then found = true end
        end
        assert(foundSprite, "single-texture map object must survive reload")
        assert(found, "placed pump and its stored fuel must survive reload")
    "#);
    let initial_chunks = mannequin::check_live(false);
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
    lua_check(reloaded.lua.as_mut().unwrap(), r#"
        local p = getPlayer()
        local events = p:getMusicIntensityEvents()
        local count = events:getEventCount()
        local container = ItemContainer.new("floor", nil, nil)
        container:setExplored(false)
        ISInventoryPage:checkExplored(container, p)
        assert(container:isExplored() and events:getEventCount() == count + 1)
        local search = events:getEventByIndex(count)
        assert(search:getId() == "SearchNewContainer" and search:getIntensity() == -10)
        ISInventoryPage:checkExplored(container, p)
        assert(events:getEventCount() == count + 1)
    "#);
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
        rustSessionOldSkin = XuiManager.GetDefaultSkin()
        assert(rustSessionOldSkin ~= nil and not rustSessionOldSkin:isInvalidated())
        rustSessionOldEnvironment = true
        rustSessionOldMenuCount = 0
        Events.OnMainMenuEnter.Add(function() rustSessionOldMenuCount = rustSessionOldMenuCount + 1 end)
    "#);
    movement_diagnostics(&mut reloaded, "reloaded");
    windows::check(&mut reloaded);
    let after_climb = { let p = loaded_player.borrow(); (p.vars.x, p.vars.y, p.vars.z) };
    let old_scripts = pz_lua_api::bind_registry::shared_script_manager();
    let old_globals = reloaded.lua.as_ref().unwrap().thread.globals().clone();
    // Preserve the first save in the isolated test cache. Each resident
    // chunk must be written again by quit; the earlier files cannot satisfy it.
    let reloaded_chunks = mannequin::check_live(false);
    let streamed_out: Vec<_> = initial_chunks.into_iter().filter(|xy| !reloaded_chunks.contains(xy)).collect();
    mannequin::check_saved(&save_dir, &streamed_out);
    let first_save = cache_dir.join("first-close-chunks");
    std::fs::create_dir(&first_save).unwrap();
    for &(wx, wy) in &reloaded_chunks {
        let path = pz_world::save::chunk_file::chunk_path(&save_dir, wx, wy);
        if path.is_file() {
            std::fs::rename(&path, first_save.join(format!("{wx}_{wy}.bin"))).unwrap();
        }
    }
    let active_alarms = reloaded.world.as_ref().unwrap().meta_tracker.borrow().alarms.clone();
    assert_eq!(active_alarms.len(), 2);
    lua_check(reloaded.lua.as_mut().unwrap(), "getCore():quit()");
    assert!(run_frames(&mut reloaded, 1), "returning to the menu ended the client");
    assert_eq!(reloaded.current_state(), Some("zombie.gameStates.MainScreenState"), "quit did not return to the main menu");
    assert!(reloaded.world.is_none(), "menu return retained the old world");
    mannequin::check_saved(&save_dir, &reloaded_chunks);
    let mut saved_meta = pz_core::save_trackers::MetaTracker::default();
    saved_meta.load_file(&save_dir, false, true).unwrap();
    assert_eq!(saved_meta.alarms, active_alarms, "quit must persist the active alarms");
    let mut db = PlayerDb::new(false, &save_dir.to_string_lossy());
    let saved = db.load_player(sql_id).expect("quit after climbing saved the player");
    assert_eq!((saved.x, saved.y, saved.z), after_climb);
    db.close();
    assert!(!Rc::ptr_eq(&old_scripts, &pz_lua_api::bind_registry::shared_script_manager()), "menu retained the old script definitions");
    assert!(reloaded.lua.as_ref().unwrap().thread.globals().get_str("rustSessionOldEnvironment").is_nil(), "menu retained old Lua globals");
    assert_eq!(old_globals.get_str("rustSessionOldMenuCount").as_num(), Some(0.0), "old menu callback ran after reset");
    let lua = reloaded.lua.as_mut().unwrap();
    lua.thread.globals().set_str("rustPreviousSkin", old_globals.get_str("rustSessionOldSkin"));
    lua_check(lua, r#"
        assert(rustPreviousSkin:isInvalidated())
        local skin = XuiManager.GetDefaultSkin()
        assert(skin ~= nil and skin ~= rustPreviousSkin and not skin:isInvalidated())
        rustPreviousSkin = nil
    "#);
    assert!(matches!(reloaded.launch.start, Start::MainMenu), "startup game settings leaked into future menu loads");
    lua_check(reloaded.lua.as_mut().unwrap(), "getCore():quit()");
    assert!(!run_frames(&mut reloaded, 1), "desktop exit left the client running");
    assert!(reloaded.error.is_none(), "menu quit failed: {:?}", reloaded.error);
    assert!(reloaded.world.is_none(), "native close retained the reloaded world");
    assert_eq!(reloaded.current_state(), None);
    assert!(bind_players::player(0).is_none(), "native close retained the reloaded player");
    eprintln!("session-check passed: {before:?} -> {after:?} -> reloaded {loaded_position:?}; save: {}", db_path.display());
}
