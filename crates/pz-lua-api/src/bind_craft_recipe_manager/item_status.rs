//! The item side of the craft checks: `InputScript.containsItem`, the
//! `doesItemPass*Tests` status tests, and `CraftRecipeManager.consumeInputItem`
//! for an `Item` input in its test-only form (`testOnly == true`), which
//! `isItemValidForInputScript`, `InputScriptData.acceptsInputItem` and
//! `CraftRecipeData.consumeInputFromItems` call.
//!
//! The reads the port does not have yet return `unsupported` when a flag
//! makes the original read them: sharpness and head condition (the item's
//! attribute container), the sealed-name test (`Translator.getFluidText`),
//! `Food.isWholeFoodItem`, radio device power, the full and empty tests of
//! fluid containers, energy items (`DrainableComboItem.isEnergy`), and the
//! `CreateToItem` lines.

use super::RecipeCharacter;
use crate::bind_world_object::unsupported;
use pz_kahlua::LuaResult;
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, InputFlag, InputScript, LuaCall};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::objects::items::Item;
use std::cell::{Cell, RefCell};
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

/// A fluid of a `FluidContainer`: `getFluidTypeString()` and the
/// `FluidCategory` names it is in (`isCategory`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FluidId {
    pub name: String,
    pub categories: Vec<String>,
}

/// What the craft checks read from an item's `FluidContainer`.
#[derive(Clone, Debug, Default)]
pub struct FluidState {
    /// `getAmount()`, `getCapacity()`.
    pub amount: f32,
    pub capacity: f32,
    /// The `FluidInstance`s in order: the fluid (`None` for a null one) and
    /// its amount.
    pub fluids: Vec<(Option<FluidId>, f32)>,
}

impl FluidState {
    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.amount == 0.0
    }

    /// `isFull()`: `PZMath.equal(getAmount(), capacity, 1.0E-4f)`.
    pub fn is_full(&self) -> bool {
        (self.capacity - self.amount).abs() < 1.0e-4
    }

    /// `isMixture()`.
    fn is_mixture(&self) -> bool {
        self.fluids.len() > 1
    }

    /// `getPrimaryFluid()`: none when empty, else the first fluid with the
    /// largest amount.
    fn primary_fluid(&self) -> Option<&FluidId> {
        if self.is_empty() {
            return None;
        }
        let mut best = self.fluids.first()?;
        for f in &self.fluids {
            if f.1 > best.1 {
                best = f;
            }
        }
        best.0.as_ref()
    }
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
    /// `getFluidContainer()`; `None` without one.
    fn fluid_container(&self) -> LuaResult<Option<FluidState>>;
}

/// Whether `Fluid.Get(name)` finds a fluid (the registry is keyed by the
/// exact type string).
fn is_registered_fluid(name: &str) -> LuaResult<bool> {
    let env = crate::bind_items::item_env();
    let Some(ctx) = env.entity() else {
        return Err(unsupported("Fluid.Get without the fluid registry"));
    };
    match ctx.fluid_by_name(name) {
        pz_entity::Lookup::Found(_) => Ok(true),
        pz_entity::Lookup::Missing => Ok(false),
        pz_entity::Lookup::Unknown => Err(unsupported("Fluid.Get without the fluid registry")),
    }
}

/// `InputScript.containsFluid(fluid)`: `acceptsAnyFluid`, or the input's
/// whitelist `fluidFilter` (the listed fluids that `Fluid.Get` finds, and
/// the listed categories) allows the fluid. An empty part of the filter
/// allows every fluid.
fn contains_fluid(input: &InputScript, fluid: Option<&FluidId>) -> LuaResult<bool> {
    let Some(fluid) = fluid else {
        return Ok(false);
    };
    if input.is_accepts_any_fluid() {
        return Ok(true);
    }
    let mut names = Vec::new();
    for name in input.loaded_fluids() {
        if is_registered_fluid(name)? {
            names.push(name);
        }
    }
    let by_name = names.is_empty() || names.iter().any(|n| **n == fluid.name);
    let categories = input.categories();
    let by_category = categories.is_empty() || categories.iter().any(|c| fluid.categories.contains(c));
    Ok(by_name && by_category)
}

