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

/// `ModData.create(tag)`: creates a fresh table for `tag` if one does not
/// already exist. Preserves an existing table and returns `None` for
/// duplicate names (matching `GlobalModData.create(String)`).
pub fn create(tag: &str) -> Option<TableRef> {
    if exists(tag) {
        return None;
    }
    let tbl = TableRef::new();
    TABLES.with(|m| {
        m.borrow_mut().insert(tag.to_owned(), tbl.clone());
    });
    Some(tbl)
}

/// `ModData.create()`: generates a unique table name, creates its table, and
/// returns the generated name (matching `GlobalModData.create()`).
pub fn create_anonymous() -> String {
    loop {
        let a = pz_core::random::next(i32::MAX) as u32;
        let b = pz_core::random::next(i32::MAX) as u32;
        let c = pz_core::random::next(i32::MAX) as u32;
        let d = pz_core::random::next(i32::MAX) as u32;
        let tag = format!(
            "{a:08x}-{:04x}-4{:03x}-{:04x}-{:04x}{d:08x}",
            (b >> 16) & 0xffff,
            b & 0x0fff,
            ((c >> 16) & 0x3fff) | 0x8000,
            c & 0xffff
        );
        if create(&tag).is_some() {
            return tag;
        }
    }
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

/// Encode all live global mod-data tables in native Build 42 `global_mod_data.bin`
/// format: `i32(WORLD_VERSION)`, `i32(count)`, then for each entry:
/// `i32(record_payload_bytes)`, `StringUTF(tag)`, `KahluaTable`.
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
        let mut rec = Writer::new();
        rec.string(tag);
        save_live_table_to_writer(tbl, &mut rec)
            .map_err(|e| format!("GlobalModData.save({tag}): {e:?}"))?;
        let payload_len = i32::try_from(rec.buf.len())
            .map_err(|_| format!("GlobalModData.save({tag}): record exceeds i32::MAX bytes"))?;
        w.i32(payload_len);
        w.bytes(&rec.buf);
    }
    Ok(w.buf)
}

/// Decode native `global_mod_data.bin` bytes into the live global mod-data store.
pub fn load_bytes(bytes: &[u8]) -> Result<(), String> {
    let mut r = Reader::new(bytes);
    let version = r.i32().map_err(|e| format!("GlobalModData.load version: {e}"))?;
    let count = r.i32().map_err(|e| format!("GlobalModData.load count: {e}"))?.max(0);
    let mut loaded = IndexMap::with_capacity(count as usize);
    for _ in 0..count {
        let block_size = r
            .i32()
            .map_err(|e| format!("GlobalModData.load record length: {e}"))?;
        if block_size < 0 {
            return Err(format!("GlobalModData.load: negative record length {block_size}"));
        }
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

/// `GlobalModData.save()` to `<save_dir>/global_mod_data.bin` via atomic
/// `.tmp` replacement matching `GlobalModData.save()`.
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
    let tmp_path = save_dir.join(format!("{GLOBAL_MOD_DATA_FILE}.tmp"));
    std::fs::write(&tmp_path, bytes)
        .map_err(|e| format!("GlobalModData.save({}): {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path).or_else(|_| {
        std::fs::copy(&tmp_path, &path)
            .map(|_| ())
            .and_then(|()| std::fs::remove_file(&tmp_path))
    })
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
            if c.nargs() == 0 || matches!(c.arg(0), Value::Nil) {
                return Ok(c.push(Value::str(&create_anonymous())));
            }
            let tag = str_arg(c, 0).ok_or_else(|| LuaError::runtime("ModData.create: tag must be string"))?;
            let v = create(&tag).map(Value::Table).unwrap_or(Value::Nil);
            Ok(c.push(v))
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
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
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
            -- Duplicate create(name) must preserve existing table and return nil.
            local dup = ModData.create("forageData")
            -- Zero-arg create() generates a new unique tag name.
            local anon = ModData.create()
            local anon_ok = type(anon) == "string" and ModData.exists(anon)
            ModData.remove(anon)
            ModData.add("UniqueVehiclesSpawned", { police = 2 })
            ModData.transmit("forageData")
            ModData.request("forageData")
            return same == fd, ModData.exists("forageData"), ModData.getTableNames():size(), dup == nil, ModData.get("forageData").version, anon_ok
            "#,
        );
        assert!(res[0].truthy());
        assert!(res[1].truthy());
        assert_eq!(res[2].as_num(), Some(2.0));
        assert!(res[3].truthy(), "duplicate ModData.create(tag) must return nil");
        assert_eq!(res[4].as_num(), Some(42.0), "duplicate ModData.create(tag) must preserve existing table");
        assert!(res[5].truthy(), "0-arg ModData.create() must return created tag name");

        let dir = std::env::temp_dir().join(format!("pz-gmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        save_file(&dir, false).unwrap();
        let file_path = dir.join(GLOBAL_MOD_DATA_FILE);
        assert!(file_path.exists());

        // Verify native Build 42 4-byte length-framed record layout:
        // [version: i32][count: i32] then for each entry [len: i32][payload: len bytes].
        let raw = std::fs::read(&file_path).unwrap();
        let mut r = Reader::new(&raw);
        assert_eq!(r.i32().unwrap(), WORLD_VERSION);
        assert_eq!(r.i32().unwrap(), 2);
        for _ in 0..2 {
            let rec_len = r.i32().unwrap() as usize;
            let start = r.pos;
            let _tag = r.string().unwrap();
            let _tbl = LuaTable::load(&mut r, WORLD_VERSION).unwrap();
            assert_eq!(
                r.pos - start,
                rec_len,
                "record length prefix must match serialized tag + KahluaTable bytes"
            );
        }
        assert_eq!(r.pos, raw.len());

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
