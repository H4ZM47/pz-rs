//! `zombie.core.raknet.VoiceManager.LuaRegister`: the `VoiceManager` table.
//!
//! Voice chat needs a multiplayer connection and FMOD recording, neither of
//! which exists in single player here, so this matches the original's
//! behaviour with VOIP disabled (`Core.soundDisabled || voipDisabled`).

use pz_kahlua::{NativeFunction, TableRef, Thread, Value};

pub fn register(thread: &mut Thread) {
    let t = TableRef::new();
    // Returns 1 without pushing, so the caller receives the last argument.
    t.set_str(
        "playerSetMute",
        Value::Native(NativeFunction::new("playerSetMute", |c| {
            let last = if c.nargs() > 0 { c.arg(c.nargs() - 1) } else { Value::Nil };
            Ok(c.push(last))
        })),
    );
    // No connected players: the loop finds nobody and returns true.
    t.set_str("playerGetMute", Value::Native(NativeFunction::new("playerGetMute", |c| Ok(c.push(true)))));
    t.set_str("RecordDevices", Value::Native(NativeFunction::new("RecordDevices", |c| Ok(c.push(TableRef::new())))));
    thread.globals().set_str("VoiceManager", t);
}

/// `zombie.core.input.Input` globals: `getKeyName` / `getKeyCode`. They
/// go through the Lua-visible `Keyboard` and translator functions, exactly
/// the calls `Input` makes.
pub fn register_input_globals(thread: &mut Thread) {
    fn call_global(c: &mut pz_kahlua::NativeCall<'_>, path: &[&str], args: &[Value]) -> pz_kahlua::LuaResult<Value> {
        let mut v = Value::Table(c.thread.globals().clone());
        for p in path {
            v = c.thread.table_get(&v, &Value::str(p))?;
        }
        c.thread.call1(&v, args)
    }
    crate::expose::global(thread, "getKeyName", |c| {
        let code = crate::expose::int_arg(c, 0);
        if code >= 10000 {
            let t = call_global(c, &["getText"], &[Value::from(format!("IGUI_mouse_btn_{}", code - 10000))])?;
            return Ok(c.push(t));
        }
        let key = call_global(c, &["Keyboard", "getKeyName"], &[Value::from(code)])?;
        let name = pz_kahlua::exec::java_value_of(&key);
        let tr = call_global(c, &["getTextOrNull"], &[Value::from(format!("IGUI_Key_{name}"))])?;
        Ok(c.push(if tr.is_nil() { key } else { tr }))
    });
    crate::expose::global(thread, "getKeyCode", |c| {
        let name = crate::expose::str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let fixed = match name.as_str() {
            "Left ALT" if cfg!(target_os = "macos") => Some(219),
            "Right ALT" if cfg!(target_os = "macos") => Some(220),
            "Right SHIFT" => Some(54),
            "Left SHIFT" => Some(42),
            "Left ALT" => Some(56),
            "Right ALT" => Some(184),
            _ => None,
        };
        if let Some(k) = fixed {
            return Ok(c.push(k));
        }
        let k = call_global(c, &["Keyboard", "getKeyIndex"], &[Value::from(name)])?;
        Ok(c.push(k))
    });
}
