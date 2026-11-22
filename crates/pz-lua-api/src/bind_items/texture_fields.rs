//! Actual InventoryItem texture assignment and virtual world-texture reads.
//! These methods use retained asset allocations, never descriptor imports or
//! texture-name lookups. Null is a genuine field value.

use std::any::Any;
use std::rc::Rc;

use pz_entity::{EntityOwner, RetiredComponents};
use pz_inventory::item_texture;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_render::{LuaTexture, TextureRef};

use super::{ItemRef, LuaInventoryItem, INVENTORY_ITEM_CLASS};

struct Inputs(RetiredComponents);
impl Inputs {
    fn new(call: &NativeCall<'_>) -> Self {
        let mut pins = RetiredComponents::new();
        for index in 0..call.nargs() {
            pins.push_payload(Box::new(call.arg(index)));
        }
        Self(pins)
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}

fn receiver(call: &NativeCall<'_>, pins: &mut Inputs) -> LuaResult<ItemRef> {
    let value = call.arg(0);
    if let Value::Object(object) = &value {
        // Intrinsic allocation metadata: no virtual as_any, formatter or
        // mutable item-body access during argument conversion.
        if let Some(item) = (&**object as &dyn Any).downcast_ref::<LuaInventoryItem>() {
            pins.0.push_owner(EntityOwner::from_ref(&item.item));
            return Ok(item.item.clone());
        }
    }
    Err(crate::bind_world_object::unsupported(
        "InventoryItem texture receiver conversion",
    ))
}

fn nullable_texture(value: Value) -> LuaResult<Option<TextureRef>> {
    match value {
        Value::Nil => Ok(None),
        Value::Object(object) => {
            // Preserve the same owning Rc, including Texture subclasses.
            let object: Rc<dyn Any> = object;
            object.downcast::<LuaTexture>().map(Some).map_err(|_| {
                crate::bind_world_object::unsupported(
                    "InventoryItem nullable Texture argument conversion",
                )
            })
        }
        _ => Err(crate::bind_world_object::unsupported(
            "InventoryItem nullable Texture argument conversion",
        )),
    }
}

pub(super) fn register(thread: &mut Thread) {
    let builder = crate::expose::expose_class(thread, INVENTORY_ITEM_CLASS, None);
    for name in ["setTexture", "setIcon"] {
        builder.method(name, |call| {
            let mut inputs = Inputs::new(call);
            let item = receiver(call, &mut inputs)?;
            if call.nargs() != 2 {
                return Err(crate::bind_world_object::unsupported(
                    "InventoryItem texture setter argument count conversion",
                ));
            }
            let texture = nullable_texture(call.arg(1))?;
            // Native setIcon delegates to the same setTexture assignment.
            item_texture::set_texture_ref(&item, texture.as_ref())
                .map_err(crate::bind_world_object::host_error)?;
            Ok(0)
        });
    }
    builder.method("getWorldTexture", |call| {
        let mut inputs = Inputs::new(call);
        let item = receiver(call, &mut inputs)?;
        if call.nargs() != 1 {
            return Err(crate::bind_world_object::unsupported(
                "InventoryItem world-texture getter argument count conversion",
            ));
        }
        let name = item_texture::get_world_texture_ref(&item)
            .map_err(crate::bind_world_object::host_error)?;
        Ok(call.push(name.map(Value::from).unwrap_or(Value::Nil)))
    });
}

#[cfg(test)]
mod tests;
