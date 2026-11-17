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

/// The object's square position: `square.getX/Y/Z()` as a float. A null
/// square throws a NullPointerException, as in the original.
fn square_position(object: &IsoObjectRef) -> LuaResult<(i32, i32, i32)> {
    let body = object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
    let square = body.square_slot().ok_or_else(|| LuaError::Runtime {
        message: "Cannot invoke \"zombie.iso.IsoGridSquare.getX()\" because \"this.square\" is null".into(),
        class: "java.lang.NullPointerException",
    })?;
    let square = square.borrow();
    Ok((square.x, square.y, square.z))
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
        let (x, _, _) = square_position(&receiver(c)?)?;
        Ok(c.push(Value::Num(x as f32 as f64)))
    });
    object.method("getY", |c| {
        let (_, y, _) = square_position(&receiver(c)?)?;
        Ok(c.push(Value::Num(y as f32 as f64)))
    });
    object.method("getZ", |c| {
        let (_, _, z) = square_position(&receiver(c)?)?;
        Ok(c.push(Value::Num(z as f32 as f64)))
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
        for (id, sprite, on_square) in [(1, grave, true), (2, bench, true), (3, bench, false)] {
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
        for (name, o) in ["grave", "bench", "loose"].iter().zip(&objects) {
            super::super::ensure_class(o, super::ISO_OBJECT).unwrap();
            t.globals().set_str(name, super::super::object_value(o).unwrap());
        }
        let src = "return grave:getX(), grave:getY(), grave:getZ(), grave:isGrave(), bench:isGrave(),
            pcall(loose.getX, loose)";
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), t.globals().clone()));
        let r = t.call(&f, &[]).unwrap_or_else(|e| panic!("{e}"));
        crate::bind_sprites::install_sprite_manager(old);
        let shown: Vec<String> = r.iter().map(pz_kahlua::exec::java_value_of).collect();
        assert_eq!(shown[..3], ["10.0", "20.0", "1.0"], "{shown:?}");
        assert!(r[3].truthy() && !r[4].truthy() && !r[5].truthy(), "{shown:?}");
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
