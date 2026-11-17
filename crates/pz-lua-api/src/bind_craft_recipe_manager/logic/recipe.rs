//! The selected recipe of a `HandcraftLogic`: its `CraftRecipeData` (the
//! `handcraft` engine over the `DataState` the logic keeps), what the
//! crafting window reads from it (the craft count, the input amounts, can
//! perform), the craft action and `performCurrentRecipe`, and the
//! `CraftRecipeData` and `CraftRecipeData.InputScriptData` Lua objects.
//!
//! Not ported, and reported with `unsupported` when reached: offering and
//! removing manual input items, the input item nodes, the recipe-at-hand items of a recipe
//! that needs learning, a floor container that `updateFloorContainer` has
//! to change, vehicle part containers, `CraftRecipeData.addXP`, keep inputs
//! in `processDestroyAndUsedItems`, and `PlayerCraftHistory.cleanupHistory`.

use super::super::handcraft::{DataState, OnTest, OutputItem, RecipeData, EPSILON};
use super::super::item_status::CraftItem;
use super::super::{call_on_test, container_items, is_recipe_actually_known, LuaCraftItem, LuaRecipeCharacter};
use super::{class_cast, logic, npe, object, recipe_arg, state, trigger_event, CRAFT_RECIPE_DATA_CLASS};
use crate::bind_character_sim::CharacterObject;
use crate::bind_items::ItemRef;
use crate::bind_java_util::JavaList;
use crate::bind_scripts::{script_list, script_value, ScriptHandle};
use crate::bind_world_object::unsupported;
use crate::expose::{expose_class, this, ClassBuilder};
use pz_characters::character::save::CraftHistoryEntry;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, InputFlag, InputScript, LuaCall};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::{ScriptRef, ScriptType};
use std::any::Any;
use std::rc::Rc;

const INPUT_DATA_CLASS: &str = "zombie.entity.components.crafting.recipe.CraftRecipeData$InputScriptData";
const ISO_THUMPABLE: &str = "zombie.iso.objects.IsoThumpable";

/// The logic's `CraftRecipeData`.
pub(super) struct RecipeDataObject(pub Rc<dyn LuaObject>);

