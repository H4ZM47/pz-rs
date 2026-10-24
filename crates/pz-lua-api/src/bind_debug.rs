//! `zombie.debug.DebugType`, `zombie.debug.LogSeverity`, the
//! `zombie.debug.DebugLog` statics, `zombie.debug.DebugOptions` /
//! `BooleanDebugOption`, and the `LuaManager.GlobalObject` globals that
//! touch them (`log`, `writeLog`, `getDebugOptions`), over
//! [`pz_core::debug`]'s thread-local `DebugLog`.
//!
//! Overloads resolve like Kahlua's `MultiLuaJavaInvoker` for the argument
//! shapes Lua can produce: `X:warn(v)` is the `warn(Object)` overload
//! (`String.valueOf(v)`, never formatted), `X:warn(fmt, ...)` formats with
//! the remaining Lua values passed unconverted (numbers are `Double`, so
//! `%d` raises `IllegalFormatConversionException` when the line is
//! actually written). Java exceptions surface as Kahlua does it for an
//! invoked method: a `RuntimeException` whose message is the cause's
//! `toString()`.
//!
//! The ` at <caller>` affix of `trace`/`warn`/`error`/... is the calling
//! Lua function in the original (`Lua(Vanilla).fn`). The Kahlua port does
//! not expose call frames, so it comes from [`set_lua_caller_hook`]; without
//! a hook it is `(UnknownStack)`, the original's text for a missing frame.
//! [`set_lua_stack_hook`] likewise supplies the frames `printStackTrace`
//! prints.
//!
//! Enum constants are exposed from [`DEBUG_TYPE_NAMES`] and
//! [`LOG_SEVERITY_NAMES`] only.

use crate::bind_core::JavaList;
use crate::expose::{
    bool_arg, enum_value, expose_class, expose_enum, global, int_arg, ClassBuilder, JavaEnum,
};
use pz_core::debug::{
    self, DebugLog, DebugType, FmtArg, JavaError, LogSeverity, LogThrowable, StackElement,
    DEBUG_TYPE_NAMES, LOG_SEVERITY_NAMES,
};
use pz_kahlua::exec::java_value_of;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const DEBUG_TYPE_CLASS: &str = "zombie.debug.DebugType";
pub const LOG_SEVERITY_CLASS: &str = "zombie.debug.LogSeverity";
pub const DEBUG_LOG_CLASS: &str = "zombie.debug.DebugLog";
pub const DEBUG_LOG_STREAM_CLASS: &str = "zombie.debug.DebugLogStream";
pub const DEBUG_OPTIONS_CLASS: &str = "zombie.debug.DebugOptions";
pub const BOOLEAN_DEBUG_OPTION_CLASS: &str = "zombie.debug.BooleanDebugOption";

/// Supplies the calling Lua frame for the ` at ...` affix.
pub type CallerHook = Box<dyn Fn(&Thread) -> Option<StackElement>>;
/// Supplies the stack (innermost caller first) for `printStackTrace`.
pub type StackHook = Box<dyn Fn(&Thread) -> Vec<StackElement>>;

thread_local! {
    static CALLER_HOOK: RefCell<Option<CallerHook>> = const { RefCell::new(None) };
    static STACK_HOOK: RefCell<Option<StackHook>> = const { RefCell::new(None) };
}

pub fn set_lua_caller_hook(hook: Option<CallerHook>) {
    CALLER_HOOK.with(|h| *h.borrow_mut() = hook);
}

pub fn set_lua_stack_hook(hook: Option<StackHook>) {
    STACK_HOOK.with(|h| *h.borrow_mut() = hook);
}

fn lua_caller(t: &Thread) -> Option<StackElement> {
    CALLER_HOOK.with(|h| h.borrow().as_ref().and_then(|f| f(t)))
}

fn lua_stack(t: &Thread) -> Vec<StackElement> {
    STACK_HOOK.with(|h| h.borrow().as_ref().map(|f| f(t)).unwrap_or_default())
}

/// Route Kahlua `print` to `DebugType.Lua`'s log stream, as
/// `KahluaThread`'s default output does.
pub fn route_print(thread: &mut Thread) {
    thread.out = Box::new(|s| debug::with(|d| d.println(DebugType::Lua, s)));
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn jerr(e: JavaError) -> LuaError {
    LuaError::runtime(e.to_string())
}

fn simple_class(v: &Value) -> String {
    let n = pz_kahlua::stdlib::java_class_name(v);
    n.rsplit('.').next().unwrap_or(&n).to_string()
}

fn enum_ordinal(v: &Value, class: &str) -> Option<usize> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == class)
            .map(|e| e.ordinal as usize),
        _ => None,
    }
}

pub fn debug_type_value(t: DebugType) -> Value {
    enum_value(DEBUG_TYPE_CLASS, t.name())
}

pub fn severity_value(s: LogSeverity) -> Value {
    enum_value(LOG_SEVERITY_CLASS, s.name())
}

fn type_of(v: &Value) -> Option<DebugType> {
    enum_ordinal(v, DEBUG_TYPE_CLASS).and_then(DebugType::from_ordinal)
}

fn sev_of(v: &Value) -> Option<LogSeverity> {
    enum_ordinal(v, LOG_SEVERITY_CLASS).and_then(LogSeverity::from_ordinal)
}

