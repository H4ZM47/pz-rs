//! `zombie.entity.components.crafting.recipe.CraftRecipeManager`: the static
//! `getUniqueRecipeItems`, which `ISInventoryPaneContextMenu.createMenu`
//! calls for each selected item, and the parts of the manager it needs:
//! the recipe tag query (`queryRecipes`, over `TaggedObjectManager`),
//! `isValidRecipeForCharacter` and `getValidInputScriptForItem`.
//!
//! Not ported yet, and reported with `unsupported` when reached:
//! - `HandcraftLogic`: `canPerformCurrentRecipe` for a recipe that accepts
//!   the item;
//! - the item status tests of `consumeInputItemInternal`
//!   (`InputScript.doesItemPassRoutineStatusTests` and the others) for an
//!   item that an input script lists;
//! - `CraftRecipe.validateBenefitFromRecipeAtHand` for a learned recipe
//!   whose skill requirement the character does not meet
//!   (`IsoPlayer.tooDarkToRead`, `hasRecipeAtHand`).

use crate::bind_character_sim::CharacterObject;
use crate::bind_scripts::script_list;
use crate::expose::expose_class;
use pz_characters::Characters;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_scripts::java;
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, InputScript, LuaCall};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::objects::items::Item;
use pz_scripts::{ScriptManager, ScriptRef, ScriptType};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

pub const CRAFT_RECIPE_MANAGER_CLASS: &str = "zombie.entity.components.crafting.recipe.CraftRecipeManager";

/// `TaggedObjectManager.defaultTag`: the tag of an object without tags.
const DEFAULT_TAG: &str = "untagged";

/// `TaggedObjectManager.sanitizeTag`.
fn sanitize_tag(tag: &str) -> String {
    java::to_lower(java::trim(tag))
}

/// `TaggedObjectManager.readWhitelist` and `readBlackList`, as sets of
/// sanitized tags. `None` is the original's `null` array.
type TagLists = (Option<HashSet<String>>, Option<HashSet<String>>);

fn read_tag_lists(query: &str) -> LuaResult<TagLists> {
    let set = |s: &str| -> HashSet<String> { java::split_char(s, ';').iter().map(|t| sanitize_tag(t)).collect() };
    if !query.contains('-') {
        return Ok((Some(set(query)), None));
    }
    if let Some(rest) = query.strip_prefix('-') {
        return Ok((None, Some(set(rest))));
    }
    let parts = java::split_char(query, '-');
    let at = |i: usize| {
        parts
            .get(i)
            .copied()
            .ok_or_else(|| LuaError::runtime(format!("ArrayIndexOutOfBoundsException: Index {i} out of bounds")))
    };
    Ok((Some(set(at(0)?)), Some(set(at(1)?))))
}

/// The tags `TaggedObjectManager.registerObject` gives a recipe: its
/// `getTags()`, sanitized, or the default tag when it has none.
fn recipe_tags(recipe: &CraftRecipe) -> HashSet<String> {
    if recipe.tags().is_empty() {
        return HashSet::from([DEFAULT_TAG.to_string()]);
    }
    recipe.tags().iter().map(|t| sanitize_tag(t)).collect()
}

/// `CraftRecipeManager.queryRecipes(String)`: every recipe for `"*"`,
/// otherwise `TaggedObjectManager.queryTaggedObjects`. A recipe matches when
/// it has a whitelisted tag (if there is a whitelist) and no blacklisted tag;
/// the order is `ScriptManager.getAllCraftRecipes()`.
///
/// The original caches each query as a `TagGroup` that it updates when tags
/// change; the result is the same list. Its "not initialized" check throws
/// only in debug mode, before `CraftRecipeManager.Init` has run with the
/// scripts.
pub fn query_recipes(sm: &ScriptManager, query: &str) -> LuaResult<Vec<ScriptRef>> {
    let all = sm.collection(ScriptType::CraftRecipe).get_all_scripts();
    if !java::is_null_or_whitespace(Some(query)) && query == "*" {
        return Ok(all);
    }
    if java::is_null_or_whitespace(Some(query)) {
        // `DebugType.General.warn`, then the shared empty list.
        return Ok(Vec::new());
    }
    // `formatQueryString` sanitizes and sorts the tags; as sets they are
    // the same.
    let (white, black) = read_tag_lists(query)?;
    let white = white.filter(|w| !w.is_empty());
    let black = black.filter(|b| !b.is_empty());
    if white.is_none() && black.is_none() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for script in all {
        let Some(recipe) = script.downcast::<CraftRecipe>() else {
            continue;
        };
        let tags = recipe_tags(&recipe.borrow());
        if white.as_ref().is_some_and(|w| w.is_disjoint(&tags)) {
            continue;
        }
        if black.as_ref().is_some_and(|b| !b.is_disjoint(&tags)) {
            continue;
        }
        out.push(script);
    }
    Ok(out)
}

