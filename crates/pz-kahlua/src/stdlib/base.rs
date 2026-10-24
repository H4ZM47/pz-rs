//! `BaseLib` and `LuaCompiler.register` (loadstring).

use super::*;
use crate::exec::java_value_of;
use crate::number::kahlua_tonumber;
use crate::vm::LuaClosure;
use std::rc::Rc;

pub fn register(env: &TableRef) {
    reg(env, "pcall", |c| {
        let f = c.arg(0);
        let args: Vec<Value> = (1..c.nargs()).map(|i| c.arg(i)).collect();
        match c.thread.pcall_ex(&f, &args) {
            Ok(vals) => {
                c.push(true);
                for v in vals.iter() {
                    c.push(v.clone());
                }
                Ok(1 + vals.len())
            }
            Err((msg, trace, err)) => {
                c.push(false);
                c.push(msg);
                c.push(trace);
                c.push(Value::Object(std::rc::Rc::new(JavaException(err))));
                Ok(4)
            }
        }
    });
    reg(env, "print", |c| {
        let tostring = c.thread.globals().get_str("tostring");
        let mut s = String::new();
        for i in 0..c.nargs() {
            if i > 0 {
                s.push('\t');
            }
            let r = c.thread.call1(&tostring, &[c.arg(i)])?;
            s.push_str(&java_value_of(&r));
        }
        (c.thread.out)(&s);
        Ok(0)
    });
    reg(env, "select", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let a = c.arg(0);
        if let Value::Str(s) = &a {
            if s.starts_with('#') {
                return Ok(c.push((c.nargs() - 1) as f64));
            }
        }
        let d = raw_tonumber(&a).unwrap_or(f64::NAN);
        let index = d as i32;
        if index >= 1 && index as usize <= c.nargs() - 1 {
            // The last nArguments-index values on the stack are the result.
            return Ok(c.nargs() - index as usize);
        }
        Ok(0)
    });
    reg(env, "type", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let t = c.arg(0).type_name();
        Ok(c.push(t))
    });
    reg(env, "tostring", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let s = c.thread.tostring(&c.arg(0))?;
        Ok(c.push(s))
    });
    reg(env, "tonumber", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let o = c.arg(0);
        if c.nargs() == 1 {
            let r = raw_tonumber(&o).map(Value::Num).unwrap_or_default();
            return Ok(c.push(r));
        }
        let Value::Str(s) = &o else {
            return Err(LuaError::Runtime {
                message: format!("class {} cannot be cast to class java.lang.String", java_class_name(&o)),
                class: "java.lang.ClassCastException",
            });
        };
        let Some(dradix) = raw_tonumber(&c.arg(1)) else {
            return Ok(c.push(Value::Nil));
        };
        let radix = dradix as i32;
        if radix as f64 != dradix {
            return Ok(c.push(Value::Nil));
        }
        let r = kahlua_tonumber(s, radix).map_err(LuaError::runtime)?;
        Ok(c.push(r.map(Value::Num).unwrap_or_default()))
    });
    reg(env, "getmetatable", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let m = c.thread.metatable(&c.arg(0));
        Ok(c.push(m))
    });
    reg(env, "setmetatable", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let o = c.arg(0);
        let meta = match c.arg(1) {
            Value::Nil => None,
            Value::Table(t) => Some(t),
            other => return Err(table_cast(&other).unwrap_err()),
        };
        setmetatable(c.thread, &o, meta, false)?;
        c.set_top(1)?;
        Ok(1)
    });
    reg(env, "error", |c| {
        if c.nargs() >= 1 {
            let st = opt_string_arg(c, 2).map(|s| s.to_string()).unwrap_or_default();
            c.thread.current.borrow_mut().stack_trace = st;
            return Err(LuaError::Kahlua(c.arg(0)));
        }
        Ok(0)
    });
    reg(env, "unpack", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let t = table_cast(&c.arg(0))?;
        let di = if c.nargs() >= 2 { c.arg(1) } else { Value::Nil };
        let dj = if c.nargs() >= 3 { c.arg(2) } else { Value::Nil };
        let i = if di.is_nil() { 1 } else { from_double(&di)? as i32 as i64 };
        let j = if dj.is_nil() { t.len() } else { from_double(&dj)? as i32 as i64 };
        let n = 1 + j - i;
        if n <= 0 {
            c.set_top(0)?;
            return Ok(0);
        }
        let n = n as usize;
        c.set_top(n)?;
        for b in 0..n {
            let v = t.get_int(i + b as i64);
            c.set(b, v);
        }
        Ok(n)
    });
    reg(env, "setfenv", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let new_env = match c.arg(1) {
            Value::Table(t) => t,
            Value::Nil => return Err(LuaError::runtime("expected a table")),
            other => return Err(table_cast(&other).unwrap_err()),
        };
        match c.arg(0) {
            Value::Closure(cl) => {
                *cl.env.borrow_mut() = new_env;
            }
            o => {
                let Some(d) = raw_tonumber(&o) else {
                    return Err(LuaError::runtime("expected a lua function or a number"));
                };
                let level = d as i32;
                if level == 0 {
                    c.thread.current.borrow_mut().env = Some(new_env);
                    return Ok(0);
                }
                let frames = &c.thread.frames;
                let idx = frames.len() as i64 - level as i64 - 1;
                if idx < 0 {
                    return Err(LuaError::runtime("Level too high"));
                }
                match &frames[idx as usize].closure {
                    Some(cl) => *cl.env.borrow_mut() = new_env,
                    None => return Err(LuaError::runtime(format!("No closure found at this level: {level}"))),
                }
            }
        }
        c.set_top(1)?;
        Ok(1)
    });
    reg(env, "getfenv", |c| {
        let o = if c.nargs() >= 1 { c.arg(0) } else { Value::Num(1.0) };
        let res = match &o {
            Value::Nil | Value::Native(_) => c.thread.current.borrow().env.clone(),
            Value::Closure(cl) => Some(cl.env.borrow().clone()),
            _ => {
                let Some(d) = raw_tonumber(&o) else { return Err(LuaError::runtime("Expected number")) };
                let level = d as i32;
                lua_assert(level >= 0, "level must be non-negative")?;
                let frames = &c.thread.frames;
                let idx = frames.len() as i64 - level as i64 - 1;
                if idx < 0 {
                    return Err(LuaError::runtime("Level too high"));
                }
                match &frames[idx as usize].closure {
                    Some(cl) => Some(cl.env.borrow().clone()),
                    None => c.thread.current.borrow().env.clone(),
                }
            }
        };
        Ok(c.push(res.map(Value::Table).unwrap_or_default()))
    });
    reg(env, "rawequal", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let r = c.arg(0).raw_equals(&c.arg(1));
        Ok(c.push(r))
    });
    reg(env, "rawset", |c| {
        lua_assert(c.nargs() >= 3, "Not enough arguments")?;
        let t = table_cast(&c.arg(0))?;
        t.rawset(c.arg(1), c.arg(2));
        c.set_top(1)?;
        Ok(1)
    });
    reg(env, "rawget", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let t = table_cast(&c.arg(0))?;
        let v = t.rawget(&c.arg(1));
        Ok(c.push(v))
    });
    reg(env, "collectgarbage", |c| {
        let option = if c.nargs() > 0 { c.arg(0) } else { Value::Nil };
        match option.as_str() {
            None if option.is_nil() => Ok(0),
            Some("step") | Some("collect") => Ok(0),
            Some("count") => {
                // Memory statistics in KiB: used, free, total.
                c.set_top(3)?;
                c.set(0, Value::Num(0.0));
                c.set(1, Value::Num(0.0));
                c.set(2, Value::Num(0.0));
                Ok(3)
            }
            _ => Err(LuaError::runtime(format!("invalid option: {}", java_value_of(&option)))),
        }
    });
    reg(env, "debugstacktrace", |c| {
        let level = opt_number_arg(c, 2).map(|d| d as i32).unwrap_or(0).max(0) as usize;
        let count = opt_number_arg(c, 3).map(|d| d as i32).unwrap_or(i32::MAX).max(0) as usize;
        let halt = opt_number_arg(c, 4).map(|d| d as i32).unwrap_or(0).max(0) as usize;
        let frames = &c.thread.frames;
        let mut s = String::new();
        let mut remaining = count;
        let mut i = frames.len() as i64 - 1 - level as i64;
        while i >= halt as i64 && remaining > 0 {
            s.push_str(&frame_line(&frames[i as usize]));
            remaining -= 1;
            i -= 1;
        }
        Ok(c.push(s))
    });
    reg(env, "bytecodeloader", |c| {
        let modname = string_arg(c, 1, "loader")?;
        Ok(c.push(format!("Could not find the bytecode for '{modname}' in classpath")))
    });
}

