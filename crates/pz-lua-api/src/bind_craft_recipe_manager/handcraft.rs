//! `CraftRecipeData` for a `CraftMode.Handcraft` logic without a craft bench
//! (it takes input items and no input resources, and gives output items),
//! with its `InputScriptData` list: the input checks, consuming the inputs,
//! and creating the outputs.
//!
//! `DataState` is the data's state, with the items as keys
//! (`CraftItem::key`); `RecipeData` runs the original's methods over it for
//! a list of items that holds every item the state names. The context menu
//! (`CraftRecipeManager.getUniqueRecipeItems`) and the cached recipe info use
//! a fresh state per recipe; the crafting window's logic keeps one.
//!
//! Not ported, and reported with `unsupported` when reached: the overfill
//! bookkeeping of variable inputs when consuming, output mappers that match
//! input items, output chances, the output item flags (`IsBlunt`,
//! `HasOneUse`, `HasNoUses`, `SetActivated`, `EquipSecondary`), the
//! `Inherit*` input flags, and `ResearchInput`.

use super::item_status::{consume_input_item, is_tag_item, CraftItem, InputCache};
use super::RecipeCharacter;
use crate::bind_world_object::unsupported;
use pz_kahlua::{LuaError, LuaResult};
use pz_scripts::objects::entity::components::crafting::{
    CraftRecipe, InputFlag, InputScript, OutputFlag, OutputScript,
};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::objects::items::Item;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `CraftRecipeData.InputScriptData`.
#[derive(Clone, Debug, Default)]
pub(crate) struct InputData {
    /// `inputItems` (item keys).
    pub items: Vec<usize>,
    /// Its `CacheData`.
    pub cache: InputCache,
    /// `cachedCanConsume`.
    pub cached_can_consume: bool,
}

/// An entry of the `toOutputItems` `ItemDataList`.
#[derive(Clone, Debug)]
pub(crate) enum OutputItem {
    /// A test entry: the script item (`addItem(Item)`).
    Script(Rc<RefCell<Item>>),
    /// An item to create (`InventoryItemFactory.CreateItem` in
    /// `createOutputItemInternal`), not processed.
    Create(Rc<RefCell<Item>>),
    /// A kept input item (`collectKeepItems`), processed.
    Kept(usize),
    /// An item `Create` made, not processed (the crafting logic makes it).
    Created(usize),
}

/// The state of a `CraftRecipeData` set to a recipe.
#[derive(Clone, Debug)]
pub(crate) struct DataState {
    pub inputs: Vec<InputData>,
    /// `usedItems`.
    pub used_items: Vec<usize>,
    /// `consumedUsedItems`.
    pub consumed_used_items: Vec<usize>,
    /// `allViableItems`.
    pub all_viable_items: Vec<usize>,
    pub has_consumed_inputs: bool,
    pub has_tested_inputs: bool,
    /// `targetVariableInputRatio`.
    pub target_ratio: f32,
    /// `calculatedVariableInputRatio`.
    pub calculated_ratio: f32,
    /// `toOutputItems`.
    pub to_output: Vec<OutputItem>,
}

impl DataState {
    /// `setRecipe(recipe)` on a data without that recipe: one
    /// `InputScriptData` per input.
    pub fn new(recipe: &CraftRecipe) -> Self {
        DataState {
            inputs: recipe.inputs().map(|_| InputData::default()).collect(),
            used_items: Vec::new(),
            consumed_used_items: Vec::new(),
            all_viable_items: Vec::new(),
            has_consumed_inputs: false,
            has_tested_inputs: false,
            target_ratio: f32::MAX,
            calculated_ratio: f32::MAX,
            to_output: Vec::new(),
        }
    }

    /// A data with no recipe (`clearRecipe()`), keeping
    /// `consumedUsedItems`, which only the constructor clears.
    pub fn empty(consumed_used_items: Vec<usize>) -> Self {
        DataState {
            inputs: Vec::new(),
            used_items: Vec::new(),
            consumed_used_items,
            all_viable_items: Vec::new(),
            has_consumed_inputs: false,
            has_tested_inputs: false,
            target_ratio: f32::MAX,
            calculated_ratio: f32::MAX,
            to_output: Vec::new(),
        }
    }

    /// `clearCaches()`.
    pub fn clear_caches(&mut self) {
        for d in &mut self.inputs {
            d.cache.clear();
        }
        self.has_consumed_inputs = false;
        self.has_tested_inputs = false;
        self.used_items.clear();
        self.to_output.clear();
    }

