//! `zombie.iso.BrokenFences` as `media/lua/server/Items/BrokenFences.lua`
//! uses it: `BrokenFences.getInstance():addBrokenTiles(tiles)` and
//! `:addDebrisTiles(debris)` fill the shared [`BrokenFences`] registry
//! ([`shared_broken_fences`]) that fence destruction reads.
//!
//! The `IsoObject` / `IsoGridSquare` methods (`destroyFence`,
//! `setDestroyed`, `setDamagedLeft/Right`, `updateSprite`,
//! `isBreakableObject`, `getBreakableObject`, `addItems`) take world
//! objects, which have no Lua binding here; the engine calls the
//! registry's decision functions.

use crate::bind_java_util::class_cast;
use crate::expose::{expose_class, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world_states::{BrokenFences, BrokenTile};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const CLASS: &str = "zombie.iso.BrokenFences";
const TABLE_CLASS: &str = "se.krka.kahlua.j2se.KahluaTableImpl";

thread_local! {
    static INSTANCE: Rc<RefCell<BrokenFences>> = Rc::default();
}

/// `BrokenFences.getInstance()`.
pub fn shared_broken_fences() -> Rc<RefCell<BrokenFences>> {
    INSTANCE.with(Rc::clone)
}

struct LuaBrokenFences;

impl LuaObject for LuaBrokenFences {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn java_err(class: &'static str, message: &str) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

/// `(KahluaTableImpl) v`; `null` stays `None`.
fn opt_table(v: Value) -> LuaResult<Option<TableRef>> {
    match v {
        Value::Nil => Ok(None),
        Value::Table(t) => Ok(Some(t)),
        other => Err(class_cast(&other, TABLE_CLASS)),
    }
}

/// `tableToTiles(table, key)`: the values' `toString()`, `null` for a
/// missing or empty table.
fn tiles(t: &TableRef, k: &str) -> LuaResult<Option<Vec<String>>> {
    let Some(t) = opt_table(t.get_str(k))? else {
        return Ok(None);
    };
    let v: Vec<String> = t.entries().into_iter().map(|(_, v)| pz_kahlua::exec::java_value_of(&v)).collect();
    Ok((!v.is_empty()).then_some(v))
}

/// The value table of a key (`(KahluaTableImpl) it.getValue()`, then
/// dereferenced).
fn value_table(v: Value) -> LuaResult<TableRef> {
    opt_table(v)?.ok_or_else(|| java_err("java.lang.NullPointerException", ""))
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.static_fn("getInstance", |c| Ok(c.push(Value::Object(Rc::new(LuaBrokenFences)))));
    b.method("addBrokenTiles", |c: &mut NativeCall<'_>| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let t = opt_table(c.arg(1))?.ok_or_else(|| java_err("java.lang.NullPointerException", ""))?;
        // Each tile is stored before the next one is read; `allMap` is
        // filled at the end.
        let mut defs = Vec::new();
        let mut err = None;
        for (k, v) in t.entries() {
            let key = pz_kahlua::exec::java_value_of(&k);
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let read = value_table(v).and_then(|value| {
                Ok(BrokenTile {
                    self_: tiles(&value, "self")?,
                    left: tiles(&value, "left")?,
                    right: tiles(&value, "right")?,
                    ..Default::default()
                })
            });
            match read {
                Ok(tile) => {
                    // `IsoSpriteManager.instance.getSprite(key)`: the sound
                    // check (and adding the sprite) needs the tile
                    // definitions.
                    if let Some(None) = crate::bind_sprites::sprite_property(&key, "ThumpSound") {
                        eprintln!("ERROR: Sound: Missing ThumpSound for breakable object {key}");
                    }
                    defs.push((key, tile));
                }
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        INSTANCE.with(|i| {
            let mut i = i.borrow_mut();
            if err.is_some() {
                // The thrown exception skips `allMap.putAll`.
                let all = i.all_map.clone();
                i.add_broken_tiles(defs);
                i.all_map = all;
            } else {
                i.add_broken_tiles(defs);
            }
        });
        match err {
            Some(e) => Err(e),
            None => Ok(0),
        }
    });
    b.method("addDebrisTiles", |c: &mut NativeCall<'_>| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let t = opt_table(c.arg(1))?.ok_or_else(|| java_err("java.lang.NullPointerException", ""))?;
        for (k, v) in t.entries() {
            let key = pz_kahlua::exec::java_value_of(&k);
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let value = value_table(v)?;
            let sides = [
                tiles(&value, "north")?,
                tiles(&value, "south")?,
                tiles(&value, "west")?,
                tiles(&value, "east")?,
            ];
            INSTANCE
                .with(|i| i.borrow_mut().add_debris_tiles(vec![(key, sides)]))
                .map_err(|e| java_err("java.lang.IllegalArgumentException", &e.to_string()))?;
        }
        Ok(0)
    });
    b.method("isBreakableSprite", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let name = str_arg(c, 1);
        let r = INSTANCE.with(|i| i.borrow().is_breakable_sprite(name.as_deref()));
        Ok(c.push(r))
    });
    b.method("Reset", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        INSTANCE.with(|i| i.borrow_mut().reset());
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        // The installed game's stdlib.lua defines assert; this fixture
        // uses only Rust-provided primitives and an equivalent local check.
        let src = format!("local function assert(c) if not c then error('assertion failed') end end\n{src}");
        let f = pz_kahlua::stdlib::load_string(&src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).map(|_| ()).map_err(|e| e.to_string())
    }

    #[test]
    fn add_tiles_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        run(
            &mut t,
            r#"
            local tiles = {}
            tiles.VERSION = 1
            tiles["fencing_01_1"] = { self = { "d_1" }, left = { "d_2" }, right = { "d_0" } }
            tiles["fencing_01_2"] = { self = { "d_4" }, left = {}, right = { "d_3" } }
            BrokenFences.getInstance():addBrokenTiles(tiles)
            local debris = {}
            debris.VERSION = 1
            debris["fencing_01_1"] = { north = { "n_0" }, south = { "s_0" }, west = { "w_0" }, east = { "e_0" } }
            debris["fencing_01_2"] = debris["fencing_01_1"]
            BrokenFences.getInstance():addDebrisTiles(debris)
            assert(BrokenFences.getInstance():isBreakableSprite("d_3"))
            assert(not BrokenFences.getInstance():isBreakableSprite("d_4"))
        "#,
        )
        .unwrap();
        let b = shared_broken_fences();
        {
            let b = b.borrow();
            let t2 = &b.tiles[b.unbroken_map["fencing_01_2"]];
            assert_eq!(t2.left, None);
            assert_eq!(t2.debris_e.as_deref(), Some(&["e_0".to_string()][..]));
        }
        let e = run(&mut t, r#"BrokenFences.getInstance():addDebrisTiles({ x = { north = {"a"} } })"#).unwrap_err();
        assert!(e.contains("addDebrisTiles() with unknown tile"), "{e}");
    }
}
