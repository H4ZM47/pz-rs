//! The item side of the craft checks: `InputScript.containsItem`, the
//! `doesItemPass*Tests` status tests, and `CraftRecipeManager.consumeInputItem`
//! for an `Item` input in its test-only form (`testOnly == true`), which
//! `isItemValidForInputScript`, `InputScriptData.acceptsInputItem` and
//! `CraftRecipeData.consumeInputFromItems` call.
//!
//! The reads the port does not have yet return `unsupported` when a flag
//! makes the original read them: sharpness and head condition (the item's
//! attribute container), the sealed-name test (`Translator.getFluidText`),
//! `Food.isWholeFoodItem`, radio device power, fluid containers, energy
//! items, and the `ConsumeFromItem`/`CreateToItem` lines.

use super::RecipeCharacter;
use crate::bind_world_object::unsupported;
use pz_kahlua::LuaResult;
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, InputFlag, InputScript, LuaCall};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::objects::items::Item;
use std::cell::RefCell;
use std::rc::Rc;

/// `Food` state the status tests read.
#[derive(Clone, Copy, Debug, Default)]
pub struct FoodState {
    /// `Food.isRotten()`.
    pub rotten: bool,
    /// `Food.isFrozen()`.
    pub frozen: bool,
    /// `InventoryItem.isCookable()`.
    pub cookable: bool,
    /// `isCooked()`, `isBurnt()` (`Food.isUncooked` is neither).
    pub cooked: bool,
    pub burnt: bool,
}

/// The `instanceof` tests of `doesItemPassIsOrNotEmptyAndFullTests`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraftItemClass {
    /// `InventoryContainer`.
    Container,
    /// `DrainableComboItem`.
    Drainable,
    /// `Radio`.
    Radio,
    /// `WeaponPart`.
    WeaponPart,
    Other,
}

/// What the craft checks read from an `InventoryItem` (and its holder).
pub trait CraftItem {
    /// The item's identity (`==` on the Java object).
    fn key(&self) -> usize;
    /// `getScriptItem()`.
    fn script_item(&self) -> Option<Rc<RefCell<Item>>>;
    /// `getFullType()`.
    fn full_type(&self) -> String;
    /// `getCurrentUses()`, `getMaxUses()`.
    fn current_uses(&self) -> i32;
    fn max_uses(&self) -> i32;
    fn class(&self) -> CraftItemClass;
    /// `isNoRecipes(player)`, for a player character.
    fn is_no_recipes(&self) -> LuaResult<bool>;
    /// `isFavorite()`.
    fn is_favorite(&self) -> bool;
    /// The `Food` state; `None` when `isFood()` is false.
    fn food(&self) -> Option<FoodState>;
    /// `isBroken()`.
    fn is_broken(&self) -> bool;
    /// `getContainer() != null && getContainer().getParent() instanceof
    /// IsoDeadBody`.
    fn in_dead_body(&self) -> bool;
    /// `isWorn()`, `isEquipped()`.
    fn is_worn(&self) -> bool;
    fn is_equipped(&self) -> bool;
    /// `getCondition()`, `getConditionMax()`.
    fn condition(&self) -> i32;
    fn condition_max(&self) -> i32;
    /// `DrainableComboItem.isFullUses()`, `isEmptyUses()`.
    fn is_full_uses(&self) -> bool;
    fn is_empty_uses(&self) -> bool;
    /// `InventoryContainer.isEmpty()`.
    fn container_is_empty(&self) -> bool;
    /// `isFluidContainer()`.
    fn is_fluid_container(&self) -> LuaResult<bool>;
}

/// `InputScript.containsItem(item)`: `itemScriptCache.contains(item)`. The
/// original's `OnPostWorldDictionaryInit` adds the items of each input tag
/// (`ItemTags.getItemsForTag`) to that cache after the listed items; the
/// port's cache has only the listed items yet (TODO(ItemTags) in
/// `InputScript::resolve_items`), so the tags are tested here.
pub fn contains_item(input: &InputScript, script_item: &Rc<RefCell<Item>>) -> bool {
    input.possible_input_items().iter().any(|s| s.downcast::<Item>().is_some_and(|s| Rc::ptr_eq(&s, script_item)))
        || is_tag_item(input, script_item)
}

