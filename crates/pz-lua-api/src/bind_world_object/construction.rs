//! Installed map scripts construct replacement tiles and generators before
//! placing them through the same square mutation service as player builds.
use super::*;
use crate::bind_map_objects::{apply_tile, PlacedTile};
use crate::bind_world::{square_coords, try_with_world_read};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};
use pz_kahlua::NativeCall;
use pz_world::objects::{IsoGenerator, ObjectSprite, SpriteName};
use pz_world::square_mutation::SquarePos;
use pz_world::{ObjectSlot, SquareRef};

pub(super) fn get_square(value: &Value) -> LuaResult<(SquarePos, SquareRef)> {
    let pos =
        square_coords(value).ok_or_else(|| LuaError::runtime("expected loaded IsoGridSquare"))?;
    let square = try_with_world_read(|w| {
        Ok(w.cell
            .as_ref()
            .and_then(|c| c.get_grid_square(pos.0, pos.1, pos.2)))
    })
    .map_err(host_error)?
    .ok_or_else(|| LuaError::runtime("object square is unloaded"))?;
    Ok((pos, square))
}

pub(super) fn named_sprite(name: &str) -> LuaResult<ObjectSprite> {
    let manager = crate::bind_sprites::manager()?;
    let mut m = manager
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoSpriteManager"))?;
    Ok(ObjectSprite::Tile(
        m.named_map
            .get(name)
            .copied()
            .unwrap_or_else(|| m.add_sprite_named(name)),
    ))
}

fn new_object(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut body = IsoObject::new(0, ObjectKind::Object, None);
    let first = c.arg(0);
    if c.nargs() == 0
        || (c.nargs() == 1
            && (first.is_nil() || first.raw_equals(&crate::bind_world::current_cell())))
    {
        return Ok(c.push(object_value(ObjectSlot::new(body).owner_slot())?));
    }
    let sq = if square_coords(&first).is_some() {
        let (_, sq) = get_square(&first)?;
        let tile = str_arg(c, 1).ok_or_else(|| LuaError::runtime("expected tile name"))?;
        if c.nargs() == 3 && (c.arg(2).is_nil() || matches!(c.arg(2), Value::Str(_))) {
            body.sprite = named_sprite(&tile)?;
            body.sprite_name = Some(SpriteName::Str(tile.to_string()));
            body.name = str_arg(c, 2).map(|s| ObjectName::Str(s.to_string()));
        } else if (c.nargs() == 3 && matches!(c.arg(2), Value::Bool(false)))
            || (c.nargs() == 4 && matches!(c.arg(3), Value::Bool(false)))
        {
            let manager = crate::bind_sprites::manager()?;
            let m = manager
                .try_borrow()
                .map_err(|_| borrowed("IsoSpriteManager"))?;
            body.sprite = m.named_map.get(tile.as_ref()).copied().into();
            if c.nargs() == 4 {
                body.name = str_arg(c, 2).map(|s| ObjectName::Str(s.to_string()));
            }
        } else {
            return Err(unsupported("IsoObject single page sprite constructor"));
        }
        body.tile = Some(tile.to_string());
        Some(sq)
    } else if c.nargs() == 3
        && (first.raw_equals(&crate::bind_world::current_cell()) || first.is_nil())
    {
        let square = c.arg(1);
        if let Some(tile) = str_arg(c, 2) {
            body.sprite = named_sprite(&tile)?;
            body.tile = Some(tile.to_string());
        } else {
            body.sprite = crate::bind_sprites::object_sprite_argument(&c.arg(2))?;
        }
        if square.is_nil() {
            None
        } else {
            Some(get_square(&square)?.1)
        }
    } else {
        return Err(LuaError::runtime("invalid IsoObject constructor"));
    };
    body.replace_square(sq.as_ref(), &mut pz_entity::RetiredComponents::new());
    Ok(c.push(object_value(ObjectSlot::new(body).owner_slot())?))
}

