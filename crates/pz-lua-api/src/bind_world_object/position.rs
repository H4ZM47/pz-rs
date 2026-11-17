//! `IsoObject.getX/getY/getZ` and `isGrave`, which the world context menu
//! handlers read for each clicked object (`ISTrapMenu.doTrapMenu`,
//! `ISFarmingMenu.canDigHere`).

use super::{borrowed, object_from_value, unsupported, ISO_OBJECT};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_world::objects::ObjectSprite;
use pz_world::IsoObjectRef;

fn receiver(c: &NativeCall<'_>) -> LuaResult<IsoObjectRef> {
    object_from_value(&c.arg(0))
        .map(|o| o.object.clone())
        .ok_or_else(|| LuaError::runtime("expected IsoObject receiver"))
}

const SQUARE_GETTERS: [&str; 3] = ["getX", "getY", "getZ"];

/// `square.getX/Y/Z()` of the object's square, as a float. The square is
/// the object's native link. Without one, it is what the receiver's
/// `getSquare()` returns, because other bindings can know the square that
/// the object was read from. A null square throws a NullPointerException,
/// as in the original.
fn square_coord(c: &mut NativeCall<'_>, axis: usize) -> LuaResult<f64> {
    let getter = SQUARE_GETTERS[axis];
    let object = receiver(c)?;
    let linked = object
        .try_borrow()
        .map_err(|_| borrowed("IsoObject body"))?
        .square_slot()
        .map(|sq| {
            let sq = sq.borrow();
            [sq.x, sq.y, sq.z]
        });
    if let Some(xyz) = linked {
        return Ok(xyz[axis] as f32 as f64);
    }
    let this = c.arg(0);
    let get_square = c.thread.table_get(&this, &Value::str("getSquare"))?;
    let square = if get_square.is_nil() { Value::Nil } else { c.thread.call1(&get_square, &[this])? };
    if square.is_nil() {
        return Err(LuaError::Runtime {
            message: format!("Cannot invoke \"zombie.iso.IsoGridSquare.{getter}()\" because \"this.square\" is null"),
            class: "java.lang.NullPointerException",
        });
    }
    let f = c.thread.table_get(&square, &Value::str(getter))?;
    if f.is_nil() {
        return Err(LuaError::runtime(format!(
            "{}.{getter} is not bound",
            pz_kahlua::stdlib::java_class_name(&square)
        )));
    }
    let v = c.thread.call1(&f, &[square])?;
    let n = pz_kahlua::exec::raw_tonumber(&v).ok_or_else(|| LuaError::runtime(format!("IsoGridSquare.{getter} is not a number")))?;
    Ok(n as i32 as f32 as f64)
}

/// `isGrave()` for a tile sprite: a `CustomName` that contains "Grave" or
/// "grave", or a sprite name that contains "cemetary".
pub(crate) fn is_grave_sprite(custom_name: Option<&str>, sprite_name: Option<&str>) -> bool {
    if custom_name.is_some_and(|n| n.contains("Grave") || n.contains("grave")) {
        return true;
    }
    sprite_name.is_some_and(|n| n.contains("cemetary"))
}

fn is_grave(object: &IsoObjectRef) -> LuaResult<bool> {
    let body = object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
    let idx = match &body.sprite {
        ObjectSprite::None => return Ok(false),
        ObjectSprite::Tile(idx) => *idx,
        _ => return Err(unsupported("IsoObject.isGrave: the sprite has no Lua object")),
    };
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    let sprite = &sprites.sprites[idx];
    let custom = sprite.properties.get(&sprites.alias, "CustomName");
    Ok(is_grave_sprite(custom, sprite.name.as_deref()))
}

pub(super) fn register(thread: &mut Thread) {
    let object = crate::expose::expose_class(thread, ISO_OBJECT, None);
    object.method("getX", |c| {
        let x = square_coord(c, 0)?;
        Ok(c.push(Value::Num(x)))
    });
    object.method("getY", |c| {
        let y = square_coord(c, 1)?;
        Ok(c.push(Value::Num(y)))
    });
    object.method("getZ", |c| {
        let z = square_coord(c, 2)?;
        Ok(c.push(Value::Num(z)))
    });
    object.method("isGrave", |c| {
        let grave = is_grave(&receiver(c)?)?;
        Ok(c.push(Value::Bool(grave)))
    });
}

