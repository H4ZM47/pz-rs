//! Reads of world objects that `ISWorldObjectContextMenuLogic` makes and
//! that no other binding answers yet.

use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_world::objects::ObjectSprite;

use super::Cx;
use crate::bind_world_object::{borrowed, object_from_value, object_square, unsupported, LuaWorldObject, ISO_OBJECT};
use crate::expose::expose_class;

pub(super) fn register(thread: &mut Thread) {
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
    // getAttachedAnimSprite(): `attachedAnimSprite`.
    object.method("getAttachedAnimSprite", |c| {
        let this = c.arg(0);
        let object = receiver(&this)?;
        let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        let list = match &body.attached_anim_sprite {
            None => Value::Nil,
            Some(list) if list.is_empty() => crate::bind_java_util::list_value(Vec::new()),
            Some(_) => {
                return Err(unsupported("IsoObject.getAttachedAnimSprite: IsoSpriteInstance has no Lua object"))
            }
        };
        Ok(c.push(list))
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
