//! `LuaManager.GlobalObject` global functions that don't depend on a
//! ported subsystem yet. Subsystem-specific globals live in their bind_*
//! modules.

use crate::expose::{global, int_arg, str_arg};
use pz_kahlua::{TableRef, Thread, Value};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `LuaManager.copyTable(null, from)`: deep copy of nested tables, keeping
/// iteration order.
pub fn copy_table(from: &TableRef) -> TableRef {
    let to = TableRef::new();
    for (k, v) in from.entries() {
        match v {
            Value::Table(t) => to.rawset(k, Value::Table(copy_table(&t))),
            v => to.rawset(k, v),
        }
    }
    to
}

/// Java `isInstance` by simple class name.
pub fn instance_of(v: &Value, name: &str) -> bool {
    match v {
        Value::Nil => false,
        Value::Object(o) => {
            let simple = |c: &str| crate::expose::simple_name(c).to_string();
            simple(o.class_name()) == name || o.super_classes().iter().any(|c| simple(c) == name)
        }
        Value::Closure(_) => name == "LuaClosure",
        Value::Table(_) => name == "KahluaTableImpl",
        Value::Str(_) => matches!(name, "String" | "Object" | "CharSequence" | "Comparable"),
        Value::Num(_) => matches!(name, "Double" | "Number" | "Object" | "Comparable"),
        Value::Bool(_) => matches!(name, "Boolean" | "Object" | "Comparable"),
        _ => false,
    }
}

thread_local! {
    /// `TextureID.useFiltering`.
    pub static USE_TEXTURE_FILTERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn register(thread: &mut Thread) {
    let ret_false: &[&str] = &[
        "isServer",
        "isClient",
        "isDemo",
        "isCoopHost",
        "isMultiplayer",
        "isServerSoftReset",
        "getDebug",
    ];
    for name in ret_false {
        global(thread, name, |c| Ok(c.push(false)));
    }
    global(thread, "isDesktopOpenSupported", |c| Ok(c.push(true)));
    global(thread, "getCellSizeInSquares", |c| Ok(c.push(256.0)));
    global(thread, "getChunkSizeInSquares", |c| Ok(c.push(8.0)));
    global(thread, "copyTable", |c| {
        let r = match c.arg(0) {
            Value::Table(t) => copy_table(&t),
            _ => TableRef::new(),
        };
        Ok(c.push(r))
    });
    global(thread, "mergeTable", |c| {
        let result = TableRef::new();
        let mut index = 1;
        for i in 0..c.nargs() {
            if let Value::Table(t) = c.arg(i) {
                let n = t.len();
                for j in 1..=n {
                    result.set_int(index, t.get_int(j));
                    index += 1;
                }
            }
        }
        Ok(c.push(result))
    });
    global(thread, "instanceof", |c| {
        let name = str_arg(c, 1).unwrap_or_else(|| std::rc::Rc::from(""));
        let r = instance_of(&c.arg(0), &name);
        Ok(c.push(r))
    });
    global(thread, "getTimestamp", |c| {
        Ok(c.push((now_ms() / 1000) as f64))
    });
    global(thread, "getTimestampMs", |c| Ok(c.push(now_ms() as f64)));
    global(thread, "getTimeInMillis", |c| Ok(c.push(now_ms() as f64)));
    global(thread, "getFileSeparator", |c| {
        Ok(c.push(std::path::MAIN_SEPARATOR.to_string()))
    });
    global(thread, "isSystemMacOS", |c| {
        Ok(c.push(cfg!(target_os = "macos")))
    });
    global(thread, "isSystemWindows", |c| {
        Ok(c.push(cfg!(target_os = "windows")))
    });
    global(thread, "isSystemLinux", |c| {
        Ok(c.push(!cfg!(target_os = "macos") && !cfg!(target_os = "windows")))
    });
    // Steam integration is not part of this port's runtime yet.
    global(thread, "getSteamModeActive", |c| Ok(c.push(false)));
    global(thread, "isSteamOverlayEnabled", |c| Ok(c.push(false)));
    // TextureID.useFiltering: read by texture loading in pz-render.
    global(thread, "useTextureFiltering", |c| {
        USE_TEXTURE_FILTERING.with(|f| f.set(crate::expose::bool_arg(c, 0)));
        Ok(0)
    });
    // Only returns a trace in debug mode with DebugType.ISUIStackTrace on.
    global(thread, "getISUIStackTrace", |c| Ok(c.push(Value::Nil)));
    global(thread, "getNumActivePlayers", |c| Ok(c.push(0.0)));
    global(thread, "getPlayer", |c| Ok(c.push(Value::Nil)));
    global(thread, "getSpecificPlayer", |c| {
        let _ = int_arg(c, 0);
        Ok(c.push(Value::Nil))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_table_is_deep_and_ordered() {
        let inner = TableRef::new();
        inner.set_str("z", 1);
        let t = TableRef::new();
        t.set_str("b", inner.clone());
        t.set_str("a", 2);
        let c = copy_table(&t);
        assert_eq!(
            c.keys()
                .iter()
                .map(|k| k.as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(!c.get_str("b").as_table().unwrap().ptr_eq(&inner));
    }
}