    /// Every item key the state names.
    pub fn keys(&self) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        let mut add = |k: usize| {
            if !out.contains(&k) {
                out.push(k);
            }
        };
        for d in &self.inputs {
            d.items.iter().chain(&d.cache.applied).for_each(|&k| add(k));
        }
        self.used_items.iter().chain(&self.consumed_used_items).chain(&self.all_viable_items).for_each(|&k| add(k));
        for o in &self.to_output {
            if let OutputItem::Kept(k) | OutputItem::Created(k) = o {
                add(*k);
            }
        }
        out
    }
}

pub(crate) const EPSILON: f32 = 0.00001;

/// The `Inherit*` input flags that `createOutputItems` reads from the first
/// applied item of an input with the flag.
const INHERIT_FLAGS: [InputFlag; 18] = [
    InputFlag::InheritColor,
    InputFlag::InheritModelVariation,
    InputFlag::InheritCondition,
    InputFlag::IsHeadPart,
    InputFlag::InheritHeadCondition,
    InputFlag::InheritEquipped,
    InputFlag::InheritSharpness,
    InputFlag::InheritUses,
    InputFlag::InheritUsesAndEmpty,
    InputFlag::InheritFavorite,
    InputFlag::InheritAmmunition,
    InputFlag::CopyClothing,
    InputFlag::InheritWeight,
    InputFlag::InheritFreezingTime,
    InputFlag::InheritName,
    InputFlag::InheritFood,
    InputFlag::InheritCooked,
    InputFlag::InheritFoodAge,
];

/// `CraftRecipeData` over a `DataState`, for the items in `items`.
pub(crate) struct RecipeData<'a> {
    recipe: &'a CraftRecipe,
    inputs: Vec<&'a InputScript>,
    pub st: DataState,
    items: &'a [&'a dyn CraftItem],
    index: HashMap<usize, usize>,
    chr: &'a dyn RecipeCharacter,
}

impl<'a> RecipeData<'a> {
    /// A fresh data set to the recipe.
    pub fn new(recipe: &'a CraftRecipe, items: &'a [&'a dyn CraftItem], chr: &'a dyn RecipeCharacter) -> Self {
        Self::with_state(recipe, items, chr, DataState::new(recipe))
    }

    /// The data in `st`; `items` must hold every item `st` names.
    pub fn with_state(
        recipe: &'a CraftRecipe,
        items: &'a [&'a dyn CraftItem],
        chr: &'a dyn RecipeCharacter,
        st: DataState,
    ) -> Self {
        let index = items.iter().enumerate().map(|(i, item)| (item.key(), i)).collect();
        RecipeData { recipe, inputs: recipe.inputs().collect(), st, items, index, chr }
    }

