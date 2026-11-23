//! Reads of world objects that `ISWorldObjectContextMenuLogic` makes and
//! that no other binding answers yet.

use std::any::Any;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_world::objects::ObjectSprite;

use super::Cx;
use crate::bind_world_object::{borrowed, object_from_value, object_square, unsupported, LuaWorldObject, ISO_OBJECT};
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
        let sprite = tile_sprite(receiver(&this)?, "IsoObject.getProperties")?;
        Ok(c.push(crate::bind_sprites::sprite_value(sprite, true)))
    });
    // getProperty(String): `getProperties()`, then its `get(key)`; null
    // without a sprite.
    object.method("getProperty", |c| {
        let this = c.arg(0);
        let sprite = tile_sprite(receiver(&this)?, "IsoObject.getProperty")?;
        let props = crate::bind_sprites::sprite_value(sprite, true);
        if props.is_nil() {
            return Ok(c.push(Value::Nil));
        }
        let get = c.thread.table_get(&props, &Value::str("get"))?;
        let v = c.thread.call1(&get, &[props, c.arg(1)])?;
        Ok(c.push(v))
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
        let names = match tile_sprite(receiver(&this)?, "IsoObject.getTileName")? {
            None => None,
            Some(sprite) => {
                let m = crate::bind_sprites::manager()?;
                let m = m.try_borrow().map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
                let props = &m.sprites[sprite].properties;
                props.get(&m.alias, "CustomName").map(|custom| {
                    match props.get(&m.alias, "GroupName") {
                        Some(group) => format!("{group} {custom}"),
                        None => custom.to_string(),
                    }
                })
            }
        };
        let r = match names {
            Some(name) => {
                let display = crate::bind_text::with_translator(|t| t.get_moveable_display_name(&name));
                Value::str(&display.unwrap_or(name))
            }
            None => Cx { t: c.thread }.call(&this, "getName", &[])?,
        };
        Ok(c.push(r))
    });
    // getSpriteName(): the sprite's name while `spriteName` is null, else
    // `spriteName`.
    object.method("getSpriteName", |c| {
        let this = c.arg(0);
        let object = receiver(&this)?;
        let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        if body.sprite_name.is_none() {
            if let ObjectSprite::Tile(sprite) = body.sprite {
                let m = crate::bind_sprites::manager()?;
                let m = m.try_borrow().map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
                if let Some(name) = &m.sprites[sprite].name {
                    return Ok(c.push(Value::str(name)));
                }
            } else if !matches!(body.sprite, ObjectSprite::None) {
                return Err(unsupported("IsoObject.getSpriteName: the sprite has no Lua object"));
            }
        }
        let r = match &body.sprite_name {
            None => Value::Nil,
            Some(pz_world::objects::SpriteName::Str(name)) => Value::str(name),
            Some(pz_world::objects::SpriteName::Dict(_)) => {
                return Err(unsupported("IsoObject.getSpriteName: a spriteName from the save dictionary"))
            }
        };
        Ok(c.push(r))
    });
    object.method("hasFluid", |c| {
        let this = c.arg(0);
        let r = has_fluid(c, &this)?;
        Ok(c.push(r))
    });
    object.method("getPipedFuelAmount", |c| {
        let this = c.arg(0);
        let r = piped_fuel_amount(c, &this)?;
        Ok(c.push(r))
    });
}

fn receiver(this: &Value) -> LuaResult<&LuaWorldObject> {
    match this {
        Value::Nil => Err(super::null_pointer()),
        v => object_from_value(v).ok_or_else(|| crate::bind_java_util::class_cast(v, ISO_OBJECT)),
    }
}

/// The object's tile sprite; `None` for no sprite.
fn tile_sprite(object: &LuaWorldObject, what: &str) -> LuaResult<Option<pz_world::sprite::SpriteIdx>> {
    let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
    match &body.sprite {
        ObjectSprite::None => Ok(None),
        ObjectSprite::Tile(idx) => Ok(Some(*idx)),
        _ => Err(unsupported(format!("{what}: the sprite has no Lua object"))),
    }
}

