//! Save-game `GlobalObject` globals: the save-folder listing and info the
//! main menu and `LoadGameScreen` use (`getFullSaveDirectoryTable`,
//! `getSaveInfo`, `getLastPlayedDate`, ...), `getMapInfo`
//! (`ChooseGameInfo.getMapDetails`), and `java.io.File` objects for
//! `getSaveDirectory`.
//!
//! Paths are built as the original builds them: `<cache>/Saves` (created
//! when missing, as `getSaveDir` does), then `<mode>/<world>`. The file
//! reads live in [`crate::savefile`].
//!
//! The menu's save-folder changes are here too: `deleteSave`,
//! `deleteDatabase`, `deleteAllGameModeSaves`, `renameSavefile`,
//! `manipulateSavefile` and `setSavefilePlayer1` (the `players.db` change
//! is in [`crate::player_db`]).

use crate::bind_core::shared_core;
use crate::bind_java_util::list_value;
use crate::bind_render::{texture_value, textures};
use crate::expose::{expose_class, global, int_arg, str_arg, with_this};
use crate::savefile;
use crate::LuaManager;
use pz_fs::jfile::{self, SEP};
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::rc::Rc;

pub const FILE: &str = "java.io.File";
const ACTIVE_MODS: &str = "zombie.modding.ActiveMods";

