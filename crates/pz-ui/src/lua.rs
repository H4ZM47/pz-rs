//! Calling into Lua the way `UIManager.tableget` and `LuaCaller` do.

use pz_kahlua::{TableRef, Thread, Value};

/// `UIManager.tableget(table, key)`: `KahluaThread.tableget`, so
/// `__index` chains (the `ISUIElement` class hierarchy) are followed.
/// Returns `None` for nil (and for an erroring `__index`, which the Java
/// would propagate as an exception).
pub fn tableget(thread: &mut Thread, table: &TableRef, key: &str) -> Option<Value> {
    match thread.table_get(&Value::Table(table.clone()), &Value::str(key)) {
        Ok(Value::Nil) | Err(_) => None,
        Ok(v) => Some(v),
    }
}

/// `LuaCaller.pcallvoid` / `protectedCallVoid`: errors are swallowed (the
/// VM reports them itself).
pub fn pcall_void(thread: &mut Thread, f: &Value, args: &[Value]) {
    let _ = thread.pcall(f, args);
}

/// `pcallBoolean` / `protectedCallBoolean`: `Some(b)` only when the call
/// succeeded and its first result is a boolean.
pub fn pcall_bool(thread: &mut Thread, f: &Value, args: &[Value]) -> Option<bool> {
    match thread.pcall(f, args) {
        Ok(v) => match v.first() {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        },
        Err(_) => None,
    }
}
