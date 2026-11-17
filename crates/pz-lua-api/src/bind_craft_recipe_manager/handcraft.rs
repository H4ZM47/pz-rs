//! The part of `HandcraftLogic` (with its `BaseCraftingLogic` and
//! `CraftRecipeData`) that `CraftRecipeManager.getUniqueRecipeItems` runs
//! for each recipe: `setRecipeFromContextClick(recipe, item)` and
//! `canPerformCurrentRecipe()`, for a logic made with
//! `new HandcraftLogic(chr, null, null)` and the context menu's containers.
//!
//! That logic has no craft bench, so its `CraftRecipeData` is a
//! `CraftMode.Handcraft` one that takes input items and no input resources,
//! and its source resources are empty. Everything here is the test-only
//! path (`testOnly == true`); the overfill and variable-ratio bookkeeping
//! that only consuming uses is left out.

use super::item_status::{consume_input_item_test, is_tag_item, CraftItem};
use super::RecipeCharacter;
use crate::bind_world_object::unsupported;
use pz_kahlua::LuaResult;
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, InputFlag, InputScript};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::objects::items::Item;
use std::collections::HashSet;

/// `CraftRecipeData` with its `InputScriptData` list, for one recipe.
struct RecipeData<'a> {
    recipe: &'a CraftRecipe,
    inputs: Vec<&'a InputScript>,
    /// `InputScriptData.inputItems` per input, as indices into `items`.
    input_items: Vec<Vec<usize>>,
    /// `usedItems` (item keys).
    used_items: HashSet<usize>,
    items: &'a [&'a dyn CraftItem],
    chr: &'a dyn RecipeCharacter,
}

const EPSILON: f32 = 0.00001;

impl<'a> RecipeData<'a> {
    /// `setRecipe(recipe)` on a fresh `CraftRecipeData`: one
    /// `InputScriptData` per input.
    fn new(recipe: &'a CraftRecipe, items: &'a [&'a dyn CraftItem], chr: &'a dyn RecipeCharacter) -> Self {
        let inputs: Vec<_> = recipe.inputs().collect();
        RecipeData {
            recipe,
            input_items: vec![Vec::new(); inputs.len()],
            inputs,
            used_items: HashSet::new(),
            items,
            chr,
        }
    }

    fn item(&self, i: usize) -> &'a dyn CraftItem {
        self.items[i]
    }

    fn consume_input_item(&self, input: usize, item: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        consume_input_item_test(self.inputs[input], self.recipe, self.item(item), self.chr, on_test)
    }

