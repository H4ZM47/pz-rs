//! `zombie.world.moddata.GlobalModData` and the Lua global `ModData` table
//! (`getOrCreate`, `get`, `create`, `exists`, `add`, `remove`, `getTableNames`,
//! `transmit`, `request`), plus `global_mod_data.bin` binary persistence using
//! Project Zomboid's `KahluaTableImpl.save`/`load` format.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use indexmap::IndexMap;
use pz_core::game_time_save::WORLD_VERSION;
use pz_inventory::io::{Reader, Writer};
use pz_inventory::lua_table::LuaTable;
use pz_inventory::moddata::{adopt_archive, save_live_table_to_writer};
use pz_kahlua::{LuaError, TableRef, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::expose::str_arg;

pub const GLOBAL_MOD_DATA_FILE: &str = "global_mod_data.bin";

thread_local! {
    /// Named global mod-data Lua tables in deterministic insertion order.
    static TABLES: RefCell<IndexMap<String, TableRef>> = RefCell::new(IndexMap::new());
}

/// `GlobalModData.reset()`: clear all registered global mod-data tables.
pub fn reset() {
    TABLES.with(|m| m.borrow_mut().clear());
}

/// `ModData.getOrCreate(tag)`.
pub fn get_or_create(tag: &str) -> TableRef {
    TABLES.with(|m| {
        m.borrow_mut()
            .entry(tag.to_owned())
            .or_insert_with(TableRef::new)
            .clone()
    })
}

/// `ModData.get(tag)`.
pub fn get(tag: &str) -> Option<TableRef> {
    TABLES.with(|m| m.borrow().get(tag).cloned())
}

/// `ModData.create(tag)`: create a fresh table, replacing any previous entry.
pub fn create(tag: &str) -> TableRef {
    let tbl = TableRef::new();
    TABLES.with(|m| {
        m.borrow_mut().insert(tag.to_owned(), tbl.clone());
    });
    tbl
}

/// `ModData.exists(tag)`.
pub fn exists(tag: &str) -> bool {
    TABLES.with(|m| m.borrow().contains_key(tag))
}

/// `ModData.add(tag, table)`.
pub fn add(tag: &str, table: TableRef) {
    TABLES.with(|m| {
        m.borrow_mut().insert(tag.to_owned(), table);
    });
}

/// `ModData.remove(tag)`.
pub fn remove(tag: &str) -> Option<TableRef> {
    TABLES.with(|m| m.borrow_mut().shift_remove(tag))
}

/// `ModData.getTableNames()`.
pub fn table_names() -> Vec<String> {
    TABLES.with(|m| m.borrow().keys().cloned().collect())
}

/// Encode all live global mod-data tables in `global_mod_data.bin` format:
/// `i32(WORLD_VERSION)`, `i32(count)`, then `(StringUTF(tag), KahluaTable)` entries.
pub fn save_bytes() -> Result<Vec<u8>, String> {
    let entries: Vec<(String, TableRef)> = TABLES.with(|m| {
        m.borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    });
    let mut w = Writer::new();
    w.i32(WORLD_VERSION);
    w.i32(entries.len() as i32);
    for (tag, tbl) in &entries {
        w.string(tag);
        save_live_table_to_writer(tbl, &mut w)
            .map_err(|e| format!("GlobalModData.save({tag}): {e:?}"))?;
    }
    Ok(w.buf)
}

/// Decode `global_mod_data.bin` bytes into the live global mod-data store.
pub fn load_bytes(bytes: &[u8]) -> Result<(), String> {
    let mut r = Reader::new(bytes);
    let version = r.i32().map_err(|e| format!("GlobalModData.load version: {e}"))?;
    let count = r.i32().map_err(|e| format!("GlobalModData.load count: {e}"))?.max(0);
    let mut loaded = IndexMap::with_capacity(count as usize);
    for _ in 0..count {
        let tag = r.string().map_err(|e| format!("GlobalModData.load tag: {e}"))?;
        let archive = LuaTable::load(&mut r, version)
            .map_err(|e| format!("GlobalModData.load({tag}): {e}"))?;
        let tbl = adopt_archive(&archive)
            .map_err(|e| format!("GlobalModData.adopt({tag}): {e:?}"))?;
        loaded.insert(tag, tbl);
    }
    TABLES.with(|m| {
        *m.borrow_mut() = loaded;
    });
    Ok(())
}

/// `GlobalModData.save()` to `<save_dir>/global_mod_data.bin`.
pub fn save_file(save_dir: &Path, no_save: bool) -> Result<(), String> {
    if no_save {
        return Ok(());
    }
    let is_empty = TABLES.with(|m| m.borrow().is_empty());
    let path = save_dir.join(GLOBAL_MOD_DATA_FILE);
    if is_empty && !path.exists() {
        return Ok(());
    }
    let bytes = save_bytes()?;
    std::fs::write(&path, bytes)
        .map_err(|e| format!("GlobalModData.save({}): {e}", path.display()))
}

/// `GlobalModData.load()` from `<save_dir>/global_mod_data.bin`.
/// Returns `Ok(true)` if an existing file was loaded, `Ok(false)` if starting fresh.
pub fn load_file(save_dir: &Path, no_save: bool) -> Result<bool, String> {
    reset();
    if no_save {
        return Ok(false);
    }
    let path = save_dir.join(GLOBAL_MOD_DATA_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            load_bytes(&bytes)?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("GlobalModData.load({}): {e}", path.display())),
    }
}