/// An item the input accepts through one of its tags (see `contains_item`).
pub fn is_tag_item(input: &InputScript, script_item: &Rc<RefCell<Item>>) -> bool {
    !input.is_accepts_any_item() && {
        let script_item = script_item.borrow();
        input.item_tags().iter().flatten().any(|t| script_item.has_tag(Some(t)))
    }
}

/// `CraftRecipe.OnTestItem(item, chr)` with a character: true without an
/// `OnTest` function, otherwise the function's result (`on_test` calls it
/// with the item's key).
pub fn on_test_item(recipe: &CraftRecipe, item: usize, on_test: &mut dyn FnMut(&str, usize) -> bool) -> bool {
    match recipe.lua_call_string(LuaCall::OnTest) {
        Some(name) => on_test(name, item),
        None => true,
    }
}

/// `doesItemPassRoutineStatusTests(item, chr)` with its `passes*Test`
/// helpers.
fn passes_routine_status_tests(
    input: &InputScript,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
) -> LuaResult<bool> {
    if chr.is_player() && item.is_no_recipes()? {
        return Ok(false);
    }
    // passesFavoriteTest
    if item.is_favorite() && !input.allow_favorites() {
        return Ok(false);
    }
    if let Some(food) = item.food() {
        // passesRottenTest, passesFrozenTest
        if food.rotten && !input.has_flag(InputFlag::AllowRottenItem) {
            return Ok(false);
        }
        if food.frozen && !input.has_flag(InputFlag::AllowFrozenItem) {
            return Ok(false);
        }
    }
    // passesBrokenTest
    if item.is_broken() && !input.has_flag(InputFlag::AllowDestroyedItem) {
        return Ok(false);
    }
    // passesSealedTest: `getName().contains(Translator.getFluidText("Fluid_Sealed"))`.
    if input.has_flag(InputFlag::IsSealed) || input.has_flag(InputFlag::IsNotSealed) {
        return Err(unsupported("InputScript.passesSealedTest"));
    }
    Ok(true)
}

/// `doesItemPassClothingTypeStatusTests`.
fn passes_clothing_type_status_tests(input: &InputScript, item: &dyn CraftItem) -> bool {
    if item.in_dead_body() {
        return true;
    }
    if input.has_flag(InputFlag::IsWorn) && !item.is_worn() {
        return false;
    }
    !(input.has_flag(InputFlag::IsNotWorn) && (item.is_worn() || item.is_equipped()))
}

/// `doesItemPassSharpnessStatusTests`: both flags read the item's
/// `Sharpness` attribute first.
fn passes_sharpness_status_tests(input: &InputScript) -> LuaResult<bool> {
    if input.has_flag(InputFlag::IsNotDull) || input.has_flag(InputFlag::IsSharpenable) {
        return Err(unsupported("InputScript.doesItemPassSharpnessStatusTests"));
    }
    Ok(true)
}

/// `doesItemPassDamageStatusTests` (`isDamaged`: condition below its
/// maximum).
fn passes_damage_status_tests(input: &InputScript, item: &dyn CraftItem) -> bool {
    let damaged = item.condition() < item.condition_max();
    if input.has_flag(InputFlag::IsDamaged) && !damaged {
        return false;
    }
    !(input.has_flag(InputFlag::IsUndamaged) && damaged)
}

/// `doesItemPassIsOrNotEmptyAndFullTests`. Without one of the flags it
/// tests, every branch passes, so the item is not read then.
fn passes_is_or_not_empty_and_full_tests(input: &InputScript, item: &dyn CraftItem) -> LuaResult<bool> {
    let (is_full, is_empty, not_full, not_empty) = (
        input.has_flag(InputFlag::IsFull),
        input.has_flag(InputFlag::IsEmpty),
        input.has_flag(InputFlag::NotFull),
        input.has_flag(InputFlag::NotEmpty),
    );
    if !(is_full || is_empty || not_full || not_empty || input.has_flag(InputFlag::ItemIsEnergy)) {
        return Ok(true);
    }
    match item.class() {
        CraftItemClass::Container => {
            if is_empty && !item.container_is_empty() {
                return Ok(false);
            }
        }
        CraftItemClass::Drainable => {
            if is_full && !item.is_full_uses()
                || is_empty && !item.is_empty_uses()
                || not_full && item.is_full_uses()
                || not_empty && item.is_empty_uses()
            {
                return Ok(false);
            }
        }
        CraftItemClass::Radio => {
            // `getDeviceData().getIsBatteryPowered()`, then its power.
            return Err(unsupported("InputScript.doesItemPassIsOrNotEmptyAndFullTests (Radio)"));
        }
        CraftItemClass::WeaponPart => {
            let (uses, max) = (item.current_uses(), item.max_uses());
            if is_full && uses != max || is_empty && uses > 0 || not_full && uses == max || not_empty && uses <= 0 {
                return Ok(false);
            }
        }
        CraftItemClass::Other => {}
    }
    if (is_full || is_empty) && item.is_fluid_container()? {
        // `FluidContainer.isFull()`, `isEmpty()`.
        return Err(unsupported("InputScript.doesItemPassIsOrNotEmptyAndFullTests (FluidContainer)"));
    }
    if input.has_flag(InputFlag::ItemIsEnergy) {
        if item.class() != CraftItemClass::Drainable {
            return Ok(false);
        }
        return Err(unsupported("DrainableComboItem.isEnergy"));
    }
    Ok(true)
}