fn update_from_item(object: &mut IsoObject, item: &pz_inventory::shared::ItemRef) -> LuaResult<()> {
    let (condition, fuel, name) = {
        let i = item.try_borrow().map_err(|_| borrowed("generator item"))?;
        (
            i.condition,
            i.mod_data_value("fuel")
                .map_err(|e| e.to_lua_error())?
                .and_then(|v| v.as_num()),
            i.get_full_type(),
        )
    };
    let ObjectKind::Generator(g) = &mut object.kind else {
        return Err(LuaError::runtime("expected IsoGenerator"));
    };
    g.condition = condition;
    if let Some(fuel) = fuel {
        g.fuel = fuel as f32;
    }
    object
        .install_runtime_mod_data()
        .map_err(|e| LuaError::runtime(e.to_string()))?
        .set_str("generatorFullType", Value::str(&name));
    Ok(())
}

fn new_generator(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let state = IsoGenerator {
        connected: false,
        activated: false,
        fuel: 0.,
        condition: 0,
        last_hour: -1,
        total_power_using: 0.,
    };
    let mut body = IsoObject::new(0, ObjectKind::Generator(state), None);
    let cell = if c.nargs() == 1 { c.arg(0) } else { c.arg(1) };
    if !cell.is_nil() && !cell.raw_equals(&crate::bind_world::current_cell()) {
        return Err(LuaError::runtime("expected IsoCell"));
    }
    let square = if c.nargs() == 1 {
        None
    } else if c.nargs() == 3 {
        let item = crate::bind_items::item_arg(c, 0)?;
        let (pos, sq) = get_square(&c.arg(2))?;
        let sm = crate::bind_registry::shared_script_manager();
        let script = pz_characters::character::items::script_item(&sm, &item.borrow())
            .ok_or_else(|| LuaError::runtime("generator script item is missing"))?;
        if let Some(name) = &script.borrow().world_object_sprite {
            body.sprite = named_sprite(name)?;
        }
        update_from_item(&mut body, &item)?;
        body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
        Some(pos)
    } else {
        return Err(LuaError::runtime("invalid IsoGenerator constructor"));
    };
    let owner = ObjectSlot::new(body).ref_handle();
    if let Some(pos) = square {
        apply_tile(
            c.thread,
            pos,
            PlacedTile::Existing {
                object: owner.clone(),
                index: -1,
                special: true,
            },
        )?;
    }
    pz_world::world_state::generator_constructed().map_err(|e| LuaError::runtime(e.to_string()))?;
    Ok(c.push(object_value(&owner)?))
}

pub(super) fn register(t: &mut Thread) {
    let class = expose_class(t, ISO_OBJECT, None);
    class.static_fn("new", new_object);
    class.method("setOutlineOnMouseover", |c| {
        let b = bool_arg(c, 1);
        receiver_object(c)?
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoObject"))?
            .outline_on_mouseover = b;
        Ok(0)
    });
    let g = expose_class(t, "zombie.iso.objects.IsoGenerator", Some(ISO_OBJECT));
    g.static_fn("new", new_generator);
    g.method("setInfoFromItem", |c| {
        let object = receiver_object(c)?;
        let item = crate::bind_items::item_arg(c, 1)?;
        let mut o = object
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoGenerator"))?;
        update_from_item(&mut o, &item)?;
        Ok(0)
    });
    for method in [
        "getFuel",
        "getFuelPercentage",
        "getMaxFuel",
        "getCondition",
        "isConnected",
        "isActivated",
        "setFuel",
        "setCondition",
        "setConnected",
    ] {
        g.method(method, move |c| {
            let obj = receiver_object(c)?;
            let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoGenerator"))?;
            let ObjectKind::Generator(g) = &mut body.kind else {
                return Err(LuaError::runtime("expected IsoGenerator"));
            };
            Ok(match method {
                "getFuel" => c.push(g.fuel as f64),
                "getFuelPercentage" => c.push((g.fuel / 10. * 100.) as f64),
                "getMaxFuel" => c.push(10.),
                "getCondition" => c.push(g.condition),
                "isActivated" => c.push(g.activated),
                "isConnected" => c.push(g.connected),
                "setFuel" => {
                    g.fuel = (num_arg(c, 1) as f32).clamp(0., 10.);
                    0
                }
                "setCondition" => {
                    g.condition = int_arg(c, 1).clamp(0, 100);
                    0
                }
                _ => {
                    g.connected = bool_arg(c, 1);
                    0
                }
            })
        });
    }
}
