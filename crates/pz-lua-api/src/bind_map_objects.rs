//! `zombie.Lua.MapObjects`: the per-sprite Lua callbacks the
//! `media/lua/server/Map/MapObjects/MO*.lua` files register
//! (`OnNewWithSprite` for objects of a brand-new square, `OnLoadWithSprite`
//! for loaded ones), ordered by descending priority with one function per
//! priority.
//!
//! `newGridSquare` / `loadGridSquare` (calling them for each object of a
//! square) need the `IsoObject` Lua objects, which have no binding yet; the
//! engine reads the callbacks with [`on_new_callbacks`] /
//! [`on_load_callbacks`].

use crate::expose::{expose_class, int_arg};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::cell::RefCell;
use std::collections::HashMap;

/// `MapObjects.Callback`.
#[derive(Default)]
struct Callback {
    functions: Vec<Value>,
    priority: Vec<i16>,
}

thread_local! {
    static ON_NEW: RefCell<HashMap<String, Callback>> = RefCell::default();
    static ON_LOAD: RefCell<HashMap<String, Callback>> = RefCell::default();
}

fn java_err(class: &'static str, message: &str) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

/// `OnNewWithSprite(String, LuaClosure, int)` / `OnLoadWithSprite` on one
/// map.
fn add(map: &RefCell<HashMap<String, Callback>>, sprite: &str, function: Value, priority: i32) {
    let priority = priority as i16;
    let mut map = map.borrow_mut();
    let cb = map.entry(sprite.to_string()).or_default();
    for i in 0..cb.functions.len() {
        if cb.priority[i] < priority {
            cb.functions.insert(i, function);
            cb.priority.insert(i, priority);
            return;
        }
        if cb.priority[i] != priority {
            continue;
        }
        cb.functions[i] = function;
        cb.priority[i] = priority;
        return;
    }
    cb.functions.push(function);
    cb.priority.push(priority);
}

/// Both overloads: a sprite name or a table of names.
fn with_sprite(
    c: &mut NativeCall<'_>,
    map: &'static std::thread::LocalKey<RefCell<HashMap<String, Callback>>>,
) -> LuaResult<usize> {
    let names = c.arg(0);
    let function = c.arg(1);
    let priority = int_arg(c, 2);
    let check_fn = |f: &Value| match f {
        Value::Nil => Err(java_err(
            "java.lang.NullPointerException",
            "function is null",
        )),
        Value::Closure(_) => Ok(()),
        other => Err(LuaError::runtime(format!(
            "Expected se.krka.kahlua.vm.LuaClosure but got {}",
            pz_kahlua::stdlib::java_class_name(other)
        ))),
    };
    match &names {
        Value::Table(t) => {
            if t.is_empty() {
                return Err(java_err(
                    "java.lang.IllegalArgumentException",
                    "invalid sprite-name table",
                ));
            }
            check_fn(&function)?;
            for (_, v) in t.entries() {
                let Value::Str(s) = &v else {
                    return Err(java_err(
                        "java.lang.IllegalArgumentException",
                        &format!(
                            "expected string but got \"{}\"",
                            pz_kahlua::exec::java_value_of(&v)
                        ),
                    ));
                };
                if s.is_empty() {
                    return Err(java_err(
                        "java.lang.IllegalArgumentException",
                        "invalid sprite name",
                    ));
                }
                map.with(|m| add(m, s, function.clone(), priority));
            }
        }
        Value::Str(s) if !s.is_empty() => {
            check_fn(&function)?;
            map.with(|m| add(m, s, function.clone(), priority));
        }
        Value::Str(_) | Value::Nil => {
            return Err(java_err(
                "java.lang.IllegalArgumentException",
                "invalid sprite name",
            ));
        }
        other => {
            return Err(LuaError::runtime(format!(
                "Expected java.lang.String but got {}",
                pz_kahlua::stdlib::java_class_name(other)
            )))
        }
    }
    Ok(0)
}

fn callbacks(
    map: &'static std::thread::LocalKey<RefCell<HashMap<String, Callback>>>,
    sprite: &str,
) -> Vec<Value> {
    map.with(|m| {
        m.borrow()
            .get(sprite)
            .map(|c| c.functions.clone())
            .unwrap_or_default()
    })
}

/// The `OnNewWithSprite` functions of a sprite, highest priority first.
pub fn on_new_callbacks(sprite: &str) -> Vec<Value> {
    callbacks(&ON_NEW, sprite)
}

/// The `OnLoadWithSprite` functions of a sprite, highest priority first.
pub fn on_load_callbacks(sprite: &str) -> Vec<Value> {
    callbacks(&ON_LOAD, sprite)
}

/// `MapObjects.Reset()`.
pub fn reset() {
    ON_NEW.with(|m| m.borrow_mut().clear());
    ON_LOAD.with(|m| m.borrow_mut().clear());
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.Lua.MapObjects", Some("java.lang.Object"));
    b.static_fn("OnNewWithSprite", |c| with_sprite(c, &ON_NEW));
    b.static_fn("OnLoadWithSprite", |c| with_sprite(c, &ON_LOAD));
    b.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn run(src: &str) -> Result<(), String> {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).map(|_| ()).map_err(|e| e.to_string())
    }

    #[test]
    fn priorities_order_and_replace() {
        reset();
        run(r#"
            a = function() end; b = function() end; c = function() end; d = function() end
            MapObjects.OnNewWithSprite("s_0", a, 5)
            MapObjects.OnNewWithSprite("s_0", b, 10)
            MapObjects.OnNewWithSprite({"s_0", "s_1"}, c, 1)
            MapObjects.OnNewWithSprite("s_0", d, 5)
            MapObjects.OnLoadWithSprite("s_0", a, 5)
        "#)
        .unwrap();
        let n = on_new_callbacks("s_0");
        assert_eq!(n.len(), 3);
        assert_eq!(on_new_callbacks("s_1").len(), 1);
        assert_eq!(on_load_callbacks("s_0").len(), 1);
        assert!(on_new_callbacks("other").is_empty());
        let e = run(r#"MapObjects.OnNewWithSprite("", function() end, 1)"#).unwrap_err();
        assert!(e.contains("invalid sprite name"), "{e}");
        let e = run(r#"MapObjects.OnNewWithSprite("x", nil, 1)"#).unwrap_err();
        assert!(e.contains("function is null"), "{e}");
        let e = run(r#"MapObjects.OnNewWithSprite({}, function() end, 1)"#).unwrap_err();
        assert!(e.contains("invalid sprite-name table"), "{e}");
    }
}