fn frame_line(fr: &crate::vm::CallFrame) -> String {
    if let Some(cl) = &fr.closure {
        let lines = &cl.prototype.lines;
        if fr.pc >= 1 && fr.pc - 1 < lines.len() {
            return format!("at {}:{}\n", cl.prototype.name.as_deref().unwrap_or("null"), lines[fr.pc - 1]);
        }
        return String::new();
    }
    fr.native.as_ref().map(|n| format!("at {}\n", n.name)).unwrap_or_default()
}

fn from_double(v: &Value) -> LuaResult<f64> {
    match v {
        Value::Num(n) => Ok(*n),
        other => Err(LuaError::Runtime {
            message: format!("class {} cannot be cast to class java.lang.Double", java_class_name(other)),
            class: "java.lang.ClassCastException",
        }),
    }
}

/// `BaseLib.setmetatable(thread, o, newMeta, raw)`.
pub fn setmetatable(thread: &mut Thread, o: &Value, meta: Option<TableRef>, raw: bool) -> LuaResult<()> {
    lua_assert(!o.is_nil(), "Expected table, got nil")?;
    let old = thread.raw_metatable(o);
    if !raw {
        if let Some(old) = &old {
            if !thread.table_get(&Value::Table(old.clone()), &Value::str("__metatable"))?.is_nil() {
                return Err(LuaError::runtime("cannot change a protected metatable"));
            }
        }
    }
    match o {
        Value::Table(t) => {
            t.set_metatable(meta);
            Ok(())
        }
        _ => Err(LuaError::runtime("Could not set metatable for object")),
    }
}

