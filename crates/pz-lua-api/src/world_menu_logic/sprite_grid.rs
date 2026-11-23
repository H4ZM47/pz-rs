//! IsoObject sprite-grid membership used by beds, lighting and powered appliances.
use super::Cx;
use crate::bind_world_object::{
    borrowed, host_error, object_from_value, object_square, ISO_OBJECT,
};
use pz_entity::runtime_load::RuntimeHostError;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use std::rc::Rc;

pub(crate) fn objects(this: &Value, include_self: bool) -> LuaResult<Vec<Value>> {
    let receiver = object_from_value(this)
        .ok_or_else(|| crate::bind_java_util::class_cast(this, ISO_OBJECT))?;
    let manager = crate::bind_sprites::manager()?;
    let sprites = manager
        .try_borrow()
        .map_err(|_| borrowed("IsoSpriteManager"))?;
    let (grid_id, offset) = {
        let body = receiver
            .object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject"))?;
        let sprite = body.sprite.try_read(&sprites).map_err(host_error)?;
        let Some(id) = sprite.and_then(|s| s.sprite_grid) else {
            return Ok(if include_self {
                vec![this.clone()]
            } else {
                Vec::new()
            });
        };
        let grid = sprites
            .grids
            .get(id)
            .ok_or_else(|| LuaError::runtime("invalid sprite grid"))?;
        // Native sprites have independent identity from the manager's tile allocations.
        let offset = body
            .sprite
            .tile_projection()
            .and_then(|tile| grid.sprite_pos(tile))
            .unwrap_or((-1, -1, -1));
        (id, offset)
    };
    let grid = &sprites.grids[grid_id];
    let square = object_square(receiver)?;
    let (x, y, z) = crate::bind_world::square_coords(&square).ok_or_else(super::null_pointer)?;
    let (ox, oy, oz) = (x - offset.0, y - offset.1, z - offset.2);
    let owners = crate::bind_world::try_with_world_read(|w| {
        let cell = w
            .cell
            .as_ref()
            .ok_or(RuntimeHostError::InvalidHostState("no current cell"))?;
        let mut owners = Vec::new();
        for z in oz..oz + grid.levels {
            for y in oy..oy + grid.height {
                for x in ox..ox + grid.width {
                    let Some(sq) = cell.get_grid_square(x, y, z) else {
                        continue;
                    };
                    let sq = sq
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("IsoGridSquare"))?;
                    for slot in &sq.objects {
                        let owner = slot.ref_handle();
                        if !include_self && Rc::ptr_eq(&owner, &receiver.object) {
                            continue;
                        }
                        let same_grid = {
                            let body = owner
                                .try_borrow()
                                .map_err(|_| RuntimeHostError::Borrowed("IsoObject"))?;
                            let sprite = body.sprite.try_read(&sprites)?;
                            sprite.and_then(|s| s.sprite_grid) == Some(grid_id)
                        };
                        if same_grid {
                            owners.push((owner, (x, y, z)));
                        }
                    }
                }
            }
        }
        Ok(owners)
    })
    .map_err(host_error)?;
    drop(sprites);
    owners
        .into_iter()
        .map(|(owner, xyz)| crate::bind_world::square_queries::square_object_value(&owner, xyz))
        .collect()
}

pub(super) fn register(thread: &mut Thread) {
    let object = crate::expose::expose_class(thread, ISO_OBJECT, None);
    for (name, include_self) in [
        ("getSpriteGridObjects", None),
        ("getSpriteGridObjectsExcludingSelf", Some(false)),
        ("getSpriteGridObjectsIncludingSelf", Some(true)),
    ] {
        object.method(name, move |c| {
            let this = c.arg(0);
            let list = c.arg(1);
            let include_self = include_self.unwrap_or_else(|| crate::expose::bool_arg(c, 2));
            let mut cx = Cx { t: c.thread };
            cx.call(&list, "clear", &[])?;
            for item in objects(&this, include_self)? {
                cx.call(&list, "add", &[item])?;
            }
            Ok(c.push(list))
        });
    }
}
