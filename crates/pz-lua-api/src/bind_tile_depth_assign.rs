//! Build 42.22 Lua bindings for:
//! - `zombie.tileDepth.TileDepthTextureAssignmentManager`
//! - `zombie.debug.objects.ObjectDebuggerLua`

use std::any::Any;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};
use pz_render::tile_depth::TileDepthTextureAssignmentManager;

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;

const ASSIGN_MGR_CLASS: &str = "zombie.tileDepth.TileDepthTextureAssignmentManager";
const OBJECT_DEBUGGER_CLASS: &str = "zombie.debug.objects.ObjectDebuggerLua";

thread_local! {
    static MANAGER: RefCell<TileDepthTextureAssignmentManager> =
        RefCell::new(TileDepthTextureAssignmentManager::default());
    static GAME_FS: RefCell<Option<Rc<RefCell<GameFs>>>> = const { RefCell::new(None) };
}

/// Replace the thread-local `TileDepthTextureAssignmentManager` (used by engine/tests).
pub fn install_manager(mgr: TileDepthTextureAssignmentManager) {
    MANAGER.with(|m| *m.borrow_mut() = mgr);
}

/// Reset session state on world teardown.
pub fn reset() {
    MANAGER.with(|m| *m.borrow_mut() = TileDepthTextureAssignmentManager::default());
    GAME_FS.with(|f| *f.borrow_mut() = None);
}

#[derive(Debug)]
struct AssignmentManagerHandle;