fn self_type(c: &NativeCall<'_>, method: &str) -> LuaResult<DebugType> {
    type_of(&c.arg(0)).ok_or_else(|| {
        LuaError::runtime(format!(
            "{method}: Expected a method call but got a function call."
        ))
    })
}

fn wrong_type(expected: &str, v: &Value) -> LuaError {
    LuaError::runtime(format!(
        "expected argument of type {expected}, got {}",
        simple_class(v)
    ))
}

/// A `DebugType` parameter (nil = `null`).
fn type_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<DebugType>> {
    let v = c.arg(i);
    if v.is_nil() {
        return Ok(None);
    }
    type_of(&v)
        .map(Some)
        .ok_or_else(|| wrong_type("DebugType", &v))
}

/// A `LogSeverity` parameter (nil = `null`).
fn sev_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<LogSeverity>> {
    let v = c.arg(i);
    if v.is_nil() {
        return Ok(None);
    }
    sev_of(&v)
        .map(Some)
        .ok_or_else(|| wrong_type("LogSeverity", &v))
}

/// A `String` parameter (nil = `null`; Kahlua has no number-to-String
/// conversion).
fn str_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<String>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Str(s) => Ok(Some(s.to_string())),
        v => Err(wrong_type("String", &v)),
    }
}

fn npe(what: &str) -> LuaError {
    jerr(JavaError::npe(what))
}

fn need_type(t: Option<DebugType>, method: &str) -> LuaResult<DebugType> {
    t.ok_or_else(|| {
        npe(&format!(
            "Cannot invoke \"zombie.debug.DebugType.{method}\" because \"type\" is null"
        ))
    })
}

fn fmt_args(c: &NativeCall<'_>, from: usize) -> Vec<FmtArg> {
    (from..c.nargs())
        .map(|i| FmtArg::from_lua(&c.arg(i)))
        .collect()
}

/// `(Object formatNoParams)` vs `(String format, Object... params)`.
fn format_and_params(c: &NativeCall<'_>, first: usize) -> LuaResult<(String, Vec<FmtArg>)> {
    let n = c.nargs().saturating_sub(first);
    match n {
        0 => Err(LuaError::runtime("expected 1 argument, got 0")),
        1 => Ok((java_value_of(&c.arg(first)), Vec::new())),
        _ => match c.arg(first) {
            Value::Str(s) => Ok((s.to_string(), fmt_args(c, first + 1))),
            Value::Nil => Ok(("null".into(), fmt_args(c, first + 1))),
            _ => Err(LuaError::runtime(format!("expected 1 argument, got {n}"))),
        },
    }
}

fn throwable_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<LogThrowable>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => Ok(Some(LogThrowable::new(o.to_lua_string()))),
        v => Err(wrong_type("Throwable", &v)),
    }
}

fn caller_prefix(c: &NativeCall<'_>) -> String {
    let el = lua_caller(c.thread);
    debug::with(|d| {
        debug::stack_trace_element_string(el.as_ref(), d.is_log_trace_file_location_enabled())
    })
}

fn list_of(values: Vec<Value>) -> Value {
    Value::Object(Rc::new(JavaList(values)))
}

fn str_list(v: Vec<String>) -> Value {
    list_of(v.into_iter().map(Value::from).collect())
}

