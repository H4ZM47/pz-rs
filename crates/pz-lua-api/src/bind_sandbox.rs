//! Lua bindings for `zombie.SandboxOptions` and its option classes.
//!
//! `SandboxOptions.instance` is created on first use, as the original's
//! static initializer is: the option list is read from the installed jar,
//! `media/lua/shared/defines.lua` runs, the "Apocalypse" preset is applied
//! and becomes the defaults.

use crate::bind_text::with_translator;
use crate::expose::{expose_class, global, int_arg, num_arg, str_arg, with_this};
use crate::lua_manager::{run_lua, Loader};
use pz_core::config::OptionKind as CfgKind;
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_sandbox::{config_object, lua_value, SandboxOption, SandboxOptions};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

const CLASS: &str = "zombie.SandboxOptions";

type Shared = Rc<RefCell<SandboxOptions>>;

struct Env {
    fs: Rc<RefCell<GameFs>>,
    loader: Rc<RefCell<Loader>>,
    defs: Option<Vec<pz_sandbox::OptionDef>>,
}

thread_local! {
    static ENV: RefCell<Option<Env>> = const { RefCell::new(None) };
    static INSTANCE: RefCell<Option<Value>> = const { RefCell::new(None) };
    static OPTION_OBJECTS: RefCell<HashMap<(usize, usize), Value>> = RefCell::new(HashMap::new());
}

/// `zombie.SandboxOptions` object.
pub struct SandboxObj(pub Shared);

impl LuaObject for SandboxObj {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One option of a `SandboxOptions` (by index).
struct OptionObj {
    owner: Shared,
    index: usize,
    class: &'static str,
}

impl LuaObject for OptionObj {
    fn class_name(&self) -> &str {
        self.class
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        match self.class {
            "zombie.SandboxOptions$EnumSandboxOption" => &[
                "zombie.config.EnumConfigOption",
                "zombie.config.IntegerConfigOption",
                "zombie.config.ConfigOption",
            ],
            "zombie.SandboxOptions$IntegerSandboxOption" => &[
                "zombie.config.IntegerConfigOption",
                "zombie.config.ConfigOption",
            ],
            "zombie.SandboxOptions$DoubleSandboxOption" => &[
                "zombie.config.DoubleConfigOption",
                "zombie.config.ConfigOption",
            ],
            "zombie.SandboxOptions$StringSandboxOption" => &[
                "zombie.config.StringConfigOption",
                "zombie.config.ConfigOption",
            ],
            _ => &[
                "zombie.config.BooleanConfigOption",
                "zombie.config.ConfigOption",
            ],
        }
    }
}

fn option_class(o: &SandboxOption) -> &'static str {
    if o.enum_constants.is_some() {
        "zombie.SandboxOptions$StrongEnumSandboxOption"
    } else if o.is_enum {
        "zombie.SandboxOptions$EnumSandboxOption"
    } else {
        match o.config.kind {
            CfgKind::Integer(_) => "zombie.SandboxOptions$IntegerSandboxOption",
            CfgKind::Double(_) => "zombie.SandboxOptions$DoubleSandboxOption",
            CfgKind::String(_) => "zombie.SandboxOptions$StringSandboxOption",
            _ => "zombie.SandboxOptions$BooleanSandboxOption",
        }
    }
}

fn option_value(owner: &Shared, index: usize) -> Value {
    let key = (Rc::as_ptr(owner) as usize, index);
    OPTION_OBJECTS.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let class = option_class(&owner.borrow().options[index]);
                Value::Object(Rc::new(OptionObj {
                    owner: owner.clone(),
                    index,
                    class,
                }))
            })
            .clone()
    })
}

