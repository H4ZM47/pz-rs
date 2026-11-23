//! The original IsoObject water queries use the sandbox clock, sprite
//! properties, live fluid components, saved reserves and rooftop collectors.
use super::reads::{receiver, with_sprite};
use super::Cx;
use crate::bind_world::square_queries::square_object_value;
use crate::bind_world_object::{borrowed, object_square};
use pz_kahlua::{LuaError, LuaResult, TableRef, Value};
use pz_world::square_mutation::SquarePos;
use pz_world::IsoObjectRef;
use std::rc::Rc;

#[derive(Clone, Copy)]
enum Measure {
    Amount,
    Capacity,
}

pub(super) fn register(object: &crate::expose::ClassBuilder) {
    for (name, measure) in [
        ("getFluidAmount", Measure::Amount),
        ("getFluidCapacity", Measure::Capacity),
    ] {
        object.method(name, move |c| {
            let this = c.arg(0);
            let water = quantity(&mut Cx { t: c.thread }, &this, measure, &mut Vec::new())?;
            Ok(c.push(water as f64))
        });
    }
    object.method("hasFluid", |c| {
        let this = c.arg(0);
        let wet = has_fluid(&mut Cx { t: c.thread }, &this, &mut Vec::new())?;
        Ok(c.push(wet))
    });
    object.method("getUsesExternalWaterSource", |c| {
        let this = c.arg(0);
        let external = receiver(&this)?
            .object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject"))?
            .uses_external_water_source;
        Ok(c.push(external))
    });
    object.method("setUsesExternalWaterSource", |c| {
        let this = c.arg(0);
        receiver(&this)?
            .object
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoObject"))?
            .uses_external_water_source = crate::expose::bool_arg(c, 1);
        Ok(0)
    });
    object.method("hasExternalWaterSource", |c| {
        let this = c.arg(0);
        Ok(c.push(cached_source(&this)?.is_some()))
    });
    object.method("doFindExternalWaterSource", |c| {
        let this = c.arg(0);
        find_and_cache(&mut Cx { t: c.thread }, &this, &mut Vec::new())?;
        Ok(0)
    });
}

fn puddle(cx: &mut Cx<'_>, this: &Value) -> LuaResult<f32> {
    let object = receiver(this)?;
    let solid_floor = with_sprite(object, |m, s| {
        s.is_some_and(|s| s.properties.has(m.tables.flag.solidfloor))
    })?;
    if solid_floor {
        let square = object_square(object)?;
        if !square.is_nil() {
            let depth = cx.num(&square, "getPuddlesInGround", &[])? as f32;
            if depth > 0.09 {
                return Ok(depth);
            }
        }
    }
    Ok(0.0)
}

fn has_fluid(cx: &mut Cx<'_>, this: &Value, visiting: &mut Vec<IsoObjectRef>) -> LuaResult<bool> {
    Ok(puddle(cx, this)? > 0.0 || quantity(cx, this, Measure::Amount, visiting)? > 0.0)
}

fn stored_number(data: Option<&TableRef>, key: &str) -> LuaResult<Option<f32>> {
    let field = data.map_or(Value::Nil, |d| d.get_str(key));
    if field.is_nil() {
        return Ok(None);
    }
    field
        .as_num()
        .map(|n| Some(n as f32))
        .ok_or_else(|| crate::bind_java_util::class_cast(&field, "java.lang.Double"))
}

fn property_number(value: Option<&str>) -> LuaResult<Option<f32>> {
    value
        .map(|v| {
            v.parse()
                .map_err(|_| LuaError::runtime(format!("invalid water quantity {v}")))
        })
        .transpose()
}