/// An exposed Java object Lua can hold but has no use for.
struct Opaque(&'static str);

impl LuaObject for Opaque {
    fn class_name(&self) -> &str {
        self.0
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `DebugLog.getInstance()`.
struct DebugLogObject;

impl LuaObject for DebugLogObject {
    fn class_name(&self) -> &str {
        DEBUG_LOG_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// DebugType
// ---------------------------------------------------------------------------

type LogFn =
    fn(&mut DebugLog, DebugType, Option<&StackElement>, &str, &[FmtArg]) -> Result<(), JavaError>;

fn reg_log(b: &ClassBuilder, name: &'static str, f: LogFn) {
    b.method(name, move |c| {
        let t = self_type(c, name)?;
        let (fmt, params) = format_and_params(c, 1)?;
        let caller = lua_caller(c.thread);
        debug::with(|d| f(d, t, caller.as_ref(), &fmt, &params)).map_err(jerr)?;
        Ok(0)
    });
}

fn register_debug_type(thread: &mut Thread) {
    let b = expose_enum(thread, DEBUG_TYPE_CLASS, DEBUG_TYPE_NAMES);
    b.field("Default", debug_type_value(DebugType::DEFAULT));

    reg_log(&b, "trace", DebugLog::trace);
    reg_log(&b, "debugln", DebugLog::debugln);
    reg_log(&b, "debugOnceln", DebugLog::debug_once);
    reg_log(&b, "noise", DebugLog::noise);
    reg_log(&b, "warn", DebugLog::warn);
    reg_log(&b, "warnOnce", DebugLog::warn_once);
    reg_log(&b, "error", DebugLog::error);

    b.method("isName", |c| {
        let t = self_type(c, "isName")?;
        let s = str_arg(c, 1)?;
        Ok(c.push(s.is_some_and(|s| t.is_name(&s))))
    });
    b.method("isEnabled", |c| {
        let t = self_type(c, "isEnabled")?;
        if c.nargs() <= 1 {
            return Ok(c.push(debug::with(|d| d.is_enabled(t))));
        }
        match sev_arg(c, 1)? {
            Some(s) => Ok(c.push(debug::with(|d| d.is_enabled_for(t, s)))),
            None => {
                // `logSeverity.ordinal()` is reached once a level in the
                // orWith chain is not Off.
                let mut cur = Some(t);
                while let Some(x) = cur {
                    if debug::with(|d| d.severity(x)) != LogSeverity::Off {
                        return Err(npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"logSeverity\" is null"));
                    }
                    cur = x.or_with();
                }
                Ok(c.push(false))
            }
        }
    });
    b.method("getLogSeverity", |c| {
        let t = self_type(c, "getLogSeverity")?;
        Ok(c.push(severity_value(debug::with(|d| d.severity(t)))))
    });
    b.method("setLogSeverity", |c| {
        let t = self_type(c, "setLogSeverity")?;
        // Java stores null; this port has no null level and uses Off.
        let s = sev_arg(c, 1)?.unwrap_or(LogSeverity::Off);
        debug::with(|d| d.set_severity(t, s));
        Ok(0)
    });
    b.method("getLogStream", |c| {
        self_type(c, "getLogStream")?;
        Ok(c.push(Value::Object(Rc::new(Opaque(DEBUG_LOG_STREAM_CLASS)))))
    });
    b.method("getFormatter", |c| {
        self_type(c, "getFormatter")?;
        Ok(c.push(Value::Object(Rc::new(Opaque(
            "zombie.debug.IDebugLogFormatter",
        )))))
    });
    b.method("print", |c| {
        let t = self_type(c, "print")?;
        if c.nargs() < 2 {
            return Err(LuaError::runtime("expected 1 argument, got 0"));
        }
        if c.nargs() > 2 {
            return Err(LuaError::runtime(format!(
                "expected 1 argument, got {}",
                c.nargs() - 1
            )));
        }
        let s = java_value_of(&c.arg(1));
        debug::with(|d| d.print(t, &s));
        Ok(0)
    });
    b.method("printf", |c| {
        let t = self_type(c, "printf")?;
        let f = str_arg(c, 1)?
            .ok_or_else(|| npe("Cannot invoke \"String.length()\" because \"s\" is null"))?;
        let args = fmt_args(c, 2);
        debug::with(|d| d.printf(t, &f, &args)).map_err(jerr)?;
        Ok(c.push(Value::Object(Rc::new(Opaque(DEBUG_LOG_STREAM_CLASS)))))
    });
    b.method("println", |c| {
        let t = self_type(c, "println")?;
        if c.nargs() <= 1 {
            debug::with(|d| d.println(t, ""));
            return Ok(0);
        }
        let (fmt, params) = format_and_params(c, 1)?;
        debug::with(|d| d.println_fmt(t, &fmt, &params)).map_err(jerr)?;
        Ok(0)
    });
    b.method("write", |c| {
        let t = self_type(c, "write")?;
        let s = sev_arg(c, 1)?;
        let text = str_arg(c, 2)?.unwrap_or_else(|| "null".into());
        let caller = lua_caller(c.thread);
        if let Some(s) = s {
            debug::with(|d| d.routed_write(t, s, caller.as_ref(), &text));
        } else {
            return Err(npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"logSeverity\" is null"));
        }
        Ok(0)
    });
    b.method("routedWrite", |c| {
        let t = self_type(c, "routedWrite")?;
        let _offset = int_arg(c, 1);
        let s = sev_arg(c, 2)?
            .ok_or_else(|| npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"logSeverity\" is null"))?;
        let text = str_arg(c, 3)?.unwrap_or_else(|| "null".into());
        let caller = lua_caller(c.thread);
        debug::with(|d| d.routed_write(t, s, caller.as_ref(), &text));
        Ok(0)
    });
    b.method("printException", |c| {
        let t = self_type(c, "printException")?;
        let ex = throwable_arg(c, 1)?;
        let prefix = caller_prefix(c);
        let n = c.nargs() - 1;
        // (Throwable, LogSeverity) | (Throwable, String, LogSeverity) |
        // (Throwable, LogSeverity, String, Object...)
        if n == 2 {
            let s = sev_arg(c, 2)?.unwrap_or(LogSeverity::Off);
            debug::with(|d| d.print_exception(t, ex.as_ref(), None, &prefix, s));
        } else if n == 3 && sev_of(&c.arg(3)).is_some() {
            let msg = str_arg(c, 2)?;
            let s = sev_arg(c, 3)?.unwrap();
            debug::with(|d| d.print_exception(t, ex.as_ref(), msg.as_deref(), &prefix, s));
        } else if n >= 3 {
            let s = sev_arg(c, 2)?
                .ok_or_else(|| npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"logSeverity\" is null"))?;
            let f = str_arg(c, 3)?;
            debug::with(|d| d.print_exception_type_fmt(t, ex.as_ref(), s, &prefix, f.as_deref())).map_err(jerr)?;
        } else {
            return Err(LuaError::runtime(format!("expected 2 arguments, got {n}")));
        }
        Ok(0)
    });
    b.method("printStackTrace", |c| {
        let t = self_type(c, "printStackTrace")?;
        let frames = lua_stack(c.thread);
        let n = c.nargs() - 1;
        let r = match n {
            0 => debug::with(|d| d.print_stack_trace(t, LogSeverity::Error, 1, -1, None, &[], &frames)),
            1 => {
                let m = str_arg(c, 1)?;
                debug::with(|d| d.print_stack_trace(t, LogSeverity::Error, 1, -1, m.as_deref(), &[], &frames))
            }
            _ => {
                let s = sev_arg(c, 1)?
                    .ok_or_else(|| npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"severity\" is null"))?;
                let depth = int_arg(c, 2);
                let m = str_arg(c, 3)?;
                let params = fmt_args(c, 4);
                debug::with(|d| d.print_stack_trace(t, s, 1, depth, m.as_deref(), &params, &frames))
            }
        };
        r.map_err(jerr)?;
        Ok(0)
    });
}

