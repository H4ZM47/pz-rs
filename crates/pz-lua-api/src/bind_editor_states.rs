//! Build 42.22 debug/editor `GameState` singletons and overhead-map/state
//! globals:
//! - `zombie.gameStates.TileGeometryState` (`getTileGeometryState`)
//! - `zombie.gameStates.SpriteModelEditorState` (`getSpriteModelEditorState`)
//! - `zombie.gameStates.AnimationViewerState` (`getAnimationViewerState`)
//! - `zombie.gameStates.SeamEditorState` (`getSeamEditorState`)
//! - `zombie.vehicles.EditVehicleState` (`getEditVehicleState`)
//! - `zombie.gameStates.AttachmentEditorState` (`getAttachmentEditorState`)
//! - `forceChangeState`, `translatePointXInOverheadMapToWorld`,
//!   `translatePointYInOverheadMapToWorld`, `setPlayerButtonsActive`

use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub const TILE_GEOMETRY_STATE_CLASS: &str = "zombie.gameStates.TileGeometryState";
pub const SPRITE_MODEL_EDITOR_STATE_CLASS: &str = "zombie.gameStates.SpriteModelEditorState";
pub const ANIMATION_VIEWER_STATE_CLASS: &str = "zombie.gameStates.AnimationViewerState";
pub const SEAM_EDITOR_STATE_CLASS: &str = "zombie.gameStates.SeamEditorState";
pub const EDIT_VEHICLE_STATE_CLASS: &str = "zombie.vehicles.EditVehicleState";
pub const ATTACHMENT_EDITOR_STATE_CLASS: &str = "zombie.gameStates.AttachmentEditorState";

const EDITOR_STATES: &[(&str, &str)] = &[
    (TILE_GEOMETRY_STATE_CLASS, "getTileGeometryState"),
    (SPRITE_MODEL_EDITOR_STATE_CLASS, "getSpriteModelEditorState"),
    (ANIMATION_VIEWER_STATE_CLASS, "getAnimationViewerState"),
    (SEAM_EDITOR_STATE_CLASS, "getSeamEditorState"),
    (EDIT_VEHICLE_STATE_CLASS, "getEditVehicleState"),
    (ATTACHMENT_EDITOR_STATE_CLASS, "getAttachmentEditorState"),
];

#[derive(Default)]
struct EditorStateData {
    table: Value,
    booleans: BTreeMap<String, bool>,
    exited: bool,
}

struct LuaEditorState {
    class: &'static str,
    data: RefCell<EditorStateData>,
}

impl LuaObject for LuaEditorState {
    fn class_name(&self) -> &str {
        self.class
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["zombie.gameStates.GameState"]
    }
}

thread_local! {
    static INSTANCES: RefCell<BTreeMap<&'static str, Rc<LuaEditorState>>> =
        RefCell::new(BTreeMap::new());
    static PLAYER_BUTTONS_ACTIVE: RefCell<[bool; 4]> = const { RefCell::new([true; 4]) };
}

fn instance_for(class: &'static str) -> Rc<LuaEditorState> {
    INSTANCES.with(|map| {
        map.borrow_mut()
            .entry(class)
            .or_insert_with(|| {
                Rc::new(LuaEditorState {
                    class,
                    data: RefCell::new(EditorStateData::default()),
                })
            })
            .clone()
    })
}

/// Whether joypad/player action buttons are enabled for `player_index`
/// (`setPlayerButtonsActive`).
pub fn player_buttons_active(player_index: usize) -> bool {
    PLAYER_BUTTONS_ACTIVE.with(|b| b.borrow().get(player_index).copied().unwrap_or(true))
}

/// Clear per-session editor state tables and flags.
pub(crate) fn reset_session_handles() {
    INSTANCES.with(|map| map.borrow_mut().clear());
    PLAYER_BUTTONS_ACTIVE.with(|b| *b.borrow_mut() = [true; 4]);
}

fn with_editor<R>(v: &Value, f: impl FnOnce(&EditorStateData) -> R) -> Option<R> {
    if let Value::Object(o) = v {
        if let Some(st) = o.as_any().downcast_ref::<LuaEditorState>() {
            let res = Some(f(&st.data.borrow()));
            return res;
        }
    }
    None
}

fn with_editor_mut<R>(v: &Value, f: impl FnOnce(&mut EditorStateData) -> R) -> Option<R> {
    if let Value::Object(o) = v {
        if let Some(st) = o.as_any().downcast_ref::<LuaEditorState>() {
            let res = Some(f(&mut st.data.borrow_mut()));
            return res;
        }
    }
    None
}

fn handle_from_lua(this: &Value, func: &str) -> Value {
    if func.eq_ignore_ascii_case("exit") {
        with_editor_mut(this, |d| d.exited = true);
    }
    Value::Nil
}

