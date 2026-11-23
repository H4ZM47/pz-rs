//! Reads of world objects that `ISWorldObjectContextMenuLogic` makes and
//! that no other binding answers yet.

use std::any::Any;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, Thread, Value};

use super::Cx;
use crate::bind_world_object::{borrowed, object_from_value, object_square, LuaWorldObject, ISO_OBJECT};
use crate::expose::expose_class;

const SPRITE_INSTANCE: &str = "zombie.iso.sprite.IsoSpriteInstance";

/// An `IsoSpriteInstance` of an object's `attachedAnimSprite`, copied when
/// Lua reads the list.
struct SpriteInstance {
    sprite: pz_world::sprite::SpriteIdx,
}

impl LuaObject for SpriteInstance {
    fn class_name(&self) -> &str {
        SPRITE_INSTANCE
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(super) fn register(thread: &mut Thread) {
    let instance = expose_class(thread, SPRITE_INSTANCE, Some("java.lang.Object"));
    // getParentSprite(): `parentSprite`.
    instance.method("getParentSprite", |c| {
        let sprite = match c.arg(0) {
            Value::Object(o) => o.as_any().downcast_ref::<SpriteInstance>().map(|i| i.sprite),
            _ => None,
        };
        let sprite = sprite.ok_or_else(|| crate::bind_java_util::class_cast(&c.arg(0), SPRITE_INSTANCE))?;
        Ok(c.push(crate::bind_sprites::sprite_value(Some(sprite), false)))
    });
    let object = expose_class(thread, ISO_OBJECT, None);
    // getSquare(): `square`.
    object.method("getSquare", |c| {
        let this = c.arg(0);
        let square = object_square(receiver(&this)?)?;
        Ok(c.push(square))
    });
    // getProperties(): `sprite == null ? null : sprite.getProperties()`.
    object.method("getProperties", |c| {
        let this = c.arg(0);
        let body = receiver(&this)?.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        let props = crate::bind_sprites::object_sprite_value(&body.sprite, true)?;
        Ok(c.push(props))
    });
    object.method("getProperty", |c| {
        let this = c.arg(0);
        let mut cx = Cx { t: c.thread };
        let props = cx.call(&this, "getProperties", &[])?;
        let value = if props.is_nil() { Value::Nil } else {
            let key = c.arg(1);
            Cx { t: c.thread }.call(&props, "get", &[key])?
        };
        Ok(c.push(value))
    });
    // getAttachedAnimSprite(): `attachedAnimSprite`, as a copy.
    object.method("getAttachedAnimSprite", |c| {
        let this = c.arg(0);
        let object = receiver(&this)?;
        let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        let list = body.attached_anim_sprite.as_ref().map_or(Value::Nil, |list| {
            let items = list
                .iter()
                .map(|instance| Value::Object(Rc::new(SpriteInstance { sprite: instance.sprite })))
                .collect();
            crate::bind_java_util::list_value(items)
        });
        Ok(c.push(list))
    });
    // getTileName(): with a CustomName property, the moveable display name
    // of `GroupName CustomName` (or of `CustomName`); else getName().
    object.method("getTileName", |c| {
        let this = c.arg(0);
        let names = with_sprite(receiver(&this)?, |manager, sprite| {
            let props = &sprite?.properties;
            let custom = props.get(&manager.alias, "CustomName")?;
            Some(match props.get(&manager.alias, "GroupName") {
                Some(group) => format!("{group} {custom}"),
                None => custom.to_owned(),
            })
        })?;
        let r = match names {
            Some(name) => {
                let display = crate::bind_text::with_translator(|t| t.get_moveable_display_name(&name));
                Value::str(&display.unwrap_or(name))
            }
            None => Cx { t: c.thread }.call(&this, "getName", &[])?,
        };
        Ok(c.push(r))
    });
    object.method("getType", |c| {
        let this = c.arg(0);
        let value = with_sprite(receiver(&this)?, |manager, sprite| {
            let ty = sprite.map_or(manager.tables.object_type_max(), |sprite| sprite.tile_type);
            crate::expose::enum_value(crate::bind_iso_enums::ISO_OBJECT_TYPE, manager.tables.object_type_name(ty))
        })?;
        Ok(c.push(value))
    });
    // getSpriteName(): the sprite's name while `spriteName` is null, else
    // `spriteName`.
    object.method("getSpriteName", |c| {
        let this = c.arg(0);
        let object = receiver(&this)?;
        let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        let manager = crate::bind_sprites::manager()?;
        let manager = manager.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
        let name = match &body.sprite_name {
            Some(name) => crate::bind_world_object::resolve_sprite_name(name, &manager)?,
            None => body.sprite.try_read(&manager).map_err(crate::bind_world_object::host_error)?
                .and_then(|sprite| sprite.name.clone()),
        };
        let r = name.as_deref().map_or(Value::Nil, Value::str);
        Ok(c.push(r))
    });
    super::water::register(&object);
    super::fuel::register(&object);
}

pub(super) fn receiver(this: &Value) -> LuaResult<&LuaWorldObject> {
    match this {
        Value::Nil => Err(super::null_pointer()),
        v => object_from_value(v).ok_or_else(|| crate::bind_java_util::class_cast(v, ISO_OBJECT)),
    }
}

pub(super) fn with_sprite<R>(
    object: &LuaWorldObject,
    f: impl FnOnce(&pz_world::IsoSpriteManager, Option<&pz_world::sprite::IsoSprite>) -> R,
) -> LuaResult<R> {
    let manager = crate::bind_sprites::manager()?;
    let manager = manager.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
    let sprite = body.sprite.try_read(&manager).map_err(crate::bind_world_object::host_error)?;
    Ok(f(&manager, sprite.as_deref()))
}