// ---------------------------------------------------------------------------
// LogSeverity
// ---------------------------------------------------------------------------

fn register_log_severity(thread: &mut Thread) {
    let b = expose_enum(thread, LOG_SEVERITY_CLASS, LOG_SEVERITY_NAMES);
    b.field("All", severity_value(LogSeverity::ALL));
    b.static_fn("getValueList", |c| {
        let v = LogSeverity::VALUES
            .iter()
            .map(|s| severity_value(*s))
            .collect();
        Ok(c.push(list_of(v)))
    });
    b.method("isLogEnabled", |c| {
        let s = sev_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("isLogEnabled: Expected a method call but got a function call."))?;
        match sev_arg(c, 1)? {
            Some(o) => Ok(c.push(s.is_log_enabled(o))),
            None if s == LogSeverity::Off => Ok(c.push(false)),
            None => Err(npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"logSeverity\" is null")),
        }
    });
    b.method("isName", |c| {
        let s = sev_of(&c.arg(0)).ok_or_else(|| {
            LuaError::runtime("isName: Expected a method call but got a function call.")
        })?;
        let n = str_arg(c, 1)?;
        Ok(c.push(n.is_some_and(|n| s.is_name(&n))))
    });
}

// ---------------------------------------------------------------------------
// DebugLog
// ---------------------------------------------------------------------------