/// `InputScript.isFluidMatch(fluidContainer)`.
fn is_fluid_match(input: &InputScript, fc: &FluidState) -> LuaResult<bool> {
    if input.resource_type() != ResourceType::Fluid || fc.is_empty() {
        return Ok(false);
    }
    if input.is_fluid_exact() {
        return Ok(!fc.is_mixture() && contains_fluid(input, fc.primary_fluid())?);
    }
    if input.is_fluid_mixture() {
        // `createFluidSample()`: every fluid of the container.
        for (f, _) in &fc.fluids {
            if !contains_fluid(input, f.as_ref())? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if input.is_fluid_primary() {
        return contains_fluid(input, fc.primary_fluid());
    }
    Ok(true)
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
///
/// `cache` is the `fluidConsumed` of the `CacheData` (an
/// `InputScriptData`), or `None` for a null cache. A failed test
/// `softReset`s the cache.
pub fn consume_input_item_test(
    input: &InputScript,
    recipe: &CraftRecipe,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
    on_test: &mut dyn FnMut(&str, usize) -> bool,
    cache: Option<&Cell<f32>>,
) -> LuaResult<bool> {
    if input.resource_type() != ResourceType::Item {
        return Ok(false);
    }
    let ok = consume_item_tests(input, recipe, item, chr, on_test, cache)?;
    if !ok {
        if let Some(c) = cache {
            c.set(0.0);
        }
    }
    Ok(ok)
}

fn consume_item_tests(
    input: &InputScript,
    recipe: &CraftRecipe,
    item: &dyn CraftItem,
    chr: &dyn RecipeCharacter,
    on_test: &mut dyn FnMut(&str, usize) -> bool,
    cache: Option<&Cell<f32>>,
) -> LuaResult<bool> {
    if !consume_input_item_internal(input, recipe, item, chr, on_test)? {
        return Ok(false);
    }
    let queued = chr.item_queued_amount(item)?;
    if queued == f32::MAX || !input.has_consume_from_item() && item.current_uses() < queued as i32 {
        return Ok(false);
    }
    if let Some(from) = input.consume_from_item_script() {
        let from = io_input(recipe, from)?;
        let ok = match from.resource_type() {
            ResourceType::Item => consume_input_item_uses(from, item, queued as i32),
            ResourceType::Fluid => consume_input_fluid(from, recipe, item.fluid_container()?, cache, queued)?,
            ResourceType::Energy => consume_input_energy_from_item(from, item, queued as i32)?,
            ResourceType::Any => true,
        };
        if !ok {
            return Ok(false);
        }
    }
    if input.has_create_to_item() {
        // `createOutputItemUsesInternal` and the others.
        return Err(unsupported("CraftRecipeManager.consumeInputItem (CreateToItem)"));
    }
    // `cache.setMoveToOutputs(isKeep)` only records the input mode.
    Ok(true)
}

/// The input line at `index` of the recipe's `ioLines`.
fn io_input(recipe: &CraftRecipe, index: usize) -> LuaResult<&InputScript> {
    recipe
        .io_lines()
        .get(index)
        .and_then(|l| l.as_input())
        .ok_or_else(|| pz_kahlua::LuaError::runtime("java.lang.NullPointerException"))
}

/// `consumeInputItemUsesInternal(from, item, true, cache, queued)`: a
/// `DrainableComboItem` the line lists with the line's uses left over the
/// queued uses.
fn consume_input_item_uses(from: &InputScript, item: &dyn CraftItem, queued: i32) -> bool {
    if item.class() != CraftItemClass::Drainable {
        return false;
    }
    if !item.script_item().is_some_and(|s| contains_item(from, &s)) {
        return false;
    }
    if from.has_flag(InputFlag::IsFull) && !item.is_full_uses() {
        return false;
    }
    let amount = if from.amount() > 0.0 { from.amount() as i32 } else { 0 };
    item.current_uses() >= amount + queued
}

/// `consumeInputFluidInternal(from, fluidContainer, true, cache, queued)`.
fn consume_input_fluid(
    from: &InputScript,
    recipe: &CraftRecipe,
    fc: Option<FluidState>,
    cache: Option<&Cell<f32>>,
    queued: f32,
) -> LuaResult<bool> {
    let Some(fc) = fc.filter(|fc| !fc.is_empty()) else {
        return Ok(false);
    };
    if from.has_flag(InputFlag::IsFull) && !fc.is_full() {
        return Ok(false);
    }
    let mut amount = from.amount();
    let parent = match from.parent_script() {
        Some(i) => io_input(recipe, i)?,
        None => from,
    };
    if parent.is_item_count() && fc.amount < amount + queued {
        return Ok(false);
    }
    if let Some(c) = cache {
        amount = from.amount() - c.get();
    }
    if !is_fluid_match(from, &fc)? {
        return Ok(false);
    }
    let consumed = amount.min((fc.amount - queued).max(0.0));
    if consumed < EPSILON {
        return Ok(false);
    }
    if let Some(c) = cache {
        c.set(c.get() + consumed);
    }
    Ok(true)
}

/// `consumeInputEnergyFromItemInternal(from, item, true, cache, queued)`.
fn consume_input_energy_from_item(from: &InputScript, item: &dyn CraftItem, queued: i32) -> LuaResult<bool> {
    if item.class() != CraftItemClass::Drainable {
        return Ok(false);
    }
    if from.has_flag(InputFlag::IsFull) && !item.is_full_uses() {
        return Ok(false);
    }
    let amount = if from.amount() > 0.0 { from.amount() as i32 } else { 1 };
    if item.current_uses() < amount + queued {
        return Ok(false);
    }
    // `isEnergyMatch(item)`: `DrainableComboItem.isEnergy()` and its energy.
    Err(unsupported("DrainableComboItem.isEnergy"))
}

const EPSILON: f32 = 0.00001;
