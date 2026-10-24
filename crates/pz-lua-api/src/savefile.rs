//! Save-folder reads behind the save globals: `PlayerDBHelper` /
//! `VehicleDBHelper` player queries on `players.db` / `vehicles.db`,
//! `map_ver.bin`, save-folder listing and the "last played" date.
//!
//! The databases are read with `pz-sqlite`, read-only. The original's
//! `File.setReadable(true, false)` before opening is not repeated (it would
//! change permissions in the user's save folder).

use pz_fs::jfile::{self, SEP};
use pz_sqlite::{Database, Table, Value};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// `ExceptionLogger.logException`.
fn log_exception(what: &str, e: &dyn std::fmt::Display) {
    eprintln!("ERROR: {what}: {e}");
}

/// `new File(path).exists()`: the path is normalized first, so a trailing
/// separator after a file name is ignored as in Java.
pub fn exists(path: &str) -> bool {
    Path::new(&jfile::normalize(path)).exists()
}

/// `new File(path).isDirectory()`.
pub fn is_directory(path: &str) -> bool {
    Path::new(&jfile::normalize(path)).is_dir()
}

/// `File.lastModified()`: milliseconds since the epoch, 0 when the file is
/// missing or unreadable.
pub fn last_modified(path: &str) -> i64 {
    let Ok(t) = std::fs::metadata(jfile::normalize(path)).and_then(|m| m.modified()) else {
        return 0;
    };
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// `File.list()`: entry names in directory order, `None` when `path` is not
/// a readable directory.
pub fn list(path: &str) -> Option<Vec<String>> {
    let rd = std::fs::read_dir(jfile::normalize(path)).ok()?;
    Some(
        rd.flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
    )
}

/// `new File(path).getName()`.
pub fn file_name(path: &str) -> String {
    let n = jfile::normalize(path);
    n.rsplit(SEP).next().unwrap_or("").to_string()
}

/// `new File(path).getAbsolutePath()` against the working directory.
pub fn absolute_path(path: &str) -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    jfile::absolute(path, &cwd)
}

/// `ZomboidFileSystem.ensureFolderExists(path)`.
pub fn ensure_folder_exists(path: &str) {
    let abs = absolute_path(path);
    if !Path::new(&abs).is_dir() {
        let _ = std::fs::create_dir_all(&abs);
    }
}

/// `SimpleDateFormat("dd/MM/yyyy HH:mm")` in the JVM default time zone.
pub fn format_last_played(epoch_ms: i64) -> String {
    use pz_core::debug::{LocalTime, LogClock, SystemClock};
    let offset = SystemClock::new().local_offset_millis(epoch_ms);
    let t = LocalTime::from_epoch(epoch_ms, offset);
    format!(
        "{:02}/{:02}/{:04} {:02}:{:02}",
        t.day, t.month, t.year, t.hour, t.minute
    )
}

// ---- map_ver.bin --------------------------------------------------------------

/// What `getSaveInfo` takes from `map_ver.bin`; fields stay `None` where the
/// original's reads threw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapVer {
    pub world_version: Option<i32>,
    pub map_name: Option<String>,
    pub difficulty: Option<String>,
}

/// `GameWindow.ReadString(DataInputStream)`: an int length then that many
/// UTF-16 chars; lengths above 65536 or below 0 throw.
fn read_string(r: &mut impl Read) -> Option<String> {
    let mut b4 = [0u8; 4];
    r.read_exact(&mut b4).ok()?;
    let len = i32::from_be_bytes(b4);
    if len == 0 {
        return Some(String::new());
    }
    if !(0..=65536).contains(&len) {
        return None;
    }
    let mut buf = vec![0u8; len as usize * 2];
    r.read_exact(&mut buf).ok()?;
    let units: Vec<u16> = buf
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_be_bytes(c))
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// The `map_ver.bin` part of `getSaveInfo`; `None` when the file is missing.
pub fn read_map_ver(path: &str) -> Option<MapVer> {
    if !exists(path) {
        return None;
    }
    let mut out = MapVer::default();
    let mut f = match std::fs::File::open(jfile::normalize(path)) {
        Ok(f) => std::io::BufReader::new(f),
        Err(e) => {
            log_exception(path, &e);
            return Some(out);
        }
    };
    let mut b4 = [0u8; 4];
    if let Err(e) = f.read_exact(&mut b4) {
        log_exception(path, &e);
        return Some(out);
    }
    let world_version = i32::from_be_bytes(b4);
    out.world_version = Some(world_version);
    if let Some(name) = read_string(&mut f) {
        out.map_name = Some(if name == "DEFAULT" {
            "Muldraugh, KY".to_string()
        } else {
            name
        });
    }
    if world_version < 244 {
        out.difficulty = read_string(&mut f);
    }
    Some(out)
}

