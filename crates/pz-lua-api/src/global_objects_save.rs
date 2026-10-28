//! `gos_<name>.bin`: `SGlobalObjectSystem.save`/`load` and
//! `SGlobalObject.save`/`load`.
//!
//! Layout: `"GLOS"`, the world version, then the system's mod data (a
//! presence byte and a `ByteBuffer`-flavour table holding only the keys the
//! Lua side listed with `setModDataKeys`), the object count, and per object
//! `x`, `y` (ints), `z` (a byte) and its mod data filtered by
//! `setObjectModDataKeys`. Loading keeps every key it finds.
//!
//! [`set_save_dir`] plays the part of `ZomboidFileSystem`'s current save
//! folder: with one set, a newly registered server system loads its file
//! (`SGlobalObjects.registerSystem`).

use crate::global_objects::{debug_log, system_by_index, system_count, GlobalObjectSystem, Side};
use pz_kahlua::java_io::{JavaReader, JavaWrite};
use pz_kahlua::table_save::{load_table, save_table, Flavor};
use pz_kahlua::{TableRef, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// `IsoWorld.getWorldVersion()` in this build.
pub const WORLD_VERSION: i32 = 249;
const MAGIC: [u8; 4] = *b"GLOS";

thread_local! {
    static SAVE_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Sets (or clears) the folder `gos_*.bin` files are read from and written to.
pub fn set_save_dir(dir: Option<PathBuf>) {
    SAVE_DIR.with(|d| *d.borrow_mut() = dir);
}

/// The folder set by [`set_save_dir`].
pub fn save_dir() -> Option<PathBuf> {
    SAVE_DIR.with(|d| d.borrow().clone())
}

/// `getFileName()`.
pub fn file_name(save_dir: &Path, system: &str) -> PathBuf {
    save_dir.join(format!("gos_{system}.bin"))
}

fn lua_err(e: pz_kahlua::LuaError) -> io::Error {
    io::Error::other(format!("{e:?}"))
}

/// The entries of `t` whose string key is in `keys` (the original's
/// `tempTable`), in `t`'s order.
fn filtered(t: &TableRef, keys: &HashSet<String>) -> TableRef {
    let out = TableRef::new();
    for (k, v) in t.entries() {
        if matches!(&k, Value::Str(s) if keys.contains(&**s)) {
            out.rawset(k, v);
        }
    }
    out
}

fn save_filtered(t: &TableRef, keys: &HashSet<String>, out: &mut Vec<u8>) -> io::Result<()> {
    let temp = filtered(t, keys);
    if temp.is_empty() {
        out.put_i8(0);
        Ok(())
    } else {
        out.put_i8(1);
        save_table(&temp, out, Flavor::ByteBuffer)
    }
}

/// `SGlobalObjectSystem.save(ByteBuffer)` (no header).
pub fn save_system_data(system: &GlobalObjectSystem, out: &mut Vec<u8>) -> io::Result<()> {
    save_filtered(&system.mod_data, &system.mod_data_keys.borrow(), out)?;
    let n = system.object_count();
    out.put_i32(n as i32);
    let object_keys = system.object_mod_data_keys.borrow();
    for i in 0..n {
        let o = system.object_by_index(i as i32).expect("index in range");
        out.put_i32(o.x);
        out.put_i32(o.y);
        out.put_i8(o.z as i8);
        save_filtered(&o.mod_data, &object_keys, out)?;
    }
    Ok(())
}

/// `SGlobalObjectSystem.load(ByteBuffer, worldVersion)` (no header). Each
/// object goes through `newObject`, so a second object on the same square
/// is an error and the objects before it stay loaded.
pub fn load_system_data(
    system: &GlobalObjectSystem,
    r: &mut JavaReader<'_>,
    world_version: i32,
) -> io::Result<()> {
    if r.get_i8()? != 0 {
        load_table(&system.mod_data, r, world_version, Flavor::ByteBuffer)?;
    }
    let count = r.get_i32()?;
    for _ in 0..count.max(0) {
        let x = r.get_i32()?;
        let y = r.get_i32()?;
        let z = r.get_i8()? as i32;
        let object = system.new_object(x, y, z).map_err(lua_err)?;
        if r.get_i8()? != 0 {
            load_table(&object.mod_data, r, world_version, Flavor::ByteBuffer)?;
        }
    }
    system.loaded_world_version.set(world_version);
    Ok(())
}

/// The bytes `save()` writes: header plus [`save_system_data`].
pub fn save_system_bytes(system: &GlobalObjectSystem) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.put_i32(WORLD_VERSION);
    save_system_data(system, &mut out)?;
    Ok(out)
}

/// `load()` on file contents: checks the header and rejects newer versions.
pub fn load_system_bytes(system: &GlobalObjectSystem, data: &[u8], file: &Path) -> io::Result<()> {
    let mut r = JavaReader::new(data);
    if r.take(4)? != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "doesn't appear to be a GlobalObjectSystem file:{}",
                file.display()
            ),
        ));
    }
    let world_version = r.get_i32()?;
    if world_version > WORLD_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "file is from a newer version {world_version} of the game: {}",
                file.display()
            ),
        ));
    }
    load_system_data(system, &mut r, world_version)
}

