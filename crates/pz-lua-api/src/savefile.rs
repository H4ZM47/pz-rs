//! Save-folder file work behind the save globals: `PlayerDBHelper` /
//! `VehicleDBHelper` player queries on `players.db` / `vehicles.db`,
//! `map_ver.bin`, save-folder listing and the "last played" date, and the
//! menu's deleting, renaming and cleaning of save folders (`deleteSave`,
//! `renameSavefile`, `manipulateSavefile`, `deleteAllGameModeSaves`,
//! `deleteDatabase`).
//!
//! The read queries open the databases with `pz-sqlite`, read-only. The
//! original's `File.setReadable(true, false)` before those reads is not
//! repeated (it would change permissions in the user's save folder); the
//! writes in [`crate::player_db`] do repeat it.

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

// ---- save folder changes ----------------------------------------------------------

/// `File.delete()`: removes a file, a symbolic link (not its target) or an
/// empty directory; `false` when that fails.
pub fn delete(path: &str) -> bool {
    let p = jfile::normalize(path);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir(&p).is_ok(),
        Ok(_) => std::fs::remove_file(&p).is_ok(),
        Err(_) => false,
    }
}

/// `StringUtils.containsDoubleDot`.
pub fn contains_double_dot(s: &str) -> bool {
    s.contains("..") || s.contains("\0.\0.")
}

/// `GlobalObject.deleteSave(folder)` after its relative-path check, for
/// the saves folder `saves`: deletes the entries of `<saves>/<folder>`
/// (recursing into directories, symbolic links to directories included,
/// through `has_relative_path` again) and then the folder. Failures are
/// silent, as `File.delete` reports them only by its result.
pub fn delete_save(saves: &str, folder: &str, has_relative_path: &dyn Fn(&str) -> bool) {
    if has_relative_path(folder) {
        return;
    }
    let to_remove = format!("{saves}{SEP}{folder}");
    let Some(names) = list(&to_remove) else {
        return;
    };
    for name in names {
        let child = format!("{saves}{SEP}{folder}{SEP}{name}");
        if is_directory(&child) {
            delete_save(
                saves,
                &format!("{folder}{SEP}{}", file_name(&child)),
                has_relative_path,
            );
        }
        delete(&child);
    }
    delete(&to_remove);
}

/// `GlobalObject.deleteDatabase(folder)` after its relative-path check:
/// deletes `<cache>/db/<last component of folder>.db`.
pub fn delete_database(cache_dir: &str, folder: &str) {
    // Path.of(folder).getFileName(): the last name; null for a root.
    let n = jfile::normalize(folder);
    let last = n.rsplit(SEP).find(|c| !c.is_empty());
    let name = match last {
        Some(l) => l.to_string(),
        None if n.is_empty() => String::new(),
        None => "null".to_string(),
    };
    let db = format!("{cache_dir}{SEP}db{SEP}{name}.db");
    if exists(&db) {
        delete(&db);
    }
}

/// `GlobalObject.sanitizeWorldName`.
pub fn sanitize_world_name(name: &str) -> String {
    let s = name.replace(' ', "_");
    let s: String = s
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '?' | '*' | '<' | '>' | ':' | '|'))
        .collect();
    pz_fs::java::trim(&s).to_string()
}

/// `GlobalObject.isInvalidForRenameSavefile`.
fn invalid_for_rename(s: Option<&str>) -> bool {
    match s {
        None => true,
        Some(s) => s.contains('/') || s.contains('\\') || s.contains(SEP) || contains_double_dot(s),
    }
}

/// `Files.move(source, target)` without options: fails when `target`
/// exists, unless both are the same file (then nothing happens).
pub fn files_move(source: &str, target: &str) -> std::io::Result<()> {
    let (src, dst) = (jfile::normalize(source), jfile::normalize(target));
    let sm = std::fs::symlink_metadata(&src)?;
    if let Ok(tm) = std::fs::symlink_metadata(&dst) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if sm.dev() == tm.dev() && sm.ino() == tm.ino() {
                return Ok(());
            }
        }
        #[cfg(not(unix))]
        let _ = (&sm, &tm);
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("java.nio.file.FileAlreadyExistsException: {dst}"),
        ));
    }
    std::fs::rename(&src, &dst)
}