// ---- PlayerDBHelper -------------------------------------------------------------

/// `rs.getBoolean(i)` in sqlite-jdbc: `getInt(i) != 0`.
fn get_boolean(v: &Value) -> bool {
    v.as_i32() != 0
}

/// `SELECT <col> FROM <table> WHERE id=?` with an int parameter: the first
/// matching row's `col`, `None` without a match.
fn select_where_id(
    db: &Database,
    table: &Table,
    col: usize,
    id: i32,
) -> pz_sqlite::Result<Option<Value>> {
    let id_col = table.column("id")?;
    if table.is_rowid_alias(id_col) {
        return Ok(db
            .row_by_rowid(table, i64::from(id))?
            .map(|r| r.get(table, col)));
    }
    // A plain column: integer comparison, as SQLite compares an INTEGER
    // parameter with numeric values (text and blobs never equal it).
    let mut found = None;
    db.for_each_row(table, |r| {
        let hit = match r.get(table, id_col) {
            Value::Integer(i) => i == i64::from(id),
            Value::Real(x) => x == f64::from(id),
            _ => false,
        };
        if hit {
            found = Some(r.get(table, col));
        }
        !hit
    })?;
    Ok(found)
}

/// `SELECT isDead FROM localPlayers WHERE id=?` → `!isDead`, `None` when no
/// row matches.
fn query_alive(path: &str, id: i32) -> pz_sqlite::Result<Option<bool>> {
    let db = Database::open(jfile::normalize(path))?;
    let t = db.table("localPlayers")?;
    let col = t.column("isDead")?;
    Ok(select_where_id(&db, &t, col, id)?.map(|v| !get_boolean(&v)))
}

/// `VehicleDBHelper.isPlayerAlive(saveDir, playerSqlId)`.
pub fn vehicle_db_is_player_alive(save_dir: &str, id: i32) -> bool {
    if exists(&format!("{save_dir}{SEP}map_p.bin")) {
        return true;
    }
    if id == -1 {
        return false;
    }
    let path = format!("{save_dir}{SEP}vehicles.db");
    if !exists(&path) {
        return false;
    }
    // Connection failures are logged; query failures (no such table, ...)
    // quietly mean "not alive".
    let db = match Database::open(jfile::normalize(&path)) {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "failed to get connection to vehicles database: {}",
                absolute_path(&path)
            );
            log_exception(&path, &e);
            return false;
        }
    };
    let q = || -> pz_sqlite::Result<Option<bool>> {
        let t = db.table("localPlayers")?;
        let col = t.column("isDead")?;
        Ok(select_where_id(&db, &t, col, id)?.map(|v| !get_boolean(&v)))
    };
    q().ok().flatten().unwrap_or(false)
}

/// `PlayerDBHelper.isPlayerAlive(saveDir, playerSqlId)`.
pub fn is_player_alive(no_save: bool, save_dir: &str, id: i32) -> bool {
    if no_save {
        return false;
    }
    if exists(&format!("{save_dir}{SEP}map_p.bin")) {
        return true;
    }
    if vehicle_db_is_player_alive(save_dir, id) {
        return true;
    }
    if id == -1 {
        return false;
    }
    let path = format!("{save_dir}{SEP}players.db");
    if !exists(&path) {
        return false;
    }
    match query_alive(&path, id) {
        Ok(alive) => alive.unwrap_or(false),
        Err(e) => {
            log_exception(&path, &e);
            false
        }
    }
}

/// One `localPlayers` row as `getPlayers` returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerRow {
    pub sql_id: i32,
    /// `rs.getString`: `None` for NULL.
    pub name: Option<String>,
    pub is_dead: bool,
}

/// `PlayerDBHelper.getPlayers(saveDir)`: `SELECT id, name, isDead FROM
/// localPlayers` in table order. `Err` where the original throws an
/// `SQLException`.
pub fn get_players(no_save: bool, save_dir: &str) -> pz_sqlite::Result<Vec<PlayerRow>> {
    if no_save {
        return Ok(Vec::new());
    }
    let path = format!("{save_dir}{SEP}players.db");
    if !exists(&path) {
        return Ok(Vec::new());
    }
    let db = Database::open(jfile::normalize(&path))?;
    let t = db.table("localPlayers")?;
    let (id, name, dead) = (t.column("id")?, t.column("name")?, t.column("isDead")?);
    let mut out = Vec::new();
    db.for_each_row(&t, |r| {
        out.push(PlayerRow {
            sql_id: r.get(&t, id).as_i32(),
            name: r.get(&t, name).as_text(),
            is_dead: get_boolean(&r.get(&t, dead)),
        });
        true
    })?;
    Ok(out)
}

// ---- save folders ---------------------------------------------------------------

