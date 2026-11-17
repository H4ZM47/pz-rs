//! The `IsoObject` and `IsoWindow` reads that the fence and window climbs
//! make from Lua (`ISClimbOverFence`, `ISClimbThroughWindow`,
//! `luautils.walkAdjFence`, `AdjacentFreeTileFinder.FindWindowOrDoor`).

use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_world::objects::{ObjectKind, ObjectSprite};
use pz_world::IsoObjectRef;

use crate::bind_world_object::{borrowed, unsupported, ISO_OBJECT};
use crate::expose::expose_class;

const ISO_WINDOW: &str = "zombie.iso.objects.IsoWindow";

fn receiver(c: &NativeCall<'_>) -> LuaResult<IsoObjectRef> {
    let this = c.arg(0);
    crate::bind_world_object::object_from_value(&this)
        .map(|o| o.object.clone())
        .ok_or_else(|| crate::bind_java_util::class_cast(&this, ISO_OBJECT))
}

/// The sprite's property flag named `flag` (`false` without a sprite).
fn sprite_has(object: &IsoObjectRef, flag: &str) -> LuaResult<bool> {
    let body = object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
    let ObjectSprite::Tile(idx) = &body.sprite else {
        return Ok(false);
    };
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    Ok(sprites
        .tables
        .flag_from_string(flag)
        .is_some_and(|f| sprites.sprites[*idx].properties.has(f)))
}

fn window_field(c: &NativeCall<'_>, f: fn(&pz_world::objects::IsoWindow) -> bool) -> LuaResult<bool> {
    let object = receiver(c)?;
    let body = object.try_borrow().map_err(|_| borrowed("IsoWindow body"))?;
    match &body.kind {
        ObjectKind::Window(w) => Ok(f(w)),
        _ => Err(unsupported("IsoWindow method on another object kind")),
    }
}

pub(crate) fn register(thread: &mut Thread) {
    let object = expose_class(thread, ISO_OBJECT, None);
    // `getProperties()`: the sprite's property container.
    object.method("getProperties", |c| {
        let object = receiver(c)?;
        let body = object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        match &body.sprite {
            ObjectSprite::None => Ok(c.push(Value::Nil)),
            ObjectSprite::Tile(idx) => Ok(c.push(crate::bind_sprites::sprite_value(Some(*idx), true))),
            _ => Err(unsupported("IsoObject.getProperties: the sprite has no Lua object")),
        }
    });
    // `isTallHoppable()`: `TallHoppableN` or `TallHoppableW` on the sprite.
    object.method("isTallHoppable", |c| {
        let object = receiver(c)?;
        let tall = sprite_has(&object, "TallHoppableN")? || sprite_has(&object, "TallHoppableW")?;
        Ok(c.push(tall))
    });

    let window = expose_class(thread, ISO_WINDOW, Some(ISO_OBJECT));
    window.method("IsOpen", |c| Ok(c.push(window_field(c, |w| w.open)?)));
    window.method("isOpen", |c| Ok(c.push(window_field(c, |w| w.open)?)));
    window.method("getNorth", |c| Ok(c.push(window_field(c, |w| w.north)?)));
    window.method("isNorth", |c| Ok(c.push(window_field(c, |w| w.north)?)));
    window.method("isLocked", |c| Ok(c.push(window_field(c, |w| w.locked)?)));
    window.method("isPermaLocked", |c| Ok(c.push(window_field(c, |w| w.perma_locked)?)));
    window.method("isDestroyed", |c| Ok(c.push(window_field(c, |w| w.destroyed)?)));
    window.method("isSmashed", |c| Ok(c.push(window_field(c, |w| w.destroyed)?)));
    window.method("isGlassRemoved", |c| Ok(c.push(window_field(c, |w| w.glass_removed)?)));
}
