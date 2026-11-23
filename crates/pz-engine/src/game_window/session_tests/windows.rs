//! Exercise the shipped window action against a window from the installed map.
use super::{lua_check, run_frames};
use crate::game_window::App;
use pz_characters::character::states::ids;
use pz_lua_api::bind_players;

pub(super) fn check(app: &mut App) {
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
                            rustSessionWindow, best = obj, d
                            rustWindowX, rustWindowY = x+dx, y+dy
                            break
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
    let player = bind_players::player(0).unwrap();
    {
        let mut p = player.borrow_mut();
        p.set_x(x as f32 + 0.5);
        p.set_y(y as f32 + 0.5);
        let z = p.vars.z as i32;
        p.path.current_square = Some((x, y, z));
        p.vars.last_square = p.path.current_square;
    }
    super::queue::wait_for_idle(app);
    cancel_open_with_movement(app);
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
    lua_check(app.lua.as_mut().unwrap(), "assert(not rustAlarmBuilding:isAlarmed())");
    assert_eq!(app.world.as_ref().unwrap().meta_tracker.borrow().alarms.len(), 2);
}

/// Start the native state directly so the queue cannot cancel on its behalf.
fn cancel_open_with_movement(app: &mut App) {
    lua_check(app.lua.as_mut().unwrap(), "getPlayer():openWindow(rustSessionWindow)");
    let player = bind_players::player(0).unwrap();
    for _ in 0..30 {
        run_frames(app, 1);
        if player.borrow().is_current_state_id(ids::OPEN_WINDOW) { break }
    }
    assert!(player.borrow().is_current_state_id(ids::OPEN_WINDOW));
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
