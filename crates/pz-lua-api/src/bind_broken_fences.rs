//! `zombie.iso.BrokenFences` as `media/lua/server/Items/BrokenFences.lua`
//! and `DebugContextMenu.lua` use it: `addBrokenTiles`, `addDebrisTiles`,
//! `isBreakableSprite`, `isBreakableObject`, `getBreakableObject`,
//! `setDestroyed`, `setDamagedLeft`, `setDamagedRight`, `updateSprite`,
//! `destroyFence`, `addItems`, and `Reset`.

use crate::bind_java_util::class_cast;
use crate::expose::{bool_arg, expose_class, int_arg, str_arg, JavaEnum};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::IsoDirections;
use pz_world_states::{
    add_debris_items_in_cell, breakable_object_on_square, destroy_fence_in_cell, object_materials,
    set_fence_damaged_left_in_cell, set_fence_damaged_right_in_cell, set_fence_destroyed_in_cell,
    update_fence_sprite_in_cell, BrokenFences, BrokenTile, FenceRand, FenceWorldHooks,
};
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
    b.method("isBreakableObject", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let arg = c.arg(1);
        let Some(obj) = crate::bind_world_object::object_from_value(&arg) else {
            return Ok(c.push(false));
        };
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites.borrow();
        let body = obj
            .object
            .try_borrow()
            .map_err(|_| crate::bind_world_object::borrowed("IsoObject body"))?;
        let name = body
            .sprite
            .try_read(&sprites)
            .map_err(crate::bind_world_object::host_error)?
            .and_then(|s| s.name.clone());
        let r = INSTANCE.with(|i| i.borrow().is_breakable_sprite(name.as_deref()));
        Ok(c.push(r))
    });
    b.method("getBreakableObject", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let Some((x, y, z)) = crate::bind_world::square_coords(&c.arg(1)) else {
            return Ok(c.push(Value::Nil));
        };
        let north = bool_arg(c, 2);
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites.borrow();
        let fences = INSTANCE.with(|i| i.borrow().clone());
        let found = crate::bind_world::with_cell_square(x, y, z, |sq| {
            breakable_object_on_square(&fences, sq, &sprites, north)
        })
        .flatten();
        match found {
            Some(obj) => Ok(c.push(crate::bind_world_object::object_value(&obj)?)),
            None => Ok(c.push(Value::Nil)),
        }
    });
    b.method("setDestroyed", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let Some(r) = crate::bind_world_object::object_ref(&c.arg(1))? else {
            return Ok(0);
        };
        let fences = INSTANCE.with(|i| i.borrow().clone());
        let sprites = crate::bind_sprites::manager()?;
        let mut sprites = sprites.borrow_mut();
        crate::bind_world::with_world(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_mut(), w.meta_grid.as_ref()) else {
                return;
            };
            let mut hooks = LuaFenceHooks::new(&w.geometry);
            set_fence_destroyed_in_cell(&fences, cell, meta, &mut sprites, r, &mut hooks);
        });
        Ok(0)
    });
    b.method("setDamagedLeft", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let Some(r) = crate::bind_world_object::object_ref(&c.arg(1))? else {
            return Ok(0);
        };
        let fences = INSTANCE.with(|i| i.borrow().clone());
        let sprites = crate::bind_sprites::manager()?;
        let mut sprites = sprites.borrow_mut();
        crate::bind_world::with_world(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_mut(), w.meta_grid.as_ref()) else {
                return;
            };
            let mut hooks = LuaFenceHooks::new(&w.geometry);
            set_fence_damaged_left_in_cell(&fences, cell, meta, &mut sprites, r, &mut hooks);
        });
        Ok(0)
    });
    b.method("setDamagedRight", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let Some(r) = crate::bind_world_object::object_ref(&c.arg(1))? else {
            return Ok(0);
        };
        let fences = INSTANCE.with(|i| i.borrow().clone());
        let sprites = crate::bind_sprites::manager()?;
        let mut sprites = sprites.borrow_mut();
        crate::bind_world::with_world(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_mut(), w.meta_grid.as_ref()) else {
                return;
            };
            let mut hooks = LuaFenceHooks::new(&w.geometry);
            set_fence_damaged_right_in_cell(&fences, cell, meta, &mut sprites, r, &mut hooks);
        });
        Ok(0)
    });
    b.method("updateSprite", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let Some(r) = crate::bind_world_object::object_ref(&c.arg(1))? else {
            return Ok(0);
        };
        let broken_left = bool_arg(c, 2);
        let broken_right = bool_arg(c, 3);
        let fences = INSTANCE.with(|i| i.borrow().clone());
        let sprites = crate::bind_sprites::manager()?;
        let mut sprites = sprites.borrow_mut();
        crate::bind_world::with_world(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_mut(), w.meta_grid.as_ref()) else {
                return;
            };
            let mut hooks = LuaFenceHooks::new(&w.geometry);
            update_fence_sprite_in_cell(
                &fences,
                cell,
                meta,
                &mut sprites,
                r,
                broken_left,
                broken_right,
                &mut hooks,
            );
        });
        Ok(0)
    });
    b.method("destroyFence", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let Some(r) = crate::bind_world_object::object_ref(&c.arg(1))? else {
            return Ok(0);
        };
        let dir = parse_direction(c, 2)?;
        let fences = INSTANCE.with(|i| i.borrow().clone());
        let sprites = crate::bind_sprites::manager()?;
        let drops = {
            let mut sprites = sprites.borrow_mut();
            let mut drops = Vec::new();
            let err = crate::bind_world::with_world(|w| {
                let (Some(cell), Some(meta)) = (w.cell.as_mut(), w.meta_grid.as_ref()) else {
                    return Ok(());
                };
                let mut hooks = LuaFenceHooks::new(&w.geometry);
                destroy_fence_in_cell(&fences, cell, meta, &mut sprites, r, dir, &mut hooks)?;
                drops = hooks.drops;
                Ok(())
            });
            err.map_err(|msg| java_err("java.lang.IllegalArgumentException", msg))?;
            drops
        };
        drop_lua_debris_items(c.thread, drops)?;
        Ok(0)
    });
    b.method("addItems", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        let arg = c.arg(1);
        let Some(obj) = crate::bind_world_object::object_from_value(&arg) else {
            return Ok(0);
        };
        let Some(sq_pos) = crate::bind_world::square_coords(&c.arg(2)) else {
            return Ok(0);
        };
        let sprites = crate::bind_sprites::manager()?;
        let materials = {
            let sprites = sprites.borrow();
            let body = obj
                .object
                .try_borrow()
                .map_err(|_| crate::bind_world_object::borrowed("IsoObject body"))?;
            object_materials(&body, &sprites)
        };
        let drops = {
            let mut sprites = sprites.borrow_mut();
            let mut drops = Vec::new();
            crate::bind_world::with_world(|w| {
                let (Some(cell), Some(meta)) = (w.cell.as_mut(), w.meta_grid.as_ref()) else {
                    return;
                };
                let mut hooks = LuaFenceHooks::new(&w.geometry);
                add_debris_items_in_cell(cell, meta, &mut sprites, sq_pos, materials, &mut hooks);
                drops = hooks.drops;
            });
            drops
        };
        drop_lua_debris_items(c.thread, drops)?;
        Ok(0)
    });
    b.method("Reset", |c| {
        crate::expose::this::<LuaBrokenFences>(c)?;
        INSTANCE.with(|i| i.borrow_mut().reset());
        Ok(0)
    });
}