fn register_debug_log(thread: &mut Thread) {
    let b = expose_class(thread, DEBUG_LOG_CLASS, None);
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(DebugLogObject))))
    });
    b.static_fn("setDefaultLogSeverity", |_| {
        debug::with(|d| d.set_default_log_severity());
        Ok(0)
    });
    b.static_fn("printLogLevels", |_| {
        debug::with(|d| d.print_log_levels());
        Ok(0)
    });
    b.static_fn("isEnabled", |c| {
        let t = need_type(type_arg(c, 0)?, "isEnabled()")?;
        Ok(c.push(debug::with(|d| d.is_enabled(t))))
    });
    b.static_fn("isLogEnabled", |c| {
        let t = need_type(type_arg(c, 0)?, "isEnabled(zombie.debug.LogSeverity)")?;
        let s = sev_arg(c, 1)?
            .ok_or_else(|| npe("Cannot invoke \"zombie.debug.LogSeverity.ordinal()\" because \"logSeverity\" is null"))?;
        Ok(c.push(debug::with(|d| d.is_enabled_for(t, s))))
    });
    b.static_fn("log", |c| {
        // log(DebugType, String) | log(String)
        let (t, s) = if c.nargs() >= 2 {
            (
                need_type(type_arg(c, 0)?, "println(String)")?,
                str_arg(c, 1)?,
            )
        } else {
            (DebugType::General, str_arg(c, 0)?)
        };
        let s = s.unwrap_or_else(|| "null".into());
        debug::with(|d| d.log(t, &s));
        Ok(0)
    });
    b.static_fn("setLogEnabled", |c| {
        let t = need_type(type_arg(c, 0)?, "isEnabled()")?;
        let e = bool_arg(c, 1);
        debug::with(|d| d.set_log_enabled(t, e));
        Ok(0)
    });
    b.static_fn("setLogEnabledFromCommandLine", |c| {
        let s = str_arg(c, 0)?
            .ok_or_else(|| npe("Cannot invoke \"String.isEmpty()\" because \"token\" is null"))?;
        debug::with(|d| d.set_log_enabled_from_command_line(&s));
        Ok(0)
    });
    b.static_fn("getDebugTypes", |c| {
        let v = DebugLog::debug_types_sorted()
            .into_iter()
            .map(debug_type_value)
            .collect();
        Ok(c.push(list_of(v)))
    });
    b.static_fn("getSelectedProfileName", |c| {
        let n = debug::with(|d| d.selected_profile_name());
        Ok(c.push(n.map_or(Value::Nil, Value::from)))
    });
    b.static_fn("getProfileNames", |c| {
        Ok(c.push(str_list(debug::with(|d| d.profile_names()))))
    });
    b.static_fn("getProfileAliases", |c| {
        Ok(c.push(str_list(debug::with(|d| d.profile_aliases()))))
    });
    b.static_fn("getLogSeverityForSelectedProfile", |c| {
        let t = need_type(type_arg(c, 0)?, "name()")?;
        let s = debug::with(|d| d.log_severity_for_selected_profile(t)).map_err(jerr)?;
        Ok(c.push(s.map_or(Value::Nil, severity_value)))
    });
    b.static_fn("invokeProfile", |c| {
        let n = str_arg(c, 0)?.ok_or_else(|| {
            npe("Cannot invoke \"String.startsWith(String)\" because \"profileOrAlias\" is null")
        })?;
        debug::with(|d| d.invoke_profile(&n)).map_err(jerr)?;
        Ok(0)
    });
    b.static_fn("invokeSelectedProfile", |_| {
        debug::with(|d| d.invoke_selected_profile()).map_err(jerr)?;
        Ok(0)
    });
    b.static_fn("updateSelectedProfileAll", |c| {
        let s = sev_arg(c, 0)?;
        debug::with(|d| d.update_selected_profile_all(s)).map_err(jerr)?;
        Ok(0)
    });
    b.static_fn("updateSelectedProfile", |c| {
        let t = need_type(type_arg(c, 0)?, "name()")?;
        let s = sev_arg(c, 1)?;
        debug::with(|d| d.update_selected_profile(t, s)).map_err(jerr)?;
        Ok(0)
    });
    b.static_fn("writeConfigFile", |_| {
        debug::with(|d| d.write_config_file()).map_err(jerr)?;
        Ok(0)
    });
    b.static_fn("nativeLog", |c| {
        let t = str_arg(c, 0)?.unwrap_or_default();
        let s = str_arg(c, 1)?.unwrap_or_default();
        let text = str_arg(c, 2)?.unwrap_or_else(|| "null".into());
        let caller = lua_caller(c.thread);
        debug::with(|d| d.native_log(&t, &s, &text, caller.as_ref()));
        Ok(0)
    });

    // Instance methods of DebugLog.getInstance().
    b.method("init", |_| {
        debug::with(|d| d.init());
        Ok(0)
    });
    b.method("loadDebugConfig", |c| {
        let p = str_arg(c, 1)?;
        debug::with(|d| d.load_debug_config(p.as_deref()));
        Ok(0)
    });
    b.method("readConfigCommand", |c| {
        let s = str_arg(c, 1)?.unwrap_or_default();
        let e = bool_arg(c, 2);
        debug::with(|d| d.read_config_command(&s, e));
        Ok(0)
    });
    b.method("isLogTraceFileLocationEnabled", |c| {
        Ok(c.push(debug::with(|d| d.is_log_trace_file_location_enabled())))
    });
    b.method("shouldLogIncludeTimeMs", |c| {
        Ok(c.push(debug::with(|d| d.should_log_include_time_ms())))
    });
    b.method("shouldLogIncludeServerTime", |c| {
        Ok(c.push(debug::with(|d| d.should_log_include_server_time())))
    });
    b.method("isLogServerTimeMsEnabled", |c| {
        Ok(c.push(debug::with(|d| d.is_log_server_time_ms_enabled())))
    });
    b.method("setLogServerTimeMsEnabled", |c| {
        let e = bool_arg(c, 1);
        debug::with(|d| d.set_log_server_time_ms_enabled(e));
        Ok(0)
    });
    for (name, kind) in [
        ("formatLogStringForConsole", 0),
        ("formatLogStringForLogFile", 1),
        ("formatLogStringAnimationRecordingFile", 2),
    ] {
        b.method(name, move |c| {
            let t = type_arg(c, 1)?;
            let s = sev_arg(c, 2)?.ok_or_else(|| {
                npe("Cannot read field \"logPrefix\" because \"logSeverity\" is null")
            })?;
            let affix = str_arg(c, 3)?.unwrap_or_default();
            let text = java_value_of(&c.arg(4));
            let line = debug::with(|d| match kind {
                0 => d.format_console_line(t, s, &affix, &text),
                1 => d.format_log_file_line(t, s, &affix, &text),
                _ => d.format_recording_line(t, s, &affix, &text),
            });
            Ok(c.push(line))
        });
    }
    b.method("echoToLogFiles", |c| {
        let t = type_arg(c, 1)?;
        let s = sev_arg(c, 2)?.unwrap_or(LogSeverity::General);
        let affix = str_arg(c, 3)?.unwrap_or_default();
        let text = str_arg(c, 4)?.unwrap_or_else(|| "null".into());
        debug::with(|d| d.echo_to_log_files(t, s, &affix, &text));
        Ok(0)
    });
    b.method("createLogStream", |c| {
        Ok(c.push(Value::Object(Rc::new(Opaque(DEBUG_LOG_STREAM_CLASS)))))
    });
}

// ---------------------------------------------------------------------------
// DebugOptions
// ---------------------------------------------------------------------------

/// `DebugOptions.instance`.
struct DebugOptionsObject;

impl LuaObject for DebugOptionsObject {
    fn class_name(&self) -> &str {
        DEBUG_OPTIONS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["zombie.debug.options.IDebugOptionGroup"]
    }
}

/// One `BooleanDebugOption` (index into the registry).
struct BooleanDebugOptionObject(usize);

impl LuaObject for BooleanDebugOptionObject {
    fn class_name(&self) -> &str {
        BOOLEAN_DEBUG_OPTION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[
            "zombie.config.BooleanConfigOption",
            "zombie.config.ConfigOption",
        ]
    }
}