/// `GlobalObject.getSaveDirectory(folder)` after its prefix check: the
/// sub-directories of `folder` in listing order (as paths), creating
/// `folder` first unless saving is off. `None` when it cannot be listed.
pub fn save_directory(folder: &str, no_save: bool) -> Option<Vec<String>> {
    let fo = format!("{folder}{SEP}");
    if !exists(&fo) && !no_save {
        let _ = std::fs::create_dir(jfile::normalize(&fo));
    }
    let names = list(&fo)?;
    Some(
        names
            .into_iter()
            .map(|n| jfile::normalize(&format!("{folder}{SEP}{n}")))
            .filter(|p| is_directory(p))
            .collect(),
    )
}

/// `GlobalObject.getSaveName(file)`: the last two path components.
pub fn save_name(path: &str) -> String {
    let n = jfile::normalize(path);
    let parts: Vec<&str> = n.split(SEP).filter(|c| !c.is_empty()).collect();
    let k = parts.len();
    format!(
        "{}{SEP}{}",
        parts.get(k.wrapping_sub(2)).copied().unwrap_or(""),
        file_name(path)
    )
}

/// `GlobalObject.getFullSaveDirectoryTable()` for the saves folder
/// `save_dir`: every `<mode>/<world>` save except under `Multiplayer`,
/// newest first (stable for equal times).
pub fn full_save_directory(save_dir: &str, no_save: bool) -> Vec<String> {
    let fo = format!("{save_dir}{SEP}");
    if !exists(&fo) {
        let _ = std::fs::create_dir(jfile::normalize(&fo));
    }
    let Some(names) = list(&fo) else {
        return Vec::new();
    };
    let mut saves = Vec::new();
    for n in names {
        let path = format!("{save_dir}{SEP}{n}");
        if !is_directory(&path) || n == "Multiplayer" {
            continue;
        }
        // `addAll(null)` would throw; a mode folder that cannot be listed
        // contributes nothing here.
        saves.extend(save_directory(&path, no_save).unwrap_or_default());
    }
    let mut keyed: Vec<(i64, String)> = saves.into_iter().map(|p| (last_modified(&p), p)).collect();
    keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
    keyed.into_iter().map(|(_, p)| save_name(&p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_ver() {
        let dir = std::env::temp_dir().join(format!("pz-mapver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("map_ver.bin");
        let s = |s: &str| {
            let mut v = (s.encode_utf16().count() as i32).to_be_bytes().to_vec();
            v.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
            v
        };
        let ps = p.to_str().unwrap();
        let mut b = 249i32.to_be_bytes().to_vec();
        b.extend(s("Valley Station, KY;Muldraugh, KY"));
        std::fs::write(&p, &b).unwrap();
        let mv = read_map_ver(ps).unwrap();
        assert_eq!(mv.world_version, Some(249));
        assert_eq!(
            mv.map_name.as_deref(),
            Some("Valley Station, KY;Muldraugh, KY")
        );
        assert_eq!(mv.difficulty, None);
        // Old versions also carry a difficulty; DEFAULT means Muldraugh.
        let mut b = 200i32.to_be_bytes().to_vec();
        b.extend(s("DEFAULT"));
        b.extend(s("Hard"));
        std::fs::write(&p, &b).unwrap();
        let mv = read_map_ver(ps).unwrap();
        assert_eq!(mv.map_name.as_deref(), Some("Muldraugh, KY"));
        assert_eq!(mv.difficulty.as_deref(), Some("Hard"));
        // Truncated name, negative length, short file.
        let mut b = 249i32.to_be_bytes().to_vec();
        b.extend(5i32.to_be_bytes());
        b.extend([0, 65]);
        std::fs::write(&p, &b).unwrap();
        assert_eq!(
            read_map_ver(ps).unwrap(),
            MapVer {
                world_version: Some(249),
                ..Default::default()
            }
        );
        let mut b = 1i32.to_be_bytes().to_vec();
        b.extend((-1i32).to_be_bytes());
        std::fs::write(&p, &b).unwrap();
        assert_eq!(
            read_map_ver(ps).unwrap(),
            MapVer {
                world_version: Some(1),
                ..Default::default()
            }
        );
        std::fs::write(&p, [0, 0]).unwrap();
        assert_eq!(read_map_ver(ps).unwrap(), MapVer::default());
        std::fs::remove_file(&p).unwrap();
        assert_eq!(read_map_ver(ps), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names() {
        let s = SEP;
        assert_eq!(
            save_name(&format!("{s}a{s}Saves{s}Sandbox{s}w1")),
            format!("Sandbox{s}w1")
        );
        assert_eq!(file_name(&format!("x{s}y{s}")), "y");
        let d = format_last_played(0);
        assert_eq!(d.len(), 16);
        assert_eq!(&d[2..3], "/");
    }
}