/// `doesItemPassFoodAndCookingTests`.
fn passes_food_and_cooking_tests(input: &InputScript, item: &dyn CraftItem) -> LuaResult<bool> {
    let Some(food) = item.food() else {
        return Ok(true);
    };
    if input.has_flag(InputFlag::IsWholeFoodItem) {
        // `Food.isWholeFoodItem` compares with a newly created item.
        return Err(unsupported("Food.isWholeFoodItem"));
    }
    let uncooked = !food.cooked && !food.burnt;
    if input.has_flag(InputFlag::IsUncookedFoodItem) && food.cookable && !uncooked {
        return Ok(false);
    }
    Ok(!(input.has_flag(InputFlag::IsCookedFoodItem) && food.cookable && uncooked))
}

/// `CraftRecipeManager.consumeInputItemInternal(input, item, true, cache,
/// chr)`. The empty-container check (`IsEmptyContainer`) only logs, and the
/// cache's `addAppliedItem` only records the item.
fn consume_input_item_internal(
    input: &InputScript,
    recipe: &CraftRecipe,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
    on_test: &mut dyn FnMut(&str, usize) -> bool,
) -> LuaResult<bool> {
    let Some(script_item) = item.script_item() else {
        return Ok(false);
    };
    if !contains_item(input, &script_item) {
        return Ok(false);
    }
    // `input.getParentRecipe().OnTestItem(item, chr)`.
    if !on_test_item(recipe, item.key(), on_test) {
        return Ok(false);
    }
    // Each failed test is logged (`DebugType.CraftLogic`).
    Ok(passes_routine_status_tests(input, item, chr)?
        && passes_clothing_type_status_tests(input, item)
        && passes_sharpness_status_tests(input)?
        && passes_damage_status_tests(input, item)
        && passes_is_or_not_empty_and_full_tests(input, item)?
        && passes_food_and_cooking_tests(input, item)?)
}

/// `CraftRecipeManager.consumeInputItem(input, item, true, cache, chr)` for
/// an `Item` input (`validateInputScript` is false for other inputs; its
/// exceptions are for debug builds and non-test calls on a client).
pub fn consume_input_item_test(
    input: &InputScript,
    recipe: &CraftRecipe,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
    on_test: &mut dyn FnMut(&str, usize) -> bool,
) -> LuaResult<bool> {
    if input.resource_type() != ResourceType::Item {
        return Ok(false);
    }
    if !consume_input_item_internal(input, recipe, item, chr, on_test)? {
        return Ok(false);
    }
    let queued = chr.item_queued_amount(item)?;
    if queued == f32::MAX || !input.has_consume_from_item() && item.current_uses() < queued as i32 {
        return Ok(false);
    }
    if input.has_consume_from_item() {
        // `consumeInputItemUsesInternal`, `consumeInputFluidInternal` or
        // `consumeInputEnergyFromItemInternal`.
        return Err(unsupported("CraftRecipeManager.consumeInputItem (ConsumeFromItem)"));
    }
    if input.has_create_to_item() {
        // `createOutputItemUsesInternal` and the others.
        return Err(unsupported("CraftRecipeManager.consumeInputItem (CreateToItem)"));
    }
    // `cache.setMoveToOutputs(isKeep)` only records the input mode.
    Ok(true)
}
