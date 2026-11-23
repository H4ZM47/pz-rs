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
        rustWindowEvents = {}
        rustObserveWindow = function()
            if rustSessionWindow:getObjectIndex() ~= -1 then
                table.insert(rustWindowEvents, rustSessionWindow:IsOpen())
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
    for (state, open) in [(ids::OPEN_WINDOW, true), (ids::CLOSE_WINDOW, false)] {
        let lua = app.lua.as_mut().unwrap();
        lua_check(lua, r#"
            rustWindowEvents = {}
            ISTimedActionQueue.add(ISOpenCloseWindow:new(getPlayer(), rustSessionWindow))
        "#);
        let mut entered = false;
        let mut changed = false;
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
            drop(p);
            let lua = app.lua.as_mut().unwrap();
            lua_check(lua, "rustSessionWindowOpen = rustSessionWindow:IsOpen()");
            changed |= lua.thread.globals().get_str("rustSessionWindowOpen").truthy() == open;
            if entered && changed && last_state.eq_ignore_ascii_case("idle") {
                break;
            }
        }
        assert!(entered && changed && last_state.eq_ignore_ascii_case("idle"),
            "window at ({x},{y}), requested open={open}, entered={entered}, changed={changed}, last state={last_state}");
        let assertion = if open { "assert(rustSessionWindow:canClimbThrough(getPlayer()))" }
                        else { "assert(not rustSessionWindow:canClimbThrough(getPlayer()))" };
        lua_check(app.lua.as_mut().unwrap(), assertion);
        lua_check(app.lua.as_mut().unwrap(), "assert(#rustWindowEvents > 0 and rustWindowEvents[#rustWindowEvents] == rustSessionWindow:IsOpen())");
    }
    lua_check(app.lua.as_mut().unwrap(), "Events.OnContainerUpdate.Remove(rustObserveWindow)");
}
