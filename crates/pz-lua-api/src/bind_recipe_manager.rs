//! `zombie.inventory.RecipeManager`: the static `getEvolvedRecipe`, which
//! `ISInventoryPaneContextMenu.createMenu` calls for each selected item.
//!
//! Not exposed yet: the statics that work on the old `Recipe` scripts
//! (`getAvailableItemsAll`, `getAvailableItemsNeeded`,
//! `getNumberOfTimesRecipeCanBeDone`, `GetMovableRecipeTool`,
//! `IsRecipeValid`, ...).

use crate::bind_character_sim::CharacterObject;
use crate::bind_scripts::script_list;
use crate::expose::{bool_arg, expose_class};
use pz_characters::Characters;
use pz_inventory::InventoryItem;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_scripts::objects::recipes::EvolvedRecipe;
use pz_scripts::{ScriptManager, ScriptRef, ScriptType};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const RECIPE_MANAGER_CLASS: &str = "zombie.inventory.RecipeManager";

/// `RecipeManager.getEvolvedRecipe(item, chr, containers, includeItemsCanBeUse)`:
/// the evolved recipes whose base item or result item is `item`, in
/// `ScriptManager.getAllEvolvedRecipes()` order.
///
/// Rotten food gives an empty list below Cooking 7; `cooking_level` is read
/// only in that case, as in the original. A cooked item matches only recipes
/// with `AddIngredientIfCooked`, and a `WaterPot` only with at least 0.75
/// uses. With `include_items_can_be_use`, the original keeps a match only if
/// `EvolvedRecipe.getItemsCanBeUse` finds ingredients in the containers.
/// That method is not ported, so a match then returns an error.
pub fn evolved_recipes_for(
    sm: &ScriptManager,
    item: &InventoryItem,
    cooking_level: impl FnOnce() -> LuaResult<i32>,
    include_items_can_be_use: bool,
) -> LuaResult<Vec<ScriptRef>> {
    evolved_recipes_for_with_filter(sm, item, cooking_level, include_items_can_be_use, |_| Ok(true))
}

/// Internal variant allowing callers to filter candidate recipes with
/// `EvolvedRecipe.getItemsCanBeUse` when `include_items_can_be_use` is true.
fn evolved_recipes_for_with_filter(
    sm: &ScriptManager,
    item: &InventoryItem,
    cooking_level: impl FnOnce() -> LuaResult<i32>,
    include_items_can_be_use: bool,
    mut has_usable_items: impl FnMut(&EvolvedRecipe) -> LuaResult<bool>,
) -> LuaResult<Vec<ScriptRef>> {
    let mut out = Vec::new();
    if item.is_rotten() && cooking_level()? < 7 {
        return Ok(out);
    }
    for script in sm.collection(ScriptType::EvolvedRecipe).get_all_scripts() {
        let Some(recipe) = script.downcast::<EvolvedRecipe>() else {
            continue;
        };
        let recipe = recipe.borrow();
        if item.is_cooked() && !recipe.add_ingredient_if_cooked {
            continue;
        }
        let full_type = item.get_full_type();
        if recipe.base_item.as_deref() != Some(full_type.as_str())
            && recipe.result_item.as_deref() != Some(full_type.as_str())
        {
            continue;
        }
        // The original widens the float to a double before the compare.
        if item.get_type() == "WaterPot" && f64::from(item.get_current_uses_float()) < 0.75 {
            continue;
        }
        if include_items_can_be_use && !has_usable_items(&recipe)? {
            continue;
        }
        out.push(script.clone());
    }
    Ok(out)
}

/// The Cooking level of the `IsoGameCharacter` argument.
fn cooking_level(chars: &RefCell<Characters>, value: &Value) -> LuaResult<i32> {
    let character = match value {
        Value::Object(o) => (&**o as &dyn Any).downcast_ref::<CharacterObject>(),
        Value::Nil => return Err(crate::bind_scripts::npe()),
        _ => None,
    }
    .ok_or_else(|| LuaError::runtime("expected IsoGameCharacter"))?;
    let perk = chars
        .borrow()
        .perks
        .constant("Cooking")
        .ok_or_else(|| crate::bind_world_object::unsupported("PerkFactory.Perks.Cooking"))?;
    let level = character.0.borrow().perk_level(Some(&perk));
    Ok(level)
}

pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    crate::bind_evolved_recipe::register(thread, sm.clone(), chars.clone());
    let b = expose_class(thread, RECIPE_MANAGER_CLASS, None);
    b.static_fn("getEvolvedRecipe", move |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let chr_val = c.arg(1);
        let containers_val = c.arg(2);
        let include = bool_arg(c, 3);
        let chr_ref = match &chr_val {
            Value::Object(o) => (&**o as &dyn Any)
                .downcast_ref::<CharacterObject>()
                .map(|ch| ch.0.clone()),
            _ => None,
        };
        let list = evolved_recipes_for_with_filter(
            &sm,
            &item.borrow(),
            || cooking_level(&chars, &chr_val),
            include,
            |recipe| {
                let Some(chr) = chr_ref.as_ref() else {
                    return Ok(false);
                };
                Ok(!crate::bind_evolved_recipe::items_can_be_use(
                    recipe,
                    &chars,
                    chr,
                    &item,
                    &containers_val,
                )
                .is_empty())
            },
        )?;
        Ok(c.push(script_list(&list)))
    });
}

#[cfg(test)]
mod tests;
