//! `TableLib`.

use super::*;
use crate::exec::{table_access_error, table_length};

fn get_table(c: &NativeCall<'_>) -> LuaResult<TableRef> {
    lua_assert(c.nargs() >= 1, "expected table, got no arguments")?;
    table_cast(&c.arg(0))
}

fn int_of(v: &Value) -> LuaResult<i64> {
    // `rawTonumber(o).intValue()`; a NullPointerException when absent.
    match raw_tonumber(v) {
        Some(d) => Ok(d as i32 as i64),
        None => Err(LuaError::Runtime {
            message: "null".into(),
            class: "java.lang.NullPointerException",
        }),
    }
}

/// `TableLib.insert(state, table, position, element)`.
pub fn insert_at(t: &mut Thread, table: &TableRef, position: i64, element: Value) -> LuaResult<()> {
    let result = insert_at_value(t, table, position, &element);
    let mut retired = crate::table::RetiredTableValues::new();
    retired.push(element);
    crate::gc::retire_table_values(retired);
    result
}

fn insert_at_value(
    t: &mut Thread,
    table: &TableRef,
    position: i64,
    element: &Value,
) -> LuaResult<()> {
    let tv = Value::Table(table.clone());
    let len = table_length(table)?;
    let mut i = len;
    while i >= position {
        let v = t.table_get(&tv, &Value::Num(i as f64))?;
        t.table_set(&tv, &Value::Num((i + 1) as f64), v)?;
        i -= 1;
    }
    t.table_set(&tv, &Value::Num(position as f64), element.clone())
}

/// `TableLib.remove(thread, table, position)`.
pub fn remove_at(t: &mut Thread, table: &TableRef, position: i64) -> LuaResult<Value> {
    let tv = Value::Table(table.clone());
    let ret = t.table_get(&tv, &Value::Num(position as f64))?;
    let len = table_length(table)?;
    for i in position..len {
        let v = t.table_get(&tv, &Value::Num((i + 1) as f64))?;
        t.table_set(&tv, &Value::Num(i as f64), v)?;
    }
    t.table_set(&tv, &Value::Num(len as f64), Value::Nil)?;
    Ok(ret)
}

pub fn register(env: &TableRef) {
    let table = TableRef::new();
    reg_named(&table, "concat", "table.concat", |c| {
        let t = get_table(c)?;
        let sep = if c.nargs() >= 2 {
            raw_tostring(&c.arg(1)).map(|s| s.to_string())
        } else {
            Some(String::new())
        };
        let first = if c.nargs() >= 3 {
            int_of(&c.arg(2))?
        } else {
            1
        };
        let last = if c.nargs() >= 4 {
            int_of(&c.arg(3))?
        } else {
            table_length(&t)?
        };
        let mut s = String::new();
        for i in first..=last {
            if i > first {
                // StringBuilder.append(null) appends "null".
                s.push_str(sep.as_deref().unwrap_or("null"));
            }
            match raw_tostring(
                &t.try_rawget(&Value::Num(i as f64))
                    .map_err(table_access_error)?,
            ) {
                Some(v) => s.push_str(&v),
                None => s.push_str("null"),
            }
        }
        Ok(c.push(s))
    });
    reg_named(&table, "insert", "table.insert", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let t = table_cast(&c.arg(0))?;
        let (pos, elem) = if c.nargs() > 2 {
            (int_of(&c.arg(1))?, c.arg(2))
        } else {
            (table_length(&t)? + 1, c.arg(1))
        };
        insert_at(c.thread, &t, pos, elem)?;
        Ok(0)
    });
    reg_named(&table, "remove", "table.remove", |c| {
        let t = get_table(c)?;
        let pos = if c.nargs() > 1 {
            int_of(&c.arg(1))?
        } else {
            table_length(&t)?
        };
        let v = remove_at(c.thread, &t, pos)?;
        Ok(c.push(v))
    });
    reg_named(&table, "newarray", "table.newarray", |c| {
        // KahluaArray behaves as a dense table for scripts.
        let ret = TableRef::new();
        let param = opt_arg(c, 1);
        if let (Value::Table(t), 1) = (&param, c.nargs()) {
            let n = table_length(&t)?;
            let mut i = n;
            while i >= 1 {
                ret.set_int(
                    i,
                    t.try_rawget(&Value::Num(i as f64))
                        .map_err(table_access_error)?,
                );
                i -= 1;
            }
            return Ok(c.push(ret));
        }
        let mut i = c.nargs();
        while i >= 1 {
            ret.set_int(i as i64, c.arg(i - 1));
            i -= 1;
        }
        Ok(c.push(ret))
    });
    reg_named(&table, "pairs", "table.pairs", |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let Value::Table(t) = c.arg(0) else {
            return Err(LuaError::runtime("Expected a table"));
        };
        Ok(c.push(pairs_iterator(&t)?))
    });
    reg_named(&table, "isempty", "table.isempty", |c| {
        let t = get_table(c)?;
        Ok(c.push(t.try_is_empty().map_err(table_access_error)?))
    });
    reg_named(&table, "wipe", "table.wipe", |c| {
        let mut retired = crate::table::RetiredTableValues::new();
        let result = get_table(c)?
            .try_wipe(&mut retired)
            .map_err(table_access_error);
        crate::gc::retire_table_values(retired);
        result?;
        Ok(0)
    });
    let ipairs_iter = Value::Native(NativeFunction::new("ipairs_iterator", |c| {
        lua_assert(c.nargs() >= 2, "Not enough arguments")?;
        let Value::Table(t) = c.arg(0) else {
            return Err(LuaError::runtime("Expected a table"));
        };
        let Value::Num(i) = c.arg(1) else {
            return Err(LuaError::runtime("Expected a number"));
        };
        let next = Value::Num(i + 1.0);
        let v = t.try_rawget(&next).map_err(table_access_error)?;
        if v.is_nil() {
            return Ok(0);
        }
        c.push(next);
        c.push(v);
        Ok(2)
    }));
    reg_named(&table, "ipairs", "table.ipairs", move |c| {
        lua_assert(c.nargs() >= 1, "Not enough arguments")?;
        let Value::Table(t) = c.arg(0) else {
            return Err(LuaError::runtime("Expected a table"));
        };
        c.push(ipairs_iter.clone());
        c.push(t);
        c.push(0.0);
        Ok(3)
    });
    env.set_str("table", table);
}

/// `KahluaTableImpl.iterator()`: a stateful iterator over a key snapshot;
/// values are read live, so a removed key yields nil (ending the loop).
pub fn pairs_iterator(t: &TableRef) -> LuaResult<Value> {
    let keys = t.try_keys().map_err(table_access_error)?;
    let t = t.clone();
    let idx = std::cell::Cell::new(0usize);
    Ok(Value::Native(NativeFunction::new("iterator", move |c| {
        let i = idx.get();
        if i < keys.len() {
            idx.set(i + 1);
            let k = keys[i].clone();
            let v = t.try_get_own(&k).map_err(table_access_error)?;
            c.push(k);
            c.push(v);
            return Ok(2);
        }
        Ok(0)
    })))
}
