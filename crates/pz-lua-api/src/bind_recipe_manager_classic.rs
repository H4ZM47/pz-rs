//! Classic `zombie.inventory.RecipeManager` static methods (Build 42.22):
//! `IsRecipeValid`, `PerformMakeItem`, `getNumberOfTimesRecipeCanBeDone`,
//! `getAvailableItemsNeeded`, `getAvailableItemsAll`, `getSourceItemsNeeded`,
//! `getSourceItemsAll`, `getUniqueRecipeItems`, `getKnownRecipesNumber`,
//! `getAllEvolvedRecipes`, `getDismantleRecipeFor`, `GetMovableRecipeTool`,
//! `HasAllRequiredItems`, `isAllItemsUsableRotten`, `hasHeat`,
//! `validateHasHeat`, `validateRecipeContainsSourceItem`,
//! `printDebugRecipeValid`, and the deprecated Build 42 stubs
//! (`IsItemDestroyed`, `UseAmount`, `DoesWipeUseDelta`, `DoesUseItemUp`).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use pz_characters::Characters;
use pz_inventory::shared::{self, ContainerRef, ItemRef};
use pz_inventory::{InventoryItem, ItemClass, ItemKind};
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_scripts::objects::recipes::{LuaCall, Recipe, Source};
use pz_scripts::{ScriptManager, ScriptRef, ScriptType};

use crate::bind_character_sim::{current_sim_context, CharacterObject, LuaGameCharacterRef};
use crate::bind_item_container::{
    add_item, container_of_value, function_object, remove as remove_from_container,
};
use crate::bind_items::{create_item, initialise_item, item_arg_value, item_value};
use crate::bind_java_util::{list_value, JavaList};
use crate::bind_scripts::{npe, script_list, script_value, ScriptHandle};
use crate::expose::{bool_arg, int_arg, str_arg, ClassBuilder};

fn recipe_of_value(v: &Value) -> LuaResult<Option<Rc<RefCell<Recipe>>>> {
    match v {
        Value::Nil => Ok(None),
        Value::Object(o) => {
            if let Some(h) = o.as_any().downcast_ref::<ScriptHandle>() {
                if let Some(r) = h.root.downcast::<Recipe>() {
                    return Ok(Some(r));
                }
            }
            Err(LuaError::runtime("expected zombie.scripting.objects.Recipe"))
        }
        _ => Err(LuaError::runtime("expected zombie.scripting.objects.Recipe")),
    }
}

fn character_of_value(v: &Value) -> LuaResult<Option<LuaGameCharacterRef>> {
    match v {
        Value::Nil => Ok(None),
        Value::Object(o) => {
            if let Some(ch) = (&**o as &dyn Any).downcast_ref::<CharacterObject>() {
                return Ok(Some(ch.0.clone()));
            }
            Err(LuaError::runtime("expected IsoGameCharacter"))
        }
        _ => Err(LuaError::runtime("expected IsoGameCharacter")),
    }
}

fn cooking_level(chars: &RefCell<Characters>, chr: &Option<LuaGameCharacterRef>) -> i32 {
    let Some(chr) = chr else {
        return 0;
    };
    let perk = chars.borrow().perks.constant("Cooking");
    perk.map_or(0, |p| chr.borrow().perk_level(Some(&p)))
}

fn see_not_learnt_recipe(thread: &mut Thread) -> bool {
    let _ = crate::bind_sandbox::instance(thread);
    crate::bind_sandbox::current_bool("SeeNotLearntRecipe").unwrap_or(false)
}

fn is_recipe_known(
    thread: &mut Thread,
    chr: &LuaGameCharacterRef,
    recipe: &Recipe,
) -> bool {
    let ctx = current_sim_context();
    if chr.borrow().cheats.is_set(&*ctx, "KNOW_ALL_RECIPES") {
        return true;
    }
    if see_not_learnt_recipe(thread) {
        return true;
    }
    let Some(orig) = recipe.originalname() else {
        return false;
    };
    chr.borrow().known_recipes.iter().any(|k| k == orig)
}

fn has_required_skills(
    chars: &RefCell<Characters>,
    chr: &LuaGameCharacterRef,
    recipe: &Recipe,
) -> bool {
    let Some(skills) = recipe.skill_required.as_ref() else {
        return true;
    };
    let chars_b = chars.borrow();
    let chr_b = chr.borrow();
    for req in skills {
        let Some(perk) = chars_b.perks.constant(req.perk()) else {
            return false;
        };
        if chr_b.perk_level(Some(&perk)) < req.level() {
            return false;
        }
    }
    true
}

fn resolve_containers(
    chr: &Option<LuaGameCharacterRef>,
    containers_val: &Value,
) -> Vec<ContainerRef> {
    let mut out = Vec::new();
    if let Value::Object(o) = containers_val {
        if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
            for elem in list.snapshot() {
                if let Some(c) = container_of_value(&elem) {
                    out.push(c);
                }
            }
        }
    }
    if let Some(chr) = chr {
        let inv = chr.borrow().items.inventory.clone();
        if !out.iter().any(|c| Rc::ptr_eq(c, &inv)) {
            out.push(inv);
        }
    }
    out
}