pub fn register(thread: &mut Thread) {
    for &(class, global_name) in EDITOR_STATES {
        let cls = expose_class(thread, class, Some("zombie.gameStates.GameState"));
        cls.static_fn("checkInstance", move |c| {
            Ok(c.push(Value::Object(instance_for(class))))
        });
        cls.method("setTable", |c| {
            let tbl = c.arg(1);
            with_editor_mut(&c.arg(0), |d| d.table = tbl);
            Ok(0)
        });
        cls.method("getTable", |c| {
            let v = with_editor(&c.arg(0), |d| d.table.clone()).unwrap_or(Value::Nil);
            Ok(c.push(v))
        });
        cls.method("getBoolean", |c| {
            let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            let v = with_editor(&c.arg(0), |d| d.booleans.get(&name).copied().unwrap_or(false))
                .unwrap_or(false);
            Ok(c.push(v))
        });
        cls.method("setBoolean", |c| {
            let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            let val = bool_arg(c, 2);
            if !name.is_empty() {
                with_editor_mut(&c.arg(0), |d| {
                    d.booleans.insert(name, val);
                });
            }
            Ok(0)
        });
        cls.method("getOptionCount", |c| {
            let n = with_editor(&c.arg(0), |d| d.booleans.len() as i32).unwrap_or(0);
            Ok(c.push(n))
        });
        cls.method("fromLua0", |c| {
            let func = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            Ok(c.push(handle_from_lua(&c.arg(0), &func)))
        });
        cls.method("fromLua1", |c| {
            let func = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            Ok(c.push(handle_from_lua(&c.arg(0), &func)))
        });
        cls.method("fromLua2", |c| {
            let func = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            Ok(c.push(handle_from_lua(&c.arg(0), &func)))
        });
        cls.method("fromLua3", |c| {
            let func = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            Ok(c.push(handle_from_lua(&c.arg(0), &func)))
        });
        cls.method("fromLua4", |c| {
            let func = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            Ok(c.push(handle_from_lua(&c.arg(0), &func)))
        });

        global(thread, global_name, move |c| {
            Ok(c.push(Value::Object(instance_for(class))))
        });
    }

    // `translatePointXInOverheadMapToWorld(x, ui, zoom, offset)` / `Y`:
    // `(x - ui:getAbsoluteX()) / zoom + offset` (Build 42.22 GlobalObject).
    global(thread, "translatePointXInOverheadMapToWorld", |c| {
        let x = num_arg(c, 0);
        let ui = c.arg(1);
        let zoom = num_arg(c, 2);
        let offset = num_arg(c, 3);
        let abs_x = if !ui.is_nil() {
            let f = c.thread.table_get(&ui, &Value::str("getAbsoluteX"))?;
            if f.is_nil() {
                0.0
            } else {
                let r = c.thread.call1(&f, &[ui])?;
                pz_kahlua::exec::raw_tonumber(&r).unwrap_or(0.0)
            }
        } else {
            0.0
        };
        let z = if zoom.abs() < f64::EPSILON { 1.0 } else { zoom };
        Ok(c.push((x - abs_x) / z + offset))
    });
    global(thread, "translatePointYInOverheadMapToWorld", |c| {
        let y = num_arg(c, 0);
        let ui = c.arg(1);
        let zoom = num_arg(c, 2);
        let offset = num_arg(c, 3);
        let abs_y = if !ui.is_nil() {
            let f = c.thread.table_get(&ui, &Value::str("getAbsoluteY"))?;
            if f.is_nil() {
                0.0
            } else {
                let r = c.thread.call1(&f, &[ui])?;
                pz_kahlua::exec::raw_tonumber(&r).unwrap_or(0.0)
            }
        } else {
            0.0
        };
        let z = if zoom.abs() < f64::EPSILON { 1.0 } else { zoom };
        Ok(c.push((y - abs_y) / z + offset))
    });

    global(thread, "setPlayerButtonsActive", |c| {
        let idx = int_arg(c, 0);
        let active = bool_arg(c, 1);
        if (0..4).contains(&idx) {
            PLAYER_BUTTONS_ACTIVE.with(|b| b.borrow_mut()[idx as usize] = active);
        }
        Ok(0)
    });
}

/// If `obj` is one of the 6 Build 42.22 editor `GameState` singletons, return
/// its canonical Java class name (used by `bind_game_window::forceChangeState`).
pub(crate) fn editor_state_class(obj: &dyn LuaObject) -> Option<&'static str> {
    obj.as_any()
        .downcast_ref::<LuaEditorState>()
        .map(|s| s.class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn editor_states_and_overhead_map_globals_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_game_window::register(&mut t, None, std::path::Path::new("/tmp/x/Zomboid"));
        register(&mut t);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            local tg = getTileGeometryState()
            assert(tg == TileGeometryState.checkInstance())
            local ui = { getAbsoluteX = function() return 10 end, getAbsoluteY = function() return 20 end }
            tg:setTable(ui)
            assert(tg:getTable() == ui)
            assert(tg:getBoolean("DrawGrid") == false)
            tg:setBoolean("DrawGrid", true)
            assert(tg:getBoolean("DrawGrid") == true)
            assert(tg:getOptionCount() == 1)
            tg:fromLua0("exit")

            assert(getSpriteModelEditorState() == SpriteModelEditorState.checkInstance())
            assert(getAnimationViewerState() == AnimationViewerState.checkInstance())
            assert(getSeamEditorState() == SeamEditorState.checkInstance())
            assert(getEditVehicleState() == EditVehicleState.checkInstance())
            assert(getAttachmentEditorState() == AttachmentEditorState.checkInstance())

            forceChangeState(tg)
            assert(translatePointXInOverheadMapToWorld(50, ui, 2.0, 100) == 120)
            assert(translatePointYInOverheadMapToWorld(60, ui, 4.0, 10) == 20)

            setPlayerButtonsActive(0, false)
        "#;
        let f = stdlib::load_string(src, Some("editor_states"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();

        assert_eq!(
            crate::bind_game_window::take_forced_state(),
            Some(Some(crate::bind_game_window::ForcedState::Editor(
                TILE_GEOMETRY_STATE_CLASS
            )))
        );
        assert_eq!(crate::bind_game_window::take_forced_state(), None);
        assert!(!player_buttons_active(0));
        assert!(player_buttons_active(1));
    }
}