    pub fn item(&self, key: usize) -> &'a dyn CraftItem {
        self.items[self.index[&key]]
    }

    /// `inputs[input].getConsumeFromItemScript()`.
    fn consume_from_item_script(&self, input: usize) -> Option<&'a InputScript> {
        self.inputs[input].consume_from_item_script().and_then(|i| self.recipe.io_lines().get(i)?.as_input())
    }

    /// `CraftRecipeManager.consumeInputItem(input, item, test, cache, chr)`,
    /// with the input's data as the cache or a null cache.
    fn consume_input_item(
        &mut self,
        input: usize,
        key: usize,
        on_test: &mut OnTest,
        with_cache: bool,
        test: bool,
    ) -> LuaResult<bool> {
        let item = self.item(key);
        let cache = if with_cache { Some(&mut self.st.inputs[input].cache) } else { None };
        consume_input_item(self.inputs[input], self.recipe, item, self.chr, on_test, cache, test)
    }

    /// The item types `getPossibleInputItems()` gives, in order, for the
    /// items offered: the listed items, then (see `contains_item`) the items
    /// of the input tags, which the port's cache does not hold yet. Item
    /// types that none of the offered items has add nothing to the amounts,
    /// so only those are listed. The original matches types by
    /// `getFullName()`; the `ScriptManager` holds one script per full name,
    /// so the scripts are compared instead.
    fn possible_input_items(&self, input: usize, list: &[usize]) -> Vec<Rc<RefCell<Item>>> {
        let input = self.inputs[input];
        let offered: Vec<_> = list.iter().filter_map(|&k| self.item(k).script_item()).collect();
        let has = |types: &[Rc<RefCell<Item>>], s: &Rc<RefCell<Item>>| types.iter().any(|t| Rc::ptr_eq(t, s));
        let mut types: Vec<_> = input
            .possible_input_items()
            .iter()
            .filter_map(|s| s.downcast::<Item>())
            .filter(|s| has(&offered, s))
            .collect();
        for script in offered {
            if is_tag_item(input, &script) && !has(&types, &script) {
                types.push(script);
            }
        }
        types
    }

    /// `consumeInputFromItems(input, items, test, cache, clearUsed, chr)`.
    fn consume_input_from_items(
        &mut self,
        input: usize,
        list: &[usize],
        test: bool,
        with_cache: bool,
        clear_used: bool,
        on_test: &mut OnTest,
    ) -> LuaResult<bool> {
        let script = self.inputs[input];
        if script.resource_type() != ResourceType::Item || list.is_empty() {
            return Ok(false);
        }
        // With a `ConsumeFromItem` line, the amounts are that line's.
        let from = self.consume_from_item_script(input);
        let (amount, max_amount) = match from {
            Some(from) => (from.amount(), from.max_amount()),
            None => (script.int_amount() as f32, script.int_max_amount() as f32),
        };
        if amount <= 0.0 {
            return Ok(false);
        }
        if clear_used {
            self.st.used_items.clear();
        }
        let (mut used, mut viable, mut consumed, mut overfilled) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut left, mut max_left) = (amount, max_amount);
        let from_fluid = from.is_some_and(|f| f.resource_type() == ResourceType::Fluid);
        for item_type in self.possible_input_items(input, list) {
            for &k in list {
                let item = self.item(k);
                if !item.script_item().is_some_and(|s| Rc::ptr_eq(&s, &item_type)) {
                    continue;
                }
                if self.st.used_items.contains(&k) || used.contains(&k) {
                    continue;
                }
                if !self.consume_input_item(input, k, on_test, with_cache, test)? {
                    continue;
                }
                used.push(k);
                if !self.st.all_viable_items.contains(&k) {
                    viable.push(k);
                }
                if !test {
                    if left > EPSILON {
                        consumed.push(k);
                    } else {
                        overfilled.push(k);
                    }
                }
                let uses = if script.is_item_count() { 1.0 } else { item.current_uses() as f32 };
                let queued = if test { self.chr.item_queued_amount(item)? } else { 0.0 };
                if queued == f32::MAX {
                    continue;
                }
                let mut uses = (uses - queued).max(0.0);
                let mut scaled = uses / script.relative_scale(&item.full_type());
                // A fluid `ConsumeFromItem` line counts the item's fluid
                // over the queued amount. (Consuming one is not ported:
                // `consume_input_item` has failed for it.)
                if from_fluid {
                    let Some(fc) = item.fluid_container()? else {
                        return Err(LuaError::runtime("java.lang.NullPointerException"));
                    };
                    uses = (fc.amount - queued).max(0.0);
                    scaled = uses;
                }
                let uses = scaled.min(uses);
                left -= uses;
                max_left -= uses;
                if max_left <= EPSILON {
                    break;
                }
            }
            if left <= EPSILON {
                self.st.used_items.extend(used);
                self.st.consumed_used_items.extend(consumed);
                self.st.all_viable_items.extend(viable);
                if script.is_variable_amount() {
                    let ratio = (amount - left).min(max_amount) / amount;
                    self.st.calculated_ratio = self.st.calculated_ratio.min(ratio);
                    // `variableInputOverfilledItems.put(input, overfilled)`:
                    // only consuming reads it, and that is not ported (see
                    // `consume_recipe_inputs`).
                }
                return Ok(true);
            }
            if script.is_exclusive() {
                (left, max_left) = (amount, max_amount);
                used.clear();
                viable.clear();
                consumed.clear();
                overfilled.clear();
            }
        }
        Ok(false)
    }

    /// `InputScriptData.isInputItemsSatisfied()`.
    pub fn is_satisfied(&mut self, input: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if self.inputs[input].resource_type() != ResourceType::Item || self.st.inputs[input].items.is_empty() {
            return Ok(false);
        }
        let list = self.st.inputs[input].items.clone();
        self.consume_input_from_items(input, &list, true, false, true, on_test)
    }

    /// `InputScriptData.isInputItemsSatisifiedToMaximum()`: with the target
    /// ratio at `Float.MAX_VALUE`, the maximum is `getIntMaxAmount()`.
    fn is_satisfied_to_maximum(&mut self, input: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if !self.is_satisfied(input, on_test)? {
            return Ok(false);
        }
        let script = self.inputs[input];
        let maximum = (script.amount() * self.st.target_ratio).min(script.int_max_amount() as f32) as i32;
        Ok(self.st.used_items.len() as i32 >= maximum)
    }

    /// `InputScriptData.acceptsInputItem(item)`.
    fn accepts_input_item(&mut self, input: usize, key: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if self.inputs[input].resource_type() != ResourceType::Item {
            return Ok(false);
        }
        if self.st.inputs[input].items.contains(&key) {
            return Ok(false);
        }
        self.consume_input_item(input, key, on_test, false, true)
    }

    /// `InputScriptData.addInputItem(item)` (a non-`Item` input only logs).
    fn add_input_item(&mut self, input: usize, key: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if self.inputs[input].resource_type() != ResourceType::Item || !self.accepts_input_item(input, key, on_test)? {
            return Ok(false);
        }
        if self.is_satisfied(input, on_test)? && self.is_satisfied_to_maximum(input, on_test)? {
            return Ok(false);
        }
        self.st.inputs[input].items.push(key);
        Ok(true)
    }

    /// `canOfferInputItem(input, item, false)`: the input is the recipe's,
    /// so it has its data.
    fn can_offer_input_item(&mut self, input: usize, key: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        self.accepts_input_item(input, key, on_test)
    }

    /// `containsInputItem(item)`.
    pub fn contains_input_item(&self, key: usize) -> bool {
        (0..self.inputs.len())
            .any(|i| self.inputs[i].resource_type() == ResourceType::Item && self.st.inputs[i].items.contains(&key))
    }

    fn first_type_differs(&self, input: usize, key: usize) -> bool {
        self.st.inputs[input].items.first().is_some_and(|&f| self.item(f).full_type() != self.item(key).full_type())
    }

    /// `offerInputItem(input, item, false)`.
    fn offer_input_item(&mut self, input: usize, key: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        if !self.can_offer_input_item(input, key, on_test)? || self.contains_input_item(key) {
            return Ok(false);
        }
        if self.inputs[input].is_exclusive() && self.first_type_differs(input, key) {
            return Ok(false);
        }
        if self.is_satisfied(input, on_test)? && self.is_satisfied_to_maximum(input, on_test)? {
            return Ok(false);
        }
        self.add_input_item(input, key, on_test)
    }

    /// `offerAndReplaceInputItem(item)`: the first input that can take the
    /// item gets it, replacing what an exclusive input holds of another
    /// type, or the last item of a full input.
    fn offer_and_replace_input_item(&mut self, key: usize, on_test: &mut OnTest) -> LuaResult<bool> {
        for input in 0..self.inputs.len() {
            if !self.can_offer_input_item(input, key, on_test)? || self.contains_input_item(key) {
                continue;
            }
            if self.inputs[input].is_exclusive() && self.first_type_differs(input, key) {
                self.st.inputs[input].items.clear();
            }
            if self.is_satisfied(input, on_test)? && self.is_satisfied_to_maximum(input, on_test)? {
                self.st.inputs[input].items.pop();
            }
            if self.add_input_item(input, key, on_test)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `populateInputs(chr, items, resources, clear)`: with `clear`, each
    /// input first drops its items (last first); each input that is not
    /// satisfied is offered the items in order until it is.
    pub fn populate_inputs(&mut self, all: &[usize], clear: bool, on_test: &mut OnTest) -> LuaResult<()> {
        for input in 0..self.inputs.len() {
            if clear {
                self.st.inputs[input].items.clear();
            }
            if self.is_satisfied(input, on_test)? {
                continue;
            }
            for &key in all {
                self.offer_input_item(input, key, on_test)?;
                if self.is_satisfied(input, on_test)? {
                    break;
                }
            }
        }
        Ok(())
    }

    /// `isVariableAmount()`: an input has a variable amount.
    fn is_variable_amount(&self) -> bool {
        self.inputs.iter().any(|i| i.is_variable_amount())
    }

    /// `consumeRecipeInputs(test, resources, items, continueOnFail,
    /// clearViable, chr)` with no resources (the logic has no craft bench,
    /// so `allowInputResources` is false): every input that applies to hand
    /// crafting must be covered by `items`, or by the items it holds when
    /// `items` is null.
    fn consume_recipe_inputs(
        &mut self,
        test: bool,
        items: Option<&[usize]>,
        continue_on_fail: bool,
        clear_viable: bool,
        on_test: &mut OnTest,
    ) -> LuaResult<bool> {
        self.st.used_items.clear();
        if clear_viable {
            self.st.all_viable_items.clear();
        }
        self.st.calculated_ratio = if self.is_variable_amount() { self.st.target_ratio } else { 1.0 };
        let mut failed = false;
        for input in 0..self.inputs.len() {
            let script = self.inputs[input];
            // `HandcraftOnly` lines apply (`CraftMode.Handcraft`).
            if script.has_flag(InputFlag::AutomationOnly) || script.is_apply_on_tick() {
                continue;
            }
            let list = match items {
                Some(items) => items.to_vec(),
                None => self.st.inputs[input].items.clone(),
            };
            let consumed = !list.is_empty()
                && script.resource_type() == ResourceType::Item
                && self.consume_input_from_items(input, &list, test, true, false, on_test)?;
            self.st.inputs[input].cached_can_consume = consumed;
            if !consumed {
                failed = true;
                if !continue_on_fail {
                    break;
                }
            }
        }
        if failed {
            return Ok(false);
        }
        if !test && self.is_variable_amount() {
            // The overfilled items of variable inputs go back to the
            // character's inventory.
            return Err(unsupported("CraftRecipeData.consumeRecipeInputs (variable input overfill)"));
        }
        Ok(true)
    }

    /// `consumeInputsInternal(chr, test, resources, items, continueOnFail,
    /// clearViable)`.
    pub fn consume_inputs_internal(
        &mut self,
        test: bool,
        items: Option<&[usize]>,
        continue_on_fail: bool,
        clear_viable: bool,
        on_test: &mut OnTest,
    ) -> LuaResult<bool> {
        if self.st.has_consumed_inputs {
            return Ok(true);
        }
        if !self.recipe.can_be_done_in_dark() && self.chr.is_player() && self.chr.too_dark_to_read()? {
            return Ok(false);
        }
        self.st.clear_caches();
        let r = self.consume_recipe_inputs(test, items, continue_on_fail, clear_viable, on_test)?;
        if r {
            self.st.has_consumed_inputs = !test;
            self.st.has_tested_inputs = test;
        }
        Ok(r)
    }

    /// `createOutputsInternal(test, null, chr)`.
    pub fn create_outputs_internal(&mut self, test: bool) -> LuaResult<bool> {
        let consumed = if test { self.st.has_tested_inputs || self.st.has_consumed_inputs } else { self.st.has_consumed_inputs };
        if !consumed {
            if crate::bind_core::shared_core().borrow().debug {
                let message = if test {
                    "(test) createOutputs requires consumeInputs to be called first"
                } else {
                    "createOutputs requires consumeInputs to be called first"
                };
                return Err(LuaError::Runtime { message: message.into(), class: "java.lang.RuntimeException" });
            }
            return Ok(false);
        }
        let r = self.create_recipe_outputs(test)?;
        if test {
            self.st.has_tested_inputs = false;
        } else {
            self.st.has_consumed_inputs = false;
        }
        if !test && r {
            // `destroyAllSurvivingDestroyInputs()`.
            if self.destroy_input_items().next().is_some() {
                return Err(unsupported("CraftRecipeData.destroyAllSurvivingDestroyInputs (ItemUser.RemoveItem)"));
            }
        }
        Ok(r)
    }

    /// `getAllDestroyInputItems()`.
    fn destroy_input_items(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.inputs.len())
            .filter(|&i| self.inputs[i].resource_type() == ResourceType::Item && self.inputs[i].is_destroy())
            .flat_map(|i| self.st.inputs[i].cache.applied.iter().copied())
    }

    /// `getFirstInputItemWithFlag(flag)`: the first applied item of the first
    /// `Item` input with the flag that has one.
    fn first_input_item_with_flag(&self, flag: InputFlag) -> Option<usize> {
        (0..self.inputs.len())
            .filter(|&i| self.inputs[i].resource_type() == ResourceType::Item && self.inputs[i].has_flag(flag))
            .find_map(|i| self.st.inputs[i].cache.applied.first().copied())
    }

    /// `createRecipeOutputs(test, null, chr)`.
    fn create_recipe_outputs(&mut self, test: bool) -> LuaResult<bool> {
        self.st.to_output.clear();
        let mut failed = false;
        for output in self.recipe.outputs() {
            // `HandcraftOnly` lines apply (`CraftMode.Handcraft`).
            if output.has_flag(OutputFlag::AutomationOnly) || output.is_apply_on_tick() {
                continue;
            }
            let created = output.resource_type() == ResourceType::Item && self.create_output_items(output, test)?;
            // `allowOutputResources` is false.
            if !created {
                failed = true;
                break;
            }
        }
        if failed {
            return Ok(false);
        }
        self.collect_keep_items();
        // `distributeItemsToResources` needs `allowOutputResources`, and
        // `allowOutputItems` takes the unprocessed items.
        if self.first_input_item_with_flag(InputFlag::ResearchInput).is_some() {
            // `InventoryItem.getResearchableRecipes(chr)` and
            // `IsoGameCharacter.researchRecipe`.
            return Err(unsupported("CraftRecipeData.createRecipeOutputs (ResearchInput)"));
        }
        Ok(true)
    }

    /// `OutputScript.getItem(data)`: `OutputMapper.getOutputItem(data)`.
    fn output_item(&self, output: &OutputScript) -> LuaResult<Option<Rc<RefCell<Item>>>> {
        let Some(mapper) = self.recipe.output_mapper_for(output) else {
            return Ok(None);
        };
        if !mapper.entrees().is_empty() {
            // `OutputEntree.matchItem` against the input items.
            return Err(unsupported("OutputMapper.getOutputItem (entrees)"));
        }
        Ok(mapper.default_output_entree().and_then(|e| e.result.as_ref()?.downcast::<Item>()))
    }

    /// `createOutputItems(output, toOutputItems, test, data, chr)`.
    fn create_output_items(&mut self, output: &OutputScript, test: bool) -> LuaResult<bool> {
        if output.int_amount() <= 0 {
            return Ok(false);
        }
        let Some(item) = self.output_item(output)? else {
            return Ok(false);
        };
        let mut count = output.int_amount();
        if output.is_variable_amount() {
            let ratio = if test { self.st.calculated_ratio } else { self.st.target_ratio };
            count = (((output.amount() * ratio) as f64).ceil() as i32).min(output.int_max_amount());
        }
        if test {
            for _ in 0..count {
                self.st.to_output.push(OutputItem::Script(item.clone()));
            }
            return Ok(true);
        }
        if self.recipe.has_tag("RemoveResultItems") {
            return Ok(true);
        }
        if count <= 0 {
            return Ok(true);
        }
        // `CraftRecipeManager.createOutputItem`: a chance below 1 rolls
        // `Rand.Next`, and these flags change the new item.
        if output.chance() < 1.0 {
            return Err(unsupported("CraftRecipeManager.createOutputItem (chance)"));
        }
        let flags = [
            OutputFlag::IsBlunt,
            OutputFlag::HasOneUse,
            OutputFlag::HasNoUses,
            OutputFlag::SetActivated,
            OutputFlag::EquipSecondary,
        ];
        if let Some(f) = flags.iter().find(|f| output.has_flag(**f)) {
            return Err(unsupported(format!("CraftRecipeManager.createOutputItem ({f:?})")));
        }
        if let Some(f) = INHERIT_FLAGS.iter().find(|f| self.first_input_item_with_flag(**f).is_some()) {
            return Err(unsupported(format!("CraftRecipeData.createOutputItems ({f:?})")));
        }
        for _ in 0..count {
            self.st.to_output.push(OutputItem::Create(item.clone()));
        }
        Ok(true)
    }

    /// `collectKeepItems(toOutputItems, test)`: the applied items of the
    /// inputs that move them to the outputs, as processed entries.
    fn collect_keep_items(&mut self) {
        for i in 0..self.inputs.len() {
            let d = &self.st.inputs[i];
            if self.inputs[i].resource_type() != ResourceType::Item || !d.cache.move_to_outputs {
                continue;
            }
            let kept: Vec<_> = d.cache.applied.iter().map(|&k| OutputItem::Kept(k)).collect();
            self.st.to_output.extend(kept);
        }
    }
}

