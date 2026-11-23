//! Exercise the shipped window action against a window from the installed map.
use super::{lua_check, run_frames};
use crate::game_window::App;
use pz_characters::character::states::ids;
use pz_lua_api::bind_players;

pub(super) fn check(app: &mut App) {
    // A zombie bite legitimately interrupts the stock timed action. Keep this
    // player out of combat while exercising window transitions, then restore it.
    let previous_debug = app.core.borrow().debug;
    app.core.borrow_mut().debug = true;
    let player = bind_players::player(0).unwrap();
    let sim = crate::sim::FrameSim::capture();
    let previous_invisible = player.borrow().cheats.is_set(&sim, "INVISIBLE");
    player.borrow_mut().cheats.set(&sim, "INVISIBLE", true);
    let lua = app.lua.as_mut().unwrap();
    lua_check(lua, r#"
        require "TimedActions/ISOpenCloseWindow"
        local p = getPlayer()
        local x, y, z = math.floor(p:getX()), math.floor(p:getY()), math.floor(p:getZ())
        local best = math.huge
        for dy = -12,12 do
            for dx = -12,12 do
                local d = dx*dx + dy*dy
                local sq = d < best and getCell():getGridSquare(x+dx,y+dy,z)
                if sq then
                    local objects = sq:getObjects()
                    for i = 0,objects:size()-1 do
                        local obj = objects:get(i)
                        if instanceof(obj, "IsoWindow") and not obj:isPermaLocked()
                            and not obj:isLocked() and not obj:isSmashed() and not obj:IsOpen() then
                            local north = obj:getNorth()
                            local opposite = getCell():getGridSquare(x+dx-(north and 0 or 1), y+dy-(north and 1 or 0), z)
                            -- This scenario immediately closes the pane from the landing tile.
                            -- A counter instead carries a climber to the free square beyond it.
                            if opposite and not opposite:has(IsoFlagType.solidtrans)
                                and not opposite:has(IsoFlagType.solid) and (sq:getRoomDef() or opposite:getRoomDef()) then
                                rustSessionWindow, best = obj, d
                                rustWindowX, rustWindowY = x+dx, y+dy
                                break
                            end
                        end
                    end
                end
            end
        end
        assert(rustSessionWindow, "no intact unlocked map window near the test spawn")
        local obj = rustSessionWindow
        local sq = getCell():getGridSquare(rustWindowX, rustWindowY, z)
        local opposite = getCell():getGridSquare(rustWindowX - (obj:getNorth() and 0 or 1),
            rustWindowY - (obj:getNorth() and 1 or 0), z)
        rustAlarmRoom = sq:getRoomDef() or opposite:getRoomDef()
        rustAlarmBuilding = rustAlarmRoom:getBuilding()
        rustAlarmBuilding:setAlarmed(true)
        rustAlarmBuilding:setAllExplored(false)
        rustWindowEvents = {}
        rustObserveWindow = function()
            if rustSessionWindow:getObjectIndex() ~= -1 then
                table.insert(rustWindowEvents, rustSessionWindow:IsOpen())
                if rustSessionWindow:IsOpen() then
                    rustAlarmCallback = not rustAlarmBuilding:isAlarmed() and rustAlarmBuilding:isAllExplored()
                end
            end
        end
        Events.OnContainerUpdate.Add(rustObserveWindow)
    "#);
    let globals = lua.thread.globals();
    let (x, y) = (globals.get_str("rustWindowX").as_num().unwrap() as i32,
                  globals.get_str("rustWindowY").as_num().unwrap() as i32);
    {
        let mut p = player.borrow_mut();
        p.set_x(x as f32 + 0.5);
        p.set_y(y as f32 + 0.5);
        let z = p.vars.z as i32;
        p.path.current_square = Some((x, y, z));
        p.vars.last_square = p.path.current_square;
    }
    super::queue::wait_for_idle(app);
    {
        use pz_sound::emitter::EmitterWorld;
        let audio = pz_lua_api::bind_audio::GameEmitterWorld;
        let listener = audio.player(0).expect("live emitter player");
        let square = listener.square.expect("live emitter player square");
        assert_eq!((square.x, square.y, square.z), (x, y, 0));
        assert!(listener.num_nearby_buildings_rooms > 0.0);
        assert_eq!(audio.square(x as f32 + 0.5, y as f32 + 0.5, 0.0), Some(square));
    }
    cancel_open_with_movement(app);
    check_direct_alarm(app, x, y);
    for (state, open) in [(ids::OPEN_WINDOW, true), (ids::CLOSE_WINDOW, false)] {
        let lua = app.lua.as_mut().unwrap();
        lua_check(lua, r#"
            rustWindowEvents = {}
            ISTimedActionQueue.add(ISOpenCloseWindow:new(getPlayer(), rustSessionWindow))
        "#);
        let mut entered = false;
        let mut changed = false;
        let mut blocked_during_close = false;
        let mut last_state = String::new();
        for frame in 1..=300 {
            run_frames(app, 1);
            assert!(app.error.is_none(), "window frame failed: {:?}", app.error);
            let p = player.borrow();
            let name = p.state.action_context.current_state_name();
            if name != last_state {
                eprintln!("window session at ({x},{y}), open={open}, frame {frame}, state {name}");
                last_state = name.to_owned();
            }
            entered |= p.is_current_state_id(state);
            let closing = !open && p.is_current_state_id(state)
                && p.variable_string("CloseWindowOutcome").eq_ignore_ascii_case("success");
            drop(p);
            let lua = app.lua.as_mut().unwrap();
            lua_check(lua, "rustSessionWindowOpen = rustSessionWindow:IsOpen()");
            let actual_open = lua.thread.globals().get_str("rustSessionWindowOpen").truthy();
            if actual_open && closing {
                lua_check(lua, "assert(not rustSessionWindow:canClimbThrough(nil))");
                blocked_during_close = true;
            }
            changed |= actual_open == open;
            if entered && changed && last_state.eq_ignore_ascii_case("idle") {
                break;
            }
        }
        assert!(entered && changed && last_state.eq_ignore_ascii_case("idle"),
            "window at ({x},{y}), requested open={open}, entered={entered}, changed={changed}, last state={last_state}");
        if !open {
            assert!(blocked_during_close, "no successful close observed while the pane was still open");
        }
        let assertion = if open { "assert(rustSessionWindow:canClimbThrough(getPlayer()))" }
                        else { "assert(not rustSessionWindow:canClimbThrough(getPlayer()))" };
        lua_check(app.lua.as_mut().unwrap(), assertion);
        lua_check(app.lua.as_mut().unwrap(), "assert(#rustWindowEvents > 0 and rustWindowEvents[#rustWindowEvents] == rustSessionWindow:IsOpen())");
        let id = if open { "WindowOpen" } else { "WindowClose" };
        lua_check(app.lua.as_mut().unwrap(), &format!(
            "assert(getPlayer():getMusicIntensityEvents():findEventById('{id}'):getIntensity() == 10)"
        ));
        if open {
            lua_check(app.lua.as_mut().unwrap(), "assert(rustAlarmCallback, 'window callback preceded building alarm')");
            assert_eq!(app.world.as_ref().unwrap().meta_tracker.borrow().alarms.len(), 1, "powered room should start one house alarm");
            climb(app, x, y);
        }
    }
    lua_check(app.lua.as_mut().unwrap(), "Events.OnContainerUpdate.Remove(rustObserveWindow)");
    lua_check(app.lua.as_mut().unwrap(), "rustAlarmBuilding:setAlarmed(true)");
    smash(app);
    lua_check(app.lua.as_mut().unwrap(), r#"
        require "TimedActions/ISRemoveBrokenGlass"
        rustGlassSounds = 0
        rustCheckGlassSound = function(x, y, z, radius, volume, source)
            if radius == 6 and volume == 1 and source == getPlayer() then
                rustGlassSounds = rustGlassSounds + 1
                if rustGlassSounds == 1 then addSound(source, x, y, z, 6, 1) end
            end
        end
        Events.OnWorldSound.Add(rustCheckGlassSound)
        local action = ISRemoveBrokenGlass:new(getPlayer(), rustSessionWindow)
        assert(action:isValid())
        ISTimedActionQueue.add(action)
    "#);
    let mut cleared = false;
    for _ in 0..360 {
        run_frames(app, 1);
        assert!(app.error.is_none(), "glass-removal frame failed: {:?}", app.error);
        let lua = app.lua.as_mut().unwrap();
        lua_check(lua, "rustWindowCleared = rustSessionWindow:isGlassRemoved()");
        if lua.thread.globals().get_str("rustWindowCleared").truthy() {
            cleared = true;
            break;
        }
    }
    assert!(cleared, "shipped ISRemoveBrokenGlass action did not clear the pane");
    lua_check(app.lua.as_mut().unwrap(), r#"
        Events.OnWorldSound.Remove(rustCheckGlassSound)
        assert(rustGlassSounds == 2, "expected both original and nested glass-removal sounds")
    "#);
    lua_check(app.lua.as_mut().unwrap(), "assert(not rustAlarmBuilding:isAlarmed())");
    assert_eq!(app.world.as_ref().unwrap().meta_tracker.borrow().alarms.len(), 2);
    player.borrow_mut().cheats.set(&sim, "INVISIBLE", previous_invisible);
    app.core.borrow_mut().debug = previous_debug;
}

/// Direct Lua, the ambient update and expiry all use the live save owner.
fn check_direct_alarm(app: &mut App, x: i32, y: i32) {
    lua_check(app.lua.as_mut().unwrap(), r#"
        rustAlarmSoundCount = 0
        rustListenToAlarm = function(x,y,z,radius,volume,source)
            if radius == 600 and volume == 600 then
                rustAlarmSameSource = rustAlarmSource == nil or rustAlarmSource == source
                rustAlarmSource = source
                rustAlarmSoundCount = rustAlarmSoundCount + 1
                rustAlarmCoordinates = {x, y, z}
            end
        end
        Events.OnWorldSound.Add(rustListenToAlarm)
        rustSessionWindow:ToggleWindow(getPlayer())
        assert(rustSessionWindow:IsOpen() and rustAlarmCallback and rustAlarmBuilding:isAllExplored())
    "#);
    let tracker = app.world.as_ref().unwrap().meta_tracker.clone();
    let alarm = tracker.borrow().alarms[0];
    run_frames(app, 2);
    lua_check(app.lua.as_mut().unwrap(), &format!(r#"
        assert(rustAlarmSoundCount == 2 and rustAlarmSameSource and rustAlarmSource ~= nil)
        assert(rustAlarmCoordinates[1] == {} and rustAlarmCoordinates[2] == {} and rustAlarmCoordinates[3] == 0)
        assert(getPlayer():getMusicIntensityEvents():findEventById('AlarmNearby'):getIntensity() == 20)
    "#, alarm.x, alarm.y));
    tracker.borrow_mut().alarms[0].end_game_time = pz_lua_api::bind_climate::shared_game_time().borrow().world_age_hours() as f32 - 1.0;
    run_frames(app, 2);
    assert!(tracker.borrow().alarms.is_empty());
    lua_check(app.lua.as_mut().unwrap(), r#"
        assert(rustAlarmSoundCount == 3 and rustAlarmSameSource)
        Events.OnWorldSound.Remove(rustListenToAlarm)
        -- Reverse the close from inside the event, before the path worker runs.
        local reopened = false
        local function reopen()
            if not reopened and not rustSessionWindow:IsOpen() then
                reopened = true
                rustSessionWindow:ToggleWindow(getPlayer())
            end
        end
        Events.OnContainerUpdate.Add(reopen)
        rustSessionWindow:ToggleWindow(getPlayer())
        Events.OnContainerUpdate.Remove(reopen)
        assert(reopened and rustSessionWindow:IsOpen())
        rustSessionWindow:ToggleWindow(getPlayer())
        assert(not rustSessionWindow:IsOpen() and not rustAlarmBuilding:isAlarmed())
        rustAlarmCallback = nil
        rustAlarmBuilding:setAlarmed(true)
    "#);
    let path = app.world.as_ref().unwrap().pathfind.as_ref().unwrap().service();
    let load_id = path.chunk_load_id(x.div_euclid(8), y.div_euclid(8)).unwrap();
    let snapshots: Vec<_> = path.queued_square_tasks().iter()
        .filter(|t| (t.update.x, t.update.y, t.update.z) == (x, y, 32))
        .map(|t| {
            use pz_pathfind::square_bits::bits::{WINDOW_N_UNBLOCKED, WINDOW_W_UNBLOCKED};
            assert_eq!(t.load_id, load_id);
            t.update.bits & (WINDOW_N_UNBLOCKED | WINDOW_W_UNBLOCKED) != 0
        }).collect();
    assert!(snapshots.ends_with(&[false, true, false]), "reentrant window path snapshots {snapshots:?}");
}

/// Start the native state directly so the queue cannot cancel on its behalf.
fn cancel_open_with_movement(app: &mut App) {
    let options = pz_core::debug::debug_options();
    let was_debug = app.core.borrow().debug;
    let option_debug = options.borrow().core_debug;
    app.core.borrow_mut().debug = true;
    options.borrow_mut().core_debug = true;
    lua_check(app.lua.as_mut().unwrap(), r#"
        rustOldWindowCheat = getDebugOptions():getBoolean("Cheat.Window.Unlock")
        getDebugOptions():setBoolean("Cheat.Window.Unlock", true)
        rustSessionWindow:setIsLocked(true)
        rustSessionWindow:setPermaLocked(true)
        assert(rustSessionWindow:isLocked() and rustSessionWindow:isPermaLocked())
        getPlayer():openWindow(rustSessionWindow)
    "#);
    let player = bind_players::player(0).unwrap();
    for _ in 0..30 {
        run_frames(app, 1);
        if player.borrow().is_current_state_id(ids::OPEN_WINDOW) { break }
    }
    assert!(player.borrow().is_current_state_id(ids::OPEN_WINDOW));
    lua_check(app.lua.as_mut().unwrap(), r#"
        assert(not rustSessionWindow:isLocked() and not rustSessionWindow:isPermaLocked())
        getDebugOptions():setBoolean("Cheat.Window.Unlock", rustOldWindowCheat)
    "#);
    app.core.borrow_mut().debug = was_debug;
    options.borrow_mut().core_debug = option_debug;
    pz_input::with_input(|i| i.key(17, 1));
    run_frames(app, 1);
    pz_input::with_input(|i| i.key(17, 0));
    run_frames(app, 1);
    super::queue::wait_for_idle(app);
    lua_check(app.lua.as_mut().unwrap(), "assert(not rustSessionWindow:IsOpen() and #rustWindowEvents == 0)");
}

fn smash(app: &mut App) {
    lua_check(app.lua.as_mut().unwrap(), r#"
        require "TimedActions/ISSmashWindow"
        rustWindowHitCount = 0
        rustObserveWindowHit = function(actor, weapon, target)
            if actor == getPlayer() and weapon == nil and target == rustSessionWindow and not target:isSmashed() then
                rustWindowHitCount = rustWindowHitCount + 1
            end
        end
        Events.OnWeaponHitThumpable.Add(rustObserveWindowHit)
        ISTimedActionQueue.add(ISSmashWindow:new(getPlayer(), rustSessionWindow))
    "#);
    let player = bind_players::player(0).unwrap();
    let mut entered = false;
    for frame in 1..=300 {
        run_frames(app, 1);
        assert!(app.error.is_none(), "smash frame failed: {:?}", app.error);
        let p = player.borrow();
        entered |= p.is_current_state_id(ids::SMASH_WINDOW);
        if entered && p.state.action_context.current_state_name().eq_ignore_ascii_case("idle") {
            drop(p);
            lua_check(app.lua.as_mut().unwrap(), "assert(rustSessionWindow:isSmashed() and rustSessionWindow:canClimbThrough(nil))");
            lua_check(app.lua.as_mut().unwrap(), "Events.OnWeaponHitThumpable.Remove(rustObserveWindowHit); assert(rustWindowHitCount == 1)");
            eprintln!("installed smash window returned to idle at frame {frame}");
            return;
        }
    }
    panic!("shipped window smash failed to finish, entered={entered}");
}

/// Queue the installed action and observe both the animation state and
/// physical occupancy of the square on the other side.
fn climb(app: &mut App, x: i32, y: i32) {
    let lua = app.lua.as_mut().unwrap();
    lua_check(lua, r#"
        require "TimedActions/ISClimbThroughWindow"
        local action = ISClimbThroughWindow:new(getPlayer(), rustSessionWindow, 1)
        assert(action:isValid())
        rustSessionWindowNorth = rustSessionWindow:getNorth()
        ISTimedActionQueue.add(action)
    "#);
    let north = lua.thread.globals().get_str("rustSessionWindowNorth").truthy();
    let expected = (x - i32::from(!north), y - i32::from(north), 0);
    let player = bind_players::player(0).unwrap();
    let mut entered = false;
    let mut last_state = String::new();
    for frame in 1..=300 {
        run_frames(app, 1);
        assert!(app.error.is_none(), "climb frame failed: {:?}", app.error);
        let actor = player.borrow();
        entered |= actor.is_current_state_id(ids::CLIMB_THROUGH_WINDOW);
        let state = actor.state.action_context.current_state_name();
        if state != last_state {
            eprintln!("window traversal frame {frame}, state {state}, square {:?}", actor.current_square_xyz());
            last_state = state.to_owned();
        }
        if entered && state.eq_ignore_ascii_case("idle") {
            assert_eq!(actor.current_square_xyz(), Some(expected), "window traversal ended on the wrong tile");
            return;
        }
    }
    panic!("window traversal failed to finish, entered={entered}, state={last_state}, square={:?}", player.borrow().current_square_xyz());
}