/// Expose global `ModData` table (`ModData.getOrCreate`, `get`, `create`,
/// `exists`, `add`, `remove`, `getTableNames`, `transmit`, `request`).
pub fn register(thread: &mut Thread) {
    use pz_kahlua::NativeFunction;

    let md = TableRef::new();

    md.set_str(
        "getOrCreate",
        Value::Native(NativeFunction::new("ModData.getOrCreate", |c| {
            let tag = str_arg(c, 0).ok_or_else(|| LuaError::runtime("ModData.getOrCreate: tag required"))?;
            Ok(c.push(Value::Table(get_or_create(&tag))))
        })),
    );
    md.set_str(
        "get",
        Value::Native(NativeFunction::new("ModData.get", |c| {
            let v = str_arg(c, 0)
                .and_then(|tag| get(&tag))
                .map(Value::Table)
                .unwrap_or(Value::Nil);
            Ok(c.push(v))
        })),
    );
    md.set_str(
        "create",
        Value::Native(NativeFunction::new("ModData.create", |c| {
            let tag = str_arg(c, 0).ok_or_else(|| LuaError::runtime("ModData.create: tag required"))?;
            Ok(c.push(Value::Table(create(&tag))))
        })),
    );
    md.set_str(
        "exists",
        Value::Native(NativeFunction::new("ModData.exists", |c| {
            let found = str_arg(c, 0).is_some_and(|tag| exists(&tag));
            Ok(c.push(found))
        })),
    );
    md.set_str(
        "add",
        Value::Native(NativeFunction::new("ModData.add", |c| {
            let tag = str_arg(c, 0).ok_or_else(|| LuaError::runtime("ModData.add: tag required"))?;
            let Value::Table(tbl) = c.arg(1) else {
                return Err(LuaError::runtime("ModData.add: expected table"));
            };
            add(&tag, tbl);
            Ok(0)
        })),
    );
    md.set_str(
        "remove",
        Value::Native(NativeFunction::new("ModData.remove", |c| {
            let v = str_arg(c, 0)
                .and_then(|tag| remove(&tag))
                .map(Value::Table)
                .unwrap_or(Value::Nil);
            Ok(c.push(v))
        })),
    );
    md.set_str(
        "getTableNames",
        Value::Native(NativeFunction::new("ModData.getTableNames", |c| {
            let items = table_names().into_iter().map(|s| Value::str(&s)).collect();
            Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
        })),
    );
    // Single-player no-ops matching `GlobalModData.transmit`/`request`.
    md.set_str("transmit", Value::Native(NativeFunction::new("ModData.transmit", |_| Ok(0))));
    md.set_str("request", Value::Native(NativeFunction::new("ModData.request", |_| Ok(0))));

    thread.globals().set_str("ModData", Value::Table(md));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("global_moddata_test.lua"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap()
    }

    #[test]
    fn mod_data_lua_api_and_binary_file_roundtrip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let res = run(
            &mut t,
            r#"
            local fd = ModData.getOrCreate("forageData")
            fd.version = 42
            fd.zone = "Muldraugh"
            fd.nested = { active = true }
            local same = ModData.getOrCreate("forageData")
            ModData.add("UniqueVehiclesSpawned", { police = 2 })
            ModData.transmit("forageData")
            ModData.request("forageData")
            return same == fd, ModData.exists("forageData"), ModData.getTableNames():size()
            "#,
        );
        assert!(res[0].truthy());
        assert!(res[1].truthy());
        assert_eq!(res[2].as_num(), Some(2.0));

        let dir = std::env::temp_dir().join(format!("pz-gmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        save_file(&dir, false).unwrap();
        assert!(dir.join(GLOBAL_MOD_DATA_FILE).exists());

        reset();
        assert!(!exists("forageData"));
        assert!(load_file(&dir, false).unwrap());

        let restored = run(
            &mut t,
            r#"
            local fd = ModData.get("forageData")
            local uv = ModData.get("UniqueVehiclesSpawned")
            return fd.version, fd.zone, fd.nested.active, uv.police
            "#,
        );
        assert_eq!(restored[0].as_num(), Some(42.0));
        assert_eq!(restored[1].as_str(), Some("Muldraugh"));
        assert!(restored[2].truthy());
        assert_eq!(restored[3].as_num(), Some(2.0));

        let _ = std::fs::remove_dir_all(&dir);
        reset();
    }
}