pub(crate) type OnTest<'f> = dyn FnMut(&str, usize) -> bool + 'f;

/// The parts of `CraftRecipeData` the crafting window's logic uses.
impl<'a> RecipeData<'a> {
    /// `getInputs()`.
    pub fn input_scripts(&self) -> &[&'a InputScript] {
        &self.inputs
    }

    /// `CraftRecipeManager.isValidRecipeForCharacter(recipe, chr, monitor,
    /// containers)`.
    pub fn is_valid(&self, has_containers: bool) -> LuaResult<bool> {
        super::is_valid_recipe_for_character(self.recipe, Some(self.chr), has_containers)
    }

    /// `canPerform(chr, resources, items, continueOnFail, containers)`.
    pub fn can_perform(
        &mut self,
        items: Option<&[usize]>,
        continue_on_fail: bool,
        has_containers: bool,
        on_test: &mut OnTest,
    ) -> LuaResult<bool> {
        if !self.is_valid(has_containers)? {
            return Ok(false);
        }
        Ok(self.consume_inputs_internal(true, items, continue_on_fail, false, on_test)?
            && self.create_outputs_internal(true)?)
    }

    /// `getPossibleCraftCount(resources, items, outResources, outItems,
    /// manual)` without manual input selection (the items `getInputCraftCount`
    /// lists are only read for it) and with no resources
    /// (`getResourceCraftCount` gives 0).
    pub fn possible_craft_count(&mut self, items: &[usize], manual: bool, on_test: &mut OnTest) -> LuaResult<i32> {
        let mut count = 100;
        for input in 0..self.inputs.len() {
            count = count.min(self.input_craft_count(input, items, manual, on_test)?);
            if count == 0 {
                break;
            }
        }
        Ok(count)
    }