fn collect_ignore_ids(ignore_val: &Value) -> HashSet<i32> {
    let mut ids = HashSet::new();
    if let Value::Object(o) = ignore_val {
        if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
            for elem in list.snapshot() {
                if let Some(item) = item_arg_value(&elem) {
                    ids.insert(item.borrow().id);
                }
            }
        }
    }
    ids
}

fn item_matches_pattern(
    thread: &mut Thread,
    item: &InventoryItem,
    pattern: &str,
) -> LuaResult<bool> {
    if pattern.is_empty() {
        return Ok(false);
    }
    if pattern == "Water" {
        return Ok(crate::bind_item_container::is_water_source(item));
    }
    if let Some(inner) = pattern.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let func = function_object(thread, inner);
        if !func.is_nil() {
            let list = Rc::new(JavaList::new(Vec::new()));
            let list_val = Value::Object(list.clone());
            let _ = thread.call(&func, &[list_val])?;
            let full_type = item.get_full_type();
            let short_type = item.get_type();
            for entry in list.snapshot() {
                if let Some(s) = entry.as_str() {
                    if s == full_type || s == short_type {
                        return Ok(true);
                    }
                } else if let Some(other) = item_arg_value(&entry) {
                    let ob = other.borrow();
                    if ob.id == item.id || ob.get_full_type() == full_type {
                        return Ok(true);
                    }
                }
            }
        }
        return Ok(false);
    }
    if let Some(tag) = pattern
        .strip_prefix("Tag:")
        .or_else(|| pattern.strip_prefix("tag:"))
    {
        return Ok(item.has_tag(tag));
    }
    let full = item.get_full_type();
    if full == pattern || item.get_type() == pattern {
        return Ok(true);
    }
    if !pattern.contains('.') {
        if let Some((_, short)) = full.split_once('.') {
            if short == pattern {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn item_matches_source(
    thread: &mut Thread,
    item: &InventoryItem,
    source: &Source,
) -> LuaResult<bool> {
    for pat in source.items() {
        if item_matches_pattern(thread, item, pat)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn item_passes_recipe_checks(
    thread: &mut Thread,
    recipe: &Recipe,
    item_ref: &ItemRef,
    cook_lvl: i32,
) -> LuaResult<bool> {
    {
        let item = item_ref.borrow();
        if !recipe.allow_destroyed_item && item.is_broken() {
            return Ok(false);
        }
        if !recipe.allow_frozen_item && matches!(&item.kind, ItemKind::Food(f) if f.frozen) {
            return Ok(false);
        }
        if item.is_rotten() && !recipe.allow_rotten_item && cook_lvl < 7 {
            return Ok(false);
        }
    }
    if let Some(test_fn) = recipe.lua_calls().get(&LuaCall::LuaTest) {
        let func = function_object(thread, test_fn);
        if !func.is_nil() {
            let res = thread.call(&func, &[item_value(item_ref)])?;
            if !res.first().is_some_and(Value::truthy) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Available units contributed by `item` for `source`.
fn item_available_amount(item: &InventoryItem, source: &Source) -> f32 {
    if source.destroy || source.keep || source.use_ <= 0.0 {
        return 1.0;
    }
    if item.template.class == ItemClass::Drainable {
        let delta = item.get_use_delta();
        if delta > 0.0 {
            return (item.get_current_uses_float() / delta).round().max(0.0);
        }
        return item.get_current_uses() as f32;
    }
    if item.template.class == ItemClass::Food {
        return (-item.get_hunger_change() * 100.0).max(0.0);
    }
    1.0
}

fn source_required_amount(source: &Source) -> f32 {
    if !source.destroy && !source.keep && source.use_ > 0.0 {
        source.use_
    } else {
        source.count.max(1.0)
    }
}

struct MatchedSource {
    items: Vec<ItemRef>,
    /// Amount consumed from each item (same length as `items`).
    consume_amounts: Vec<f32>,
    satisfied: bool,
    total_available: f32,
    required: f32,
    keep: bool,
}

struct MatchedRecipe {
    sources: Vec<MatchedSource>,
    valid: bool,
}

#[allow(clippy::too_many_arguments)]
fn match_recipe(
    thread: &mut Thread,
    chars: &RefCell<Characters>,
    recipe: &Recipe,
    chr: &Option<LuaGameCharacterRef>,
    containers: &[ContainerRef],
    selected_item: &Option<ItemRef>,
    ignore_ids: &HashSet<i32>,
    all_items: bool,
) -> LuaResult<MatchedRecipe> {
    let cook_lvl = cooking_level(chars, chr);

    // Honor `in_same_inventory` when `selected_item` lives inside one of the containers.
    let effective_containers: Vec<ContainerRef> = if recipe.in_same_inventory {
        if let Some(sel) = selected_item {
            if let Some(sel_c) = shared::container_of(sel) {
                if containers.iter().any(|c| Rc::ptr_eq(c, &sel_c)) {
                    vec![sel_c]
                } else {
                    containers.to_vec()
                }
            } else {
                containers.to_vec()
            }
        } else {
            containers.to_vec()
        }
    } else {
        containers.to_vec()
    };

    // Build candidate pool in container order, putting `selected_item` first if present.
    let mut pool: Vec<ItemRef> = Vec::new();
    let mut seen_pool: HashSet<i32> = HashSet::new();
    if let Some(sel) = selected_item {
        let id = sel.borrow().id;
        if !ignore_ids.contains(&id) {
            seen_pool.insert(id);
            pool.push(sel.clone());
        }
    }
    for c in &effective_containers {
        let items = c.borrow().items.clone();
        for item in items {
            let id = item.borrow().id;
            if !ignore_ids.contains(&id) && seen_pool.insert(id) {
                pool.push(item);
            }
        }
    }

    let mut reserved_non_keep: HashSet<i32> = HashSet::new();
    let mut matched_sources = Vec::with_capacity(recipe.source().len());
    let mut all_satisfied = true;
    let mut selected_used = selected_item.is_none();
    let sel_id = selected_item.as_ref().map(|s| s.borrow().id);

    for source in recipe.source() {
        if source.items().is_empty() {
            matched_sources.push(MatchedSource {
                items: Vec::new(),
                consume_amounts: Vec::new(),
                satisfied: true,
                total_available: 0.0,
                required: 0.0,
                keep: source.keep,
            });
            continue;
        }
        let required = source_required_amount(source);
        let mut remaining = required;
        let mut total_avail = 0.0f32;
        let mut picked: Vec<ItemRef> = Vec::new();
        let mut amounts: Vec<f32> = Vec::new();

        for cand in &pool {
            let id = cand.borrow().id;
            if !all_items && !source.keep && reserved_non_keep.contains(&id) {
                continue;
            }
            let matches = {
                let item_b = cand.borrow();
                item_matches_source(thread, &item_b, source)?
            };
            if !matches {
                continue;
            }
            if !item_passes_recipe_checks(thread, recipe, cand, cook_lvl)? {
                continue;
            }
            let avail = item_available_amount(&cand.borrow(), source);
            if avail <= 0.0 {
                continue;
            }
            total_avail += avail;
            if sel_id == Some(id) {
                selected_used = true;
            }
            if all_items {
                picked.push(cand.clone());
                amounts.push(avail);
            } else if remaining > 1e-4 {
                let take = if source.keep {
                    0.0
                } else {
                    avail.min(remaining)
                };
                picked.push(cand.clone());
                amounts.push(take);
                if !source.keep {
                    reserved_non_keep.insert(id);
                }
                remaining -= avail;
            }
        }
        let satisfied = if all_items {
            total_avail + 1e-4 >= required
        } else {
            remaining <= 1e-4
        };
        if !satisfied {
            all_satisfied = false;
        }
        matched_sources.push(MatchedSource {
            items: picked,
            consume_amounts: amounts,
            satisfied,
            total_available: total_avail,
            required,
            keep: source.keep,
        });
    }

    Ok(MatchedRecipe {
        sources: matched_sources,
        valid: all_satisfied && selected_used,
    })
}

fn validate_can_perform(
    thread: &mut Thread,
    recipe_val: &Value,
    recipe: &Recipe,
    chr_val: &Value,
    item_val: &Value,
) -> LuaResult<bool> {
    let Some(name) = recipe.lua_calls().get(&LuaCall::LuaCanPerform) else {
        return Ok(true);
    };
    let func = function_object(thread, name);
    if func.is_nil() {
        return Ok(true);
    }
    let res = thread.call(&func, &[recipe_val.clone(), chr_val.clone(), item_val.clone()])?;
    Ok(res.first().is_some_and(Value::truthy))
}

fn validate_has_heat(
    recipe: &Recipe,
    selected_item: &Option<ItemRef>,
    containers: &[ContainerRef],
) -> bool {
    let heat = recipe.heat();
    if heat == 0.0 {
        return true;
    }
    let check_item = |item: &InventoryItem| -> bool {
        let inv_heat = 1.0 - item.get_item_heat();
        if heat < 0.0 {
            inv_heat <= heat
        } else {
            inv_heat >= heat
        }
    };
    if let Some(sel) = selected_item {
        if check_item(&sel.borrow()) {
            return true;
        }
    }
    for c in containers {
        for item in &c.borrow().items {
            if check_item(&item.borrow()) {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn is_recipe_valid_inner(
    thread: &mut Thread,
    chars: &RefCell<Characters>,
    recipe_val: &Value,
    recipe: &Recipe,
    chr_val: &Value,
    chr: &Option<LuaGameCharacterRef>,
    item_val: &Value,
    selected_item: &Option<ItemRef>,
    containers_val: &Value,
) -> LuaResult<bool> {
    if recipe.result.is_none() {
        return Ok(false);
    }
    if let Some(ch) = chr {
        if !is_recipe_known(thread, ch, recipe) {
            return Ok(false);
        }
        if !has_required_skills(chars, ch, recipe) {
            return Ok(false);
        }
    }
    if let Some(sel) = selected_item {
        let sel_b = sel.borrow();
        let mut found = false;
        for src in recipe.source() {
            if item_matches_source(thread, &sel_b, src)? {
                found = true;
                break;
            }
        }
        if !found {
            return Ok(false);
        }
    }
    let containers = resolve_containers(chr, containers_val);
    let matched = match_recipe(
        thread,
        chars,
        recipe,
        chr,
        &containers,
        selected_item,
        &HashSet::new(),
        false,
    )?;
    if !matched.valid {
        return Ok(false);
    }
    if !validate_has_heat(recipe, selected_item, &containers) {
        return Ok(false);
    }
    validate_can_perform(thread, recipe_val, recipe, chr_val, item_val)
}

fn consume_matched_item(
    thread: &mut Thread,
    source: &Source,
    item_ref: &ItemRef,
    amount: f32,
) -> LuaResult<()> {
    if source.keep {
        return Ok(());
    }
    let container = shared::container_of(item_ref);
    if source.destroy || source.use_ <= 0.0 {
        if let Some(c) = container {
            remove_from_container(thread, &c, item_ref)?;
        }
        return Ok(());
    }
    let (depleted, replace_type): (bool, Option<String>) = {
        let mut item = item_ref.borrow_mut();
        if item.template.class == ItemClass::Drainable {
            let delta = item.get_use_delta();
            let cur = item.get_current_uses_float();
            let next = (cur - delta * amount).max(0.0);
            item.set_current_uses_float(next, None);
            let rep = item.template.replace_on_deplete.clone();
            (next <= 1e-4, rep)
        } else if item.template.class == ItemClass::Food {
            let cur = item.get_hunger_change();
            let next = (cur + amount / 100.0).min(0.0);
            item.set_hung_change(next);
            (next >= -1e-4, None)
        } else {
            (true, None)
        }
    };
    if depleted {
        if let Some(c) = container {
            remove_from_container(thread, &c, item_ref)?;
            if let Some(rep_ty) = replace_type.filter(|s| !s.is_empty()) {
                if let Some(rep) = create_item(&rep_ty, 1.0) {
                    initialise_item(thread, &rep);
                    let _ = add_item(thread, &c, Some(rep))?;
                }
            }
        }
    }
    Ok(())
}

/// Registers all classic `zombie.inventory.RecipeManager` static methods on `b`.
pub fn register_classic_statics(
    b: &ClassBuilder,
    sm: Rc<ScriptManager>,
    chars: Rc<RefCell<Characters>>,
) {
    // `getAllEvolvedRecipes()`
    {
        let sm = sm.clone();
        b.static_fn("getAllEvolvedRecipes", move |c| {
            let list: Vec<ScriptRef> = sm
                .collection(ScriptType::EvolvedRecipe)
                .get_all_scripts()
                .to_vec();
            Ok(c.push(script_list(&list)))
        });
    }

    // `getKnownRecipesNumber(chr)`
    {
        let sm = sm.clone();
        b.static_fn("getKnownRecipesNumber", move |c| {
            let Some(chr) = character_of_value(&c.arg(0))? else {
                return Ok(c.push(0i32));
            };
            let mut count = 0i32;
            for script in sm.collection(ScriptType::Recipe).get_all_scripts() {
                if let Some(r) = script.downcast::<Recipe>() {
                    if is_recipe_known(c.thread, &chr, &r.borrow()) {
                        count += 1;
                    }
                }
            }
            Ok(c.push(count))
        });
    }

    // `getUniqueRecipeItems(item, chr, containers)`
    {
        let sm = sm.clone();
        let chars = chars.clone();
        b.static_fn("getUniqueRecipeItems", move |c| {
            let item_val = c.arg(0);
            let chr_val = c.arg(1);
            let containers_val = c.arg(2);
            let selected = item_arg_value(&item_val);
            let chr = character_of_value(&chr_val)?;
            let mut out: Vec<ScriptRef> = Vec::new();
            for script in sm.collection(ScriptType::Recipe).get_all_scripts() {
                let Some(r) = script.downcast::<Recipe>() else {
                    continue;
                };
                let r_val = script_value(Some(&script));
                let valid = is_recipe_valid_inner(
                    c.thread,
                    &chars,
                    &r_val,
                    &r.borrow(),
                    &chr_val,
                    &chr,
                    &item_val,
                    &selected,
                    &containers_val,
                )?;
                if valid {
                    out.push(script.clone());
                }
            }
            Ok(c.push(script_list(&out)))
        });
    }

    // `IsRecipeValid(recipe, chr, item, containers)`
    {
        let chars = chars.clone();
        b.static_fn("IsRecipeValid", move |c| {
            let recipe_val = c.arg(0);
            let chr_val = c.arg(1);
            let item_val = c.arg(2);
            let containers_val = c.arg(3);
            let Some(recipe) = recipe_of_value(&recipe_val)? else {
                return Ok(c.push(false));
            };
            let chr = character_of_value(&chr_val)?;
            let selected = item_arg_value(&item_val);
            let valid = is_recipe_valid_inner(
                c.thread,
                &chars,
                &recipe_val,
                &recipe.borrow(),
                &chr_val,
                &chr,
                &item_val,
                &selected,
                &containers_val,
            )?;
            Ok(c.push(valid))
        });
    }

    // `printDebugRecipeValid(recipe, chr, item, containers)`
    b.static_fn("printDebugRecipeValid", |_c| Ok(0));

    // `validateRecipeContainsSourceItem(recipe, item)`
    b.static_fn("validateRecipeContainsSourceItem", |c| {
        let Some(recipe) = recipe_of_value(&c.arg(0))? else {
            return Ok(c.push(false));
        };
        let Some(item) = item_arg_value(&c.arg(1)) else {
            return Ok(c.push(true));
        };
        let item_b = item.borrow();
        let recipe_b = recipe.borrow();
        for src in recipe_b.source() {
            if item_matches_source(c.thread, &item_b, src)? {
                return Ok(c.push(true));
            }
        }
        Ok(c.push(false))
    });

    // `validateHasHeat(recipe, item, containers, chr)` and `hasHeat(...)`
    for name in ["validateHasHeat", "hasHeat"] {
        b.static_fn(name, |c| {
            let Some(recipe) = recipe_of_value(&c.arg(0))? else {
                return Ok(c.push(false));
            };
            let selected = item_arg_value(&c.arg(1));
            let containers_val = c.arg(2);
            let chr = character_of_value(&c.arg(3))?;
            let containers = resolve_containers(&chr, &containers_val);
            let ok = validate_has_heat(&recipe.borrow(), &selected, &containers);
            Ok(c.push(ok))
        });
    }

    // `HasAllRequiredItems(recipe, chr, item, containers)`
    {
        let chars = chars.clone();
        b.static_fn("HasAllRequiredItems", move |c| {
            let Some(recipe) = recipe_of_value(&c.arg(0))? else {
                return Ok(c.push(false));
            };
            let chr = character_of_value(&c.arg(1))?;
            let selected = item_arg_value(&c.arg(2));
            let containers_val = c.arg(3);
            let containers = resolve_containers(&chr, &containers_val);
            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe.borrow(),
                &chr,
                &containers,
                &selected,
                &HashSet::new(),
                false,
            )?;
            Ok(c.push(matched.valid))
        });
    }

    // `isAllItemsUsableRotten(recipe, chr, item, containers)`
    {
        let chars = chars.clone();
        b.static_fn("isAllItemsUsableRotten", move |c| {
            let Some(recipe) = recipe_of_value(&c.arg(0))? else {
                return Ok(c.push(false));
            };
            let chr = character_of_value(&c.arg(1))?;
            if cooking_level(&chars, &chr) < 7 {
                return Ok(c.push(false));
            }
            let selected = item_arg_value(&c.arg(2));
            let containers_val = c.arg(3);
            let containers = resolve_containers(&chr, &containers_val);
            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe.borrow(),
                &chr,
                &containers,
                &selected,
                &HashSet::new(),
                false,
            )?;
            if !matched.valid {
                return Ok(c.push(false));
            }
            let mut has_rotten = false;
            for src in &matched.sources {
                for item in &src.items {
                    if item.borrow().is_rotten() {
                        has_rotten = true;
                        break;
                    }
                }
            }
            Ok(c.push(has_rotten))
        });
    }

    // `getAvailableItemsAll` / `getAvailableItemsNeeded`
    for (name, all_items) in [
        ("getAvailableItemsAll", true),
        ("getAvailableItemsNeeded", false),
    ] {
        let chars = chars.clone();
        b.static_fn(name, move |c| {
            let Some(recipe) = recipe_of_value(&c.arg(0))? else {
                return Ok(c.push(list_value(Vec::new())));
            };
            let chr = character_of_value(&c.arg(1))?;
            let containers_val = c.arg(2);
            let selected = item_arg_value(&c.arg(3));
            let ignore = collect_ignore_ids(&c.arg(4));
            let containers = resolve_containers(&chr, &containers_val);
            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe.borrow(),
                &chr,
                &containers,
                &selected,
                &ignore,
                all_items,
            )?;
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            for src in matched.sources {
                for item in src.items {
                    let id = item.borrow().id;
                    if seen.insert(id) {
                        out.push(item_value(&item));
                    }
                }
            }
            Ok(c.push(list_value(out)))
        });
    }

    // `getSourceItemsAll` / `getSourceItemsNeeded`
    for (name, all_items) in [
        ("getSourceItemsAll", true),
        ("getSourceItemsNeeded", false),
    ] {
        let chars = chars.clone();
        b.static_fn(name, move |c| {
            let Some(recipe) = recipe_of_value(&c.arg(0))? else {
                return Ok(c.push(list_value(Vec::new())));
            };
            let source_index = int_arg(c, 1);
            let chr = character_of_value(&c.arg(2))?;
            let containers_val = c.arg(3);
            let selected = item_arg_value(&c.arg(4));
            let ignore = collect_ignore_ids(&c.arg(5));
            let containers = resolve_containers(&chr, &containers_val);
            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe.borrow(),
                &chr,
                &containers,
                &selected,
                &ignore,
                all_items,
            )?;
            let items = usize::try_from(source_index)
                .ok()
                .and_then(|i| matched.sources.get(i))
                .map(|src| src.items.iter().map(item_value).collect())
                .unwrap_or_default();
            Ok(c.push(list_value(items)))
        });
    }

    // `getNumberOfTimesRecipeCanBeDone(recipe, chr, containers, selectedItem)`
    {
        let chars = chars.clone();
        b.static_fn("getNumberOfTimesRecipeCanBeDone", move |c| {
            let Some(recipe) = recipe_of_value(&c.arg(0))? else {
                return Ok(c.push(0i32));
            };
            let chr = character_of_value(&c.arg(1))?;
            let containers_val = c.arg(2);
            let selected = item_arg_value(&c.arg(3));
            let containers = resolve_containers(&chr, &containers_val);
            let recipe_b = recipe.borrow();
            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe_b,
                &chr,
                &containers,
                &selected,
                &HashSet::new(),
                true,
            )?;
            if !matched.valid {
                return Ok(c.push(0i32));
            }
            let mut max_times = i32::MAX;
            let mut any_consumable = false;
            for src in &matched.sources {
                if src.required <= 0.0 {
                    continue;
                }
                if !src.satisfied {
                    return Ok(c.push(0i32));
                }
                if src.keep {
                    continue;
                }
                any_consumable = true;
                let times = ((src.total_available + 1e-4) / src.required).floor() as i32;
                max_times = max_times.min(times);
            }
            let result = if !any_consumable {
                1
            } else if recipe_b.allow_only_one {
                max_times.min(1)
            } else {
                max_times.max(0)
            };
            Ok(c.push(result))
        });
    }

    // `PerformMakeItem(recipe, selectedItem, chr, containers)`
    {
        let chars = chars.clone();
        b.static_fn("PerformMakeItem", move |c| {
            let recipe_val = c.arg(0);
            let selected_val = c.arg(1);
            let chr_val = c.arg(2);
            let containers_val = c.arg(3);
            let Some(recipe_rc) = recipe_of_value(&recipe_val)? else {
                return Err(npe());
            };
            let selected = item_arg_value(&selected_val);
            let chr = character_of_value(&chr_val)?;
            let containers = resolve_containers(&chr, &containers_val);

            let (results_spec, remove_result_item, lua_create, lua_give_xp, sources_spec) = {
                let r = recipe_rc.borrow();
                (
                    r.results().to_vec(),
                    r.remove_result_item,
                    r.lua_calls().get(&LuaCall::LuaCreate).cloned(),
                    r.lua_calls().get(&LuaCall::LuaGiveXP).cloned(),
                    r.source().to_vec(),
                )
            };

            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe_rc.borrow(),
                &chr,
                &containers,
                &selected,
                &HashSet::new(),
                false,
            )?;
            if !matched.valid {
                return Ok(c.push(list_value(Vec::new())));
            }

            let mut used_items: Vec<Value> = Vec::new();
            for (src_spec, src_match) in sources_spec.iter().zip(matched.sources.iter()) {
                for (item_ref, &amt) in src_match
                    .items
                    .iter()
                    .zip(src_match.consume_amounts.iter())
                {
                    used_items.push(item_value(item_ref));
                    consume_matched_item(c.thread, src_spec, item_ref, amt)?;
                }
            }

            let inv = chr.as_ref().map(|ch| ch.borrow().items.inventory.clone());
            let mut created_items: Vec<ItemRef> = Vec::new();
            for res in &results_spec {
                let full_type = res.full_type();
                for _ in 0..res.count.max(1) {
                    let Some(item) = create_item(&full_type, 1.0) else {
                        continue;
                    };
                    if res.drainable_count > 0 {
                        let mut b = item.borrow_mut();
                        let delta = b.get_use_delta();
                        if delta > 0.0 {
                            b.set_current_uses_float(
                                (res.drainable_count as f32 * delta).min(1.0),
                                None,
                            );
                        }
                    }
                    initialise_item(c.thread, &item);
                    if !remove_result_item {
                        if let Some(inv_c) = &inv {
                            let _ = add_item(c.thread, inv_c, Some(item.clone()))?;
                        }
                    }
                    created_items.push(item);
                }
            }

            let used_list_val = list_value(used_items);
            let first_result_val = created_items
                .first()
                .map_or(Value::Nil, item_value);
            if let Some(fn_name) = lua_create {
                let func = function_object(c.thread, &fn_name);
                if !func.is_nil() {
                    let _ = c.thread.call(
                        &func,
                        &[
                            used_list_val.clone(),
                            first_result_val.clone(),
                            chr_val.clone(),
                            selected_val.clone(),
                        ],
                    )?;
                }
            }
            if let Some(fn_name) = lua_give_xp {
                let func = function_object(c.thread, &fn_name);
                if !func.is_nil() {
                    let _ = c.thread.call(
                        &func,
                        &[
                            recipe_val.clone(),
                            used_list_val,
                            first_result_val,
                            chr_val,
                        ],
                    )?;
                }
            }

            let result_vals: Vec<Value> = created_items.iter().map(item_value).collect();
            Ok(c.push(list_value(result_vals)))
        });
    }

    // `getDismantleRecipeFor(itemFullType)`
    {
        let sm = sm.clone();
        b.static_fn("getDismantleRecipeFor", move |c| {
            let Some(item_type) = str_arg(c, 0) else {
                return Ok(c.push(Value::Nil));
            };
            for script in sm.collection(ScriptType::Recipe).get_all_scripts() {
                let Some(r) = script.downcast::<Recipe>() else {
                    continue;
                };
                let r_b = r.borrow();
                if !r_b.name().to_ascii_lowercase().starts_with("dismantle ") {
                    continue;
                }
                let has_item = r_b.source().iter().any(|src| {
                    src.items()
                        .iter()
                        .any(|it| it.eq_ignore_ascii_case(&item_type))
                });
                if has_item {
                    return Ok(c.push(script_value(Some(&script))));
                }
            }
            Ok(c.push(Value::Nil))
        });
    }

    // `GetMovableRecipeTool(isPrimary, recipe, selectedItem, chr, containers)`
    {
        let chars = chars.clone();
        b.static_fn("GetMovableRecipeTool", move |c| {
            let is_primary = bool_arg(c, 0);
            let Some(recipe) = recipe_of_value(&c.arg(1))? else {
                return Ok(c.push(Value::Nil));
            };
            let selected = item_arg_value(&c.arg(2));
            let chr = character_of_value(&c.arg(3))?;
            let containers_val = c.arg(4);
            let containers = resolve_containers(&chr, &containers_val);
            let recipe_b = recipe.borrow();
            // Primary/secondary tool sources on movable recipes are the `keep` sources
            // (first keep source for primary, second keep source for secondary).
            let keep_indices: Vec<usize> = recipe_b
                .source()
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.keep.then_some(i))
                .collect();
            let target_idx = if is_primary {
                keep_indices.first().copied()
            } else {
                keep_indices.get(1).copied()
            };
            let Some(idx) = target_idx else {
                return Ok(c.push(Value::Nil));
            };
            let matched = match_recipe(
                c.thread,
                &chars,
                &recipe_b,
                &chr,
                &containers,
                &selected,
                &HashSet::new(),
                false,
            )?;
            let tool = matched
                .sources
                .get(idx)
                .and_then(|s| s.items.first())
                .map_or(Value::Nil, item_value);
            Ok(c.push(tool))
        });
    }

    // Build 42.22 deprecated stubs (`DebugType.Recipe.error` + false/0.0).
    b.static_fn("IsItemDestroyed", |_c| Ok(_c.push(false)));
    b.static_fn("UseAmount", |_c| Ok(_c.push(0.0f64)));
    b.static_fn("DoesWipeUseDelta", |_c| Ok(_c.push(false)));
    b.static_fn("DoesUseItemUp", |_c| Ok(_c.push(false)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::character_value;
    use crate::bind_item_container::container_value;
    use crate::bind_registry::install_script_manager;
    use pz_characters::character::{CharacterKind, GameCharacter};
    use pz_kahlua::TableRef;
    use pz_scripts::objects::recipes::Recipe;

    fn run(t: &mut Thread, src: &str) {
        let src = format!(
            "local function assert(c, m) if not c then error(m or 'assertion failed') end end\n{src}"
        );
        let f = pz_kahlua::stdlib::load_string(&src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).unwrap();
    }

    const SCRIPT_TEXT: &str = r#"
module Base {
    item Plank { DisplayName = Plank, ItemType = base:normal, Weight = 1.0, }
    item Hammer { DisplayName = Hammer, ItemType = base:normal, Weight = 1.0, }
    item BarricadeKit { DisplayName = Kit, ItemType = base:normal, Weight = 1.5, }
    item Thread { DisplayName = Thread, ItemType = base:drainable, UseDelta = 0.25, Weight = 0.1, }

    recipe MakeBarricadeKit {
        keep Base.Hammer,
        Base.Plank=2,
        Base.Thread;2,
        Result:Base.BarricadeKit=1,
        Time:50.0,
        OnCreate:RecipeTest_OnCreate,
        OnGiveXP:RecipeTest_OnGiveXP,
    }

    recipe Dismantle Kit {
        Base.BarricadeKit,
        keep Base.Hammer,
        Result:Base.Plank=1,
    }
}
"#;

    fn make_script_manager() -> Rc<ScriptManager> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "pz-recipe-manager-classic-{}-{n}",
            std::process::id()
        ));
        let file = dir.join("media/scripts/test.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, SCRIPT_TEXT).unwrap();
        let sm = ScriptManager::new();
        sm.enable_texture_capture();
        {
            let reg = sm.registries.get("item_type").unwrap();
            let mut r = reg.borrow_mut();
            let _ = r.register_id("base:normal", true);
            let _ = r.register_id("base:drainable", true);
        }
        {
            let mut st = sm.statics.items.borrow_mut();
            st.constants.insert("item_type", "NORMAL", "base:normal");
            st.constants.insert("item_type", "DRAINABLE", "base:drainable");
        }
        sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        // Build 42.22 `Recipe::load` only parses sources from script text; results
        // and Lua callbacks are configured programmatically (`DoResult` / `setLua*`).
        for script in sm.collection(ScriptType::Recipe).get_all_scripts() {
            if let Some(r) = script.downcast::<Recipe>() {
                let mut r = r.borrow_mut();
                if r.name() == "MakeBarricadeKit" {
                    r.do_result("Base.BarricadeKit=1").unwrap();
                    r.set_lua_call(LuaCall::LuaCreate, Some("RecipeTest_OnCreate".into()));
                    r.set_lua_call(LuaCall::LuaGiveXP, Some("RecipeTest_OnGiveXP".into()));
                } else if r.name() == "Dismantle Kit" {
                    r.do_result("Base.Plank=1").unwrap();
                }
            }
        }
        Rc::new(sm)
    }

    #[test]
    fn classic_recipe_manager_validates_counts_and_performs_craft() {
        let sm = make_script_manager();
        install_script_manager(sm.clone());
        let chars = Rc::new(RefCell::new(Characters::default()));

        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_scripts::register(&mut t, sm.clone());
        crate::bind_items::register(&mut t);
        crate::bind_item_container::register(&mut t);
        crate::bind_recipe_manager::register(&mut t, sm.clone(), chars.clone());

        let mut chr = GameCharacter::new(&sm, CharacterKind::Player);
        chr.known_recipes.push("MakeBarricadeKit".into());
        chr.known_recipes.push("Dismantle Kit".into());
        let inv = chr.items.inventory.clone();
        for ty in ["Base.Hammer", "Base.Plank", "Base.Plank", "Base.Plank", "Base.Plank", "Base.Thread"] {
            let item = create_item(ty, 1.0).unwrap();
            add_item(&mut t, &inv, Some(item)).unwrap();
        }
        let chr_rc = Rc::new(RefCell::new(chr));
        let chr_val = character_value(&chr_rc);
        let recipe_ref = sm
            .collection(ScriptType::Recipe)
            .get_all_scripts()
            .iter()
            .find(|s| s.downcast::<Recipe>().is_some_and(|r| r.borrow().name() == "MakeBarricadeKit"))
            .unwrap()
            .clone();

        t.globals().set_str("test_chr", chr_val);
        t.globals().set_str("test_inv", container_value(&inv));
        t.globals().set_str("test_recipe", script_value(Some(&recipe_ref)));

        run(
            &mut t,
            r#"
            created_calls = 0
            xp_calls = 0
            function RecipeTest_OnCreate(items, result, player, selected)
                created_calls = created_calls + 1
                assert(items:size() == 4, "expected 4 used items (1 hammer + 2 planks + 1 thread)")
                assert(result ~= nil and result:getFullType() == "Base.BarricadeKit")
            end
            function RecipeTest_OnGiveXP(recipe, ingredients, result, player)
                xp_calls = xp_calls + 1
            end

            local containers = ArrayList.new()
            containers:add(test_inv)

            assert(RecipeManager.getKnownRecipesNumber(test_chr) == 2, "known=" .. tostring(RecipeManager.getKnownRecipesNumber(test_chr)))
            assert(RecipeManager.IsRecipeValid(test_recipe, test_chr, nil, containers) == true, "IsRecipeValid")
            assert(RecipeManager.HasAllRequiredItems(test_recipe, test_chr, nil, containers) == true, "HasAllRequiredItems")
            -- 4 planks (2/craft) + 1 full thread (4 uses, 2/craft) + 1 keep hammer => 2 crafts!
            assert(RecipeManager.getNumberOfTimesRecipeCanBeDone(test_recipe, test_chr, containers, nil) == 2, "times=" .. tostring(RecipeManager.getNumberOfTimesRecipeCanBeDone(test_recipe, test_chr, containers, nil)))

            local needed = RecipeManager.getAvailableItemsNeeded(test_recipe, test_chr, containers, nil, nil)
            assert(needed:size() == 4, "needed=" .. tostring(needed:size()))
            local all_planks = RecipeManager.getSourceItemsAll(test_recipe, 1, test_chr, containers, nil, nil)
            assert(all_planks:size() == 4, "all_planks=" .. tostring(all_planks:size()))
            local needed_planks = RecipeManager.getSourceItemsNeeded(test_recipe, 1, test_chr, containers, nil, nil)
            assert(needed_planks:size() == 2, "needed_planks=" .. tostring(needed_planks:size()))

            local tool = RecipeManager.GetMovableRecipeTool(true, test_recipe, nil, test_chr, containers)
            assert(tool ~= nil and tool:getFullType() == "Base.Hammer", "tool")

            local results = RecipeManager.PerformMakeItem(test_recipe, nil, test_chr, containers)
            assert(results:size() == 1, "results=" .. tostring(results:size()))
            assert(results:get(0):getFullType() == "Base.BarricadeKit", "result type")
            assert(created_calls == 1 and xp_calls == 1, "callbacks")

            -- After 1 craft: 1 hammer kept, 2 planks left, thread at 0.5 uses (2 uses left), 1 BarricadeKit added => 1 craft left!
            assert(RecipeManager.getNumberOfTimesRecipeCanBeDone(test_recipe, test_chr, containers, nil) == 1, "times_after=" .. tostring(RecipeManager.getNumberOfTimesRecipeCanBeDone(test_recipe, test_chr, containers, nil)))

            local dismantle = RecipeManager.getDismantleRecipeFor("Base.BarricadeKit")
            assert(dismantle ~= nil, "dismantle")
            local unique = RecipeManager.getUniqueRecipeItems(results:get(0), test_chr, containers)
            assert(unique:size() == 1, "unique=" .. tostring(unique:size()))
        "#,
        );

        // Verify thread drained from 1.0 -> 0.5 and hammer was kept.
        let inv_items = inv.borrow().items.clone();
        let thread = inv_items
            .iter()
            .find(|i| i.borrow().get_full_type() == "Base.Thread")
            .expect("thread should still exist with 2 uses remaining");
        assert!((thread.borrow().get_current_uses_float() - 0.5).abs() < 1e-4);
        assert!(inv_items.iter().any(|i| i.borrow().get_full_type() == "Base.Hammer"));
    }
}