#[cfg(test)]
mod tests {
    use super::is_grave_sprite;
    use pz_kahlua::{stdlib, TableRef, Thread, Value};
    use pz_world::objects::ObjectKind;
    use pz_world::sprite::{IsoSpriteManager, PropertyValueMap, TilePropertyAliasMap};
    use pz_world::{IsoGridSquare, IsoObject, ObjectSlot};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn lua_reads_the_square_position_and_graves() {
        let old = crate::bind_sprites::manager().ok();
        let mut m = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let mut values = PropertyValueMap::default();
        values.put("CustomName", vec!["Grave".into(), "Bench".into()]);
        m.alias = TilePropertyAliasMap::generate(&values, &[]);
        let grave = m.add_sprite_named("grave_0");
        let bench = m.add_sprite_named("bench_0");
        m.sprites[grave].properties.set_raw(&m.alias, "CustomName", "Grave");
        m.sprites[bench].properties.set_raw(&m.alias, "CustomName", "Bench");
        crate::bind_sprites::install_sprite_manager(Some(Rc::new(RefCell::new(m))));
        let square = Rc::new(RefCell::new(IsoGridSquare::new(10, 20, 1)));
        let mut retired = pz_entity::RetiredComponents::new();
        let mut objects = Vec::new();
        for (id, sprite, on_square) in [(1, grave, true), (2, bench, true), (3, bench, false), (4, bench, false)] {
            let mut o = IsoObject::new(id, ObjectKind::Object, Some(sprite));
            if on_square {
                o.replace_square(Some(&square), &mut retired);
            }
            objects.push(ObjectSlot::new(o).ref_handle());
        }
        assert!(retired.is_empty());
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        super::super::register_classes(&mut t);
        // Another binding knows the square that object 4 was read from.
        let object = crate::expose::expose_class(&mut t, super::ISO_OBJECT, None);
        object.method("getSquare", |c| {
            let id = super::receiver(c)?.borrow().id;
            let square = if id == 4 { c.thread.globals().get_str("readFrom") } else { Value::Nil };
            Ok(c.push(square))
        });
        for (name, o) in ["grave", "bench", "loose", "read"].iter().zip(&objects) {
            super::super::ensure_class(o, super::ISO_OBJECT).unwrap();
            t.globals().set_str(name, super::super::object_value(o).unwrap());
        }
        let src = "readFrom = {getX = function() return 7 end, getY = function() return 8 end,
                getZ = function() return 0 end}
            return grave:getX(), grave:getY(), grave:getZ(), grave:isGrave(), bench:isGrave(),
            read:getX(), read:getY(), read:getZ(), pcall(loose.getX, loose)";
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), t.globals().clone()));
        let r = t.call(&f, &[]).unwrap_or_else(|e| panic!("{e}"));
        crate::bind_sprites::install_sprite_manager(old);
        let shown: Vec<String> = r.iter().map(pz_kahlua::exec::java_value_of).collect();
        assert_eq!(shown[..3], ["10.0", "20.0", "1.0"], "{shown:?}");
        assert!(r[3].truthy() && !r[4].truthy(), "{shown:?}");
        assert_eq!(shown[5..8], ["7.0", "8.0", "0.0"], "{shown:?}");
        assert!(!r[8].truthy() && shown[9].contains("this.square\" is null"), "{shown:?}");
    }

    #[test]
    fn graves_by_custom_name_or_cemetary_sprite() {
        assert!(is_grave_sprite(Some("Grave"), None));
        assert!(is_grave_sprite(Some("Empty grave"), Some("x")));
        assert!(!is_grave_sprite(Some("GRAVE"), Some("location_community_cemetery_01_0")));
        assert!(is_grave_sprite(None, Some("location_community_cemetary_01_0")));
        assert!(!is_grave_sprite(None, None));
    }
}