    /// `getInputCraftCount(data, items, outItems, manual)`. With manual
    /// input selection only the item types the input holds count.
    fn input_craft_count(&mut self, input: usize, items: &[usize], manual: bool, on_test: &mut OnTest) -> LuaResult<i32> {
        let script = self.inputs[input];
        let from = self.consume_from_item_script(input);
        let from_fluid = from.is_some_and(|f| f.resource_type() == ResourceType::Fluid);
        let mut total = 0f64;
        for &key in items {
            // `InputScript.canUseItem(item, chr)`.
            if !self.consume_input_item(input, key, on_test, false, true)? {
                continue;
            }
            if manual && !self.has_applied_item_type(input, key) {
                continue;
            }
            let item = self.item(key);
            let scale = script.relative_scale(&item.full_type()) as f64;
            let uses = if script.is_item_count() { 1 } else { item.current_uses() };
            let mut d = uses as f64 / scale;
            if !script.is_item_count() {
                if from_fluid {
                    let Some(fc) = item.fluid_container()? else {
                        return Err(LuaError::runtime("java.lang.NullPointerException"));
                    };
                    d = fc.amount as f64;
                }
                if script.has_create_to_item() {
                    // `getFluidContainer().getFreeCapacity()` over the
                    // `CreateToItem` line's amount.
                    return Err(unsupported("CraftRecipeData.getInputCraftCount (CreateToItem)"));
                }
            }
            total += d;
        }
        // The original adds the double constant of `(double) 1.0E-5f`.
        total += 9.999999747378752e-6;
        if script.is_keep() && from.is_none() && !script.has_create_to_item() {
            return Ok(if total >= script.int_amount() as f64 { 100 } else { 0 });
        }
        let mut amount = script.amount();
        if !script.is_item_count() {
            if let Some(from) = from.filter(|_| from_fluid) {
                amount = from.amount();
            }
        }
        Ok((total / amount as f64).floor() as i32)
    }

