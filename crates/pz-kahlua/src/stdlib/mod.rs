//! Kahlua standard library: `BaseLib`, `StringLib`, `TableLib`, `MathLib`,
//! `OsLib`, `CoroutineLib`, `RandomLib` and `LuaCompiler.register`.
//!
//! The game additionally runs `stdlib.lua` from its install directory after
//! these are registered; pass its text to [`install`].

mod base;
mod coroutine;
mod math;
mod os;
mod random;
mod string;
mod table;

pub use base::{load_string, setmetatable};
pub use table::{insert_at, pairs_iterator, remove_at};

use crate::exec::{raw_tonumber, raw_tostring};
use crate::table::TableRef;
use crate::value::Value;
use crate::vm::{LuaError, LuaResult, NativeCall, NativeFunction, Thread};

/// `J2SEPlatform.setupEnvironment`, in the original registration order.
/// `stdlib_lua` is the text of the game's `stdlib.lua`, if available.
pub fn install(thread: &mut Thread, stdlib_lua: Option<&str>) -> LuaResult<()> {
    let env = thread.globals().clone();
    env.wipe();
    env.set_str("_G", env.clone());
    env.set_str("_VERSION", "Kahlua kahlua.major.kahlua.minor.kahlua.fix for Lua lua.version (J2SE)");
    math::register(&env);
    base::register(&env);
    random::register(thread, &env);
    string::register(thread, &env);
    coroutine::register(&env);
    os::register(&env);
    table::register(&env);
    base::register_compiler(&env);
    if let Some(src) = stdlib_lua {
        let p = crate::compiler::compile(src, Some("stdlib.lua"), Some("stdlib.lua"), None)?;
        let f = Value::Closure(crate::vm::LuaClosure::new(std::rc::Rc::new(p), env.clone()));
        thread.call(&f, &[])?;
    }
    Ok(())
}

/// Register a native function under `name` in `table`.
pub fn reg(table: &TableRef, name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) {
    table.set_str(name, Value::Native(NativeFunction::new(name, f)));
}

/// Like [`reg`] but with a different display name (`toString()` of the
/// Java function object, e.g. `table.insert`).
pub fn reg_named(table: &TableRef, key: &str, display: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) {
    table.set_str(key, Value::Native(NativeFunction::new(display, f)));
}

// ---- KahluaUtil argument helpers (1-based `n`, like the original) ----

/// `luaAssert(cond, msg)`.
pub fn lua_assert(cond: bool, msg: &str) -> LuaResult<()> {
    if cond {
        Ok(())
    } else {
        Err(LuaError::runtime(msg))
    }
}

/// `getOptionalArg`: arguments beyond the frame top are nil.
pub fn opt_arg(c: &NativeCall<'_>, n: usize) -> Value {
    if n > c.top() {
        Value::Nil
    } else {
        c.arg(n - 1)
    }
}

/// `getArg`.
pub fn arg(c: &NativeCall<'_>, n: usize, function: &str) -> LuaResult<Value> {
    let v = opt_arg(c, n);
    if v.is_nil() {
        return Err(LuaError::runtime(format!("missing argument #{n} to '{function}'")));
    }
    Ok(v)
}

/// `getStringArg`.
pub fn string_arg(c: &NativeCall<'_>, n: usize, function: &str) -> LuaResult<std::rc::Rc<str>> {
    let v = arg(c, n, function)?;
    raw_tostring(&v).ok_or_else(|| {
        LuaError::runtime(format!("bad argument #{n} to '{function}' (string expected, got nil)"))
    })
}

/// `getOptionalStringArg`.
pub fn opt_string_arg(c: &NativeCall<'_>, n: usize) -> Option<std::rc::Rc<str>> {
    raw_tostring(&opt_arg(c, n))
}

/// `getNumberArg` / `getDoubleArg`.
pub fn number_arg(c: &NativeCall<'_>, n: usize, function: &str) -> LuaResult<f64> {
    let v = arg(c, n, function)?;
    raw_tonumber(&v).ok_or_else(|| {
        LuaError::runtime(format!("bad argument #{n} to '{function}' (double expected, got nil)"))
    })
}

/// `getOptionalNumberArg`.
pub fn opt_number_arg(c: &NativeCall<'_>, n: usize) -> Option<f64> {
    raw_tonumber(&opt_arg(c, n))
}

/// Java `(Double) o` cast on an argument, failing like a ClassCastException.
pub fn table_cast(v: &Value) -> LuaResult<TableRef> {
    match v {
        Value::Table(t) => Ok(t.clone()),
        other => Err(LuaError::Runtime {
            message: format!("class {} cannot be cast to class se.krka.kahlua.vm.KahluaTable", java_class_name(other)),
            class: "java.lang.ClassCastException",
        }),
    }
}

/// Java class name of a value's runtime object.
pub fn java_class_name(v: &Value) -> String {
    match v {
        Value::Nil => "null".into(),
        Value::Bool(_) => "java.lang.Boolean".into(),
        Value::Num(_) => "java.lang.Double".into(),
        Value::Str(_) => "java.lang.String".into(),
        Value::Table(_) => "se.krka.kahlua.j2se.KahluaTableImpl".into(),
        Value::Closure(_) => "se.krka.kahlua.vm.LuaClosure".into(),
        Value::Native(_) => "se.krka.kahlua.vm.JavaFunction".into(),
        Value::Coroutine(_) => "se.krka.kahlua.vm.Coroutine".into(),
        Value::Object(o) => o.class_name().into(),
    }
}