fn quantity(
    cx: &mut Cx<'_>,
    this: &Value,
    measure: Measure,
    visiting: &mut Vec<IsoObjectRef>,
) -> LuaResult<f32> {
    let object = receiver(this)?;
    let fields = with_sprite(object, |m, s| {
        let s = s?;
        Some((
            s.properties.get(&m.alias, "waterAmount").map(str::to_owned),
            s.properties
                .get(&m.alias, "waterMaxAmount")
                .map(str::to_owned),
            m.tables
                .flag_from_string("waterPiped")
                .is_some_and(|f| s.properties.has(f)),
        ))
    })?;
    let Some((amount, maximum, piped)) = fields else {
        return Ok(0.0);
    };
    if let Some(amount) = &amount {
        if amount
            .parse::<i32>()
            .map_err(|_| LuaError::runtime("invalid waterAmount sprite property"))?
            >= 9999
        {
            return Ok(10_000.0);
        }
    }
    let external = object
        .object
        .try_borrow()
        .map_err(|_| borrowed("IsoObject"))?
        .uses_external_water_source;
    if external {
        if visiting.iter().any(|o| Rc::ptr_eq(o, &object.object)) {
            return Err(LuaError::runtime("cyclic external water sources"));
        }
        visiting.push(object.object.clone());
        let source = match cached_source(this)? {
            Some(source) if has_fluid(cx, &source, visiting)? => Some(source),
            _ => find_and_cache(cx, this, visiting)?,
        };
        let result = match source {
            Some(source) => quantity(cx, &source, measure, visiting),
            None => Ok(0.0),
        };
        visiting.pop();
        return result;
    }
    let square = object_square(object)?;
    let data = if piped && cx.is(this, "hasModData", &[])? {
        cx.call(this, "getModData", &[])?.as_table().cloned()
    } else {
        None
    };
    let unmoved = piped
        && !matches!(
            data.as_ref().map(|d| d.get_str("canBeWaterPiped")),
            Some(Value::Bool(true))
        );
    let no_water = unmoved && !square.is_nil() && cx.is(&square, "isNoWater", &[])?;
    if unmoved && !square.is_nil() && !no_water && !cx.call(&square, "getRoom", &[])?.is_nil() {
        let time = crate::bind_climate::shared_game_time();
        let clock = time.try_borrow().map_err(|_| borrowed("GameTime"))?;
        let active = crate::bind_sandbox::with_current(|sb| {
            clock.world_age_days_since_begin(sb.time_since_apo())
                < f64::from(sb.water_shut_modifier())
        })
        .ok_or_else(|| LuaError::runtime("water supply requires sandbox options"))?;
        if active {
            return Ok(10_000.0);
        }
    }
    let fluid = cx.call(this, "getFluidContainer", &[])?;
    if !fluid.is_nil() {
        let name = match measure {
            Measure::Amount => "getAmount",
            Measure::Capacity => "getCapacity",
        };
        let value = cx.num(&fluid, name, &[])? as f32;
        if value > 0.0 {
            return Ok(value);
        }
    }
    if unmoved && !no_water {
        let reserve = match stored_number(data.as_ref(), "waterAmount")? {
            Some(n) => n,
            None => property_number(amount.as_deref())?.unwrap_or(0.0),
        };
        if reserve > 0.0 {
            return match measure {
                Measure::Amount => Ok(reserve),
                Measure::Capacity => match stored_number(data.as_ref(), "waterMaxAmount")? {
                    Some(n) => Ok(n),
                    None => Ok(property_number(maximum.as_deref())?.unwrap_or(reserve)),
                },
            };
        }
    }
    let depth = puddle(cx, this)?;
    if depth > 0.0 {
        let props = cx.call(&square, "getProperties", &[])?;
        let flag = cx.static_field("IsoFlagType", "water");
        if !cx.is(&props, "has", &[flag])? {
            return Ok(depth * 10.0);
        }
    }
    Ok(0.0)
}