fn option_index(c: &NativeCall<'_>, method: &str) -> LuaResult<usize> {
    match c.arg(0) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<BooleanDebugOptionObject>()
            .map(|b| b.0)
            .ok_or_else(|| {
                LuaError::runtime(format!(
                    "{method}: Expected a method call but got a function call."
                ))
            }),
        _ => Err(LuaError::runtime(format!(
            "{method}: Expected a method call but got a function call."
        ))),
    }
}

fn option_value(i: usize) -> Value {
    Value::Object(Rc::new(BooleanDebugOptionObject(i)))
}

fn register_debug_options(thread: &mut Thread) {
    let opts = debug::debug_options();
    let b = expose_class(thread, DEBUG_OPTIONS_CLASS, None);
    b.field("VERSION", 1.0);
    b.static_fn("testThreadCrash", |_| Ok(0));
    let o = opts.clone();
    b.method("getOptionByName", move |c| {
        let n = str_arg(c, 1)?;
        let i = n.and_then(|n| o.borrow().index_of(&n));
        Ok(c.push(i.map_or(Value::Nil, option_value)))
    });
    let o = opts.clone();
    b.method("getOptionCount", move |c| {
        let n = o.borrow().option_count();
        Ok(c.push(n as f64))
    });
    let o = opts.clone();
    b.method("getOptionByIndex", move |c| {
        let i = int_arg(c, 1);
        let n = o.borrow().option_count();
        if i < 0 || i as usize >= n {
            return Err(jerr(JavaError::new(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {i} out of bounds for length {n}"),
            )));
        }
        Ok(c.push(option_value(i as usize)))
    });
    let o = opts.clone();
    b.method("setBoolean", move |c| {
        let n = str_arg(c, 1)?.unwrap_or_default();
        let v = bool_arg(c, 2);
        o.borrow_mut().set_boolean(&n, v);
        Ok(0)
    });
    let o = opts.clone();
    b.method("getBoolean", move |c| {
        let n = str_arg(c, 1)?.unwrap_or_default();
        let v = o.borrow().get_boolean(&n);
        Ok(c.push(v))
    });
    let o = opts.clone();
    b.method("save", move |_| {
        o.borrow().save();
        Ok(0)
    });
    let o = opts.clone();
    b.method("load", move |_| {
        o.borrow_mut().load();
        Ok(0)
    });
    let o = opts.clone();
    b.method("init", move |_| {
        o.borrow_mut().load();
        Ok(0)
    });
    b.method("getName", |c| Ok(c.push("DebugOptions")));
    b.method("getParent", |c| Ok(c.push(Value::Nil)));

    let ob = expose_class(thread, BOOLEAN_DEBUG_OPTION_CLASS, None);
    let o = opts.clone();
    ob.method("getName", move |c| {
        let i = option_index(c, "getName")?;
        let n = o.borrow().options[i].0.name.clone();
        Ok(c.push(n))
    });
    let o = opts.clone();
    ob.method("getValue", move |c| {
        let i = option_index(c, "getValue")?;
        let v = o.borrow().get_value(i);
        Ok(c.push(v))
    });
    let o = opts.clone();
    ob.method("getValueAsObject", move |c| {
        let i = option_index(c, "getValueAsObject")?;
        let v = o.borrow().get_value(i);
        Ok(c.push(v))
    });
    let o = opts.clone();
    ob.method("getValueAsString", move |c| {
        let i = option_index(c, "getValueAsString")?;
        let v = o.borrow().get_value(i);
        Ok(c.push(v.to_string()))
    });
    let o = opts.clone();
    ob.method("setValue", move |c| {
        let i = option_index(c, "setValue")?;
        let v = bool_arg(c, 1);
        o.borrow_mut().set_value(i, v);
        Ok(0)
    });
    let o = opts.clone();
    ob.method("getDefaultValue", move |c| {
        let i = option_index(c, "getDefaultValue")?;
        let v = o.borrow().options[i].0.default_value;
        Ok(c.push(v))
    });
    let o = opts.clone();
    ob.method("isDebugOnly", move |c| {
        let i = option_index(c, "isDebugOnly")?;
        let v = o.borrow().options[i].0.debug_only;
        Ok(c.push(v))
    });
    let o = opts.clone();
    ob.method("resetToDefault", move |c| {
        let i = option_index(c, "resetToDefault")?;
        o.borrow_mut().options[i].1.reset_to_default();
        Ok(0)
    });
    let o = opts.clone();
    ob.method("setDefaultToCurrentValue", move |c| {
        let i = option_index(c, "setDefaultToCurrentValue")?;
        let mut r = o.borrow_mut();
        r.options[i].1.set_default_to_current_value();
        let v = r.options[i].1.get_bool();
        r.options[i].0.default_value = v;
        Ok(0)
    });
    let o = opts.clone();
    ob.method("parse", move |c| {
        let i = option_index(c, "parse")?;
        let s = str_arg(c, 1)?.unwrap_or_default();
        o.borrow_mut().options[i].1.parse(&s);
        Ok(0)
    });
    let o = opts;
    ob.method("isValidString", move |c| {
        let i = option_index(c, "isValidString")?;
        let s = str_arg(c, 1)?.unwrap_or_default();
        let v = o.borrow().options[i].1.is_valid_string(&s);
        Ok(c.push(v))
    });
    ob.method("getType", |c| Ok(c.push("boolean")));
    ob.method("getParent", |c| Ok(c.push(Value::Nil)));
    ob.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o.to_lua_string(),
            _ => String::new(),
        };
        Ok(c.push(s))
    });
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the classes and globals. Also performs what the game has done
/// by the time Lua starts: `DebugLog.init()`, and `Core.debug` /
/// `GameServer.server` / `GameClient.client` taken from the shared Core.
pub fn register(thread: &mut Thread) {
    if let Ok(core) = crate::bind_core::shared_core().try_borrow() {
        let (dbg, server, client) = (core.debug, core.is_server, core.is_client);
        debug::with(|d| {
            d.core_debug = dbg;
            d.is_server = server;
            d.is_client = client;
        });
        debug::debug_options().borrow_mut().core_debug = dbg;
    }
    debug::with(|d| d.init());

    register_debug_type(thread);
    register_log_severity(thread);
    register_debug_log(thread);
    register_debug_options(thread);

    // LuaManager.GlobalObject.log(DebugType, String): type.debugln(message).
    global(thread, "log", |c| {
        let t = need_type(type_arg(c, 0)?, "debugln(Object)")?;
        let m = str_arg(c, 1)?.unwrap_or_else(|| "null".into());
        let caller = lua_caller(c.thread);
        debug::with(|d| d.debugln(t, caller.as_ref(), &m, &[])).map_err(jerr)?;
        Ok(0)
    });
    // writeLog(loggerName, logs)
    global(thread, "writeLog", |c| {
        let n = str_arg(c, 0)?
            .ok_or_else(|| npe("Cannot invoke \"String.length()\" because \"name\" is null"))?;
        let l = str_arg(c, 1)?
            .ok_or_else(|| npe("Cannot invoke \"String.length()\" because \"logs\" is null"))?;
        debug::with(|d| d.write_log(&n, &l));
        Ok(0)
    });
    global(thread, "getDebugOptions", |c| {
        Ok(c.push(Value::Object(Rc::new(DebugOptionsObject))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_core::debug::{FixedClock, MemorySink};
    use pz_kahlua::TableRef;

    fn setup() -> (Thread, MemorySink) {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_core::register(
            &mut t,
            None,
            Rc::new(RefCell::new(pz_core::core::Core::new())),
        );
        let sink = MemorySink::new();
        debug::with(|d| {
            d.set_sink(sink.clone());
            d.set_clock(Rc::new(FixedClock::new(1_792_791_004_860, 0)));
        });
        register(&mut t);
        (t, sink)
    }

    fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone())
            .map_err(|e| e.to_string())?;
        t.pcall(&f, &[]).map_err(|(m, _)| java_value_of(&m))
    }

    #[test]
    fn constants_are_singletons() {
        let (mut t, _) = setup();
        let r = run(
            &mut t,
            "return DebugType.Lua == DebugType.valueOf('Lua'), DebugType.Default == DebugType.General, \
             LogSeverity.All == LogSeverity.Trace, DebugType.AnimationRecorder:ordinal(), #DebugType.values(), \
             tostring(DebugType.ISUI), DebugType.Lua:getLogSeverity() == LogSeverity.General",
        )
        .unwrap();
        assert!(r[0].truthy() && r[1].truthy() && r[2].truthy());
        assert_eq!(r[3].as_num(), Some(75.0));
        assert_eq!(r[4].as_num(), Some(76.0));
        assert_eq!(r[5].as_str(), Some("ISUI"));
        assert!(r[6].truthy());
    }

    #[test]
    fn lua_logging_calls() {
        let (mut t, sink) = setup();
        run(
            &mut t,
            "DebugType.Lua:warn('require(\"%s\") failed', 'X') \
             DebugType.Lua:warn(5) \
             DebugType.Lua:println() \
             DebugType.Lua:println('a %d') \
             DebugType.Lua:println('n=%s b=%s', 3, true) \
             DebugType.Lua:error('boom %.1f', 2.25) \
             DebugType.ISUI:trace('hidden %d', 1) \
             DebugType.General:debugln('hidden in release') \
             log(DebugType.Action, 'hidden too') \
             DebugLog.log('via DebugLog')",
        )
        .unwrap();
        let log = sink.log();
        assert_eq!(
            log.stdout(),
            "WARN : Lua          f:0 at (UnknownStack)                      > require(\"X\") failed\n\
             WARN : Lua          f:0 at (UnknownStack)                      > 5.0\n\
             LOG  : Lua          f:0> \n\
             LOG  : Lua          f:0> a %d\n\
             LOG  : Lua          f:0> n=3.0 b=true\n\
             LOG  : General      f:0> via DebugLog\n"
        );
        assert_eq!(
            log.stderr(),
            "ERROR: Lua          f:0 at (UnknownStack)                      > boom 2.3\n"
        );
    }

    #[test]
    fn caller_hook_and_format_errors() {
        let (mut t, sink) = setup();
        set_lua_caller_hook(Some(Box::new(|_| {
            Some(StackElement::lua(
                "Vanilla",
                "corpseStorageCheck.lua",
                None,
                1,
            ))
        })));
        run(&mut t, "DebugType.Lua:warn('x')").unwrap();
        assert_eq!(
            sink.log().stdout(),
            "WARN : Lua          f:0 at Lua(Vanilla).corpseStorageCheck.lua > x\n"
        );
        let e = run(&mut t, "DebugType.Lua:warn('%d', 1)").unwrap_err();
        assert_eq!(e, "java.util.IllegalFormatConversionException: d != java.lang.Double java.lang.RuntimeException");
        // Disabled types never format, so no exception.
        run(&mut t, "DebugType.ISUI:warn('%d', 1)").unwrap();
        let e = run(&mut t, "DebugType.ISUI.warn('x')").unwrap_err();
        assert!(e.contains("Expected a method call"), "{e}");
        set_lua_caller_hook(None);
    }

    #[test]
    fn debug_mode_and_severity_api() {
        let (mut t, sink) = setup();
        debug::with(|d| d.core_debug = true);
        let r = run(
            &mut t,
            "DebugType.Grapple:setLogSeverity(LogSeverity.Noise) \
             log(DebugType.Grapple, 'hello') \
             DebugType.Grapple:noise('n') \
             DebugType.Grapple:debugOnceln('once') DebugType.Grapple:debugOnceln('once') \
             DebugType.AnimationRecorder:setLogSeverity(LogSeverity.Trace) \
             return DebugType.Grapple:isEnabled(), DebugType.Grapple:isEnabled(LogSeverity.Trace), \
                    DebugType.Death:isEnabled(), DebugLog.isLogEnabled(DebugType.Death, LogSeverity.Trace), \
                    LogSeverity.Warning:isLogEnabled(LogSeverity.Error), LogSeverity.getValueList():size(), \
                    DebugLog.getDebugTypes():get(0):name()",
        )
        .unwrap();
        assert!(r[0].truthy());
        assert!(!r[1].truthy());
        assert!(r[2].truthy() && r[3].truthy() && r[4].truthy());
        assert_eq!(r[5].as_num(), Some(7.0));
        assert_eq!(r[6].as_str(), Some("Action"));
        let out = sink.log().stdout();
        assert_eq!(out.matches("DEBUG: Grapple").count(), 2, "{out}");
        assert!(
            out.contains("NOISE: Grapple      f:0 at (UnknownStack)                      > n\n")
        );
        debug::with(|d| d.core_debug = false);
    }

    #[test]
    fn stack_trace_and_profiles_from_lua() {
        let (mut t, sink) = setup();
        set_lua_stack_hook(Some(Box::new(|_| {
            vec![StackElement::lua(
                "Vanilla",
                "f",
                Some("ISBaseObject.lua"),
                37,
            )]
        })));
        run(&mut t, "DebugType.LuaObject:setLogSeverity(LogSeverity.Warning) \
                     DebugType.LuaObject:printStackTrace(LogSeverity.Warning, 4, 'No Type found: x')").unwrap();
        assert_eq!(
            sink.log().stdout(),
            "No Type found: x\n\tLua(Vanilla).f(ISBaseObject.lua:37)\n"
        );
        set_lua_stack_hook(None);

        let e = run(
            &mut t,
            "return DebugLog.getLogSeverityForSelectedProfile(DebugType.Lua)",
        )
        .unwrap_err();
        assert!(e.starts_with("java.lang.NullPointerException"), "{e}");
        debug::with(|d| d.load_debug_config(None));
        let r = run(
            &mut t,
            "DebugLog.updateSelectedProfile(DebugType.Sound, LogSeverity.Error) \
             return DebugLog.getSelectedProfileName(), DebugLog.getProfileNames():get(0), \
                    DebugLog.getLogSeverityForSelectedProfile(DebugType.Sound) == LogSeverity.Error",
        )
        .unwrap();
        assert_eq!(r[0].as_str(), Some("default"));
        assert_eq!(r[1].as_str(), Some("default"));
        assert!(r[2].truthy());
    }

    #[test]
    fn write_log_and_debug_options() {
        let (mut t, sink) = setup();
        debug::debug_options().borrow_mut().load_options_list_xml(
            "<debugOptionsXml><options><name>Cheat.Farming.FastGrow</name><value>false</value></options>\
             <options><name>UI.Render.Outline</name><value>true</value></options></debugOptionsXml>",
        );
        let r = run(
            &mut t,
            "writeLog('admin', 'did a thing') \
             local o = getDebugOptions() \
             o:setBoolean('Cheat.Farming.FastGrow', true) \
             local opt = o:getOptionByIndex(1) \
             return o:getBoolean('Cheat.Farming.FastGrow'), o:getOptionCount(), opt:getName(), opt:getValue(), \
                    o:getOptionByName('Nope'), o:getBoolean('Nope')",
        )
        .unwrap();
        assert!(r[0].truthy());
        assert_eq!(r[1].as_num(), Some(2.0));
        assert_eq!(r[2].as_str(), Some("UI.Render.Outline"));
        assert!(r[3].truthy());
        assert!(r[4].is_nil() && !r[5].truthy());
        assert_eq!(
            sink.log().file_lines("admin"),
            ["[23-10-26 21:30:04.860] did a thing."]
        );
        let e = run(&mut t, "getDebugOptions():getOptionByIndex(5)").unwrap_err();
        assert!(
            e.contains("IndexOutOfBoundsException: Index 5 out of bounds for length 2"),
            "{e}"
        );
    }
}