impl LuaObject for AssignmentManagerHandle {
    fn class_name(&self) -> &'static str {
        ASSIGN_MGR_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn str_arg(c: &NativeCall<'_>, idx: usize) -> Option<String> {
    c.arg(idx).as_str().map(str::to_string)
}

fn format_value_lines(
    val: &Value,
    out: &mut Vec<Value>,
    indent: usize,
    depth: usize,
    max_depth: usize,
) {
    let pad = "  ".repeat(indent);
    match val {
        Value::Nil => out.push(Value::str(&format!("{pad}nil"))),
        Value::Bool(b) => out.push(Value::str(&format!("{pad}{b}"))),
        Value::Num(n) => out.push(Value::str(&format!("{pad}{n}"))),
        Value::Str(s) => out.push(Value::str(&format!("{pad}\"{}\"", s.as_ref()))),
        Value::Object(obj) => {
            out.push(Value::str(&format!("{pad}[{}]", obj.class_name())));
        }
        Value::Table(tbl) => {
            if depth >= max_depth {
                out.push(Value::str(&format!("{pad}<table>")));
                return;
            }
            out.push(Value::str(&format!("{pad}{{")));
            let mut pairs: Vec<(String, Value)> = tbl
                .try_entries()
                .unwrap_or_default()
                .into_iter()
                .take(64)
                .map(|(k, v)| {
                    let k_str = match &k {
                        Value::Str(s) => s.as_ref().to_string(),
                        Value::Num(n) => format!("{n}"),
                        Value::Bool(b) => format!("{b}"),
                        _ => format!("{k:?}"),
                    };
                    (k_str, v)
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in pairs {
                match v {
                    Value::Table(_) if depth + 1 < max_depth => {
                        out.push(Value::str(&format!("{pad}  {k} =")));
                        format_value_lines(&v, out, indent + 2, depth + 1, max_depth);
                    }
                    Value::Str(s) => {
                        out.push(Value::str(&format!("{pad}  {k} = \"{}\"", s.as_ref())));
                    }
                    Value::Num(n) => out.push(Value::str(&format!("{pad}  {k} = {n}"))),
                    Value::Bool(b) => out.push(Value::str(&format!("{pad}  {k} = {b}"))),
                    Value::Object(obj) => {
                        out.push(Value::str(&format!("{pad}  {k} = [{}]", obj.class_name())));
                    }
                    _ => out.push(Value::str(&format!("{pad}  {k} = nil"))),
                }
            }
            out.push(Value::str(&format!("{pad}}}")));
        }
        Value::Closure(_) | Value::Native(_) => {
            out.push(Value::str(&format!("{pad}<function>")));
        }
        Value::Coroutine(_) => {
            out.push(Value::str(&format!("{pad}<thread>")));
        }
    }
}

/// Register `TileDepthTextureAssignmentManager` and `ObjectDebuggerLua`.
pub fn register(thread: &mut Thread, fs: Option<Rc<RefCell<GameFs>>>) {
    GAME_FS.with(|slot| *slot.borrow_mut() = fs);

    let mgr = expose_class(thread, ASSIGN_MGR_CLASS, Some("java.lang.Object"));
    mgr.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(AssignmentManagerHandle))))
    });
    mgr.method("init", |_| {
        GAME_FS.with(|slot| {
            if let Some(fs_rc) = slot.borrow().as_ref() {
                let loaded = TileDepthTextureAssignmentManager::init(&mut fs_rc.borrow_mut());
                MANAGER.with(|m| *m.borrow_mut() = loaded);
            }
        });
        Ok(0)
    });
    mgr.method("initGameData", |_| {
        GAME_FS.with(|slot| {
            if let Some(fs_rc) = slot.borrow().as_ref() {
                let media = PathBuf::from(fs_rc.borrow().media_root());
                let loaded = TileDepthTextureAssignmentManager::init_from_dirs(&[], &media);
                MANAGER.with(|m| *m.borrow_mut() = loaded);
            }
        });
        Ok(0)
    });
    mgr.method("initModData", |c| {
        if let Some(mod_id) = str_arg(c, 1) {
            if !mod_id.is_empty() {
                MANAGER.with(|m| {
                    m.borrow_mut().ensure_mod_data(&mod_id, PathBuf::new());
                });
            }
        }
        Ok(0)
    });
    mgr.method("save", |c| {
        let mod_id = str_arg(c, 1).unwrap_or_else(|| "game".to_string());
        MANAGER.with(|m| {
            let _ = m.borrow().save_mod(&mod_id);
        });
        Ok(0)
    });
    mgr.method("initSprites", |_| Ok(0));
    mgr.method("assignTileName", |c| {
        let Some(mod_id) = str_arg(c, 1) else {
            return Ok(0);
        };
        let Some(assign_to) = str_arg(c, 2) else {
            return Ok(0);
        };
        let other_tile = str_arg(c, 3).unwrap_or_default();
        MANAGER.with(|m| {
            let mut m = m.borrow_mut();
            if m.mod_data(&mod_id).is_none() {
                let media_dir = GAME_FS.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .filter(|_| mod_id == "game")
                        .map(|f| PathBuf::from(f.borrow().media_root()))
                        .unwrap_or_default()
                });
                m.ensure_mod_data(&mod_id, media_dir);
            }
            m.assign_tile_name(&mod_id, &assign_to, &other_tile);
        });
        Ok(0)
    });
    mgr.method("getAssignedTileName", |c| {
        let found = if c.nargs() >= 3 {
            let mod_id = str_arg(c, 1).unwrap_or_default();
            let tile_name = str_arg(c, 2).unwrap_or_default();
            MANAGER.with(|m| {
                m.borrow()
                    .assigned_tile_name_for_mod(&mod_id, &tile_name)
                    .map(str::to_string)
            })
        } else {
            let tile_name = str_arg(c, 1).unwrap_or_default();
            MANAGER.with(|m| m.borrow().assigned_tile_name(&tile_name).map(str::to_string))
        };
        Ok(c.push(found.map_or(Value::Nil, |s| Value::str(&s))))
    });
    mgr.method("clearAssignedTileName", |c| {
        if let (Some(mod_id), Some(assign_to)) = (str_arg(c, 1), str_arg(c, 2)) {
            MANAGER.with(|m| {
                m.borrow_mut().clear_assigned_tile_name(&mod_id, &assign_to);
            });
        }
        Ok(0)
    });
    mgr.method("assignDepthTextureToSprite", |_| Ok(0));

    let dbg = expose_class(thread, OBJECT_DEBUGGER_CLASS, Some("java.lang.Object"));
    dbg.static_fn("AllocList", |c| {
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });
    dbg.static_fn("ReleaseList", |c| {
        if let Value::Object(obj) = c.arg(0) {
            if let Some(list) = obj.as_any().downcast_ref::<JavaList>() {
                list.items.borrow_mut().clear();
            }
        }
        Ok(0)
    });
    dbg.static_fn("GetLines", |c| {
        let target = c.arg(0);
        let max_depth = c
            .arg(3)
            .as_num()
            .or_else(|| c.arg(2).as_num())
            .filter(|n| *n >= 0.0)
            .map(|n| (n as usize).min(6))
            .unwrap_or(2);
        if let Value::Object(obj) = c.arg(1) {
            if let Some(list) = obj.as_any().downcast_ref::<JavaList>() {
                let mut lines = Vec::new();
                format_value_lines(&target, &mut lines, 0, 0, max_depth);
                list.items.borrow_mut().extend(lines);
            }
        }
        Ok(0)
    });
    dbg.static_fn("Log", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn tile_depth_assign_and_object_debugger_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t, None);

        let dir = std::env::temp_dir().join(format!(
            "pz_tile_depth_assign_lua_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = TileDepthTextureAssignmentManager::default();
        mgr.ensure_mod_data("game", dir.clone());
        install_manager(mgr);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            local m = TileDepthTextureAssignmentManager.getInstance()
            assert(m:getAssignedTileName("game", "walls_01_1") == nil)
            m:assignTileName("game", "walls_01_1", "walls_01_0")
            assert(m:getAssignedTileName("game", "walls_01_1") == "walls_01_0")
            assert(m:getAssignedTileName("walls_01_1") == "walls_01_0")
            m:save("game")
            m:clearAssignedTileName("game", "walls_01_1")
            assert(m:getAssignedTileName("game", "walls_01_1") == nil)

            local lines = ObjectDebuggerLua.AllocList()
            ObjectDebuggerLua.GetLines({ alpha = 1, beta = "hi" }, lines)
            assert(lines:size() >= 3, "expected debug lines")
            ObjectDebuggerLua.Log(m)
            ObjectDebuggerLua.ReleaseList(lines)
            assert(lines:size() == 0, "expected cleared list")
            "#,
            Some("tile_depth_assign_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));

        let reloaded = TileDepthTextureAssignmentManager::init_from_dirs(&[], &dir);
        assert_eq!(reloaded.assigned_tile_name("walls_01_1"), Some("walls_01_0"));

        let _ = std::fs::remove_dir_all(&dir);
        reset();
    }
}