    /// `InputScriptData.hasAppliedItemType(item.getScriptItem())`: an
    /// applied item of the input has the same script item.
    fn has_applied_item_type(&self, input: usize, key: usize) -> bool {
        let script = self.item(key).script_item();
        self.st.inputs[input].cache.applied.iter().any(|&k| match (self.item(k).script_item(), &script) {
            (Some(a), Some(b)) => Rc::ptr_eq(&a, b),
            (None, None) => true,
            _ => false,
        })
    }

    /// `getAppliedInputItemTypes(set)`: the full types of `getAllInputItems`.
    pub fn applied_input_item_types(&self) -> Vec<String> {
        self.all_input_items().into_iter().map(|k| self.item(k).full_type()).collect()
    }

    /// The items whose full type is one of `types`
    /// (`BaseCraftingLogic.populateInputs` with manual input item types).
    pub fn items_of_types(&self, keys: &[usize], types: &[String]) -> Vec<usize> {
        keys.iter().copied().filter(|&k| types.contains(&self.item(k).full_type())).collect()
    }

    /// `InputScriptData.verifyInputItems(allItems)`, last item first: an
    /// item stays only for a keep input, when `allItems` has it and the
    /// input still accepts it.
    pub fn verify_input_items(&mut self, input: usize, all: &[usize], on_test: &mut OnTest) -> LuaResult<()> {
        for i in (0..self.st.inputs[input].items.len()).rev() {
            let key = self.st.inputs[input].items[i];
            let ok = self.inputs[input].resource_type() == ResourceType::Item
                && self.consume_input_item(input, key, on_test, false, true)?;
            if self.inputs[input].is_keep() && all.contains(&key) && ok {
                continue;
            }
            self.st.inputs[input].items.remove(i);
        }
        Ok(())
    }