/// `CraftRecipeManager.queryRecipes(CraftRecipeTag...)`: each tag's id
/// followed by `;`.
pub fn query_recipes_for_tags(sm: &ScriptManager, tags: &[&str]) -> LuaResult<Vec<ScriptRef>> {
    let query: String = tags.iter().map(|t| format!("{t};")).collect();
    query_recipes(sm, &query)
}

/// What the recipe checks read from the `IsoGameCharacter`.
pub trait RecipeCharacter {
    /// `getPerkLevel(PerkFactory.Perks.<perk>)`; 0 for an unknown perk.
    fn perk_level(&self, perk: &str) -> i32;
    /// `instanceof IsoPlayer`.
    fn is_player(&self) -> bool;
    /// `isKnowAllRecipes`.
    fn is_know_all_recipes(&self) -> bool;
    /// `getKnownRecipes().contains(name)`.
    fn knows(&self, name: &str) -> bool;
}

/// `IsoGameCharacter.isRecipeKnown(CraftRecipe, true)` (the
/// `isRecipeActuallyKnown` form, which skips `SeeNotLearntRecipe`).
fn is_recipe_actually_known(recipe: &CraftRecipe, chr: &dyn RecipeCharacter) -> bool {
    if chr.is_know_all_recipes() {
        return true;
    }
    if !recipe.need_to_be_learn() {
        return true;
    }
    let knows = |name: Option<&str>| name.is_some_and(|n| chr.knows(n));
    knows(recipe.name()) || knows(recipe.meta_recipe()) || knows(recipe.translation_name())
}

/// `CraftRecipe.validateBenefitFromRecipeAtHand(chr, containers)`:
/// `canBenefitFromRecipeAtHand(chr) && hasRecipeAtHand(chr, containers)`.
///
/// `canBenefitFromRecipeAtHand` is false for a non-player, then for
/// `tooDarkToRead`, then unless `couldBenefitFromRecipeAtHand`, which is
/// false for a recipe that need not be learned or that the character does
/// not know. `tooDarkToRead` only reads state, so testing those two first
/// gives the same result.
fn validate_benefit_from_recipe_at_hand(recipe: &CraftRecipe, chr: &dyn RecipeCharacter) -> LuaResult<bool> {
    if !chr.is_player() || !recipe.need_to_be_learn() || !is_recipe_actually_known(recipe, chr) {
        return Ok(false);
    }
    // TODO(CraftRecipe.validateBenefitFromRecipeAtHand): `tooDarkToRead`,
    // the one-level skill gap test and `hasRecipeAtHand`.
    Err(crate::bind_world_object::unsupported("CraftRecipe.validateBenefitFromRecipeAtHand"))
}

