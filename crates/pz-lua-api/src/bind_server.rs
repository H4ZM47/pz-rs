//! Lua bindings for `ServerOptions`, `ServerSettings`,
//! `ServerSettingsManager` and the server spawn files, with the local
//! behaviour of a client that is not running a server.

use crate::bind_sandbox::{construct as new_sandbox, SandboxObj, Shared as SharedSandbox};
use crate::bind_text::with_translator;
use crate::expose::{expose_class, global, int_arg, num_arg, str_arg, with_this};
use crate::lua_manager::{run_lua, Loader};
use pz_classfile::Jar;
use pz_core::config::OptionKind as CfgKind;
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_sandbox::{config_object, lua_value};
use pz_server_settings::options::{read_defs, ServerOptionDefs};
use pz_server_settings::settings::{reserved_names, SUFFIXES};
use pz_server_settings::spawn_regions::{self, Profession, Region};
use pz_server_settings::{file_name_sanitize, ServerOption, ServerOptions, ServerSettingsManager};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type SharedOptions = Rc<RefCell<ServerOptions>>;

/// `GameServer.serverName` on a client that is not hosting.
const DEFAULT_SERVER_NAME: &str = "servertest";

struct Env {
    fs: Rc<RefCell<GameFs>>,
    loader: Rc<RefCell<Loader>>,
    defs: Option<Rc<ServerOptionDefs>>,
    reserved: Option<Rc<Vec<String>>>,
}

thread_local! {
    static ENV: RefCell<Option<Env>> = const { RefCell::new(None) };
    static INSTANCE: RefCell<Option<Value>> = const { RefCell::new(None) };
    static OPTION_OBJECTS: RefCell<HashMap<(usize, usize), Value>> = RefCell::new(HashMap::new());
    static MANAGER: RefCell<Option<Value>> = const { RefCell::new(None) };
}

