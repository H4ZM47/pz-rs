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

/// The engine of a query-only states world: nothing it is asked for draws
/// a random number or changes the world.
struct QueryRand;

impl pz_world::java::GameRand for QueryRand {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

struct QueryEngine(QueryRand);

impl pz_world_states::StatesEngine for QueryEngine {
    fn rand(&mut self) -> &mut dyn pz_world::java::GameRand {
        &mut self.0
    }
}

/// `IsoWindow.canClimbThrough(chr)` of a window on the map, answered by the
/// character states' world over the current cell.
fn can_climb_through(c: &NativeCall<'_>) -> LuaResult<bool> {
    let this = c.arg(0);
    if crate::bind_world_object::object_from_value(&this).is_none() {
        return Err(crate::bind_java_util::class_cast(&this, ISO_WINDOW));
    }
    let Some(object) = crate::bind_world_object::object_ref(&this)? else {
        return Ok(false);
    };
    let thumper = match c.arg(1) {
        Value::Nil => return Err(crate::bind_world_object::unsupported("IsoWindow.canClimbThrough(null)")),
        Value::Object(o) => match o.as_any().downcast_ref::<crate::bind_character_sim::CharacterObject>() {
            Some(chr) => pz_world_states::Thumper::of(&chr.0.borrow()),
            None => return Err(unsupported("IsoWindow.canClimbThrough of a non-character")),
        },
        _ => return Err(unsupported("IsoWindow.canClimbThrough of a non-character")),
    };
    let sprites = crate::bind_sprites::manager()?;
    let mut sprites = sprites.try_borrow_mut().map_err(|_| borrowed("IsoSpriteManager"))?;
    crate::bind_world::with_world_state_cell(|w| {
        let mut w = w.try_borrow_mut().map_err(|_| borrowed("IsoWorld"))?;
        let state = &mut *w;
        let (Some(cell), Some(meta)) = (state.cell.as_mut(), state.meta_grid.as_ref()) else {
            return Err(unsupported("IsoWindow.canClimbThrough without a world"));
        };
        let mut runtime = pz_world_states::StatesRuntime::default();
        let mut engine = QueryEngine(QueryRand);
        let adapter = pz_world_states::StatesWorldAdapter::new(
            cell,
            meta,
            &mut sprites,
            pz_world_states::ThumpSettings::default(),
            &mut runtime,
            &mut engine,
        );
        Ok(adapter.window_climb_for(&thumper, object).is_some_and(|climb| climb.portal_can_climb))
    })
}

/// `IsoWindow.ToggleWindow(chr)` of a window on the map: a permanently locked,
/// smashed or barricaded window stays as it is; otherwise it unlocks, flips
/// `open` and takes its open or closed sprite, and the square and its
/// neighbours recalculate.
///
/// TODO(port): the house alarm of `handleAlarm`, the `OnContainerUpdate`
/// event, `PolygonalMap2.squareChanged` and the music intensity event.
fn toggle_window(c: &NativeCall<'_>) -> LuaResult<()> {
    let this = c.arg(0);
    let world = crate::bind_world_object::object_from_value(&this)
        .ok_or_else(|| crate::bind_java_util::class_cast(&this, ISO_WINDOW))?;
    let square = crate::bind_world_object::object_square(world)?;
    let Some((x, y, z)) = crate::bind_world::square_coords(&square) else {
        return Ok(());
    };
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    crate::bind_world::with_world_state_cell(|w| {
        let mut w = w.try_borrow_mut().map_err(|_| borrowed("IsoWorld"))?;
        let state = &mut *w;
        let (Some(cell), Some(meta)) = (state.cell.as_mut(), state.meta_grid.as_ref()) else {
            return Err(crate::bind_world_object::unsupported("IsoWindow.ToggleWindow without a world"));
        };
        let Some(square) = cell.get_grid_square(x, y, z) else {
            return Ok(());
        };
        let (skip, sprite, native_class) = {
            let sq = square.try_borrow().map_err(|_| borrowed("IsoGridSquare"))?;
            let body = world.object.try_borrow().map_err(|_| borrowed("IsoWindow body"))?;
            let ObjectKind::Window(w) = &body.kind else {
                return Err(unsupported("IsoWindow.ToggleWindow on another object kind"));
            };
            let hooks = pz_world::world_state::ObjectHooks::new(cell, &sprites);
            let skip = w.perma_locked || w.destroyed || hooks.object_barricaded(&sq, &body);
            let sprite = if w.open { w.closed_sprite } else { w.open_sprite };
            (skip, sprite, body.kind.native_class_name())
        };
        if skip {
            return Ok(());
        }
        let old = {
            let mut body = world.object.try_borrow_mut().map_err(|_| borrowed("IsoWindow body"))?;
            if let ObjectKind::Window(w) = &mut body.kind {
                w.locked = false;
                w.open = !w.open;
            }
            std::mem::replace(&mut body.sprite, sprite.into())
        };
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_payload(Box::new(old));
        cell.retire_components(retired);
        pz_world::object_slot::observe_object(&world.object, native_class);
        cell.recalc_all_with_neighbours(x, y, z, meta, &sprites);
        Ok(())
    })
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
    // `getObjectIndex()`: the position in the square's object list, `-1`
    // when the object is off the map.
    object.method("getObjectIndex", |c| {
        let this = c.arg(0);
        let world = crate::bind_world_object::object_from_value(&this)
            .ok_or_else(|| crate::bind_java_util::class_cast(&this, ISO_OBJECT))?;
        let square = crate::bind_world_object::object_square(world)?;
        let Some((x, y, z)) = crate::bind_world::square_coords(&square) else {
            return Ok(c.push(-1));
        };
        let index = crate::bind_world::try_with_world_read(|w| {
            let Some(square) = w.cell.as_ref().and_then(|cell| cell.get_grid_square(x, y, z)) else {
                return Ok(-1);
            };
            let square = square
                .try_borrow()
                .map_err(|_| pz_entity::runtime_load::RuntimeHostError::Borrowed("IsoGridSquare"))?;
            Ok(square
                .objects
                .iter()
                .position(|slot| std::rc::Rc::ptr_eq(&slot.ref_handle(), &world.object))
                .map_or(-1, |i| i as i32))
        })
        .map_err(crate::bind_world_object::host_error)?;
        Ok(c.push(index))
    });
    // `isTallHoppable()`: `TallHoppableN` or `TallHoppableW` on the sprite.
    object.method("isTallHoppable", |c| {
        let object = receiver(c)?;
        let tall = sprite_has(&object, "TallHoppableN")? || sprite_has(&object, "TallHoppableW")?;
        Ok(c.push(tall))
    });

    let window = expose_class(thread, ISO_WINDOW, Some(ISO_OBJECT));
    window.method("ToggleWindow", |c| {
        toggle_window(c)?;
        Ok(0)
    });
    window.method("canClimbThrough", |c| {
        let can = can_climb_through(c)?;
        Ok(c.push(can))
    });
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

#[cfg(test)]
mod tests;