fn cached_source(this: &Value) -> LuaResult<Option<Value>> {
    let cached = receiver(this)?
        .object
        .try_borrow()
        .map_err(|_| borrowed("IsoObject"))?
        .external_water_source
        .get();
    let Some((pos, object)) = cached else {
        return Ok(None);
    };
    let present = crate::bind_world::try_with_world_read(|w| {
        let Some(sq) = w
            .cell
            .as_ref()
            .and_then(|c| c.get_grid_square(pos.0, pos.1, pos.2))
        else {
            return Ok(false);
        };
        let sq = sq.try_borrow().map_err(|_| {
            pz_entity::runtime_load::RuntimeHostError::Borrowed("water source square")
        })?;
        Ok(sq
            .objects
            .iter()
            .any(|s| Rc::ptr_eq(s.owner_slot(), &object)))
    })
    .map_err(crate::bind_world_object::host_error)?;
    present
        .then(|| square_object_value(&object, pos))
        .transpose()
}

fn find_and_cache(
    cx: &mut Cx<'_>,
    this: &Value,
    visiting: &mut Vec<IsoObjectRef>,
) -> LuaResult<Option<Value>> {
    let object = receiver(this)?;
    let square = object_square(object)?;
    let source = match crate::bind_world::square_coords(&square) {
        Some(pos) => find_source(cx, this, pos, visiting)?,
        None => None,
    };
    let cached = match &source {
        Some((pos, value)) => Some((*pos, &receiver(value)?.object)),
        None => None,
    };
    object
        .object
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoObject"))?
        .external_water_source
        .set(cached);
    Ok(source.map(|(_, value)| value))
}

fn find_source(
    cx: &mut Cx<'_>,
    this: &Value,
    (x, y, z): SquarePos,
    visiting: &mut Vec<IsoObjectRef>,
) -> LuaResult<Option<(SquarePos, Value)>> {
    let object = receiver(this)?;
    let (ox, oy, width, height) = {
        let manager = crate::bind_sprites::manager()?;
        let m = manager
            .try_borrow()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        let body = object
            .object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject"))?;
        let sprite = body
            .sprite
            .try_read(&m)
            .map_err(crate::bind_world_object::host_error)?;
        match sprite
            .and_then(|s| s.sprite_grid)
            .and_then(|idx| m.grids.get(idx))
        {
            Some(grid) => {
                let (dx, dy, _) = body
                    .sprite
                    .tile_projection()
                    .and_then(|i| grid.sprite_pos(i))
                    .unwrap_or((-1, -1, -1));
                (x - dx, y - dy, grid.width, grid.height)
            }
            None => (x, y, 1, 1),
        }
    };
    for gx in ox..ox + width {
        for gy in oy..oy + height {
            let mut empty = None;
            for (dx, dy) in [
                (0, 0),
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let pos = (gx + dx, gy + dy, z + 1);
                if let Some(value) = source_on_square(cx, pos, visiting)? {
                    if has_fluid(cx, &value, visiting)? {
                        return Ok(Some((pos, value)));
                    }
                    if empty.is_none() {
                        empty = Some((pos, value));
                    }
                }
            }
            if empty.is_some() {
                return Ok(empty);
            }
        }
    }
    Ok(None)
}

fn source_on_square(
    cx: &mut Cx<'_>,
    pos: SquarePos,
    visiting: &mut Vec<IsoObjectRef>,
) -> LuaResult<Option<Value>> {
    let objects = crate::bind_world::with_cell_square(pos.0, pos.1, pos.2, |sq| {
        sq.objects
            .iter()
            .map(|s| s.ref_handle())
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    for object in objects {
        {
            let body = object
                .try_borrow()
                .map_err(|_| borrowed("water collector"))?;
            if !matches!(body.kind, pz_world::ObjectKind::Thumpable(_))
                || body.uses_external_water_source
            {
                continue;
            }
        }
        let value = square_object_value(&object, pos)?;
        if with_sprite(receiver(&value)?, |_, s| s.is_some_and(|s| s.solidfloor))? {
            continue;
        }
        if quantity(cx, &value, Measure::Capacity, visiting)? > 0.0 {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
