//! Character.hasReadMap and nullable IsoPlayer conversion without body dispatch.
use super::{CharacterKind, CharacterObject, LuaGameCharacterRef};
use crate::expose::ClassBuilder;
use pz_entity::{EntityOwner, RetiredComponents};
use pz_kahlua::{LuaResult, NativeCall, Value};
use std::any::Any;
fn unsupported(operation: &'static str) -> pz_kahlua::LuaError {
    crate::bind_world_object::unsupported(operation)
}
struct Inputs(RetiredComponents);
impl Inputs {
    fn new(call: &NativeCall<'_>) -> Self {
        let mut pins = RetiredComponents::new();
        for i in 0..call.nargs() {
            pins.push_payload(Box::new(call.arg(i)));
        }
        Self(pins)
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}
fn receiver(call: &NativeCall<'_>, pins: &mut RetiredComponents) -> LuaResult<LuaGameCharacterRef> {
    let value = call.arg(0);
    let Value::Object(object) = &value else {
        return Err(unsupported("hasReadMap character receiver conversion"));
    };
    let Some(character) = (&**object as &dyn Any).downcast_ref::<CharacterObject>() else {
        return Err(unsupported("hasReadMap character receiver conversion"));
    };
    pins.push_owner(EntityOwner::from_ref(&character.0));
    Ok(character.0.clone())
}
/// IsoAnimal extends IsoPlayer in the current JAR. Validate immutable kind,
/// including that subclass, before any item field phase. No virtual helpers.
pub(crate) fn optional_player(
    value: &Value,
    pins: &mut RetiredComponents,
) -> LuaResult<Option<LuaGameCharacterRef>> {
    if value.is_nil() {
        return Ok(None);
    }
    let Value::Object(object) = value else {
        return Err(unsupported(
            "media read nullable IsoPlayer argument conversion",
        ));
    };
    let Some(character) = (&**object as &dyn Any).downcast_ref::<CharacterObject>() else {
        return Err(unsupported(
            "media read nullable IsoPlayer argument conversion",
        ));
    };
    if !matches!(character.1, CharacterKind::Player | CharacterKind::Animal) {
        return Err(unsupported(
            "media read nullable IsoPlayer argument conversion",
        ));
    }
    pins.push_owner(EntityOwner::from_ref(&character.0));
    Ok(Some(character.0.clone()))
}
pub(super) fn register(builder: &ClassBuilder) {
    builder.method("hasReadMap", |call| {
        let mut inputs = Inputs::new(call);
        let character = receiver(call, &mut inputs.0)?;
        if call.nargs() != 2 {
            return Err(unsupported("hasReadMap argument count conversion"));
        }
        let Some((item, is_map)) =
            crate::bind_items::row_read_flags::nullable_item(&call.arg(1), &mut inputs.0)?
        else {
            return Ok(call.push(false));
        };
        if !is_map {
            return Ok(call.push(false));
        }
        let media = pz_inventory::row_read_flags::map_media_id_ref(&item)
            .map_err(crate::bind_world_object::host_error)?;
        let result =
            pz_characters::character::reading::is_print_media_read(&character, media.as_deref())
                .map_err(crate::bind_world_object::host_error)?;
        Ok(call.push(result))
    });
}
#[cfg(test)]
mod tests;