fn parse_direction(c: &NativeCall<'_>, i: usize) -> LuaResult<IsoDirections> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_iso_enums::ISO_DIRECTIONS)
            .and_then(|e| IsoDirections::from_name(e.name))
            .ok_or_else(|| LuaError::runtime("Expected an IsoDirections")),
        Value::Num(_) => Ok(IsoDirections::ALL[(int_arg(c, i) & 7) as usize]),
        Value::Nil => Err(java_err("java.lang.NullPointerException", "")),
        _ => Err(LuaError::runtime("Expected an IsoDirections")),
    }
}

struct LuaFenceHooks<'a> {
    geometry: &'a crate::bind_world::GeometryChanges,
    drops: Vec<((i32, i32, i32), String, f32, f32)>,
}

impl<'a> LuaFenceHooks<'a> {
    fn new(geometry: &'a crate::bind_world::GeometryChanges) -> Self {
        Self {
            geometry,
            drops: Vec::new(),
        }
    }
}

impl FenceRand for LuaFenceHooks<'_> {
    fn next(&mut self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
}

impl FenceWorldHooks for LuaFenceHooks<'_> {
    fn spawn_debris_item(
        &mut self,
        _cell: &mut pz_world::IsoCell,
        _meta: &pz_world::IsoMetaGrid,
        _sprites: &mut pz_world::IsoSpriteManager,
        sq: (i32, i32, i32),
        item_type: &str,
        x: f32,
        y: f32,
    ) {
        self.drops.push((sq, item_type.to_string(), x, y));
    }

    fn geometry_changed(
        &mut self,
        cell: &pz_world::IsoCell,
        meta: &pz_world::IsoMetaGrid,
        sprites: &pz_world::IsoSpriteManager,
        pos: (i32, i32, i32),
    ) {
        self.geometry.notify(cell, meta, sprites, pos);
    }
}