/// `GlobalObject.renameSaveFile(gameMode, oldName, newName)` (Lua
/// `renameSavefile`) for the saves folder `saves`.
pub fn rename_savefile(
    saves: &str,
    game_mode: Option<&str>,
    old_name: Option<&str>,
    new_name: Option<&str>,
) -> bool {
    if invalid_for_rename(game_mode) || invalid_for_rename(old_name) || invalid_for_rename(new_name)
    {
        return false;
    }
    let (game_mode, old_name, new_name) =
        (game_mode.unwrap(), old_name.unwrap(), new_name.unwrap());
    let world = sanitize_world_name(new_name);
    if world != new_name || world.starts_with('.') || world.ends_with('.') {
        return false;
    }
    if !exists(&format!("{saves}{SEP}{game_mode}")) {
        return false;
    }
    let old = format!("{saves}{SEP}{game_mode}{SEP}{old_name}");
    let new = format!("{saves}{SEP}{game_mode}{SEP}{world}");
    files_move(&old, &new).is_ok()
}

/// `Files.newDirectoryStream(dir, filter)` entries whose name fully
/// matches `re`, in directory order.
fn matching_entries(dir: &str, re: &regex::Regex) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(jfile::normalize(dir))? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if re.is_match(&name) {
            out.push(format!("{}{SEP}{name}", jfile::normalize(dir)));
        }
    }
    Ok(out)
}