/// `SGlobalObjectSystem.load()`: a missing file is not an error; other
/// errors are returned after logging, like the original's
/// `ExceptionLogger.logException`.
pub fn load_system(system: &GlobalObjectSystem, save_dir: &Path) -> io::Result<()> {
    let file = file_name(save_dir, &system.name);
    let result = match std::fs::read(&file) {
        Ok(data) => load_system_bytes(system, &data, &file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => Err(e),
    };
    if let Err(e) = &result {
        debug_log(&format!("ERROR: {e}"));
    }
    result
}

/// `SGlobalObjectSystem.save()`: nothing with `Core.noSave` or on a
/// multiplayer client.
pub fn save_system(
    system: &GlobalObjectSystem,
    save_dir: &Path,
    no_save: bool,
    client: bool,
) -> io::Result<()> {
    if no_save || client {
        return Ok(());
    }
    let bytes = save_system_bytes(system)?;
    std::fs::write(file_name(save_dir, &system.name), bytes)
}

/// `SGlobalObjects.save()`: every server system, in registration order.
/// The first error stops nothing: each system is attempted and the first
/// error is returned.
pub fn save_all(save_dir: &Path, no_save: bool, client: bool) -> io::Result<()> {
    let mut first = Ok(());
    for i in 0..system_count(Side::Server) {
        let system = system_by_index(Side::Server, i as i32).expect("index in range");
        if let Err(e) = save_system(&system, save_dir, no_save, client) {
            debug_log(&format!("ERROR: {e}"));
            if first.is_ok() {
                first = Err(e);
            }
        }
    }
    first
}

/// What `registerSystem` does for a new server system: load its file from
/// the folder set by [`set_save_dir`], logging errors.
pub(crate) fn load_registered(system: &GlobalObjectSystem) {
    if let Some(dir) = save_dir() {
        let _ = load_system(system, &dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_objects::{init_lookup, register_system, reset, GridBounds};

    fn keys(k: &[&str]) -> HashSet<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn setup() {
        reset(Side::Server);
        init_lookup(GridBounds {
            min_x: 0,
            min_y: 0,
            width: 4,
            height: 4,
        });
    }

    #[test]
    fn saves_only_listed_keys_and_round_trips() {
        setup();
        let s = GlobalObjectSystem::new(Side::Server, "farming").unwrap();
        s.mod_data.set_str("hours", 3.0);
        s.mod_data.set_str("Type", Value::str("not listed"));
        *s.mod_data_keys.borrow_mut() = keys(&["hours"]);
        *s.object_mod_data_keys.borrow_mut() = keys(&["state", "health"]);
        let o = s.new_object(10, 20, 0).unwrap();
        o.mod_data.set_str("state", Value::str("seeded"));
        o.mod_data.set_str("health", 50.0);
        o.mod_data.set_str("x", 10.0);
        s.new_object(11, 20, -1).unwrap();
        let bytes = save_system_bytes(&s).unwrap();

        let mut want = b"GLOS\0\0\0\xf9".to_vec();
        want.extend([1, 0, 0, 0, 1, 0, 0, 5, b'h', b'o', b'u', b'r', b's', 1]);
        want.extend(3.0f64.to_be_bytes());
        want.extend([0, 0, 0, 2, 0, 0, 0, 10, 0, 0, 0, 20, 0, 1, 0, 0, 0, 2]);
        want.extend([0, 0, 5, b's', b't', b'a', b't', b'e', 0, 0, 6]);
        want.extend(b"seeded");
        want.extend([0, 0, 6, b'h', b'e', b'a', b'l', b't', b'h', 1]);
        want.extend(50.0f64.to_be_bytes());
        want.extend([0, 0, 0, 11, 0, 0, 0, 20, 0xff, 0]);
        assert_eq!(bytes, want);

        setup();
        let t = GlobalObjectSystem::new(Side::Server, "farming").unwrap();
        load_system_bytes(&t, &bytes, Path::new("gos_farming.bin")).unwrap();
        assert_eq!(t.object_count(), 2);
        assert_eq!(t.object_by_index(1).unwrap().z, -1);
        assert_eq!(t.loaded_world_version.get(), 249);
        *t.mod_data_keys.borrow_mut() = keys(&["hours"]);
        *t.object_mod_data_keys.borrow_mut() = keys(&["state", "health"]);
        assert_eq!(save_system_bytes(&t).unwrap(), bytes);
    }

    #[test]
    fn header_checks_and_duplicates() {
        setup();
        let s = GlobalObjectSystem::new(Side::Server, "trap").unwrap();
        assert!(load_system_bytes(&s, b"NOPE\0\0\0\xf9", Path::new("f")).is_err());
        assert!(load_system_bytes(&s, b"GLOS\0\0\0\xfa\0\0\0\0\0", Path::new("f")).is_err());
        let mut dup = b"GLOS\0\0\0\xf9\0\0\0\0\x02".to_vec();
        for _ in 0..2 {
            dup.extend([0, 0, 0, 1, 0, 0, 0, 1, 0, 0]);
        }
        assert!(load_system_bytes(&s, &dup, Path::new("f")).is_err());
        assert_eq!(s.object_count(), 1);
    }

    #[test]
    fn register_loads_from_save_dir() {
        setup();
        let dir = std::env::temp_dir().join(format!("pz-gos-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = GlobalObjectSystem::new(Side::Server, "campfire").unwrap();
        s.new_object(5, 6, 0).unwrap();
        save_system(&s, &dir, false, false).unwrap();
        setup();
        set_save_dir(Some(dir.clone()));
        let r = register_system(Side::Server, "campfire").unwrap();
        set_save_dir(None);
        assert_eq!(r.object_count(), 1);
        save_system(&r, &dir, true, false).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        reset(Side::Server);
    }
}
