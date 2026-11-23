//! `IsoPlayer.performContextualAction(action)`: what the interact key does
//! with the action `doContext` chose, and `IsoGameCharacter.
//! triggerContextualAction`, which hands the action to the Lua
//! `Hook.ContextualAction` callbacks (`ISContextualActions.lua`).

use pz_characters::character::player::{ContextualAction, ContextualActionKind};
use pz_kahlua::Value;
use pz_world::objects::ObjectKind;

use crate::bind_character_sim::{character_value, LuaGameCharacterRef};
use crate::lua_manager::LuaManager;

/// The actions `IsoGameCharacter.concurrentActionList` holds.
const CONCURRENT: &[&str] = &[
    "OpenDoor",
    "CloseDoor",
    "ClimbThroughWindow",
    "ClimbOverFence",
    "OpenHutch",
    "CloseCurtain",
    "OpenCurtain",
    "CloseWindow",
    "OpenWindow",
];

/// `forbidConcurrentAction(action)`: a listed action is refused while the
/// character is doing something that cannot be cancelled. A dead player is
/// the only such case here; the Java `getCharacterActions()` stack of the
/// old timed actions is not used by the Lua timed actions.
fn forbid_concurrent_action(chr: &LuaGameCharacterRef, action: &str) -> bool {
    CONCURRENT.contains(&action) && chr.borrow().is_dead()
}

/// `triggerContextualAction(action, args...)`.
pub fn trigger(lua: &mut LuaManager, chr: &LuaGameCharacterRef, action: &str, args: &[Value]) {
    if forbid_concurrent_action(chr, action) {
        (lua.thread.log)(&format!("Action forbidden: {action}"));
        return;
    }
    let mut params = vec![Value::str(action), character_value(chr)];
    params.extend_from_slice(args);
    lua.trigger_hook("ContextualAction", &params);
}

/// The Lua object of the action's `object`.
fn object_of(action: &ContextualAction) -> Result<Value, String> {
    let (Some((x, y, z)), Some(id)) = (action.square, action.object) else {
        return Err("the action has no object".into());
    };
    crate::bind_object_picker::world_object_value_by_id([x, y, z], id as u32)
        .map_err(|e| format!("{e:?}"))?
        .ok_or_else(|| format!("the object {id} is no longer on ({x}, {y}, {z})"))
}

fn dir_value(action: &ContextualAction) -> Value {
    action.dir.map_or(Value::Nil, |d| {
        crate::expose::enum_value(crate::bind_iso_enums::ISO_DIRECTIONS, &format!("{d:?}"))
    })
}

/// `IsoWindow.IsOpen()` of the action's object.
fn window_is_open(action: &ContextualAction) -> Result<bool, String> {
    let object = object_of(action)?;
    let object = crate::bind_world_object::object_from_value(&object).ok_or("the object is not a world object")?;
    let body = object.object.try_borrow().map_err(|_| "the window is in use".to_string())?;
    match &body.kind {
        ObjectKind::Window(w) => Ok(w.open),
        _ => Err("the object is not an IsoWindow".into()),
    }
}

/// `performContextualAction`: the branches that are ported. An action
/// that is not is an `Err` naming it.
pub fn perform(lua: &mut LuaManager, chr: &LuaGameCharacterRef, action: &ContextualAction) -> Result<(), String> {
    use ContextualActionKind as K;
    match action.action {
        K::ClimbOverFence => {
            let args = [object_of(action)?, dir_value(action)];
            trigger(lua, chr, "ClimbOverFence", &args);
        }
        K::ClimbThroughWindow => {
            let args = [object_of(action)?];
            trigger(lua, chr, "ClimbThroughWindow", &args);
        }
        K::ToggleWindow => {
            let name = if window_is_open(action)? { "CloseWindow" } else { "OpenWindow" };
            let args = [object_of(action)?];
            trigger(lua, chr, name, &args);
        }
        K::ClimbOverWall => {
            use pz_pathfind::mover::PlayerMover;
            let dir = action.dir.ok_or("ClimbOverWall has no direction")?;
            PlayerMover::climb_over_wall(&mut *chr.borrow_mut(), dir);
        }
        K::ClimbSheetRope => {
            let (x, y, z) = action.square.ok_or("ClimbSheetRope has no square")?;
            let args = [crate::bind_world::square_value(x, y, z), Value::Bool(false)];
            trigger(lua, chr, "ClimbSheetRope", &args);
        }
        other => return Err(format!("performContextualAction({other:?}) is not ported")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::LuaGameCharacter;
    use pz_characters::character::CharacterKind;
    use pz_world::square::IsoDirections;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    #[test]
    fn perform_climb_sheet_rope_triggers_contextual_action_hook_with_square_and_false() {
        struct RestoreCell(Option<pz_world::IsoCell>);
        impl Drop for RestoreCell {
            fn drop(&mut self) {
                crate::bind_world::set_cell(self.0.take());
            }
        }
        let _restore = RestoreCell(crate::bind_world::with_world(|w| w.cell.take()));
        let mut chunk = pz_world::IsoChunk::new(1, 4);
        chunk.set_square(4, 2, 0, Some(pz_world::IsoGridSquare::new(12, 34, 0).into()));
        let mut cell = pz_world::IsoCell::default();
        cell.chunks.insert((1, 4), chunk);
        crate::bind_world::set_cell(Some(cell));

        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(PathBuf::new())));
        let mut lua = LuaManager::init(fs).unwrap();
        let sm = crate::bind_registry::shared_script_manager();
        let player = Rc::new(RefCell::new(LuaGameCharacter::new(&sm, CharacterKind::Player)));
        lua.thread.globals().set_str("player", character_value(&player));
        crate::bind_character_sim::tests::run(
            &mut lua.thread,
            r#"
            captured = nil
            Hook.ContextualAction.Add(function(action, chr, sq, down)
                captured = { action = action, same = (chr == player), x = sq:getX(), y = sq:getY(), z = sq:getZ(), down = down }
            end)
            "#,
        )
        .unwrap();
        let action = ContextualAction::new(
            ContextualActionKind::ClimbSheetRope,
            Some(IsoDirections::N),
            Some((12, 34, 0)),
            None,
        );
        perform(&mut lua, &player, &action).unwrap();
        crate::bind_character_sim::tests::run(
            &mut lua.thread,
            r#"
            if captured == nil then error("captured is nil") end
            if captured.action ~= "ClimbSheetRope" then error("wrong action") end
            if captured.same ~= true then error("wrong player") end
            if captured.x ~= 12 or captured.y ~= 34 or captured.z ~= 0 then error("wrong square") end
            if captured.down ~= false then error("wrong down flag") end
            "#,
        )
        .unwrap();
    }
}