/// `hasFluid()`: a solid floor on a square with more than 0.09 of
/// puddles (`IsoGridSquare.getPuddlesInGround`).
fn has_fluid(c: &mut NativeCall<'_>, this: &Value) -> LuaResult<bool> {
    let object = receiver(this)?;
    let square = object_square(object)?;
    if square.is_nil() {
        return Ok(false);
    }
    let Some(sprite) = tile_sprite(object, "IsoObject.hasFluid")? else {
        return Ok(false);
    };
    let solid_floor = {
        let m = crate::bind_sprites::manager()?;
        let m = m.try_borrow().map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        m.sprites[sprite].properties.has(m.tables.flag.solidfloor)
    };
    if !solid_floor {
        return Ok(false);
    }
    Ok(puddles_in_ground(&mut Cx { t: c.thread }, &square)? > 0.09)
}

/// `IsoGridSquare.getPuddlesInGround()` as far as it answers without
/// puddles: -1 in a room, else `IsoPuddlesCompute.computePuddle`, which
/// gives -0.1 without shaders, with puddles off, above the ground with
/// puddles on, or while `IsoPuddles.getPuddlesSize()` is 0. The cache of
/// the original only keeps the last computed level.
fn puddles_in_ground(cx: &mut Cx<'_>, square: &Value) -> LuaResult<f32> {
    let puddles_size = crate::bind_puddles::shared_puddles()
        .try_borrow()
        .map_err(|_| LuaError::runtime("IsoPuddles is in use"))?
        .state
        .puddles_size
        .get();
    if puddles_size <= 0.0 {
        // Both the room answer (-1) and every early answer of
        // computePuddle (-0.1) are below any threshold the callers use;
        // with no puddles computePuddle answers -0.1 too.
        return Ok(-0.1);
    }
    if !cx.call(square, "getRoom", &[])?.is_nil() {
        return Ok(-1.0);
    }
    let core = cx.global("getCore", &[])?;
    if !cx.is(&core, "getUseShaders", &[])? {
        return Ok(-0.1);
    }
    if cx.num(&core, "getPerfPuddlesOnLoad", &[])? == 3.0 || cx.num(&core, "getPerfPuddles", &[])? == 3.0 {
        return Ok(-0.1);
    }
    if cx.num(&core, "getPerfPuddles", &[])? > 0.0 && cx.num(square, "getZ", &[])? > 0.0 {
        return Ok(-0.1);
    }
    Err(unsupported(
        "IsoGridSquare.getPuddlesInGround: IsoGridSquare.getIsoWorldRegion and IsoPuddlesCompute.computePuddle",
    ))
}

/// `getPipedFuelAmount()`: the mod data's `fuelAmount`. A fuel pump
/// (`fuelAmount` property) fills it on first use from the sandbox options.
fn piped_fuel_amount(c: &mut NativeCall<'_>, this: &Value) -> LuaResult<i32> {
    let object = receiver(this)?;
    let Some(sprite) = tile_sprite(object, "IsoObject.getPipedFuelAmount")? else {
        return Ok(0);
    };
    let mut cx = Cx { t: c.thread };
    let mut fuel = 0.0;
    if cx.is(this, "hasModData", &[])? {
        if let Value::Table(data) = cx.call(this, "getModData", &[])? {
            if !data.is_empty() {
                let v = data.get_str("fuelAmount");
                if !v.is_nil() {
                    fuel = v.as_num().ok_or_else(|| crate::bind_java_util::class_cast(&v, "java.lang.Double"))?;
                }
            }
        }
    }
    let pump = {
        let m = crate::bind_sprites::manager()?;
        let m = m.try_borrow().map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        m.sprites[sprite].properties.has_name(&m.alias, "fuelAmount")
    };
    if pump {
        return Err(unsupported("IsoObject.getPipedFuelAmount: fuel pumps"));
    }
    Ok(fuel as i32)
}
