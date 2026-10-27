//! Runtime Literature getters over the actual item allocation. Script Item
//! getters are a separate class and do not supply mutable instance fields.
//! Nonnull learned-recipe lists require a genuine native list backend; this
//! packet exposes only the completed null result, never a copied list.

use std::any::Any;

use pz_entity::runtime_load::HostResult;
use pz_entity::RetiredComponents;
use pz_inventory::literature::{self, LearnedRecipesResult};
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};

use super::{ItemRef, LuaInventoryItem, INVENTORY_ITEM_CLASS};

const LITERATURE: &str = "zombie.inventory.types.Literature";

struct InputPins(RetiredComponents);
impl InputPins {
    fn new(call: &NativeCall<'_>) -> Self {
        let mut pins = RetiredComponents::new();
        for index in 0..call.nargs() {
            pins.push_payload(Box::new(call.arg(index)));
        }
        Self(pins)
    }

    fn keep(&mut self, item: &ItemRef) {
        self.0.push_owner(pz_entity::EntityOwner::from_ref(item));
    }
}
impl Drop for InputPins {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}

fn getter_arity(call: &NativeCall<'_>) -> LuaResult<()> {
    if call.nargs() != 1 {
        return Err(crate::bind_world_object::unsupported(
            "Literature getter argument count conversion",
        ));
    }
    Ok(())
}

/// Class dispatch uses the userdata's immutable allocation metadata. It must
/// not borrow the item's mutable body or call unknown userdata formatters.
fn item_receiver(call: &NativeCall<'_>) -> LuaResult<(ItemRef, bool)> {
    let value = call.arg(0);
    let Value::Object(object) = &value else {
        return Err(crate::bind_world_object::unsupported(
            "Literature getter receiver conversion",
        ));
    };
    let Some(item) = (&**object as &dyn Any).downcast_ref::<LuaInventoryItem>() else {
        return Err(crate::bind_world_object::unsupported(
            "Literature getter receiver conversion",
        ));
    };
    Ok((item.item.clone(), item.class == LITERATURE))
}

fn literature_receiver(call: &NativeCall<'_>) -> LuaResult<ItemRef> {
    let (item, is_literature) = item_receiver(call)?;
    if !is_literature {
        return Err(crate::bind_world_object::unsupported(
            "Literature getter runtime class conversion",
        ));
    }
    Ok(item)
}

/// The already exposed virtual class fact, independent of current item data.
pub(super) fn is_literature(call: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut pins = InputPins::new(call);
    getter_arity(call)?;
    let (item, is_literature) = item_receiver(call)?;
    pins.keep(&item);
    Ok(call.push(is_literature))
}

pub(super) fn register(thread: &mut Thread) {
    let builder = crate::expose::expose_class(thread, LITERATURE, Some(INVENTORY_ITEM_CLASS));
    builder.method("getSkillTrained", |call| {
        let mut pins = InputPins::new(call);
        getter_arity(call)?;
        let item = literature_receiver(call)?;
        pins.keep(&item);
        let value = literature::get_skill_trained_ref(&item)
            .map_err(crate::bind_world_object::host_error)?;
        Ok(call.push(value.map(Value::from).unwrap_or(Value::Nil)))
    });

    type Getter = fn(&ItemRef) -> HostResult<i32>;
    for (name, get) in [
        (
            "getLvlSkillTrained",
            literature::get_lvl_skill_trained_ref as Getter,
        ),
        ("getMaxLevelTrained", literature::get_max_level_trained_ref),
        (
            "getNumLevelsTrained",
            literature::get_num_levels_trained_ref,
        ),
        ("getNumberOfPages", literature::get_number_of_pages_ref),
    ] {
        builder.method(name, move |call| {
            let mut pins = InputPins::new(call);
            getter_arity(call)?;
            let item = literature_receiver(call)?;
            pins.keep(&item);
            let value = get(&item).map_err(crate::bind_world_object::host_error)?;
            Ok(call.push(value))
        });
    }

    builder.method("getLearnedRecipes", |call| {
        let mut pins = InputPins::new(call);
        getter_arity(call)?;
        let item = literature_receiver(call)?;
        pins.keep(&item);
        match literature::get_learned_recipes_ref(&item)
            .map_err(crate::bind_world_object::host_error)?
        {
            LearnedRecipesResult::Null => Ok(call.push(Value::Nil)),
        }
    });
}

#[cfg(test)]
mod tests;