impl LuaObject for RecipeDataObject {
    fn class_name(&self) -> &str {
        CRAFT_RECIPE_DATA_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `CraftRecipeData.InputScriptData` of the logic's data: the index of
/// its input in `getInputs()`.
struct InputDataObject {
    logic: Rc<dyn LuaObject>,
    input: usize,
}

impl LuaObject for InputDataObject {
    fn class_name(&self) -> &str {
        INPUT_DATA_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An item's key in the `DataState` (`CraftItem::key`).
fn key(item: &ItemRef) -> usize {
    Rc::as_ptr(item) as *const () as usize
}

fn character_object(v: &Value) -> LuaResult<&CharacterObject> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<CharacterObject>().ok_or_else(class_cast),
        _ => Err(npe()),
    }
}

/// `obj.name(args)` through the object's Lua binding.
fn call(t: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
    let f = t.table_get(obj, &Value::str(name))?;
    if f.is_nil() {
        return Err(LuaError::runtime(format!(
            "HandcraftLogic: {}.{name} is not bound",
            pz_kahlua::stdlib::java_class_name(obj)
        )));
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

fn instance_of(v: &Value, class: &str) -> bool {
    match v {
        Value::Object(o) => o.class_name() == class || o.super_classes().contains(&class),
        _ => false,
    }
}

fn number(v: &Value) -> LuaResult<f64> {
    match v {
        Value::Num(n) => Ok(*n),
        _ => Err(LuaError::runtime("expected a number")),
    }
}

/// The logic's items: `allItems` then the items its data holds.
fn known_items(o: &Rc<dyn LuaObject>) -> Vec<ItemRef> {
    let st = state(o).borrow();
    let mut refs = st.all_items.clone();
    for i in &st.held {
        if !refs.iter().any(|r| Rc::ptr_eq(r, i)) {
            refs.push(i.clone());
        }
    }
    refs
}

fn items_for_keys(o: &Rc<dyn LuaObject>, keys: &[usize]) -> Vec<ItemRef> {
    let known = known_items(o);
    keys.iter().filter_map(|&k| known.iter().find(|i| key(i) == k).cloned()).collect()
}

fn item_list(items: &[ItemRef]) -> LuaResult<Value> {
    let values = items.iter().map(crate::bind_items::try_item_value).collect::<LuaResult<Vec<_>>>()?;
    Ok(Value::Object(Rc::new(JavaList::new(values))))
}

/// The selected recipe.
fn selected(o: &Rc<dyn LuaObject>) -> Option<ScriptRef> {
    state(o).borrow().recipe.clone()
}

/// Runs `f` on the logic's `CraftRecipeData` (set to its recipe) over the
/// logic's items, the items the data holds and `extra`; `f` gets the keys
/// of `allItems`. The data's state is stored back, with the items it names.
pub(super) fn with_data<R>(
    thread: &mut Thread,
    o: &Rc<dyn LuaObject>,
    extra: &[ItemRef],
    f: impl FnOnce(&mut RecipeData<'_>, &[usize], &mut OnTest<'_>) -> LuaResult<R>,
) -> LuaResult<R> {
    let (script, data, all_items, player) = {
        let st = state(o).borrow();
        let script = st.recipe.clone().ok_or_else(npe)?;
        (script, st.data.clone(), st.all_items.clone(), st.player.clone())
    };
    let mut refs = known_items(o);
    for i in extra {
        if !refs.iter().any(|r| Rc::ptr_eq(r, i)) {
            refs.push(i.clone());
        }
    }
    let obj = object(o);
    let character = character_object(&player)?;
    let chr = LuaRecipeCharacter {
        sm: &obj.sm,
        chars: &obj.chars,
        character,
        value: player.clone(),
        env: thread.globals().clone(),
    };
    let items = refs.iter().map(|i| LuaCraftItem::new(&obj.sm, i.clone(), &player)).collect::<LuaResult<Vec<_>>>()?;
    let values: Vec<(usize, Value)> = items.iter().map(|i| (i.key(), i.value.clone())).collect();
    let all: Vec<usize> = all_items.iter().map(key).collect();
    let dyn_items: Vec<&dyn CraftItem> = items.iter().map(|i| i as &dyn CraftItem).collect();
    let recipe = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
    let recipe = recipe.borrow();
    let mut on_test = |name: &str, k: usize| {
        let item = values.iter().find(|(x, _)| *x == k).map_or(Value::Nil, |(_, v)| v.clone());
        call_on_test(thread, name, &item, &player)
    };
    let mut data = RecipeData::with_state(&recipe, &dyn_items, &chr, data);
    let r = f(&mut data, &all, &mut on_test);
    let new_state = data.st;
    let keys = new_state.keys();
    let mut st = state(o).borrow_mut();
    st.held = refs.into_iter().filter(|i| keys.contains(&key(i))).collect();
    st.data = new_state;
    r
}

/// Runs `f` with the logic's character.
fn with_character<R>(
    thread: &mut Thread,
    o: &Rc<dyn LuaObject>,
    f: impl FnOnce(&LuaRecipeCharacter<'_>) -> LuaResult<R>,
) -> LuaResult<R> {
    let player = state(o).borrow().player.clone();
    let obj = object(o);
    let chr = LuaRecipeCharacter {
        sm: &obj.sm,
        chars: &obj.chars,
        character: character_object(&player)?,
        value: player.clone(),
        env: thread.globals().clone(),
    };
    f(&chr)
}

/// The index in `getInputs()` of the data `getDataForInputScript(input)`
/// gives: the input itself, or the input whose `ConsumeFromItem` line it
/// is.
fn input_index(recipe: &CraftRecipe, v: &Value) -> LuaResult<Option<usize>> {
    let h = match v {
        Value::Nil => return Ok(None),
        Value::Object(o) => o.as_any().downcast_ref::<ScriptHandle>().ok_or_else(class_cast)?,
        _ => return Err(class_cast()),
    };
    let ptr = h.target_ptr()?;
    let at = |i: &InputScript| i as *const InputScript as *const () as usize;
    let inputs: Vec<&InputScript> = recipe.inputs().collect();
    if let Some(i) = inputs.iter().position(|i| at(i) == ptr) {
        return Ok(Some(i));
    }
    Ok(inputs.iter().position(|i| {
        i.consume_from_item_script()
            .and_then(|l| recipe.io_lines().get(l)?.as_input())
            .is_some_and(|from| at(from) == ptr)
    }))
}

/// `recipe.containsIO(line)`.
fn contains_io(recipe: &CraftRecipe, v: &Value) -> LuaResult<bool> {
    let Value::Object(o) = v else {
        return Ok(false);
    };
    let Some(h) = o.as_any().downcast_ref::<ScriptHandle>() else {
        return Ok(false);
    };
    let ptr = h.target_ptr()?;
    Ok(recipe.io_lines().iter().any(|io| {
        let p = match (io.as_input(), io.as_output()) {
            (Some(i), _) => i as *const InputScript as *const () as usize,
            (_, Some(o)) => o as *const _ as *const () as usize,
            _ => 0,
        };
        p == ptr
    }))
}

/// `BaseCraftingLogic.setRecipe(recipe)`, after `HandcraftLogic.setRecipe`
/// has saved it as the last selected recipe.
pub(super) fn base_set_recipe(thread: &mut Thread, o: &Rc<dyn LuaObject>, recipe: Option<ScriptRef>) -> LuaResult<()> {
    {
        // `recipeData.setRecipe(recipe)`: `clearRecipe`, then one
        // `InputScriptData` per input. `consumedUsedItems` stays.
        let mut st = state(o).borrow_mut();
        let consumed = std::mem::take(&mut st.data.consumed_used_items);
        st.data = match &recipe {
            Some(r) => {
                let r = r.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
                let mut d = DataState::new(&r.borrow());
                d.consumed_used_items = consumed;
                d
            }
            None => DataState::empty(consumed),
        };
        st.recipe = recipe.clone();
    }
    if recipe.is_some() {
        // `recipeData.canConsumeInputs(sourceResources, allItems, true,
        // true)`.
        with_data(thread, o, &[], |d, all, t| d.consume_inputs_internal(true, Some(all), true, true, t))?;
    }
    // `inputItemNodeCollection.setRecipe(recipe)` builds the nodes that
    // only the manual input item list reads.
    state(o).borrow_mut().manual_input_allowed_item_types.clear();
    auto_populate_inputs(thread, o)?;
    trigger_event(thread, o, "onRecipeChanged", &[script_value(recipe.as_ref())]);
    trigger_event(thread, o, "onRebuildInputItemNodes", &[]);
    Ok(())
}

/// `HandcraftLogic.getPossibleCraftCount(true)`: the data's count for
/// `allItems`, cached.
fn recalculate_possible_craft_count(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<i32> {
    let count = if selected(o).is_some() {
        let manual = state(o).borrow().manual_select_inputs;
        let (count, consumed) = with_data(thread, o, &[], |d, all, t| {
            let mut consumed = Vec::new();
            Ok((d.possible_craft_count(all, manual, &mut consumed, t)?, consumed))
        })?;
        state(o).borrow_mut().multicraft_consumed_items = consumed;
        count
    } else {
        // `CraftRecipeData.getPossibleCraftCount` with no inputs.
        100
    };
    state(o).borrow_mut().cached_possible_craft_count = count;
    Ok(count)
}

/// `autoPopulateInputs()`: the viable items fill the inputs, the inputs
/// are tested (`canConsumeInputs(resources, null, true, false)`), the craft
/// count is recalculated, then `onInputsChanged`.
pub(super) fn auto_populate_inputs(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    if selected(o).is_some() {
        // `populateInputs(player, getAllViableInputInventoryItems(),
        // sourceResources, false)`.
        let types = state(o).borrow().manual_input_allowed_item_types.clone();
        with_data(thread, o, &[], |d, _, t| {
            let viable = d.st.all_viable_items.clone();
            let viable = if types.is_empty() { viable } else { d.items_of_types(&viable, &types) };
            d.populate_inputs(&viable, false, t)?;
            d.consume_inputs_internal(true, None, true, false, t)
        })?;
        state(o).borrow_mut().cached_can_perform_dirty = true;
    }
    recalculate_possible_craft_count(thread, o)?;
    trigger_event(thread, o, "onInputsChanged", &[]);
    Ok(())
}

/// `BaseCraftingLogic.populateInputs(player, items, resources, true)`.
fn populate_inputs_clearing(thread: &mut Thread, o: &Rc<dyn LuaObject>, items: &[ItemRef]) -> LuaResult<()> {
    let keys: Vec<usize> = items.iter().map(key).collect();
    let types = state(o).borrow().manual_input_allowed_item_types.clone();
    with_data(thread, o, items, |d, _, t| {
        let keys = if types.is_empty() { keys } else { d.items_of_types(&keys, &types) };
        d.populate_inputs(&keys, true, t)
    })?;
    state(o).borrow_mut().cached_can_perform_dirty = true;
    Ok(())
}

/// `setContainers(containers)` with a recipe, once the items changed: the
/// inputs keep the items they had that are still there.
pub(super) fn set_containers_with_recipe(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    let input_items = with_data(thread, o, &[], |d, all, t| {
        let items = d.all_input_items();
        d.consume_inputs_internal(true, Some(all), true, true, t)?;
        Ok(items.into_iter().filter(|k| all.contains(k)).collect::<Vec<_>>())
    })?;
    let items = items_for_keys(o, &input_items);
    populate_inputs_clearing(thread, o, &items)
}

/// `manualInputAllowedItemTypes.addAll(types)`: a set.
fn add_allowed_item_types(o: &Rc<dyn LuaObject>, types: Vec<String>) {
    let mut st = state(o).borrow_mut();
    for t in types {
        if !st.manual_input_allowed_item_types.contains(&t) {
            st.manual_input_allowed_item_types.push(t);
        }
    }
}

/// `refresh()`.
fn refresh(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    if selected(o).is_none() {
        return Ok(());
    }
    let input_items = with_data(thread, o, &[], |d, all, t| {
        for input in 0..d.input_scripts().len() {
            d.verify_input_items(input, all, t)?;
        }
        let items = d.all_input_items();
        d.consume_inputs_internal(true, Some(all), true, true, t)?;
        Ok(items)
    })?;
    let items = items_for_keys(o, &input_items);
    populate_inputs_clearing(thread, o, &items)?;
    // The manual input types are saved, cleared and restored around
    // `autoPopulateInputs`.
    let types = std::mem::take(&mut state(o).borrow_mut().manual_input_allowed_item_types);
    auto_populate_inputs(thread, o)?;
    add_allowed_item_types(o, types);
    Ok(())
}

/// `canPerformCurrentRecipe()`.
fn can_perform_current_recipe(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<bool> {
    if selected(o).is_none() {
        return Err(unsupported("HandcraftLogic.canPerformCurrentRecipe without a recipe"));
    }
    let manual = state(o).borrow().manual_select_inputs;
    state(o).borrow_mut().cached_can_perform_dirty = false;
    let mut can = with_data(thread, o, &[], |d, all, t| {
        let items = if manual { None } else { Some(all) };
        d.can_perform(items, true, true, t)
    })?;
    state(o).borrow_mut().cached_can_perform = can;
    let (surface, workstation, iso_object) = {
        let st = state(o).borrow();
        let script = st.recipe.clone().ok_or_else(npe)?;
        let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        let r = r.borrow();
        (r.is_any_surface_craft(), r.requires_specific_workstation(), st.iso_object.clone())
    };
    if surface {
        // `isCharacterInRangeOfWorkbench()`.
        if can && !iso_object.is_nil() {
            return Err(unsupported("HandcraftLogic.isCharacterInRangeOfWorkbench (IsoGridSquare.DistToProper)"));
        }
        can = false;
        state(o).borrow_mut().cached_can_perform = can;
    }
    if workstation {
        return Err(unsupported("BaseCraftingLogic.hasRequiredWorkstation"));
    }
    Ok(state(o).borrow().cached_can_perform)
}

/// `BaseCraftingLogic.isContainersAccessible(containers)`.
fn is_containers_accessible(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<bool> {
    let (containers, player) = {
        let st = state(o).borrow();
        (st.containers.clone(), st.player.clone())
    };
    for c in containers {
        let outer = call(thread, &c, "getOutermostContainer", &[])?;
        if !call(thread, &outer, "getVehiclePart", &[])?.is_nil() {
            return Err(unsupported("BaseCraftingLogic.isContainersAccessible (VehiclePart)"));
        }
        let square = call(thread, &outer, "getSquare", &[])?;
        if !square.is_nil() {
            // `IsoGridSquare.DistToProper(player)`: from the square's
            // centre.
            let sx = number(&call(thread, &square, "getX", &[])?)? as f32 + 0.5;
            let sy = number(&call(thread, &square, "getY", &[])?)? as f32 + 0.5;
            let px = number(&call(thread, &player, "getX", &[])?)? as f32;
            let py = number(&call(thread, &player, "getY", &[])?)? as f32;
            let dist = ((sx - px) * (sx - px) + (sy - py) * (sy - py)).sqrt();
            if dist > 2.5 {
                return Ok(false);
            }
        }
        let parent = call(thread, &outer, "getParent", &[])?;
        if instance_of(&parent, ISO_THUMPABLE) && call(thread, &parent, "isLockedToCharacter", &[player.clone()])?.truthy()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `BaseCraftingLogic.updateFloorContainer(containers)`: the `floor`
/// container is refilled from the item objects of the 3x3 squares around
/// the player, then `allItems` is read again. Only the case where that
/// leaves the floor container as it was is ported.
fn update_floor_container(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    let (containers, character) = {
        let st = state(o).borrow();
        (st.containers.clone(), st.character.clone())
    };
    let square = character.borrow().current_square_xyz();
    if let Some((x, y, z)) = square {
        let mut floor = None;
        for c in &containers {
            if matches!(call(thread, c, "getType", &[])?, Value::Str(t) if &*t == "floor") {
                floor = Some(c.clone());
                break;
            }
        }
        let mut world_items: Vec<ItemRef> = Vec::new();
        for sx in x - 1..=x + 1 {
            for sy in y - 1..=y + 1 {
                let sq = crate::bind_world::square_value(sx, sy, z);
                if sq.is_nil() {
                    continue;
                }
                let objects = call(thread, &sq, "getWorldObjects", &[])?;
                let list = match &objects {
                    Value::Object(l) => l.as_any().downcast_ref::<JavaList>().map(JavaList::snapshot),
                    _ => None,
                };
                for obj in list.ok_or_else(class_cast)? {
                    let item = call(thread, &obj, "getItem", &[])?;
                    if !item.is_nil() {
                        world_items.push(crate::bind_world_object::item_argument(&item)?);
                    }
                }
            }
        }
        let same = match &floor {
            Some(fl) => {
                let list = Value::Object(Rc::new(JavaList::new(vec![fl.clone()])));
                let items = container_items(&list)?;
                items.len() == world_items.len() && items.iter().zip(&world_items).all(|(a, b)| Rc::ptr_eq(a, b))
            }
            None => world_items.is_empty(),
        };
        if !same {
            return Err(unsupported("BaseCraftingLogic.updateFloorContainer (the floor items changed)"));
        }
    }
    let list = Value::Object(Rc::new(JavaList::new(containers)));
    let items = container_items(&list)?;
    state(o).borrow_mut().all_items = items;
    Ok(())
}

/// `CraftRecipeData.luaCallOnStart(chr)` / `luaCallOnCreate(chr)`: the
/// recipe's Lua function, if it has one, called with the data and the
/// character (`protectedCallVoid`).
fn lua_call(thread: &mut Thread, o: &Rc<dyn LuaObject>, which: LuaCall, chr: Value) -> LuaResult<()> {
    let Some(script) = selected(o) else {
        return Ok(());
    };
    let name = {
        let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        let r = r.borrow();
        if !r.has_lua_call(which) {
            return Ok(());
        }
        r.lua_call_string(which).unwrap_or_default().to_string()
    };
    let f = crate::bind_item_container::function_object(thread, &name);
    if f.is_nil() {
        return Ok(());
    }
    let data = Value::Object(Rc::new(RecipeDataObject(o.clone())));
    crate::global_objects::pcall_logged(thread, &f, &[data, chr]);
    Ok(())
}

/// The `toOutputItems` entries to create: `InventoryItemFactory.CreateItem`
/// for each (`createOutputItemInternal`).
fn create_output_items(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    let entries = state(o).borrow().data.to_output.clone();
    let mut out = Vec::with_capacity(entries.len());
    let mut created = Vec::new();
    for entry in entries {
        let OutputItem::Create(script) = entry else {
            out.push(entry);
            continue;
        };
        let full_name = script.borrow().full_name();
        let Some(item) = crate::bind_items::try_create_item(&full_name, 1.0)? else {
            // `createOutputItem` warns and the output counts as made.
            return Err(unsupported(format!("CraftRecipeManager.createOutputItem ({full_name} not created)")));
        };
        crate::bind_items::initialise_item(thread, &item);
        out.push(OutputItem::Created(key(&item)));
        created.push(item);
    }
    let mut st = state(o).borrow_mut();
    st.data.to_output = out;
    st.held.extend(created);
    Ok(())
}

/// `PlayerCraftHistory.addCraftHistoryCraftedEvent(name)`.
fn add_craft_history_crafted_event(o: &Rc<dyn LuaObject>, name: &str) -> LuaResult<()> {
    let character = state(o).borrow().character.clone();
    let mut chr = character.borrow_mut();
    if !chr.is_player() {
        // `(IsoPlayer) chr`.
        return Err(class_cast());
    }
    let ctx = crate::bind_character_sim::current_sim_context();
    let hours = chr.hours_survived(&*ctx);
    let history = &mut chr.player_fields.craft_history;
    if history.get(name).is_none() {
        history.insert(name.to_string(), CraftHistoryEntry { craft_count: 0, last_craft_time: 0.0 });
    }
    let entry = history.get_mut(name).ok_or_else(npe)?;
    entry.craft_count += 1;
    entry.last_craft_time = hours;
    if history.len() > 20 {
        return Err(unsupported("PlayerCraftHistory.cleanupHistory"));
    }
    Ok(())
}

/// `HandcraftLogic.performCurrentRecipe()`.
fn perform_current_recipe(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<bool> {
    let Some(script) = selected(o) else {
        return Err(unsupported("HandcraftLogic.performCurrentRecipe without a recipe"));
    };
    if !is_containers_accessible(thread, o)? {
        return Ok(false);
    }
    update_floor_container(thread, o)?;
    // `recipeData.perform(player, sourceResources, manual ? null :
    // allItems, containers)`.
    let (manual, player) = {
        let st = state(o).borrow();
        (st.manual_select_inputs, st.player.clone())
    };
    let consumed = with_data(thread, o, &[], |d, all, t| {
        if !d.is_valid(true)? {
            return Ok(false);
        }
        let items = if manual { None } else { Some(all) };
        d.consume_inputs_internal(false, items, false, false, t)
    })?;
    if !consumed {
        return Ok(false);
    }
    lua_call(thread, o, LuaCall::OnStart, player)?;
    if !with_data(thread, o, &[], |d, _, _| d.create_outputs_internal(false))? {
        return Ok(false);
    }
    create_output_items(thread, o)?;
    let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
    let name = {
        let r = r.borrow();
        if r.xp_award.is_some() {
            return Err(unsupported("CraftRecipeData.addXP"));
        }
        r.name().unwrap_or_default().to_string()
    };
    add_craft_history_crafted_event(o, &name)?;
    Ok(true)
}

/// `CraftRecipeData.processDestroyAndUsedItems(chr)`: each consumed item
/// loses the uses its inputs still need (`ItemUser.UseItem`). The original
/// walks a `HashSet`; this walks the items in the order they were consumed.
fn process_destroy_and_used_items(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<()> {
    let script = selected(o).ok_or_else(npe)?;
    let (data, sm) = (state(o).borrow().data.clone(), object(o).sm.clone());
    let recipe = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
    let recipe = recipe.borrow();
    let inputs: Vec<&InputScript> = recipe.inputs().collect();
    let mut amounts: Vec<f32> = inputs
        .iter()
        .map(|i| if i.is_variable_amount() { i.int_amount() as f32 * data.calculated_ratio } else { i.int_amount() as f32 })
        .collect();
    let items = items_for_keys(o, &data.consumed_used_items);
    for item in items {
        let k = key(&item);
        let script_item = pz_characters::character::items::script_item(&sm, &item.borrow());
        for (i, input) in inputs.iter().enumerate() {
            if !data.inputs[i].cache.applied.contains(&k) {
                continue;
            }
            // (`Unseal` with a fluid container does nothing.)
            if !input.is_item_count()
                && (input.consume_from_item_script().is_some()
                    || crate::bind_item_container::fluid_container(&item.borrow()).is_some())
            {
                continue;
            }
            if input.is_keep() {
                // `processKeepInputItem`: the keep flags (`MayDegrade`,
                // sharpness, ...).
                return Err(unsupported("CraftRecipeData.processKeepInputItem"));
            }
            if amounts[i] <= EPSILON {
                continue;
            }
            let full_name = script_item.as_ref().ok_or_else(npe)?.borrow().full_name();
            let ceil = (amounts[i] as f64).ceil() as f32;
            let mut destroy = input.is_destroy();
            let replaces = item.borrow().get_replace_on_use().is_some()
                || script_item.as_ref().is_some_and(|s| s.borrow().replace_on_deplete.is_some());
            if input.has_flag(InputFlag::DontReplace) && replaces {
                destroy = true;
            }
            let scale = input.relative_scale(&full_name);
            if input.is_item_count() {
                let uses = item.borrow().get_current_uses();
                crate::bind_item_container::item_user_use_item(thread, &item, uses, input.is_keep(), destroy)?;
                amounts[i] -= 1.0 / scale;
            } else {
                let n = ((ceil * scale) as i32).min(ceil as i32);
                let used = crate::bind_item_container::item_user_use_item(thread, &item, n, input.is_keep(), destroy)?;
                amounts[i] -= used as f32 / scale;
            }
        }
    }
    Ok(())
}

/// `CraftRecipeData.getModel(handOne)`: the static model of the most
/// recent item of the first `Prop1`/`Prop2` input that has one, else the
/// timed action's prop.
fn model(thread: &mut Thread, o: &Rc<dyn LuaObject>, hand_one: bool) -> LuaResult<Value> {
    let Some(script) = selected(o) else {
        return Ok(Value::Nil);
    };
    let (recent, prop) = {
        let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        let r = r.borrow();
        let data = &state(o).borrow().data;
        let recent = r.inputs().enumerate().find_map(|(i, input)| {
            let prop = if hand_one { input.is_prop1() } else { input.is_prop2() };
            if input.resource_type() != ResourceType::Item || !prop {
                return None;
            }
            data.inputs[i].cache.applied.last().copied()
        });
        let prop = r.timed_action_script().and_then(|s| {
            let t = s.downcast::<pz_scripts::objects::definitions::TimedActionScript>()?;
            let t = t.borrow();
            let p = if hand_one { t.prop1() } else { t.prop2() };
            Some(p.map(str::to_string))
        });
        (recent, prop)
    };
    if let Some(k) = recent {
        let item = items_for_keys(o, &[k]).pop().ok_or_else(npe)?;
        let v = crate::bind_items::try_item_value(&item)?;
        return call(thread, &v, "getStaticModel", &[]);
    }
    Ok(match prop {
        Some(Some(p)) => Value::str(&p),
        _ => Value::Nil,
    })
}

/// `isUsingRecipeAtHandBenefit()`: false for a non-player, in the dark, or
/// when `CraftRecipe.canBenefitFromRecipeAtHand` is false for a recipe that
/// need not be learned or that the character does not know.
fn is_using_recipe_at_hand_benefit(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<bool> {
    let script = selected(o).ok_or_else(npe)?;
    with_character(thread, o, |chr| {
        use super::super::RecipeCharacter;
        if !chr.is_player() || chr.too_dark_to_read()? {
            return Ok(false);
        }
        let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        let r = r.borrow();
        if !r.need_to_be_learn() || !is_recipe_actually_known(&r, chr) {
            return Ok(false);
        }
        Err(unsupported("CraftRecipe.canBenefitFromRecipeAtHand"))
    })
}

fn input_data(o: &Rc<dyn LuaObject>, input: &Value) -> LuaResult<Option<usize>> {
    let Some(script) = selected(o) else {
        return Ok(None);
    };
    let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
    let r = r.borrow();
    input_index(&r, input)
}

/// `getInputUses(input)`.
fn input_uses(o: &Rc<dyn LuaObject>, input: &Value) -> LuaResult<f32> {
    let Some(i) = input_data(o, input)? else {
        return Ok(0.0);
    };
    let script = selected(o).ok_or_else(npe)?;
    let (fluid, count) = {
        let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
        let r = r.borrow();
        let s = r.inputs().nth(i).ok_or_else(npe)?;
        (s.resource_type() == ResourceType::Fluid, s.is_item_count())
    };
    if fluid {
        return Err(unsupported("CraftRecipeData.InputScriptData.getInputItemFluidUses"));
    }
    let keys = state(o).borrow().data.inputs[i].items.clone();
    if count {
        return Ok(keys.len() as f32);
    }
    Ok(items_for_keys(o, &keys).iter().map(|i| i.borrow().get_current_uses()).sum::<i32>() as f32)
}

fn logic_arg(c: &NativeCall<'_>) -> LuaResult<Rc<dyn LuaObject>> {
    logic(c)
}

pub(super) fn register(b: &ClassBuilder) {
    b.method("getRecipeData", |c| {
        let o = logic_arg(c)?;
        Ok(c.push(Value::Object(Rc::new(RecipeDataObject(o)))))
    });
    b.method("autoPopulateInputs", |c| {
        let o = logic_arg(c)?;
        auto_populate_inputs(c.thread, &o)?;
        Ok(0)
    });
    b.method("getPossibleCraftCount", |c| {
        let o = logic_arg(c)?;
        let v = if c.arg(1).truthy() {
            recalculate_possible_craft_count(c.thread, &o)?
        } else {
            state(&o).borrow().cached_possible_craft_count
        };
        Ok(c.push(v))
    });
    b.method("refresh", |c| {
        let o = logic_arg(c)?;
        refresh(c.thread, &o)?;
        Ok(0)
    });
    b.method("canPerformCurrentRecipe", |c| {
        let o = logic_arg(c)?;
        let v = can_perform_current_recipe(c.thread, &o)?;
        Ok(c.push(v))
    });
    b.method("cachedCanPerformCurrentRecipe", |c| {
        let o = logic_arg(c)?;
        let dirty = state(&o).borrow().cached_can_perform_dirty;
        let v = if dirty { can_perform_current_recipe(c.thread, &o)? } else { state(&o).borrow().cached_can_perform };
        Ok(c.push(v))
    });
    b.method("performCurrentRecipe", |c| {
        let o = logic_arg(c)?;
        let v = perform_current_recipe(c.thread, &o)?;
        Ok(c.push(v))
    });
    // `getCreatedOutputItems(list)`: the created items not yet processed.
    b.method("getCreatedOutputItems", |c| {
        let o = logic_arg(c)?;
        let entries = state(&o).borrow().data.to_output.clone();
        let mut keys = Vec::new();
        for e in &entries {
            match e {
                OutputItem::Created(k) => keys.push(*k),
                OutputItem::Kept(_) => {}
                OutputItem::Script(_) | OutputItem::Create(_) => {
                    return Err(unsupported("HandcraftLogic.getCreatedOutputItems (test outputs)"))
                }
            }
        }
        let list = match c.arg(1) {
            Value::Object(l) => l,
            Value::Nil => return Err(npe()),
            _ => return Err(class_cast()),
        };
        let list = list.as_any().downcast_ref::<JavaList>().ok_or_else(class_cast)?;
        for item in items_for_keys(&o, &keys) {
            list.push(crate::bind_items::try_item_value(&item)?)?;
        }
        Ok(0)
    });
    b.method("startCraftAction", |c| {
        let o = logic_arg(c)?;
        let Value::Table(table) = c.arg(1) else {
            return Err(class_cast());
        };
        {
            let mut st = state(&o).borrow_mut();
            st.craft_action_in_progress = true;
            st.craft_action_table = Value::Table(table.clone());
            st.craft_action_count += 1;
        }
        let same = match (selected(&o), recipe_arg(&table.rawget(&Value::str("craftRecipe")))) {
            (None, Ok(None)) => true,
            (Some(a), Ok(Some(b))) => a.ptr_eq(&b),
            _ => false,
        };
        if same {
            if let Value::Object(l) = table.rawget(&Value::str("items")) {
                let list = l.as_any().downcast_ref::<JavaList>().ok_or_else(class_cast)?;
                let items = list
                    .snapshot()
                    .iter()
                    .map(crate::bind_world_object::item_argument)
                    .collect::<LuaResult<Vec<_>>>()?;
                if selected(&o).is_none() {
                    return Err(unsupported("HandcraftLogic.startCraftAction without a recipe"));
                }
                populate_inputs_clearing(c.thread, &o, &items)?;
            }
        }
        trigger_event(c.thread, &o, "onStartCraft", &[Value::Table(table)]);
        Ok(0)
    });
    b.method("stopCraftAction", |c| {
        let o = logic_arg(c)?;
        let all = c.arg(1).truthy();
        {
            let mut st = state(&o).borrow_mut();
            if st.craft_action_count > 0 {
                st.craft_action_count -= 1;
            }
            if all {
                st.craft_action_count = 0;
            }
            if st.craft_action_count == 0 {
                st.craft_action_in_progress = false;
                st.craft_action_table = Value::Nil;
            }
        }
        trigger_event(c.thread, &o, "onStopCraft", &[]);
        Ok(0)
    });
    b.method("isCraftActionInProgress", |c| {
        let v = state(&logic_arg(c)?).borrow().craft_action_in_progress;
        Ok(c.push(v))
    });
    b.method("getCraftActionTable", |c| {
        let v = state(&logic_arg(c)?).borrow().craft_action_table.clone();
        Ok(c.push(v))
    });
    b.method("getVariableInputRatio", |c| {
        let r = state(&logic_arg(c)?).borrow().data.calculated_ratio;
        Ok(c.push(Value::Num(if r == f32::MAX { 1.0 } else { r as f64 })))
    });
    b.method("setTargetVariableInputRatio", |c| {
        let o = logic_arg(c)?;
        let v = number(&c.arg(1))? as f32;
        let mut st = state(&o).borrow_mut();
        st.data.target_ratio = v;
        st.cached_can_perform_dirty = true;
        Ok(0)
    });
    b.method("isInputSatisfied", |c| {
        let o = logic_arg(c)?;
        let input = c.arg(1);
        let Some(script) = selected(&o) else {
            return Ok(c.push(false));
        };
        let (contains, index, item) = {
            let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
            let r = r.borrow();
            let index = input_index(&r, &input)?;
            let item = index.and_then(|i| r.inputs().nth(i)).is_some_and(|i| i.resource_type() == ResourceType::Item);
            (contains_io(&r, &input)?, index, item)
        };
        if !contains {
            return Ok(c.push(false));
        }
        let i = index.ok_or_else(npe)?;
        let v = if item && state(&o).borrow().manual_select_inputs {
            with_data(c.thread, &o, &[], |d, _, t| d.is_satisfied(i, t))?
        } else {
            state(&o).borrow().data.inputs[i].cached_can_consume
        };
        Ok(c.push(v))
    });
    // `areAllInputItemsSatisfied()`: the recipe data's, which has no inputs
    // without a recipe.
    b.method("areAllInputItemsSatisfied", |c| {
        let o = logic_arg(c)?;
        let v = all_input_items_satisfied(c.thread, &o)?;
        Ok(c.push(v))
    });
    b.method("getInputUses", |c| {
        let o = logic_arg(c)?;
        let v = input_uses(&o, &c.arg(1))?;
        Ok(c.push(Value::Num(v as f64)))
    });
    b.method("getInputCount", |c| {
        let o = logic_arg(c)?;
        let v = input_uses(&o, &c.arg(1))? as i32;
        Ok(c.push(v))
    });
    // `getSatisfiedInputItems(input)`: the script items of the input's
    // applied items.
    b.method("getSatisfiedInputItems", |c| {
        let o = logic_arg(c)?;
        let i = input_data(&o, &c.arg(1))?.ok_or_else(npe)?;
        let keys = state(&o).borrow().data.inputs[i].cache.applied.clone();
        let sm = object(&o).sm.clone();
        let mut scripts = Vec::new();
        for item in items_for_keys(&o, &keys) {
            let full_type = item.borrow().get_full_type();
            scripts.push(sm.get_script(ScriptType::Item, &full_type).ok_or_else(npe)?);
        }
        Ok(c.push(script_list(&scripts)))
    });
    b.method("clearManualInputs", |c| {
        let o = logic_arg(c)?;
        with_data(c.thread, &o, &[], |d, _, _| {
            d.clear_manual_inputs();
            Ok(())
        })?;
        Ok(0)
    });
    // `setManualInputsFor(input, items)`: false without manual input
    // selection, or for an input of another recipe.
    b.method("setManualInputsFor", |c| {
        let o = logic_arg(c)?;
        if !state(&o).borrow().manual_select_inputs {
            return Ok(c.push(false));
        }
        let input = c.arg(1);
        let index = match selected(&o) {
            Some(script) => {
                let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
                let r = r.borrow();
                if !contains_io(&r, &input)? {
                    return Ok(c.push(false));
                }
                input_index(&r, &input)?
            }
            None => return Ok(c.push(false)),
        };
        let i = index.ok_or_else(npe)?;
        let list = match c.arg(2) {
            Value::Object(l) => l,
            Value::Nil => return Err(npe()),
            _ => return Err(class_cast()),
        };
        let list = list.as_any().downcast_ref::<JavaList>().ok_or_else(class_cast)?;
        // An item in no container counts only on a square.
        let mut items = Vec::new();
        for v in list.snapshot() {
            let item = crate::bind_world_object::item_argument(&v)?;
            if call(c.thread, &v, "getContainer", &[])?.is_nil() {
                let world = call(c.thread, &v, "getWorldItem", &[])?;
                if world.is_nil() || number(&call(c.thread, &world, "getWorldObjectIndex", &[])?)? == -1.0 {
                    continue;
                }
            }
            items.push(item);
        }
        let keys: Vec<usize> = items.iter().map(key).collect();
        let v = with_data(c.thread, &o, &items, |d, _, t| d.set_manual_inputs_for(i, &keys, t))?;
        Ok(c.push(v))
    });
    b.method("getMulticraftConsumedItemsFor", |c| {
        let o = logic_arg(c)?;
        let i = input_data(&o, &c.arg(1))?.ok_or_else(npe)?;
        let list = match c.arg(2) {
            Value::Object(l) => l,
            Value::Nil => return Err(npe()),
            _ => return Err(class_cast()),
        };
        let java_list = list.as_any().downcast_ref::<JavaList>().ok_or_else(class_cast)?;
        let mut keys = Vec::new();
        for v in java_list.snapshot() {
            keys.push(key(&crate::bind_world_object::item_argument(&v)?));
        }
        let before = keys.len();
        let consumed = state(&o).borrow().multicraft_consumed_items.clone();
        with_data(c.thread, &o, &[], |d, _, t| d.multicraft_consumed_items_for(i, &consumed, &mut keys, t))?;
        for item in items_for_keys(&o, &keys[before..]) {
            java_list.push(crate::bind_items::try_item_value(&item)?)?;
        }
        Ok(c.push(Value::Object(list)))
    });
    b.method("updateManualInputAllowedItemTypes", |c| {
        let o = logic_arg(c)?;
        state(&o).borrow_mut().manual_input_allowed_item_types.clear();
        if state(&o).borrow().manual_select_inputs {
            let types = with_data(c.thread, &o, &[], |d, _, _| Ok(d.applied_input_item_types()))?;
            add_allowed_item_types(&o, types);
        }
        Ok(0)
    });
    b.method("isUsingRecipeAtHandBenefit", |c| {
        let o = logic_arg(c)?;
        let v = is_using_recipe_at_hand_benefit(c.thread, &o)?;
        Ok(c.push(v))
    });
    // `getUsingRecipeAtHandItem()`: null where `isUsingRecipeAtHandBenefit`
    // is false.
    b.method("getUsingRecipeAtHandItem", |c| {
        let o = logic_arg(c)?;
        is_using_recipe_at_hand_benefit(c.thread, &o)?;
        Ok(c.push(Value::Nil))
    });
    b.method("getModelHandOne", |c| {
        let o = logic_arg(c)?;
        let v = model(c.thread, &o, true)?;
        Ok(c.push(v))
    });
    b.method("getModelHandTwo", |c| {
        let o = logic_arg(c)?;
        let v = model(c.thread, &o, false)?;
        Ok(c.push(v))
    });
}

/// `CraftRecipeData.areAllInputItemsSatisfied()`.
fn all_input_items_satisfied(thread: &mut Thread, o: &Rc<dyn LuaObject>) -> LuaResult<bool> {
    if selected(o).is_none() {
        return Ok(true);
    }
    with_data(thread, o, &[], |d, _, t| d.are_all_input_items_satisfied(t))
}

fn data_logic(c: &NativeCall<'_>) -> LuaResult<Rc<dyn LuaObject>> {
    let o = this::<RecipeDataObject>(c)?;
    Ok(o.as_any().downcast_ref::<RecipeDataObject>().unwrap().0.clone())
}

/// The applied items `f` picks from the data, as an `ArrayList`; empty
/// without a recipe.
fn data_items(
    c: &mut NativeCall<'_>,
    f: impl FnOnce(&RecipeData<'_>) -> Vec<usize>,
) -> LuaResult<usize> {
    let o = data_logic(c)?;
    let keys = if selected(&o).is_some() { with_data(c.thread, &o, &[], |d, _, _| Ok(f(d)))? } else { Vec::new() };
    let v = item_list(&items_for_keys(&o, &keys))?;
    Ok(c.push(v))
}

pub(super) fn register_recipe_data(thread: &mut Thread) {
    let b = expose_class(thread, CRAFT_RECIPE_DATA_CLASS, None);
    b.method("getRecipe", |c| {
        let o = data_logic(c)?;
        Ok(c.push(script_value(selected(&o).as_ref())))
    });
    b.method("getDataForInputScript", |c| {
        let o = data_logic(c)?;
        let v = match input_data(&o, &c.arg(1))? {
            Some(input) => Value::Object(Rc::new(InputDataObject { logic: o, input })),
            None => Value::Nil,
        };
        Ok(c.push(v))
    });
    // `containsInputItem(item)`, or `(InputScriptData, item)`.
    b.method("containsInputItem", |c| {
        let o = data_logic(c)?;
        if c.nargs() > 2 {
            let data = match c.arg(1) {
                Value::Object(d) => d,
                _ => return Err(npe()),
            };
            let d = data.as_any().downcast_ref::<InputDataObject>().ok_or_else(class_cast)?;
            let item = crate::bind_world_object::item_argument(&c.arg(2))?;
            let v = state(&o).borrow().data.inputs[d.input].items.contains(&key(&item));
            return Ok(c.push(v));
        }
        if selected(&o).is_none() {
            return Ok(c.push(false));
        }
        let item = crate::bind_world_object::item_argument(&c.arg(1))?;
        let k = key(&item);
        let v = with_data(c.thread, &o, &[item], |d, _, _| Ok(d.contains_input_item(k)))?;
        Ok(c.push(v))
    });
    b.method("getAllInputItems", |c| data_items(c, |d| d.all_input_items()));
    b.method("getAllPutBackInputItems", |c| data_items(c, |d| d.put_back_input_items()));
    b.method("getAllConsumedItems", |c| {
        if c.nargs() > 1 {
            return Err(unsupported("CraftRecipeData.getAllConsumedItems (ArrayList)"));
        }
        data_items(c, |d| d.consumed_items())
    });
    b.method("areAllInputItemsSatisfied", |c| {
        let o = data_logic(c)?;
        let v = all_input_items_satisfied(c.thread, &o)?;
        Ok(c.push(v))
    });
    // `getFirstManualInputFor(input)`: the first of the input's
    // `inputItems`; null for an input of another recipe.
    b.method("getFirstManualInputFor", |c| {
        let o = data_logic(c)?;
        let input = c.arg(1);
        let Some(script) = selected(&o) else {
            return Ok(c.push(Value::Nil));
        };
        let (contains, index) = {
            let r = script.downcast::<CraftRecipe>().ok_or_else(class_cast)?;
            let r = r.borrow();
            (contains_io(&r, &input)?, input_index(&r, &input)?)
        };
        if !contains {
            return Ok(c.push(Value::Nil));
        }
        let i = index.ok_or_else(npe)?;
        let first = state(&o).borrow().data.inputs[i].items.first().copied();
        let item = first.and_then(|k| items_for_keys(&o, &[k]).into_iter().next());
        let v = match item {
            Some(item) => crate::bind_items::try_item_value(&item)?,
            None => Value::Nil,
        };
        Ok(c.push(v))
    });
    b.method("getVariableInputRatio", |c| {
        let r = state(&data_logic(c)?).borrow().data.calculated_ratio;
        Ok(c.push(Value::Num(if r == f32::MAX { 1.0 } else { r as f64 })))
    });
    b.method("luaCallOnCreate", |c| {
        let o = data_logic(c)?;
        let chr = c.arg(1);
        lua_call(c.thread, &o, LuaCall::OnCreate, chr)?;
        Ok(0)
    });
    b.method("processDestroyAndUsedItems", |c| {
        let o = data_logic(c)?;
        process_destroy_and_used_items(c.thread, &o)?;
        Ok(0)
    });

    let d = expose_class(thread, INPUT_DATA_CLASS, None);
    d.method("isInputItemsSatisfied", |c| {
        let o = this::<InputDataObject>(c)?;
        let d = o.as_any().downcast_ref::<InputDataObject>().unwrap();
        let input = d.input;
        let logic = d.logic.clone();
        let v = with_data(c.thread, &logic, &[], |data, _, t| data.is_satisfied(input, t))?;
        Ok(c.push(v))
    });
}
