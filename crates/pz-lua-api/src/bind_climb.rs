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

/// Archived unmanaged sprites have an ID but no property body.
fn sprite_has_either(object: &IsoObjectRef, flags: [&str; 2]) -> LuaResult<bool> {
    let body = object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
    if matches!(body.sprite, ObjectSprite::None | ObjectSprite::ArchiveUnmanaged(_)) {
        return Ok(false);
    }
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    let sprite = body.sprite.try_read(&sprites).map_err(crate::bind_world_object::host_error)?;
    Ok(sprite.is_some_and(|sprite| {
        flags.into_iter().filter_map(|name| sprites.tables.flag_from_string(name))
            .any(|flag| sprite.properties.has(flag))
    }))
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

struct QueryEngine(QueryRand, Vec<pz_world_states::ClosingWindow>);

impl pz_world_states::StatesEngine for QueryEngine {
    fn closing_windows(&self) -> &[pz_world_states::ClosingWindow] { &self.1 }
    fn rand(&mut self) -> &mut dyn pz_world::java::GameRand {
        &mut self.0
    }
}

/// Read-only character-state queries against the current world, including
/// the sprite properties and barricades on either side of a window.
fn with_query_world<T>(f: impl FnOnce(&pz_world_states::StatesWorldAdapter<'_>) -> LuaResult<T>) -> LuaResult<T> {
    let sprites = crate::bind_sprites::manager()?;
    let mut sprites = sprites.try_borrow_mut().map_err(|_| borrowed("IsoSpriteManager"))?;
    crate::bind_world::with_world_state_cell(|w| {
        let mut w = w.try_borrow_mut().map_err(|_| borrowed("IsoWorld"))?;
        let state = &mut *w;
        let (Some(cell), Some(meta)) = (state.cell.as_mut(), state.meta_grid.as_ref()) else {
            return Err(unsupported("window query without a world"));
        };
        let mut runtime = pz_world_states::StatesRuntime::default();
        let mut engine = QueryEngine(QueryRand, crate::bind_players::closing_windows());
        f(&pz_world_states::StatesWorldAdapter::new(
            cell, meta, &mut sprites, pz_world_states::ThumpSettings::default(), &mut runtime, &mut engine,
        ))
    })
}

/// Resolve a retained Lua receiver against the actual cell before translating
/// its identity into the coordinate and object ID used by character states.
pub(crate) fn with_live_object<T>(
    value: &Value,
    f: impl FnOnce(&pz_world_states::StatesWorldAdapter<'_>, &pz_world::IsoGridSquare, &pz_world::IsoObject,
        pz_pathfind::mover::ObjectRef) -> LuaResult<T>,
) -> LuaResult<Option<T>> {
    let world = crate::bind_world_object::object_from_value(value)
        .ok_or_else(|| crate::bind_java_util::class_cast(value, ISO_OBJECT))?;
    let Some(target) = crate::bind_world_object::object_ref(value)? else { return Ok(None) };
    with_query_world(|adapter| {
        let Some(square) = adapter.cell.get_grid_square(target.x, target.y, target.z) else { return Ok(None) };
        let square = square.try_borrow().map_err(|_| borrowed("IsoGridSquare"))?;
        if !square.objects.iter().any(|slot| std::rc::Rc::ptr_eq(&slot.ref_handle(), &world.object)) {
            return Ok(None);
        }
        let body = world.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        f(adapter, &square, &body, target).map(Some)
    })
}

fn window_actor(c: &NativeCall<'_>) -> LuaResult<Option<pz_world_states::Thumper>> {
    let value = c.arg(1);
    if matches!(value, Value::Nil) { return Ok(None) }
    let Value::Object(handle) = &value else {
        return Err(crate::bind_java_util::class_cast(&value, crate::bind_character_sim::ISO_GAME_CHARACTER_CLASS));
    };
    let actor = handle.as_any().downcast_ref::<crate::bind_character_sim::CharacterObject>()
        .ok_or_else(|| crate::bind_java_util::class_cast(&value, crate::bind_character_sim::ISO_GAME_CHARACTER_CLASS))?;
    let actor = actor.0.try_borrow().map_err(|_| borrowed("window actor"))?;
    Ok(Some(pz_world_states::Thumper::of(&actor)))
}

fn can_climb_through(c: &NativeCall<'_>) -> LuaResult<bool> {
    let this = c.arg(0);
    let chr = window_actor(c)?;
    Ok(with_live_object(&this, |adapter, square, body, _| {
        Ok(adapter.portal_can_climb_through(square, body, chr.as_ref()))
    })?.unwrap_or(false))
}

/// Toggle the pane unless it is permanently locked, smashed or barricaded
/// on the supplied actor's side. Dispatch after releasing world and sprite borrows.
/// TODO(port): alarms, pathfinder invalidation and local music intensity.
fn toggle_window(c: &mut NativeCall<'_>) -> LuaResult<()> {
    let this = c.arg(0);
    let world = crate::bind_world_object::object_from_value(&this)
        .ok_or_else(|| crate::bind_java_util::class_cast(&this, ISO_WINDOW))?;
    let square = crate::bind_world_object::object_square(world)?;
    let Some((x, y, z)) = crate::bind_world::square_coords(&square) else {
        return Ok(());
    };
    let actor = window_actor(c)?.map(|t| (t.x, t.y));
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    let changed = crate::bind_world::with_world_state_cell(|w| {
        let mut w = w.try_borrow_mut().map_err(|_| borrowed("IsoWorld"))?;
        let state = &mut *w;
        let (Some(cell), Some(meta)) = (state.cell.as_mut(), state.meta_grid.as_ref()) else {
            return Err(crate::bind_world_object::unsupported("IsoWindow.ToggleWindow without a world"));
        };
        pz_world::world_state::toggle_window(cell, meta, &sprites, (x, y, z), &world.object, actor)
            .map_err(crate::bind_world_object::host_error)
    })?;
    drop(sprites);
    if changed { crate::bind_character_sim::fire_container_update(c.thread); }
    Ok(())
}

pub(crate) fn register(thread: &mut Thread) {
    let object = expose_class(thread, ISO_OBJECT, None);
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
        let tall = sprite_has_either(&object, ["TallHoppableN", "TallHoppableW"])?;
        Ok(c.push(tall))
    });

    object.method("isWindow", |c| {
        let is_window = sprite_has_either(&receiver(c)?, ["WindowN", "WindowW"])?;
        Ok(c.push(is_window))
    });

    let window = expose_class(thread, ISO_WINDOW, Some(ISO_OBJECT));
    window.method("ToggleWindow", |c| {
        toggle_window(c)?;
        Ok(0)
    });
    for class in [ISO_WINDOW, "zombie.iso.objects.IsoThumpable", "zombie.iso.objects.IsoWindowFrame"] {
        let portal = expose_class(thread, class, Some(ISO_OBJECT));
        portal.method("canClimbThrough", |c| {
            let can = can_climb_through(c)?;
            Ok(c.push(can))
        });
        portal.method("getNorth", |c| {
            let object = receiver(c)?;
            let body = object.try_borrow().map_err(|_| borrowed("window or frame"))?;
            let north = match &body.kind {
                ObjectKind::Window(w) => w.north,
                ObjectKind::Thumpable(w) => w.north,
                ObjectKind::WindowFrame { north } => *north,
                _ => return Err(unsupported("getNorth on an object without a window edge")),
            };
            Ok(c.push(north))
        });
    }
    window.method("IsOpen", |c| Ok(c.push(window_field(c, |w| w.open)?)));
    window.method("isOpen", |c| Ok(c.push(window_field(c, |w| w.open)?)));
    window.method("isNorth", |c| Ok(c.push(window_field(c, |w| w.north)?)));
    window.method("isLocked", |c| Ok(c.push(window_field(c, |w| w.locked)?)));
    window.method("isPermaLocked", |c| Ok(c.push(window_field(c, |w| w.perma_locked)?)));
    window.method("isDestroyed", |c| Ok(c.push(window_field(c, |w| w.destroyed)?)));
    window.method("isSmashed", |c| Ok(c.push(window_field(c, |w| w.destroyed)?)));
    window.method("isGlassRemoved", |c| Ok(c.push(window_field(c, |w| w.glass_removed)?)));
}

#[cfg(test)]
mod tests;