/// `Files.deleteIfExists`.
fn delete_if_exists(path: &str) -> std::io::Result<()> {
    let p = jfile::normalize(path);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir(&p),
        Ok(_) => std::fs::remove_file(&p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn full_match(regex: &str) -> regex::Regex {
    regex::Regex::new(&format!("^(?:{regex})$")).expect("valid pattern")
}

/// `deleteSavefileFilesMatching(folder, regex)`: the first failure is
/// logged and ends the deletion.
pub fn delete_files_matching(folder: &str, regex: &str) {
    let re = full_match(regex);
    let r = (|| -> std::io::Result<()> {
        for path in matching_entries(folder, &re)? {
            println!("DELETE {path}");
            delete_if_exists(&path)?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        log_exception(folder, &e);
    }
}

/// `deleteSavefileFilesMatchingInSubdirectories(folder, regex)`.
pub fn delete_files_matching_in_subdirectories(folder: &str, regex: &str) {
    let re = full_match(regex);
    let r = (|| -> std::io::Result<()> {
        for e in std::fs::read_dir(jfile::normalize(folder))? {
            let name = e?.file_name().to_string_lossy().into_owned();
            let outer = format!("{}{SEP}{name}", jfile::normalize(folder));
            // Files.isDirectory follows links.
            if !std::path::Path::new(&outer).is_dir() {
                continue;
            }
            for path in matching_entries(&outer, &re)? {
                println!("DELETE {path}");
                delete_if_exists(&path)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = r {
        log_exception(folder, &e);
    }
}

/// The `manipulateSavefile` actions: `(name, sub-folder, pattern, in
/// sub-directories)`.
pub const SAVEFILE_ACTIONS: &[(&str, &str, &str, bool)] = &[
    (
        "DeleteAPopXYBin",
        "apop",
        "apop_-?[0-9]+_-?[0-9]+\\.bin",
        false,
    ),
    (
        "DeleteChunkDataXYBin",
        "chunkdata",
        "chunkdata_-?[0-9]+_-?[0-9]+\\.bin",
        false,
    ),
    ("DeleteEntityDataBin", "", "entity_data.bin", false),
    ("DeleteMapXYBin", "map", "-?[0-9]+\\.bin", true),
    ("DeleteMapAnimalsBin", "", "map_animals\\.bin", false),
    ("DeleteMapBasementsBin", "", "map_basements\\.bin", false),
    ("DeleteMapMetaBin", "", "map_meta\\.bin", false),
    ("DeleteMapTBin", "", "map_t\\.bin", false),
    ("DeleteMapZoneBin", "", "map_zone\\.bin", false),
    ("DeletePlayersDB", "", "players\\.db", false),
    (
        "DeletePlayerBuildingsBin",
        "",
        "player_buildings\\.bin",
        false,
    ),
    ("DeleteReanimatedBin", "", "reanimated\\.bin", false),
    ("DeleteVehiclesDB", "", "vehicles\\.db", false),
    ("DeleteZOutfitsBin", "", "z_outfits\\.bin", false),
    ("DeleteZPopVirtualBin", "zpop", "zpop_virtual\\.bin", false),
    ("DeleteZPopXYBin", "zpop", "zpop_[0-9]+_[0-9]+\\.bin", false),
];

/// `GlobalObject.manipulateSavefile(folder, action)` after its argument
/// checks, for the save folder `dir` (an existing directory). `Err` with
/// the `IllegalArgumentException` message for an unknown action;
/// `WriteModsDotTxt` is left to the caller (`Ok(false)`).
pub fn manipulate_savefile(dir: &str, action: &str) -> Result<bool, String> {
    if action == "WriteModsDotTxt" {
        return Ok(false);
    }
    let Some(&(_, sub, re, nested)) = SAVEFILE_ACTIONS.iter().find(|a| a.0 == action) else {
        return Err(format!("unknown action \"{action}\""));
    };
    let folder = if sub.is_empty() {
        dir.to_string()
    } else {
        format!("{dir}{SEP}{sub}")
    };
    if nested {
        delete_files_matching_in_subdirectories(&folder, re);
    } else {
        delete_files_matching(&folder, re);
    }
    Ok(true)
}

thread_local! {
    /// `ActiveMods.s_activeMods`, by id.
    static ACTIVE_MODS: std::cell::RefCell<Vec<(String, pz_fs::ActiveMods)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// `ActiveMods.getById(id)`: a copy of the registered mod list, registering
/// an empty one first when `id` is new.
pub fn active_mods_by_id(id: &str) -> pz_fs::ActiveMods {
    let id = pz_fs::java::trim(id).to_string();
    ACTIVE_MODS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, a)) = m.iter().find(|(k, _)| k.eq_ignore_ascii_case(&id)) {
            return a.clone();
        }
        m.push((id, pz_fs::ActiveMods::default()));
        pz_fs::ActiveMods::default()
    })
}

/// Replaces the mod list registered as `id` (`ActiveMods.getById(id)
/// .copyFrom(...)`), e.g. `currentGame` when a save is loaded.
pub fn set_active_mods(id: &str, mods: pz_fs::ActiveMods) {
    let id = pz_fs::java::trim(id).to_string();
    ACTIVE_MODS.with(|m| {
        let mut m = m.borrow_mut();
        match m.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&id)) {
            Some((_, a)) => *a = mods,
            None => m.push((id, mods)),
        }
    });
}

/// `ActiveModsFile.write(path, activeMods)`.
pub fn write_active_mods(no_save: bool, path: &str, mods: &pz_fs::ActiveMods) -> bool {
    if no_save {
        return false;
    }
    match std::fs::write(
        jfile::normalize(path),
        pz_fs::mods::active_mods_to_string(mods),
    ) {
        Ok(()) => true,
        Err(e) => {
            log_exception(path, &e);
            false
        }
    }
}

/// The `Files.walkFileTree` of `deleteAllGameModeSaves`: deletes files
/// (links included, not followed) and then each directory after its
/// entries. A directory that cannot be read is logged and skipped; the
/// first failed delete ends the walk.
pub fn delete_tree(root: &str) -> std::io::Result<()> {
    fn walk(p: &std::path::Path) -> std::io::Result<()> {
        let m = match std::fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(e) => {
                log_exception(&p.to_string_lossy(), &e);
                return Ok(());
            }
        };
        if !m.is_dir() {
            return std::fs::remove_file(p);
        }
        match std::fs::read_dir(p) {
            Ok(rd) => {
                for e in rd {
                    match e {
                        Ok(e) => walk(&e.path())?,
                        Err(e) => log_exception(&p.to_string_lossy(), &e),
                    }
                }
            }
            Err(e) => {
                log_exception(&p.to_string_lossy(), &e);
                return Ok(());
            }
        }
        std::fs::remove_dir(p)
    }
    walk(std::path::Path::new(&jfile::normalize(root)))
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