fn runtime(class: &'static str, message: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

fn jar() -> LuaResult<Jar> {
    let path = ENV.with(|e| {
        e.borrow()
            .as_ref()
            .map(|e| e.fs.borrow().base.join("projectzomboid.jar"))
    });
    let path = path.ok_or_else(|| {
        runtime(
            "java.lang.IllegalStateException",
            "server settings used before init",
        )
    })?;
    Jar::open(&path).map_err(|err| {
        runtime(
            "java.lang.RuntimeException",
            format!("{}: {err}", path.display()),
        )
    })
}

fn defs() -> LuaResult<Rc<ServerOptionDefs>> {
    if let Some(d) = ENV.with(|e| e.borrow().as_ref().and_then(|e| e.defs.clone())) {
        return Ok(d);
    }
    let d = Rc::new(
        read_defs(&jar()?)
            .map_err(|e| runtime("java.lang.RuntimeException", format!("ServerOptions: {e}")))?,
    );
    ENV.with(|e| e.borrow_mut().as_mut().map(|e| e.defs = Some(d.clone())));
    Ok(d)
}

fn reserved() -> LuaResult<Rc<Vec<String>>> {
    if let Some(r) = ENV.with(|e| e.borrow().as_ref().and_then(|e| e.reserved.clone())) {
        return Ok(r);
    }
    let r = Rc::new(
        reserved_names(&jar()?)
            .map_err(|e| runtime("java.lang.RuntimeException", format!("FileName: {e}")))?,
    );
    ENV.with(|e| {
        e.borrow_mut()
            .as_mut()
            .map(|e| e.reserved = Some(r.clone()))
    });
    Ok(r)
}

fn manager() -> ServerSettingsManager {
    let cache = ENV
        .with(|e| {
            e.borrow()
                .as_ref()
                .map(|e| e.fs.borrow().cache_dir().to_path_buf())
        })
        .unwrap_or_default();
    ServerSettingsManager::new(cache)
}

fn settings_folder() -> PathBuf {
    PathBuf::from(manager().settings_folder())
}

fn key(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

fn opt_str(v: Option<String>) -> Value {
    v.map_or(Value::Nil, Value::from)
}

// ---- ServerOptions ----------------------------------------------------------------

/// `zombie.network.ServerOptions`.
struct ServerOptionsObj(SharedOptions);

impl LuaObject for ServerOptionsObj {
    fn class_name(&self) -> &str {
        "zombie.network.ServerOptions"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ServerOptionObj {
    owner: SharedOptions,
    index: usize,
    class: &'static str,
}

impl LuaObject for ServerOptionObj {
    fn class_name(&self) -> &str {
        self.class
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["zombie.config.ConfigOption"]
    }
}

fn options_value(o: SharedOptions) -> Value {
    Value::Object(Rc::new(ServerOptionsObj(o)))
}

fn option_value(owner: &SharedOptions, index: usize) -> Value {
    let k = (Rc::as_ptr(owner) as usize, index);
    OPTION_OBJECTS.with(|m| {
        m.borrow_mut()
            .entry(k)
            .or_insert_with(|| {
                let class = owner.borrow().options[index].kind.java_class();
                Value::Object(Rc::new(ServerOptionObj {
                    owner: owner.clone(),
                    index,
                    class,
                }))
            })
            .clone()
    })
}

fn new_server_options() -> LuaResult<SharedOptions> {
    Ok(Rc::new(RefCell::new(ServerOptions::new(&*defs()?))))
}

/// `ServerOptions.instance`.
fn instance() -> LuaResult<Value> {
    if let Some(v) = INSTANCE.with(|i| i.borrow().clone()) {
        return Ok(v);
    }
    let v = options_value(new_server_options()?);
    INSTANCE.with(|i| *i.borrow_mut() = Some(v.clone()));
    Ok(v)
}

fn this_options(c: &NativeCall<'_>) -> LuaResult<SharedOptions> {
    with_this::<ServerOptionsObj, _>(c, |s| s.0.clone())
}

fn register_server_options(thread: &mut Thread) {
    global(thread, "getServerOptions", |c| {
        let v = instance()?;
        Ok(c.push(v))
    });
    let b = expose_class(thread, "zombie.network.ServerOptions", None);
    b.field("MAX_PORT", 65535);
    b.ctor(|c| {
        let o = new_server_options()?;
        Ok(c.push(options_value(o)))
    });
    b.static_fn("getInstance", |c| {
        let v = instance()?;
        Ok(c.push(v))
    });
    b.method("getNumOptions", |c| {
        let n = this_options(c)?.borrow().num_options();
        Ok(c.push(n as f64))
    });
    b.method("getOptionByIndex", |c| {
        let o = this_options(c)?;
        let i = int_arg(c, 1);
        let n = o.borrow().num_options();
        if i < 0 || i as usize >= n {
            return Err(runtime(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {i} out of bounds for length {n}"),
            ));
        }
        Ok(c.push(option_value(&o, i as usize)))
    });
    b.method("getOptionByName", |c| {
        let o = this_options(c)?;
        let idx = o.borrow().index_of(&key(c, 1));
        Ok(c.push(idx.map_or(Value::Nil, |i| option_value(&o, i))))
    });
    b.method("getOption", |c| {
        let v = this_options(c)?.borrow().get_option(&key(c, 1));
        Ok(c.push(opt_str(v)))
    });
    b.method("getBoolean", |c| {
        let v = this_options(c)?.borrow().get_boolean(&key(c, 1));
        Ok(c.push(v.map_or(Value::Nil, Value::Bool)))
    });
    b.method("getFloat", |c| {
        let v = this_options(c)?.borrow().get_float(&key(c, 1));
        Ok(c.push(v.map_or(Value::Nil, |f| Value::Num(f as f64))))
    });
    b.method("getDouble", |c| {
        let v = this_options(c)?.borrow().get_double(&key(c, 1));
        Ok(c.push(v.map_or(Value::Nil, Value::Num)))
    });
    b.method("getInteger", |c| {
        let v = this_options(c)?.borrow().get_integer(&key(c, 1));
        Ok(c.push(v.map_or(Value::Nil, |i| Value::Num(i as f64))))
    });
    b.method("putOption", |c| {
        let (k, v) = (key(c, 1), key(c, 2));
        this_options(c)?.borrow_mut().put_option(&k, &v);
        Ok(0)
    });
    b.method("putSaveOption", |c| {
        let (k, v) = (key(c, 1), key(c, 2));
        let o = this_options(c)?;
        o.borrow_mut().put_option(&k, &v);
        save_text_file(&o.borrow(), DEFAULT_SERVER_NAME)?;
        Ok(0)
    });
    b.method("changeOption", |c| {
        let (k, v) = (key(c, 1), key(c, 2));
        let o = this_options(c)?;
        let folder = settings_folder();
        let r = with_translator(|t| {
            o.borrow_mut()
                .change_option(&folder, DEFAULT_SERVER_NAME, &k, &v, t)
        })
        .unwrap_or_default();
        Ok(c.push(r))
    });
    b.method("getMaxPlayers", |c| {
        let v = this_options(c)?.borrow().max_players();
        Ok(c.push(v as f64))
    });
    b.method("getPublicOptions", |c| {
        let v: Vec<Value> = this_options(c)?
            .borrow()
            .public_options()
            .iter()
            .cloned()
            .map(Value::from)
            .collect();
        Ok(c.push(crate::bind_java_util::list_value(v)))
    });
    b.method("getOptions", |c| {
        let o = this_options(c)?;
        let n = o.borrow().num_options();
        let v = (0..n).map(|i| option_value(&o, i)).collect();
        Ok(c.push(crate::bind_java_util::list_value(v)))
    });
    b.method("loadServerTextFile", |c| {
        let name = key(c, 1);
        let r = this_options(c)?
            .borrow_mut()
            .load_server_text_file(&settings_folder(), &name);
        Ok(c.push(r))
    });
    b.method("saveServerTextFile", |c| {
        let name = key(c, 1);
        let r = save_text_file(&this_options(c)?.borrow(), &name)?;
        Ok(c.push(r))
    });
    register_option_methods(thread);
}

fn save_text_file(o: &ServerOptions, name: &str) -> LuaResult<bool> {
    let folder = settings_folder();
    let _ = std::fs::create_dir_all(&folder);
    match with_translator(|t| o.save_server_text_file(&folder, name, t)) {
        Some(Ok(r)) => Ok(r),
        Some(Err(m)) => Err(runtime("java.lang.IllegalArgumentException", m)),
        None => Ok(false),
    }
}

fn with_option<R>(c: &NativeCall<'_>, f: impl FnOnce(&ServerOption) -> R) -> LuaResult<R> {
    with_this::<ServerOptionObj, _>(c, |o| f(&o.owner.borrow().options[o.index]))
}

fn with_option_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut ServerOption) -> R) -> LuaResult<R> {
    with_this::<ServerOptionObj, _>(c, |o| f(&mut o.owner.borrow_mut().options[o.index]))
}

fn register_option_methods(thread: &mut Thread) {
    for kind in [
        pz_server_settings::ServerOptionKind::Boolean,
        pz_server_settings::ServerOptionKind::String,
        pz_server_settings::ServerOptionKind::Text,
        pz_server_settings::ServerOptionKind::Integer,
        pz_server_settings::ServerOptionKind::Double,
        pz_server_settings::ServerOptionKind::Enum,
    ] {
        let b = expose_class(thread, kind.java_class(), None);
        b.method("asConfigOption", |c| Ok(c.push(c.arg(0))));
        b.method("getName", |c| {
            let v = with_option(c, |o| o.name().to_string())?;
            Ok(c.push(v))
        });
        b.method("getType", |c| {
            let v = with_option(c, ServerOption::get_type)?;
            Ok(c.push(v))
        });
        b.method("getTooltip", |c| {
            let v = with_option(c, |o| with_translator(|t| o.tooltip(t)).flatten())?;
            Ok(c.push(opt_str(v)))
        });
        b.method("getValue", |c| {
            let v = with_option(c, |o| match &o.config.kind {
                CfgKind::Boolean(b) => Value::Bool(b.value),
                CfgKind::Integer(i) => Value::Num(i.value as f64),
                CfgKind::Double(d) => Value::Num(d.value),
                CfgKind::String(s) => Value::from(s.value.clone()),
                CfgKind::Array(_) => Value::from(o.config.get_value_as_string()),
            })?;
            Ok(c.push(v))
        });
        b.method("getDefaultValue", |c| {
            let v = with_option(c, |o| match &o.config.kind {
                CfgKind::Boolean(b) => Value::Bool(b.default_value),
                CfgKind::Integer(i) => Value::Num(i.default_value as f64),
                CfgKind::Double(d) => Value::Num(d.default_value),
                CfgKind::String(s) => Value::from(s.default_value.clone()),
                CfgKind::Array(_) => Value::Nil,
            })?;
            Ok(c.push(v))
        });
        b.method("getMin", |c| {
            let v = with_option(c, |o| match &o.config.kind {
                CfgKind::Integer(i) => i.min as f64,
                CfgKind::Double(d) => d.min,
                _ => 0.0,
            })?;
            Ok(c.push(v))
        });
        b.method("getMax", |c| {
            let v = with_option(c, |o| match &o.config.kind {
                CfgKind::Integer(i) => i.max as f64,
                CfgKind::Double(d) => d.max,
                _ => 0.0,
            })?;
            Ok(c.push(v))
        });
        b.method("getNumValues", |c| {
            let v = with_option(c, |o| match &o.config.kind {
                CfgKind::Integer(i) => i.max as f64,
                _ => 0.0,
            })?;
            Ok(c.push(v))
        });
        b.method("setValue", |c| {
            let a = c.arg(1);
            with_option_mut(c, |o| match &o.config.kind {
                CfgKind::Boolean(_) => o.config.set_bool(matches!(a, Value::Bool(true))),
                CfgKind::Integer(_) => o
                    .config
                    .set_int(pz_kahlua::exec::raw_tonumber(&a).unwrap_or(0.0) as i32),
                CfgKind::Double(_) => o
                    .config
                    .set_double(pz_kahlua::exec::raw_tonumber(&a).unwrap_or(0.0)),
                CfgKind::String(_) => {
                    let s = pz_kahlua::exec::raw_tostring(&a).map(|s| s.to_string());
                    o.config.set_string(s.as_deref());
                }
                CfgKind::Array(_) => {}
            })?;
            Ok(0)
        });
        b.method("setDefault", |c| {
            let d = num_arg(c, 1);
            with_option_mut(c, |o| {
                if let CfgKind::Double(x) = &mut o.config.kind {
                    x.default_value = d;
                }
            })?;
            Ok(0)
        });
        b.method("getValueAsString", |c| {
            let v = with_option(c, |o| o.config.get_value_as_string())?;
            Ok(c.push(v))
        });
        b.method("getValueAsLuaString", |c| {
            let v = with_option(c, |o| o.config.get_value_as_lua_string())?;
            Ok(c.push(v))
        });
        b.method("getValueAsObject", |c| {
            let v = with_option(c, |o| lua_value(&o.config.get_value_as_object()))?;
            Ok(c.push(v))
        });
        b.method("setValueFromObject", |c| {
            let v = config_object(&c.arg(1));
            with_option_mut(c, |o| o.config.set_value_from_object(&v))?;
            Ok(0)
        });
        b.method("parse", |c| {
            let s = key(c, 1);
            with_option_mut(c, |o| o.config.parse(&s))?;
            Ok(0)
        });
        b.method("isValidString", |c| {
            let s = key(c, 1);
            let v = with_option(c, |o| o.config.is_valid_string(&s))?;
            Ok(c.push(v))
        });
        b.method("resetToDefault", |c| {
            with_option_mut(c, |o| o.config.reset_to_default())?;
            Ok(0)
        });
        b.method("setDefaultToCurrentValue", |c| {
            with_option_mut(c, |o| o.config.set_default_to_current_value())?;
            Ok(0)
        });
        if kind == pz_server_settings::ServerOptionKind::Enum {
            b.method("getValueTranslationByIndex", |c| {
                let i = int_arg(c, 1);
                match with_option(c, |o| {
                    with_translator(|t| o.value_translation_by_index(t, i))
                })? {
                    Some(Err(())) => Err(runtime("java.lang.ArrayIndexOutOfBoundsException", "")),
                    Some(Ok(s)) => Ok(c.push(opt_str(s))),
                    None => Ok(c.push(Value::Nil)),
                }
            });
        }
    }
}

// ---- spawn files (SpawnRegions) -------------------------------------------------------

/// Runs a spawn file that defines a global function `func` and returns that
/// function's first result (`loadRegionsFile` / `loadPointsFile`).
fn run_spawn_file(thread: &mut Thread, file: &str, func: &str) -> Option<TableRef> {
    if !Path::new(file).exists() {
        return None;
    }
    let loader = ENV.with(|e| e.borrow().as_ref().map(|e| e.loader.clone()))?;
    let globals = thread.globals().clone();
    globals.set_str(func, Value::Nil);
    loader.borrow_mut().forget(file);
    run_lua(thread, &loader, file);
    let f = globals.get_str(func);
    if !matches!(f, Value::Closure(_)) {
        return None;
    }
    match thread.call(&f, &[]) {
        Ok(r) => match r.into_iter().next() {
            Some(Value::Table(t)) => Some(t),
            _ => None,
        },
        Err(e) => {
            eprintln!("ERROR: {e}");
            None
        }
    }
}

fn load_regions_file(thread: &mut Thread, file: &str) -> Option<Vec<Region>> {
    run_spawn_file(thread, file, "SpawnRegions").map(|t| spawn_regions::parse_regions(&t))
}

fn load_points_file(thread: &mut Thread, file: &str) -> Option<Vec<Profession>> {
    run_spawn_file(thread, file, "SpawnPoints").and_then(|t| spawn_regions::parse_professions(&t))
}

fn write_file(file: &str, text: String) -> bool {
    eprintln!("LOG  : writing {file}");
    std::fs::write(file, text).is_ok()
}

fn default_regions() -> Vec<Region> {
    let maps = ENV
        .with(|e| {
            e.borrow()
                .as_ref()
                .map(|e| e.fs.borrow().media_file("maps"))
        })
        .unwrap_or_default();
    spawn_regions::default_server_regions(&maps)
}

// ---- ServerSettings ---------------------------------------------------------------------

#[derive(Default)]
struct SettingsState {
    name: String,
    server: Option<SharedOptions>,
    sandbox: Option<SharedSandbox>,
    regions: Option<Vec<Region>>,
    points: Option<Vec<Profession>>,
    valid: bool,
    error_msg: Option<String>,
}

/// `zombie.network.ServerSettings`.
struct SettingsObj(RefCell<SettingsState>);

impl LuaObject for SettingsObj {
    fn class_name(&self) -> &str {
        "zombie.network.ServerSettings"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `new ServerSettings(name)`. The profanity filter is not ported yet, so
/// every name passes its check.
fn new_settings(name: &str) -> LuaResult<SettingsState> {
    let reserved = reserved()?;
    let s = pz_server_settings::ServerSettings::new(name, &reserved, None, |e| {
        with_translator(|t| t.get_text("UI_BadWordCheck", &[pz_text::JavaArg::Str(e.to_string())]))
            .unwrap_or_default()
    });
    let fs_ok = ENV
        .with(|e| {
            e.borrow()
                .as_ref()
                .map(|e| e.fs.borrow().validate_prefix(&s.name))
        })
        .unwrap_or(true);
    if !fs_ok {
        return Err(runtime(
            "java.lang.IllegalArgumentException",
            format!("Invalid prefix found for: {}", s.name),
        ));
    }
    Ok(SettingsState {
        name: s.name,
        valid: s.valid,
        error_msg: s.error_msg,
        ..Default::default()
    })
}

fn settings_value(st: SettingsState) -> Value {
    Value::Object(Rc::new(SettingsObj(RefCell::new(st))))
}

fn with_settings<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut SettingsState) -> R) -> LuaResult<R> {
    with_this::<SettingsObj, _>(c, |s| f(&mut s.0.borrow_mut()))
}

fn in_folder(name: &str) -> String {
    manager().name_in_settings_folder(name)
}

/// `readLuaFile`: run `<server>_SandboxVars.lua` with `SandboxVars` cleared,
/// upgrade its table by `VERSION` and apply it; the old `SandboxVars` is put
/// back afterwards.
fn read_sandbox_lua_file(thread: &mut Thread, opts: &SharedSandbox, file: &str) -> bool {
    if !Path::new(file).exists() {
        return false;
    }
    let Some(loader) = ENV.with(|e| e.borrow().as_ref().map(|e| e.loader.clone())) else {
        return false;
    };
    let globals = thread.globals().clone();
    let old = globals.get_str("SandboxVars");
    globals.set_str("SandboxVars", Value::Nil);
    loader.borrow_mut().forget(file);
    run_lua(thread, &loader, file);
    let result = match globals.get_str("SandboxVars") {
        Value::Nil => false,
        Value::Table(t) => {
            let mut version = 0;
            match t.rawget(&Value::str("VERSION")) {
                Value::Nil => {}
                Value::Num(d) => {
                    version = d as i32;
                    t.rawset(Value::str("VERSION"), Value::Nil);
                }
                other => {
                    eprintln!(
                        "ERROR: VERSION=\"{}\" in {file}",
                        pz_kahlua::exec::java_value_of(&other)
                    );
                    t.rawset(Value::str("VERSION"), Value::Nil);
                }
            }
            let upgraded = opts.borrow_mut().upgrade_lua_table("", &t, version);
            match upgraded {
                Ok(u) => {
                    opts.borrow_mut().apply_table(&u);
                    true
                }
                Err(e) => {
                    eprintln!("ERROR: {e}");
                    false
                }
            }
        }
        _ => true,
    };
    if matches!(old, Value::Table(_)) {
        globals.set_str("SandboxVars", old);
    }
    result
}

/// `loadFiles()`.
fn load_files(thread: &mut Thread, st: &mut SettingsState) -> LuaResult<()> {
    let folder = settings_folder();
    let server = new_server_options()?;
    server.borrow_mut().load_server_text_file(&folder, &st.name);
    st.server = Some(server);
    let sandbox = new_sandbox(thread)?;
    read_sandbox_lua_file(
        thread,
        &sandbox,
        &in_folder(&format!("{}_SandboxVars.lua", st.name)),
    );
    sandbox
        .borrow_mut()
        .load_server_zombies_file(Path::new(&in_folder(&format!("{}_zombies.ini", st.name))));
    st.sandbox = Some(sandbox);
    st.regions = load_regions_file(thread, &in_folder(&format!("{}_spawnregions.lua", st.name)))
        .or_else(|| Some(default_regions()));
    st.points = load_points_file(thread, &in_folder(&format!("{}_spawnpoints.lua", st.name)));
    Ok(())
}

/// `saveFiles()`.
fn save_files(st: &SettingsState) -> LuaResult<bool> {
    let Some(server) = &st.server else {
        return Ok(false);
    };
    let _ = std::fs::create_dir_all(settings_folder());
    save_text_file(&server.borrow(), &st.name)?;
    if let Some(sandbox) = &st.sandbox {
        let file = in_folder(&format!("{}_SandboxVars.lua", st.name));
        if file.contains("..") {
            return Err(runtime(
                "java.lang.IllegalArgumentException",
                format!("Unable to save options to filename: {file}"),
            ));
        }
        let text =
            with_translator(|t| sandbox.borrow().lua_file_text(false, t)).unwrap_or_default();
        write_file(&file, text);
    }
    if let Some(r) = &st.regions {
        write_file(
            &in_folder(&format!("{}_spawnregions.lua", st.name)),
            spawn_regions::regions_file_text(r),
        );
    }
    if let Some(p) = &st.points {
        write_file(
            &in_folder(&format!("{}_spawnpoints.lua", st.name)),
            spawn_regions::points_file_text(p),
        );
    }
    let z = in_folder(&format!("{}_zombies.ini", st.name));
    if Path::new(&z).exists() {
        let _ = std::fs::remove_file(&z);
    }
    Ok(true)
}

fn delete_files(name: &str) {
    for s in SUFFIXES {
        let p = in_folder(&format!("{name}{s}"));
        if Path::new(&p).exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn retarget(st: &mut SettingsState, new_name: &str) {
    let old = format!("{}_spawnpoints.lua", st.name);
    for r in st.regions.iter_mut().flatten() {
        if r.serverfile.as_deref() == Some(old.as_str()) {
            r.serverfile = Some(format!("{new_name}_spawnpoints.lua"));
        }
    }
}

fn register_settings(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.network.ServerSettings", None);
    b.ctor(|c| {
        let st = new_settings(&key(c, 0))?;
        Ok(c.push(settings_value(st)))
    });
    b.method("getName", |c| {
        let v = with_settings(c, |s| s.name.clone())?;
        Ok(c.push(v))
    });
    b.method("isValid", |c| {
        let v = with_settings(c, |s| s.valid)?;
        Ok(c.push(v))
    });
    b.method("getErrorMsg", |c| {
        let v = with_settings(c, |s| s.error_msg.clone())?;
        Ok(c.push(opt_str(v)))
    });
    b.method("resetToDefault", |c| {
        let server = new_server_options()?;
        let sandbox = new_sandbox(c.thread)?;
        let regions = default_regions();
        with_settings(c, |s| {
            s.server = Some(server);
            s.sandbox = Some(sandbox);
            s.regions = Some(regions);
            s.points = None;
        })?;
        Ok(0)
    });
    b.method("loadFiles", |c| {
        let this = crate::expose::this::<SettingsObj>(c)?;
        let obj = this.as_any().downcast_ref::<SettingsObj>().unwrap();
        let mut st = std::mem::take(&mut *obj.0.borrow_mut());
        let r = load_files(c.thread, &mut st);
        *obj.0.borrow_mut() = st;
        r?;
        Ok(c.push(true))
    });
    b.method("saveFiles", |c| {
        let r = with_settings(c, |s| save_files(s))??;
        Ok(c.push(r))
    });
    b.method("deleteFiles", |c| {
        let name = with_settings(c, |s| s.name.clone())?;
        delete_files(&name);
        Ok(c.push(true))
    });
    b.method("duplicateFiles", |c| {
        let new_name = key(c, 1);
        if !manager().is_valid_new_name(&new_name, &reserved()?) {
            return Ok(c.push(false));
        }
        let name = with_settings(c, |s| s.name.clone())?;
        let mut copy = new_settings(&name)?;
        load_files(c.thread, &mut copy)?;
        retarget(&mut copy, &new_name);
        copy.name = file_name_sanitize(&new_name, &reserved()?);
        save_files(&copy)?;
        Ok(c.push(true))
    });
    b.method("rename", |c| {
        let new_name = key(c, 1);
        if !manager().is_valid_new_name(&new_name, &reserved()?) {
            return Ok(c.push(false));
        }
        let this = crate::expose::this::<SettingsObj>(c)?;
        let obj = this.as_any().downcast_ref::<SettingsObj>().unwrap();
        let mut st = std::mem::take(&mut *obj.0.borrow_mut());
        let r = (|| -> LuaResult<()> {
            load_files(c.thread, &mut st)?;
            delete_files(&st.name);
            retarget(&mut st, &new_name);
            st.name = file_name_sanitize(&new_name, &reserved()?);
            save_files(&st)?;
            Ok(())
        })();
        *obj.0.borrow_mut() = st;
        r?;
        Ok(c.push(true))
    });
    b.method("getServerOptions", |c| {
        let v = with_settings(c, |s| s.server.clone())?;
        Ok(c.push(v.map_or(Value::Nil, options_value)))
    });
    b.method("getSandboxOptions", |c| {
        let v = with_settings(c, |s| s.sandbox.clone())?;
        Ok(c.push(v.map_or(Value::Nil, |o| Value::Object(Rc::new(SandboxObj(o))))))
    });
    b.method("getNumSpawnRegions", |c| {
        let v = with_settings(c, |s| s.regions.as_ref().map(Vec::len))?;
        let n = v.ok_or_else(|| runtime("java.lang.NullPointerException", "spawnRegions"))?;
        Ok(c.push(n as f64))
    });
    b.method("getSpawnRegionName", |c| {
        let i = int_arg(c, 1);
        let v = with_settings(c, |s| {
            s.regions
                .as_ref()
                .and_then(|r| usize::try_from(i).ok().and_then(|i| r.get(i)))
                .map(|r| r.name.clone())
        })?;
        let v = v.ok_or_else(|| {
            runtime(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {i} out of bounds"),
            )
        })?;
        Ok(c.push(v))
    });
    b.method("getSpawnRegionFile", |c| {
        let i = int_arg(c, 1);
        let v = with_settings(c, |s| {
            s.regions
                .as_ref()
                .and_then(|r| usize::try_from(i).ok().and_then(|i| r.get(i)))
                .map(|r| r.file.clone().or_else(|| r.serverfile.clone()))
        })?;
        let v = v.ok_or_else(|| {
            runtime(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {i} out of bounds"),
            )
        })?;
        Ok(c.push(opt_str(v)))
    });
    b.method("clearSpawnRegions", |c| {
        with_settings(c, |s| s.regions.as_mut().map(Vec::clear))?
            .ok_or_else(|| runtime("java.lang.NullPointerException", "spawnRegions"))?;
        Ok(0)
    });
    b.method("addSpawnRegion", |c| {
        let (name, file) = (str_arg(c, 1), str_arg(c, 2));
        let (Some(name), Some(file)) = (name, file) else {
            return Err(runtime("java.lang.NullPointerException", "null"));
        };
        let r = if file.starts_with("media") {
            Region {
                name: name.to_string(),
                file: Some(file.to_string()),
                ..Default::default()
            }
        } else {
            Region {
                name: name.to_string(),
                serverfile: Some(file.to_string()),
                ..Default::default()
            }
        };
        with_settings(c, |s| s.regions.as_mut().map(|v| v.push(r)))?
            .ok_or_else(|| runtime("java.lang.NullPointerException", "spawnRegions"))?;
        Ok(0)
    });
    b.method("removeSpawnRegion", |c| {
        let i = int_arg(c, 1);
        let ok = with_settings(c, |s| {
            s.regions.as_mut().is_some_and(|v| {
                let ok = i >= 0 && (i as usize) < v.len();
                if ok {
                    v.remove(i as usize);
                }
                ok
            })
        })?;
        if !ok {
            return Err(runtime(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {i} out of bounds"),
            ));
        }
        Ok(0)
    });
    b.method("loadSpawnPointsFile", |c| {
        let file = in_folder(&key(c, 1));
        let r = load_points_file(c.thread, &file).map(|p| spawn_regions::points_table(&p));
        Ok(c.push(r.map_or(Value::Nil, Value::Table)))
    });
    b.method("saveSpawnPointsFile", |c| {
        let file = in_folder(&key(c, 1));
        let r = match c.arg(2) {
            Value::Table(t) => spawn_regions::parse_professions(&t)
                .is_some_and(|p| write_file(&file, spawn_regions::points_file_text(&p))),
            _ => false,
        };
        Ok(c.push(r))
    });
}

// ---- ServerSettingsManager ------------------------------------------------------------------

struct ManagerObj(RefCell<Vec<Value>>);

impl LuaObject for ManagerObj {
    fn class_name(&self) -> &str {
        "zombie.network.ServerSettingsManager"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn manager_value() -> Value {
    MANAGER.with(|m| {
        m.borrow_mut()
            .get_or_insert_with(|| Value::Object(Rc::new(ManagerObj(RefCell::new(Vec::new())))))
            .clone()
    })
}

fn register_manager(thread: &mut Thread) {
    global(thread, "getServerSettingsManager", |c| {
        Ok(c.push(manager_value()))
    });
    let b = expose_class(thread, "zombie.network.ServerSettingsManager", None);
    b.static_fn("getInstance", |c| Ok(c.push(manager_value())));
    b.method("getSettingsFolder", |c| {
        Ok(c.push(manager().settings_folder()))
    });
    b.method("getNameInSettingsFolder", |c| {
        let n = key(c, 1);
        Ok(c.push(in_folder(&n)))
    });
    b.method("settingsFolderPathValidLength", |c| {
        let n = key(c, 1);
        Ok(c.push(manager().settings_folder_path_valid_length(&n)))
    });
    b.method("settingsFolderNameValidLength", |c| {
        let n = key(c, 1);
        Ok(c.push(manager().settings_folder_name_valid_length(&n)))
    });
    b.method("settingsFolderValidLengthChecks", |c| {
        let n = key(c, 1);
        Ok(c.push(manager().settings_folder_valid_length_checks(&n)))
    });
    b.method("isValidName", |c| {
        let r = ServerSettingsManager::is_valid_name(str_arg(c, 1).as_deref());
        Ok(c.push(r))
    });
    b.method("isValidNewName", |c| {
        let n = key(c, 1);
        let r = manager().is_valid_new_name(&n, &reserved()?);
        Ok(c.push(r))
    });
    b.method("getSuffixes", |c| {
        let v = SUFFIXES.iter().map(|s| Value::from(*s)).collect();
        Ok(c.push(crate::bind_java_util::list_value(v)))
    });
    b.method("readAllSettings", |c| {
        let mut m = manager();
        m.read_all_settings();
        let mut list = Vec::new();
        for name in &m.settings {
            list.push(settings_value(new_settings(name)?));
        }
        with_this::<ManagerObj, _>(c, |o| *o.0.borrow_mut() = list)?;
        Ok(0)
    });
    b.method("getSettingsCount", |c| {
        let n = with_this::<ManagerObj, _>(c, |o| o.0.borrow().len())?;
        Ok(c.push(n as f64))
    });
    b.method("getSettingsByIndex", |c| {
        let i = int_arg(c, 1);
        let v = with_this::<ManagerObj, _>(c, |o| {
            usize::try_from(i)
                .ok()
                .and_then(|i| o.0.borrow().get(i).cloned())
        })?;
        Ok(c.push(v.unwrap_or_default()))
    });
}

pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>, loader: Rc<RefCell<Loader>>) {
    ENV.with(|e| {
        *e.borrow_mut() = Some(Env {
            fs,
            loader,
            defs: None,
            reserved: None,
        })
    });
    INSTANCE.with(|i| *i.borrow_mut() = None);
    MANAGER.with(|m| *m.borrow_mut() = None);
    register_server_options(thread);
    register_settings(thread);
    register_manager(thread);
}