    /// The full names `getPossibleInputItems()` gives, in order, for the
    /// items offered: the listed items, then (see `contains_item`) the items
    /// of the input tags, which the port's cache does not hold yet. Item
    /// types that none of the offered items has add nothing to the amounts,
    /// so only those are listed.
    fn possible_input_items(&self, input: usize, list: &[usize]) -> Vec<String> {
        let input = self.inputs[input];
        let mut names: Vec<String> = input
            .possible_input_items()
            .iter()
            .filter_map(|s| s.downcast::<Item>())
            .map(|s| s.borrow().full_name())
            .collect();
        for &i in list {
            let Some(script) = self.item(i).script_item() else {
                continue;
            };
            let name = script.borrow().full_name();
            if is_tag_item(input, &script) && !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }

    /// `consumeInputFromItems(input, items, true, data, clearUsed, chr)`.
    fn consume_input_from_items(
        &mut self,
        input: usize,
        list: &[usize],
        clear_used: bool,
        on_test: &mut OnTest,
    ) -> LuaResult<bool> {
        let script = self.inputs[input];
        if script.resource_type() != ResourceType::Item || list.is_empty() {
            return Ok(false);
        }
        if script.has_consume_from_item() {
            // The amounts come from the `ConsumeFromItem` line (fluids).
            return Err(unsupported("CraftRecipeData.consumeInputFromItems (ConsumeFromItem)"));
        }
        let (amount, max_amount) = (script.int_amount() as f32, script.int_max_amount() as f32);
        if amount <= 0.0 {
            return Ok(false);
        }
        if clear_used {
            self.used_items.clear();
        }
        let mut used: Vec<usize> = Vec::new();
        let (mut left, mut max_left) = (amount, max_amount);
        for name in self.possible_input_items(input, list) {
            for &i in list {
                let item = self.item(i);
                if item.script_item().map(|s| s.borrow().full_name()).as_deref() != Some(name.as_str()) {
                    continue;
                }
                if self.used_items.contains(&item.key()) || used.contains(&item.key()) {
                    continue;
                }
                if !self.consume_input_item(input, i, on_test)? {
                    continue;
                }
                used.push(item.key());
                let uses = if script.is_item_count() { 1.0 } else { item.current_uses() as f32 };
                let queued = self.chr.item_queued_amount(item)?;
                if queued == f32::MAX {
                    continue;
                }
                let uses = (uses - queued).max(0.0);
                let scaled = uses / script.relative_scale(&item.full_type());
                let uses = scaled.min(uses);
                left -= uses;
                max_left -= uses;
                if max_left <= EPSILON {
                    break;
                }
            }
            if left <= EPSILON {
                self.used_items.extend(used);
                return Ok(true);
            }
            if script.is_exclusive() {
                (left, max_left) = (amount, max_amount);
                used.clear();
            }
        }
        Ok(false)
    }

    /// `InputScriptData.isInputItemsSatisfied()`.
    fn is_satisfied(&mut self, input: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if self.inputs[input].resource_type() != ResourceType::Item || self.input_items[input].is_empty() {
            return Ok(false);
        }
        let list = self.input_items[input].clone();
        self.consume_input_from_items(input, &list, true, on_test)
    }

    /// `InputScriptData.isInputItemsSatisifiedToMaximum()`. The target
    /// ratio is `Float.MAX_VALUE` (never set here), so the maximum is
    /// `getIntMaxAmount()`, and test calls leave no overfilled items.
    fn is_satisfied_to_maximum(&mut self, input: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if !self.is_satisfied(input, on_test)? {
            return Ok(false);
        }
        let script = self.inputs[input];
        let maximum = (script.amount() * f32::MAX).min(script.int_max_amount() as f32) as i32;
        Ok(self.used_items.len() as i32 >= maximum)
    }

    /// `InputScriptData.acceptsInputItem(item)`.
    fn accepts_input_item(&self, input: usize, item: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if self.inputs[input].resource_type() != ResourceType::Item {
            return Ok(false);
        }
        if self.input_items[input].contains(&item) {
            return Ok(false);
        }
        self.consume_input_item(input, item, on_test)
    }

    /// `InputScriptData.addInputItem(item)` (a non-`Item` input only logs).
    fn add_input_item(&mut self, input: usize, item: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if self.inputs[input].resource_type() != ResourceType::Item || !self.accepts_input_item(input, item, on_test)? {
            return Ok(false);
        }
        if self.is_satisfied(input, on_test)? && self.is_satisfied_to_maximum(input, on_test)? {
            return Ok(false);
        }
        self.input_items[input].push(item);
        Ok(true)
    }

    /// `canOfferInputItem(input, item, false)`: the input is the recipe's,
    /// so it has its data.
    fn can_offer_input_item(&self, input: usize, item: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        self.accepts_input_item(input, item, on_test)
    }

    /// `containsInputItem(item)`.
    fn contains_input_item(&self, item: usize) -> bool {
        (0..self.inputs.len())
            .any(|i| self.inputs[i].resource_type() == ResourceType::Item && self.input_items[i].contains(&item))
    }

    fn first_type_differs(&self, input: usize, item: usize) -> bool {
        self.input_items[input].first().is_some_and(|&f| self.item(f).full_type() != self.item(item).full_type())
    }

    /// `offerInputItem(input, item, false)`.
    fn offer_input_item(&mut self, input: usize, item: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if !self.can_offer_input_item(input, item, on_test)? || self.contains_input_item(item) {
            return Ok(false);
        }
        if self.inputs[input].is_exclusive() && self.first_type_differs(input, item) {
            return Ok(false);
        }
        if self.is_satisfied(input, on_test)? && self.is_satisfied_to_maximum(input, on_test)? {
            return Ok(false);
        }
        self.add_input_item(input, item, on_test)
    }

    /// `offerAndReplaceInputItem(item)`: the first input that can take the
    /// item gets it, replacing what an exclusive input holds of another
    /// type, or the last item of a full input.
    fn offer_and_replace_input_item(&mut self, item: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        for input in 0..self.inputs.len() {
            if !self.can_offer_input_item(input, item, on_test)? || self.contains_input_item(item) {
                continue;
            }
            if self.inputs[input].is_exclusive() && self.first_type_differs(input, item) {
                self.input_items[input].clear();
            }
            if self.is_satisfied(input, on_test)? && self.is_satisfied_to_maximum(input, on_test)? {
                self.input_items[input].pop();
            }
            if self.add_input_item(input, item, on_test)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `populateInputs(chr, items, resources, false)`: each input that is
    /// not satisfied is offered the items in order until it is.
    fn populate_inputs(&mut self, all: &[usize], on_test: &mut OnTest) -> LuaResult<()> {
        for input in 0..self.inputs.len() {
            if self.is_satisfied(input, on_test)? {
                continue;
            }
            for &item in all {
                self.offer_input_item(input, item, on_test)?;
                if self.is_satisfied(input, on_test)? {
                    break;
                }
            }
        }
        Ok(())
    }

    /// `consumeRecipeInputs(true, resources, null, true, false, chr)` with
    /// no resources: every input that applies to hand crafting must be
    /// covered by the items it holds.
    fn consume_recipe_inputs(&mut self, on_test: &mut OnTest) -> LuaResult<bool> {
        self.used_items.clear();
        let mut failed = false;
        for input in 0..self.inputs.len() {
            let script = self.inputs[input];
            // HandcraftOnly lines apply (`CraftMode.Handcraft`).
            if script.has_flag(InputFlag::AutomationOnly) || script.is_apply_on_tick() {
                continue;
            }
            let list = self.input_items[input].clone();
            let consumed = !list.is_empty()
                && script.resource_type() == ResourceType::Item
                && self.consume_input_from_items(input, &list, false, on_test)?;
            // The original goes on to the next inputs (`cachedCanConsume`).
            failed |= !consumed;
        }
        Ok(!failed)
    }
}

type OnTest<'f> = dyn FnMut(&str, usize) -> bool + 'f;

/// `setRecipeFromContextClick(recipe, item)` then `canPerformCurrentRecipe()`
/// on the context menu's `HandcraftLogic`. `items` holds the containers'
/// items (`allItems`) followed by the clicked item when it is not one of
/// them; `clicked` is its index.
///
/// `setRecipeFromContextClick` also calls `canConsumeInputs`, whose result
/// it drops; `canPerform` repeats `isValidRecipeForCharacter`, which the
/// caller has just passed with the same arguments. The original tests
/// `tooDarkToRead` before the inputs; it only reads state, so this tests the
/// inputs first and reads it only for a recipe whose inputs are covered.
/// The outputs (`createRecipeOutputs`), then the craft surface
/// (`findCraftSurface`, `isCharacterInRangeOfWorkbench`) and the
/// workstation come after that and are not ported.
pub fn can_perform_from_context_click(
    recipe: &CraftRecipe,
    items: &[&dyn CraftItem],
    all_items: usize,
    clicked: usize,
    chr: &dyn RecipeCharacter,
    on_test: &mut OnTest,
) -> LuaResult<bool> {
    let mut data = RecipeData::new(recipe, items, chr);
    data.offer_and_replace_input_item(clicked, on_test)?;
    let all: Vec<usize> = (0..all_items).collect();
    data.populate_inputs(&all, on_test)?;
    if !data.consume_recipe_inputs(on_test)? {
        return Ok(false);
    }
    // `consumeInputsInternal`: a recipe that cannot be done in the dark
    // fails while the player is `tooDarkToRead`.
    if !recipe.can_be_done_in_dark() && chr.is_player() && chr.too_dark_to_read()? {
        return Ok(false);
    }
    // `createOutputsInternal(true, null, chr)`: `createRecipeOutputs`.
    Err(unsupported(format!("CraftRecipeData.createRecipeOutputs ({})", recipe.name().unwrap_or_default())))
}