fn drop_lua_debris_items(
    thread: &mut Thread,
    drops: Vec<((i32, i32, i32), String, f32, f32)>,
) -> LuaResult<()> {
    for ((x, y, z), item_type, ox, oy) in drops {
        let Some(item) = crate::bind_items::create_item(&item_type, 1.0) else {
            continue;
        };
        let sq = crate::bind_world::square_value(x, y, z);
        if let Value::Table(mt) = thread.metatable(&sq)? {
            let method = match mt.get_str("__index") {
                Value::Table(idx) => idx.get_str("AddWorldInventoryItem"),
                _ => Value::Nil,
            };
            if !method.is_nil() {
                let item_val = crate::bind_items::item_value(&item);
                let _ = thread.call(
                    &method,
                    &[
                        sq,
                        item_val,
                        Value::Num(ox as f64),
                        Value::Num(oy as f64),
                        Value::Num(0.0),
                    ],
                )?;
            }
        }
    }
    Ok(())
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

    #[test]
    fn lua_world_object_methods_inspect_and_destroy_fence() {
        use pz_world::chunk::IsoChunk;
        use pz_world::objects::{IsoObject, ObjectKind};
        use pz_world::{IsoCell, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, ObjectSlot, SquareSlot};

        let mut sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let add_named = |sm: &mut IsoSpriteManager, name: &str| {
            let idx = sm.add_sprite_named(name);
            sm.sprites[idx].name = Some(name.to_string());
            idx
        };
        let floor_idx = add_named(&mut sprites, "floors_0");
        let fence_idx = add_named(&mut sprites, "fencing_01_1");
        let _d1 = add_named(&mut sprites, "d_1");
        let _d2 = add_named(&mut sprites, "d_2");
        let _deb_s = add_named(&mut sprites, "s_0");
        let solidfloor = sprites.tables.flag_from_string("solidfloor").unwrap();
        let collide_n = sprites.tables.flag_from_string("collideN").unwrap();
        sprites.sprites[floor_idx].properties.set_flag(solidfloor);
        sprites.sprites[fence_idx].properties.set_flag(collide_n);

        let mut cell = IsoCell::default();
        let mut ch = IsoChunk::new(1, 1);
        let mut sq = IsoGridSquare::new(10, 10, 0);
        let fid = sq.new_object_id();
        sq.objects
            .push(ObjectSlot::new(IsoObject::new(fid, ObjectKind::Object, Some(floor_idx))));
        let id = sq.new_object_id();
        let slot = ObjectSlot::new(IsoObject::new(id, ObjectKind::Object, Some(fence_idx)));
        let fence_ref = slot.ref_handle();
        sq.objects.push(slot);
        sq.recalc_properties(&sprites);
        ch.set_square(2, 2, 0, Some(SquareSlot::new(sq)));
        cell.chunks.insert((1, 1), ch);

        let sprites_rc = Rc::new(RefCell::new(sprites));
        crate::bind_sprites::install_sprite_manager(Some(sprites_rc.clone()));
        crate::bind_world::set_meta_grid(Some(IsoMetaGrid::empty()));
        crate::bind_world::set_cell(Some(cell));

        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_world::register(&mut t);
        crate::bind_sprites::register(&mut t);
        register(&mut t);
        crate::bind_world_object::remember_class(&fence_ref, "zombie.iso.IsoObject").unwrap();
        let fence_val = crate::bind_world_object::object_value(&fence_ref).unwrap();
        crate::bind_world_object::remember_square(&fence_val, [10, 10, 0]);
        t.globals().set_str("test_fence", fence_val);
        t.globals().set_str("test_sq", crate::bind_world::square_value(10, 10, 0));

        run(
            &mut t,
            r#"
            local bf = BrokenFences.getInstance()
            bf:Reset()
            bf:addBrokenTiles({
                VERSION = 1,
                fencing_01_1 = { self = { "d_1" }, left = { "d_2" }, right = { "d_0" } }
            })
            bf:addDebrisTiles({
                VERSION = 1,
                fencing_01_1 = { north = { "n_0" }, south = { "s_0" }, west = { "w_0" }, east = { "e_0" } }
            })
            assert(bf:isBreakableObject(test_fence) == true)
            local found = bf:getBreakableObject(test_sq, true)
            assert(found ~= nil)
            assert(bf:getBreakableObject(test_sq, false) == nil)
            bf:destroyFence(test_fence, 4) -- IsoDirections.S
            assert(bf:isBreakableObject(test_fence) == false)
        "#,
        )
        .unwrap();

        let names: Vec<Option<String>> = crate::bind_world::with_cell_square(10, 10, 0, |sq| {
            let sm = sprites_rc.borrow();
            sq.objects
                .iter()
                .map(|slot| {
                    slot.borrow()
                        .sprite
                        .try_read(&sm)
                        .unwrap()
                        .and_then(|s| s.name.clone())
                })
                .collect()
        })
        .unwrap();
        assert!(names.contains(&Some("d_1".into())), "names={names:?}");
        assert!(names.contains(&Some("s_0".into())), "names={names:?}");

        crate::bind_world::set_cell(None);
        crate::bind_world::set_meta_grid(None);
        crate::bind_sprites::install_sprite_manager(None);
    }
}
