//! The `IsoGridSquare` reads that `AdjacentFreeTileFinder` (the Lua one in
//! `shared/Util`) makes to pick a tile beside a fence, window or door:
//! `getAdjacentSquare`, `canStand`, `has`, `isDoorBlockedTo` and
//! `DistToProper`.

use super::*;
use crate::expose::JavaEnum;
use pz_world::world_state::ObjectHooks;
use pz_world::IsoDirections;

fn direction_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<IsoDirections> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_iso_enums::ISO_DIRECTIONS)
            .and_then(|e| IsoDirections::from_name(e.name))
            .ok_or_else(|| LuaError::runtime("Expected an IsoDirections")),
        Value::Nil => Err(npe()),
        _ => Err(LuaError::runtime("Expected an IsoDirections")),
    }
}

/// The `IsoFlagType` constant or flag name in argument `i`, as a name.
fn flag_name(c: &NativeCall<'_>, i: usize) -> Option<String> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_iso_enums::ISO_FLAG_TYPE)
            .map(|e| e.name.to_string()),
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

pub(super) fn register(b: &ClassBuilder) {
    // `getAdjacentSquare(dir)`: the loaded square one step in `dir` on the
    // same level, `nil` when it is not loaded.
    get(b, "getAdjacentSquare", |c| {
        let (x, y, z) = self_square(c)?;
        let (dx, dy) = direction_arg(c, 1)?.delta();
        Ok(square_value(x + dx, y + dy, z))
    });
    // `has(IsoFlagType)` / `has(String)`: the square's property flags.
    get(b, "has", |c| {
        let (x, y, z) = self_square(c)?;
        let Some(name) = flag_name(c, 1) else {
            return Ok(false.into());
        };
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        let Some(flag) = sprites.tables.flag_from_string(&name) else {
            return Ok(false.into());
        };
        cell(|cell| {
            let square = cell
                .get_grid_square(x, y, z)
                .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))?;
            let square = square
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
            Ok(square.has(flag).into())
        })
    });
    // `canStand()`.
    get(b, "canStand", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        let flag = |name| sprites.tables.flag_from_string(name);
        cell(|cell| {
            let square = cell
                .get_grid_square(x, y, z)
                .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))?;
            let square = square
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
            if flag("solid").is_some_and(|f| square.has(f)) {
                return Ok(false.into());
            }
            if flag("solidtrans").is_some_and(|f| square.has(f)) {
                let hooks = ObjectHooks::new(cell, &sprites);
                return Ok((square.is_adjacent_to_window(&sprites, cell, &hooks)
                    || square.is_adjacent_to_hoppable(&sprites, cell, &hooks))
                .into());
            }
            Ok(square.treat_as_solid_floor(&sprites).into())
        })
    });
    // `isDoorBlockedTo(other)`.
    get(b, "isDoorBlockedTo", |c| {
        let (x, y, z) = self_square(c)?;
        let other = match c.arg(1) {
            Value::Nil => None,
            _ => Some(square_arg(c, 1)?),
        };
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        cell(|cell| {
            let loaded = |(x, y, z)| {
                cell.get_grid_square(x, y, z)
                    .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))
            };
            let a = loaded((x, y, z))?;
            let a = a.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
            let other = other.map(loaded).transpose()?;
            let other = match &other {
                Some(o) => Some(o.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?),
                None => None,
            };
            let hooks = ObjectHooks::new(cell, &sprites);
            Ok(a.is_door_blocked_to(other.as_deref(), cell, &hooks).into())
        })
    });
    // `DistToProper(IsoMovingObject)`: the distance from the square's
    // centre to the object.
    get(b, "DistToProper", |c| {
        let (x, y, _) = self_square(c)?;
        let (ox, oy) = match c.arg(1) {
            Value::Nil => return Err(npe()),
            Value::Object(o) => match o.as_any().downcast_ref::<crate::bind_character_sim::CharacterObject>() {
                Some(chr) => {
                    let chr = chr.0.borrow();
                    (chr.vars.x, chr.vars.y)
                }
                None => return Err(LuaError::runtime("Expected an IsoMovingObject")),
            },
            _ => return Err(LuaError::runtime("Expected an IsoMovingObject")),
        };
        let (dx, dy) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
        Ok(Value::Num(f64::from((dx * dx + dy * dy).sqrt())))
    });
}
