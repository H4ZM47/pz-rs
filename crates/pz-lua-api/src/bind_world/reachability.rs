//! The actual square edge and object-owner queries behind loot reachability.

use super::*;
use pz_world::world_state::ObjectHooks;

pub(super) fn register(b: &ClassBuilder) {
    get(b, "canReachTo", |c| {
        let from = self_square(c)?;
        // The original dereferences a null other square after its identity
        // test; nil must raise NPE rather than act as an unreachable square.
        let to = square_arg(c, 1)?;
        let excluded = crate::bind_iso_enums::ignore_blocking_sprite_names();
        cell(|cell| {
            let square = |(x, y, z)| {
                cell.get_grid_square(x, y, z)
                    .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))
            };
            let (from, to) = (square(from)?, square(to)?);
            // These original early returns do not read sprite definitions.
            if Rc::ptr_eq(&from, &to) {
                return Ok(true.into());
            }
            let from = from.borrow();
            let to = to.borrow();
            if (to.x - from.x).abs() > 1 || (to.y - from.y).abs() > 1 || to.z != from.z {
                return Ok(false.into());
            }
            let sprites = crate::bind_sprites::manager()?;
            let sprites = sprites
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
            let objects = ObjectHooks::new(cell, &sprites);
            Ok(from
                .can_reach_to_excluding(&to, &sprites, cell, &objects, &excluded)
                .into())
        })
    });
}

#[cfg(test)]
mod tests;
