//! Square queries over the current world's actual stored properties.

use super::*;

pub(super) fn register(b: &ClassBuilder) {
    get(b, "isOutside", |c| {
        let (x, y, z) = self_square(c)?;
        cell(|cell| {
            let square_owner = cell
                .get_grid_square(x, y, z)
                .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))?;
            let square = square_owner.borrow();
            let sprites = crate::bind_sprites::manager()?;
            let sprites = sprites
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
            // The original reads properties.has(exterior). Recalculation
            // belongs to square mutations, never to this cached-flag query.
            Ok(square.is_exterior(&sprites).into())
        })
    });
}

#[cfg(test)]
mod tests;