    /// The applied items of the `Item` inputs that pass `keep`, in order.
    fn applied_items(&self, keep: impl Fn(&InputScript) -> bool) -> Vec<usize> {
        (0..self.inputs.len())
            .filter(|&i| self.inputs[i].resource_type() == ResourceType::Item && keep(self.inputs[i]))
            .flat_map(|i| self.st.inputs[i].cache.applied.iter().copied())
            .collect()
    }

    /// `getAllInputItems()`.
    pub fn all_input_items(&self) -> Vec<usize> {
        self.applied_items(|_| true)
    }

    /// `getAllPutBackInputItems()`.
    pub fn put_back_input_items(&self) -> Vec<usize> {
        self.applied_items(|i| !i.has_flag(InputFlag::DontPutBack))
    }

    /// `getAllConsumedItems(list, false, false)`: the keep inputs' items
    /// are left out.
    pub fn consumed_items(&self) -> Vec<usize> {
        self.applied_items(|i| !i.is_keep())
    }
}

/// `setRecipeFromContextClick(recipe, item)` then `canPerformCurrentRecipe()`
/// on the context menu's `HandcraftLogic`. `items` holds the containers'
/// items (`allItems`) followed by the clicked item when it is not one of
/// them; `clicked` is its index.
///
/// `setRecipeFromContextClick` turns manual input selection on, so
/// `canPerform` consumes from the items the inputs hold; it also calls
/// `canConsumeInputs`, whose result it drops. `canPerform` repeats
/// `isValidRecipeForCharacter`, which the caller has just passed with the
/// same arguments. The craft surface (`findCraftSurface`,
/// `isCharacterInRangeOfWorkbench`) and the workstation are not ported.
pub fn can_perform_from_context_click(
    recipe: &CraftRecipe,
    items: &[&dyn CraftItem],
    all_items: usize,
    clicked: usize,
    chr: &dyn RecipeCharacter,
    on_test: &mut OnTest,
) -> LuaResult<bool> {
    let mut data = RecipeData::new(recipe, items, chr);
    data.offer_and_replace_input_item(items[clicked].key(), on_test)?;
    let all: Vec<usize> = items[..all_items].iter().map(|i| i.key()).collect();
    data.populate_inputs(&all, false, on_test)?;
    data.consume_inputs_internal(true, None, false, false, on_test)?;
    if !data.consume_inputs_internal(true, None, true, false, on_test)? || !data.create_outputs_internal(true)? {
        return Ok(false);
    }
    if recipe.is_any_surface_craft() {
        return Err(unsupported("HandcraftLogic.isCharacterInRangeOfWorkbench (context menu)"));
    }
    if recipe.requires_specific_workstation() {
        return Err(unsupported("BaseCraftingLogic.hasRequiredWorkstation"));
    }
    Ok(true)
}

/// `CraftRecipeData.canPerform(chr, sourceResources, allItems, false,
/// null)` on a `testRecipeData` just set to the recipe, the way
/// `BaseCraftingLogic.createCachedRecipeInfo` calls it: every input that
/// applies to hand crafting is consumed, in test mode, from the supplied
/// item list (`items`), and the outputs are made in test mode.
///
/// The caller has already tested `isValidRecipeForCharacter`.
pub fn can_perform_with_items(
    recipe: &CraftRecipe,
    items: &[&dyn CraftItem],
    chr: &dyn RecipeCharacter,
    on_test: &mut OnTest,
) -> LuaResult<bool> {
    let mut data = RecipeData::new(recipe, items, chr);
    let all: Vec<usize> = items.iter().map(|i| i.key()).collect();
    Ok(data.consume_inputs_internal(true, Some(&all), false, false, on_test)? && data.create_outputs_internal(true)?)
}