/// `LuaCompiler.register`: `loadstring` and `loadstream`.
pub fn register_compiler(env: &TableRef) {
    reg(env, "loadstring", |c| {
        // All failures are caught and returned as (nil, message).
        let fail = |c: &mut NativeCall<'_>, m: &str| {
            c.push(Value::Nil);
            c.push(m.to_string());
            Ok(2)
        };
        if c.nargs() < 1 {
            return fail(c, "not enough arguments");
        }
        let src = match c.arg(0) {
            Value::Str(s) => s,
            Value::Nil => return fail(c, "No source given"),
            other => return fail(c, &format!("class {} cannot be cast to class java.lang.String", java_class_name(&other))),
        };
        let name = if c.nargs() >= 2 { c.arg(1).as_str().map(str::to_string) } else { None };
        let env = c.thread.current_env();
        match load_string(&src, name.as_deref(), env) {
            Ok(f) => Ok(c.push(f)),
            Err(e) => {
                c.push(Value::Nil);
                let msg = match e {
                    LuaError::Kahlua(v) => v,
                    LuaError::Runtime { message, .. } => Value::from(message),
                };
                c.push(msg);
                Ok(2)
            }
        }
    });
    reg(env, "loadstream", |c| {
        // Readers are engine objects; the engine registers file loading.
        c.push(Value::Nil);
        c.push("Invalid type to loadstream");
        Ok(2)
    });
}

/// `LuaCompiler.loadstring(source, name, env)`.
pub fn load_string(src: &str, name: Option<&str>, env: TableRef) -> LuaResult<Value> {
    let p = crate::compiler::compile(src, name, None, None)?;
    Ok(Value::Closure(LuaClosure::new(Rc::new(p), env)))
}

/// A Java exception object as Lua sees it (pcall's fourth result).
pub struct JavaException(pub LuaError);

impl crate::value::LuaObject for JavaException {
    fn class_name(&self) -> &str {
        match &self.0 {
            LuaError::Kahlua(_) => "se.krka.kahlua.vm.KahluaException",
            LuaError::Runtime { class, .. } => class,
        }
    }
    /// `Throwable.toString()`.
    fn to_lua_string(&self) -> String {
        match &self.0 {
            LuaError::Kahlua(v) => format!("se.krka.kahlua.vm.KahluaException: {}", java_value_of(v)),
            LuaError::Runtime { message, class } => format!("{class}: {message}"),
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