/// `CraftRecipeManager.validateHasRequiredSkill(recipe, chr, containers)`.
fn validate_has_required_skill(
    recipe: &CraftRecipe,
    chr: &dyn RecipeCharacter,
    has_containers: bool,
) -> LuaResult<bool> {
    for skill in recipe.skill_required.iter().flatten() {
        if chr.perk_level(skill.perk()) >= skill.level() {
            continue;
        }
        if !has_containers || !validate_benefit_from_recipe_at_hand(recipe, chr)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `CraftRecipeManager.isValidRecipeForCharacter(recipe, chr, null,
/// containers)`. The skipped recipes are only logged
/// (`DebugType.CraftLogic`).
pub fn is_valid_recipe_for_character(
    recipe: &CraftRecipe,
    chr: Option<&dyn RecipeCharacter>,
    has_containers: bool,
) -> LuaResult<bool> {
    let Some(chr) = chr else {
        return Ok(true);
    };
    if !validate_has_required_skill(recipe, chr, has_containers)? {
        return Ok(false);
    }
    if recipe.need_to_be_learn() && !is_recipe_actually_known(recipe, chr) {
        return Ok(false);
    }
    Ok(true)
}

/// `CraftRecipe.OnTestItem(item, chr)`: false without a character, true
/// without an `OnTest` function, otherwise the function's result
/// (`on_test` calls it).
fn on_test_item(recipe: &CraftRecipe, has_chr: bool, on_test: &mut dyn FnMut(&str) -> bool) -> bool {
    if !has_chr {
        return false;
    }
    match recipe.lua_call_string(LuaCall::OnTest) {
        Some(name) => on_test(name),
        None => true,
    }
}

/// `CraftRecipeManager.consumeInputItem(input, item, true, null, chr)` for
/// an `Item` input, through `consumeInputItemInternal`: false for an item
/// without a script or one the input does not list (`containsItem`), and
/// false when the recipe's `OnTestItem` rejects it.
fn is_item_valid_for_input_script(
    input: &InputScript,
    recipe: &CraftRecipe,
    script_item: Option<&Rc<RefCell<Item>>>,
    has_chr: bool,
    on_test: &mut dyn FnMut(&str) -> bool,
) -> LuaResult<bool> {
    if input.resource_type() != ResourceType::Item {
        return Ok(false);
    }
    let Some(script_item) = script_item else {
        return Ok(false);
    };
    // `containsItem`: `itemScriptCache.contains(item)`. The original's
    // `OnPostWorldDictionaryInit` adds the items of each input tag
    // (`ItemTags.getItemsForTag`) to that cache after the listed items; the
    // port's cache has only the listed items yet (TODO(ItemTags) in
    // `InputScript::resolve_items`), so the tags are tested here.
    let listed =
        input.possible_input_items().iter().any(|s| s.downcast::<Item>().is_some_and(|s| Rc::ptr_eq(&s, script_item)))
            || (!input.is_accepts_any_item() && {
                let script_item = script_item.borrow();
                input.item_tags().iter().flatten().any(|t| script_item.has_tag(Some(t)))
            });
    if !listed {
        return Ok(false);
    }
    if !on_test_item(recipe, has_chr, on_test) {
        return Ok(false);
    }
    // TODO(CraftRecipeManager.consumeInputItemInternal): the item status
    // tests (`doesItemPassRoutineStatusTests`, clothing, sharpness, damage,
    // empty/full, food), the empty-container check, and the uses, fluid and
    // energy amounts.
    Err(crate::bind_world_object::unsupported(format!(
        "CraftRecipeManager.consumeInputItemInternal status tests ({})",
        recipe.name().unwrap_or_default()
    )))
}

/// `CraftRecipeManager.getValidInputScriptForItem(recipe, item, chr)`: the
/// index (into `getInputs()`) of the first `Item` input that accepts the
/// item.
fn valid_input_script_for_item(
    recipe: &CraftRecipe,
    script_item: Option<&Rc<RefCell<Item>>>,
    has_chr: bool,
    on_test: &mut dyn FnMut(&str) -> bool,
) -> LuaResult<Option<usize>> {
    for (i, input) in recipe.inputs().enumerate() {
        if input.resource_type() != ResourceType::Item {
            continue;
        }
        if is_item_valid_for_input_script(input, recipe, script_item, has_chr, on_test)? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// `CraftRecipeManager.getUniqueRecipeItems(item, chr, containers)`: the
/// `InHandCraft`/`AnySurfaceCraft` recipes that are valid for the character,
/// have an input that accepts the item, pass `OnTestItem`, and that a
/// `HandcraftLogic` set to the recipe (with the item, a craft surface within
/// two squares and the containers) can perform.
///
/// The original builds the `HandcraftLogic` (and its `findCraftSurface`)
/// before the loop. Nothing reads it before `canPerformCurrentRecipe`, which
/// is not ported, so a recipe that gets that far returns an error (until the
/// item status tests are ported, those return the error first). It also
/// returns its one static `RecipeList`, cleared on each call; this returns a
/// new list.
pub fn unique_recipe_items(
    sm: &ScriptManager,
    script_item: Option<&Rc<RefCell<Item>>>,
    chr: Option<&dyn RecipeCharacter>,
    has_containers: bool,
    on_test: &mut dyn FnMut(&str) -> bool,
) -> LuaResult<Vec<ScriptRef>> {
    use pz_scripts::objects::entity::components::crafting::craft_recipe_tag::{ANY_SURFACE_CRAFT, IN_HAND_CRAFT};
    let recipes = query_recipes_for_tags(sm, &[IN_HAND_CRAFT, ANY_SURFACE_CRAFT])?;
    let out = Vec::new();
    for script in recipes {
        let Some(recipe) = script.downcast::<CraftRecipe>() else {
            continue;
        };
        let recipe = recipe.borrow();
        if !is_valid_recipe_for_character(&recipe, chr, has_containers)? {
            continue;
        }
        if valid_input_script_for_item(&recipe, script_item, chr.is_some(), on_test)?.is_none() {
            continue;
        }
        if !on_test_item(&recipe, chr.is_some(), on_test) {
            continue;
        }
        // TODO(HandcraftLogic): `setRecipeFromContextClick(recipe, item)`
        // and `canPerformCurrentRecipe()`.
        return Err(crate::bind_world_object::unsupported(format!(
            "HandcraftLogic.canPerformCurrentRecipe ({})",
            recipe.name().unwrap_or_default()
        )));
    }
    Ok(out)
}

/// The `IsoGameCharacter` argument, read through the shared characters.
struct LuaRecipeCharacter<'a> {
    chars: &'a RefCell<Characters>,
    character: &'a CharacterObject,
}

impl RecipeCharacter for LuaRecipeCharacter<'_> {
    fn perk_level(&self, perk: &str) -> i32 {
        let perk = self.chars.borrow().perks.constant(perk);
        self.character.0.borrow().perk_level(perk.as_ref())
    }
    fn is_player(&self) -> bool {
        self.character.0.borrow().is_player()
    }
    fn is_know_all_recipes(&self) -> bool {
        let ctx = crate::bind_character_sim::current_sim_context();
        self.character.0.borrow().cheats.is_set(&*ctx, "KNOW_ALL_RECIPES")
    }
    fn knows(&self, name: &str) -> bool {
        self.character.0.borrow().known_recipes.iter().any(|k| k == name)
    }
}

/// `protectedCallBoolean(LuaManager.getFunctionObject(name), item, chr)`:
/// true only for a `true` result; a missing function counts as passing.
fn call_on_test(thread: &mut Thread, name: &str, item: &Value, chr: &Value) -> bool {
    let f = crate::bind_item_container::function_object(thread, name);
    if f.is_nil() {
        return true;
    }
    matches!(thread.pcall(&f, &[item.clone(), chr.clone()]).as_deref(), Ok([Value::Bool(true), ..]))
}

pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    let b = expose_class(thread, CRAFT_RECIPE_MANAGER_CLASS, None);
    b.static_fn("getUniqueRecipeItems", move |c| {
        let item_value = c.arg(0);
        let item = crate::bind_world_object::item_argument(&item_value)?;
        let chr_value = c.arg(1);
        let character = match &chr_value {
            Value::Object(o) => Some(
                (&**o as &dyn Any)
                    .downcast_ref::<CharacterObject>()
                    .ok_or_else(|| LuaError::runtime("expected IsoGameCharacter"))?,
            ),
            Value::Nil => None,
            _ => return Err(LuaError::runtime("expected IsoGameCharacter")),
        };
        let has_containers = !c.arg(2).is_nil();
        let lua_chr = character.map(|character| LuaRecipeCharacter { chars: &chars, character });
        let thread = &mut *c.thread;
        let mut on_test = |name: &str| call_on_test(thread, name, &item_value, &chr_value);
        // `InventoryItem.getScriptItem`, read before `OnTest` gets the item.
        let script_item = pz_characters::character::items::script_item(&sm, &item.borrow());
        let list = unique_recipe_items(
            &sm,
            script_item.as_ref(),
            lua_chr.as_ref().map(|c| c as &dyn RecipeCharacter),
            has_containers,
            &mut on_test,
        )?;
        Ok(c.push(script_list(&list)))
    });
}

#[cfg(test)]
mod tests;
