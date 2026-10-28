//! Real InventoryItem seen/heard methods with typed bounded frontend phases.
use super::{ItemRef, LuaInventoryItem, INVENTORY_ITEM_CLASS};
use pz_entity::{EntityOwner, RetiredComponents};
use pz_inventory::row_read_flags;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use std::any::Any;
const MAP: &str = "zombie.inventory.types.MapItem";
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
fn unsupported(operation: &'static str) -> pz_kahlua::LuaError {
    crate::bind_world_object::unsupported(operation)
}
fn receiver(call: &NativeCall<'_>, pins: &mut RetiredComponents) -> LuaResult<ItemRef> {
    let value = call.arg(0);
    let Value::Object(object) = &value else {
        return Err(unsupported("media read InventoryItem receiver conversion"));
    };
    let Some(item) = (&**object as &dyn Any).downcast_ref::<LuaInventoryItem>() else {
        return Err(unsupported("media read InventoryItem receiver conversion"));
    };
    pins.push_owner(EntityOwner::from_ref(&item.item));
    Ok(item.item.clone())
}
/// Native reference argument conversion: null or an actual InventoryItem. The
/// returned class fact is allocation metadata; no item-body read/virtual call.
pub(crate) fn nullable_item(
    value: &Value,
    pins: &mut RetiredComponents,
) -> LuaResult<Option<(ItemRef, bool)>> {
    if value.is_nil() {
        return Ok(None);
    }
    let Value::Object(object) = value else {
        return Err(unsupported(
            "read-state nullable InventoryItem argument conversion",
        ));
    };
    let Some(item) = (&**object as &dyn Any).downcast_ref::<LuaInventoryItem>() else {
        return Err(unsupported(
            "read-state nullable InventoryItem argument conversion",
        ));
    };
    pins.push_owner(EntityOwner::from_ref(&item.item));
    Ok(Some((item.item.clone(), item.class == MAP)))
}
/// Stored OBJECT is the equals receiver. The name has already been
/// evaluated before entering this dispatcher. Only genuine immutable scalar
/// classes are admitted; no virtual Rust host helpers or coercions run.
fn stored_equals_name(value: &Value, name: &str) -> LuaResult<bool> {
    match value {
        Value::Str(value) => Ok(value.as_ref() == name),
        Value::Bool(_) | Value::Num(_) => Ok(false),
        Value::Object(object) => {
            // LuaObject: Any permits a nonvirtual trait upcast. This does NOT
            // call the overridable as_any/class_name/to_lua_string methods.
            let intrinsic = &**object as &dyn Any;
            if intrinsic.is::<pz_kahlua::table::BoxedFloat>()
                || intrinsic.is::<crate::expose::JavaInteger>()
                || intrinsic.is::<crate::bind_java_util::JavaInteger>()
            {
                return Ok(false);
            }
            Err(unsupported(
                "InventoryItem.isNoRecipes stored Object.equals dispatch",
            ))
        }
        _ => Err(unsupported(
            "InventoryItem.isNoRecipes stored Object.equals dispatch",
        )),
    }
}

/// Shared actual method body for the Lua method and native tooltip caller.
/// Argument admission is pointer/class metadata only; nil exits before the
/// first Item body/getModData access. The captured table value is retained
/// across actual player.getFullName and equality, including every failure.
pub(crate) fn is_no_recipes_value(item: &ItemRef, player: &Value) -> LuaResult<bool> {
    let mut inputs = Inputs(RetiredComponents::new());
    inputs.0.push_owner(EntityOwner::from_ref(item));
    inputs.0.push_payload(Box::new(player.clone()));
    let Some(player) =
        crate::bind_character_sim::read_flags::optional_player(player, &mut inputs.0)?
    else {
        return Ok(false);
    };
    let table =
        pz_inventory::moddata::get_mod_data_ref(item).map_err(|error| error.to_lua_error())?;
    inputs.0.push_payload(Box::new(table.clone()));
    let value = table
        .try_rawget(&Value::str("itemNoRecipes"))
        .map_err(|error| pz_inventory::ModDataError::from(error).to_lua_error())?;
    inputs.0.push_payload(Box::new(value.clone()));
    if value.is_nil() {
        return Ok(false);
    }
    let name = pz_characters::character::known_poison::full_name_ref(&player)
        .map_err(crate::bind_world_object::host_error)?;
    stored_equals_name(&value, &name)
}

pub(super) fn register(thread: &mut Thread) {
    let builder = crate::expose::expose_class(thread, INVENTORY_ITEM_CLASS, None);
    builder.method("isNoRecipes", |call| {
        let mut inputs = Inputs::new(call);
        let item = receiver(call, &mut inputs.0)?;
        if call.nargs() != 2 {
            return Err(unsupported("isNoRecipes argument count conversion"));
        }
        let result = is_no_recipes_value(&item, &call.arg(1))?;
        Ok(call.push(result))
    });
    for (name, seen) in [("hasBeenSeen", true), ("hasBeenHeard", false)] {
        builder.method(name, move |call| {
            let mut inputs = Inputs::new(call);
            let item = receiver(call, &mut inputs.0)?;
            if call.nargs() != 2 {
                return Err(unsupported("media read argument count conversion"));
            }
            let _player = crate::bind_character_sim::read_flags::optional_player(
                &call.arg(1),
                &mut inputs.0,
            )?;
            let result = if seen {
                row_read_flags::has_been_seen_ref(&item)
            } else {
                row_read_flags::has_been_heard_ref(&item)
            }
            .map_err(crate::bind_world_object::host_error)?;
            Ok(call.push(result))
        });
    }
}
#[cfg(test)]
mod tests;
