//! `zombie.modding.ActiveMods`: named mod/map-order lists (`"default"`,
//! `"currentGame"`, ...) kept in a static registry, plus the globals that
//! edit the default list (`toggleModActive`, `saveModsFile`).
//!
//! `getMods()` and `getMapOrder()` return the live `ArrayList`s, so Lua edits
//! show through, as in Java.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};

use crate::bind_java_util::{java_equals, JavaList};
use crate::expose::{bool_arg, expose_class, global, int_arg, str_arg, this};
use crate::LuaManager;

pub const ACTIVE_MODS: &str = "zombie.modding.ActiveMods";

/// One `ActiveMods` list.
pub struct ActiveMods {
    pub id: String,
    pub mods: Rc<JavaList>,
    pub map_order: Rc<JavaList>,
}

impl LuaObject for ActiveMods {
    fn class_name(&self) -> &str {
        ACTIVE_MODS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ActiveMods {
    fn new(id: &str) -> Self {
        ActiveMods {
            id: id.to_string(),
            mods: Rc::new(JavaList::new(Vec::new())),
            map_order: Rc::new(JavaList::new(Vec::new())),
        }
    }

    /// The mod ids as strings.
    pub fn mod_ids(&self) -> Vec<String> {
        strings(&self.mods)
    }

    pub fn map_order_names(&self) -> Vec<String> {
        strings(&self.map_order)
    }

    /// `clear`.
    pub fn clear(&self) {
        self.mods.items.borrow_mut().clear();
        self.map_order.items.borrow_mut().clear();
    }

    /// `copyFrom(other)`.
    pub fn copy_from(&self, other: &ActiveMods) {
        let (m, o) = (
            other.mods.items.borrow().clone(),
            other.map_order.items.borrow().clone(),
        );
        *self.mods.items.borrow_mut() = m;
        *self.map_order.items.borrow_mut() = o;
    }

    /// Replaces the contents with a file's lists.
    pub fn set_from(&self, src: &pz_fs::ActiveMods) {
        *self.mods.items.borrow_mut() = src.mods.iter().map(|s| Value::str(s)).collect();
        *self.map_order.items.borrow_mut() = src.map_order.iter().map(|s| Value::str(s)).collect();
    }

    pub fn to_file(&self) -> pz_fs::ActiveMods {
        pz_fs::ActiveMods {
            mods: self.mod_ids(),
            map_order: self.map_order_names(),
        }
    }

    /// `setModActive(modID, active)`.
    pub fn set_mod_active(&self, mod_id: &str, active: bool) {
        let id = pz_fs::java::trim(mod_id);
        if pz_fs::java::is_null_or_whitespace(id) {
            return;
        }
        let v = Value::str(id);
        let mut items = self.mods.items.borrow_mut();
        let pos = items.iter().position(|x| java_equals(x, &v));
        match (active, pos) {
            (true, None) => items.push(v),
            (false, Some(i)) => {
                items.remove(i);
            }
            _ => {}
        }
    }

    /// `isModActive(modID)`.
    pub fn is_mod_active(&self, mod_id: &str) -> bool {
        let id = pz_fs::java::trim(mod_id);
        !pz_fs::java::is_null_or_whitespace(id)
            && self
                .mods
                .items
                .borrow()
                .iter()
                .any(|x| java_equals(x, &Value::str(id)))
    }

    /// `checkMissingMods`: drops mods that are missing or unavailable.
    pub fn check_missing_mods(&self, fs: &mut GameFs) {
        let ids = self.mods.items.borrow().clone();
        let keep: Vec<Value> = ids
            .into_iter()
            .filter(|v| {
                v.as_str()
                    .is_some_and(|id| fs.available_mod_dir(id).is_some())
            })
            .collect();
        *self.mods.items.borrow_mut() = keep;
    }
}

fn strings(l: &JavaList) -> Vec<String> {
    l.items
        .borrow()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// `ArrayList.remove(Object)`: the first equal element.
fn remove_value(l: &JavaList, v: &Value) {
    let mut items = l.items.borrow_mut();
    if let Some(i) = items.iter().position(|x| java_equals(x, v)) {
        items.remove(i);
    }
}

thread_local! {
    /// `s_activeMods`.
    static ALL: RefCell<Vec<Rc<ActiveMods>>> = const { RefCell::new(Vec::new()) };
    /// `s_loaded`.
    static LOADED: Rc<ActiveMods> = Rc::new(ActiveMods::new("loaded"));
}

fn require_valid_id(id: &str) -> LuaResult<()> {
    if pz_fs::java::is_null_or_whitespace(id) {
        return Err(LuaError::Runtime {
            message: "id is null or whitespace".into(),
            class: "java.lang.IllegalArgumentException",
        });
    }
    Ok(())
}

/// `ActiveMods.indexOf(id)` (case-insensitive, trimmed).
pub fn index_of(id: &str) -> LuaResult<Option<usize>> {
    let id = pz_fs::java::trim(id);
    require_valid_id(id)?;
    Ok(ALL.with(|a| {
        a.borrow()
            .iter()
            .position(|m| pz_fs::java::case_insensitive_cmp(&m.id, id).is_eq())
    }))
}

/// `ActiveMods.getById(id)`: creates the list on first use.
pub fn get_by_id(id: &str) -> LuaResult<Rc<ActiveMods>> {
    if let Some(i) = index_of(id)? {
        return Ok(ALL.with(|a| a.borrow()[i].clone()));
    }
    // `create(id)` keeps the untrimmed id.
    require_valid_id(id)?;
    let m = Rc::new(ActiveMods::new(id));
    ALL.with(|a| a.borrow_mut().push(m.clone()));
    Ok(m)
}

/// `ActiveMods.setLoadedMods(activeMods)`.
pub fn set_loaded_mods(m: &ActiveMods) {
    LOADED.with(|l| l.copy_from(m));
}

/// `ZomboidFileSystem.loadMods("default")`'s list handling: reads
/// `mods/default.txt` into `ActiveMods "default"`, drops missing mods and
/// records it as the loaded set. Returns the ids to pass to
/// [`GameFs::load_mods`].
pub fn read_default(fs: &mut GameFs) -> Vec<String> {
    let file = fs.read_default_mods();
    let Ok(m) = get_by_id("default") else {
        return Vec::new();
    };
    m.set_from(&file);
    m.check_missing_mods(fs);
    // TODO(pz-world): `checkMissingMaps` once MapGroups is on main.
    set_loaded_mods(&m);
    m.mod_ids()
}

fn with_self<R>(c: &NativeCall<'_>, f: impl FnOnce(&ActiveMods) -> R) -> LuaResult<R> {
    let o = this::<ActiveMods>(c)?;
    Ok(f(o.as_any().downcast_ref::<ActiveMods>().unwrap()))
}

fn value_of(m: Rc<ActiveMods>) -> Value {
    Value::Object(m)
}

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    let thread = &mut mgr.thread;
    let b = expose_class(thread, ACTIVE_MODS, None);
    b.ctor(|c| {
        let id = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        require_valid_id(&id)?;
        Ok(c.push(value_of(Rc::new(ActiveMods::new(&id)))))
    });
    b.static_fn("getById", |c| {
        let id = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let m = get_by_id(&id)?;
        Ok(c.push(value_of(m)))
    });
    b.static_fn("indexOf", |c| {
        let id = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let i = index_of(&id)?.map_or(-1.0, |i| i as f64);
        Ok(c.push(i))
    });
    b.static_fn("getByIndex", |c| {
        let i = int_arg(c, 0);
        let m = ALL.with(|a| {
            usize::try_from(i)
                .ok()
                .and_then(|i| a.borrow().get(i).cloned())
        });
        let len = ALL.with(|a| a.borrow().len());
        let m = m.ok_or_else(|| LuaError::Runtime {
            message: format!("Index {i} out of bounds for length {len}"),
            class: "java.lang.IndexOutOfBoundsException",
        })?;
        Ok(c.push(value_of(m)))
    });
    b.static_fn("setLoadedMods", |c| {
        if let Value::Object(o) = c.arg(0) {
            if let Some(m) = o.as_any().downcast_ref::<ActiveMods>() {
                set_loaded_mods(m);
            }
        }
        Ok(0)
    });
    b.static_fn("requiresResetLua", |c| {
        let o = match c.arg(0) {
            Value::Object(o) if o.as_any().is::<ActiveMods>() => o,
            _ => {
                return Err(LuaError::Runtime {
                    message: "activeMods is null".into(),
                    class: "java.lang.NullPointerException",
                })
            }
        };
        let m = o.as_any().downcast_ref::<ActiveMods>().unwrap();
        // `!s_loaded.mods.equals(activeMods.mods)`: element-wise equals.
        let differs = LOADED.with(|l| {
            let (a, b) = (l.mods.items.borrow(), m.mods.items.borrow());
            a.len() != b.len() || a.iter().zip(b.iter()).any(|(x, y)| !java_equals(x, y))
        });
        Ok(c.push(differs))
    });
    // `renderUI` draws only with the debug option `ModRenderLoaded`.
    b.static_fn("renderUI", |_c| Ok(0));
    b.static_fn("Reset", |_c| {
        LOADED.with(|l| l.clear());
        Ok(0)
    });

    b.method("getMods", |c| {
        let l = with_self(c, |m| m.mods.clone())?;
        Ok(c.push(Value::Object(l)))
    });
    b.method("getMapOrder", |c| {
        let l = with_self(c, |m| m.map_order.clone())?;
        Ok(c.push(Value::Object(l)))
    });
    b.method("clear", |c| {
        with_self(c, ActiveMods::clear)?;
        Ok(0)
    });
    b.method("copyFrom", |c| {
        let other = match c.arg(1) {
            Value::Object(o) if o.as_any().is::<ActiveMods>() => o,
            _ => {
                return Err(LuaError::Runtime {
                    message: "other is null".into(),
                    class: "java.lang.NullPointerException",
                })
            }
        };
        let other = other.as_any().downcast_ref::<ActiveMods>().unwrap();
        with_self(c, |m| m.copy_from(other))?;
        Ok(0)
    });
    b.method("setModActive", |c| {
        let id = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        let on = bool_arg(c, 2);
        with_self(c, |m| m.set_mod_active(&id, on))?;
        Ok(0)
    });
    b.method("isModActive", |c| {
        let id = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        let r = with_self(c, |m| m.is_mod_active(&id))?;
        Ok(c.push(r))
    });
    b.method("removeMod", |c| {
        let id = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        with_self(c, |m| {
            remove_value(&m.mods, &Value::str(pz_fs::java::trim(&id)))
        })?;
        Ok(0)
    });
    b.method("removeMapOrder", |c| {
        let v = c.arg(1);
        with_self(c, |m| remove_value(&m.map_order, &v))?;
        Ok(0)
    });
    let f = fs.clone();
    b.method("checkMissingMods", move |c| {
        let o = this::<ActiveMods>(c)?;
        let m = o.as_any().downcast_ref::<ActiveMods>().unwrap();
        m.check_missing_mods(&mut f.borrow_mut());
        Ok(0)
    });
    // TODO(pz-world): needs MapGroups.createGroups/checkMapConflicts.
    b.method("checkMissingMaps", |_c| Ok(0));

    // `GlobalObject.toggleModActive(mod, active)`.
    global(thread, "toggleModActive", |c| {
        let id = crate::bind_fs::mod_id_of(c, 0);
        let on = bool_arg(c, 1);
        if let Some(id) = id {
            get_by_id("default")?.set_mod_active(&id, on);
        }
        Ok(0)
    });
    // `GlobalObject.saveModsFile()` -> `ZomboidFileSystem.saveModsFile()`.
    let f = fs;
    global(thread, "saveModsFile", move |_c| {
        if crate::bind_core::shared_core().borrow().is_no_save() {
            return Ok(0);
        }
        let m = get_by_id("default")?;
        let dir = f.borrow().cache_dir().join("mods");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("default.txt"),
            pz_fs::mods::active_mods_to_string(&m.to_file()),
        );
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_live_lists() {
        let a = get_by_id("Default").unwrap();
        let b = get_by_id(" default ").unwrap();
        assert!(Rc::ptr_eq(&a, &b));
        assert!(get_by_id("  ").is_err());
        a.set_mod_active(" ModA ", true);
        a.set_mod_active("ModA", true);
        a.set_mod_active("ModB", true);
        a.set_mod_active("ModB", false);
        assert_eq!(a.mod_ids(), ["ModA"]);
        assert!(b.is_mod_active("ModA"));
        set_loaded_mods(&a);
        a.clear();
        assert!(LOADED.with(|l| l.mod_ids()) == ["ModA"]);
    }
}
