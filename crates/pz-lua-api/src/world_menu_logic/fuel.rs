//! IsoObject's single-player piped fuel supply and empty-station signs.
use super::Cx;
use crate::bind_world_object::{borrowed, object_from_value, object_square};
use pz_kahlua::{LuaError, LuaResult, Value};

pub(super) fn register(object: &crate::expose::ClassBuilder) {
    object.method("getPipedFuelAmount", |c| {
        let this = c.arg(0);
        let r = piped_fuel_amount(&mut Cx { t: c.thread }, &this)?;
        Ok(c.push(r))
    });
    object.method("setPipedFuelAmount", |c| {
        let this = c.arg(0);
        let units = crate::expose::int_arg(c, 1).max(0);
        if piped_fuel_amount(&mut Cx { t: c.thread }, &this)? != units {
            store_fuel(&this, units)?;
        }
        Ok(0)
    });
}

/// `getPipedFuelAmount()`: the mod data's `fuelAmount`. A fuel pump
/// (`fuelAmount` property) fills it on first use from the sandbox options.
fn piped_fuel_amount(cx: &mut Cx<'_>, this: &Value) -> LuaResult<i32> {
    let object = object_from_value(this).ok_or_else(super::null_pointer)?;
    let manager = crate::bind_sprites::manager()?;
    let capacity = {
        let body = object.object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        let manager = manager.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
        let sprite = body.sprite.try_read(&manager).map_err(crate::bind_world_object::host_error)?;
        let Some(sprite) = sprite else { return Ok(0) };
        sprite.properties.get(&manager.alias, "fuelAmount").map(str::to_owned)
    };
    let mut amount = -1.0;
    if cx.is(this, "hasModData", &[])? {
        if let Value::Table(data) = cx.call(this, "getModData", &[])? {
            let stored = data.get_str("fuelAmount");
            if !stored.is_nil() {
                amount = stored.as_num().ok_or_else(|| crate::bind_java_util::class_cast(&stored, "java.lang.Double"))?;
            }
        }
    }
    let Some(capacity) = capacity else { return Ok(amount as i32) };
    if crate::bind_sandbox::current_bool("FuelStationGasInfinite") == Some(true) {
        return Ok(1000);
    }
    if amount == -1.0 {
        let square = object_square(object)?;
        let generator = crate::bind_sandbox::current_bool("AllowExteriorGenerator") == Some(true)
            && cx.is(&square, "haveElectricity", &[])?;
        if generator || cx.is(this, "hasGridPower", &[])? {
            let capacity: i32 = capacity.parse().map_err(|_| LuaError::runtime("invalid fuelAmount sprite property"))?;
            let (min, max, empty_chance) = crate::bind_sandbox::with_current(|sb| {
                Some((
                    sb.option_by_name("FuelStationGasMin")?.config.get_double() as f32,
                    sb.option_by_name("FuelStationGasMax")?.config.get_double() as f32,
                    sb.option_by_name("FuelStationGasEmptyChance")?.config.get_int(),
                ))
            }).flatten().ok_or_else(|| LuaError::runtime("missing fuel station sandbox options"))?;
            let mut units = pz_core::random::next_float(capacity as f32 * min.min(max), capacity as f32 * max) as i32;
            if cx.is(&square, "isNoGas", &[])? || pz_core::random::next(100) < empty_chance {
                units = 0;
            }
            if units == 0 && pz_core::random::next(2) == 0 {
                place_empty_sign(cx, this, &square)?;
            }
            store_fuel(this, units)?;
            return Ok(units);
        }
    }
    Ok(amount as i32)
}

fn store_fuel(this: &Value, units: i32) -> LuaResult<()> {
    let object = object_from_value(this).ok_or_else(super::null_pointer)?;
    let data = object.object.try_borrow_mut().map_err(|_| borrowed("IsoObject body"))?
        .install_runtime_mod_data().map_err(|e| LuaError::runtime(e.0))?;
    data.set_str("fuelAmount", Value::Num(f64::from(units)));
    Ok(())
}

fn adjacent(cx: &mut Cx<'_>, square: &Value, direction: &str) -> LuaResult<Value> {
    let direction = cx.static_field("IsoDirections", direction);
    cx.call(square, "getAdjacentSquare", &[direction])
}

fn open_space(cx: &mut Cx<'_>, square: &Value) -> LuaResult<bool> {
    if square.is_nil() { return Ok(false); }
    let objects = cx.call(square, "getObjects", &[])?;
    Ok(cx.num(&objects, "size", &[])? < 2.0)
}

fn place_empty_sign(cx: &mut Cx<'_>, pump: &Value, square: &Value) -> LuaResult<()> {
    let facing = cx.call(pump, "getProperty", &[Value::str("Facing")])?;
    let mut direction = match facing.as_str() {
        Some("W") => "W", Some("N") => "N", Some("S") => "S", _ => "E",
    };
    if pz_core::random::next(2) == 0 {
        direction = match direction { "E" => "W", "S" => "N", other => other };
    }
    let target = adjacent(cx, square, direction)?;
    if target.is_nil() || cx.is(&target, "isNoGas", &[])? || !open_space(cx, &target)? { return Ok(()); }
    let east_west = matches!(direction, "E" | "W");
    for side in if east_west { ["N", "S"] } else { ["E", "W"] } {
        let neighbor = adjacent(cx, &target, side)?;
        if !open_space(cx, &neighbor)? { return Ok(()); }
    }
    let zone = cx.call(square, "getZombiesType", &[])?;
    let (sheet, tile) = match zone.as_str() {
        Some("Gas2Go") => ("location_shop_gas2go_01_3", 8),
        Some("Fossoil") => ("location_shop_fossoil_01_6", 8),
        _ => ("signs_one-off_07_8", 5),
    };
    let name = format!("{sheet}{}", tile + i32::from(east_west));
    cx.call(&target, "addTileObject", &[Value::str(&name)])?;
    Ok(())
}