fn runtime(class: &'static str, message: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

fn media_file(fs: &GameFs, sub: &str) -> PathBuf {
    fs.media_file(sub)
}

/// `new SandboxOptions()`.
fn construct(thread: &mut Thread) -> LuaResult<Shared> {
    let (fs, loader, defs) = ENV.with(|e| -> LuaResult<_> {
        let mut e = e.borrow_mut();
        let env = e.as_mut().ok_or_else(|| {
            runtime(
                "java.lang.IllegalStateException",
                "SandboxOptions used before init",
            )
        })?;
        if env.defs.is_none() {
            let jar_path = env.fs.borrow().base.join("projectzomboid.jar");
            let jar = pz_classfile::Jar::open(&jar_path).map_err(|err| {
                runtime(
                    "java.lang.RuntimeException",
                    format!("{}: {err}", jar_path.display()),
                )
            })?;
            let defs = pz_sandbox::defs::builtin_options(&jar).map_err(|err| {
                runtime(
                    "java.lang.RuntimeException",
                    format!("SandboxOptions: {err}"),
                )
            })?;
            env.defs = Some(defs);
        }
        Ok((
            env.fs.clone(),
            env.loader.clone(),
            env.defs.clone().unwrap(),
        ))
    })?;
    let opts = Rc::new(RefCell::new(SandboxOptions::new(&defs)));
    // CustomSandboxOptions.instance.initInstance(this): mod options are added
    // by the mod-loading port.
    let defines = media_file(&fs.borrow(), "lua/shared/defines.lua");
    run_lua(thread, &loader, &defines.to_string_lossy());
    load_game_file(thread, &fs, &loader, &opts, "Apocalypse");
    opts.borrow_mut().set_defaults_to_current_values();
    Ok(opts)
}

/// `loadGameFile(presetName)`: runs `media/lua/shared/Sandbox/<name>.lua`
/// and applies the table it returns.
fn load_game_file(
    thread: &mut Thread,
    fs: &Rc<RefCell<GameFs>>,
    loader: &Rc<RefCell<Loader>>,
    opts: &Shared,
    preset: &str,
) -> bool {
    let file = media_file(&fs.borrow(), &format!("lua/shared/Sandbox/{preset}.lua"));
    if !file.exists() {
        return true;
    }
    let path = file.to_string_lossy().into_owned();
    loader.borrow_mut().forget(&path);
    match run_lua(thread, loader, &path) {
        Value::Table(t) => {
            opts.borrow_mut().apply_table(&t);
            true
        }
        _ => {
            eprintln!(
                "ERROR: {} must return a SandboxVars table",
                file.file_name().unwrap_or_default().to_string_lossy()
            );
            false
        }
    }
}

/// `SandboxOptions.instance`.
pub fn instance(thread: &mut Thread) -> LuaResult<Value> {
    if let Some(v) = INSTANCE.with(|i| i.borrow().clone()) {
        return Ok(v);
    }
    let v = Value::Object(Rc::new(SandboxObj(construct(thread)?)));
    INSTANCE.with(|i| *i.borrow_mut() = Some(v.clone()));
    Ok(v)
}

fn this_opts(c: &NativeCall<'_>) -> LuaResult<Shared> {
    with_this::<SandboxObj, _>(c, |s| s.0.clone())
}

fn sandbox_vars(c: &NativeCall<'_>) -> Option<TableRef> {
    match c.thread.globals().get_str("SandboxVars") {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

fn cache_dir() -> Option<PathBuf> {
    ENV.with(|e| {
        e.borrow()
            .as_ref()
            .map(|e| e.fs.borrow().cache_dir().to_path_buf())
    })
}

fn presets_dir() -> Option<PathBuf> {
    let dir = pz_sandbox::sandbox_cache_dir(&cache_dir()?);
    let _ = std::fs::create_dir(&dir);
    Some(dir)
}

pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>, loader: Rc<RefCell<Loader>>) {
    ENV.with(|e| {
        *e.borrow_mut() = Some(Env {
            fs,
            loader,
            defs: None,
        })
    });
    INSTANCE.with(|i| *i.borrow_mut() = None);

    global(thread, "getSandboxOptions", |c| {
        let v = instance(c.thread)?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, CLASS, None);
    b.field("FIRST_YEAR", pz_sandbox::FIRST_YEAR);
    b.ctor(|c| {
        let o = construct(c.thread)?;
        Ok(c.push(Value::Object(Rc::new(SandboxObj(o)))))
    });
    b.static_fn("getInstance", |c| {
        let v = instance(c.thread)?;
        Ok(c.push(v))
    });
    b.static_fn("isValidPresetName", |c| {
        let r = SandboxOptions::is_valid_preset_name(str_arg(c, 0).as_deref());
        Ok(c.push(r))
    });
    b.static_fn("Reset", |_| {
        if let Some(Value::Object(o)) = INSTANCE.with(|i| i.borrow().clone()) {
            if let Some(s) = o.as_any().downcast_ref::<SandboxObj>() {
                s.0.borrow_mut().remove_custom_options();
            }
        }
        Ok(0)
    });

    b.method("getNumOptions", |c| {
        let n = this_opts(c)?.borrow().num_options();
        Ok(c.push(n as f64))
    });
    b.method("getOptionByIndex", |c| {
        let o = this_opts(c)?;
        let i = int_arg(c, 1);
        if i < 0 || i as usize >= o.borrow().num_options() {
            return Err(runtime(
                "java.lang.IndexOutOfBoundsException",
                format!(
                    "Index {i} out of bounds for length {}",
                    o.borrow().num_options()
                ),
            ));
        }
        Ok(c.push(option_value(&o, i as usize)))
    });
    b.method("getOptionByName", |c| {
        let o = this_opts(c)?;
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let idx = o.borrow().index_of(&name);
        Ok(c.push(idx.map_or(Value::Nil, |i| option_value(&o, i))))
    });
    b.method("set", |c| {
        let o = this_opts(c)?;
        let name = str_arg(c, 1).map(|s| s.to_string());
        let value = config_object(&c.arg(2));
        o.borrow_mut()
            .set(name.as_deref(), &value)
            .map_err(|m| runtime("java.lang.IllegalArgumentException", m))?;
        Ok(0)
    });
    b.method("copyValuesFrom", |c| {
        let o = this_opts(c)?;
        let other = match c.arg(1) {
            Value::Object(x) => x.as_any().downcast_ref::<SandboxObj>().map(|s| s.0.clone()),
            _ => None,
        }
        .ok_or_else(|| runtime("java.lang.NullPointerException", "null"))?;
        if Rc::ptr_eq(&o, &other) {
            return Ok(0);
        }
        let src = other.borrow().clone();
        o.borrow_mut().copy_values_from(&src);
        Ok(0)
    });
    b.method("newCopy", |c| {
        let o = this_opts(c)?;
        let copy = construct(c.thread)?;
        let src = o.borrow().clone();
        copy.borrow_mut().copy_values_from(&src);
        Ok(c.push(Value::Object(Rc::new(SandboxObj(copy)))))
    });
    b.method("resetToDefault", |c| {
        this_opts(c)?.borrow_mut().reset_to_default();
        Ok(0)
    });
    b.method("setDefaultsToCurrentValues", |c| {
        this_opts(c)?.borrow_mut().set_defaults_to_current_values();
        Ok(0)
    });
    b.method("initSandboxVars", |c| {
        let o = this_opts(c)?;
        let vars = sandbox_vars(c)
            .ok_or_else(|| runtime("java.lang.NullPointerException", "SandboxVars"))?;
        o.borrow_mut().init_sandbox_vars(&vars);
        Ok(0)
    });
    b.method("toLua", |c| {
        let o = this_opts(c)?;
        let vars = sandbox_vars(c)
            .ok_or_else(|| runtime("java.lang.NullPointerException", "SandboxVars"))?;
        o.borrow().to_lua(&vars);
        Ok(0)
    });
    b.method("updateFromLua", |c| {
        let o = this_opts(c)?;
        let vars = sandbox_vars(c)
            .ok_or_else(|| runtime("java.lang.NullPointerException", "SandboxVars"))?;
        o.borrow_mut().update_from_lua(&vars);
        Ok(0)
    });
    b.method("loadGameFile", |c| {
        let o = this_opts(c)?;
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let (fs, loader) = ENV.with(|e| {
            let e = e.borrow();
            let e = e.as_ref().unwrap();
            (e.fs.clone(), e.loader.clone())
        });
        let r = load_game_file(c.thread, &fs, &loader, &o, &name);
        Ok(c.push(r))
    });
    // Writing media/lua/shared/Sandbox/<name>.lua is a debug-only tool.
    b.method("saveGameFile", |c| Ok(c.push(false)));
    b.method("loadPresetFile", |c| {
        let o = this_opts(c)?;
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let r = presets_dir().is_some_and(|d| o.borrow_mut().load_preset_file(&d, &name));
        Ok(c.push(r))
    });
    b.method("savePresetFile", |c| {
        let o = this_opts(c)?;
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let r = presets_dir().is_some_and(|d| {
            with_translator(|t| o.borrow().save_preset_file(&d, &name, t)).unwrap_or(false)
        });
        Ok(c.push(r))
    });
    // Not a client: nothing to send.
    b.method("sendToServer", |_| Ok(0));
    // GameTime is set from these by the game-time port.
    b.method("applySettings", |_| Ok(0));
    b.method("isUnstableScriptNameSpam", |c| Ok(c.push(false)));

    macro_rules! int_getter {
        ($name:literal, $f:ident) => {
            b.method($name, |c| {
                let v = this_opts(c)?.borrow().$f();
                Ok(c.push(v as f64))
            });
        };
    }
    int_getter!("getFirstYear", first_year);
    int_getter!("getTemperatureModifier", temperature_modifier);
    int_getter!("getRainModifier", rain_modifier);
    int_getter!("getErosionSpeed", erosion_speed);
    int_getter!("getWaterShutModifier", water_shut_modifier);
    int_getter!("getElecShutModifier", elec_shut_modifier);
    int_getter!("getTimeSinceApo", time_since_apo);
    int_getter!("getDayLengthMinutes", day_length_minutes);
    int_getter!("getDayLengthMinutesDefault", day_length_minutes_default);
    int_getter!("getCompostHours", compost_hours);
    int_getter!("getEnduranceRegenMultiplier", endurance_regen_multiplier);
    int_getter!("getStatsDecreaseMultiplier", stats_decrease_multiplier);
    b.method("getAllClothesUnlocked", |c| {
        let v = this_opts(c)?.borrow().all_clothes_unlocked();
        Ok(c.push(v))
    });
    for (name, f) in [
        (
            "randomWaterShut",
            SandboxOptions::random_water_shut as fn(&SandboxOptions, i32) -> i32,
        ),
        (
            "randomElectricityShut",
            SandboxOptions::random_electricity_shut,
        ),
        ("randomAlarmDecay", SandboxOptions::random_alarm_decay),
    ] {
        b.method(name, move |c| {
            let m = int_arg(c, 1);
            let v = f(&this_opts(c)?.borrow(), m);
            Ok(c.push(v as f64))
        });
    }
    b.method("lootItemRemovalListContains", |c| {
        let s = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let v = this_opts(c)?.borrow().loot_item_removal_list_contains(&s);
        Ok(c.push(v))
    });
    b.method("worldItemRemovalListContains", |c| {
        let s = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let v = this_opts(c)?.borrow().world_item_removal_list_contains(&s);
        Ok(c.push(v))
    });

    register_option_methods(thread);
}

fn with_option<R>(c: &NativeCall<'_>, f: impl FnOnce(&SandboxOption) -> R) -> LuaResult<R> {
    with_this::<OptionObj, _>(c, |o| f(&o.owner.borrow().options[o.index]))
}

fn with_option_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut SandboxOption) -> R) -> LuaResult<R> {
    with_this::<OptionObj, _>(c, |o| f(&mut o.owner.borrow_mut().options[o.index]))
}

fn opt_str(v: Option<String>) -> Value {
    v.map_or(Value::Nil, Value::from)
}

fn register_option_methods(thread: &mut Thread) {
    for class in [
        "zombie.SandboxOptions$BooleanSandboxOption",
        "zombie.SandboxOptions$IntegerSandboxOption",
        "zombie.SandboxOptions$DoubleSandboxOption",
        "zombie.SandboxOptions$StringSandboxOption",
        "zombie.SandboxOptions$EnumSandboxOption",
        "zombie.SandboxOptions$StrongEnumSandboxOption",
    ] {
        let b = expose_class(thread, class, None);
        b.method("asConfigOption", |c| Ok(c.push(c.arg(0))));
        b.method("getName", |c| {
            let v = with_option(c, |o| o.name().to_string())?;
            Ok(c.push(v))
        });
        b.method("getType", |c| {
            let v = with_option(c, SandboxOption::get_type)?;
            Ok(c.push(v))
        });
        b.method("getShortName", |c| {
            let v = with_option(c, |o| o.short_name.clone())?;
            Ok(c.push(v))
        });
        b.method("getTableName", |c| {
            let v = with_option(c, |o| o.table_name.clone())?;
            Ok(c.push(opt_str(v)))
        });
        b.method("getPageName", |c| {
            let v = with_option(c, |o| o.page_name.clone())?;
            Ok(c.push(opt_str(v)))
        });
        b.method("setPageName", |c| {
            let p = str_arg(c, 1).map(|s| s.to_string());
            with_option_mut(c, |o| o.page_name = p)?;
            Ok(c.push(c.arg(0)))
        });
        b.method("isCustom", |c| {
            let v = with_option(c, |o| o.custom)?;
            Ok(c.push(v))
        });
        b.method("setCustom", |c| {
            with_option_mut(c, |o| o.custom = true)?;
            Ok(0)
        });
        b.method("setTranslation", |c| {
            let t = str_arg(c, 1).map(|s| s.to_string());
            with_option_mut(c, |o| o.translation = t)?;
            Ok(c.push(c.arg(0)))
        });
        b.method("getTranslatedName", |c| {
            let v = with_option(c, |o| with_translator(|t| o.translated_name(t)))?;
            Ok(c.push(opt_str(v)))
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
            let v = with_option(c, SandboxOption::num_values)?;
            Ok(c.push(v as f64))
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
            let s = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
            with_option_mut(c, |o| o.config.parse(&s))?;
            Ok(0)
        });
        b.method("isValidString", |c| {
            let s = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
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
        b.method("fromTable", |c| {
            if let Value::Table(t) = c.arg(1) {
                with_option_mut(c, |o| o.from_table(&t))?;
            }
            Ok(0)
        });
        b.method("toTable", |c| {
            if let Value::Table(t) = c.arg(1) {
                with_option(c, |o| o.to_table(&t))?;
            }
            Ok(0)
        });
        b.method("getSplitCSVList", |c| {
            let v = with_option(c, SandboxOption::split_csv_list)?;
            Ok(c.push(crate::bind_java_util::list_value(
                v.into_iter().map(Value::from).collect(),
            )))
        });
        b.method("getValueTranslation", |c| {
            let v = with_option(c, |o| o.value_translation().to_string())?;
            Ok(c.push(v))
        });
        b.method("setValueTranslation", |c| {
            let t = str_arg(c, 1).map(|s| s.to_string());
            with_option_mut(c, |o| o.value_translation = t)?;
            Ok(c.push(c.arg(0)))
        });
        b.method("getValueTranslationByIndex", |c| {
            let i = int_arg(c, 1);
            let v = with_option(c, |o| {
                with_translator(|t| o.value_translation_by_index(t, i))
            })?;
            match v {
                Some(Err(_)) => Err(runtime("java.lang.ArrayIndexOutOfBoundsException", "")),
                Some(Ok(s)) => Ok(c.push(s)),
                None => Ok(c.push(Value::Nil)),
            }
        });
        b.method("getValueTranslationByIndexOrNull", |c| {
            let i = int_arg(c, 1);
            let v = with_option(c, |o| {
                with_translator(|t| o.value_translation_by_index_or_null(t, i))
            })?;
            match v {
                Some(Err(_)) => Err(runtime("java.lang.ArrayIndexOutOfBoundsException", "")),
                Some(Ok(s)) => Ok(c.push(opt_str(s))),
                None => Ok(c.push(Value::Nil)),
            }
        });
        if class.ends_with("StrongEnumSandboxOption") {
            b.method("getEnumValue", |c| {
                let v = with_option(c, |o| o.enum_value().map(str::to_string))?;
                Ok(c.push(opt_str(v)))
            });
        }
    }
}