fn java_error(message: impl Into<String>, class: &'static str) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

/// A Java `String` argument; `null` concatenates as "null".
fn string_arg(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map_or_else(|| "null".to_string(), |s| s.to_string())
}

/// `ZomboidFileSystem.getSaveDir()`: `<cache>/Saves`, created if missing.
pub fn save_dir(fs: &RefCell<GameFs>) -> String {
    let cache = fs.borrow().cache_dir().to_string_lossy().into_owned();
    let dir = format!("{cache}{SEP}Saves");
    savefile::ensure_folder_exists(&dir);
    dir
}

/// `getCurrentSaveDir()`: `<Saves>/<Core.gameMode>/<Core.gameSaveWorld>`.
fn current_save_dir(fs: &RefCell<GameFs>) -> String {
    let saves = save_dir(fs);
    let core = shared_core();
    let core = core.borrow();
    format!(
        "{saves}{SEP}{}{SEP}{}",
        core.game_mode, core.game_save_world
    )
}

fn is_no_save() -> bool {
    shared_core().borrow().is_no_save()
}

/// `Translator.getText(key)`, the key itself when no translator is set up.
fn text(key: &str) -> String {
    crate::bind_text::with_translator(|t| t.get_text(key, &[])).unwrap_or_else(|| key.to_string())
}

// ---- java.io.File ---------------------------------------------------------------

/// A `java.io.File` holding a normalized path.
pub struct JFile {
    pub path: String,
}

impl LuaObject for JFile {
    fn class_name(&self) -> &str {
        FILE
    }
    fn to_lua_string(&self) -> String {
        self.path.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn file_value(path: String) -> Value {
    Value::Object(Rc::new(JFile {
        path: jfile::normalize(&path),
    }))
}

fn file_string(c: &mut NativeCall<'_>, f: fn(&str) -> String) -> LuaResult<usize> {
    let r = with_this::<JFile, _>(c, |j| f(&j.path))?;
    Ok(c.push(r))
}

fn file_bool(c: &mut NativeCall<'_>, f: fn(&str) -> bool) -> LuaResult<usize> {
    let r = with_this::<JFile, _>(c, |j| f(&j.path))?;
    Ok(c.push(r))
}

fn register_file(thread: &mut Thread) {
    let b = expose_class(thread, FILE, None);
    b.method("getName", |c| file_string(c, savefile::file_name));
    b.method("getPath", |c| file_string(c, str::to_string));
    b.method("toString", |c| file_string(c, str::to_string));
    b.method("getAbsolutePath", |c| {
        file_string(c, savefile::absolute_path)
    });
    b.method("getParent", |c| {
        let r = with_this::<JFile, _>(c, |j| j.path.rsplit_once(SEP).map(|(p, _)| p.to_string()))?;
        Ok(c.push(r.map_or(Value::Nil, Value::from)))
    });
    b.method("exists", |c| file_bool(c, savefile::exists));
    b.method("isDirectory", |c| file_bool(c, savefile::is_directory));
    b.method("isFile", |c| {
        file_bool(c, |p| std::path::Path::new(p).is_file())
    });
    b.method("lastModified", |c| {
        let r = with_this::<JFile, _>(c, |j| savefile::last_modified(&j.path) as f64)?;
        Ok(c.push(r))
    });
}

// ---- ActiveMods for getSaveInfo ---------------------------------------------------

/// `ActiveMods` stand-in used only while no `ActiveMods` class with a
/// constructor is bound: `getMods()` and `getMapOrder()` as live lists.
struct SaveActiveMods {
    mods: Value,
    map_order: Value,
}

impl LuaObject for SaveActiveMods {
    fn class_name(&self) -> &str {
        ACTIVE_MODS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn call_method(thread: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
    let f = thread.table_get(obj, &Value::str(name))?;
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    thread.call1(&f, &all)
}

/// `new ActiveMods(saveDir)` filled from `mods.txt`. Uses the bound
/// `zombie.modding.ActiveMods` constructor when there is one, so the
/// object is the real class; otherwise a minimal stand-in.
fn active_mods_value(thread: &mut Thread, id: &str, m: &pz_fs::ActiveMods) -> LuaResult<Value> {
    let mut class = Value::Table(thread.globals().clone());
    for k in ["zombie", "modding", "ActiveMods"] {
        class = class.as_table().map_or(Value::Nil, |t| t.get_str(k));
    }
    let ctor = class
        .as_table()
        .map(|t| t.get_str("new"))
        .filter(|f| !f.is_nil());
    if let Some(ctor) = ctor {
        let obj = thread.call1(&ctor, &[Value::str(id)])?;
        let mods = call_method(thread, &obj, "getMods", &[])?;
        for s in &m.mods {
            call_method(thread, &mods, "add", &[Value::str(s)])?;
        }
        let maps = call_method(thread, &obj, "getMapOrder", &[])?;
        for s in &m.map_order {
            call_method(thread, &maps, "add", &[Value::str(s)])?;
        }
        return Ok(obj);
    }
    let methods = thread
        .class_metatables
        .get_str(ACTIVE_MODS)
        .as_table()
        .map(|t| t.get_str("__index"));
    if !methods
        .as_ref()
        .and_then(Value::as_table)
        .is_some_and(|t| !t.get_str("getMods").is_nil())
    {
        let b = expose_class(thread, ACTIVE_MODS, None);
        b.method("getMods", |c| {
            let l = with_this::<SaveActiveMods, _>(c, |m| m.mods.clone())?;
            Ok(c.push(l))
        });
        b.method("getMapOrder", |c| {
            let l = with_this::<SaveActiveMods, _>(c, |m| m.map_order.clone())?;
            Ok(c.push(l))
        });
    }
    let strings = |v: &[String]| list_value(v.iter().map(|s| Value::str(s)).collect());
    Ok(Value::Object(Rc::new(SaveActiveMods {
        mods: strings(&m.mods),
        map_order: strings(&m.map_order),
    })))
}

// ---- ChooseGameInfo.getMapDetails ------------------------------------------------

/// `ChooseGameInfo.Map`, the fields `getMapInfo` returns.
struct MapDetails {
    dir: String,
    title: String,
    lots: Vec<String>,
    desc: Option<String>,
    zoom: [f32; 3],
    demo_video: Option<String>,
    only_for_game_mode: Value,
    thumb: Value,
    worldmap: Value,
    pyramid: Option<String>,
}

thread_local! {
    /// `ChooseGameInfo.Maps`.
    static MAPS: RefCell<HashMap<String, Rc<MapDetails>>> = RefCell::new(HashMap::new());
}

/// `Float.parseFloat`.
fn parse_float(s: &str) -> Option<f32> {
    pz_kahlua::number::java_parse_double(s).map(|d| d as f32)
}

/// `Registries.GAME_MODE.get(ResourceLocation.of(s))`, through the bound
/// `GameMode` / `ResourceLocation` classes. TODO(registries): nil until
/// those classes are bound.
fn game_mode(thread: &mut Thread, s: &str) -> LuaResult<Value> {
    let g = thread.globals().clone();
    let (rl, gm) = (g.get_str("ResourceLocation"), g.get_str("GameMode"));
    let (Some(rl), Some(gm)) = (rl.as_table(), gm.as_table()) else {
        return Ok(Value::Nil);
    };
    let (of, get) = (rl.get_str("of"), gm.get_str("get"));
    if of.is_nil() || get.is_nil() {
        return Ok(Value::Nil);
    }
    let loc = thread.call1(&of, &[Value::str(s)])?;
    thread.call1(&get, &[loc])
}

/// `ChooseGameInfo.getMapDetails(dir)`: `None` for a missing or unreadable
/// `map.info` (failures are not cached).
fn map_details(thread: &mut Thread, fs: &RefCell<GameFs>, dir: &str) -> Option<Rc<MapDetails>> {
    if let Some(m) = MAPS.with(|m| m.borrow().get(dir).cloned()) {
        return Some(m);
    }
    let file = fs
        .borrow()
        .get_string(&format!("media/maps/{dir}/map.info"));
    if !savefile::exists(&file) {
        return None;
    }
    let parent = jfile::normalize(&file)
        .rsplit_once(SEP)
        .map_or(String::new(), |(p, _)| p.to_string());
    let mut map = MapDetails {
        dir: savefile::absolute_path(&parent),
        title: dir.to_string(),
        lots: Vec::new(),
        desc: None,
        zoom: [0.0; 3],
        demo_video: None,
        only_for_game_mode: Value::Nil,
        thumb: Value::Nil,
        worldmap: Value::Nil,
        pyramid: None,
    };
    let fail = |e: &dyn std::fmt::Display| {
        eprintln!("ERROR: {file}: {e}");
        None
    };
    let f = match std::fs::File::open(jfile::normalize(&file)) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    // An IOException while reading ends the loop; the rest still runs.
    for line in std::io::BufReader::new(f).lines() {
        let Ok(line) = line else { break };
        let line = pz_fs::java::trim(&line);
        let num = |key: &str| parse_float(&line.replace(key, ""));
        let zoom = |i: usize, key: &str, map: &mut MapDetails| {
            num(key)
                .map(|v| map.zoom[i] = v)
                .ok_or("java.lang.NumberFormatException")
        };
        let r = if line.starts_with("title=") {
            map.title = line.replace("title=", "");
            Ok(())
        } else if line.starts_with("lots=") {
            map.lots
                .push(pz_fs::java::trim(&line.replace("lots=", "")).to_string());
            Ok(())
        } else if line.starts_with("description=") {
            map.desc
                .get_or_insert_with(String::new)
                .push_str(&line.replace("description=", ""));
            Ok(())
        } else if line.starts_with("zoomX=") {
            zoom(0, "zoomX=", &mut map)
        } else if line.starts_with("zoomY=") {
            zoom(1, "zoomY=", &mut map)
        } else if line.starts_with("zoomS=") {
            zoom(2, "zoomS=", &mut map)
        } else if line.starts_with("demoVideo=") {
            map.demo_video = Some(line.replace("demoVideo=", ""));
            Ok(())
        } else if line.starts_with("only_for_game_mode=") {
            match game_mode(thread, &line.replace("only_for_game_mode=", "")) {
                Ok(v) => {
                    map.only_for_game_mode = v;
                    Ok(())
                }
                Err(e) => return fail(&e),
            }
        } else {
            // `fixed2x=` is read too, but getMapInfo does not return it.
            Ok(())
        };
        if let Err(e) = r {
            return fail(&e);
        }
    }
    // TODO(render): the original loads these with TextureID.useFiltering
    // as it is (off); filtering is not modelled by the texture manager.
    let shared = |name: &str| {
        let t = textures().borrow_mut().shared_texture(&fs.borrow(), name);
        texture_value(t)
    };
    map.thumb = shared(&format!("{}/thumb.png", map.dir));
    let pyramid = fs
        .borrow()
        .get_string(&format!("{}/spawnSelectImagePyramid.zip", map.dir));
    if savefile::exists(&pyramid) {
        map.pyramid = Some(pyramid);
    } else {
        map.worldmap = shared(&format!("{}/worldmap.png", map.dir));
    }
    match crate::bind_text::with_translator_mut(|t| t.read_map_translation(dir)) {
        Some(Ok((title, desc))) => {
            if let Some(t) = title {
                map.title = t;
            }
            if desc.is_some() {
                map.desc = desc;
            }
        }
        Some(Err(e)) => return fail(&e),
        None => {}
    }
    let map = Rc::new(map);
    MAPS.with(|m| m.borrow_mut().insert(dir.to_string(), map.clone()));
    Some(map)
}

// ---- globals --------------------------------------------------------------------

fn opt(s: &Option<String>) -> Value {
    s.as_deref().map_or(Value::Nil, Value::str)
}

/// `getSaveInfo(saveDir)`.
fn save_info(c: &mut NativeCall<'_>, fs: &RefCell<GameFs>) -> LuaResult<Value> {
    let Some(arg) = str_arg(c, 0) else {
        return Err(java_error(
            "saveDir is null",
            "java.lang.NullPointerException",
        ));
    };
    let mut save = arg.to_string();
    if !save.contains(SEP) {
        let mode = shared_core().borrow().game_mode.clone();
        save = format!("{mode}{SEP}{save}");
    }
    let no_save = is_no_save();
    let t = TableRef::new();
    let path = format!("{}{SEP}{save}", save_dir(fs));
    if savefile::exists(&path) {
        t.set_str(
            "lastPlayed",
            savefile::format_last_played(savefile::last_modified(&path)),
        );
        let parts = pz_fs::java::split_char(&save, SEP);
        t.set_str("saveDir", save.as_str());
        t.set_str("saveName", savefile::file_name(&path));
        let Some(mode) = parts.len().checked_sub(2).and_then(|i| parts.get(i)) else {
            let msg = format!(
                "Index {} out of bounds for length {}",
                parts.len() as i64 - 2,
                parts.len()
            );
            return Err(java_error(msg, "java.lang.ArrayIndexOutOfBoundsException"));
        };
        t.set_str("gameMode", mode.as_str());
    }
    if let Some(mv) = savefile::read_map_ver(&format!("{path}{SEP}map_ver.bin")) {
        if let Some(v) = mv.world_version {
            t.set_str("worldVersion", f64::from(v));
        }
        t.set_str("mapName", opt(&mv.map_name));
        t.set_str("difficulty", opt(&mv.difficulty));
    }
    let mods_txt = jfile::normalize(&format!("{path}{SEP}mods.txt"));
    if let Some(m) = pz_fs::mods::read_active_mods(std::path::Path::new(&mods_txt)) {
        let v = active_mods_value(c.thread, &save, &m)?;
        t.set_str("activeMods", v);
    }
    t.set_str("playerAlive", savefile::is_player_alive(no_save, &path, 1));
    let players = TableRef::new();
    match savefile::get_players(no_save, &path) {
        Ok(rows) => {
            for (i, p) in rows.into_iter().enumerate() {
                let pt = TableRef::new();
                pt.set_str("sqlID", f64::from(p.sql_id));
                pt.set_str("name", opt(&p.name));
                pt.set_str("isDead", p.is_dead);
                players.set_int(i as i64 + 1, pt);
            }
        }
        Err(e) => eprintln!("ERROR: {path}{SEP}players.db: {e}"),
    }
    t.set_str("players", players);
    Ok(Value::Table(t))
}

/// `getMapInfo(mapDir)`.
fn map_info(c: &mut NativeCall<'_>, fs: &RefCell<GameFs>) -> LuaResult<Value> {
    let mut dir = string_arg(c, 0);
    if dir.contains(';') {
        dir = pz_fs::java::split_char(&dir, ';')
            .into_iter()
            .next()
            .ok_or_else(|| {
                java_error(
                    "Index 0 out of bounds for length 0",
                    "java.lang.ArrayIndexOutOfBoundsException",
                )
            })?;
    }
    let Some(m) = map_details(c.thread, fs, &dir) else {
        return Ok(Value::Nil);
    };
    let t = TableRef::new();
    t.set_str("description", opt(&m.desc));
    t.set_str("dir", m.dir.as_str());
    let lots = TableRef::new();
    // The original stores every lot at index 1, so the last one wins.
    for l in &m.lots {
        lots.set_int(1, l.as_str());
    }
    t.set_str("lots", lots);
    t.set_str("thumb", m.thumb.clone());
    t.set_str("title", m.title.as_str());
    t.set_str("worldmap", m.worldmap.clone());
    t.set_str("spawnSelectImagePyramid", opt(&m.pyramid));
    t.set_str("zoomX", f64::from(m.zoom[0]));
    t.set_str("zoomY", f64::from(m.zoom[1]));
    t.set_str("zoomS", f64::from(m.zoom[2]));
    t.set_str("demoVideo", opt(&m.demo_video));
    t.set_str("only_for_game_mode", m.only_for_game_mode.clone());
    Ok(Value::Table(t))
}

/// `GlobalObject.hasRelativePath` (`StringUtils.containsDoubleDot`).
fn has_relative_path(s: &str) -> bool {
    if s.contains("..") || s.contains("\0.\0.") {
        pz_core::debug::with(|d| {
            let _ = d.warn(
                pz_core::debug::DebugType::Lua,
                None,
                "relative paths not allowed",
                &[],
            );
        });
        return true;
    }
    false
}

/// Registers a global taking the shared `GameFs`.
fn fs_global(
    mgr: &mut LuaManager,
    name: &str,
    f: impl Fn(&mut NativeCall<'_>, &RefCell<GameFs>) -> LuaResult<Value> + 'static,
) {
    let fs = mgr.fs.clone();
    global(&mut mgr.thread, name, move |c| {
        let v = f(c, &fs)?;
        Ok(c.push(v))
    });
}

pub fn register(mgr: &mut LuaManager) {
    {
        let fs = mgr.fs.clone();
        // <cache>/Lua/<filename> exists.
        crate::expose::global(&mut mgr.thread, "cacheFileExists", move |c| {
            let name = crate::expose::str_arg(c, 0).map(|s| s.replace('\\', "/")).unwrap_or_default();
            let p = fs.borrow().cache_dir().join("Lua").join(name);
            Ok(c.push(p.exists()))
        });
    }
    register_file(&mut mgr.thread);

    // Lines of <cache>/latestSave.ini, 1-based; empty when missing.
    fs_global(mgr, "getLatestSave", |_, fs| {
        let t = TableRef::new();
        let path = fs.borrow().cache_dir().join("latestSave.ini");
        if let Ok(f) = std::fs::File::open(path) {
            for (i, line) in std::io::BufReader::new(f).lines().enumerate() {
                let Ok(line) = line else { break };
                t.set_int(i as i64 + 1, line);
            }
        }
        Ok(Value::Table(t))
    });

    fs_global(mgr, "getSaveDirectory", |c, fs| {
        let folder = string_arg(c, 0);
        if !fs.borrow().validate_prefix(&folder) {
            return Err(java_error(
                format!("Invalid prefix found for: {folder}"),
                "java.lang.IllegalArgumentException",
            ));
        }
        let r = savefile::save_directory(&folder, is_no_save());
        Ok(r.map_or(Value::Nil, |v| {
            list_value(v.into_iter().map(file_value).collect())
        }))
    });

    fs_global(mgr, "getFullSaveDirectoryTable", |_, fs| {
        let t = TableRef::new();
        for (i, s) in savefile::full_save_directory(&save_dir(fs), is_no_save())
            .into_iter()
            .enumerate()
        {
            t.set_int(i as i64 + 1, s);
        }
        Ok(Value::Table(t))
    });

    global(&mut mgr.thread, "getSaveDirectoryTable", |c| {
        Ok(c.push(TableRef::new()))
    });

    fs_global(mgr, "getSaveInfo", save_info);

    fs_global(mgr, "checkSaveFolderExists", |c, fs| {
        Ok(savefile::exists(&format!("{}{SEP}{}", save_dir(fs), string_arg(c, 0))).into())
    });

    fs_global(mgr, "getAbsoluteSaveFolderName", |c, fs| {
        Ok(savefile::absolute_path(&format!("{}{SEP}{}", save_dir(fs), string_arg(c, 0))).into())
    });

    fs_global(mgr, "checkSaveFileExists", |c, fs| {
        Ok(savefile::exists(&format!(
            "{}{SEP}{}",
            current_save_dir(fs),
            string_arg(c, 0)
        ))
        .into())
    });

    // Single player: the GameClient branch (ClientPlayerDB) is not ported.
    fs_global(mgr, "checkSavePlayerExists", |_, fs| {
        Ok(savefile::is_player_alive(is_no_save(), &current_save_dir(fs), 1).into())
    });

    fs_global(mgr, "getLastPlayedDate", |c, fs| {
        let path = format!("{}{SEP}{}", save_dir(fs), string_arg(c, 0));
        let prefix = text("UI_LastPlayed");
        Ok(if savefile::exists(&path) {
            format!(
                "{prefix}{}",
                savefile::format_last_played(savefile::last_modified(&path))
            )
        } else {
            format!("{prefix}???")
        }
        .into())
    });

    fs_global(mgr, "getTextureFromSaveDir", |c, fs| {
        let (file, save) = (string_arg(c, 0), string_arg(c, 1));
        if has_relative_path(&file) || has_relative_path(&save) {
            return Ok(Value::Nil);
        }
        // TODO(render): TextureID.useFiltering is switched on around this
        // load in the original.
        let path = format!("{}{SEP}{save}{SEP}{file}", save_dir(fs));
        let t = textures().borrow_mut().shared_texture(&fs.borrow(), &path);
        Ok(texture_value(t))
    });

    fs_global(mgr, "getMapInfo", map_info);

    register_save_changes(mgr);
}

fn null_pointer() -> LuaError {
    java_error("null", "java.lang.NullPointerException")
}

fn register_save_changes(mgr: &mut LuaManager) {
    fs_global(mgr, "deleteSave", |c, fs| {
        let folder = string_arg(c, 0);
        if !has_relative_path(&folder) {
            savefile::delete_save(&save_dir(fs), &folder, &has_relative_path);
        }
        Ok(Value::Nil)
    });

    fs_global(mgr, "deleteDatabase", |c, fs| {
        let Some(folder) = str_arg(c, 0) else {
            return Err(null_pointer());
        };
        if !has_relative_path(&folder) {
            let cache = fs.borrow().cache_dir().to_string_lossy().into_owned();
            savefile::delete_database(&cache, &folder);
        }
        Ok(Value::Nil)
    });

    fs_global(mgr, "deleteAllGameModeSaves", |c, fs| {
        let core = shared_core();
        let old = core.borrow().game_mode.clone();
        // A null mode becomes "Sandbox" in getGameModeCacheDir.
        let mode = str_arg(c, 0).map_or_else(|| "Sandbox".to_string(), |s| s.to_string());
        core.borrow_mut().set_game_mode(&mode);
        let path = format!("{}{SEP}{mode}", save_dir(fs));
        if savefile::exists(&path) {
            if let Err(e) = savefile::delete_tree(&path) {
                eprintln!("ERROR: General: {e}");
            }
        }
        core.borrow_mut().set_game_mode(&old);
        Ok(Value::Nil)
    });

    fs_global(mgr, "renameSavefile", |c, fs| {
        let (mode, old, new) = (str_arg(c, 0), str_arg(c, 1), str_arg(c, 2));
        Ok(savefile::rename_savefile(
            &save_dir(fs),
            mode.as_deref(),
            old.as_deref(),
            new.as_deref(),
        )
        .into())
    });

    fs_global(mgr, "manipulateSavefile", |c, fs| {
        let Some(folder) = str_arg(c, 0) else {
            return Ok(Value::Nil);
        };
        if pz_fs::java::is_null_or_whitespace(&folder) || has_relative_path(&folder) {
            return Ok(Value::Nil);
        }
        let dir = format!("{}{SEP}{folder}", save_dir(fs));
        if !savefile::exists(&dir) || !savefile::is_directory(&dir) {
            return Ok(Value::Nil);
        }
        let Some(action) = str_arg(c, 1) else {
            return Err(null_pointer());
        };
        match savefile::manipulate_savefile(&dir, &action) {
            Err(msg) => Err(java_error(msg, "java.lang.IllegalArgumentException")),
            Ok(true) => Ok(Value::Nil),
            Ok(false) => {
                // WriteModsDotTxt
                let mods = savefile::active_mods_by_id("currentGame");
                savefile::write_active_mods(is_no_save(), &format!("{dir}{SEP}mods.txt"), &mods);
                Ok(Value::Nil)
            }
        }
    });

    fs_global(mgr, "setSavefilePlayer1", |c, fs| {
        let path = format!(
            "{}{SEP}{}{SEP}{}",
            save_dir(fs),
            string_arg(c, 0),
            string_arg(c, 1)
        );
        if let Err(e) = crate::player_db::set_player1(is_no_save(), &path, int_arg(c, 2)) {
            eprintln!("ERROR: {path}: {e}");
        }
        Ok(Value::Nil)
    });
}

#[cfg(test)]
mod tests;
