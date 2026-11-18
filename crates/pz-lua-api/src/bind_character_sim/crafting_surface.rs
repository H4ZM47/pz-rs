//! `IsoGameCharacter.canUseAsGenericCraftingSurface`: the object counts as a
//! generic crafting surface and the character's square can reach its square.

use super::chr_of;
use crate::bind_world::{square_queries::object_square_near, square_value};
use crate::bind_world_object::{is_generic_crafting_surface, object_from_value, ISO_OBJECT};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

pub(super) fn can_use_as_generic_crafting_surface(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let chr = chr_of(c)?;
    let object = c.arg(1);
    // `obj.isGenericCraftingSurface()` runs first, so a nil object raises the
    // null-pointer error before the character's square is read.
    let world = match &object {
        Value::Nil => return Err(npe()),
        other => object_from_value(other).ok_or_else(|| super::class_cast(other, ISO_OBJECT))?,
    };
    if !is_generic_crafting_surface(world)? {
        return Ok(c.push(false));
    }
    // `getSquare().canReachTo(obj.getSquare())`.
    let at = chr.borrow().current_square_xyz().ok_or_else(npe)?;
    let from = square_value(at.0, at.1, at.2);
    if from.is_nil() {
        return Err(npe());
    }
    // An object outside the squares around the character is out of reach
    // whichever square holds it.
    let Some((x, y, z)) = object_square_near(&world.object, at)? else {
        return Ok(c.push(false));
    };
    let reach = c.thread.table_get(&from, &Value::str("canReachTo"))?;
    let reachable = c.thread.call1(&reach, &[from, square_value(x, y, z)])?;
    Ok(c.push(reachable))
}
