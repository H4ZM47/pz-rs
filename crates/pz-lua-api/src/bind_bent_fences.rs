//! `zombie.iso.BentFences` as `media/lua/server/Items/BentFences.lua` uses
//! it: `BentFences.getInstance():addFenceTiles(VERSION, tiles)` fills the
//! shared [`BentFences`] registry ([`shared_bent_fences`]), which the
//! fence-bending states read.

use crate::expose::{expose_class, int_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::IsoDirections;
use pz_world_states::{BentFences, FenceTileDef};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const CLASS: &str = "zombie.iso.BentFences";

thread_local! {
    static INSTANCE: Rc<RefCell<BentFences>> = Rc::default();
}

/// `BentFences.getInstance()`.
pub fn shared_bent_fences() -> Rc<RefCell<BentFences>> {
    INSTANCE.with(Rc::clone)
}

struct LuaBentFences;

impl LuaObject for LuaBentFences {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn java_err(class: &'static str, message: String) -> LuaError {
    LuaError::Runtime { message, class }
}

fn class_cast(v: &Value) -> LuaError {
    java_err(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class se.krka.kahlua.j2se.KahluaTableImpl",
            pz_kahlua::stdlib::java_class_name(v)
        ),
    )
}

/// `(KahluaTableImpl) value`; `nil` stays `null`.
fn opt_table(v: Value) -> LuaResult<Option<TableRef>> {
    match v {
        Value::Nil => Ok(None),
        Value::Table(t) => Ok(Some(t)),
        other => Err(class_cast(&other)),
    }
}

/// `KahluaTableImpl.rawgetInt`: -1 unless the value is a number.
fn rawget_int(t: &TableRef, k: &str) -> i32 {
    t.get_str(k).as_num().map_or(-1, |d| d as i32)
}

/// `tableToTiles(table, key)`: each value's `toString()`.
fn tiles(t: &TableRef, k: &str) -> LuaResult<Vec<String>> {
    Ok(match opt_table(t.get_str(k))? {
        None => Vec::new(),
        Some(t) => t
            .entries()
            .into_iter()
            .map(|(_, v)| pz_kahlua::exec::java_value_of(&v))
            .collect(),
    })
}

/// `IsoDirections.valueOf(value.rawgetStr("dir"))`.
fn direction(t: &TableRef) -> LuaResult<IsoDirections> {
    let Value::Str(name) = t.get_str("dir") else {
        return Err(java_err(
            "java.lang.NullPointerException",
            "Name is null".into(),
        ));
    };
    IsoDirections::ALL
        .into_iter()
        .find(|d| format!("{d:?}") == *name)
        .ok_or_else(|| {
            java_err(
                "java.lang.IllegalArgumentException",
                format!("No enum constant zombie.iso.IsoDirections.{name}"),
            )
        })
}

/// The `tiles` table of `addFenceTiles`, read as the Java loop reads it.
pub fn fence_tile_defs(tiles_table: &TableRef) -> LuaResult<Vec<FenceTileDef>> {
    let mut out = Vec::new();
    for (_, v) in tiles_table.entries() {
        let Some(value) = opt_table(v.clone())? else {
            return Err(java_err("java.lang.NullPointerException", String::new()));
        };
        let mut d = FenceTileDef {
            dir: Some(direction(&value)?),
            health: rawget_int(&value, "health"),
            collapsed_offset: rawget_int(&value, "collapsedOffset"),
            collapsed_size_x: rawget_int(&value, "collapsedSizeX"),
            collapsed_size_y: rawget_int(&value, "collapsedSizeY"),
            do_smash: matches!(value.get_str("doSmash"), Value::Bool(true)),
            ..Default::default()
        };
        let stages = opt_table(value.get_str("stages"))?;
        let collapsed = opt_table(value.get_str("collapsed"))?;
        let debris = opt_table(value.get_str("debris"))?;
        if let Some(st) = stages {
            let mut list = Vec::new();
            for (_, s) in st.entries() {
                let Some(stage) = opt_table(s)? else {
                    return Err(java_err("java.lang.NullPointerException", String::new()));
                };
                list.push((rawget_int(&stage, "stage"), tiles(&stage, "tiles")?));
            }
            d.stages = Some(list);
        }
        if collapsed.is_some() {
            d.collapsed = Some(tiles(&value, "collapsed")?);
        }
        if debris.is_some() {
            d.debris = Some(tiles(&value, "debris")?);
        }
        out.push(d);
    }
    Ok(out)
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaBentFences))))
    });
    b.method("addFenceTiles", |c: &mut NativeCall<'_>| {
        crate::expose::this::<LuaBentFences>(c)?;
        let version = int_arg(c, 1);
        let Some(t) = opt_table(c.arg(2))? else {
            return Err(java_err("java.lang.NullPointerException", String::new()));
        };
        let defs = fence_tile_defs(&t)?;
        INSTANCE.with(|i| i.borrow_mut().add_fence_tiles(version, &defs));
        Ok(0)
    });
    b.method("Reset", |c| {
        crate::expose::this::<LuaBentFences>(c)?;
        INSTANCE.with(|i| {
            let mut i = i.borrow_mut();
            i.entries.clear();
            i.fence_map.clear();
        });
        Ok(0)
    });
    b.method("isEnabled", |c| {
        crate::expose::this::<LuaBentFences>(c)?;
        let e = INSTANCE.with(|i| i.borrow().is_enabled());
        Ok(c.push(e))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_fence_tiles_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let src = r#"
            local tiles = {}
            table.insert(tiles, { dir = "N", health = 50, collapsedSizeX = 1, collapsedSizeY = 2,
                stages = {{ stage = 0, tiles = {"a_0", "a_1"} }, { stage = 1, tiles = {"b_0", "b_1"} }},
                collapsed = {"c_0", "c_1"}, debris = {"d_0"} })
            table.insert(tiles, { dir = "W", stages = {{ stage = 0, tiles = {"e_0"} }} })
            BentFences.getInstance():addFenceTiles(2, tiles)
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).unwrap();
        let bf = shared_bent_fences();
        let bf = bf.borrow();
        assert_eq!(bf.entries.len(), 2);
        assert_eq!(bf.entries[0].health, 50);
        assert_eq!(bf.entries[1].health, 100);
        assert_eq!(bf.entries[0].length, 2);
        assert!(!bf.entries[0].do_smash);
        assert!(bf.collapsed_tiles.contains("c_1"));
        assert!(bf.debris_tiles.contains("d_0"));
        assert_eq!(bf.fence_map.get("b_1"), Some(&vec![0]));
        assert_eq!(bf.fence_map.get("e_0"), Some(&vec![1]));
        let bad = pz_kahlua::stdlib::load_string(
            r#"BentFences.getInstance():addFenceTiles(2, {{ dir = "Q" }})"#,
            Some("test"),
            t.globals().clone(),
        )
        .unwrap();
        let e = t.call(&bad, &[]).unwrap_err().to_string();
        assert!(e.contains("No enum constant"), "{e}");
    }
}
