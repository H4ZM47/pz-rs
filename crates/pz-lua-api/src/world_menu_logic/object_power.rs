//! ItemContainer.isObjectPowered and the per-object, per-game-tick result.
use super::{sprite_grid, Cx};
use crate::bind_world_object::{
    borrowed, host_error, object_from_value, object_square, ISO_OBJECT,
};
use pz_kahlua::{LuaResult, Thread, Value};
use pz_world::objects::ObjectKind;

fn switch_active(object: &Value) -> LuaResult<Option<bool>> {
    let object = object_from_value(object).ok_or_else(super::null_pointer)?;
    let body = object
        .object
        .try_borrow()
        .map_err(|_| borrowed("IsoObject"))?;
    Ok(match &body.kind {
        ObjectKind::LightSwitch(switch) => Some(switch.activated),
        _ => None,
    })
}

fn square_powered(cx: &mut Cx<'_>, square: &Value, generators: bool) -> LuaResult<bool> {
    if square.is_nil() {
        return Ok(false);
    }
    let hydro = cx.is(square, "hasGridPower", &[])?;
    if hydro && !cx.call(square, "getRoom", &[])?.is_nil() {
        return Ok(true);
    }
    if generators && cx.is(square, "haveElectricity", &[])? {
        return Ok(true);
    }
    if hydro {
        for direction in ["N", "S", "W", "E"] {
            let direction = cx.static_field("IsoDirections", direction);
            let near = cx.call(square, "getAdjacentSquare", &[direction])?;
            if !near.is_nil() && !cx.call(&near, "getRoom", &[])?.is_nil() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn object_powered(cx: &mut Cx<'_>, this: &Value, generators: bool) -> LuaResult<bool> {
    if this.is_nil() || cx.num(this, "getObjectIndex", &[])? == -1.0 {
        return Ok(false);
    }
    let mut objects = sprite_grid::objects(this, false)?;
    if objects.is_empty() {
        objects.push(this.clone());
    }
    let mut switch = None;
    for object in &objects {
        switch = switch_active(object)?;
        if switch.is_some() {
            break;
        }
    }
    let manager = crate::bind_sprites::manager()?;
    for object in &objects {
        let receiver = object_from_value(object).ok_or_else(super::null_pointer)?;
        let street_light = {
            let manager = manager
                .try_borrow()
                .map_err(|_| borrowed("IsoSpriteManager"))?;
            let body = receiver
                .object
                .try_borrow()
                .map_err(|_| borrowed("IsoObject"))?;
            let sprite = body.sprite.try_read(&manager).map_err(host_error)?;
            sprite.is_some_and(|sprite| sprite.properties.has_name(&manager.alias, "streetlight"))
        };
        if street_light {
            return Ok(switch != Some(false)
                && crate::bind_climate::shared_game_time().borrow().get_night() >= 0.5
                && cx.is(object, "hasGridPower", &[])?);
        }
        let square = object_square(receiver)?;
        if cx.num(object, "getPipedFuelAmount", &[])? > 0.0 {
            return Ok((generators && cx.is(&square, "haveElectricity", &[])?)
                || cx.is(&square, "hasGridPower", &[])?);
        }
        if square_powered(cx, &square, generators)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn register(thread: &mut Thread) {
    let container = crate::expose::expose_class(
        thread,
        crate::bind_item_container::ITEM_CONTAINER_CLASS,
        None,
    );
    container.static_fn("isObjectPowered", |c| {
        let object = c.arg(0);
        let generators = crate::expose::bool_arg(c, 1);
        let on = object_powered(&mut Cx { t: c.thread }, &object, generators)?;
        Ok(c.push(on))
    });
    let class = crate::expose::expose_class(thread, ISO_OBJECT, None);
    class.method("checkObjectPowered", |c| {
        let this = c.arg(0);
        let receiver = object_from_value(&this).ok_or_else(super::null_pointer)?;
        let tick = crate::bind_misc::ingame_tick();
        if let Some(tick) = tick {
            let body = receiver
                .object
                .try_borrow()
                .map_err(|_| borrowed("IsoObject"))?;
            let (previous, on) = body.power_cache.get();
            if previous == Some(tick) {
                return Ok(c.push(on));
            }
            body.power_cache.set((Some(tick), on));
        }
        let on = object_powered(&mut Cx { t: c.thread }, &this, true)?;
        if tick.is_some() {
            receiver
                .object
                .try_borrow()
                .map_err(|_| borrowed("IsoObject"))?
                .power_cache
                .set((tick, on));
        }
        Ok(c.push(on))
    });
}
